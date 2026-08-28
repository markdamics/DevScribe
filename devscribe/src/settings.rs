//! Persisted app-level settings: every value the Settings panel exposes —
//! theme mode + accent, chrome density, UI/editor font size, and the
//! Explorer/Editor/Toolchains toggles — in one JSON file under
//! `~/.config/devscribe/`, loaded once at startup, best-effort on write (a
//! failure here is never a hard error — the app just falls back to
//! defaults). `state.rs`'s `persist_settings` is the single place any of
//! this gets written, called at the end of every settings-changing
//! `Message` arm, so no such change can silently forget to save.
use crate::density::Density;
use crate::state::ChatMode;
use devscribe_core::theme::{Accent, ThemeMode};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Every persisted setting, decoupled from `State` so this module doesn't
/// need to know about tabs/trees/LSP/etc. — just the values the Settings
/// panel actually controls.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub theme_mode: ThemeMode,
    pub accent: Accent,
    pub density: Density,
    pub ui_font_scale: f32,
    pub editor_font_size: f32,
    pub git_status_in_tree: bool,
    pub show_hidden_files: bool,
    pub problem_lens_enabled: bool,
    pub save_on_focus_loss: bool,
    pub lsp_enabled: bool,
    pub chat_mode: ChatMode,
    pub chat_panel_width: f32,
}

impl Default for Settings {
    /// Mirrors `State::default()`'s own literal defaults for these same
    /// fields — the two must be kept in sync by hand, since `State` has a
    /// great deal else besides settings and can't just delegate to this.
    fn default() -> Self {
        Self {
            theme_mode: ThemeMode::default(),
            accent: Accent::default(),
            density: Density::default(),
            ui_font_scale: crate::state::UI_FONT_SCALE_DEFAULT,
            editor_font_size: crate::state::EDITOR_FONT_SIZE_DEFAULT,
            git_status_in_tree: true,
            show_hidden_files: false,
            problem_lens_enabled: true,
            save_on_focus_loss: false,
            lsp_enabled: true,
            chat_mode: ChatMode::Docked,
            chat_panel_width: crate::state::CHAT_DEFAULT_WIDTH,
        }
    }
}

/// The on-disk shape. Enums round-trip through their own stable string keys
/// (the Rust variant name, not `label()` — a *display* string, free to
/// change for wording reasons; using it here would silently reset the
/// setting the day a label gets reworded) rather than serde's derived
/// representation, so renaming/reordering variants doesn't reshuffle the
/// file format. Every field defaults independently on load (`#[serde(default...)]`)
/// so a file from before some later field existed — or with one
/// unrecognized/corrupt value — degrades one field at a time instead of the
/// whole load failing.
#[derive(Serialize, Deserialize)]
struct SettingsFile {
    #[serde(default)]
    theme_mode: String,
    #[serde(default)]
    accent: String,
    #[serde(default)]
    density: String,
    #[serde(default = "default_ui_font_scale")]
    ui_font_scale: f32,
    #[serde(default = "default_editor_font_size")]
    editor_font_size: f32,
    #[serde(default = "default_true")]
    git_status_in_tree: bool,
    #[serde(default)]
    show_hidden_files: bool,
    #[serde(default = "default_true")]
    problem_lens_enabled: bool,
    #[serde(default)]
    save_on_focus_loss: bool,
    #[serde(default = "default_true")]
    lsp_enabled: bool,
    #[serde(default)]
    chat_mode: String,
    #[serde(default = "default_chat_panel_width")]
    chat_panel_width: f32,
}

fn default_true() -> bool {
    true
}

fn default_ui_font_scale() -> f32 {
    crate::state::UI_FONT_SCALE_DEFAULT
}

fn default_editor_font_size() -> f32 {
    crate::state::EDITOR_FONT_SIZE_DEFAULT
}

fn default_chat_panel_width() -> f32 {
    crate::state::CHAT_DEFAULT_WIDTH
}

fn store_path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("devscribe").join("settings.json"))
}

fn mode_key(mode: ThemeMode) -> &'static str {
    match mode {
        ThemeMode::Dark => "Dark",
        ThemeMode::Light => "Light",
    }
}

fn mode_from_key(key: &str) -> Option<ThemeMode> {
    ThemeMode::ALL.into_iter().find(|mode| mode_key(*mode) == key)
}

