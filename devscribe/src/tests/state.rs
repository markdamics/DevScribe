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

    let files_in_dir: Vec<PathBuf> = fs_tree::flatten_files(&fs_tree::walk(&dir)).into_iter().map(Path::to_path_buf).collect();

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
        tree: fs_tree::walk(&dir),
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
        tree: fs_tree::walk(&dir),
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
        tree: fs_tree::walk(&dir),
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
    let files_in_dir: Vec<PathBuf> = fs_tree::flatten_files(&fs_tree::walk(&dir)).into_iter().map(Path::to_path_buf).collect();

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

    let files_in_dir: Vec<PathBuf> = fs_tree::flatten_files(&fs_tree::walk(&dir)).into_iter().map(Path::to_path_buf).collect();

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
