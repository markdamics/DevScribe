use super::*;
use std::path::PathBuf;

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "devscribe-settings-test-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id(),
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[test]
fn theme_key_round_trips_through_theme_from_key_for_every_theme() {
    for theme in ThemeName::ALL {
        assert_eq!(theme_from_key(theme_key(theme)), Some(theme), "{theme:?} must round-trip through its own key");
    }
}

#[test]
fn theme_from_key_rejects_an_unrecognized_key() {
    assert_eq!(theme_from_key("NotARealTheme"), None);
}

#[test]
fn save_to_and_load_from_round_trip() {
    let dir = TempDir::new("round-trip");
    let store = dir.path.join("nested").join("settings.json");

    save_to(&store, ThemeName::Abyssal);
    assert_eq!(load_from(&store), Some(ThemeName::Abyssal));
}

#[test]
fn load_from_a_missing_file_is_none_not_an_error() {
    let dir = TempDir::new("missing");
    assert_eq!(load_from(&dir.path.join("does-not-exist.json")), None);
}

#[test]
fn load_from_malformed_json_is_none_not_a_panic() {
    let dir = TempDir::new("malformed");
    let store = dir.path.join("settings.json");
    std::fs::write(&store, "not json").unwrap();
    assert_eq!(load_from(&store), None);
}
