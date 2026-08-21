//! Naive in-memory text search: one file's content in, every match's
//! position out. The caller (the app, which already walks the project tree
//! for the sidebar) is responsible for iterating files — this stays a pure,
//! headlessly-testable function with no filesystem access of its own. Start
//! naive; an index can replace this later if it's ever too slow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    /// 0-based line number.
    pub line: usize,
    /// 0-based char column of the match's start — in both the original line
    /// (for placing a cursor in the real buffer) *and* `preview` (for
    /// highlighting the match within it). Only safe because `preview` keeps
    /// leading whitespace; see its doc.
    pub col: usize,
    /// The matched line, with trailing whitespace trimmed but leading
    /// whitespace kept — deliberately, so `col` indexes correctly into both
    /// this and the original line without two different offsets.
    pub preview: String,
}

/// Case-insensitive (ASCII-only, to keep byte/char offsets in lockstep)
/// substring search across every line of `text`.
pub fn search_text(text: &str, query: &str) -> Vec<SearchHit> {
    if query.is_empty() {
        return Vec::new();
    }
    let query_lower = query.to_ascii_lowercase();

    let mut hits = Vec::new();
    for (line_idx, line) in text.lines().enumerate() {
        let line_lower = line.to_ascii_lowercase();
        let mut start = 0;
        while start <= line_lower.len() {
            let Some(pos) = line_lower[start..].find(&query_lower) else {
                break;
            };
            let byte_col = start + pos;
            let col = line[..byte_col].chars().count();
            hits.push(SearchHit {
                line: line_idx,
                col,
                preview: line.trim_end().to_string(),
            });
            start = byte_col + query_lower.len().max(1);
        }
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_single_match() {
        let hits = search_text("fn main() {\n    let x = 1;\n}\n", "let");
        assert_eq!(hits, vec![SearchHit { line: 1, col: 4, preview: "    let x = 1;".into() }]);
    }

    #[test]
    fn col_indexes_correctly_into_preview() {
        // `preview` must keep leading whitespace so `col` (measured against
        // the original line, for cursor placement) also correctly indexes
        // into `preview` (for highlighting the match within it) — a single
        // offset serving both consumers, not two that happen to agree only
        // when there's no leading whitespace. `col` is a *char* offset (see
        // its doc), so this indexes char-wise, not by byte — the safe way
        // for any consumer, since line content isn't guaranteed ASCII even
        // though query-matching is.
        let hits = search_text("\t\t  let x = 1;", "let");
        let hit = &hits[0];
        let matched: String = hit.preview.chars().skip(hit.col).take(3).collect();
        assert_eq!(matched, "let");
    }

    #[test]
    fn is_case_insensitive() {
        let hits = search_text("Hello World", "world");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].col, 6);
    }

    #[test]
    fn finds_multiple_matches_on_one_line() {
        let hits = search_text("aa aa aa", "aa");
        assert_eq!(hits.len(), 3);
        assert_eq!(hits.iter().map(|h| h.col).collect::<Vec<_>>(), vec![0, 3, 6]);
    }

    #[test]
    fn empty_query_matches_nothing() {
        assert!(search_text("anything", "").is_empty());
    }

    #[test]
    fn no_match_returns_empty() {
        assert!(search_text("hello", "xyz").is_empty());
    }
}
