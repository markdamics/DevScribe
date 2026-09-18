//! Bracket-pair matching: given where the cursor sits, finds the `(`/`)`,
//! `[`/`]`, or `{`/`}` pair it's touching, so the editor canvas can highlight
//! both halves (VS Code's/most editors' "bracket pair colorization" —
//! matching, not the rainbow-nesting-depth-coloring some editors also call
//! by that name, which is a separate, much larger feature).
//!
//! A plain text scan, not a tree-sitter node walk — `syntax::Span`s (already
//! computed for highlighting) are reused only to skip bracket characters
//! sitting inside a string or comment, which keeps this from misfiring on
//! `"a (fake) bracket"` without needing a second parse pass.

use crate::syntax::{HighlightKind, Span};
use ropey::Rope;

/// How far a scan for an unmatched bracket's partner is allowed to travel
/// before giving up — an unclosed `(` at the top of a huge file would
/// otherwise walk the entire document on every cursor move. Missing a match
/// past this distance is an acceptable trade-off; the same shape as
/// `MAX_RENDERED_LINE_CHARS`'s cap on a single pathological line.
const MAX_BRACKET_SCAN: usize = 50_000;

/// The `Span` (if any) covering `byte_idx`, via the same
/// document-ordered/non-overlapping binary search `editor_canvas.rs`'s
/// render loop already uses to find a line's starting span.
fn highlight_kind_at(highlights: &[Span], byte_idx: usize) -> Option<HighlightKind> {
    let i = highlights.partition_point(|s| s.end <= byte_idx);
    highlights.get(i).filter(|s| s.start <= byte_idx && byte_idx < s.end).map(|s| s.kind)
}

fn is_string_or_comment(highlights: &[Span], byte_idx: usize) -> bool {
    matches!(highlight_kind_at(highlights, byte_idx), Some(HighlightKind::String) | Some(HighlightKind::Comment))
}

/// Finds `idx`'s bracket partner, if `idx` is itself a bracket character not
/// inside a string/comment — walking outward from `idx` and tracking
/// nesting depth against every same-kind bracket pair encountered along the
/// way (also skipping any that fall inside a string/comment), so
/// `(a[b(c)]d)` matching the outer `(` correctly skips past the inner pair
/// rather than stopping at the first `)`. Returns `(open_idx, close_idx)`
/// (char indices) either way, regardless of which half `idx` was.
fn find_bracket_partner(rope: &Rope, highlights: &[Span], idx: usize) -> Option<(usize, usize)> {
    let ch = rope.char(idx);
    // `own`: whichever bracket char `idx` itself is — walking in the
    // matching direction, seeing another `own` is one level deeper; seeing
    // `partner` is one level shallower. Reaching depth 0 again is the match.
    let (own, partner, forward) = match ch {
        '(' => ('(', ')', true),
        '[' => ('[', ']', true),
        '{' => ('{', '}', true),
        ')' => (')', '(', false),
        ']' => (']', '[', false),
        '}' => ('}', '{', false),
        _ => return None,
    };
    if is_string_or_comment(highlights, rope.char_to_byte(idx)) {
        return None;
    }

    let total = rope.len_chars();
    let limit = if forward {
        total.min(idx + MAX_BRACKET_SCAN)
    } else {
        idx.saturating_sub(MAX_BRACKET_SCAN)
    };
    let mut depth: i32 = 0;
    let mut i = idx;
    loop {
        let c = rope.char(i);
        if c == own || c == partner {
            if !is_string_or_comment(highlights, rope.char_to_byte(i)) {
                depth += if c == own { 1 } else { -1 };
                if depth == 0 {
                    return Some(if forward { (idx, i) } else { (i, idx) });
                }
            }
        }
        if forward {
            if i + 1 >= limit {
                return None;
            }
            i += 1;
        } else {
            if i <= limit {
                return None;
            }
            i -= 1;
        }
    }
}

/// One bracket character's nesting depth, for rainbow bracket-pair
/// colorization (roadmap item 15 — the "color matching pairs by nesting
/// depth" reading of that feature, distinct from `matching_bracket_pair`'s
/// own "highlight the pair touching the cursor" one; see this module's own
/// doc comment). `byte_idx` is the bracket character's own byte offset
/// (always one byte — `(`/`)`/`[`/`]`/`{`/`}` are all ASCII); `depth` is
/// 1-based and shared by an opener and its matching closer (a top-level
/// `(...)` is depth 1 on both halves), so a caller just needs
/// `(depth - 1) % palette.len()` to pick a color, the same "no bound on how
/// deep code nests, so wrap the palette" scheme every other rainbow-bracket
/// implementation uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BracketDepth {
    pub byte_idx: usize,
    pub depth: u32,
}

/// Every bracket character in `rope` (skipping ones inside a string/comment,
/// same as `find_bracket_partner`), tagged with its nesting depth — a single
/// forward pass tracking one shared counter across all three bracket kinds
/// together (matching VS Code's own default: `([)]` reads as two nested
/// levels, not two independent counters), so mismatched/unbalanced brackets
/// degrade gracefully (an extra unmatched closer just floors at depth 1
/// rather than going negative) instead of needing real pair-matching first.
/// Returned in document order, ready for the caller's own binary search
/// (`editor_canvas.rs`'s render loop, which already has one for
/// `highlights`).
pub fn bracket_depths(rope: &Rope, highlights: &[Span]) -> Vec<BracketDepth> {
    let mut depth: u32 = 0;
    let mut out = Vec::new();
    let mut byte_idx = 0usize;
    for ch in rope.chars() {
        let is_open = matches!(ch, '(' | '[' | '{');
        let is_close = matches!(ch, ')' | ']' | '}');
        if (is_open || is_close) && !is_string_or_comment(highlights, byte_idx) {
            if is_open {
                depth += 1;
                out.push(BracketDepth { byte_idx, depth });
            } else {
                out.push(BracketDepth { byte_idx, depth: depth.max(1) });
                depth = depth.saturating_sub(1);
            }
        }
        byte_idx += ch.len_utf8();
    }
    out
}

/// The bracket pair (char indices) touching `cursor_char_idx`, if any —
/// checked at the cursor position itself first, then one char back, so the
/// caret sitting immediately *after* a bracket (where it lands right after
/// typing one) still matches it, same dual-check most editors use.
/// `highlights` should be the buffer's own current syntax spans (byte
/// offsets, as `syntax::highlight` produces) — pass an empty slice to match
/// without string/comment awareness (e.g. for a language with no grammar
/// wired up).
pub fn matching_bracket_pair(rope: &Rope, highlights: &[Span], cursor_char_idx: usize) -> Option<(usize, usize)> {
    let total = rope.len_chars();
    for idx in [cursor_char_idx, cursor_char_idx.wrapping_sub(1)] {
        if idx >= total {
            continue;
        }
        if let Some(pair) = find_bracket_partner(rope, highlights, idx) {
            return Some(pair);
        }
    }
    None
}

#[cfg(test)]
#[path = "tests/bracket.rs"]
mod tests;
