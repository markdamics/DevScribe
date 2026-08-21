//! Tree-sitter-backed syntax highlighting. Reparses the whole document on
//! every edit rather than feeding `tree_sitter::Tree::edit` incrementally —
//! tree-sitter is fast enough that this is invisible at the file sizes an
//! editor actually opens, and it avoids the extra bookkeeping of tracking
//! `InputEdit`s alongside every `Document` mutation. True incremental
//! reparsing is a well-scoped follow-up if profiling ever shows this matters.
use std::sync::OnceLock;

use tree_sitter_highlight::{
    Highlight, HighlightConfiguration, HighlightEvent, Highlighter as TsHighlighter,
};

/// A small, editor-agnostic palette of syntax categories. Deliberately coarse
/// (not the full `nvim-treesitter` taxonomy) — this is everything the editor
/// canvas actually has a distinct color for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HighlightKind {
    Default,
    Keyword,
    Type,
    Function,
    Macro,
    String,
    Number,
    Comment,
    Constant,
    Attribute,
    Punctuation,
}

/// A language this crate knows how to highlight, keyed off file extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Rust,
    Json,
    Toml,
}

impl Language {
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "rs" => Some(Language::Rust),
            "json" => Some(Language::Json),
            "toml" => Some(Language::Toml),
            _ => None,
        }
    }
}

/// A highlighted region of the document, in byte offsets (tree-sitter's
/// native unit). Non-overlapping and in document order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub kind: HighlightKind,
}

/// The capture names every `HighlightConfiguration` below is `configure`d
/// with; a `Highlight(i)` tree-sitter-highlight returns is an index into
/// this slice, so `HIGHLIGHT_KINDS[i]` (same order) is the resolved kind.
/// `configure`'s dot-part subset matching means e.g. a `type.builtin`
/// capture still matches the `"type"` entry even though it's not listed
/// verbatim, so this list stays short.
const HIGHLIGHT_NAMES: &[&str] = &[
    "attribute",
    "comment",
    "constant",
    "constant.builtin",
    "function",
    "function.macro",
    "keyword",
    "number",
    "operator",
    "property",
    "punctuation",
    "string",
    "string.escape",
    "type",
    "variable",
];

const HIGHLIGHT_KINDS: &[HighlightKind] = &[
    HighlightKind::Attribute,
    HighlightKind::Comment,
    HighlightKind::Constant,
    HighlightKind::Constant,
    HighlightKind::Function,
    HighlightKind::Macro,
    HighlightKind::Keyword,
    HighlightKind::Number,
    HighlightKind::Punctuation,
    HighlightKind::Default,
    HighlightKind::Punctuation,
    HighlightKind::String,
    HighlightKind::String,
    HighlightKind::Type,
    HighlightKind::Default,
];

fn rust_config() -> &'static HighlightConfiguration {
    static CONFIG: OnceLock<HighlightConfiguration> = OnceLock::new();
    CONFIG.get_or_init(|| {
        let mut config = HighlightConfiguration::new(
            tree_sitter_rust::LANGUAGE.into(),
            "rust",
            tree_sitter_rust::HIGHLIGHTS_QUERY,
            tree_sitter_rust::INJECTIONS_QUERY,
            "",
        )
        .expect("tree-sitter-rust ships a valid highlights.scm");
        config.configure(HIGHLIGHT_NAMES);
        config
    })
}

fn json_config() -> &'static HighlightConfiguration {
    static CONFIG: OnceLock<HighlightConfiguration> = OnceLock::new();
    CONFIG.get_or_init(|| {
        let mut config = HighlightConfiguration::new(
            tree_sitter_json::LANGUAGE.into(),
            "json",
            tree_sitter_json::HIGHLIGHTS_QUERY,
            "",
            "",
        )
        .expect("tree-sitter-json ships a valid highlights.scm");
        config.configure(HIGHLIGHT_NAMES);
        config
    })
}

fn toml_config() -> &'static HighlightConfiguration {
    static CONFIG: OnceLock<HighlightConfiguration> = OnceLock::new();
    CONFIG.get_or_init(|| {
        let mut config = HighlightConfiguration::new(
            tree_sitter_toml_ng::LANGUAGE.into(),
            "toml",
            tree_sitter_toml_ng::HIGHLIGHTS_QUERY,
            "",
            "",
        )
        .expect("tree-sitter-toml-ng ships a valid highlights.scm");
        config.configure(HIGHLIGHT_NAMES);
        config
    })
}

fn config_for(language: Language) -> &'static HighlightConfiguration {
    match language {
        Language::Rust => rust_config(),
        Language::Json => json_config(),
        Language::Toml => toml_config(),
    }
}

/// Wraps a `tree-sitter-highlight` highlighter. Reused across calls (the
/// crate's own docs recommend this for performance — it holds the parser).
pub struct Highlighter {
    inner: TsHighlighter,
}

impl Highlighter {
    pub fn new() -> Self {
        Self {
            inner: TsHighlighter::new(),
        }
    }

    /// Highlights `source` from scratch, returning non-overlapping,
    /// document-ordered spans. Returns an empty vec if the source has a
    /// parse/query error tree-sitter can't recover from.
    pub fn highlight(&mut self, language: Language, source: &str) -> Vec<Span> {
        let config = config_for(language);
        let Ok(events) = self
            .inner
            .highlight(config, source.as_bytes(), None, |_| None)
        else {
            return Vec::new();
        };

        let mut spans = Vec::new();
        let mut stack: Vec<HighlightKind> = Vec::new();

        for event in events {
            match event {
                Ok(HighlightEvent::Source { start, end }) => {
                    if start < end {
                        let kind = stack.last().copied().unwrap_or(HighlightKind::Default);
                        spans.push(Span { start, end, kind });
                    }
                }
                Ok(HighlightEvent::HighlightStart(Highlight(i))) => {
                    stack.push(
                        HIGHLIGHT_KINDS
                            .get(i)
                            .copied()
                            .unwrap_or(HighlightKind::Default),
                    );
                }
                Ok(HighlightEvent::HighlightEnd) => {
                    stack.pop();
                }
                Err(_) => break,
            }
        }

        spans
    }
}

impl Default for Highlighter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
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
}
