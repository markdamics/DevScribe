use super::*;

/// A scratch dir unique to this test run — same idiom as
/// `recent_projects.rs`'s own test-only `TempDir`. Cleaned up on drop.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "devscribe-session-test-{tag}-{}-{:?}",
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

fn sample() -> Session {
    Session {
        open_tabs: vec![
            SessionTab { path: PathBuf::from("/work/a.rs"), is_diff: false, cursor_line: 4, cursor_col: 2 },
            SessionTab { path: PathBuf::from("/work/b.rs"), is_diff: true, cursor_line: 0, cursor_col: 0 },
        ],
        active_tab: Some(1),
        sidebar_width: 260.0,
        sidebar_collapsed: false,
        collapsed_dirs: vec![PathBuf::from("/work/target")],
        changes_panel_open: true,
        problems_panel_open: false,
        chat_mode: "Docked".to_string(),
        chat_tab_open: false,
        chat_tab_active: false,
    }
}

#[test]
fn save_to_and_load_from_round_trip() {
    let dir = TempDir::new("round-trip");
    let store = dir.path.join("nested").join("session.json");

    let session = sample();
    save_to(&store, &session);

    assert_eq!(load_from(&store), session, "round-tripping through JSON must preserve the session exactly");
}

#[test]
fn load_from_a_missing_file_is_the_default_not_an_error() {
    let dir = TempDir::new("missing");
    let store = dir.path.join("does-not-exist.json");
    assert_eq!(load_from(&store), Session::default());
}

#[test]
fn load_from_malformed_json_is_the_default_not_a_panic() {
    let dir = TempDir::new("malformed");
    let store = dir.path.join("session.json");
    std::fs::write(&store, "not json").unwrap();
    assert_eq!(load_from(&store), Session::default());
}

#[test]
fn store_path_is_stable_for_the_same_root() {
    let root = PathBuf::from("/some/project/root");
    assert_eq!(store_path(&root), store_path(&root));
}

#[test]
fn store_path_differs_for_different_roots() {
    assert_ne!(store_path(&PathBuf::from("/a")), store_path(&PathBuf::from("/b")));
}
