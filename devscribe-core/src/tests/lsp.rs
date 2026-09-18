use super::*;
use async_lsp::lsp_types::InlayHintLabelPart;

#[test]
fn language_from_extension() {
    assert_eq!(LspLanguage::from_extension("rs"), Some(LspLanguage::Rust));
    assert_eq!(LspLanguage::from_extension("RS"), Some(LspLanguage::Rust));
    assert_eq!(LspLanguage::from_extension("json"), None);
}

fn sample_hint(label: InlayHintLabel, kind: Option<InlayHintKind>) -> InlayHint {
    InlayHint {
        position: Position { line: 4, character: 9 },
        label,
        kind,
        text_edits: None,
        tooltip: None,
        padding_left: None,
        padding_right: None,
        data: None,
    }
}

#[test]
fn inlay_hint_to_entry_keeps_a_type_hint() {
    let hint = sample_hint(InlayHintLabel::String(": String".into()), Some(InlayHintKind::TYPE));
    let entry = inlay_hint_to_entry(hint).expect("a type hint must survive");
    assert_eq!(entry.line, 4);
    assert_eq!(entry.character, 9);
    assert_eq!(entry.text, ": String");
}

#[test]
fn inlay_hint_to_entry_drops_a_parameter_hint() {
    // Roadmap item 13 asks for inferred *types*, not parameter names — a
    // separate, out-of-scope feature this must not also surface.
    let hint = sample_hint(InlayHintLabel::String("name:".into()), Some(InlayHintKind::PARAMETER));
    assert!(inlay_hint_to_entry(hint).is_none());
}

#[test]
fn inlay_hint_to_entry_keeps_a_hint_with_no_kind_at_all() {
    // The spec allows an absent `kind`; a server that omits it must not be
    // silently treated as a parameter hint and dropped.
    let hint = sample_hint(InlayHintLabel::String(": i32".into()), None);
    assert!(inlay_hint_to_entry(hint).is_some());
}

#[test]
fn inlay_hint_to_entry_joins_label_parts_and_applies_padding() {
    let mut hint = sample_hint(
        InlayHintLabel::LabelParts(vec![
            InlayHintLabelPart { value: ":".into(), tooltip: None, location: None, command: None },
            InlayHintLabelPart { value: " String".into(), tooltip: None, location: None, command: None },
        ]),
        Some(InlayHintKind::TYPE),
    );
    hint.padding_left = Some(true);
    let entry = inlay_hint_to_entry(hint).unwrap();
    assert_eq!(entry.text, " : String");
}

#[test]
fn inlay_hint_to_entry_drops_an_empty_label() {
    let hint = sample_hint(InlayHintLabel::String("   ".into()), Some(InlayHintKind::TYPE));
    assert!(inlay_hint_to_entry(hint).is_none());
}
