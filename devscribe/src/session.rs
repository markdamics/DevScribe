//! Persisted per-project "session" — which tabs were open, which one was
//! active, each tab's cursor position, and sidebar/panel layout — so
//! reopening a project (including the auto-reopen-last-project startup path
//! `recent_projects` already drives) restores the workspace instead of
//! always starting from a blank tab bar. Keyed by a hash of the project
//! root rather than the root path itself, since a path can contain
//! characters that aren't valid in a filename; collisions are irrelevant
//! here the same way they are for any other content-addressed cache — worst
//! case two projects share a session file and one's layout loses to the
//! other's, never a correctness problem, just a stale-looking restore.
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

/// One restored tab. `is_diff` distinguishes a `TabKey::Diff` from the plain
/// `TabKey::File` for the same path — both can be open at once.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionTab {
    pub path: PathBuf,
    pub is_diff: bool,
    pub cursor_line: usize,
    pub cursor_col: usize,
}

/// Everything `state.rs`'s `capture_session`/`restore_session` round-trip.
/// Only `File`/`Diff` tabs are ever recorded — `TabKey::Search`/`TabKey::Chat`
/// have no on-disk identity to restore, and an "Untitled-N" scratch buffer's
/// content lives nowhere but memory, so neither belongs here.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Session {
    /// In tab-bar order.
    pub open_tabs: Vec<SessionTab>,
    /// Index into `open_tabs` of the tab that was active, or `None` if the
    /// active tab at save time was `Search`/`Chat`/nothing.
    pub active_tab: Option<usize>,
    pub sidebar_width: f32,
    pub sidebar_collapsed: bool,
    pub collapsed_dirs: Vec<PathBuf>,
    pub changes_panel_open: bool,
    pub problems_panel_open: bool,
    /// `ChatMode`'s stable string key ("Docked"/"Collapsed"/"Closed") — kept
    /// as a plain `String` rather than depending on `state::ChatMode`
    /// directly, same reasoning as `settings.rs`'s own enum fields (a
    /// renamed/reordered variant shouldn't reshuffle this file's format).
    /// `state.rs`'s `capture_session`/`restore_session` round-trip it
    /// through `settings::chat_mode_key`/`chat_mode_from_key`. `#[serde(default)]`
    /// so a session file saved before this field existed still loads (as
    /// `""`, which `chat_mode_from_key` treats as "unrecognized" the same
    /// as any other unknown key) instead of invalidating the whole file.
    #[serde(default)]
    pub chat_mode: String,
    /// `true` while the AI Chat Assist panel was open as a full tab instead
    /// of docked/collapsed — mirrors `State::chat_tab_open`.
    #[serde(default)]
    pub chat_tab_open: bool,
    /// `true` if `TabKey::Chat` was the active tab at save time — `active_tab`
    /// above can't represent this itself (it only indexes `open_tabs`, and
    /// the chat tab has no backing `OpenTab` entry).
    #[serde(default)]
    pub chat_tab_active: bool,
}

fn sessions_dir() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("devscribe").join("sessions"))
}

/// A filename derived from `root`, not `root` itself — see the module doc.
fn store_path(root: &Path) -> Option<PathBuf> {
    let mut hasher = DefaultHasher::new();
    root.hash(&mut hasher);
    Some(sessions_dir()?.join(format!("{:016x}.json", hasher.finish())))
}

/// Empty (`Session::default()`) on any error — a missing file (first time
/// this project's been opened), unreadable file, or corrupt JSON are all
/// just "no session recorded yet," not a hard failure. Same shape as
/// `recent_projects::load`/`settings::load`.
pub fn load(root: &Path) -> Session {
    store_path(root).map(|path| load_from(&path)).unwrap_or_default()
}

fn load_from(path: &Path) -> Session {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Session::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

/// Best-effort: this is a convenience (restoring the workspace on reopen),
/// not state the app can't function without, so a write failure (read-only
/// config dir, disk full) is silently swallowed rather than surfaced
/// anywhere.
pub fn save(root: &Path, session: &Session) {
    if let Some(path) = store_path(root) {
        save_to(&path, session);
    }
}

fn save_to(path: &Path, session: &Session) {
    let Some(parent) = path.parent() else {
        return;
    };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    if let Ok(json) = serde_json::to_string_pretty(session) {
        let _ = std::fs::write(path, json);
    }
}

#[cfg(test)]
#[path = "tests/session.rs"]
mod tests;
