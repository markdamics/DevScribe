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
    let _ = update(
        &mut state,
        Message::Chat(ClaudeEvent::ToolResult { id: "toolu_1".to_string(), is_error: false, result: serde_json::json!("contents") }),
    );

    assert_eq!(state.chat.messages.len(), 1, "the result should update the existing entry, not add a second one");
    let ChatMessage::Tool(tool) = &state.chat.messages[0] else { panic!("expected a Tool entry") };
    assert_eq!(tool.name, "Read");
    assert!(tool.permission.is_none(), "Read never needed a permission decision");
    let result = tool.result.as_ref().expect("result should be attached");
    assert!(!result.is_error);
    assert_eq!(result.result, serde_json::json!("contents"));
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
