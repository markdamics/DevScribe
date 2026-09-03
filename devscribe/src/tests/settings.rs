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
fn mode_key_round_trips_through_mode_from_key_for_every_mode() {
    for mode in ThemeMode::ALL {
        assert_eq!(mode_from_key(mode_key(mode)), Some(mode), "{mode:?} must round-trip through its own key");
    }
}

#[test]
fn accent_key_round_trips_through_accent_from_key_for_every_accent() {
    for accent in Accent::ALL {
        assert_eq!(accent_from_key(accent_key(accent)), Some(accent), "{accent:?} must round-trip through its own key");
    }
}

#[test]
fn density_key_round_trips_through_density_from_key_for_every_density() {
    for density in Density::ALL {
        assert_eq!(
            density_from_key(density_key(density)),
            Some(density),
            "{density:?} must round-trip through its own key"
        );
    }
}

#[test]
fn chat_mode_key_round_trips_through_chat_mode_from_key_for_every_mode() {
    for mode in ChatMode::ALL {
        assert_eq!(chat_mode_from_key(chat_mode_key(mode)), Some(mode), "{mode:?} must round-trip through its own key");
    }
}

#[test]
fn mode_from_key_rejects_an_unrecognized_key() {
    assert_eq!(mode_from_key("NotARealTheme"), None);
}

#[test]
fn accent_from_key_rejects_an_unrecognized_key() {
    assert_eq!(accent_from_key("NotARealAccent"), None);
}

fn sample_settings() -> Settings {
    Settings {
        theme_mode: ThemeMode::Light,
        accent: Accent::Kohaku,
        custom_accent: Some((10, 20, 30)),
        high_contrast: true,
        density: Density::Compact,
        ui_font_scale: 1.2,
        editor_font_size: 16.0,
        git_status_in_tree: true,
        show_hidden_files: true,
        problem_lens_enabled: false,
        save_on_focus_loss: true,
        lsp_enabled: false,
        copilot_inline_enabled: true,
        chat_mode: ChatMode::Collapsed,
        chat_panel_width: 420.0,
        tab_size: 2,
        show_line_numbers: false,
        word_wrap: true,
    }
}

#[test]
fn save_to_and_load_from_round_trips_every_field() {
    let dir = TempDir::new("round-trip");
    let store = dir.path.join("nested").join("settings.json");

    save_to(&store, &sample_settings());

    assert_eq!(load_from(&store), Some(sample_settings()));
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

#[test]
fn load_from_a_pre_maho_key_defaults_every_field_not_a_panic() {
    // An old settings.json from the Axiom HUD era (`"theme": "NullGrid"`)
    // has none of the current fields at all — every one of them
    // independently defaults (`#[serde(default...)]`), so the file still
    // loads successfully rather than the whole load being rejected.
    let dir = TempDir::new("pre-maho");
    let store = dir.path.join("settings.json");
    std::fs::write(&store, r#"{"theme":"NullGrid"}"#).unwrap();
    assert_eq!(load_from(&store), Some(Settings::default()));
}

#[test]
fn load_from_a_partial_file_defaults_only_the_missing_fields() {
    // A file written before some later field existed (or with just an
    // unrecognized enum key) should keep whatever it does have and default
    // the rest, not reset everything.
    let dir = TempDir::new("partial");
    let store = dir.path.join("settings.json");
    std::fs::write(&store, r#"{"theme_mode":"Light","accent":"NotARealAccent"}"#).unwrap();

    let loaded = load_from(&store).expect("a partially-recognized file should still load");

    assert_eq!(loaded.theme_mode, ThemeMode::Light, "the recognized field should be kept");
    assert_eq!(loaded.accent, Settings::default().accent, "the unrecognized field should fall back to its own default");
    assert_eq!(loaded.density, Settings::default().density);
    assert_eq!(loaded.ui_font_scale, Settings::default().ui_font_scale);
}
