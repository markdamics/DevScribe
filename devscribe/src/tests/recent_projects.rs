use super::*;
use std::path::PathBuf;

/// A scratch dir unique to this test run, for tests that need real paths on
/// disk (`save_to`/`load_from` round-tripping, `detect_lang`'s marker-file
/// checks). Cleaned up on drop.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "devscribe-recent-projects-test-{tag}-{}-{:?}",
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
fn touch_at_inserts_new_entries_at_the_front() {
    let mut list = Vec::new();
    touch_at(&mut list, PathBuf::from("/a"), 100);
    touch_at(&mut list, PathBuf::from("/b"), 200);
    assert_eq!(list.iter().map(|p| &p.path).collect::<Vec<_>>(), vec![&PathBuf::from("/b"), &PathBuf::from("/a")]);
}

#[test]
fn touch_at_dedupes_an_existing_path_and_refreshes_its_timestamp() {
    let mut list = Vec::new();
    touch_at(&mut list, PathBuf::from("/a"), 100);
    touch_at(&mut list, PathBuf::from("/b"), 200);
    touch_at(&mut list, PathBuf::from("/a"), 300);

    assert_eq!(list.len(), 2, "re-touching /a must not duplicate its entry");
    assert_eq!(list[0].path, PathBuf::from("/a"));
    assert_eq!(list[0].last_opened_ms, 300);
    assert_eq!(list[1].path, PathBuf::from("/b"));
}

#[test]
fn touch_at_caps_the_list_at_max_recent() {
    let mut list = Vec::new();
    for i in 0..(MAX_RECENT + 5) {
        touch_at(&mut list, PathBuf::from(format!("/p{i}")), i as u64);
    }
    assert_eq!(list.len(), MAX_RECENT);
    // Most recently touched (highest i) stays at the front.
    assert_eq!(list[0].path, PathBuf::from(format!("/p{}", MAX_RECENT + 4)));
}

#[test]
fn save_to_and_load_from_round_trip() {
    let dir = TempDir::new("round-trip");
    let store = dir.path.join("nested").join("recent_projects.json");

    let mut list = Vec::new();
    touch_at(&mut list, PathBuf::from("/work/one"), 111);
    touch_at(&mut list, PathBuf::from("/work/two"), 222);
    save_to(&store, &list);

    let loaded = load_from(&store);
    assert_eq!(loaded, list, "round-tripping through JSON must preserve the list exactly");
}

#[test]
fn load_from_a_missing_file_is_an_empty_list_not_an_error() {
    let dir = TempDir::new("missing");
    let store = dir.path.join("does-not-exist.json");
    assert!(load_from(&store).is_empty());
}

#[test]
fn load_from_malformed_json_is_an_empty_list_not_a_panic() {
    let dir = TempDir::new("malformed");
    let store = dir.path.join("recent_projects.json");
    std::fs::write(&store, "not json").unwrap();
    assert!(load_from(&store).is_empty());
}

#[test]
fn relative_label_uses_the_biggest_unit_that_fits() {
    // A realistic epoch-ms "now" (2026-ish) — large enough that subtracting
    // even a few weeks in milliseconds can't underflow the `u64`.
    let now = 1_770_000_000_000u64;
    assert_eq!(relative_label_at(now - 30_000, now), "1M", "under a minute still rounds up to 1M, not 0M");
    assert_eq!(relative_label_at(now - 12 * 60_000, now), "12M");
    assert_eq!(relative_label_at(now - 2 * 3_600_000, now), "2H");
    assert_eq!(relative_label_at(now - 2 * 86_400_000, now), "2D");
    assert_eq!(relative_label_at(now - 3 * 604_800_000, now), "3W");
}

#[test]
fn detect_lang_prefers_cargo_toml() {
    let dir = TempDir::new("lang-rust");
    std::fs::write(dir.path.join("Cargo.toml"), "[package]").unwrap();
    assert_eq!(detect_lang(&dir.path), crate::fs_tree::Lang::Rust);
}

#[test]
fn detect_lang_falls_back_to_package_json() {
    let dir = TempDir::new("lang-ts");
    std::fs::write(dir.path.join("package.json"), "{}").unwrap();
    assert_eq!(detect_lang(&dir.path), crate::fs_tree::Lang::Ts);
}

#[test]
fn detect_lang_defaults_to_other_with_no_marker_files() {
    let dir = TempDir::new("lang-other");
    assert_eq!(detect_lang(&dir.path), crate::fs_tree::Lang::Other);
}
