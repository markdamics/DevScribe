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
    Java,
    Python,
    JavaScript,
    TypeScript,
    Tsx,
    Cpp,
    Yaml,
    Xml,
    Ini,
    Markdown,
}

impl Language {
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "rs" => Some(Language::Rust),
            "json" => Some(Language::Json),
            "toml" => Some(Language::Toml),
            "java" => Some(Language::Java),
            "py" | "pyi" => Some(Language::Python),
            "js" | "mjs" | "cjs" => Some(Language::JavaScript),
            "ts" | "mts" | "cts" => Some(Language::TypeScript),
            "tsx" => Some(Language::Tsx),
            "cpp" | "cc" | "cxx" | "c" | "h" | "hpp" | "hxx" => Some(Language::Cpp),
            "yml" | "yaml" => Some(Language::Yaml),
            "xml" | "svg" | "xsd" | "xsl" | "xslt" | "plist" => Some(Language::Xml),
            "ini" | "cfg" | "properties" => Some(Language::Ini),
            "md" | "markdown" => Some(Language::Markdown),
            _ => None,
        }
    }

    /// This language's single-line comment token, for `Ctrl+/` toggle-comment
    /// — `None` for a language with no such thing (`Json` has no comment
    /// syntax at all; `Xml`/`Markdown` only have block comments, which toggle
    /// differently and aren't wired up).
    pub fn line_comment(self) -> Option<&'static str> {
        match self {
            Language::Rust
            | Language::Java
            | Language::JavaScript
            | Language::TypeScript
            | Language::Tsx
            | Language::Cpp => Some("//"),
            Language::Python | Language::Toml | Language::Yaml | Language::Ini => Some("#"),
            Language::Json | Language::Xml | Language::Markdown => None,
        }
    }

    /// Display name for the status bar's language indicator.
    pub fn label(self) -> &'static str {
        match self {
            Language::Rust => "Rust",
            Language::Json => "JSON",
            Language::Toml => "TOML",
            Language::Java => "Java",
            Language::Python => "Python",
            Language::JavaScript => "JavaScript",
            Language::TypeScript => "TypeScript",
            Language::Tsx => "TSX",
            Language::Cpp => "C/C++",
            Language::Yaml => "YAML",
            Language::Xml => "XML",
            Language::Ini => "INI",
            Language::Markdown => "Markdown",
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
    "boolean",
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
    "tag",
    "type",
    "variable",
    "text.title",
    "text.literal",
    "text.uri",
    "text.reference",
    "text.strong",
    "text.emphasis",
];

const HIGHLIGHT_KINDS: &[HighlightKind] = &[
    HighlightKind::Attribute,
    HighlightKind::Constant,
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
    HighlightKind::Keyword,
    HighlightKind::Type,
    HighlightKind::Default,
    // Markdown-specific (nvim-treesitter-style capture names used by
    // tree-sitter-md's shipped queries — a different taxonomy than every
    // other language here, which is why these live at the end).
    HighlightKind::Keyword,   // text.title (headings)
    HighlightKind::String,    // text.literal (code spans / fenced code)
    HighlightKind::Constant,  // text.uri (link/image destinations)
    HighlightKind::Attribute, // text.reference (link labels/text)
    HighlightKind::Function,  // text.strong (bold)
    HighlightKind::Attribute, // text.emphasis (italic)
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

fn java_config() -> &'static HighlightConfiguration {
    static CONFIG: OnceLock<HighlightConfiguration> = OnceLock::new();
    CONFIG.get_or_init(|| {
        let mut config = HighlightConfiguration::new(
            tree_sitter_java::LANGUAGE.into(),
            "java",
            tree_sitter_java::HIGHLIGHTS_QUERY,
            "",
            "",
        )
        .expect("tree-sitter-java ships a valid highlights.scm");
        config.configure(HIGHLIGHT_NAMES);
        config
    })
}

fn python_config() -> &'static HighlightConfiguration {
    static CONFIG: OnceLock<HighlightConfiguration> = OnceLock::new();
    CONFIG.get_or_init(|| {
        let mut config = HighlightConfiguration::new(
            tree_sitter_python::LANGUAGE.into(),
            "python",
            tree_sitter_python::HIGHLIGHTS_QUERY,
            "",
            "",
        )
        .expect("tree-sitter-python ships a valid highlights.scm");
        config.configure(HIGHLIGHT_NAMES);
        config
    })
}

fn javascript_config() -> &'static HighlightConfiguration {
    static CONFIG: OnceLock<HighlightConfiguration> = OnceLock::new();
    CONFIG.get_or_init(|| {
        let mut config = HighlightConfiguration::new(
            tree_sitter_javascript::LANGUAGE.into(),
            "javascript",
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_javascript::INJECTIONS_QUERY,
            tree_sitter_javascript::LOCALS_QUERY,
        )
        .expect("tree-sitter-javascript ships a valid highlights.scm");
        config.configure(HIGHLIGHT_NAMES);
        config
    })
}

