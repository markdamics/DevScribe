use super::*;

#[test]
fn language_from_extension() {
    assert_eq!(Language::from_extension("rs"), Some(Language::Rust));
    assert_eq!(Language::from_extension("RS"), Some(Language::Rust));
    assert_eq!(Language::from_extension("json"), Some(Language::Json));
    assert_eq!(Language::from_extension("toml"), Some(Language::Toml));
    assert_eq!(Language::from_extension("java"), Some(Language::Java));
    assert_eq!(Language::from_extension("py"), Some(Language::Python));
    assert_eq!(Language::from_extension("js"), Some(Language::JavaScript));
    assert_eq!(Language::from_extension("ts"), Some(Language::TypeScript));
    assert_eq!(Language::from_extension("tsx"), Some(Language::TypeScript));
    assert_eq!(Language::from_extension("cpp"), Some(Language::Cpp));
    assert_eq!(Language::from_extension("h"), Some(Language::Cpp));
    assert_eq!(Language::from_extension("yml"), Some(Language::Yaml));
    assert_eq!(Language::from_extension("yaml"), Some(Language::Yaml));
    assert_eq!(Language::from_extension("xml"), Some(Language::Xml));
    assert_eq!(Language::from_extension("svg"), Some(Language::Xml));
    assert_eq!(Language::from_extension("ini"), Some(Language::Ini));
    assert_eq!(Language::from_extension("cfg"), Some(Language::Ini));
    assert_eq!(Language::from_extension("properties"), Some(Language::Ini));
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

#[test]
fn highlights_yaml_strings_and_comments() {
    let mut highlighter = Highlighter::new();
    let source = "# a comment\nname: \"devscribe\"\nenabled: true\n";
    let spans = highlighter.highlight(Language::Yaml, source);

    assert!(spans
        .iter()
        .any(|s| source[s.start..s.end].starts_with('#') && s.kind == HighlightKind::Comment));
    assert!(spans
        .iter()
        .any(|s| source[s.start..s.end].contains("devscribe") && s.kind == HighlightKind::String));
}

#[test]
fn highlights_xml_tags_and_comments() {
    let mut highlighter = Highlighter::new();
    let source = "<!-- hi -->\n<note><to>Tove</to></note>\n";
    let spans = highlighter.highlight(Language::Xml, source);

    assert!(spans
        .iter()
        .any(|s| source[s.start..s.end].contains("hi") && s.kind == HighlightKind::Comment));
    assert!(spans
        .iter()
        .any(|s| &source[s.start..s.end] == "note" && s.kind == HighlightKind::Keyword));
}

#[test]
fn highlights_ini_sections_and_comments() {
    let mut highlighter = Highlighter::new();
    let source = "; a comment\n[section]\nkey = value\n";
    let spans = highlighter.highlight(Language::Ini, source);

    assert!(spans
        .iter()
        .any(|s| source[s.start..s.end].starts_with(';') && s.kind == HighlightKind::Comment));
    assert!(spans
        .iter()
        .any(|s| &source[s.start..s.end] == "section" && s.kind == HighlightKind::Type));
}
