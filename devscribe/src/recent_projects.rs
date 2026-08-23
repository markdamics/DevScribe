//! Persisted "recently opened projects" list, backing the welcome screen's
//! recent list, the sidebar's projects dropdown, and startup's
//! auto-reopen-last-project behavior. Deliberately stores only `path` and
//! `last_opened_ms` — branch name, change count, and language glyph all go
//! stale between launches, so those are recomputed live (`state.rs`'s
//! `welcome_rows`) rather than persisted here.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// How many entries `touch` keeps — the welcome screen and sidebar dropdown
/// both only ever show a handful anyway, and every entry beyond what's
/// visible is a git-status scan (`state.rs`'s `compute_welcome_rows`) that
/// never gets used.
const MAX_RECENT: usize = 12;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecentProject {
    pub path: PathBuf,
    pub last_opened_ms: u64,
}

fn store_path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("devscribe").join("recent_projects.json"))
}

/// Empty on any error — a missing file (first run), unreadable file, or
/// corrupt JSON are all just "no recent projects yet," not a hard failure.
/// Only called from `state.rs`'s non-test `startup()` — the test build
/// deliberately never reads the real config file (see that module's doc),
/// so this is unreachable dead code there specifically, not in the real
/// binary.
#[cfg_attr(test, allow(dead_code))]
pub fn load() -> Vec<RecentProject> {
    match store_path() {
        Some(path) => load_from(&path),
        None => Vec::new(),
    }
}

fn load_from(path: &Path) -> Vec<RecentProject> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

/// Best-effort: this list is a convenience (auto-reopen, recent-list
/// display), not state the app can't function without, so a write failure
/// (read-only config dir, disk full) is silently swallowed rather than
/// surfaced anywhere.
pub fn save(list: &[RecentProject]) {
    if let Some(path) = store_path() {
        save_to(&path, list);
    }
}

fn save_to(path: &Path, list: &[RecentProject]) {
    let Some(parent) = path.parent() else {
        return;
    };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    if let Ok(json) = serde_json::to_string_pretty(list) {
        let _ = std::fs::write(path, json);
    }
}

/// Moves `path` to the front (deduping any existing entry for it) with a
/// fresh `last_opened_ms`, caps the list at `MAX_RECENT`, and persists the
/// result. Called on every successful project load, including the
/// auto-reopen at startup, so `last_opened_ms`/ordering stay accurate for
/// next launch.
pub fn touch(list: &mut Vec<RecentProject>, path: PathBuf) {
    touch_at(list, path, now_ms());
    save(list);
}

/// The pure list-mutation half of [`touch`], split out so tests can drive it
/// with a deterministic timestamp and without touching the real config
/// directory via [`save`].
fn touch_at(list: &mut Vec<RecentProject>, path: PathBuf, last_opened_ms: u64) {
    list.retain(|p| p.path != path);
    list.insert(0, RecentProject { path, last_opened_ms });
    list.truncate(MAX_RECENT);
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// A compact relative label for `last_opened_ms` — "12M"/"3H"/"2D"/"1W",
/// matching the welcome screen mockup's far-right timestamp column.
pub fn relative_label(last_opened_ms: u64) -> String {
    relative_label_at(last_opened_ms, now_ms())
}

fn relative_label_at(last_opened_ms: u64, now_ms: u64) -> String {
    let elapsed_secs = now_ms.saturating_sub(last_opened_ms) / 1000;
    match elapsed_secs {
        0..=3599 => format!("{}M", (elapsed_secs / 60).max(1)),
        3600..=86399 => format!("{}H", elapsed_secs / 3600),
        86400..=604_799 => format!("{}D", elapsed_secs / 86400),
        _ => format!("{}W", elapsed_secs / 604_800),
    }
}

/// Best-effort project-type glyph from marker files at `root` — good enough
/// for the welcome screen's badge, not a real "dominant language" scan.
/// Falls back to `fs_tree::Lang::Other` (a generic dot/extension glyph).
pub fn detect_lang(root: &Path) -> crate::fs_tree::Lang {
    use crate::fs_tree::Lang;
    if root.join("Cargo.toml").exists() {
        Lang::Rust
    } else if root.join("package.json").exists() {
        Lang::Ts
    } else if root.join("pom.xml").exists() || root.join("build.gradle").exists() {
        Lang::Java
    } else {
        Lang::Other
    }
}

#[cfg(test)]
#[path = "tests/recent_projects.rs"]
mod tests;