/// TypeScript's own `HIGHLIGHTS_QUERY` only adds TS-specific captures (type
/// annotations, TS-only keywords like `interface`/`enum`) — the TypeScript
/// and TSX grammars are both JavaScript supersets that reuse its node types
/// for everything else (strings, comments, functions, `let`/`const`/`if`...),
/// so the JS query has to be layered underneath or all of that goes
/// uncolored. Mirrors how nvim-treesitter's own `typescript` query declares
/// `; inherits: ecma`.
fn ts_base_highlights_query() -> String {
    format!(
        "{}\n{}",
        tree_sitter_javascript::HIGHLIGHT_QUERY,
        tree_sitter_typescript::HIGHLIGHTS_QUERY
    )
}

/// Tag/attribute captures for JSX nodes. Neither the JavaScript nor the
/// TypeScript crate ships a JSX-aware `highlights.scm` (JSX support is
/// purely grammar-level for them), so `.tsx` needs its own query on top of
/// `ts_base_highlights_query` to get tag names and prop names colored
/// instead of falling through to plain text.
const JSX_HIGHLIGHTS_QUERY: &str = r#"
(jsx_opening_element name: [(identifier) (member_expression)] @tag)
(jsx_closing_element name: [(identifier) (member_expression)] @tag)
(jsx_self_closing_element name: [(identifier) (member_expression)] @tag)
(jsx_attribute (property_identifier) @attribute)
"#;

fn typescript_config() -> &'static HighlightConfiguration {
    static CONFIG: OnceLock<HighlightConfiguration> = OnceLock::new();
    CONFIG.get_or_init(|| {
        let mut config = HighlightConfiguration::new(
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            "typescript",
            &ts_base_highlights_query(),
            tree_sitter_javascript::INJECTIONS_QUERY,
            tree_sitter_typescript::LOCALS_QUERY,
        )
        .expect("tree-sitter-typescript ships a valid highlights.scm");
        config.configure(HIGHLIGHT_NAMES);
        config
    })
}

fn tsx_config() -> &'static HighlightConfiguration {
    static CONFIG: OnceLock<HighlightConfiguration> = OnceLock::new();
    CONFIG.get_or_init(|| {
        let highlights_query = format!("{}\n{}", ts_base_highlights_query(), JSX_HIGHLIGHTS_QUERY);
        let mut config = HighlightConfiguration::new(
            tree_sitter_typescript::LANGUAGE_TSX.into(),
            "tsx",
            &highlights_query,
            tree_sitter_javascript::INJECTIONS_QUERY,
            tree_sitter_typescript::LOCALS_QUERY,
        )
        .expect("tree-sitter-typescript ships a valid highlights.scm");
        config.configure(HIGHLIGHT_NAMES);
        config
    })
}

fn cpp_config() -> &'static HighlightConfiguration {
    static CONFIG: OnceLock<HighlightConfiguration> = OnceLock::new();
    CONFIG.get_or_init(|| {
        let mut config = HighlightConfiguration::new(
            tree_sitter_cpp::LANGUAGE.into(),
            "cpp",
            tree_sitter_cpp::HIGHLIGHT_QUERY,
            "",
            "",
        )
        .expect("tree-sitter-cpp ships a valid highlights.scm");
        config.configure(HIGHLIGHT_NAMES);
        config
    })
}

fn yaml_config() -> &'static HighlightConfiguration {
    static CONFIG: OnceLock<HighlightConfiguration> = OnceLock::new();
    CONFIG.get_or_init(|| {
        let mut config = HighlightConfiguration::new(
            tree_sitter_yaml::LANGUAGE.into(),
            "yaml",
            tree_sitter_yaml::HIGHLIGHTS_QUERY,
            "",
            "",
        )
        .expect("tree-sitter-yaml ships a valid highlights.scm");
        config.configure(HIGHLIGHT_NAMES);
        config
    })
}

fn xml_config() -> &'static HighlightConfiguration {
    static CONFIG: OnceLock<HighlightConfiguration> = OnceLock::new();
    CONFIG.get_or_init(|| {
        let mut config = HighlightConfiguration::new(
            tree_sitter_xml::LANGUAGE_XML.into(),
            "xml",
            tree_sitter_xml::XML_HIGHLIGHT_QUERY,
            "",
            "",
        )
        .expect("tree-sitter-xml ships a valid highlights.scm");
        config.configure(HIGHLIGHT_NAMES);
        config
    })
}

