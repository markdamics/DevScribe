//! Cursor breadcrumbs: the stack of enclosing named/control-flow scopes at
//! a byte offset — e.g. `ledger::engine › settle_batch › for (id, amount)
//! in delta`, ported from `DevScribe.dc.html`'s breadcrumb strip.
//!
//! This runs its own `tree_sitter::Parser` rather than reusing
//! `syntax::Highlighter`: `tree_sitter_highlight::Highlighter::highlight`
//! parses internally but only ever streams out highlight events, never the
//! `Tree` it built to produce them — there's no accessor for it. Landmark
//! extraction needs the real tree (ancestors, named fields), which
//! highlighting doesn't.
//!
//! Two different costs, kept apart on purpose:
//!
//! - **Parsing** (`parse`) happens once per edit-settle, alongside
//!   `rehighlight_with` — see `EditorState::reparse_now`. It's a second
//!   full-document parse on top of the highlighter's own, so it roughly
//!   doubles the settle-time syntax cost, but that cost was already moved
//!   off the keystroke path; see `devscribe/src/state.rs`'s `EDIT_SETTLE`
//!   doc for why a per-keystroke reparse is the thing that must never
//!   happen again.
//! - **Walking** the already-parsed tree for a new cursor position
//!   (`breadcrumbs_at`) is cheap regardless of file size: it only reads the
//!   handful of small byte ranges the matched landmark nodes cover, via
//!   `Rope::get_byte_slice`, never the whole buffer. This is what runs on
//!   every `view()` — every cursor move, every caret blink.

use ropey::Rope;
use tree_sitter::{Node, Parser};

use crate::syntax::Language;

pub use tree_sitter::Tree;

/// What kind of scope a breadcrumb segment names — drives its glyph and
/// accent in the UI layer, kept out of this crate (no windowing/color deps
/// here, same reasoning as `theme::Rgba` staying a plain float struct).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrumbKind {
    Module,
    Type,
    Function,
    Closure,
    Loop,
    Conditional,
    Match,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crumb {
    pub kind: CrumbKind,
    pub label: String,
}

/// Cap on a control-flow crumb's label — these have no real name to key
/// off, so the breadcrumb shows the node's own leading source text (e.g.
/// `for (id, amount) in delta`) instead. Same reasoning and rough budget as
/// `search`'s `PREVIEW_CONTEXT_CHARS`: keeps a pathological one-line `if`
/// chain from producing a breadcrumb that's a screen-width string.
const MAX_HEADER_CHARS: usize = 48;

