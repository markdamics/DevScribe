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
