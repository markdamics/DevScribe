use super::*;

/// A scratch dir with two files, for tests that need real paths
/// `Document::open` can read. Cleaned up on drop.
struct TempFiles {
    dir: PathBuf,
    pub a: PathBuf,
    pub b: PathBuf,
}

impl TempFiles {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "devscribe-tabs-test-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id(),
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.txt");
        let b = dir.join("b.txt");
        std::fs::write(&a, "a").unwrap();
        std::fs::write(&b, "b").unwrap();
        Self { dir, a, b }
    }
}

impl Drop for TempFiles {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn open_or_focus_file_dedups_and_is_additive() {
    let files = TempFiles::new("dedup");
    let mut state = State::default();

    open_or_focus_file(&mut state, files.a.clone());
    assert_eq!(state.open_tabs.len(), 1);
    assert_eq!(state.active_tab, Some(TabKey::File(files.a.clone())));

    open_or_focus_file(&mut state, files.b.clone());
    assert_eq!(state.open_tabs.len(), 2, "opening a second file should be additive, not a replace");
    assert_eq!(state.active_tab, Some(TabKey::File(files.b.clone())));

    open_or_focus_file(&mut state, files.a.clone());
    assert_eq!(state.open_tabs.len(), 2, "reopening an already-open file must not duplicate its tab");
    assert_eq!(state.active_tab, Some(TabKey::File(files.a.clone())));
}

#[test]
fn close_tab_also_closes_its_diff_tab_and_refocuses() {
    let files = TempFiles::new("close");
    let mut state = State::default();

    open_or_focus_file(&mut state, files.a.clone());
    open_or_focus_file(&mut state, files.b.clone());
    open_or_focus_diff(&mut state, files.a.clone());
    assert_eq!(state.open_tabs.len(), 3);
    assert_eq!(state.active_tab, Some(TabKey::Diff(files.a.clone())));

    close_tab(&mut state, &TabKey::File(files.a.clone()));

    assert_eq!(
        state.open_tabs.iter().map(OpenTab::key).collect::<Vec<_>>(),
        vec![TabKey::File(files.b.clone())],
        "closing a file tab must also close its now-orphaned diff tab"
    );
    assert_eq!(
        state.active_tab,
        Some(TabKey::File(files.b.clone())),
        "closing the active tab (even indirectly, via its diff tab) must refocus a remaining tab"
    );
}

#[test]
fn closing_last_tab_clears_active_tab() {
    let files = TempFiles::new("last");
    let mut state = State::default();

    open_or_focus_file(&mut state, files.a.clone());
    close_tab(&mut state, &TabKey::File(files.a.clone()));

    assert!(state.open_tabs.is_empty());
    assert_eq!(state.active_tab, None);
}

#[test]
fn utf16_col_ascii_matches_char_col() {
    assert_eq!(utf16_col_to_char_col("let x = 1;", 4), 4);
    assert_eq!(utf16_col_to_char_col("let x = 1;", 0), 0);
}

#[test]
fn utf16_col_past_end_clamps_to_line_length() {
    assert_eq!(utf16_col_to_char_col("abc", 99), 3);
}

#[test]
fn utf16_col_bmp_multibyte_char_counts_as_one_unit() {
    // 'é' is 1 UTF-16 code unit but 2 UTF-8 bytes — this must track
    // chars/UTF-16 units, not bytes.
    let line = "café!";
    assert_eq!(utf16_col_to_char_col(line, 4), 4);
}

#[test]
fn utf16_col_surrogate_pair_counts_as_two_units() {
    // An emoji outside the BMP is 1 char but 2 UTF-16 code units, so an
    // LSP column pointing just past it is offset by 2, not 1.
    let line = "a\u{1F600}b";
    assert_eq!(utf16_col_to_char_col(line, 0), 0);
    assert_eq!(utf16_col_to_char_col(line, 1), 1);
    assert_eq!(utf16_col_to_char_col(line, 3), 2);
}

#[test]
fn commit_draft_new_file_writes_to_disk_and_opens_tab() {
    let files = TempFiles::new("draft-new-file");
    let dir = files.a.parent().unwrap().to_path_buf();
    let mut state = State {
        draft: Some(Draft {
            kind: DraftKind::NewFile,
            dir: dir.clone(),
            target: None,
            text: "new.txt".to_string(),
        }),
        ..State::default()
    };

    commit_draft(&mut state);

    assert!(state.draft.is_none(), "a successful commit should clear the draft");
    let new_path = dir.join("new.txt");
    assert!(new_path.exists(), "commit_draft should actually write the file to disk");
    assert_eq!(state.active_tab, Some(TabKey::File(new_path)));
    assert!(
        state.flash.as_ref().is_some_and(|f| f.text.contains("FILE CREATED")),
        "a successful new-file commit should fire the flash pill"
    );
}

#[test]
fn commit_draft_rename_updates_open_tab_path_in_place() {
    let files = TempFiles::new("draft-rename");
    let mut state = State::default();
    open_or_focus_file(&mut state, files.a.clone());
    let new_path = files.a.with_file_name("renamed.txt");
    state.draft = Some(Draft {
        kind: DraftKind::Rename,
        dir: files.a.parent().unwrap().to_path_buf(),
        target: Some(files.a.clone()),
        text: "renamed.txt".to_string(),
    });

    commit_draft(&mut state);

    assert!(!files.a.exists(), "the old path should no longer exist after a rename");
    assert!(new_path.exists());
    assert_eq!(
        state.active_tab,
        Some(TabKey::File(new_path.clone())),
        "the active tab's key should follow the rename, not go stale"
    );
    let editor = find_editor(&state, &new_path).expect("the renamed file's tab should still be open, under its new path");
    assert_eq!(editor.path, new_path);
    assert_eq!(
        editor.document.path(),
        Some(new_path.as_path()),
        "the document's own path must be repointed too, or a later save would write to the old (now gone) path"
    );
}

#[test]
fn close_other_tabs_keeps_only_the_active_tab() {
    let files = TempFiles::new("close-others");
    let mut state = State::default();
    open_or_focus_file(&mut state, files.a.clone());
    open_or_focus_file(&mut state, files.b.clone());
    assert_eq!(state.open_tabs.len(), 2);
    assert_eq!(state.active_tab, Some(TabKey::File(files.b.clone())));

    close_other_tabs(&mut state);

    assert_eq!(state.open_tabs.len(), 1);
    assert_eq!(
        state.active_tab,
        Some(TabKey::File(files.b.clone())),
        "the active tab itself must survive Close Others"
    );
}

#[test]
fn reopen_closed_tab_restores_the_most_recently_closed() {
    let files = TempFiles::new("reopen");
    let mut state = State::default();
    open_or_focus_file(&mut state, files.a.clone());
    close_tab(&mut state, &TabKey::File(files.a.clone()));
    assert_eq!(state.open_tabs.len(), 0);

    reopen_closed_tab(&mut state);

    assert_eq!(state.open_tabs.len(), 1);
    assert_eq!(state.active_tab, Some(TabKey::File(files.a.clone())));
}

#[test]
fn reveal_active_in_tree_uncollapses_ancestor_dirs() {
    let files = TempFiles::new("reveal");
    let root = files.a.parent().unwrap().to_path_buf();
    let sub = root.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    let nested = sub.join("nested.txt");
    std::fs::write(&nested, "x").unwrap();

    let mut state = State {
        root: root.clone(),
        ..State::default()
    };
    state.collapsed_dirs.insert(sub.clone());
    open_or_focus_file(&mut state, nested.clone());

    reveal_active_in_tree(&mut state);

    assert!(
        !state.collapsed_dirs.contains(&sub),
        "the active file's parent directory should be expanded so it's actually visible"
    );
}

#[test]
fn collapse_sidebar_sets_collapsed_and_closes_menus_anchored_to_it() {
    let mut state = State {
        projects_open: true,
        ctx_menu: Some(ContextMenu { target: None, confirm_delete: false }),
        ..State::default()
    };

    let _ = update(&mut state, Message::CollapseSidebar);

    assert!(state.sidebar_collapsed);
    assert!(!state.projects_open, "the projects dropdown is anchored to sidebar content that's about to disappear");
    assert!(state.ctx_menu.is_none(), "the tree context menu is anchored to sidebar content that's about to disappear");
}

#[test]
fn expand_sidebar_clears_collapsed_without_touching_other_state() {
    let mut state = State {
        sidebar_collapsed: true,
        projects_open: true,
        ..State::default()
    };

    let _ = update(&mut state, Message::ExpandSidebar);

    assert!(!state.sidebar_collapsed);
    assert!(state.projects_open, "expanding shouldn't have any side effects beyond un-collapsing");
}

#[test]
fn view_working_tree_diff_prefers_the_active_file() {
    let files = TempFiles::new("wtd-active");
    // `changed_files` starts empty rather than trusting `State::default()`'s
    // real-repo introspection — this test runs inside DevScribe's own
    // (often locally-modified) working tree, so that would make the
    // fallback ordering asserted below nondeterministic.
    let mut state = State {
        changed_files: Vec::new(),
        ..State::default()
    };
    open_or_focus_file(&mut state, files.a.clone());
    open_or_focus_file(&mut state, files.b.clone());
    // b is active; a stand-in "changed_files" entry for a different file
    // must NOT win over the active tab.
    state.changed_files.push(ChangesEntry {
        path: files.a.clone(),
        kind: ChangeKind::Modified,
        insertions: 1,
        deletions: 0,
    });

    view_working_tree_diff(&mut state);

    assert_eq!(state.active_tab, Some(TabKey::Diff(files.b.clone())));
}

#[test]
fn view_working_tree_diff_falls_back_to_first_changed_file_with_no_active_tab() {
    let files = TempFiles::new("wtd-fallback");
    let mut state = State {
        changed_files: vec![ChangesEntry {
            path: files.a.clone(),
            kind: ChangeKind::Modified,
            insertions: 1,
            deletions: 0,
        }],
        ..State::default()
    };
    assert_eq!(state.active_tab, None);

    view_working_tree_diff(&mut state);

    assert_eq!(state.active_tab, Some(TabKey::Diff(files.a.clone())));
}

#[test]
fn view_working_tree_diff_is_a_noop_with_nothing_to_diff() {
    let mut state = State {
        changed_files: Vec::new(),
        ..State::default()
    };

    view_working_tree_diff(&mut state);

    assert!(state.open_tabs.is_empty());
    assert_eq!(state.active_tab, None);
}

#[test]
fn diff_working_tree_palette_entry_is_gated_on_having_something_to_diff() {
    let mut state = State {
        changed_files: Vec::new(),
        ..State::default()
    };
    assert!(
        !all_palette_entries(&state).iter().any(|e| e.label == "Diff: open working tree changes"),
        "no active file and a clean tree means there's nothing to diff — the entry shouldn't be offered"
    );

    state.changed_files.push(ChangesEntry {
        path: PathBuf::from("changed.rs"),
        kind: ChangeKind::Modified,
        insertions: 1,
        deletions: 0,
    });
    assert!(
        all_palette_entries(&state).iter().any(|e| e.label == "Diff: open working tree changes"),
        "a changed file with no active tab is still something to diff, via the changed_files fallback"
    );
}

#[test]
fn save_all_dirty_files_saves_only_dirty_files_with_one_summary_toast() {
    let files = TempFiles::new("save-all-dirty");
    let mut state = State::default();
    open_or_focus_file(&mut state, files.a.clone());
    open_or_focus_file(&mut state, files.b.clone());
    find_editor_mut(&mut state, &files.a).unwrap().insert_text("edited");

    save_all_dirty_files(&mut state);

    assert_eq!(std::fs::read_to_string(&files.a).unwrap(), "editeda", "the dirty file's edit should be on disk");
    assert_eq!(std::fs::read_to_string(&files.b).unwrap(), "b", "the untouched file must not be rewritten");
    assert!(!find_editor(&state, &files.a).unwrap().document.is_dirty());
    assert_eq!(state.toasts.len(), 1, "saving one dirty file should produce exactly one toast, not per-file spam");
    assert!(state.toasts[0].message.contains("Saved 1 file"));
}

#[test]
fn window_unfocused_only_saves_when_the_toggle_is_on() {
    let files = TempFiles::new("save-on-blur");
    let mut state = State::default();
    assert!(!state.save_on_focus_loss, "off by default — see the field's doc comment for why");
    open_or_focus_file(&mut state, files.a.clone());
    find_editor_mut(&mut state, &files.a).unwrap().insert_text("x");

    let _ = update(&mut state, Message::WindowUnfocused);
    assert!(
        find_editor(&state, &files.a).unwrap().document.is_dirty(),
        "the toggle is off, so losing focus must not save"
    );

    let _ = update(&mut state, Message::ToggleSaveOnFocusLoss);
    let _ = update(&mut state, Message::WindowUnfocused);
    assert!(
        !find_editor(&state, &files.a).unwrap().document.is_dirty(),
        "with the toggle on, losing focus should save every dirty file"
    );
}

/// Drives the real `git` CLI — same rationale as
/// `devscribe_core::git::tests::git`: simplest way to get a realistic
/// index/worktree/HEAD combination without hand-assembling `gix` objects.
fn git(dir: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .status()
        .expect("git must be on PATH for this test");
    assert!(status.success(), "`git {args:?}` failed");
}

#[test]
fn window_focused_catches_up_a_change_the_watcher_never_saw() {
    // The regression this guards: a `git commit` (or `push`, or `checkout`
    // of a branch that touches no tracked file) never fires the file
    // watcher, since `.git` itself is in `SKIP_DIRS`. Before `WindowFocused`
    // existed, `state.changed_files` had no way to learn about that until
    // the next save.
    let dir = std::env::temp_dir().join(format!("devscribe-window-focused-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q"]);
    std::fs::write(dir.join("f.txt"), "one\n").unwrap();
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "initial"]);

    let mut state = State { root: dir.clone(), repo: Repo::open(&dir), welcome_open: false, ..State::default() };
    assert!(state.changed_files.is_empty(), "sanity: nothing changed yet");

    // Simulate an external edit + commit that never touched the watcher,
    // landing while `state.changed_files` is still the stale, pre-edit
    // snapshot from before this "terminal" round-trip.
    std::fs::write(dir.join("f.txt"), "two\n").unwrap();

    let _ = update(&mut state, Message::WindowFocused);

    assert_eq!(state.changed_files.len(), 1, "regaining focus should have re-scanned git status");
    assert_eq!(state.changed_files[0].path, dir.join("f.txt"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn window_focused_is_a_noop_before_a_project_is_open() {
    let mut state = State { welcome_open: true, ..State::default() };
    state.changed_files = vec![ChangesEntry {
        path: PathBuf::from("stale.txt"),
        kind: ChangeKind::Modified,
        insertions: 1,
        deletions: 0,
    }];

    let _ = update(&mut state, Message::WindowFocused);

    assert_eq!(state.changed_files.len(), 1, "no project open — must not touch changed_files");
}

#[test]
fn recompute_search_caps_a_single_file_with_many_matches_on_one_line() {
    // The real-world failure mode this guards: one file with many
    // matches on a single long line (a minified bundle, a lockfile, a
    // log) would otherwise have every match's *entire line* cloned into
    // a `SearchHit::preview` before the global `MAX_SEARCH_RESULTS` cap
    // ever got a chance to apply — for a long line matched thousands of
    // times, that's an unbounded, multiplicative memory spike on every
    // keystroke, not just a slow search. A single small file with one
    // line of 500 "e"s (well under `MAX_SEARCH_FILE_BYTES`, so this
    // isolates the match-count cap from the separate file-size guard)
    // already produces 500 matches, past `MAX_SEARCH_RESULTS`.
    let files = TempFiles::new("search-cap");
    let dir = files.a.parent().unwrap().to_path_buf();
    std::fs::write(dir.join("many_matches.txt"), "e".repeat(500)).unwrap();

    let files_in_dir: Vec<PathBuf> = fs_tree::flatten_files(&fs_tree::walk(&dir, false)).into_iter().map(Path::to_path_buf).collect();

    let outcome = run_search(&files_in_dir, "e");

    assert_eq!(outcome.results.len(), MAX_SEARCH_RESULTS);
}

#[test]
fn search_query_changed_does_not_start_a_search() {
    // The actual fix for "typing lags/kills the app": a query change
    // alone must never trigger the project-wide scan — it only
    // records *when*, for `SearchDebounceTick` to act on later.
    let mut state = State::default();

    let _ = update(&mut state, Message::SearchQueryChanged("needle".to_string()));

    assert_eq!(state.search_query, "needle");
    assert!(!state.search_in_progress, "typing alone must never start a search");
    assert!(state.search_results.is_empty());
    assert!(state.search_query_changed_at.is_some(), "the debounce tick needs this to know when to fire");
}

#[test]
fn search_debounce_tick_waits_for_the_delay_to_elapse() {
    let mut state = State {
        search_query: "needle".to_string(),
        search_query_changed_at: Some(Instant::now()),
        ..State::default()
    };

    let _ = update(&mut state, Message::SearchDebounceTick);

    assert!(!state.search_in_progress, "the delay hasn't elapsed yet — this tick should be a no-op");
}

#[test]
fn search_debounce_tick_starts_a_search_once_the_delay_elapses() {
    let files = TempFiles::new("search-debounce-fire");
    let dir = files.a.parent().unwrap().to_path_buf();
    std::fs::write(dir.join("match.txt"), "needle").unwrap();
    let mut state = State {
        root: dir.clone(),
        tree: fs_tree::walk(&dir, false),
        search_query: "needle".to_string(),
        search_query_changed_at: Instant::now().checked_sub(SEARCH_DEBOUNCE_DELAY * 2),
        ..State::default()
    };

    let _ = update(&mut state, Message::SearchDebounceTick);

    assert!(state.search_in_progress, "the delay elapsed — this tick should have started the search");
    assert!(state.search_task_handle.is_some());
    assert!(state.search_query_changed_at.is_none(), "shouldn't keep re-checking a search that's already started");
}

#[test]
fn search_submit_bypasses_the_debounce_and_starts_immediately() {
    let files = TempFiles::new("search-submit-now");
    let dir = files.a.parent().unwrap().to_path_buf();
    std::fs::write(dir.join("match.txt"), "needle").unwrap();
    let mut state = State {
        root: dir.clone(),
        tree: fs_tree::walk(&dir, false),
        search_query: "needle".to_string(),
        search_query_changed_at: Some(Instant::now()), // well within the debounce window
        ..State::default()
    };

    let _ = update(&mut state, Message::SearchSubmit);

    assert!(state.search_in_progress, "Enter should start the search right away, not wait for the debounce");
}

#[test]
fn search_completed_applies_results_only_for_the_still_current_query() {
    let mut state = State {
        search_query: "current".to_string(),
        search_in_progress: true,
        ..State::default()
    };

    // Stale: this search was for a query the user has since changed.
    let _ = update(
        &mut state,
        Message::SearchCompleted(SearchOutcome {
            query: "stale".to_string(),
            results: vec![SearchResult {
                path: PathBuf::from("x.txt"),
                hit: SearchHit { line: 0, col: 0, preview: "x".to_string(), preview_col: 0 },
                query_len_chars: 5,
            }],
            elapsed: Duration::ZERO,
        }),
    );
    assert!(state.search_results.is_empty(), "a stale completion must not overwrite newer state");
    assert!(state.search_in_progress, "a stale completion belongs to a search that isn't the current one at all");

    // Current: matches what's actually in the search box right now.
    let _ = update(
        &mut state,
        Message::SearchCompleted(SearchOutcome {
            query: "current".to_string(),
            results: vec![SearchResult {
                path: PathBuf::from("y.txt"),
                hit: SearchHit { line: 0, col: 0, preview: "y".to_string(), preview_col: 0 },
                query_len_chars: 7,
            }],
            elapsed: Duration::ZERO,
        }),
    );
    assert_eq!(state.search_results.len(), 1);
    assert_eq!(state.search_last_query, "current");
    assert!(!state.search_in_progress);
}

#[test]
fn starting_a_new_search_aborts_the_previous_ones_handle() {
    let files = TempFiles::new("search-cancel");
    let dir = files.a.parent().unwrap().to_path_buf();
    std::fs::write(dir.join("match.txt"), "needle").unwrap();
    let mut state = State {
        root: dir.clone(),
        tree: fs_tree::walk(&dir, false),
        search_query: "first".to_string(),
        ..State::default()
    };

    let _ = update(&mut state, Message::SearchSubmit);
    let first_handle = state.search_task_handle.clone().expect("first search should have a handle");
    assert!(!first_handle.is_aborted());

    state.search_query = "second".to_string();
    let _ = update(&mut state, Message::SearchSubmit);

    assert!(first_handle.is_aborted(), "starting a second search must cancel the first");
}

#[test]
fn recompute_search_skips_files_over_the_size_limit() {
    let files = TempFiles::new("search-file-size-cap");
    let dir = files.a.parent().unwrap().to_path_buf();
    // One byte over the limit — read+scan is skipped entirely rather
    // than paying the cost of a file this large regardless of whether
    // it even matches.
    let oversized = "e".repeat((MAX_SEARCH_FILE_BYTES + 1) as usize);
    std::fs::write(dir.join("too_big.txt"), oversized).unwrap();
    let files_in_dir: Vec<PathBuf> = fs_tree::flatten_files(&fs_tree::walk(&dir, false)).into_iter().map(Path::to_path_buf).collect();

    let outcome = run_search(&files_in_dir, "e");

    assert!(outcome.results.is_empty(), "an over-the-limit file should be skipped, not partially scanned");
}

#[test]
fn recompute_search_stops_after_max_files_scanned() {
    // A dedicated, otherwise-empty directory — not `TempFiles` (which
    // also creates its own a.txt/b.txt) — so the file count below is
    // exact and this test isn't sensitive to walk ordering.
    let dir = std::env::temp_dir().join(format!(
        "devscribe-search-file-count-cap-{}-{:?}",
        std::process::id(),
        std::thread::current().id(),
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // `MAX_SEARCH_FILES_SCANNED` filler files that don't match, plus
    // one that does — zero-padded and named to sort alphabetically
    // *after* every filler file, so `fs_tree::walk`'s alphabetical
    // ordering guarantees it's the one file past the cap. A single
    // possible match keeps this independent of `MAX_SEARCH_RESULTS`
    // (which a many-matches version of this test would hit first).
    let width = MAX_SEARCH_FILES_SCANNED.to_string().len();
    for i in 0..MAX_SEARCH_FILES_SCANNED {
        std::fs::write(dir.join(format!("a{i:0width$}.txt")), "no match here").unwrap();
    }
    std::fs::write(dir.join("zzz_needle.txt"), "needle").unwrap();

    let files_in_dir: Vec<PathBuf> = fs_tree::flatten_files(&fs_tree::walk(&dir, false)).into_iter().map(Path::to_path_buf).collect();

    let outcome = run_search(&files_in_dir, "needle");

    assert!(
        outcome.results.is_empty(),
        "the one matching file sorts after MAX_SEARCH_FILES_SCANNED filler files, \
         so the cap should stop the scan before ever reaching it"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn run_search_stays_fast_and_bounded_against_a_file_with_a_gigantic_line() {
    // The actual root cause behind a real, repeated "the app hangs/
    // crashes" report chased across several earlier passes: this
    // project's own `design/DevScribe.html` has a single line over a
    // million characters long, and searching a term that happened to
    // appear on it used to render that entire line as one text widget
    // — not a framework bug, genuinely catastrophic work for any
    // text-shaping pipeline. This drives the exact same file-reading
    // path (`run_search`, not just `search::search_text` in isolation)
    // a real search hits, so it also exercises `MAX_SEARCH_FILE_BYTES`
    // — this file is deliberately sized just *under* that cap, since a
    // huge line inside an otherwise-reasonably-sized file is the actual
    // failure mode, not a huge file being skipped outright.
    let files = TempFiles::new("search-huge-line");
    let dir = files.a.parent().unwrap().to_path_buf();
    let huge_line = format!("{}needle{}", "x".repeat(700_000), "y".repeat(700_000));
    assert!((huge_line.len() as u64) < MAX_SEARCH_FILE_BYTES, "must stay under the file-size cap to prove this case");
    std::fs::write(dir.join("huge_line.txt"), &huge_line).unwrap();

    let started = Instant::now();
    let outcome = run_search(&[dir.join("huge_line.txt")], "needle");
    let elapsed = started.elapsed();

    assert_eq!(outcome.results.len(), 1);
    assert!(
        outcome.results[0].hit.preview.len() < 1000,
        "the preview must be capped regardless of how long the real line is"
    );
    assert!(
        elapsed < Duration::from_secs(1),
        "matching a huge line should still be fast — got {elapsed:?}"
    );
}

#[test]
fn default_state_has_no_project_open() {
    // The test build never touches the real persisted recent-projects file
    // (see `startup`'s `#[cfg(test)]` doc) — every test gets a
    // deterministic "no project open" seed, not whatever this machine's
    // user last had open in the real app.
    let state = State::default();
    assert!(state.welcome_open);
    assert_eq!(state.root, PathBuf::new());
    assert!(state.tree.is_empty());
    assert!(state.repo.is_none());
    assert!(state.recent_projects.is_empty());
}

#[test]
fn reset_project_scoped_state_clears_everything_tied_to_the_previous_project() {
    let mut state = State {
        open_tabs: vec![OpenTab::Diff(PathBuf::from("/old/a.txt"))],
        active_tab: Some(TabKey::Diff(PathBuf::from("/old/a.txt"))),
        closed_tabs: vec![TabKey::Diff(PathBuf::from("/old/b.txt"))],
        draft: Some(Draft { kind: DraftKind::NewFile, dir: PathBuf::from("/old"), target: None, text: "x".into() }),
        ctx_menu: Some(ContextMenu { target: None, confirm_delete: false }),
        changes_panel_open: true,
        search_query: "needle".into(),
        search_in_progress: true,
        search_last_query: "needle".into(),
        search_results: vec![SearchResult {
            path: PathBuf::from("/old/a.txt"),
            hit: SearchHit { line: 0, col: 0, preview: "x".into(), preview_col: 0 },
            query_len_chars: 6,
        }],
        search_elapsed: Duration::from_millis(5),
        toasts: vec![Toast { id: 0, kind: ToastKind::Success, message: "old".into(), created_at: Instant::now() }],
        flash: Some(Flash { text: "old".into(), created_at: Instant::now() }),
        lsp_status: LspStatus::Ready,
        ..State::default()
    };

    reset_project_scoped_state(&mut state);

    assert!(state.open_tabs.is_empty());
    assert!(state.active_tab.is_none());
    assert!(state.closed_tabs.is_empty());
    assert!(state.draft.is_none());
    assert!(state.ctx_menu.is_none());
    assert!(!state.changes_panel_open);
    assert!(state.search_query.is_empty());
    assert!(!state.search_in_progress);
    assert!(state.search_last_query.is_empty());
    assert!(state.search_results.is_empty());
    assert_eq!(state.search_elapsed, Duration::ZERO);
    assert!(state.toasts.is_empty());
    assert!(state.flash.is_none());
    assert!(matches!(state.lsp_status, LspStatus::Starting));
}

#[test]
fn close_project_returns_to_the_welcome_screen() {
    let mut state = State {
        welcome_open: false,
        root: PathBuf::from("/some/project"),
        tree: vec![Node::File { name: "a.txt".into(), path: PathBuf::from("/some/project/a.txt"), lang: fs_tree::Lang::Other }],
        projects_open: true,
        ..State::default()
    };

    close_project(&mut state);

    assert!(state.welcome_open);
    assert_eq!(state.root, PathBuf::new());
    assert!(state.tree.is_empty());
    assert!(!state.projects_open);
}

#[test]
fn begin_untitled_buffer_gives_each_call_a_distinct_name_and_focuses_it() {
    let mut state = State::default();

    begin_untitled_buffer(&mut state);
    let first = state.active_tab.clone();
    begin_untitled_buffer(&mut state);
    let second = state.active_tab.clone();

    assert_ne!(first, second, "two untitled buffers must not collide on tab identity");
    assert_eq!(first, Some(TabKey::File(PathBuf::from("Untitled-1"))));
    assert_eq!(second, Some(TabKey::File(PathBuf::from("Untitled-2"))));
    assert_eq!(state.open_tabs.len(), 2);

    let editor = find_editor(&state, &PathBuf::from("Untitled-2")).expect("just-created buffer must be findable");
    assert!(editor.document.path().is_none(), "a fresh untitled buffer has no real path yet");
    assert!(!editor.document.is_dirty(), "an empty, untouched buffer isn't dirty");
}

#[test]
fn typing_into_an_untitled_buffer_marks_it_dirty() {
    let mut state = State::default();
    begin_untitled_buffer(&mut state);

    let editor = find_editor_mut(&mut state, &PathBuf::from("Untitled-1")).unwrap();
    editor.document.insert(0, "hello");

    assert!(editor.document.is_dirty());
}

#[test]
fn saving_an_untitled_buffer_does_not_error_it_just_has_nothing_to_write_to_yet() {
    // Before the Save As branch existed, `save_current_file` called
    // `document.save()` unconditionally, which errors with "document has
    // no path" for an untitled buffer — surfaced as an error toast. This
    // proves that path is no longer taken: no toast, dirty state
    // untouched, path still unset (the real dialog is what would resolve
    // that, and isn't invoked by this call itself — see `save_file_as`).
    let mut state = State::default();
    begin_untitled_buffer(&mut state);
    let editor = find_editor_mut(&mut state, &PathBuf::from("Untitled-1")).unwrap();
    editor.document.insert(0, "hello");

    let _ = save_current_file(&mut state);

    assert!(state.toasts.is_empty(), "no error toast — this should hand off to Save As instead");
    let editor = find_editor(&state, &PathBuf::from("Untitled-1")).unwrap();
    assert!(editor.document.is_dirty(), "still unsaved — nothing actually wrote yet");
    assert!(editor.document.path().is_none());
}

#[test]
fn complete_save_as_repoints_the_tab_and_writes_the_content_in_place() {
    let files = TempFiles::new("save-as");
    let new_path = files.dir.join("saved.rs");

    let mut state = State::default();
    begin_untitled_buffer(&mut state);
    let old_path = PathBuf::from("Untitled-1");
    {
        let editor = find_editor_mut(&mut state, &old_path).unwrap();
        editor.document.insert(0, "fn main() {}");
        editor.cursor = CursorPos { line: 0, col: 5 };
    }

    complete_save_as(&mut state, old_path.clone(), new_path.clone());

    assert!(find_editor(&state, &old_path).is_none(), "the synthetic-path identity must be gone, not duplicated");
    let editor = find_editor(&state, &new_path).expect("the tab must now be findable at its real path");
    assert_eq!(editor.document.path(), Some(new_path.as_path()));
    assert!(!editor.document.is_dirty(), "a successful save clears dirty");
    assert_eq!(editor.cursor, CursorPos { line: 0, col: 5 }, "cursor position must survive the repoint, not reset");
    assert_eq!(state.active_tab, Some(TabKey::File(new_path.clone())), "the active tab must follow the repoint");
    assert_eq!(
        std::fs::read_to_string(&new_path).unwrap(),
        "fn main() {}",
        "the content must actually be written to the new real path"
    );
}

#[test]
fn begin_draft_is_a_noop_on_the_welcome_screen() {
    // Regression test for a latent gap found while building this feature:
    // `⌘N` wasn't gated on a project being open, so it could start an
    // invisible draft targeting `state.root` (`PathBuf::new()` while
    // `welcome_open`), which would write into the process's CWD if ever
    // committed.
    let mut state = State { welcome_open: true, ..State::default() };

    let _ = update(&mut state, Message::BeginDraft(DraftKind::NewFile));

    assert!(state.draft.is_none(), "no draft should start while no project is open");
}

#[test]
fn delete_path_removes_a_file_and_closes_its_open_tab() {
    let files = TempFiles::new("delete-file");
    let mut state = State::default();
    open_or_focus_file(&mut state, files.a.clone());
    assert_eq!(state.active_tab, Some(TabKey::File(files.a.clone())), "sanity: the tab opened");

    delete_path(&mut state, files.a.clone());

    assert!(!files.a.exists(), "the file must actually be removed from disk");
    assert!(find_editor(&state, &files.a).is_none(), "the tab for the deleted file must be closed");
    assert!(state.ctx_menu.is_none(), "the context menu should close after a delete");
    assert!(
        state.flash.as_ref().is_some_and(|f| f.text.contains("DELETED")),
        "a successful delete should fire the flash pill"
    );
}

#[test]
fn delete_path_on_a_directory_removes_it_recursively_and_closes_tabs_under_it() {
    let files = TempFiles::new("delete-dir");
    let sub = files.dir.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    let nested = sub.join("nested.txt");
    std::fs::write(&nested, "nested").unwrap();

    let mut state = State::default();
    open_or_focus_file(&mut state, nested.clone());
    assert!(find_editor(&state, &nested).is_some(), "sanity: the nested file's tab opened");

    delete_path(&mut state, sub.clone());

    assert!(!sub.exists(), "the whole directory must be removed");
    assert!(find_editor(&state, &nested).is_none(), "a tab for a file under the deleted directory must be closed too");
}

#[test]
fn delete_path_on_a_missing_path_toasts_an_error_instead_of_panicking() {
    let mut state = State::default();
    let missing = std::env::temp_dir().join("devscribe-delete-test-does-not-exist");

    delete_path(&mut state, missing);

    assert!(
        state.toasts.iter().any(|t| t.kind == ToastKind::Error),
        "a delete that can't happen should surface as an error toast"
    );
}

// --- AI Chat Assist: state/message handling (no real subprocess — that
// end of the pipeline is proven separately, by `devscribe_core::
// claude_agent`'s own `#[ignore]`d end-to-end test against the real
// `claude` CLI). What's exercised here is purely the reducer logic: does
// `update()` translate `ClaudeEvent`s and UI actions into the right
// `ChatThread`/`ChatMode` changes.

#[test]
fn chat_toggle_opens_as_a_tab_from_fully_closed() {
    let mut state = State { chat_mode: ChatMode::Closed, chat_tab_open: false, ..State::default() };
    let _ = update(&mut state, Message::ChatToggle);
    assert_eq!(state.chat_mode, ChatMode::Closed);
    assert!(state.chat_tab_open);
    assert_eq!(state.active_tab, Some(TabKey::Chat));
}

#[test]
fn chat_toggle_closes_from_any_open_presentation() {
    for mode in [ChatMode::Docked, ChatMode::Collapsed] {
        let mut state = State { chat_mode: mode, ..State::default() };
        let _ = update(&mut state, Message::ChatToggle);
        assert_eq!(state.chat_mode, ChatMode::Closed, "toggling off from {mode:?} should fully close");
    }
}

#[test]
fn chat_toggle_from_tab_mode_closes_both_instead_of_leaving_a_dual_presentation() {
    // Opening as a tab sets `chat_mode` to `Closed` already (see
    // `Message::ChatOpenTab`), so a naive "flip chat_mode" toggle would
    // turn it back to `Docked` while `chat_tab_open` was still `true` —
    // showing the panel both docked *and* as a tab at once.
    let mut state = State { chat_mode: ChatMode::Closed, chat_tab_open: true, ..State::default() };
    assert!(chat_is_active(&state));

    let _ = update(&mut state, Message::ChatToggle);

    assert_eq!(state.chat_mode, ChatMode::Closed);
    assert!(!state.chat_tab_open);
    assert!(!chat_is_active(&state));
}

#[test]
fn chat_is_active_true_while_open_as_a_tab_even_though_chat_mode_is_closed() {
    let state = State { chat_mode: ChatMode::Closed, chat_tab_open: true, ..State::default() };
    assert!(chat_is_active(&state), "a live session as a tab must still count as active");
}

#[test]
fn chat_open_tab_switches_presentation_and_focuses_the_chat_tab() {
    let mut state = State::default();
    let _ = update(&mut state, Message::ChatOpenTab);
    assert!(state.chat_tab_open);
    assert_eq!(state.chat_mode, ChatMode::Closed);
    assert_eq!(state.active_tab, Some(TabKey::Chat));
}

#[test]
fn chat_dock_from_tab_returns_to_docked_and_refocuses_away_from_the_chat_tab() {
    let mut state = State {
        chat_tab_open: true,
        chat_mode: ChatMode::Closed,
        active_tab: Some(TabKey::Chat),
        ..State::default()
    };
    let _ = update(&mut state, Message::ChatDockFromTab);
    assert!(!state.chat_tab_open);
    assert_eq!(state.chat_mode, ChatMode::Docked);
    assert_ne!(state.active_tab, Some(TabKey::Chat));
}

#[test]
fn chat_toggle_view_menu_flips_the_field() {
    let mut state = State::default();
    assert!(!state.chat_view_menu_open);

    let _ = update(&mut state, Message::ChatToggleViewMenu);
    assert!(state.chat_view_menu_open);

    let _ = update(&mut state, Message::ChatToggleViewMenu);
    assert!(!state.chat_view_menu_open);
}

/// The "View" popup now offers every destination from every view,
/// including switching straight to Collapsed from Tab — so `ChatCollapse`
/// (not just `ChatDockFromTab`) must also leave tab presentation cleanly,
/// the same correction `chat_dock_from_tab_*` exercises for docking.
/// Otherwise picking "Collapse" from Tab's own menu would show the chat
/// both as a tab and as the collapsed rail at once.
#[test]
fn chat_collapse_leaves_tab_presentation_cleanly_when_triggered_from_a_tab() {
    let mut state = State { chat_tab_open: true, chat_mode: ChatMode::Closed, active_tab: Some(TabKey::Chat), ..State::default() };
    let _ = update(&mut state, Message::ChatCollapse);
    assert!(!state.chat_tab_open, "ChatCollapse should clear chat_tab_open");
    assert_ne!(state.active_tab, Some(TabKey::Chat), "ChatCollapse should refocus away from the now-closed chat tab");
}

#[test]
fn every_view_switching_message_closes_the_view_menu() {
    for message in [Message::ChatDock, Message::ChatCollapse, Message::ChatOpenTab, Message::ChatDockFromTab] {
        let mut state = State { chat_view_menu_open: true, ..State::default() };
        let _ = update(&mut state, message.clone());
        assert!(!state.chat_view_menu_open, "{message:?} should close the view menu");
    }
}

#[test]
fn chat_ready_event_installs_the_sender_without_touching_the_transcript() {
    // `Ready` no longer resets the thread — `SessionStarting` does that,
    // specifically *before* a resume's history-replay events arrive (see
    // the test below), so `Ready` arriving afterward must leave whatever
    // was just replayed alone.
    let mut state = State::default();
    state.chat.messages.push(ChatMessage::Assistant { text: "replayed history".to_string(), streaming: false });
    let (tx, _rx) = mpsc::channel::<ClaudeCommand>(4);

    let _ = update(&mut state, Message::Chat(ClaudeEvent::Ready(tx)));

    assert!(state.chat.sender.is_some());
    assert_eq!(state.chat.status, ChatStatus::Ready);
    assert_eq!(state.chat.messages.len(), 1, "Ready must not clear a transcript that was just replayed");
}

#[test]
fn chat_session_starting_event_clears_any_prior_thread() {
    let mut state = State::default();
    state.chat.messages.push(ChatMessage::Assistant { text: "leftover from a previous worker instance".to_string(), streaming: false });
    state.chat.cost_usd = 1.23;

    let _ = update(&mut state, Message::Chat(ClaudeEvent::SessionStarting));

    assert!(state.chat.messages.is_empty());
    assert_eq!(state.chat.cost_usd, 0.0);
    assert!(state.chat.sender.is_none());
}

#[test]
fn chat_operator_text_event_appends_a_transcript_entry() {
    let mut state = State::default();
    let _ = update(&mut state, Message::Chat(ClaudeEvent::OperatorText("what does this do?".to_string())));
    assert!(matches!(state.chat.messages.as_slice(), [ChatMessage::Operator(t)] if t == "what does this do?"));
}

#[test]
fn chat_session_init_records_session_id_and_model() {
    let mut state = State::default();
    let _ = update(
        &mut state,
        Message::Chat(ClaudeEvent::SessionInit { session_id: "sess-1".to_string(), model: "claude-sonnet-5".to_string() }),
    );
    assert_eq!(state.chat.session_id.as_deref(), Some("sess-1"));
    assert_eq!(state.chat.model.as_deref(), Some("claude-sonnet-5"));
}

#[test]
fn chat_assistant_text_appends_a_transcript_entry() {
    let mut state = State::default();
    let _ = update(&mut state, Message::Chat(ClaudeEvent::AssistantText("hello".to_string())));
    assert!(matches!(state.chat.messages.as_slice(), [ChatMessage::Assistant { text, streaming: false }] if text == "hello"));
}

#[test]
fn chat_assistant_text_delta_accumulates_into_one_streaming_bubble() {
    let mut state = State::default();
    let _ = update(&mut state, Message::Chat(ClaudeEvent::AssistantTextDelta("Hel".to_string())));
    let _ = update(&mut state, Message::Chat(ClaudeEvent::AssistantTextDelta("lo".to_string())));
    assert!(
        matches!(state.chat.messages.as_slice(), [ChatMessage::Assistant { text, streaming: true }] if text == "Hello"),
        "deltas should accumulate into a single still-streaming bubble, got {:?}",
        state.chat.messages
    );

    // The block's final `AssistantText` finalizes that same bubble rather
    // than appending a duplicate.
    let _ = update(&mut state, Message::Chat(ClaudeEvent::AssistantText("Hello".to_string())));
    assert!(
        matches!(state.chat.messages.as_slice(), [ChatMessage::Assistant { text, streaming: false }] if text == "Hello"),
        "AssistantText should finalize the streamed bubble in place, got {:?}",
        state.chat.messages
    );
}

#[test]
fn chat_assistant_text_delta_after_a_finalized_bubble_starts_a_new_one() {
    let mut state = State::default();
    let _ = update(&mut state, Message::Chat(ClaudeEvent::AssistantText("first".to_string())));
    let _ = update(&mut state, Message::Chat(ClaudeEvent::AssistantTextDelta("sec".to_string())));

    assert!(matches!(
        state.chat.messages.as_slice(),
        [
            ChatMessage::Assistant { text: first, streaming: false },
            ChatMessage::Assistant { text: second, streaming: true },
        ] if first == "first" && second == "sec"
    ));
}

#[test]
fn chat_tool_use_and_result_correlate_by_id_into_one_entry() {
    let mut state = State::default();
    let _ = update(
        &mut state,
        Message::Chat(ClaudeEvent::ToolUseStarted {
            id: "toolu_1".to_string(),
            name: "Read".to_string(),
            input: serde_json::json!({"file_path": "/tmp/x.rs"}),
        }),
    );
    let _ = update(&mut state, Message::Chat(ClaudeEvent::ToolResult { id: "toolu_1".to_string(), is_error: false }));

    assert_eq!(state.chat.messages.len(), 1, "the result should update the existing entry, not add a second one");
    let ChatMessage::Tool(tool) = &state.chat.messages[0] else { panic!("expected a Tool entry") };
    assert_eq!(tool.name, "Read");
    assert!(tool.permission.is_none(), "Read never needed a permission decision");
    let result = tool.result.as_ref().expect("result should be attached");
    assert!(!result.is_error);
}

#[test]
fn chat_history_truncated_flag_is_set_then_cleared_once_full_history_loads() {
    let mut state = State::default();
    let _ = update(&mut state, Message::Chat(ClaudeEvent::HistoryTruncated));
    assert!(state.chat.history_truncated, "the resumed session's capped replay must flag more history exists");

    // Only the two capped lines happened to be replayed live; the "full"
    // load below stands in for what a bigger, uncapped re-read would
    // return — an earlier operator line the capped replay never saw.
    let _ = update(&mut state, Message::Chat(ClaudeEvent::OperatorText("recent".to_string())));
    assert_eq!(state.chat.messages.len(), 1);

    let _ = update(
        &mut state,
        Message::ChatFullHistoryLoaded(vec![
            ClaudeEvent::OperatorText("earlier".to_string()),
            ClaudeEvent::OperatorText("recent".to_string()),
        ]),
    );

    assert!(!state.chat.history_truncated, "loading the full history must clear the flag");
    assert_eq!(state.chat.messages.len(), 2, "the full replay must replace, not append to, the capped one");
    assert!(matches!(&state.chat.messages[0], ChatMessage::Operator(t) if t == "earlier"));
    assert!(matches!(&state.chat.messages[1], ChatMessage::Operator(t) if t == "recent"));
}

#[test]
fn chat_permission_request_marks_the_matching_tool_pending() {
    let mut state = State::default();
    let _ = update(
        &mut state,
        Message::Chat(ClaudeEvent::ToolUseStarted {
            id: "toolu_2".to_string(),
            name: "Edit".to_string(),
            input: serde_json::json!({"file_path": "/tmp/x.rs"}),
        }),
    );
    let _ = update(
        &mut state,
        Message::Chat(ClaudeEvent::PermissionRequest {
            id: "toolu_2".to_string(),
            tool_name: "Edit".to_string(),
            tool_input: serde_json::json!({"file_path": "/tmp/x.rs"}),
        }),
    );

    assert_eq!(state.chat.messages.len(), 1);
    let ChatMessage::Tool(tool) = &state.chat.messages[0] else { panic!("expected a Tool entry") };
    assert_eq!(tool.permission, Some(PermissionState::Pending));
}

#[test]
fn chat_permission_request_with_no_prior_tool_use_still_surfaces_defensively() {
    // Every real capture has `ToolUseStarted` arrive first, but the handler
    // shouldn't silently drop a pending permission if that's ever not true.
    let mut state = State::default();
    let _ = update(
        &mut state,
        Message::Chat(ClaudeEvent::PermissionRequest {
            id: "toolu_3".to_string(),
            tool_name: "Write".to_string(),
            tool_input: serde_json::json!({"file_path": "/tmp/y.rs"}),
        }),
    );
    assert_eq!(state.chat.messages.len(), 1);
    let ChatMessage::Tool(tool) = &state.chat.messages[0] else { panic!("expected a Tool entry") };
    assert_eq!(tool.name, "Write");
    assert_eq!(tool.permission, Some(PermissionState::Pending));
}

#[test]
fn chat_approve_permission_records_the_decision_and_forwards_it_to_the_worker() {
    let mut state = State::default();
    let (tx, mut rx) = mpsc::channel::<ClaudeCommand>(4);
    state.chat.sender = Some(tx);
    state.chat.messages.push(ChatMessage::Tool(ToolActivity {
        id: "toolu_4".to_string(),
        name: "Edit".to_string(),
        input: serde_json::Value::Null,
        permission: Some(PermissionState::Pending),
        result: None,
    }));

    let _ = update(&mut state, Message::ChatApprovePermission("toolu_4".to_string()));

    let ChatMessage::Tool(tool) = &state.chat.messages[0] else { panic!("expected a Tool entry") };
    assert_eq!(tool.permission, Some(PermissionState::Approved));
    match rx.try_recv() {
        Ok(ClaudeCommand::RespondPermission { id, approve, .. }) => {
            assert_eq!(id, "toolu_4");
            assert!(approve);
        }
        other => panic!("expected RespondPermission{{approve: true}}, got {other:?}"),
    }
}

#[test]
fn chat_deny_permission_records_the_decision_and_forwards_it_to_the_worker() {
    let mut state = State::default();
    let (tx, mut rx) = mpsc::channel::<ClaudeCommand>(4);
    state.chat.sender = Some(tx);
    state.chat.messages.push(ChatMessage::Tool(ToolActivity {
        id: "toolu_5".to_string(),
        name: "Edit".to_string(),
        input: serde_json::Value::Null,
        permission: Some(PermissionState::Pending),
        result: None,
    }));

    let _ = update(&mut state, Message::ChatDenyPermission("toolu_5".to_string()));

    let ChatMessage::Tool(tool) = &state.chat.messages[0] else { panic!("expected a Tool entry") };
    assert_eq!(tool.permission, Some(PermissionState::Denied));
    match rx.try_recv() {
        Ok(ClaudeCommand::RespondPermission { id, approve, .. }) => {
            assert_eq!(id, "toolu_5");
            assert!(!approve);
        }
        other => panic!("expected RespondPermission{{approve: false}}, got {other:?}"),
    }
}

#[test]
fn chat_turn_result_accumulates_cost_and_replaces_token_counts() {
    let mut state = State::default();
    let _ = update(&mut state, Message::Chat(ClaudeEvent::TurnResult { cost_usd: 0.01, input_tokens: 100, output_tokens: 20 }));
    let _ = update(&mut state, Message::Chat(ClaudeEvent::TurnResult { cost_usd: 0.02, input_tokens: 150, output_tokens: 40 }));

    assert!((state.chat.cost_usd - 0.03).abs() < f64::EPSILON, "cost should accumulate across turns");
    assert_eq!(state.chat.input_tokens, 150, "token counts reflect the latest turn, not a running total");
    assert_eq!(state.chat.output_tokens, 40);
}

#[test]
fn chat_unavailable_event_clears_the_sender_and_records_the_reason() {
    let mut state = State::default();
    let (tx, _rx) = mpsc::channel::<ClaudeCommand>(4);
    state.chat.sender = Some(tx);

    let _ = update(&mut state, Message::Chat(ClaudeEvent::Unavailable("claude not found".to_string())));

    assert!(state.chat.sender.is_none());
    assert_eq!(state.chat.status, ChatStatus::Unavailable("claude not found".to_string()));
}

#[test]
fn chat_submit_pushes_an_operator_message_clears_the_draft_and_sends_the_prompt() {
    let mut state = State::default();
    let (tx, mut rx) = mpsc::channel::<ClaudeCommand>(4);
    state.chat.sender = Some(tx);
    state.chat.input = iced::widget::text_editor::Content::with_text("  explain this bug  ");

    let _ = update(&mut state, Message::ChatSubmit);

    assert!(state.chat.input.is_empty());
    assert!(matches!(state.chat.messages.as_slice(), [ChatMessage::Operator(text)] if text == "explain this bug"));
    match rx.try_recv() {
        Ok(ClaudeCommand::SendPrompt(text)) => assert_eq!(text, "explain this bug"),
        other => panic!("expected SendPrompt, got {other:?}"),
    }
}

#[test]
fn chat_submit_with_no_live_session_does_nothing() {
    let mut state = State::default();
    state.chat.input = iced::widget::text_editor::Content::with_text("hello");
    let _ = update(&mut state, Message::ChatSubmit);
    assert_eq!(state.chat.input.text(), "hello", "nothing to send to, so the draft should be left alone");
    assert!(state.chat.messages.is_empty());
}

#[test]
fn chat_submit_ignores_a_blank_draft() {
    let mut state = State::default();
    let (tx, _rx) = mpsc::channel::<ClaudeCommand>(4);
    state.chat.sender = Some(tx);
    state.chat.input = iced::widget::text_editor::Content::with_text("   ");
    let _ = update(&mut state, Message::ChatSubmit);
    assert!(state.chat.messages.is_empty());
}

#[test]
fn chat_toggle_actions_flips_the_field() {
    let mut state = State::default();
    assert!(!state.chat_actions_open);
    let _ = update(&mut state, Message::ChatToggleActions);
    assert!(state.chat_actions_open);
    let _ = update(&mut state, Message::ChatToggleActions);
    assert!(!state.chat_actions_open);
}

#[test]
fn chat_show_model_and_usage_are_no_ops_with_no_live_session() {
    for message in [Message::ChatShowModel, Message::ChatShowUsage] {
        let mut state = State { chat_actions_open: true, ..State::default() };
        let _ = update(&mut state, message);
        assert!(state.chat.messages.is_empty(), "nothing to send to, so no operator entry should appear");
        assert!(!state.chat_actions_open, "should still close the popup even when it can't send");
    }
}

#[test]
fn chat_show_model_pushes_an_operator_entry_and_sends_the_slash_command() {
    let mut state = State { chat_actions_open: true, ..State::default() };
    let (tx, mut rx) = mpsc::channel::<ClaudeCommand>(4);
    state.chat.sender = Some(tx);

    let _ = update(&mut state, Message::ChatShowModel);

    assert!(!state.chat_actions_open);
    assert!(matches!(state.chat.messages.as_slice(), [ChatMessage::Operator(text)] if text == "/model"));
    match rx.try_recv() {
        Ok(ClaudeCommand::SendPrompt(text)) => assert_eq!(text, "/model"),
        other => panic!("expected SendPrompt(\"/model\"), got {other:?}"),
    }
}

#[test]
fn chat_show_usage_sends_the_usage_slash_command() {
    let mut state = State::default();
    let (tx, mut rx) = mpsc::channel::<ClaudeCommand>(4);
    state.chat.sender = Some(tx);

    let _ = update(&mut state, Message::ChatShowUsage);

    assert!(matches!(state.chat.messages.as_slice(), [ChatMessage::Operator(text)] if text == "/usage"));
    match rx.try_recv() {
        Ok(ClaudeCommand::SendPrompt(text)) => assert_eq!(text, "/usage"),
        other => panic!("expected SendPrompt(\"/usage\"), got {other:?}"),
    }
}

#[test]
fn chat_toggle_thinking_flips_the_field_closes_the_popup_and_sends_effort() {
    let mut state = State { chat_actions_open: true, ..State::default() };
    let (tx, mut rx) = mpsc::channel::<ClaudeCommand>(4);
    state.chat.sender = Some(tx);
    assert!(!state.chat_thinking_enabled, "sanity: off by default");

    let _ = update(&mut state, Message::ChatToggleThinking);
    assert!(state.chat_thinking_enabled);
    assert!(!state.chat_actions_open);
    match rx.try_recv() {
        Ok(ClaudeCommand::SendPrompt(text)) => assert_eq!(text, "/effort high"),
        other => panic!("expected SendPrompt(\"/effort high\"), got {other:?}"),
    }

    let _ = update(&mut state, Message::ChatToggleThinking);
    assert!(!state.chat_thinking_enabled);
    match rx.try_recv() {
        Ok(ClaudeCommand::SendPrompt(text)) => assert_eq!(text, "/effort auto"),
        other => panic!("expected SendPrompt(\"/effort auto\"), got {other:?}"),
    }
}

#[test]
fn chat_toggle_thinking_still_flips_locally_with_no_live_session() {
    let mut state = State::default();
    let _ = update(&mut state, Message::ChatToggleThinking);
    assert!(state.chat_thinking_enabled, "the button's own on/off state should still track clicks even before a session exists");
}

#[test]
fn chat_toggle_shell_access_flips_the_field_and_closes_the_popup() {
    let mut state = State { chat_actions_open: true, ..State::default() };
    assert!(!state.chat_shell_access_enabled, "sanity: off by default");

    let _ = update(&mut state, Message::ChatToggleShellAccess);
    assert!(state.chat_shell_access_enabled);
    assert!(!state.chat_actions_open);

    let _ = update(&mut state, Message::ChatToggleShellAccess);
    assert!(!state.chat_shell_access_enabled);
}

#[test]
fn chat_file_dialog_result_mentions_a_project_file_relative_to_root() {
    let mut state = State { root: PathBuf::from("/some/project"), ..State::default() };
    let _ = update(&mut state, Message::ChatFileDialogResult(Some(PathBuf::from("/some/project/src/engine.rs")), true));
    assert_eq!(state.chat.input.text(), "@src/engine.rs ");
}

#[test]
fn chat_file_dialog_result_attaches_by_absolute_path_when_not_relative() {
    let mut state = State { root: PathBuf::from("/some/project"), ..State::default() };
    let _ = update(&mut state, Message::ChatFileDialogResult(Some(PathBuf::from("/elsewhere/notes.txt")), false));
    assert_eq!(state.chat.input.text(), "@/elsewhere/notes.txt ");
}

#[test]
fn chat_file_dialog_result_falls_back_to_absolute_when_outside_the_project() {
    let mut state = State { root: PathBuf::from("/some/project"), ..State::default() };
    let _ = update(&mut state, Message::ChatFileDialogResult(Some(PathBuf::from("/elsewhere/notes.txt")), true));
    assert_eq!(state.chat.input.text(), "@/elsewhere/notes.txt ");
}

#[test]
fn chat_file_dialog_result_appends_with_a_leading_space_to_an_existing_draft() {
    let mut state = State { root: PathBuf::from("/some/project"), ..State::default() };
    state.chat.input = iced::widget::text_editor::Content::with_text("look at");
    let _ = update(&mut state, Message::ChatFileDialogResult(Some(PathBuf::from("/some/project/README.md")), true));
    assert_eq!(state.chat.input.text(), "look at @README.md ");
}

#[test]
fn chat_file_dialog_result_is_a_no_op_when_the_dialog_was_cancelled() {
    let mut state = State { root: PathBuf::from("/some/project"), ..State::default() };
    let _ = update(&mut state, Message::ChatFileDialogResult(None, true));
    assert!(state.chat.input.text().is_empty());
}

#[test]
fn chat_resize_dragged_computes_width_from_the_right_edge_and_clamps() {
    let mut state = State { chat_resizing: true, window_width: 1280.0, ..State::default() };

    let _ = update(&mut state, Message::ChatResizeDragged(1000.0));
    assert_eq!(state.chat_panel_width, 280.0, "1280 - 1000 == 280, within bounds");

    let _ = update(&mut state, Message::ChatResizeDragged(0.0));
    assert_eq!(state.chat_panel_width, CHAT_MAX_WIDTH, "1280 - 0 would exceed the max, so it clamps");

    let _ = update(&mut state, Message::ChatResizeDragged(1200.0));
    assert_eq!(state.chat_panel_width, CHAT_MIN_WIDTH, "1280 - 1200 would be under the min, so it clamps");
}

#[test]
fn chat_resize_dragged_is_ignored_when_not_resizing() {
    let mut state = State { chat_resizing: false, window_width: 1280.0, chat_panel_width: 340.0, ..State::default() };
    let _ = update(&mut state, Message::ChatResizeDragged(1000.0));
    assert_eq!(state.chat_panel_width, 340.0);
}

#[test]
fn chat_new_session_picks_a_fresh_id_and_clears_the_transcript() {
    let mut state = State::default();
    let old_id = state.chat_session_id.clone();
    state.chat.messages.push(ChatMessage::Assistant { text: "old conversation".to_string(), streaming: false });
    state.chat_sessions_open = true;

    let _ = update(&mut state, Message::ChatNewSession);

    assert_ne!(state.chat_session_id, old_id, "should be a genuinely new id, not reuse the old one");
    assert!(!state.chat_session_id.is_empty());
    assert!(state.chat.messages.is_empty());
    assert!(!state.chat_sessions_open, "picking a session (new or resumed) should close the picker");
}

#[test]
fn chat_new_session_also_closes_the_actions_popup() {
    // "Clear conversation" in the Actions popup is just `ChatNewSession` —
    // it should close that popup like every other action in it does,
    // rather than leaving it floating over the now-empty transcript.
    let mut state = State { chat_actions_open: true, ..State::default() };
    let _ = update(&mut state, Message::ChatNewSession);
    assert!(!state.chat_actions_open);
}

#[test]
fn chat_resume_session_switches_the_target_id_and_clears_the_transcript() {
    let mut state = State::default();
    state.chat.messages.push(ChatMessage::Assistant { text: "wrong conversation".to_string(), streaming: false });
    state.chat_sessions_open = true;

    let _ = update(&mut state, Message::ChatResumeSession("some-other-session-id".to_string()));

    assert_eq!(state.chat_session_id, "some-other-session-id");
    assert!(state.chat.messages.is_empty());
    assert!(!state.chat_sessions_open);
}

#[test]
fn chat_toggle_sessions_flips_the_picker_open_flag() {
    let mut state = State::default();
    assert!(!state.chat_sessions_open);
    let _ = update(&mut state, Message::ChatToggleSessions);
    assert!(state.chat_sessions_open);
}

#[test]
fn chat_sessions_loaded_replaces_the_list() {
    let mut state = State::default();
    let sessions = vec![devscribe_core::claude_agent::SessionSummary {
        id: "s1".to_string(),
        title: "A past chat".to_string(),
        last_active: std::time::SystemTime::now(),
    }];
    let _ = update(&mut state, Message::ChatSessionsLoaded(sessions.clone()));
    assert_eq!(state.chat_sessions, sessions);
}

#[test]
fn chat_set_permission_mode_updates_the_field() {
    let mut state = State::default();
    assert_eq!(state.chat_permission_mode, devscribe_core::claude_agent::PermissionMode::Manual, "sanity: the documented default");

    let _ = update(&mut state, Message::ChatSetPermissionMode(devscribe_core::claude_agent::PermissionMode::Plan));

    assert_eq!(state.chat_permission_mode, devscribe_core::claude_agent::PermissionMode::Plan);
}

#[test]
fn chat_input_action_edits_the_draft_content() {
    use iced::widget::text_editor;

    let mut state = State::default();
    assert!(state.chat.input.is_empty());

    let _ = update(&mut state, Message::ChatInputAction(text_editor::Action::Edit(text_editor::Edit::Insert('h'))));
    let _ = update(&mut state, Message::ChatInputAction(text_editor::Action::Edit(text_editor::Edit::Insert('i'))));

    assert_eq!(state.chat.input.text(), "hi");
}

#[test]
fn chat_input_action_enter_inserts_a_newline_not_a_submit() {
    // Shift+Enter's actual key detection lives in `chat_panel::input_bar`'s
    // `key_binding` closure (a UI-layer concern, not reducer logic) — what
    // belongs here is confirming the reducer side does the right thing
    // once that closure decides a newline is wanted: `Edit::Enter` must
    // insert a line break, not be confused with `Message::ChatSubmit`.
    use iced::widget::text_editor;

    let mut state = State::default();
    let (tx, mut rx) = mpsc::channel::<ClaudeCommand>(4);
    state.chat.sender = Some(tx);

    let _ = update(&mut state, Message::ChatInputAction(text_editor::Action::Edit(text_editor::Edit::Insert('a'))));
    let _ = update(&mut state, Message::ChatInputAction(text_editor::Action::Edit(text_editor::Edit::Enter)));
    let _ = update(&mut state, Message::ChatInputAction(text_editor::Action::Edit(text_editor::Edit::Insert('b'))));

    assert_eq!(state.chat.input.line_count(), 2);
    assert!(state.chat.messages.is_empty(), "inserting a newline must not submit anything");
    assert!(rx.try_recv().is_err(), "nothing should have been sent to the worker");
}

#[test]
fn an_edit_updates_find_immediately_and_the_rest_once_the_buffer_settles() {
    // The split that keeps typing responsive: find matches must track the
    // buffer keystroke by keystroke (their highlight sits on screen), while
    // the tree-sitter reparse and JSON reparse — 33 ms on an 850-line file
    // in a debug build — wait for `EDIT_SETTLE`. A regression in either
    // direction matters: matches going stale is a visible bug, and the
    // reparse creeping back onto the keystroke path is the freeze.
    let dir = std::env::temp_dir().join(format!("devscribe-resync-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("f.json");
    std::fs::write(&path, "{\"a\": 1}").unwrap();

    let mut editor = EditorState::new(Document::open(&path).unwrap(), path.clone());
    editor.find = Some(FindState { query: "a".into(), matches: Vec::new(), current: 0, ..FindState::default() });
    editor.refind();
    assert_eq!(editor.find.as_ref().unwrap().matches.len(), 1);
    assert!(editor.json.as_ref().unwrap().is_ok(), "sanity: valid JSON parses");
    assert!(!editor.highlights.is_empty(), "sanity: .json has a wired grammar");

    // Type a second "a" at the end: adds a find match, and breaks the JSON.
    editor.cursor = editor.document.line_col(editor.document.text().len_chars()).into();
    editor.insert_text("a");

    assert_eq!(
        editor.find.as_ref().unwrap().matches.len(),
        2,
        "find matches must be recomputed against the edited buffer immediately"
    );
    assert!(editor.needs_reparse, "the edit must have armed a deferred reparse");
    assert!(
        editor.json.as_ref().unwrap().is_ok(),
        "the JSON tree is deliberately still the pre-edit one until the buffer settles"
    );

    editor.reparse_now();
    assert!(!editor.needs_reparse);
    assert!(
        editor.json.as_ref().unwrap().is_err(),
        "settling must reparse against the edited buffer"
    );
    assert!(!editor.highlights.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn find_matches_stay_document_ordered_so_the_canvas_can_binary_search_them() {
    // `EditorCanvas::draw` narrows `find_matches` to the visible range with
    // `partition_point`, which is only correct while the list is sorted by
    // position. Nothing else enforces that, so pin it here.
    let mut editor = EditorState::new(
        Document::from_str("token a\nb token\ntoken token\n"),
        PathBuf::from("t.txt"),
    );
    editor.find = Some(FindState { query: "token".into(), matches: Vec::new(), current: 0, ..FindState::default() });
    editor.refind();

    let matches = &editor.find.as_ref().unwrap().matches;
    assert_eq!(matches.len(), 4);
    assert!(
        matches.windows(2).all(|w| w[0].start <= w[1].start && w[0].end <= w[1].end),
        "matches must be ascending in both start and end: {matches:?}"
    );
}

#[test]
fn replace_current_drops_the_replaced_match_so_current_lands_on_the_next_one() {
    // Replacing the active match removes it from `find.matches` on the very
    // next `refind_with`, which shifts every later match's index down by
    // one — that's what should make `find.current` land on "the next match"
    // for free, with no separate advance step in `replace_current` itself.
    let mut editor = EditorState::new(Document::from_str("cat cat cat\n"), PathBuf::from("t.txt"));
    editor.find = Some(FindState {
        query: "cat".into(),
        replace_query: "dog".into(),
        matches: Vec::new(),
        current: 0,
        ..FindState::default()
    });
    editor.refind();
    assert_eq!(editor.find.as_ref().unwrap().matches.len(), 3);

    editor.replace_current();

    assert_eq!(editor.document.text().to_string(), "dog cat cat\n");
    let find = editor.find.as_ref().unwrap();
    assert_eq!(find.matches.len(), 2, "the replaced occurrence must drop out of the results");
    assert_eq!(find.current, 0, "index 0 now points at what used to be the second match");
}

#[test]
fn replace_current_is_a_noop_with_no_active_match() {
    let mut editor = EditorState::new(Document::from_str("cat\n"), PathBuf::from("t.txt"));
    editor.find = Some(FindState {
        query: "dog".into(),
        replace_query: "cat".into(),
        ..FindState::default()
    });
    editor.refind();
    assert!(editor.find.as_ref().unwrap().matches.is_empty());

    editor.replace_current();

    assert_eq!(editor.document.text().to_string(), "cat\n", "nothing to replace, buffer untouched");
}

#[test]
fn replace_all_replaces_every_match_as_a_single_undo_step() {
    let mut editor = EditorState::new(Document::from_str("cat cat cat\n"), PathBuf::from("t.txt"));
    editor.find = Some(FindState {
        query: "cat".into(),
        replace_query: "dog".into(),
        matches: Vec::new(),
        current: 0,
        ..FindState::default()
    });
    editor.refind();

    editor.replace_all();

    assert_eq!(editor.document.text().to_string(), "dog dog dog\n");
    assert!(editor.find.as_ref().unwrap().matches.is_empty(), "no more \"cat\" left to match");

    assert!(editor.undo(), "replace-all must undo in one step");
    assert_eq!(editor.document.text().to_string(), "cat cat cat\n");
}

#[test]
fn markdown_files_get_a_parsed_preview_and_other_files_do_not() {
    let md = EditorState::new(Document::from_str("# Title\n"), PathBuf::from("t.md"));
    assert!(md.markdown.is_some(), ".md files must get a parsed markdown::Content");

    let txt = EditorState::new(Document::from_str("# Title\n"), PathBuf::from("t.txt"));
    assert!(txt.markdown.is_none(), "a non-Markdown file must not get one, even with the same text");
}

#[test]
fn editing_a_markdown_buffer_reparses_the_preview_only_once_settled() {
    let mut editor = EditorState::new(Document::from_str("# Title\n"), PathBuf::from("t.md"));
    let before = editor.markdown.as_ref().unwrap().items().len();

    editor.cursor = editor.document.line_col(editor.document.text().len_chars()).into();
    editor.insert_text("\n\nAnother paragraph.\n");

    assert_eq!(
        editor.markdown.as_ref().unwrap().items().len(),
        before,
        "the preview is deliberately still the pre-edit one until the buffer settles"
    );

    editor.reparse_now();
    assert!(editor.markdown.as_ref().unwrap().items().len() > before, "settling must reparse the edited buffer");
}

#[test]
fn typing_defers_the_expensive_work_and_one_settle_covers_the_whole_burst() {
    // The freeze this guards: a tree-sitter reparse, a `HEAD` blob read and
    // a whole-file LSP `didChange` on *every* keystroke measured ~37 ms per
    // character on an 850-line file in a debug build. A burst of typing must
    // arm the timer once and leave exactly one path queued, not N.
    let files = TempFiles::new("settle");
    let mut state = State::default();
    open_or_focus_file(&mut state, files.a.clone());

    assert!(state.edit_settled_at.is_none(), "nothing pending before any edit");

    for _ in 0..5 {
        update(&mut state, Message::EditorInsertText("x".into()));
    }
    assert!(state.edit_settled_at.is_some(), "typing must arm the settle timer");
    assert_eq!(
        state.pending_edits,
        vec![files.a.clone()],
        "a burst on one file must queue that file once, not once per keystroke"
    );
    assert!(
        find_editor(&state, &files.a).unwrap().needs_reparse,
        "the reparse must still be pending, not already done"
    );

    // A tick before the buffer has settled must not fire.
    update(&mut state, Message::EditSettleTick);
    assert!(state.edit_settled_at.is_some(), "must not flush before EDIT_SETTLE elapses");

    state.edit_settled_at = Some(Instant::now() - EDIT_SETTLE);
    update(&mut state, Message::EditSettleTick);
    assert!(state.edit_settled_at.is_none(), "a settled buffer must flush");
    assert!(state.pending_edits.is_empty());
    assert!(!find_editor(&state, &files.a).unwrap().needs_reparse);
}

#[test]
fn saving_flushes_pending_work_so_the_diff_is_not_left_stale() {
    // Deferred work must never outlive the moment it matters. Save is one
    // such moment: the on-disk file changes, so a diff computed from the
    // pre-edit buffer would be wrong and would stay wrong.
    let files = TempFiles::new("settle-save");
    let mut state = State::default();
    open_or_focus_file(&mut state, files.a.clone());
    update(&mut state, Message::EditorInsertText("x".into()));
    assert!(state.edit_settled_at.is_some());

    let _ = save_current_file(&mut state);
    assert!(state.edit_settled_at.is_none(), "save must flush the settle queue");
    assert!(state.pending_edits.is_empty());
}

#[test]
fn highlight_spans_slide_across_an_edit_instead_of_going_out_of_register() {
    // Between a keystroke and the deferred reparse, spans point into a
    // buffer that has moved. A typing burst never lets the debounce fire, so
    // without this shift the colouring of everything after the cursor would
    // drift further out of alignment with every character typed.
    let mut editor = EditorState::new(
        Document::from_str("let a = 1;
let b = 2;
"),
        PathBuf::from("t.rs"),
    );
    let second_line_byte = editor.document.text().line_to_byte(1);
    let before: Vec<_> = editor
        .highlights
        .iter()
        .filter(|s| s.start >= second_line_byte)
        .copied()
        .collect();
    assert!(!before.is_empty(), "sanity: the second line is highlighted");

    // Insert 3 chars at the very start of the buffer.
    editor.cursor = CursorPos { line: 0, col: 0 };
    editor.insert_text("xyz");

    let after: Vec<_> = editor
        .highlights
        .iter()
        .filter(|s| s.start >= second_line_byte + 3)
        .copied()
        .collect();
    assert_eq!(after.len(), before.len());
    for (b, a) in before.iter().zip(&after) {
        assert_eq!(a.start, b.start + 3, "every span past the edit shifts by the insert");
        assert_eq!(a.end, b.end + 3);
        assert_eq!(a.kind, b.kind);
    }

    // And a deletion pulls them back.
    editor.cursor = CursorPos { line: 0, col: 3 };
    for _ in 0..3 {
        editor.backspace();
    }
    let restored: Vec<_> = editor
        .highlights
        .iter()
        .filter(|s| s.start >= second_line_byte)
        .copied()
        .collect();
    assert_eq!(restored, before, "deleting what was typed must restore the offsets");
}

#[test]
fn breadcrumbs_do_not_touch_the_whole_buffer_on_every_cursor_move() {
    // The regression this guards against: `outline::breadcrumbs_at` reads
    // small ranges straight off the rope via `get_byte_slice`. Materializing
    // the whole buffer here — the mistake `resync_after_edit` was built to
    // stop making — would put a per-frame full-document allocation back on
    // the cursor-move path this whole feature runs on (every `view()`).
    let dir = std::env::temp_dir().join(format!("devscribe-breadcrumb-perf-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("big.rs");
    // A large-ish file so a whole-buffer copy would show up in the timing,
    // not just be lost in noise.
    let body: String = (0..2000).map(|i| format!("fn f{i}() {{\n    let x = {i};\n}}\n")).collect();
    std::fs::write(&path, &body).unwrap();

    let mut editor = EditorState::new(Document::open(&path).unwrap(), path.clone());
    assert!(!editor.needs_reparse, "a fresh EditorState must already have parsed once");
    let last_line = editor.document.line_count() - 1;

    let start = Instant::now();
    for line in 0..last_line {
        editor.cursor = CursorPos { line, col: 4 };
        let _ = std::hint::black_box(editor.breadcrumbs());
    }
    let elapsed = start.elapsed();
    // Generous budget (a real whole-buffer copy here would run into
    // milliseconds per call on a file this size, i.e. seconds total) — this
    // is a regression tripwire, not a tight perf assertion.
    assert!(
        elapsed < std::time::Duration::from_millis(500),
        "computing breadcrumbs for every line took {elapsed:?} — looks like it's touching the whole buffer"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn breadcrumbs_go_empty_mid_edit_and_come_back_after_settling() {
    let dir = std::env::temp_dir().join(format!("devscribe-breadcrumb-settle-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("f.rs");
    std::fs::write(&path, "fn settle_batch() {\n    let a = 1;\n}\n").unwrap();

    let mut editor = EditorState::new(Document::open(&path).unwrap(), path.clone());
    editor.cursor = CursorPos { line: 1, col: 4 };
    assert_eq!(
        editor.breadcrumbs().iter().map(|c| c.label.clone()).collect::<Vec<_>>(),
        vec!["settle_batch".to_string()],
        "sanity: breadcrumbs resolve before any edit"
    );

    editor.insert_text("x");
    assert!(editor.needs_reparse);
    assert!(
        editor.breadcrumbs().is_empty(),
        "must not show a breadcrumb computed against a tree the live cursor may have outrun"
    );

    editor.reparse_now();
    assert_eq!(
        editor.breadcrumbs().iter().map(|c| c.label.clone()).collect::<Vec<_>>(),
        vec!["settle_batch".to_string()],
        "must come back once the tree is back in sync"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Builds an `EditorState` whose `diff`/`gutter_marks`/`hunks` are already
/// populated as if `recompute_diff_for` had run `old` against `new`'s text —
/// the diff view's "revert selected changes" tests don't need a real git
/// repo, just this derived state.
fn editor_with_diff(old: &str, new: &str, path: PathBuf) -> EditorState {
    let mut editor = EditorState::new(Document::from_str(new), path);
    let lines = devscribe_core::diff::diff_lines(old, new);
    let line_count = editor.document.line_count();
    editor.gutter_marks = Rc::new(devscribe_core::diff::gutter_marks(&lines, line_count));
    editor.hunks = Rc::new(devscribe_core::diff::hunks(&lines, line_count));
    editor.diff = DiffStatus::Changed(lines);
    editor
}

#[test]
fn revert_lines_reverts_every_target_as_a_single_undo_step() {
    let mut editor = editor_with_diff("a\nb\nc\nd\ne\n", "a\nx\nc\ny\ne\n", PathBuf::from("t.txt"));
    let targets: Vec<usize> = editor.hunks.iter().flat_map(|h| h.marks.iter().map(|(l, _)| *l)).collect();
    assert_eq!(targets.len(), 2, "sanity: two separate one-line replacements");

    assert!(editor.revert_lines(&targets));
    assert_eq!(editor.document.text().to_string(), "a\nb\nc\nd\ne\n");
    assert_eq!(editor.undo_stack.len(), 1, "a batch revert must be one undo step, not one per hunk");

    assert!(editor.undo());
    assert_eq!(editor.document.text().to_string(), "a\nx\nc\ny\ne\n", "undo should restore the pre-revert buffer in one step");
}

#[test]
fn revert_lines_processes_descending_so_an_earlier_target_does_not_shift_a_later_ones_line_number() {
    // A `RemovedAbove` revert re-inserts a line above its target, shifting
    // every line at or below it down by one. If targets were applied in
    // ascending order, reverting the first (lowest) target here would push
    // the second target's line number out from under it.
    let mut editor = editor_with_diff("a\nb\nc\nd\n", "a\nc\n", PathBuf::from("t.txt"));
    let targets: Vec<usize> = editor.hunks.iter().flat_map(|h| h.marks.iter().map(|(l, _)| *l)).collect();

    assert!(editor.revert_lines(&targets));
    assert_eq!(editor.document.text().to_string(), "a\nb\nc\nd\n");
}

#[test]
fn revert_lines_is_a_noop_for_targets_with_no_mark() {
    let mut editor = editor_with_diff("a\nb\n", "a\nx\n", PathBuf::from("t.txt"));
    assert!(!editor.revert_lines(&[5, 6]), "no such lines have a gutter mark");
    assert_eq!(editor.document.text().to_string(), "a\nx\n");
}

#[test]
fn toggle_diff_hunk_selected_flips_membership() {
    let files = TempFiles::new("diff-toggle");
    let mut state = State::default();
    open_or_focus_file(&mut state, files.a.clone());
    *find_editor_mut(&mut state, &files.a).unwrap() = editor_with_diff("a\n", "x\n", files.a.clone());
    let hunk_id = find_editor(&state, &files.a).unwrap().hunks[0].range.start;

    let _ = update(&mut state, Message::ToggleDiffHunkSelected { path: files.a.clone(), hunk_id });
    assert!(find_editor(&state, &files.a).unwrap().diff_selected_hunks.contains(&hunk_id));

    let _ = update(&mut state, Message::ToggleDiffHunkSelected { path: files.a.clone(), hunk_id });
    assert!(!find_editor(&state, &files.a).unwrap().diff_selected_hunks.contains(&hunk_id));
}

#[test]
fn confirm_revert_selected_hunks_reverts_only_the_checked_hunks() {
    let files = TempFiles::new("diff-revert-selected");
    let mut state = State::default();
    open_or_focus_file(&mut state, files.a.clone());
    *find_editor_mut(&mut state, &files.a).unwrap() =
        editor_with_diff("a\nb\nc\nd\ne\n", "a\nx\nc\ny\ne\n", files.a.clone());
    let path = files.a.clone();
    let first_hunk_id = find_editor(&state, &path).unwrap().hunks[0].range.start;

    let _ = update(&mut state, Message::ToggleDiffHunkSelected { path: path.clone(), hunk_id: first_hunk_id });
    let _ = update(&mut state, Message::PromptRevertSelectedHunks(path.clone()));
    assert!(find_editor(&state, &path).unwrap().pending_hunk_revert);

    let _ = update(&mut state, Message::ConfirmRevertSelectedHunks(path.clone()));

    let editor = find_editor(&state, &path).unwrap();
    assert!(!editor.pending_hunk_revert);
    assert!(editor.diff_selected_hunks.is_empty());
    assert_eq!(
        editor.document.text().to_string(),
        "a\nb\nc\ny\ne\n",
        "only the checked hunk (b -> x) should revert; the other (d -> y) stays"
    );
}

#[test]
fn cancel_revert_selected_hunks_leaves_the_buffer_and_selection_untouched() {
    let files = TempFiles::new("diff-revert-cancel");
    let mut state = State::default();
    open_or_focus_file(&mut state, files.a.clone());
    *find_editor_mut(&mut state, &files.a).unwrap() = editor_with_diff("a\nb\n", "a\nx\n", files.a.clone());
    let path = files.a.clone();
    let hunk_id = find_editor(&state, &path).unwrap().hunks[0].range.start;

    let _ = update(&mut state, Message::ToggleDiffHunkSelected { path: path.clone(), hunk_id });
    let _ = update(&mut state, Message::PromptRevertSelectedHunks(path.clone()));
    let _ = update(&mut state, Message::CancelRevertSelectedHunks(path.clone()));

    let editor = find_editor(&state, &path).unwrap();
    assert!(!editor.pending_hunk_revert);
    assert!(editor.diff_selected_hunks.contains(&hunk_id), "cancel must not clear the checked hunks, only the confirm step");
    assert_eq!(editor.document.text().to_string(), "a\nx\n");
}

/// A `State` primed as if a real project were open at `root` — `State::default()`
/// always builds with `welcome_open: true` in test builds (see `startup`'s
/// `#[cfg(test)]` doc), so session persistence tests flip these two fields
/// by hand afterward rather than going through the real project-loading
/// path.
fn state_with_project_open(root: PathBuf) -> State {
    let mut state = State::default();
    state.welcome_open = false;
    state.root = root;
    state
}

#[test]
fn capture_session_records_open_tabs_active_tab_and_cursor() {
    let files = TempFiles::new("capture");
    let mut state = state_with_project_open(files.a.parent().unwrap().to_path_buf());

    open_or_focus_file(&mut state, files.a.clone());
    find_editor_mut(&mut state, &files.a).unwrap().cursor = CursorPos { line: 0, col: 1 };
    open_or_focus_file(&mut state, files.b.clone());
    open_or_focus_diff(&mut state, files.a.clone());

    let session = capture_session(&state);
    assert_eq!(session.open_tabs.len(), 3);
    assert_eq!(session.open_tabs[0], session::SessionTab { path: files.a.clone(), is_diff: false, cursor_line: 0, cursor_col: 1 });
    assert_eq!(session.open_tabs[1].path, files.b);
    assert!(!session.open_tabs[1].is_diff);
    assert_eq!(session.open_tabs[2], session::SessionTab { path: files.a.clone(), is_diff: true, cursor_line: 0, cursor_col: 0 });
    assert_eq!(session.active_tab, Some(2), "the diff tab, opened last, should be active");
}

#[test]
fn capture_session_skips_untitled_buffers() {
    let files = TempFiles::new("capture-untitled");
    let mut state = state_with_project_open(files.a.parent().unwrap().to_path_buf());

    open_or_focus_file(&mut state, files.a.clone());
    begin_untitled_buffer(&mut state);

    let session = capture_session(&state);
    assert_eq!(session.open_tabs.len(), 1, "an untitled buffer has no disk identity to restore, so it must not be recorded");
    assert_eq!(session.open_tabs[0].path, files.a);
    assert_eq!(session.active_tab, None, "the active tab (untitled) isn't in open_tabs, so there's no index to record");
}

#[test]
fn restore_session_reopens_tabs_places_cursor_and_restores_layout() {
    let files = TempFiles::new("restore");
    let root = files.a.parent().unwrap().to_path_buf();
    session::save(
        &root,
        &session::Session {
            open_tabs: vec![
                session::SessionTab { path: files.a.clone(), is_diff: false, cursor_line: 0, cursor_col: 1 },
                session::SessionTab { path: files.b.clone(), is_diff: false, cursor_line: 0, cursor_col: 0 },
            ],
            active_tab: Some(1),
            sidebar_width: 400.0,
            sidebar_collapsed: true,
            collapsed_dirs: vec![root.join("target")],
            changes_panel_open: true,
            problems_panel_open: true,
            chat_mode: "Docked".to_string(),
            chat_tab_open: false,
            chat_tab_active: false,
        },
    );

    let mut state = state_with_project_open(root.clone());
    let session_id_before_restore = state.chat_session_id.clone();
    restore_session(&mut state);

    assert_eq!(state.open_tabs.len(), 2);
    assert_eq!(state.active_tab, Some(TabKey::File(files.b.clone())));
    assert_eq!(find_editor(&state, &files.a).unwrap().cursor, CursorPos { line: 0, col: 1 });
    assert_eq!(state.sidebar_width, 400.0);
    assert!(state.sidebar_collapsed);
    assert!(state.collapsed_dirs.contains(&root.join("target")));
    assert!(state.changes_panel_open);
    assert!(state.problems_panel_open);
    assert_eq!(state.chat_mode, ChatMode::Docked);
    assert_eq!(
        state.chat_session_id, session_id_before_restore,
        "the conversation itself must never resume across a restore — only its presentation does"
    );
}

#[test]
fn restore_session_reopens_the_chat_tab_and_makes_it_active() {
    let files = TempFiles::new("restore-chat-tab");
    let root = files.a.parent().unwrap().to_path_buf();
    session::save(
        &root,
        &session::Session {
            open_tabs: vec![session::SessionTab { path: files.a.clone(), is_diff: false, cursor_line: 0, cursor_col: 0 }],
            active_tab: Some(0),
            sidebar_width: 300.0,
            sidebar_collapsed: false,
            collapsed_dirs: Vec::new(),
            changes_panel_open: false,
            problems_panel_open: false,
            chat_mode: "Closed".to_string(),
            chat_tab_open: true,
            chat_tab_active: true,
        },
    );

    let mut state = state_with_project_open(root);
    let session_id_before_restore = state.chat_session_id.clone();
    restore_session(&mut state);

    assert!(state.chat_tab_open);
    assert_eq!(state.active_tab, Some(TabKey::Chat), "the chat tab was active at save time, so it must win over the last-opened file tab");
    assert_eq!(
        state.chat_session_id, session_id_before_restore,
        "reopening the chat tab on restore must start a new conversation, not resume the old one"
    );
}

#[test]
fn capture_session_records_chat_presentation() {
    let files = TempFiles::new("capture-chat");
    let mut state = state_with_project_open(files.a.parent().unwrap().to_path_buf());
    state.chat_tab_open = true;
    state.chat_mode = ChatMode::Closed;
    state.active_tab = Some(TabKey::Chat);

    let session = capture_session(&state);
    assert_eq!(session.chat_mode, "Closed");
    assert!(session.chat_tab_open);
    assert!(session.chat_tab_active);
    assert_eq!(session.active_tab, None, "TabKey::Chat has no backing OpenTab entry to index");
}

#[test]
fn restore_session_is_a_noop_when_nothing_was_ever_saved() {
    let files = TempFiles::new("restore-fresh");
    let root = files.a.parent().unwrap().to_path_buf();

    let mut state = state_with_project_open(root);
    let sidebar_width_before = state.sidebar_width;
    restore_session(&mut state);

    assert!(state.open_tabs.is_empty(), "nothing was ever saved for this root, so no tabs should open");
    assert_eq!(state.sidebar_width, sidebar_width_before, "must leave the fresh-snapshot default in place, not an unclamped 0.0");
}

#[test]
fn restore_session_clamps_a_cursor_past_the_files_current_end() {
    let files = TempFiles::new("restore-clamp");
    let root = files.a.parent().unwrap().to_path_buf();
    session::save(
        &root,
        &session::Session {
            open_tabs: vec![session::SessionTab { path: files.a.clone(), is_diff: false, cursor_line: 50, cursor_col: 50 }],
            active_tab: Some(0),
            sidebar_width: 300.0,
            sidebar_collapsed: false,
            collapsed_dirs: Vec::new(),
            changes_panel_open: false,
            problems_panel_open: false,
            chat_mode: String::new(),
            chat_tab_open: false,
            chat_tab_active: false,
        },
    );

    let mut state = state_with_project_open(root);
    restore_session(&mut state);

    let cursor = find_editor(&state, &files.a).unwrap().cursor;
    assert_eq!(cursor.line, 0, "a.txt is a single line, so line 50 must clamp down to the last real line");
    assert!(cursor.col <= 1, "a.txt's only line is one char (\"a\"), so col must clamp down to it");
}

#[test]
fn open_and_close_a_tab_round_trips_through_the_real_session_file() {
    let files = TempFiles::new("round-trip");
    let root = files.a.parent().unwrap().to_path_buf();
    let mut state = state_with_project_open(root.clone());

    open_or_focus_file(&mut state, files.a.clone());
    open_or_focus_file(&mut state, files.b.clone());

    let mut reopened = state_with_project_open(root);
    restore_session(&mut reopened);
    assert_eq!(reopened.open_tabs.len(), 2, "both real message-driven opens must have persisted, not just the direct capture_session call");
    assert_eq!(reopened.active_tab, Some(TabKey::File(files.b.clone())));
}

#[test]
fn the_caret_can_cross_a_crlf_line_ending_in_both_directions() {
    // `Document::open` reads files verbatim, so CRLF buffers are ordinary.
    // `line_len_chars` excludes both terminator chars, which means
    // `char_index` clamps the position between `\r` and `\n` straight back
    // onto the `\r` — so a naive `idx + 1` landed there and then bounced off
    // it forever, leaving the caret unable to leave the line at all.
    let mut editor = EditorState::new(Document::from_str("ab\r\ncd"), PathBuf::from("t.txt"));
    editor.cursor = CursorPos { line: 0, col: 2 };

    editor.move_cursor(Direction::Right, false);
    assert_eq!(editor.cursor, CursorPos { line: 1, col: 0 }, "Right at end-of-line must reach the next line, not stall inside the \\r\\n");

    editor.move_cursor(Direction::Left, false);
    assert_eq!(editor.cursor, CursorPos { line: 0, col: 2 }, "and Left must come straight back, not stall either");
}

#[test]
fn backspace_at_a_crlf_line_start_takes_the_whole_terminator() {
    // Deleting only the `\n` would leave an orphaned `\r` behind as a stray
    // char on the joined line.
    let mut editor = EditorState::new(Document::from_str("ab\r\ncd"), PathBuf::from("t.txt"));
    editor.cursor = CursorPos { line: 1, col: 0 };

    assert!(editor.backspace());
    assert_eq!(editor.document.text().to_string(), "abcd");
    assert_eq!(editor.cursor, CursorPos { line: 0, col: 2 });
}

#[test]
fn up_and_down_keep_a_goal_column_across_a_short_line() {
    let mut editor = EditorState::new(
        Document::from_str("aaaaaaaa\nbb\ncccccccc\n"),
        PathBuf::from("t.txt"),
    );
    editor.cursor = CursorPos { line: 0, col: 6 };

    editor.move_cursor(Direction::Down, false);
    assert_eq!(editor.cursor, CursorPos { line: 1, col: 2 }, "the short line clamps the column, as it must");

    editor.move_cursor(Direction::Down, false);
    assert_eq!(
        editor.cursor,
        CursorPos { line: 2, col: 6 },
        "passing through a short line must not truncate the column for good — the caret walked diagonally down the file without this"
    );

    // A horizontal move is the user picking a new column, so the next
    // vertical run re-seeds from there rather than resurrecting the old goal.
    editor.move_cursor(Direction::Left, false);
    editor.move_cursor(Direction::Up, false);
    editor.move_cursor(Direction::Up, false);
    assert_eq!(editor.cursor, CursorPos { line: 0, col: 5 }, "Left reset the goal column to 5, not the stale 6");
}

#[test]
fn a_backspace_with_nothing_to_delete_leaves_the_undo_history_alone() {
    // `record_undo_boundary` unconditionally clears the redo stack, so
    // running it before the "is there anything to delete?" check meant an
    // inert keystroke silently threw away a pending redo.
    let mut editor = EditorState::new(Document::from_str("abc"), PathBuf::from("t.txt"));
    editor.insert_text("x");
    assert!(editor.undo());
    assert_eq!(editor.document.text().to_string(), "abc");
    assert_eq!(editor.cursor, CursorPos { line: 0, col: 0 }, "sanity: undo put the caret back at the buffer start");

    assert!(!editor.backspace(), "there is nothing before the first char to delete");
    assert_eq!(editor.redo_stack.len(), 1, "the inert Backspace must not have discarded the redo");
    assert!(editor.redo());
    assert_eq!(editor.document.text().to_string(), "xabc");
}

#[test]
fn undoing_past_a_save_marks_the_buffer_dirty_again() {
    // An `UndoSnapshot` carries the `dirty` flag its `Document` clone had
    // when it was taken. For any snapshot predating a save that flag is
    // `false`, so restoring it verbatim claimed a buffer matching nothing on
    // disk was unmodified — no modified dot, and nothing to warn on close.
    let files = TempFiles::new("undo-dirty");
    let mut editor = EditorState::new(Document::open(&files.a).unwrap(), files.a.clone());

    // Two-char inserts so each is its own undo step (a single char coalesces).
    editor.insert_text("hi");
    editor.save().unwrap();
    assert!(!editor.document.is_dirty(), "sanity: a fresh save is clean");

    editor.insert_text("yo");
    assert!(editor.document.is_dirty());

    assert!(editor.undo());
    assert_eq!(editor.document.text().to_string(), "hia");
    assert!(!editor.document.is_dirty(), "undone back to exactly the saved revision — genuinely clean again");

    assert!(editor.undo());
    assert_eq!(editor.document.text().to_string(), "a");
    assert!(
        editor.document.is_dirty(),
        "undone *past* the save point: the buffer no longer matches the file on disk and must say so"
    );
}

#[test]
fn enter_carries_over_the_current_lines_indentation() {
    let mut editor = EditorState::new(Document::from_str("    let a = 1;"), PathBuf::from("t.rs"));
    editor.cursor = CursorPos { line: 0, col: 14 }; // end of the line

    editor.insert_text("\n");

    assert_eq!(editor.document.text().to_string(), "    let a = 1;\n    ");
    assert_eq!(editor.cursor, CursorPos { line: 1, col: 4 }, "the caret must land after the copied indent, not at column 0");
}

#[test]
fn enter_inside_the_indent_only_copies_up_to_the_cursor() {
    // Pressing Enter with the caret still inside the leading whitespace (not
    // yet at the first real char) must not grab whitespace the split is
    // about to move onto the new line on its own.
    let mut editor = EditorState::new(Document::from_str("    let a = 1;"), PathBuf::from("t.rs"));
    editor.cursor = CursorPos { line: 0, col: 2 };

    editor.insert_text("\n");

    // 2 copied spaces before the break, then the original line's own
    // untouched 2 remaining leading spaces after it — 4 total, not merged.
    assert_eq!(editor.document.text().to_string(), "  \n    let a = 1;");
}

#[test]
fn tab_block_indents_a_multi_line_selection_instead_of_replacing_it() {
    let mut editor = EditorState::new(Document::from_str("a\nb\nc\n"), PathBuf::from("t.txt"));
    editor.selection_anchor = Some(CursorPos { line: 0, col: 0 });
    editor.cursor = CursorPos { line: 2, col: 0 };

    editor.indent();

    assert_eq!(
        editor.document.text().to_string(),
        "    a\n    b\nc\n",
        "every line the selection touches gets indented; the selected text itself must survive, not get replaced by four spaces"
    );
}

#[test]
fn tab_indents_rather_than_replaces_even_a_single_line_selection() {
    // Any active selection indents, even one that never crosses a line
    // break — losing selected text to a Tab press would be surprising
    // regardless of how many lines it spans.
    let mut editor = EditorState::new(Document::from_str("abc"), PathBuf::from("t.txt"));
    editor.selection_anchor = Some(CursorPos { line: 0, col: 0 });
    editor.cursor = CursorPos { line: 0, col: 3 };

    editor.indent();

    assert_eq!(editor.document.text().to_string(), "    abc");
}

#[test]
fn tab_with_no_selection_inserts_four_spaces_at_the_cursor() {
    let mut editor = EditorState::new(Document::from_str("ab"), PathBuf::from("t.txt"));
    editor.cursor = CursorPos { line: 0, col: 1 };

    editor.indent();

    assert_eq!(editor.document.text().to_string(), "a    b");
}

#[test]
fn shift_tab_dedents_every_selected_line_by_one_level() {
    let mut editor = EditorState::new(Document::from_str("    a\n  b\nc\n"), PathBuf::from("t.txt"));
    editor.selection_anchor = Some(CursorPos { line: 0, col: 0 });
    editor.cursor = CursorPos { line: 2, col: 0 };

    assert!(editor.dedent());
    assert_eq!(
        editor.document.text().to_string(),
        "a\nb\nc\n",
        "each line loses up to one indent level (4 spaces or a tab), never more"
    );
}

#[test]
fn shift_tab_on_lines_with_no_leading_whitespace_is_a_no_op() {
    let mut editor = EditorState::new(Document::from_str("abc"), PathBuf::from("t.txt"));
    editor.insert_text("x");
    assert!(editor.undo());

    assert!(!editor.dedent(), "nothing to remove");
    assert_eq!(editor.redo_stack.len(), 1, "an inert Shift+Tab must not have discarded the pending redo");
}

#[test]
fn ctrl_slash_comments_then_uncomments_a_block() {
    // `col: 999` on each end just means "end of that line" — `char_index`
    // clamps it, so the exact line length doesn't need recomputing after the
    // first toggle changes it.
    let mut editor = EditorState::new(Document::from_str("let a = 1;\nlet b = 2;\n"), PathBuf::from("t.rs"));
    editor.selection_anchor = Some(CursorPos { line: 0, col: 0 });
    editor.cursor = CursorPos { line: 1, col: 999 };

    assert!(editor.toggle_comment());
    assert_eq!(editor.document.text().to_string(), "// let a = 1;\n// let b = 2;\n");

    editor.selection_anchor = Some(CursorPos { line: 0, col: 0 });
    editor.cursor = CursorPos { line: 1, col: 999 };
    assert!(editor.toggle_comment());
    assert_eq!(
        editor.document.text().to_string(),
        "let a = 1;\nlet b = 2;\n",
        "toggling an already-commented block must remove exactly what was added"
    );
}

#[test]
fn ctrl_slash_on_a_mixed_block_comments_only_the_uncommented_lines() {
    let mut editor = EditorState::new(
        Document::from_str("// already\nnot yet\n"),
        PathBuf::from("t.rs"),
    );
    editor.selection_anchor = Some(CursorPos { line: 0, col: 0 });
    editor.cursor = CursorPos { line: 1, col: 999 };

    assert!(editor.toggle_comment());
    assert_eq!(
        editor.document.text().to_string(),
        "// already\n// not yet\n",
        "a mixed block converges to fully commented, without doubling up the line that already had a marker"
    );
}

#[test]
fn ctrl_slash_does_nothing_for_a_language_with_no_comment_syntax() {
    let mut editor = EditorState::new(Document::from_str("{}"), PathBuf::from("t.json"));
    assert!(!editor.toggle_comment());
    assert_eq!(editor.document.text().to_string(), "{}");
}

#[test]
fn palette_colon_query_offers_a_go_to_line_entry_and_running_it_moves_the_cursor() {
    let files = TempFiles::new("goto-line");
    std::fs::write(&files.a, "one\ntwo\nthree\nfour\n").unwrap();
    let mut state = State::default();
    open_or_focus_file(&mut state, files.a.clone());

    state.palette_query = ":3".to_string();
    let entries = filtered_palette_entries(&state);
    assert_eq!(entries.len(), 1, "a `:N` query must offer exactly the one synthetic entry, not fall through to substring filtering");
    assert_eq!(entries[0].label, "Go to line 3");

    let _ = run_palette_action(&mut state, entries[0].action.clone());
    let editor = find_editor(&state, &files.a).unwrap();
    assert_eq!(editor.cursor, CursorPos { line: 2, col: 0 }, "line 3 is index 2");
}

#[test]
fn palette_colon_query_clamps_past_the_last_line() {
    let files = TempFiles::new("goto-line-clamp");
    std::fs::write(&files.a, "one\ntwo\n").unwrap();
    let mut state = State::default();
    open_or_focus_file(&mut state, files.a.clone());

    let _ = run_palette_action(&mut state, PaletteAction::GoToLine(999));
    let editor = find_editor(&state, &files.a).unwrap();
    // `document.line_count()` counts the trailing empty line a file ending in
    // `\n` has, same as `move_cursor`'s own Down-arrow bound does — line
    // index 2 (not 1) is genuinely the last line this document has.
    assert_eq!(editor.cursor, CursorPos { line: 2, col: 0 }, "clamped to the document's actual last line, not left past its end");
}

#[test]
fn palette_colon_query_is_empty_with_no_file_open() {
    let mut state = State::default();
    state.palette_query = ":1".to_string();
    assert!(
        filtered_palette_entries(&state).is_empty(),
        "nothing to jump to without an active file tab"
    );
}

#[test]
fn typing_an_opening_bracket_with_no_selection_pairs_it() {
    let mut editor = EditorState::new(Document::from_str(""), PathBuf::from("t.rs"));

    editor.type_char('(');

    assert_eq!(editor.document.text().to_string(), "()");
    assert_eq!(editor.cursor, CursorPos { line: 0, col: 1 }, "the caret must land between the two halves");
}

#[test]
fn typing_a_quote_with_no_selection_pairs_it_too() {
    let mut editor = EditorState::new(Document::from_str(""), PathBuf::from("t.rs"));

    editor.type_char('"');

    assert_eq!(editor.document.text().to_string(), "\"\"");
    assert_eq!(editor.cursor, CursorPos { line: 0, col: 1 });
}

#[test]
fn typing_the_closer_right_before_its_auto_inserted_partner_skips_over_it() {
    let mut editor = EditorState::new(Document::from_str(""), PathBuf::from("t.rs"));
    editor.type_char('(');
    assert_eq!(editor.document.text().to_string(), "()");

    editor.type_char(')');

    assert_eq!(
        editor.document.text().to_string(),
        "()",
        "typing the closer must not insert a second one"
    );
    assert_eq!(editor.cursor, CursorPos { line: 0, col: 2 }, "the caret steps past the existing closer instead");
}

#[test]
fn typing_a_quote_right_before_its_own_auto_inserted_partner_skips_over_it() {
    let mut editor = EditorState::new(Document::from_str(""), PathBuf::from("t.rs"));
    editor.type_char('"');

    editor.type_char('"');

    assert_eq!(editor.document.text().to_string(), "\"\"");
    assert_eq!(editor.cursor, CursorPos { line: 0, col: 2 });
}

#[test]
fn typing_a_closer_with_no_matching_partner_nearby_inserts_it_literally() {
    let mut editor = EditorState::new(Document::from_str("abc"), PathBuf::from("t.rs"));
    editor.cursor = CursorPos { line: 0, col: 3 };

    editor.type_char(')');

    assert_eq!(editor.document.text().to_string(), "abc)");
}

#[test]
fn typing_an_opener_with_an_active_selection_wraps_it_instead_of_pairing() {
    let mut editor = EditorState::new(Document::from_str("hello"), PathBuf::from("t.rs"));
    editor.selection_anchor = Some(CursorPos { line: 0, col: 0 });
    editor.cursor = CursorPos { line: 0, col: 5 };

    editor.type_char('(');

    assert_eq!(editor.document.text().to_string(), "(hello)");
    assert_eq!(
        editor.selection(),
        Some((1, 6)),
        "the original text stays selected — wrapped, not replaced or deselected"
    );
}

#[test]
fn wrapping_a_multi_line_selection_still_lands_the_closer_at_the_right_end() {
    let mut editor = EditorState::new(Document::from_str("a\nbc\n"), PathBuf::from("t.rs"));
    editor.selection_anchor = Some(CursorPos { line: 0, col: 0 });
    editor.cursor = CursorPos { line: 1, col: 2 };

    editor.type_char('{');

    assert_eq!(editor.document.text().to_string(), "{a\nbc}\n");
}

#[test]
fn typing_an_ordinary_character_is_unaffected_by_auto_pairing() {
    let mut editor = EditorState::new(Document::from_str("ab"), PathBuf::from("t.rs"));
    editor.cursor = CursorPos { line: 0, col: 1 };

    editor.type_char('x');

    assert_eq!(editor.document.text().to_string(), "axb");
    assert_eq!(editor.cursor, CursorPos { line: 0, col: 2 });
}

#[test]
fn max_line_chars_grows_as_a_line_grows() {
    let mut editor = EditorState::new(Document::from_str("ab\nc\n"), PathBuf::from("t.txt"));
    assert_eq!(editor.max_line_chars(), 2, "sanity: starts at the longest existing line");

    editor.cursor = CursorPos { line: 1, col: 1 };
    editor.insert_text("hello");

    assert_eq!(editor.max_line_chars(), 6, "typing a longer line must grow the tracked max immediately, not wait for settle");
}

#[test]
fn max_line_chars_is_capped_even_for_a_pathologically_long_line() {
    let mut editor = EditorState::new(Document::from_str(""), PathBuf::from("t.txt"));
    editor.insert_text(&"x".repeat(5000));

    assert_eq!(
        editor.max_line_chars(),
        MAX_RENDERED_LINE_CHARS,
        "the canvas must never be sized to fit a whole minified-bundle-style line verbatim"
    );
}

#[test]
fn max_line_chars_shrinks_back_once_the_longest_line_is_deleted_and_reparsed() {
    let mut editor = EditorState::new(Document::from_str("short\nreally long line here\n"), PathBuf::from("t.txt"));
    assert_eq!(editor.max_line_chars(), 21);

    // Delete the whole second (longest) line.
    editor.cursor = CursorPos { line: 1, col: 0 };
    let range = editor.document.line_char_range_with_terminator(1);
    editor.document.remove(range);
    editor.resync_after_edit();
    assert_eq!(
        editor.max_line_chars(),
        21,
        "grow-only tracking must not shrink immediately — it's stale until the next reparse, by design"
    );

    editor.reparse_now();
    assert_eq!(editor.max_line_chars(), 5, "a full reparse must correct the stale, now-too-large cached max");
}

#[test]
fn undo_immediately_corrects_max_line_chars_without_waiting_for_settle() {
    let mut editor = EditorState::new(Document::from_str("ab"), PathBuf::from("t.txt"));
    editor.cursor = CursorPos { line: 0, col: 2 };
    editor.insert_text(&"x".repeat(20));
    assert_eq!(editor.max_line_chars(), 22);

    assert!(editor.undo());

    assert_eq!(
        editor.max_line_chars(),
        2,
        "undo/redo are discrete, infrequent actions — they recompute immediately rather than waiting for the settle debounce"
    );
}

#[test]
fn typing_past_the_right_edge_of_a_narrow_viewport_scrolls_right() {
    let files = TempFiles::new("hscroll-right");
    std::fs::write(&files.a, "").unwrap();
    let mut state = State::default();
    open_or_focus_file(&mut state, files.a.clone());
    find_editor_mut(&mut state, &files.a).unwrap().viewport_width = 100.0;

    for _ in 0..20 {
        let _ = update(&mut state, Message::EditorTypeChar('x'));
    }

    let editor = find_editor(&state, &files.a).unwrap();
    assert!(editor.scroll_offset_x > 0.0, "typing past a 100px-wide viewport must scroll right to keep the caret visible");
}

#[test]
fn moving_back_to_column_zero_resets_horizontal_scroll_to_show_the_gutter_again() {
    let files = TempFiles::new("hscroll-home");
    std::fs::write(&files.a, "x".repeat(60)).unwrap();
    let mut state = State::default();
    open_or_focus_file(&mut state, files.a.clone());
    {
        let editor = find_editor_mut(&mut state, &files.a).unwrap();
        editor.viewport_width = 100.0;
        editor.cursor = CursorPos { line: 0, col: 60 };
    }
    let _ = update(&mut state, Message::EditorMove { dir: Direction::LineEnd, extend: false });
    assert!(
        find_editor(&state, &files.a).unwrap().scroll_offset_x > 0.0,
        "sanity: starting scrolled right"
    );

    let _ = update(&mut state, Message::EditorMove { dir: Direction::LineStart, extend: false });

    assert_eq!(
        find_editor(&state, &files.a).unwrap().scroll_offset_x,
        0.0,
        "column 0 must fully reset the scroll, not stop just short of it and leave the gutter hidden"
    );
}

#[test]
fn scroll_cursor_into_view_is_a_no_op_when_already_visible() {
    let files = TempFiles::new("hscroll-noop");
    std::fs::write(&files.a, "hello").unwrap();
    let mut state = State::default();
    open_or_focus_file(&mut state, files.a.clone());
    {
        let editor = find_editor_mut(&mut state, &files.a).unwrap();
        editor.viewport_width = 800.0;
        editor.viewport_height = 800.0;
    }

    let task = scroll_cursor_into_view(&mut state);

    assert_eq!(task.units(), 0, "a fully-visible caret must not produce a scroll task");
    let editor = find_editor(&state, &files.a).unwrap();
    assert_eq!(editor.scroll_offset_x, 0.0);
    assert_eq!(editor.scroll_offset, 0.0);
}
