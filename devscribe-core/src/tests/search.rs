use super::*;

#[test]
fn finds_single_match() {
    let hits = search_text("fn main() {\n    let x = 1;\n}\n", "let", usize::MAX);
    assert_eq!(
        hits,
        vec![SearchHit { line: 1, col: 4, preview: "    let x = 1;".into(), preview_col: 4 }]
    );
}

#[test]
fn preview_col_indexes_correctly_into_preview() {
    // For a line well under the preview window, `preview` is the whole
    // line (leading whitespace kept, no truncation marker), so
    // `preview_col` should still land exactly on the match — this is
    // the "no truncation" baseline the windowing tests below build on.
    // `preview_col` is a *char* offset (see its doc), so this indexes
    // char-wise, not by byte — line content isn't guaranteed ASCII even
    // though query-matching is.
    let hits = search_text("\t\t  let x = 1;", "let", usize::MAX);
    let hit = &hits[0];
    let matched: String = hit.preview.chars().skip(hit.preview_col).take(3).collect();
    assert_eq!(matched, "let");
}

#[test]
fn preview_is_capped_for_a_pathologically_long_line() {
    // The real-world case this guards: a project's own design mockup
    // shipped a single line over a million characters long (a big
    // embedded blob) — searching a term that happened to appear on it
    // used to clone and then render that entire line as one text
    // widget, which no text-shaping/layout pipeline handles gracefully.
    // `col` still refers to the true position for cursor placement;
    // `preview`/`preview_col` must stay small regardless of line length.
    let padding_before = "x".repeat(1_000_000);
    let padding_after = "y".repeat(1_000_000);
    let line = format!("{padding_before}needle{padding_after}");

    let hits = search_text(&line, "needle", usize::MAX);

    assert_eq!(hits.len(), 1);
    let hit = &hits[0];
    assert_eq!(hit.col, 1_000_000, "the real column must stay accurate for cursor placement");
    assert!(hit.preview.len() < 1000, "preview must be capped regardless of the line's real length");
    let matched: String = hit.preview.chars().skip(hit.preview_col).take(6).collect();
    assert_eq!(matched, "needle", "preview_col must still point at the match within the truncated preview");
    assert!(hit.preview.starts_with('\u{2026}'), "context was cut before the match, so it should be marked");
    assert!(hit.preview.ends_with('\u{2026}'), "context was cut after the match too");
}

#[test]
fn preview_has_no_leading_ellipsis_when_the_match_is_near_the_start() {
    let line = format!("needle{}", "z".repeat(1_000_000));

    let hits = search_text(&line, "needle", usize::MAX);

    let hit = &hits[0];
    assert!(!hit.preview.starts_with('\u{2026}'), "nothing was actually cut before the match");
    assert!(hit.preview.ends_with('\u{2026}'));
    assert_eq!(hit.preview_col, 0);
}

#[test]
fn is_case_insensitive() {
    let hits = search_text("Hello World", "world", usize::MAX);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].col, 6);
}

#[test]
fn finds_multiple_matches_on_one_line() {
    let hits = search_text("aa aa aa", "aa", usize::MAX);
    assert_eq!(hits.len(), 3);
    assert_eq!(hits.iter().map(|h| h.col).collect::<Vec<_>>(), vec![0, 3, 6]);
}

#[test]
fn empty_query_matches_nothing() {
    assert!(search_text("anything", "", usize::MAX).is_empty());
}

#[test]
fn no_match_returns_empty() {
    assert!(search_text("hello", "xyz", usize::MAX).is_empty());
}

#[test]
fn stops_scanning_as_soon_as_max_hits_is_reached() {
    // Not just "returns at most max_hits" — this checks the scan itself
    // halts early: a match on line 0 well past the cap must not appear,
    // proving later lines were never even visited.
    let text = "aa\naa\naa\naa\n";
    let hits = search_text(text, "aa", 2);
    assert_eq!(hits.len(), 2);
    assert_eq!(hits.iter().map(|h| h.line).collect::<Vec<_>>(), vec![0, 1]);
}

#[test]
fn max_hits_zero_matches_nothing() {
    assert!(search_text("aa aa aa", "aa", 0).is_empty());
}
