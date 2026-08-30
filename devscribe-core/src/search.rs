//! Naive in-memory text search: one file's content in, every match's
//! position out. The caller (the app, which already walks the project tree
//! for the sidebar) is responsible for iterating files — this stays a pure,
//! headlessly-testable function with no filesystem access of its own. Start
//! naive; an index can replace this later if it's ever too slow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    /// 0-based line number.
    pub line: usize,
    /// 0-based char column of the match's start in the **original line** —
    /// for placing a cursor in the real buffer (`SearchResultSelected`).
    /// Not an offset into `preview` — see `preview_col` for that. A real
    /// project can have a line with hundreds of thousands or millions of
    /// characters (a minified bundle, a huge generated/embedded blob — this
    /// crate's own design mockup is one such file), so `preview` is
    /// deliberately *not* "the whole line" the way `col` still refers to
    /// the whole line; the two used to share one offset back when preview
    /// always was the full line, but that made a single match on a
    /// pathological line render as a text widget with over a million
    /// characters in it — no text-shaping/layout engine handles that
    /// gracefully, GPU-accelerated or not.
    pub col: usize,
    /// A short, render-safe snippet of the line *around* the match — never
    /// the whole line — with leading whitespace kept and trailing
    /// whitespace trimmed. Truncated ends get a `…` marker.
    pub preview: String,
    /// 0-based char column of the match's start **within `preview`**
    /// (distinct from `col` whenever the window doesn't start at the
    /// original line's own start — i.e. almost always once a leading `…`
    /// is involved). This is what actually indexes into `preview` for
    /// highlighting; `col` no longer safely does.
    pub preview_col: usize,
}

/// Chars of context kept on each side of the match within `preview` — small
/// enough that even a worst-case line (all 4-byte UTF-8 chars either side)
/// keeps `preview` at a few hundred bytes, not a few hundred thousand.
const PREVIEW_CONTEXT_CHARS: usize = 60;

/// Case-insensitive (ASCII-only, to keep byte/char offsets in lockstep)
/// substring search across every line of `text`, stopping once `max_hits`
/// matches have been found.
///
/// `max_hits` isn't just a nicety for a huge project — it bounds the *work*
/// this function does, not merely how many results a caller keeps. Without
/// a cap applied *during* the scan, one large file matched many times would
/// fully materialize every match before a caller's own results cap ever
/// got a chance to apply — a real crash risk, not a hypothetical one,
/// since this runs on every search.
pub fn search_text(text: &str, query: &str, max_hits: usize) -> Vec<SearchHit> {
    if query.is_empty() || max_hits == 0 {
        return Vec::new();
    }
    let query_lower = query.to_ascii_lowercase();
    let needle = query_lower.as_bytes();

    let mut hits = Vec::new();
    'lines: for (line_idx, line) in text.lines().enumerate() {
        let haystack = line.as_bytes();
        let mut start = 0;
        // Running `(byte, char)` cursor into `line`. Converting a match's
        // byte offset to a char column used to recount from the line's start
        // per match, which is quadratic on a line carrying many matches.
        let mut walked_bytes = 0usize;
        let mut walked_chars = 0usize;
        while start + needle.len() <= haystack.len() {
            let Some(pos) = find_ascii_ci(&haystack[start..], needle) else {
                break;
            };
            let byte_col = start + pos;
            walked_chars += line[walked_bytes..byte_col].chars().count();
            walked_bytes = byte_col;
            let (preview, preview_col) = windowed_preview(line, byte_col, needle.len());
            hits.push(SearchHit { line: line_idx, col: walked_chars, preview, preview_col });
            if hits.len() >= max_hits {
                break 'lines;
            }
            start = byte_col + needle.len();
        }
    }
    hits
}

/// First offset in `haystack` where `needle` (already ASCII-lowercased)
/// matches under ASCII case folding, or `None`.
///
/// Scans bytes rather than lowercasing a copy of the line first. That copy
/// was the single biggest cost of find-as-you-type: every keystroke
/// allocated and rewrote the entire document, line by line, before looking
/// at any of it — on a multi-megabyte file, per keystroke. Skipping it is
/// also *faster*, not merely leaner, because `memchr2` finds the candidate
/// starts (either case of the needle's first byte) with SIMD.
///
/// Byte-wise matching stays UTF-8-safe: ASCII case folding only ever touches
/// `0x41..=0x5A`, and no UTF-8 continuation byte falls in that range, so a
/// byte-sequence match can only begin on a char boundary — the same property
/// the `to_ascii_lowercase` version relied on to keep byte and char offsets
/// in lockstep.
fn find_ascii_ci(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    let first = needle[0];
    let first_upper = first.to_ascii_uppercase();
    let last_start = haystack.len() - needle.len();
    let mut i = 0;
    while i <= last_start {
        i += memchr::memchr2(first, first_upper, &haystack[i..=last_start])?;
        if haystack[i + 1..i + needle.len()]
            .iter()
            .zip(&needle[1..])
            .all(|(h, n)| h.to_ascii_lowercase() == *n)
        {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Builds a bounded `(preview, preview_col)` around the match at byte
/// offset `match_start`/length `match_len` in `line` — up to
/// `PREVIEW_CONTEXT_CHARS` of context on each side, with a `…` marker
/// wherever a side got cut. Windowing happens in *bytes* first (cheap: no
/// scan of the whole line needed, just nudging two candidate offsets to the
/// nearest valid UTF-8 boundary), which already upper-bounds the resulting
/// char count — a byte-bounded slice can never contain more chars than
/// bytes — so there's no separate char-length pass needed after.
fn windowed_preview(line: &str, match_start: usize, match_len: usize) -> (String, usize) {
    // 4 bytes/char covers every UTF-8 scalar value, so this many bytes of
    // context can never be trimmed down to fewer than `PREVIEW_CONTEXT_CHARS`
    // chars by the boundary-nudging below.
    let context_bytes = PREVIEW_CONTEXT_CHARS * 4;

    let mut window_start = match_start.saturating_sub(context_bytes);
    while window_start < match_start && !line.is_char_boundary(window_start) {
        window_start += 1;
    }
    let mut window_end = (match_start + match_len + context_bytes).min(line.len());
    while window_end < line.len() && !line.is_char_boundary(window_end) {
        window_end += 1;
    }

    let truncated_start = window_start > 0;
    // Trimming trailing whitespace can only move this earlier, never past
    // `window_end`, so it can't undo the truncation check above.
    let slice = line[window_start..window_end].trim_end();
    let truncated_end = window_end < line.len();

    let mut preview = String::with_capacity(slice.len() + 6);
    let mut preview_col = 0;
    if truncated_start {
        preview.push('\u{2026}');
        preview_col += 1;
    }
    preview_col += line[window_start..match_start].chars().count();
    preview.push_str(slice);
    if truncated_end {
        preview.push('\u{2026}');
    }

    (preview, preview_col)
}

#[cfg(test)]
#[path = "tests/search.rs"]
mod tests;