/// How a landmark node's label is read off the tree.
enum LabelRule {
    /// The named field's own source text, verbatim — a real identifier.
    Field(&'static str),
    /// Rust `impl_item`: `{trait} for {type}`, or just `{type}` when
    /// there's no `trait` field (an inherent impl). Its own rule because
    /// `impl` has no `name` field at all — `type`/`trait` are the only
    /// named children.
    RustImpl,
    /// C++ `function_definition`: the name lives inside `declarator`
    /// (`function_declarator` wrapping a plain, pointer-qualified, or
    /// scope-qualified identifier) — see `cpp_declarator_name`.
    CppFunctionName,
    /// No real name: the node's own source text from its start up to (not
    /// including) the named field that opens its body, trimmed to one line
    /// and capped at `MAX_HEADER_CHARS`. Covers every control-flow landmark
    /// (`for (id, amount) in delta`, `if err.is_some()`, `match op`) and
    /// anonymous closures/lambdas (`|id, amount|`, `(id, amount) =>`).
    HeaderUpTo(&'static str),
}

struct Landmark {
    node_kind: &'static str,
    kind: CrumbKind,
    label: LabelRule,
}

macro_rules! landmarks {
    ($($node_kind:literal => $kind:ident, $label:expr;)*) => {
        &[$(Landmark { node_kind: $node_kind, kind: CrumbKind::$kind, label: $label }),*]
    };
}

use LabelRule::*;

/// Verified against each grammar's own `src/node-types.json` (field names
/// vary in ways guessing would get wrong — e.g. Rust `impl_item` has no
/// `name` field, C++ `function_definition`'s name is buried in
/// `declarator`, and `if`-family nodes anchor on `consequence` rather than
/// `body`).
const RUST_LANDMARKS: &[Landmark] = landmarks! {
    "mod_item" => Module, Field("name");
    "struct_item" => Type, Field("name");
    "enum_item" => Type, Field("name");
    "trait_item" => Type, Field("name");
    "impl_item" => Type, RustImpl;
    "function_item" => Function, Field("name");
    "closure_expression" => Closure, HeaderUpTo("body");
    "for_expression" => Loop, HeaderUpTo("body");
    "while_expression" => Loop, HeaderUpTo("body");
    "loop_expression" => Loop, HeaderUpTo("body");
    "if_expression" => Conditional, HeaderUpTo("consequence");
    "match_expression" => Match, HeaderUpTo("body");
};

const PYTHON_LANDMARKS: &[Landmark] = landmarks! {
    "class_definition" => Type, Field("name");
    "function_definition" => Function, Field("name");
    "for_statement" => Loop, HeaderUpTo("body");
    "while_statement" => Loop, HeaderUpTo("body");
    "if_statement" => Conditional, HeaderUpTo("consequence");
};

/// Shared by JavaScript and TypeScript: their node kinds and field names
/// line up exactly for everything here (checked against both grammars'
/// `node-types.json`). A kind absent from plain JS's grammar (there is
/// none among these — `interface_declaration` is TS-only but simply never
/// matches when parsing JS) costs nothing.
const JS_TS_LANDMARKS: &[Landmark] = landmarks! {
    "class_declaration" => Type, Field("name");
    "abstract_class_declaration" => Type, Field("name");
    "interface_declaration" => Type, Field("name");
    "function_declaration" => Function, Field("name");
    "function_expression" => Function, Field("name");
    "method_definition" => Function, Field("name");
    "arrow_function" => Closure, HeaderUpTo("body");
    "for_statement" => Loop, HeaderUpTo("body");
    "for_in_statement" => Loop, HeaderUpTo("body");
    "while_statement" => Loop, HeaderUpTo("body");
    "if_statement" => Conditional, HeaderUpTo("consequence");
    "switch_statement" => Match, HeaderUpTo("body");
};

const JAVA_LANDMARKS: &[Landmark] = landmarks! {
    "class_declaration" => Type, Field("name");
    "interface_declaration" => Type, Field("name");
    "enum_declaration" => Type, Field("name");
    "method_declaration" => Function, Field("name");
    "constructor_declaration" => Function, Field("name");
    "lambda_expression" => Closure, HeaderUpTo("body");
    "for_statement" => Loop, HeaderUpTo("body");
    "enhanced_for_statement" => Loop, HeaderUpTo("body");
    "while_statement" => Loop, HeaderUpTo("body");
    "if_statement" => Conditional, HeaderUpTo("consequence");
    "switch_expression" => Match, HeaderUpTo("body");
};

const CPP_LANDMARKS: &[Landmark] = landmarks! {
    "namespace_definition" => Module, Field("name");
    "class_specifier" => Type, Field("name");
    "struct_specifier" => Type, Field("name");
    "function_definition" => Function, CppFunctionName;
    "lambda_expression" => Closure, HeaderUpTo("body");
    "for_statement" => Loop, HeaderUpTo("body");
    "while_statement" => Loop, HeaderUpTo("body");
    "if_statement" => Conditional, HeaderUpTo("consequence");
    "switch_statement" => Match, HeaderUpTo("body");
};

/// `None` for languages with no meaningful "function/loop" concept
/// (JSON/TOML/YAML/XML/INI/Markdown) — the same boundary `lsp::LspLanguage`
/// already draws around "code with language intelligence". Private:
/// `Landmark` itself has no reason to be public, so this stays an internal
/// detail of `breadcrumbs_at` rather than something a caller resolves up
/// front.
fn landmarks_for(language: Language) -> Option<&'static [Landmark]> {
    match language {
        Language::Rust => Some(RUST_LANDMARKS),
        Language::Python => Some(PYTHON_LANDMARKS),
        Language::JavaScript | Language::TypeScript => Some(JS_TS_LANDMARKS),
        Language::Java => Some(JAVA_LANDMARKS),
        Language::Cpp => Some(CPP_LANDMARKS),
        Language::Json | Language::Toml | Language::Yaml | Language::Xml | Language::Ini | Language::Markdown => {
            None
        }
    }
}

fn ts_language(language: Language) -> Option<tree_sitter::Language> {
    match language {
        Language::Rust => Some(tree_sitter_rust::LANGUAGE.into()),
        Language::Python => Some(tree_sitter_python::LANGUAGE.into()),
        Language::JavaScript => Some(tree_sitter_javascript::LANGUAGE.into()),
        Language::TypeScript => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        Language::Java => Some(tree_sitter_java::LANGUAGE.into()),
        Language::Cpp => Some(tree_sitter_cpp::LANGUAGE.into()),
        Language::Json | Language::Toml | Language::Yaml | Language::Xml | Language::Ini | Language::Markdown => {
            None
        }
    }
}

/// Parses `source` (the current buffer contents) for `language`. `None` for
/// a language with no grammar wired here, or a source tree-sitter can't
/// recover from at all. Call only at settle time — see the module doc.
pub fn parse(language: Language, source: &str) -> Option<Tree> {
    let ts_lang = ts_language(language)?;
    let mut parser = Parser::new();
    parser.set_language(&ts_lang).ok()?;
    parser.parse(source, None)
}

/// The stack of enclosing landmark scopes at `byte_offset`, outermost
/// first — read straight off `tree` without touching `rope` beyond the
/// handful of small ranges each matched node covers. Empty for a language
/// with no landmark table (see `landmarks_for`).
pub fn breadcrumbs_at(tree: &Tree, rope: &Rope, byte_offset: usize, language: Language) -> Vec<Crumb> {
    let Some(landmarks) = landmarks_for(language) else {
        return Vec::new();
    };
    let root = tree.root_node();
    let offset = byte_offset.min(root.end_byte());
    let Some(leaf) = root.descendant_for_byte_range(offset, offset) else {
        return Vec::new();
    };

    let mut crumbs = Vec::new();
    let mut node = Some(leaf);
    while let Some(n) = node {
        if let Some(landmark) = landmarks.iter().find(|l| l.node_kind == n.kind())
            && let Some(label) = resolve_label(&landmark.label, n, rope)
        {
            crumbs.push(Crumb { kind: landmark.kind, label });
        }
        node = n.parent();
    }
    crumbs.reverse();
    crumbs
}

fn node_text(node: Node, rope: &Rope) -> Option<String> {
    rope.get_byte_slice(node.start_byte()..node.end_byte())
        .map(|s| s.to_string())
}

fn resolve_label(rule: &LabelRule, node: Node, rope: &Rope) -> Option<String> {
    match rule {
        LabelRule::Field(name) => node_text(node.child_by_field_name(*name)?, rope),
        LabelRule::RustImpl => {
            let ty = node_text(node.child_by_field_name("type")?, rope)?;
            Some(match node.child_by_field_name("trait") {
                Some(tr) => format!("{} for {ty}", node_text(tr, rope)?),
                None => ty,
            })
        }
        LabelRule::CppFunctionName => {
            cpp_declarator_name(node.child_by_field_name("declarator")?, rope)
        }
        LabelRule::HeaderUpTo(field) => {
            let anchor = node.child_by_field_name(*field)?;
            let end = anchor.start_byte();
            let start = node.start_byte();
            if end <= start {
                return None;
            }
            let header = rope.get_byte_slice(start..end)?.to_string();
            Some(truncate_header(&header))
        }
    }
}

/// Follows a C++ declarator's `declarator` field chain down to the actual
/// name — `function_declarator` → (optionally `pointer_declarator` /
/// `reference_declarator`, each wrapping the next declarator in turn) →
/// `identifier` / `field_identifier` / `destructor_name` / `operator_name`,
/// or `qualified_identifier`'s own `name` field for `Foo::bar`.
///
/// Deliberately does *not* do a blind "last identifier anywhere in the
/// subtree" walk: `function_declarator` also has a `parameters` field, and
/// parameter names (`int a, int b`) are `identifier` nodes too — a walk
/// that didn't respect field structure would report the last parameter's
/// name as the function's.
fn cpp_declarator_name(node: Node, rope: &Rope) -> Option<String> {
    match node.kind() {
        "identifier" | "field_identifier" | "destructor_name" | "operator_name" => {
            node_text(node, rope)
        }
        "qualified_identifier" => {
            cpp_declarator_name(node.child_by_field_name("name")?, rope)
        }
        _ => cpp_declarator_name(node.child_by_field_name("declarator")?, rope),
    }
}

/// Collapses a (possibly multi-line) header snippet to one line and caps it
/// at `MAX_HEADER_CHARS`, marking truncation with `…`.
fn truncate_header(s: &str) -> String {
    let collapsed = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= MAX_HEADER_CHARS {
        collapsed
    } else {
        let mut truncated: String = collapsed.chars().take(MAX_HEADER_CHARS).collect();
        truncated.push('\u{2026}');
        truncated
    }
}

#[cfg(test)]
#[path = "tests/outline.rs"]
mod tests;
