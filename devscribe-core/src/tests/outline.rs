use super::*;
use ropey::Rope;

fn crumbs_for(language: Language, source: &str, cursor_byte: usize) -> Vec<Crumb> {
    let tree = parse(language, source).expect("source must parse");
    let rope = Rope::from_str(source);
    breadcrumbs_at(&tree, &rope, cursor_byte, language)
}

fn labels(crumbs: &[Crumb]) -> Vec<&str> {
    crumbs.iter().map(|c| c.label.as_str()).collect()
}

#[test]
fn rust_nests_module_type_function_and_loop_in_order() {
    let src = "mod engine {\n    impl Ledger {\n        fn settle_batch(&self) {\n            for x in xs {\n                let y = 1;\n            }\n        }\n    }\n}\n";
    let at = src.find("let y").unwrap();
    let crumbs = crumbs_for(Language::Rust, src, at);
    assert_eq!(labels(&crumbs), vec!["engine", "Ledger", "settle_batch", "for x in xs"]);
    assert_eq!(
        crumbs.iter().map(|c| c.kind).collect::<Vec<_>>(),
        vec![CrumbKind::Module, CrumbKind::Type, CrumbKind::Function, CrumbKind::Loop],
    );
}

#[test]
fn rust_impl_for_trait_reads_both_sides() {
    let src = "struct Ledger;\nimpl Display for Ledger {\n    fn fmt(&self) {}\n}\n";
    let at = src.find("fn fmt").unwrap() + 3;
    let crumbs = crumbs_for(Language::Rust, src, at);
    assert_eq!(labels(&crumbs), vec!["Display for Ledger", "fmt"]);
}

#[test]
fn rust_inherent_impl_has_no_dangling_for() {
    let src = "struct Ledger;\nimpl Ledger {\n    fn new() {}\n}\n";
    let at = src.find("fn new").unwrap() + 3;
    let crumbs = crumbs_for(Language::Rust, src, at);
    assert_eq!(labels(&crumbs), vec!["Ledger", "new"]);
}

#[test]
fn rust_cursor_at_top_level_has_no_crumbs() {
    let src = "use std::collections::HashMap;\n\nconst ZERO: u32 = 0;\n";
    let at = src.find("ZERO").unwrap();
    assert!(crumbs_for(Language::Rust, src, at).is_empty());
}

#[test]
fn rust_if_and_match_anchor_on_the_right_field_not_body() {
    // `if_expression`'s child is `consequence`, not `body` (unlike every
    // other control-flow node) — the header must cut there, not run past it.
    let src = "fn f() {\n    if cond.is_some() {\n        let a = 1;\n    }\n    match op {\n        Op::A => 1,\n    };\n}\n";
    let if_at = src.find("let a").unwrap();
    assert_eq!(labels(&crumbs_for(Language::Rust, src, if_at)), vec!["f", "if cond.is_some()"]);

    let match_at = src.find("Op::A").unwrap();
    assert_eq!(labels(&crumbs_for(Language::Rust, src, match_at)), vec!["f", "match op"]);
}

#[test]
fn header_snippet_collapses_whitespace_and_truncates() {
    let long_iterable = "x".repeat(80);
    let src = format!("fn f() {{\n    for id in {long_iterable} {{\n        let y = 1;\n    }}\n}}\n");
    let at = src.find("let y").unwrap();
    let crumbs = crumbs_for(Language::Rust, &src, at);
    let header = &crumbs.last().unwrap().label;
    assert!(header.starts_with("for id in xxxx"));
    assert!(header.ends_with('\u{2026}'), "long header must be truncated with an ellipsis: {header:?}");
    assert!(header.chars().count() <= MAX_HEADER_CHARS + 1, "capped length plus the ellipsis: {header:?}");
    assert!(!header.contains('\n'));
}