fn ini_config() -> &'static HighlightConfiguration {
    static CONFIG: OnceLock<HighlightConfiguration> = OnceLock::new();
    CONFIG.get_or_init(|| {
        // tree-sitter-ini's shipped query tags `(comment)` with both
        // `@comment` and `@spell`; the second, unrecognized capture on the
        // same node makes tree-sitter-highlight drop the highlight for that
        // node entirely rather than falling back to the first. We don't use
        // `@spell` for anything, so strip it before compiling.
        let highlights_query = tree_sitter_ini::HIGHLIGHTS_QUERY.replace(" @spell", "");
        let mut config = HighlightConfiguration::new(
            tree_sitter_ini::LANGUAGE.into(),
            "ini",
            &highlights_query,
            "",
            "",
        )
        .expect("tree-sitter-ini ships a valid highlights.scm");
        config.configure(HIGHLIGHT_NAMES);
        config
    })
}

/// Markdown's grammar is split into a block grammar (headings, lists, code
/// fences, blockquotes) and a separate inline grammar (bold, italic, inline
/// code, links) that the block grammar's own query pulls in via a language
/// injection named `"markdown_inline"` — see the injection callback in
/// `Highlighter::highlight` below, which is what actually resolves it.
fn markdown_config() -> &'static HighlightConfiguration {
    static CONFIG: OnceLock<HighlightConfiguration> = OnceLock::new();
    CONFIG.get_or_init(|| {
        // The block grammar's own tokenizer still emits each inline
        // delimiter (`` ` ``, `*`, etc.) as an anonymous child of the
        // `(inline)` node it hands off for injection. Without
        // `injection.include-children`, tree-sitter-highlight's default
        // "exclude the content node's children" behavior excises exactly
        // those delimiter bytes from the range it re-parses with the inline
        // grammar — which then can't see its own delimiters and silently
        // produces no structure at all (no bold/italic/code-span captures,
        // just inert plain text). Patching in the directive here fixes it;
        // see `ini_config` for the same "shipped query needs a tweak"
        // pattern applied to a different crate's quirk.
        let injections_query = tree_sitter_md::INJECTION_QUERY_BLOCK.replace(
            "(#set! injection.language \"markdown_inline\"))",
            "(#set! injection.language \"markdown_inline\")\n  (#set! injection.include-children))",
        );
        let mut config = HighlightConfiguration::new(
            tree_sitter_md::LANGUAGE.into(),
            "markdown",
            tree_sitter_md::HIGHLIGHT_QUERY_BLOCK,
            &injections_query,
            "",
        )
        .expect("tree-sitter-md ships a valid block highlights.scm");
        config.configure(HIGHLIGHT_NAMES);
        config
    })
}

fn markdown_inline_config() -> &'static HighlightConfiguration {
    static CONFIG: OnceLock<HighlightConfiguration> = OnceLock::new();
    CONFIG.get_or_init(|| {
        let mut config = HighlightConfiguration::new(
            tree_sitter_md::INLINE_LANGUAGE.into(),
            "markdown_inline",
            tree_sitter_md::HIGHLIGHT_QUERY_INLINE,
            tree_sitter_md::INJECTION_QUERY_INLINE,
            "",
        )
        .expect("tree-sitter-md ships a valid inline highlights.scm");
        config.configure(HIGHLIGHT_NAMES);
        config
    })
}

fn config_for(language: Language) -> &'static HighlightConfiguration {
    match language {
        Language::Rust => rust_config(),
        Language::Json => json_config(),
        Language::Toml => toml_config(),
        Language::Java => java_config(),
        Language::Python => python_config(),
        Language::JavaScript => javascript_config(),
        Language::TypeScript => typescript_config(),
        Language::Tsx => tsx_config(),
        Language::Cpp => cpp_config(),
        Language::Yaml => yaml_config(),
        Language::Xml => xml_config(),
        Language::Ini => ini_config(),
        Language::Markdown => markdown_config(),
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
        let Ok(events) = self.inner.highlight(config, source.as_bytes(), None, |name| {
            (name == "markdown_inline").then(markdown_inline_config)
        }) else {
            return Vec::new();
        };

        let mut spans: Vec<Span> = Vec::new();
        let mut stack: Vec<HighlightKind> = Vec::new();

        for event in events {
            match event {
                Ok(HighlightEvent::Source { start, end }) => {
                    if start < end {
                        let kind = stack.last().copied().unwrap_or(HighlightKind::Default);
                        // Coalesce with the previous span when it is the same
                        // kind and directly abutting. tree-sitter emits a
                        // `Source` event per token boundary, so an ordinary
                        // run of unhighlighted code arrives as dozens of
                        // separate `Default` spans. Rendering is identical
                        // either way, but `editor_canvas` issues one
                        // `fill_text` — one text-shaping run — per span, on
                        // every frame.
                        match spans.last_mut() {
                            Some(last) if last.kind == kind && last.end == start => {
                                last.end = end;
                            }
                            _ => spans.push(Span { start, end, kind }),
                        }
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
#[path = "tests/syntax.rs"]
mod tests;
