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
