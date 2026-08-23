//! Persisted app-level settings — for now, just the last-selected theme
//! (`state::set_theme` calls `save_theme` on every change). Same shape as
//! `recent_projects.rs`: a small JSON file under `~/.config/devscribe/`,
//! loaded once at startup, best-effort on write (a failure here is never a
//! hard error — the app just falls back to the default theme).
use devscribe_core::theme::ThemeName;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize)]
struct SettingsFile {
    theme: String,
}

fn store_path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("devscribe").join("settings.json"))
}

/// Stable serialization keys for `ThemeName` — the Rust variant name, not
/// `ThemeName::label()` (a *display* string, free to change for wording
/// reasons; using it here would silently reset everyone's saved theme the
/// day a label gets reworded).
fn theme_key(theme: ThemeName) -> &'static str {
    match theme {
        ThemeName::NullGrid => "NullGrid",
        ThemeName::Gantry => "Gantry",
        ThemeName::Abyssal => "Abyssal",
        ThemeName::Raven => "Raven",
        ThemeName::Ember => "Ember",
        ThemeName::Verdigris => "Verdigris",
        ThemeName::Meridian => "Meridian",
        ThemeName::Stark => "Stark",
        ThemeName::Sumi => "Sumi",
        ThemeName::Washi => "Washi",
    }
}

fn theme_from_key(key: &str) -> Option<ThemeName> {
    ThemeName::ALL.into_iter().find(|theme| theme_key(*theme) == key)
}

/// `None` on any error — a missing file (first run), unreadable file,
/// corrupt JSON, or an unrecognized theme key are all just "nothing saved
/// yet, use the default," not a hard failure. Only called from `state.rs`'s
/// non-test `startup_theme()` — the test build deliberately never reads
/// the real config file (see that function's doc), so this is unreachable
/// dead code there specifically, not in the real binary.
#[cfg_attr(test, allow(dead_code))]
pub fn load() -> Option<ThemeName> {
    store_path().and_then(|path| load_from(&path))
}

fn load_from(path: &Path) -> Option<ThemeName> {
    let text = std::fs::read_to_string(path).ok()?;
    let file: SettingsFile = serde_json::from_str(&text).ok()?;
    theme_from_key(&file.theme)
}

/// Best-effort: a write failure (read-only config dir, disk full) is
/// silently swallowed — the theme just won't survive the next restart,
/// not a reason to interrupt the user with an error.
pub fn save_theme(theme: ThemeName) {
    if let Some(path) = store_path() {
        save_to(&path, theme);
    }
}

fn save_to(path: &Path, theme: ThemeName) {
    let Some(parent) = path.parent() else {
        return;
    };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    if let Ok(json) = serde_json::to_string_pretty(&SettingsFile { theme: theme_key(theme).to_string() }) {
        let _ = std::fs::write(path, json);
    }
}

#[cfg(test)]
#[path = "tests/settings.rs"]
mod tests;
