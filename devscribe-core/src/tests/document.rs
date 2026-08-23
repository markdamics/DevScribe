use super::*;

#[test]
fn insert_marks_dirty_and_updates_text() {
    let mut doc = Document::from_str("hello world");
    assert!(!doc.is_dirty());
    doc.insert(5, ",");
    assert!(doc.is_dirty());
    assert_eq!(doc.text().to_string(), "hello, world");
}

#[test]
fn remove_marks_dirty_and_updates_text() {
    let mut doc = Document::from_str("hello, world");
    doc.remove(5..6);
    assert_eq!(doc.text().to_string(), "hello world");
}

#[test]
fn save_without_path_errors() {
    let mut doc = Document::from_str("no path");
    assert!(doc.save().is_err());
}

#[test]
fn save_writes_buffer_and_clears_dirty() {
    let path = std::env::temp_dir().join(format!("devscribe-core-save-test-{}", std::process::id()));
    std::fs::write(&path, "original").unwrap();

    let mut doc = Document::open(&path).unwrap();
    doc.insert(0, "edited ");
    assert!(doc.is_dirty());

    doc.save().unwrap();
    assert!(!doc.is_dirty());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "edited original");

    std::fs::remove_file(&path).unwrap();
}

#[test]
fn line_count_matches_rope() {
    let doc = Document::from_str("a\nb\nc");
    assert_eq!(doc.line_count(), 3);
}

#[test]
fn line_len_chars_excludes_terminator() {
    let doc = Document::from_str("abc\ndef\r\ngh");
    assert_eq!(doc.line_len_chars(0), 3);
    assert_eq!(doc.line_len_chars(1), 3);
    assert_eq!(doc.line_len_chars(2), 2);
}

#[test]
fn line_text_excludes_terminator() {
    let doc = Document::from_str("abc\ndef\r\ngh");
    assert_eq!(doc.line_text(0), "abc");
    assert_eq!(doc.line_text(1), "def");
    assert_eq!(doc.line_text(2), "gh");
}

#[test]
fn char_index_and_line_col_round_trip() {
    let doc = Document::from_str("abc\ndef\ngh");
    assert_eq!(doc.char_index(1, 2), 6);
    assert_eq!(doc.line_col(6), (1, 2));
    // Column past end-of-line clamps to the line's length.
    assert_eq!(doc.char_index(0, 99), 3);
}