#[test]
fn python_class_and_function_and_for() {
    let src = "class Ledger:\n    def settle(self):\n        for x in xs:\n            y = 1\n";
    let at = src.find("y = 1").unwrap();
    let crumbs = crumbs_for(Language::Python, src, at);
    assert_eq!(labels(&crumbs), vec!["Ledger", "settle", "for x in xs:"]);
    assert_eq!(crumbs[2].kind, CrumbKind::Loop);
}

#[test]
fn typescript_class_method_and_arrow_function() {
    let src = "class Ledger {\n  settle(): void {\n    const f = (id: string) => {\n      return id;\n    };\n  }\n}\n";
    let at = src.find("return id").unwrap();
    let crumbs = crumbs_for(Language::TypeScript, src, at);
    assert_eq!(labels(&crumbs), vec!["Ledger", "settle", "(id: string) =>"]);
    assert_eq!(crumbs[2].kind, CrumbKind::Closure);
}

#[test]
fn javascript_shares_the_typescript_table_for_plain_functions() {
    let src = "function settleBatch(entries) {\n  for (const e of entries) {\n    total += e;\n  }\n}\n";
    let at = src.find("total += e").unwrap();
    assert_eq!(
        labels(&crumbs_for(Language::JavaScript, src, at)),
        vec!["settleBatch", "for (const e of entries)"],
    );
}

#[test]
fn java_class_and_enhanced_for() {
    let src = "class Ledger {\n  void settle(List<Entry> es) {\n    for (Entry e : es) {\n      apply(e);\n    }\n  }\n}\n";
    let at = src.find("apply(e)").unwrap();
    let crumbs = crumbs_for(Language::Java, src, at);
    assert_eq!(labels(&crumbs), vec!["Ledger", "settle", "for (Entry e : es)"]);
}

#[test]
fn cpp_plain_function_name_comes_from_the_declarator_not_a_parameter() {
    // Regression: a blind "last identifier in the subtree" search would
    // report the last *parameter* name ("b") as the function's own name,
    // since parameter identifiers live in the same declarator subtree.
    let src = "int64_t settle_batch(Batch* a, int64_t b) {\n    return a->apply(b);\n}\n";
    let at = src.find("return").unwrap();
    assert_eq!(labels(&crumbs_for(Language::Cpp, src, at)), vec!["settle_batch"]);
}

#[test]
fn cpp_pointer_and_qualified_declarators_resolve_to_the_real_name() {
    let src = "namespace ledger {\n  int64_t* Batch::settle(int64_t id) {\n    return nullptr;\n  }\n}\n";
    let at = src.find("return nullptr").unwrap();
    let crumbs = crumbs_for(Language::Cpp, src, at);
    assert_eq!(labels(&crumbs), vec!["ledger", "settle"]);
}

#[test]
fn languages_without_a_landmark_table_produce_no_crumbs() {
    // No grammar wired in `ts_language` either, since these languages have
    // no landmarks to ever find — `parse` returning `None` (rather than a
    // `Tree` that just never matches anything) confirms outline.rs doesn't
    // bother parsing files it can never produce a breadcrumb for.
    for lang in [Language::Json, Language::Toml, Language::Yaml, Language::Xml, Language::Ini] {
        assert!(parse(lang, "irrelevant").is_none(), "{lang:?} must not parse for breadcrumbs");
    }
}

#[test]
fn cursor_past_end_of_file_does_not_panic() {
    // `descendant_for_byte_range` is clamped to `root.end_byte()`, not
    // handed the raw (out-of-range) offset — landing just past the closing
    // `}` legitimately resolves to no enclosing landmark (there is none),
    // but the clamp's job is only to keep this from panicking, which a
    // `find_editor`-raced-ahead-of-a-stale-cursor scenario could otherwise
    // trigger for real.
    let src = "fn f() {\n    let a = 1;\n}\n";
    let _ = crumbs_for(Language::Rust, src, src.len() + 500);

    // Just inside the function's own range still resolves normally.
    let crumbs = crumbs_for(Language::Rust, src, src.len() - 2);
    assert_eq!(labels(&crumbs), vec!["f"]);
}