fn accent_key(accent: Accent) -> &'static str {
    match accent {
        Accent::Tsuki => "Tsuki",
        Accent::Seiji => "Seiji",
        Accent::Matcha => "Matcha",
        Accent::Fuji => "Fuji",
        Accent::Kohaku => "Kohaku",
        Accent::Nezu => "Nezu",
    }
}

fn accent_from_key(key: &str) -> Option<Accent> {
    Accent::ALL.into_iter().find(|accent| accent_key(*accent) == key)
}

fn density_key(density: Density) -> &'static str {
    match density {
        Density::Compact => "Compact",
        Density::Comfortable => "Comfortable",
        Density::Spacious => "Spacious",
    }
}

fn density_from_key(key: &str) -> Option<Density> {
    Density::ALL.into_iter().find(|density| density_key(*density) == key)
}

fn chat_mode_key(mode: ChatMode) -> &'static str {
    match mode {
        ChatMode::Docked => "Docked",
        ChatMode::Collapsed => "Collapsed",
        ChatMode::Window => "Window",
        ChatMode::Closed => "Closed",
    }
}

fn chat_mode_from_key(key: &str) -> Option<ChatMode> {
    ChatMode::ALL.into_iter().find(|mode| chat_mode_key(*mode) == key)
}

/// The persisted settings, falling back to `Settings::default()` wholesale
/// (missing file, unreadable file, corrupt JSON) or field-by-field (an
/// unrecognized enum key — including every key from the pre-Maho
/// ten-named-theme era) rather than treating either as a hard failure. Only
/// called from `state.rs`'s non-test `startup_settings()` (see that
/// function's doc), so this is unreachable dead code in the test build
/// specifically, not in the real binary.
#[cfg_attr(test, allow(dead_code))]
pub fn load() -> Settings {
    store_path().and_then(|path| load_from(&path)).unwrap_or_default()
}

fn load_from(path: &Path) -> Option<Settings> {
    let text = std::fs::read_to_string(path).ok()?;
    let file: SettingsFile = serde_json::from_str(&text).ok()?;
    let defaults = Settings::default();
    Some(Settings {
        theme_mode: mode_from_key(&file.theme_mode).unwrap_or(defaults.theme_mode),
        accent: accent_from_key(&file.accent).unwrap_or(defaults.accent),
        density: density_from_key(&file.density).unwrap_or(defaults.density),
        ui_font_scale: file.ui_font_scale,
        editor_font_size: file.editor_font_size,
        git_status_in_tree: file.git_status_in_tree,
        show_hidden_files: file.show_hidden_files,
        problem_lens_enabled: file.problem_lens_enabled,
        save_on_focus_loss: file.save_on_focus_loss,
        lsp_enabled: file.lsp_enabled,
        chat_mode: chat_mode_from_key(&file.chat_mode).unwrap_or(defaults.chat_mode),
        chat_panel_width: file.chat_panel_width,
    })
}

/// Best-effort: a write failure (read-only config dir, disk full) is
/// silently swallowed — settings just won't survive the next restart, not a
/// reason to interrupt the user with an error.
pub fn save(settings: &Settings) {
    if let Some(path) = store_path() {
        save_to(&path, settings);
    }
}

fn save_to(path: &Path, settings: &Settings) {
    let Some(parent) = path.parent() else {
        return;
    };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let file = SettingsFile {
        theme_mode: mode_key(settings.theme_mode).to_string(),
        accent: accent_key(settings.accent).to_string(),
        density: density_key(settings.density).to_string(),
        ui_font_scale: settings.ui_font_scale,
        editor_font_size: settings.editor_font_size,
        git_status_in_tree: settings.git_status_in_tree,
        show_hidden_files: settings.show_hidden_files,
        problem_lens_enabled: settings.problem_lens_enabled,
        save_on_focus_loss: settings.save_on_focus_loss,
        lsp_enabled: settings.lsp_enabled,
        chat_mode: chat_mode_key(settings.chat_mode).to_string(),
        chat_panel_width: settings.chat_panel_width,
    };
    if let Ok(json) = serde_json::to_string_pretty(&file) {
        let _ = std::fs::write(path, json);
    }
}

#[cfg(test)]
#[path = "tests/settings.rs"]
mod tests;
