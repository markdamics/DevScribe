use super::*;

#[test]
fn identical_text_is_all_equal() {
    let lines = diff_lines("a\nb\nc\n", "a\nb\nc\n");
    assert!(lines.iter().all(|l| l.kind == DiffLineKind::Equal));
    assert_eq!(lines.len(), 3);
}

#[test]
fn detects_insert_and_delete() {
    let lines = diff_lines("a\nb\nc\n", "a\nx\nc\n");
    let kinds: Vec<_> = lines.iter().map(|l| (l.kind, l.text.as_str())).collect();
    assert_eq!(
        kinds,
        vec![
            (DiffLineKind::Equal, "a"),
            (DiffLineKind::Delete, "b"),
            (DiffLineKind::Insert, "x"),
            (DiffLineKind::Equal, "c"),
        ]
    );
}

#[test]
fn pure_addition_has_no_old_line() {
    let lines = diff_lines("a\n", "a\nb\n");
    let added = lines
        .iter()
        .find(|l| l.kind == DiffLineKind::Insert)
        .expect("insert present");
    assert_eq!(added.old_line, None);
    assert_eq!(added.new_line, Some(1));
}

#[test]
fn gutter_marks_flags_a_pure_insertion_as_added() {
    let lines = diff_lines("a\n", "a\nb\n");
    let marks = gutter_marks(&lines, 2);
    assert_eq!(marks, vec![None, Some(GutterMark::Added)]);
}

#[test]
fn gutter_marks_flags_a_one_for_one_replace_as_modified() {
    let lines = diff_lines("a\nb\nc\n", "a\nx\nc\n");
    let marks = gutter_marks(&lines, 3);
    assert_eq!(
        marks,
        vec![
            None,
            Some(GutterMark::Modified { head_text: "b".to_string() }),
            None,
        ]
    );
}

#[test]
fn gutter_marks_flags_a_mid_file_deletion_as_removed_above_the_next_line() {
    let lines = diff_lines("a\nb\nc\n", "a\nc\n");
    let marks = gutter_marks(&lines, 2);
    assert_eq!(
        marks,
        vec![
            None,
            Some(GutterMark::RemovedAbove { head_lines: vec!["b".to_string()] }),
        ]
    );
}

#[test]
fn gutter_marks_flags_an_end_of_file_deletion_on_the_last_line() {
    let lines = diff_lines("a\nb\nc\n", "a\n");
    let marks = gutter_marks(&lines, 1);
    assert_eq!(
        marks,
        vec![Some(GutterMark::RemovedAbove {
            head_lines: vec!["b".to_string(), "c".to_string()]
        })]
    );
}

#[test]
fn gutter_marks_prefers_removed_above_when_a_replace_has_more_deletes_than_inserts() {
    let lines = diff_lines("a\nb\nc\nd\n", "a\nx\nd\n");
    let marks = gutter_marks(&lines, 3);
    assert_eq!(
        marks,
        vec![
            None,
            Some(GutterMark::RemovedAbove { head_lines: vec!["c".to_string()] }),
            None,
        ]
    );
}

#[test]
fn gutter_marks_flags_excess_inserts_as_added_when_a_replace_has_more_inserts_than_deletes() {
    let lines = diff_lines("a\nb\nd\n", "a\nx\ny\nd\n");
    let marks = gutter_marks(&lines, 4);
    assert_eq!(
        marks,
        vec![
            None,
            Some(GutterMark::Modified { head_text: "b".to_string() }),
            Some(GutterMark::Added),
            None,
        ]
    );
}

#[test]
fn hunks_groups_two_separate_replaces_into_two_hunks() {
    let lines = diff_lines("a\nb\nc\nd\ne\n", "a\nx\nc\ny\ne\n");
    let found = hunks(&lines, 5);
    assert_eq!(found.len(), 2);
    assert_eq!(
        found[0].marks,
        vec![(1, GutterMark::Modified { head_text: "b".to_string() })]
    );
    assert_eq!(
        found[1].marks,
        vec![(3, GutterMark::Modified { head_text: "d".to_string() })]
    );
}

#[test]
fn hunk_range_spans_every_row_the_hunk_contributes() {
    let lines = diff_lines("a\nb\nc\nd\n", "a\nx\nd\n");
    let found = hunks(&lines, 3);
    assert_eq!(found.len(), 1);
    let hunk = &found[0];
    assert!(lines[hunk.range.clone()]
        .iter()
        .all(|l| l.kind != DiffLineKind::Equal));
    assert_eq!(
        hunk.marks,
        vec![(1, GutterMark::RemovedAbove { head_lines: vec!["c".to_string()] })]
    );
}

#[test]
fn ignoring_whitespace_treats_a_reindented_line_as_equal() {
    let lines = diff_lines_ignoring_whitespace("a\n  b\nc\n", "a\n\tb  \nc\n");
    assert!(lines.iter().all(|l| l.kind == DiffLineKind::Equal), "{lines:?}");
}

#[test]
fn ignoring_whitespace_still_catches_a_real_content_change() {
    let lines = diff_lines_ignoring_whitespace("a\n  b\nc\n", "a\n  x\nc\n");
    let kinds: Vec<_> = lines.iter().map(|l| (l.kind, l.text.as_str())).collect();
    assert_eq!(
        kinds,
        vec![
            (DiffLineKind::Equal, "a"),
            (DiffLineKind::Delete, "  b"),
            (DiffLineKind::Insert, "  x"),
            (DiffLineKind::Equal, "c"),
        ]
    );
}

#[test]
fn exact_mode_still_flags_a_whitespace_only_change_that_ignore_mode_would_hide() {
    let lines = diff_lines("a\n  b\nc\n", "a\n\tb\nc\n");
    assert!(lines.iter().any(|l| l.kind != DiffLineKind::Equal), "{lines:?}");
}

#[test]
fn diff_words_flags_only_the_changed_word_not_the_whole_line() {
    let spans = diff_words("the quick fox", "the slow fox");
    let tagged: Vec<_> = spans.iter().map(|s| (s.kind, s.text.as_str())).collect();
    assert_eq!(
        tagged,
        vec![
            (DiffLineKind::Equal, "the"),
            (DiffLineKind::Equal, " "),
            (DiffLineKind::Delete, "quick"),
            (DiffLineKind::Insert, "slow"),
            (DiffLineKind::Equal, " "),
            (DiffLineKind::Equal, "fox"),
        ]
    );
}

#[test]
fn diff_words_of_identical_lines_is_all_equal() {
    let spans = diff_words("same text", "same text");
    assert!(spans.iter().all(|s| s.kind == DiffLineKind::Equal));
}

#[test]
fn hunks_and_gutter_marks_agree() {
    let lines = diff_lines("a\nb\nc\nd\ne\n", "a\nx\ny\nz\ne\n");
    let from_hunks = {
        let mut marks = vec![None; 5];
        for hunk in hunks(&lines, 5) {
            for (new_line, mark) in hunk.marks {
                marks[new_line] = Some(mark);
            }
        }
        marks
    };
    assert_eq!(from_hunks, gutter_marks(&lines, 5));
}
