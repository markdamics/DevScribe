use super::*;

#[test]
fn language_from_extension() {
    assert_eq!(Language::from_extension("rs"), Some(Language::Rust));
    assert_eq!(Language::from_extension("RS"), Some(Language::Rust));
    assert_eq!(Language::from_extension("json"), Some(Language::Json));
    assert_eq!(Language::from_extension("toml"), Some(Language::Toml));
    assert_eq!(Language::from_extension("md"), None);
}

#[test]
fn highlights_rust_keywords_and_types() {
    let mut highlighter = Highlighter::new();
    let source = "fn main() {\n    let x: u32 = 1;\n}\n";
    let spans = highlighter.highlight(Language::Rust, source);

    assert!(!spans.is_empty());
    // Spans are non-overlapping and in document order.
    for pair in spans.windows(2) {
        assert!(pair[0].end <= pair[1].start);
    }

    let fn_span = spans
        .iter()
        .find(|s| &source[s.start..s.end] == "fn")
        .expect("`fn` should be highlighted");
    assert_eq!(fn_span.kind, HighlightKind::Keyword);

    let let_span = spans
        .iter()
        .find(|s| &source[s.start..s.end] == "let")
        .expect("`let` should be highlighted");
    assert_eq!(let_span.kind, HighlightKind::Keyword);
}

#[test]
fn highlights_rust_comment() {
    let mut highlighter = Highlighter::new();
    let source = "// hello\nfn f() {}\n";
    let spans = highlighter.highlight(Language::Rust, source);

    let comment_span = spans
        .iter()
        .find(|s| source[s.start..s.end].starts_with("//"))
        .expect("comment should be highlighted");
    assert_eq!(comment_span.kind, HighlightKind::Comment);
}

#[test]
fn highlights_json_string_and_number() {
    let mut highlighter = Highlighter::new();
    let source = r#"{"a": 1, "b": "two"}"#;
    let spans = highlighter.highlight(Language::Json, source);

    assert!(spans
        .iter()
        .any(|s| s.kind == HighlightKind::String && source[s.start..s.end].contains("two")));
    assert!(spans
        .iter()
        .any(|s| s.kind == HighlightKind::Number && &source[s.start..s.end] == "1"));
}

#[test]
fn highlights_toml_keys_and_strings() {
    let mut highlighter = Highlighter::new();
    let source = "[package]\nname = \"devscribe\"\n";
    let spans = highlighter.highlight(Language::Toml, source);

    assert!(spans
        .iter()
        .any(|s| source[s.start..s.end].contains("devscribe")
            && s.kind == HighlightKind::String));
}
