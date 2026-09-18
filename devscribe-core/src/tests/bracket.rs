use super::*;

#[test]
fn matches_the_open_paren_when_the_cursor_sits_right_before_it() {
    let rope = Rope::from_str("fn f(x: i32) {}");
    let pair = matching_bracket_pair(&rope, &[], 4);
    assert_eq!(pair, Some((4, 11)));
}

#[test]
fn matches_the_close_paren_when_the_cursor_sits_right_after_it() {
    let rope = Rope::from_str("fn f(x: i32) {}");
    // Cursor at char 12 is *after* the ')' at index 11 — the caret landing
    // spot right after typing/clicking past a closing bracket.
    let pair = matching_bracket_pair(&rope, &[], 12);
    assert_eq!(pair, Some((4, 11)));
}

#[test]
fn skips_a_nested_pair_to_find_the_outer_match() {
    let rope = Rope::from_str("(a[b(c)]d)");
    let pair = matching_bracket_pair(&rope, &[], 0);
    assert_eq!(pair, Some((0, 9)));
}

#[test]
fn matches_square_and_curly_brackets_too() {
    let rope = Rope::from_str("[1, 2, {3: 4}]");
    assert_eq!(matching_bracket_pair(&rope, &[], 0), Some((0, 13)));
    assert_eq!(matching_bracket_pair(&rope, &[], 7), Some((7, 12)));
}

#[test]
fn returns_none_for_an_unmatched_bracket() {
    let rope = Rope::from_str("fn f(x: i32 {}");
    assert_eq!(matching_bracket_pair(&rope, &[], 4), None);
}

#[test]
fn returns_none_when_the_cursor_touches_no_bracket_at_all() {
    let rope = Rope::from_str("let x = 1;");
    assert_eq!(matching_bracket_pair(&rope, &[], 4), None);
}

#[test]
fn ignores_a_bracket_that_is_inside_a_string_span() {
    // `"(fake)"` — a real `(` earlier in the line has no partner *outside*
    // the string, so without string-awareness this would (wrongly) match
    // the real `(` to the fake `)` inside the string literal.
    let text = r#"(a "(fake)")"#;
    let rope = Rope::from_str(text);
    let string_start = text.find('"').unwrap();
    let string_end = text.rfind('"').unwrap() + 1;
    let highlights = [Span { start: string_start, end: string_end, kind: HighlightKind::String }];

    assert_eq!(
        matching_bracket_pair(&rope, &highlights, 0),
        Some((0, text.len() - 1)),
        "the outer parens must match each other, not the fake pair inside the string"
    );
    assert_eq!(
        matching_bracket_pair(&rope, &highlights, string_start + 1),
        None,
        "a bracket inside a string must not match anything at all"
    );
}

#[test]
fn ignores_a_bracket_that_is_inside_a_comment_span() {
    let text = "f(x) // (comment)";
    let rope = Rope::from_str(text);
    let comment_start = text.find("//").unwrap();
    let highlights = [Span { start: comment_start, end: text.len(), kind: HighlightKind::Comment }];

    assert_eq!(matching_bracket_pair(&rope, &highlights, 1), Some((1, 3)));
    assert_eq!(matching_bracket_pair(&rope, &highlights, comment_start + 3), None);
}

#[test]
fn a_scan_past_max_bracket_scan_gives_up_rather_than_walking_the_whole_document() {
    let text = format!("({}", " ".repeat(MAX_BRACKET_SCAN + 10));
    let rope = Rope::from_str(&text);
    assert_eq!(matching_bracket_pair(&rope, &[], 0), None);
}

#[test]
fn bracket_depths_shares_one_counter_across_bracket_kinds() {
    // `(a[b]c)` — the outer parens are depth 1, the square brackets nested
    // one level inside them are depth 2.
    let rope = Rope::from_str("(a[b]c)");
    let depths = bracket_depths(&rope, &[]);
    assert_eq!(
        depths,
        vec![
            BracketDepth { byte_idx: 0, depth: 1 }, // (
            BracketDepth { byte_idx: 2, depth: 2 }, // [
            BracketDepth { byte_idx: 4, depth: 2 }, // ]
            BracketDepth { byte_idx: 6, depth: 1 }, // )
        ]
    );
}

#[test]
fn bracket_depths_gives_an_opener_and_its_closer_the_same_depth() {
    let rope = Rope::from_str("((()))");
    let depths = bracket_depths(&rope, &[]);
    let by_idx: Vec<u32> = depths.iter().map(|b| b.depth).collect();
    assert_eq!(by_idx, vec![1, 2, 3, 3, 2, 1]);
}

#[test]
fn bracket_depths_floors_an_unmatched_closer_at_depth_one() {
    let rope = Rope::from_str(")");
    let depths = bracket_depths(&rope, &[]);
    assert_eq!(depths, vec![BracketDepth { byte_idx: 0, depth: 1 }]);
}

#[test]
fn bracket_depths_skips_brackets_inside_strings_and_comments() {
    let text = r#"(a "(b)" c) // (d)"#;
    let rope = Rope::from_str(text);
    let string_start = text.find('"').unwrap();
    let string_end = text.rfind('"').unwrap() + 1;
    let comment_start = text.find("//").unwrap();
    let highlights = [
        Span { start: string_start, end: string_end, kind: HighlightKind::String },
        Span { start: comment_start, end: text.len(), kind: HighlightKind::Comment },
    ];
    let depths = bracket_depths(&rope, &highlights);
    // Only the real outer `(` and `)` should be picked up.
    assert_eq!(depths.len(), 2);
    assert_eq!(depths[0].depth, 1);
    assert_eq!(depths[1].depth, 1);
}
