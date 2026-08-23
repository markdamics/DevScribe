use super::*;

#[test]
fn language_from_extension() {
    assert_eq!(LspLanguage::from_extension("rs"), Some(LspLanguage::Rust));
    assert_eq!(LspLanguage::from_extension("RS"), Some(LspLanguage::Rust));
    assert_eq!(LspLanguage::from_extension("json"), None);
}
