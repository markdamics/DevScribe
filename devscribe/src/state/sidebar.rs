//! Project/file-tree state: the sidebar's drafts (new file/folder), rename/delete,
//! project loading and the welcome screen's recent-project rows, project-wide search,
//! and the git changes panel. Split out of the former monolithic `state.rs` — see
//! `super` for `State`/`Message`/`update()`, which this module's functions are called
//! from but never define themselves.

use super::*;

/// One project-wide search hit, with the file it was found in.
///
/// Deliberately carries no per-token syntax-color data (`hit.preview`
/// renders in one flat color, just the matched span itself tinted) —
/// search used to run the full syntax highlighter once per matching file,
/// which was real, uncapped CPU cost that scaled with how many files a
/// broad query touched. Removed rather than capped: search's own results
/// cap only ever bounded *how many hits got kept*, not how much
/// highlighting work ran to produce them, so a query matching broadly
/// across a large project could still highlight hundreds of whole files
/// synchronously even with a small `MAX_SEARCH_RESULTS`.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub path: PathBuf,
    pub hit: SearchHit,
    /// `state.search_query`'s length in chars *at the time this result was
    /// computed* — snapshotted so a later, still-unsubmitted query edit
    /// can't desync it from `hit.col`.
    pub query_len_chars: usize,
}

/// A finished background search's payload — see `run_search` and
/// `Message::SearchCompleted`. Carries `query` so the handler can tell a
/// stale result (from a search that's since been superseded by further
/// typing) apart from the one that's actually still relevant, instead of
/// trusting arrival order.
#[derive(Debug, Clone)]
pub struct SearchOutcome {
    pub query: String,
    pub results: Vec<SearchResult>,
    pub elapsed: Duration,
}

/// One row in the sidebar's "CHANGES" panel: a working-tree change plus the
/// insertion/deletion counts the mockup shows per file. Recomputed by
/// `refresh_changed_files` — see its doc for when.
#[derive(Debug, Clone)]
pub struct ChangesEntry {
    pub path: PathBuf,
    pub kind: ChangeKind,
    pub insertions: usize,
    pub deletions: usize,
}

/// One row in the welcome screen's recent-projects list and the sidebar's
/// projects dropdown — display data derived live from a `RecentProject`
/// (name, detected language glyph, `path // BRANCH // N CHANGES` /
/// `path // NO REPOSITORY` subtitle, relative "12M"/"2D"/"3W" label).
/// Recomputed whenever `State::recent_projects` changes (`compute_welcome_rows`)
/// rather than persisted, since branch/change-count go stale between
/// launches.
#[derive(Debug, Clone)]
pub struct WelcomeRow {
    pub path: PathBuf,
    pub name: String,
    pub lang: fs_tree::Lang,
    pub subtitle: String,
    pub last_opened_label: String,
}

/// Drives the welcome screen's "Loading workspace" overlay while a
/// background project load (`start_loading_project`) is in flight.
#[derive(Debug, Clone)]
pub struct LoadingProject {
    pub name: String,
    pub path: PathBuf,
}

/// What kind of inline tree draft is open: a new file, a new folder, or an
/// in-place rename of an existing entry. Drives both the draft row's glyph/
/// placeholder (`sidebar.rs`) and how `commit_draft` interprets `text`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftKind {
    NewFile,
    NewFolder,
    Rename,
}

/// The sidebar tree's single in-progress inline edit — either a "new file"/
/// "new folder" draft row inserted under `dir`, or a rename of `target` in
/// place. Only one draft can be open at a time (starting a new one replaces
/// it), matching the mockup's single `draft` field.
#[derive(Debug, Clone)]
pub struct Draft {
    pub kind: DraftKind,
    /// Parent directory a `NewFile`/`NewFolder` draft is created in. Unused
    /// for `Rename` (the target's existing parent is used instead).
    pub dir: PathBuf,
    /// The path being renamed, for `Rename` drafts only.
    pub target: Option<PathBuf>,
    pub text: String,
}

/// The tree's right-click context menu: `target = None` means the tree
/// background/root was right-clicked (New file/New folder only, no Rename/
/// Copy path); `Some(path)` means a specific entry.
#[derive(Debug, Clone)]
pub struct ContextMenu {
    pub target: Option<PathBuf>,
    /// `true` once "Delete" has been clicked once — the menu then shows a
    /// confirm/cancel step instead of the normal rows, rather than
    /// deleting on the first click. Deleting a file/directory here is
    /// permanent (no OS trash/recycle-bin integration), so this is the one
    /// destructive action in the whole app that needs a deliberate second
    /// step before it touches disk.
    pub confirm_delete: bool,
}

/// A project root's derived data — tree, collapsed dirs, and git summary —
/// everything `snapshot_project` computes. `Repo` itself is deliberately
/// not part of this: it isn't `Clone` (wraps a `gix::Repository`), and this
/// gets sent across a background thread as a `Message` payload
/// (`start_loading_project`), which requires `Clone` project-wide. Wherever
/// a snapshot is applied to `State`, `Repo::open` (cheap — just opens
/// refs/HEAD, not a status walk) is called again synchronously alongside it.
#[derive(Debug, Clone)]
pub struct ProjectSnapshot {
    pub tree: Vec<Node>,
    pub collapsed_dirs: HashSet<PathBuf>,
    pub changed_files: Vec<ChangesEntry>,
    pub ahead_behind: Option<(usize, usize)>,
}

/// A finished background project load (`start_loading_project`), carried by
/// `Message::ProjectLoaded`.
#[derive(Debug, Clone)]
pub struct LoadedProject {
    root: PathBuf,
    snapshot: ProjectSnapshot,
}

/// Walks `root`'s file tree and computes its git summary — the expensive
/// part of "open a project," shared by `State::default()`'s startup
/// auto-reopen and `start_loading_project`'s background loader.
pub fn snapshot_project(root: &Path, show_hidden: bool) -> ProjectSnapshot {
    let tree = fs_tree::walk(root, show_hidden);
    // Directories start collapsed — an uncollapsed default would dump the
    // whole project tree open on first view.
    let collapsed_dirs: HashSet<PathBuf> =
        fs_tree::flatten_dirs(&tree).into_iter().map(Path::to_path_buf).collect();
    let repo = Repo::open(root);
    let changed_files = compute_changed_files(repo.as_ref());
    let ahead_behind = repo.as_ref().and_then(Repo::ahead_behind);
    ProjectSnapshot { tree, collapsed_dirs, changed_files, ahead_behind }
}

/// How many `recent_projects` entries get a live `WelcomeRow` — beyond
/// what the welcome screen/sidebar dropdown ever show, each row is a
/// `Repo::open` + status scan that would never get used.
pub const MAX_WELCOME_ROWS: usize = 8;

/// Computes live display data for up to `MAX_WELCOME_ROWS` of `recent`,
/// via a transient `Repo::open` per entry (not the same handle as
/// `State::repo` — this never touches the currently open project's repo).
/// Called whenever `recent_projects` changes, not on every `view()` (see
/// `WelcomeRow`'s doc).
pub fn compute_welcome_rows(recent: &[recent_projects::RecentProject]) -> Vec<WelcomeRow> {
    recent
        .iter()
        .take(MAX_WELCOME_ROWS)
        .map(|entry| {
            let path = entry.path.clone();
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());
            let lang = recent_projects::detect_lang(&path);
            let subtitle = match Repo::open(&path) {
                Some(repo) => {
                    let branch = repo.branch_name().unwrap_or_else(|| "\u{2014}".to_string());
                    let n = repo.changed_files().len();
                    let changes = match n {
                        0 => "CLEAN".to_string(),
                        1 => "1 CHANGE".to_string(),
                        n => format!("{n} CHANGES"),
                    };
                    format!("{} // {} // {}", shorten_home(&path), branch.to_uppercase(), changes)
                }
                None => format!("{} // NO REPOSITORY", shorten_home(&path)),
            };
            let last_opened_label = recent_projects::relative_label(entry.last_opened_ms);
            WelcomeRow { path, name, lang, subtitle, last_opened_label }
        })
        .collect()
}

/// `~`-shortens a path under `$HOME` — shared by the welcome screen's
/// recent-project subtitles and `sidebar.rs`'s project switcher.
pub(crate) fn shorten_home(path: &Path) -> String {
    if let Some(home) = std::env::var_os("HOME")
        && let Ok(rest) = path.strip_prefix(&home)
    {
        return format!("~/{}", rest.display());
    }
    path.display().to_string()
}

pub fn empty_project_snapshot() -> ProjectSnapshot {
    ProjectSnapshot { tree: Vec::new(), collapsed_dirs: HashSet::new(), changed_files: Vec::new(), ahead_behind: None }
}

/// Everything `Default::default()` needs to seed a startup `State`: whether
/// the welcome screen should show, and (if not) the project it auto-reopened.
pub struct Startup {
    pub welcome_open: bool,
    pub root: PathBuf,
    pub snapshot: ProjectSnapshot,
    pub repo: Option<Repo>,
    pub recent_projects: Vec<recent_projects::RecentProject>,
    pub welcome_rows: Vec<WelcomeRow>,
}

/// Loads the real persisted recent-projects list and auto-reopens the most
/// recently used one that still exists on disk (VSCode-style) — skipping
/// any stale entries for projects since moved or deleted, rather than just
/// failing on the very first one. First run (nothing recorded yet) or
/// every recorded path having vanished both fall through to the welcome
/// screen, same as explicitly closing a project.
#[cfg(not(test))]
pub fn startup(show_hidden: bool) -> Startup {
    let mut recent_projects = recent_projects::load();
    let reopen = recent_projects.iter().find(|p| p.path.is_dir()).map(|p| p.path.clone());

    match reopen {
        Some(root) => {
            recent_projects::touch(&mut recent_projects, root.clone());
            let snapshot = snapshot_project(&root, show_hidden);
            let repo = Repo::open(&root);
            let welcome_rows = compute_welcome_rows(&recent_projects);
            Startup { welcome_open: false, root, snapshot, repo, recent_projects, welcome_rows }
        }
        None => {
            let welcome_rows = compute_welcome_rows(&recent_projects);
            Startup { welcome_open: true, root: PathBuf::new(), snapshot: empty_project_snapshot(), repo: None, recent_projects, welcome_rows }
        }
    }
}

/// The test build never touches the real persisted recent-projects file —
/// reading it would make every test using `State::default()` depend on
/// whatever project this machine's user last had open in the real app
/// (nondeterministic across machines and over time), and the auto-reopen
/// path's `recent_projects::touch` would *write* to that same real file as
/// a side effect of running `cargo test`, silently corrupting real user
/// state. Every test gets a deterministic, disk-free "no project open" seed
/// instead — consistent with existing tests already not trusting
/// `State::default()`'s project fields (see e.g. the `changed_files`
/// comment a few tests down) and overriding what they need explicitly.
#[cfg(test)]
pub fn startup(_show_hidden: bool) -> Startup {
    Startup {
        welcome_open: true,
        root: PathBuf::new(),
        snapshot: empty_project_snapshot(),
        repo: None,
        recent_projects: Vec::new(),
        welcome_rows: Vec::new(),
    }
}

/// A stable id for the sidebar tree's inline draft text input, so `update()`
/// can focus it the moment a new-file/new-folder/rename draft opens.
pub fn draft_input_id() -> iced::widget::Id {
    iced::widget::Id::new("tree-draft-input")
}

/// Re-scans `root` for the sidebar tree — the only way `state.tree` picks up
/// filesystem changes (no watcher), so every draft commit that adds/removes
/// an entry calls this. Deliberately doesn't touch `collapsed_dirs`: it's
/// keyed by absolute path, so existing collapse state for untouched
/// directories survives a re-walk unchanged.
pub fn refresh_tree(state: &mut State) {
    state.tree = fs_tree::walk(&state.root, state.show_hidden_files);
}

pub fn begin_draft(state: &mut State, kind: DraftKind, dir: PathBuf) -> iced::Task<Message> {
    state.draft = Some(Draft {
        kind,
        dir,
        target: None,
        text: String::new(),
    });
    state.ctx_menu = None;
    iced::widget::operation::focus(draft_input_id())
}

pub fn begin_rename(state: &mut State, path: PathBuf) -> iced::Task<Message> {
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_else(|| state.root.clone());
    state.draft = Some(Draft {
        kind: DraftKind::Rename,
        dir,
        target: Some(path),
        text: String::new(),
    });
    state.ctx_menu = None;
    iced::widget::operation::focus(draft_input_id())
}

/// Commits the open draft, if any and non-empty: writes the new file/folder
/// to disk, or renames the target, then refreshes the tree and fires the
/// matching "flash" confirmation. Errors (name collision, permission denied)
/// surface as a regular toast rather than the flash pill — they're not the
/// success case the flash is for.
pub fn commit_draft(state: &mut State) {
    let Some(draft) = state.draft.take() else {
        return;
    };
    let name = draft.text.trim().to_string();
    if name.is_empty() {
        return;
    }

    match draft.kind {
        DraftKind::NewFile => {
            let path = draft.dir.join(&name);
            if path.exists() {
                push_toast(state, ToastKind::Error, format!("{name} already exists"));
                return;
            }
            match std::fs::write(&path, b"") {
                Ok(()) => {
                    refresh_tree(state);
                    refresh_changed_files(state);
                    open_or_focus_file(state, path);
                    push_flash(state, format!("FILE CREATED // {}", name.to_uppercase()));
                }
                Err(err) => push_toast(state, ToastKind::Error, format!("Couldn't create {name}: {err}")),
            }
        }
        DraftKind::NewFolder => {
            let path = draft.dir.join(&name);
            if path.exists() {
                push_toast(state, ToastKind::Error, format!("{name} already exists"));
                return;
            }
            match std::fs::create_dir(&path) {
                Ok(()) => {
                    refresh_tree(state);
                    push_flash(state, format!("FOLDER CREATED // {}", name.to_uppercase()));
                }
                Err(err) => push_toast(state, ToastKind::Error, format!("Couldn't create {name}: {err}")),
            }
        }
        DraftKind::Rename => {
            let Some(old_path) = draft.target else {
                return;
            };
            let new_path = draft.dir.join(&name);
            if new_path == old_path {
                return;
            }
            if new_path.exists() {
                push_toast(state, ToastKind::Error, format!("{name} already exists"));
                return;
            }
            match std::fs::rename(&old_path, &new_path) {
                Ok(()) => {
                    rename_open_tab(state, &old_path, &new_path);
                    refresh_tree(state);
                    refresh_changed_files(state);
                    push_flash(state, format!("RENAMED // {}", name.to_uppercase()));
                }
                Err(err) => push_toast(state, ToastKind::Error, format!("Couldn't rename: {err}")),
            }
        }
    }
}

/// Updates every open tab (`File` and its matching `Diff`, if any) and
/// `active_tab` referencing `old_path` to `new_path` in place, instead of
/// closing and reopening — that would lose unsaved edits, cursor position,
/// and find state. Re-notifies the LSP server (`didClose` old URI, `didOpen`
/// new URI) since a server keys documents by URI, not by identity.
pub fn rename_open_tab(state: &mut State, old_path: &Path, new_path: &Path) {
    if find_editor(state, old_path).is_none() {
        return;
    }
    if let Some(sender) = state.lsp_sender.as_mut() {
        send_did_close(sender, old_path);
    }
    if let Some(editor) = find_editor_mut(state, old_path) {
        editor.document.set_path(new_path.to_path_buf());
        editor.path = new_path.to_path_buf();
    }
    send_did_open_for(state, new_path);

    let old_diff_key = TabKey::Diff(old_path.to_path_buf());
    let new_diff_key = TabKey::Diff(new_path.to_path_buf());
    let had_diff_tab = state.open_tabs.iter().any(|t| t.key() == old_diff_key);
    if had_diff_tab {
        state.open_tabs.retain(|t| t.key() != old_diff_key);
        state.open_tabs.push(OpenTab::Diff(new_path.to_path_buf()));
    }

    let old_file_key = TabKey::File(old_path.to_path_buf());
    let new_file_key = TabKey::File(new_path.to_path_buf());
    match state.active_tab {
        Some(ref key) if *key == old_file_key => state.active_tab = Some(new_file_key),
        Some(ref key) if *key == old_diff_key => state.active_tab = Some(new_diff_key),
        _ => {}
    }
    persist_session(state);
}

/// Closes every open tab except `active_tab` — the tab-bar overflow menu's
/// "Close others". Reuses `close_tab` per key (rather than a bulk `retain`)
/// so LSP `didClose` notifications and diff-tab cleanup still happen.
/// Removes `target` (file, or recursively for a directory) from disk —
/// the confirmed "Delete" action. Closes every open tab under `target`
/// first (reusing `close_tab`, so LSP `didClose`/diff-tab cleanup/
/// `active_tab` reassignment all happen the same way `CloseActiveTab`
/// already does), then does the actual removal.
pub fn delete_path(state: &mut State, target: PathBuf) {
    state.ctx_menu = None;
    let is_dir = target.is_dir();
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();

    let affected: Vec<TabKey> = state
        .open_tabs
        .iter()
        .map(|t| t.key())
        .filter(|key| match key {
            TabKey::File(path) | TabKey::Diff(path) => path == &target || path.starts_with(&target),
            TabKey::Search | TabKey::Chat => false,
        })
        .collect();
    for key in affected {
        close_tab(state, &key);
    }

    let result = if is_dir { std::fs::remove_dir_all(&target) } else { std::fs::remove_file(&target) };
    match result {
        Ok(()) => {
            refresh_tree(state);
            refresh_changed_files(state);
            push_flash(state, format!("DELETED // {}", name.to_uppercase()));
        }
        Err(err) => push_toast(state, ToastKind::Error, format!("Couldn't delete {name}: {err}")),
    }
}

/// Expands every ancestor directory of the active file so it's visible in
/// the tree — the tab-bar overflow menu's "Reveal in tree". Doesn't scroll
/// the tree to it: `sidebar.rs`'s tree `scrollable` has no stable `.id()`
/// wired up yet, the same known gap as Ctrl+F's auto-scroll-to-match.
pub fn reveal_active_in_tree(state: &mut State) {
    let Some(path) = active_file_path(state) else {
        return;
    };
    let mut dir = path.parent();
    while let Some(d) = dir {
        if !d.starts_with(&state.root) || d == state.root {
            break;
        }
        state.collapsed_dirs.remove(d);
        dir = d.parent();
    }
}

/// Opens the (singleton) search tab, or focuses it if already open.
pub fn focus_search(state: &mut State) {
    state.active_tab = Some(TabKey::Search);
}

/// Computes the sidebar's "CHANGES" panel contents: every file `repo`
/// reports as differing from `HEAD`, with insertion/deletion counts from
/// `devscribe_core::diff::diff_lines` run against `HEAD`'s blob and the
/// file's current *on-disk* content — not the live buffer, so this covers
/// files that aren't even open as a tab, matching `changed_files()` itself
/// scanning the whole working tree rather than just open files.
pub fn compute_changed_files(repo: Option<&Repo>) -> Vec<ChangesEntry> {
    let Some(repo) = repo else {
        return Vec::new();
    };
    repo.changed_files()
        .into_iter()
        .map(|file| {
            let old = repo.head_text(&file.path).unwrap_or_default();
            let new = std::fs::read_to_string(&file.path).unwrap_or_default();
            let lines = devscribe_core::diff::diff_lines(&old, &new);
            let insertions = lines
                .iter()
                .filter(|l| l.kind == devscribe_core::diff::DiffLineKind::Insert)
                .count();
            let deletions = lines
                .iter()
                .filter(|l| l.kind == devscribe_core::diff::DiffLineKind::Delete)
                .count();
            ChangesEntry {
                path: file.path,
                kind: file.kind,
                insertions,
                deletions,
            }
        })
        .collect()
}

/// Re-scans the working tree for `state.changed_files`, and the upstream
/// comparison for `state.ahead_behind` alongside it — both are cheap-ish
/// `gix` walks with the same staleness story, so one refresh point covers
/// both. Run after actions known to change the working tree (saving a file,
/// discarding/reverting a change), after a `FilesChanged` watcher event, and
/// on `WindowFocused` — the watcher itself never fires for a `.git`-only
/// change (a commit, push, or branch switch that doesn't touch a tracked
/// file), so regaining focus is what catches those up.
pub fn refresh_changed_files(state: &mut State) {
    state.changed_files = compute_changed_files(state.repo.as_ref());
    state.ahead_behind = state.repo.as_ref().and_then(Repo::ahead_behind);
}

/// How long the search box has to sit still before `SearchDebounceTick`
/// starts a search for it — VSCode-style search-as-you-type, not a search
/// per keystroke. `SearchSubmit` (Enter) bypasses this entirely.
pub const SEARCH_DEBOUNCE_DELAY: Duration = Duration::from_millis(300);

/// Naive project-wide search: reads every file the sidebar already walked
/// and scans it. Capped so even one search can't run away — see
/// `devscribe_core::search` for the "start naive, index later if slow"
/// rationale. Briefly lowered to 50 while chasing what turned out to be an
/// unrelated, now-fixed root cause (see `SearchHit::preview`'s doc and the
/// roadmap's search bug-fix writeup, "ninth pass") — restored to 200 now
/// that every result's render cost is genuinely bounded regardless of the
/// underlying line's real length, so there's no reason left to show fewer.
pub const MAX_SEARCH_RESULTS: usize = 200;

/// Files larger than this are skipped entirely rather than read and
/// scanned. This is naive search — unlike an indexed tool (ripgrep, an
/// LSP), it has no way to bound the cost of one huge file (a lockfile, a
/// bundle, a log) other than not reading it. `MAX_SEARCH_RESULTS`/
/// `search_text`'s own `max_hits` cap the *match count*, but a
/// many-megabyte file that matches nothing would still pay the full
/// read+scan cost without this.
pub const MAX_SEARCH_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// Hard ceiling on how many files a single search examines (read + scanned
/// — not just how many are *skipped* by the size check above), regardless
/// of how many actually match. Without this, a query that's rare or absent
/// across a very large project would still walk, stat, and read every file
/// the sidebar tree has before `MAX_SEARCH_RESULTS` (which only counts
/// *hits*) ever got a chance to end the search early. Now runs on a
/// background thread (`start_search`), so this no longer bounds UI-thread
/// latency directly — it still matters, since an unbounded scan is still
/// real work an abandoned/cancelled search's thread keeps doing, and still
/// real wall-clock time before a *relevant* search's own results land.
pub const MAX_SEARCH_FILES_SCANNED: usize = 3_000;

/// The actual file-reading/scanning work, decoupled from `State` so it can
/// run on a background thread (see `start_search`) — takes owned inputs
/// and returns an owned outcome instead of borrowing app state, since
/// nothing here can hold a reference across a thread boundary.
pub fn run_search(files: &[PathBuf], query: &str) -> SearchOutcome {
    let query_len_chars = query.chars().count();
    let started = Instant::now();
    let mut results = Vec::new();

    for path in files.iter().take(MAX_SEARCH_FILES_SCANNED) {
        let remaining = MAX_SEARCH_RESULTS - results.len();
        if remaining == 0 {
            break;
        }

        let is_small_enough =
            std::fs::metadata(path).map(|meta| meta.len() <= MAX_SEARCH_FILE_BYTES).unwrap_or(false);
        if !is_small_enough {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        // Bounded by `remaining`, not just `MAX_SEARCH_RESULTS` overall — a
        // single file can't out-allocate the rest of the search budget
        // combined. See `search_text`'s own doc for why this matters: every
        // hit clones its whole line, so an uncapped scan of one large file
        // with many matches on long lines is an unbounded-memory risk, not
        // just a slow one.
        let hits = search::search_text(&content, query, remaining);

        for hit in hits {
            results.push(SearchResult {
                path: path.clone(),
                hit,
                query_len_chars,
            });
        }
    }

    SearchOutcome { query: query.to_string(), results, elapsed: started.elapsed() }
}

/// Starts a background search for `state.search_query`, cancelling
/// whatever search was already in flight first. Runs on its own OS thread
/// (`iced_runtime::task::blocking`) rather than as ordinary `async` work,
/// so a stalled `std::fs::read_to_string` (a network mount, a cloud-sync
/// placeholder file that has to download on open) blocks only that thread
/// — the UI stays responsive no matter how long, or whether, it returns.
///
/// "Cancelling" the previous search only ever means aborting its `Task` —
/// the `Handle::abort()` in `state.search_task_handle` — which stops its
/// eventual `SearchCompleted` from ever being delivered/applied. It does
/// *not*, and structurally can't, kill the OS thread already running that
/// search: `std` has no safe way to interrupt a thread mid-syscall. An
/// abandoned search's thread keeps running until its current file read
/// returns (or, in the stalled-mount case, potentially never), just
/// harmlessly discarding its result into a channel nothing reads anymore
/// instead of freezing anything.
pub fn start_search(state: &mut State) -> iced::Task<Message> {
    if let Some(handle) = state.search_task_handle.take() {
        handle.abort();
    }
    if state.search_query.is_empty() {
        state.search_in_progress = false;
        return iced::Task::none();
    }

    let query = state.search_query.clone();
    let files: Vec<PathBuf> = fs_tree::flatten_files(&state.tree).into_iter().map(Path::to_path_buf).collect();
    state.search_in_progress = true;

    let (task, handle) = iced_runtime::task::blocking(move |mut sender| {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_search(&files, &query))) {
            Ok(outcome) => {
                let _ = sender.try_send(outcome);
            }
            Err(_) => {
                crate::logging::error("start_search: run_search panicked — see the panic hook's log line above for details");
            }
        }
    })
    .map(Message::SearchCompleted)
    .abortable();
    state.search_task_handle = Some(handle);
    task
}

/// The native async folder picker behind `OpenFolderDialog`/`NewProjectDialog`.
/// `rfd`'s dialog needs no parent-window handle for this basic case — it
/// just appears as its own top-level window.
pub async fn pick_folder() -> Option<PathBuf> {
    rfd::AsyncFileDialog::new().pick_folder().await.map(|handle| handle.path().to_path_buf())
}

/// Starts a background project load: optionally `git init`s `path` (for
/// "New project", when it isn't already a repo), then computes a full
/// `snapshot_project`. Guarded by the caller checking `loading_project` is
/// already `None` — unlike search, this is a one-shot action with no
/// cancel-and-restart behavior, so a simple "ignore while one's in flight"
/// guard is enough (no `Handle`/`abortable` needed).
pub fn start_loading_project(state: &mut State, path: PathBuf, init_git: bool) -> iced::Task<Message> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());
    state.loading_project = Some(LoadingProject { name, path: path.clone() });
    let show_hidden = state.show_hidden_files;

    iced_runtime::task::blocking(move |mut sender| {
        // Same belt-and-suspenders `catch_unwind` as `start_search`, on top
        // of `logging::init`'s process-wide panic hook.
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if init_git && Repo::open(&path).is_none() {
                // Best-effort: if `git init` fails, still open the folder
                // as a non-repo project rather than losing the pick.
                Repo::init(&path);
            }
            LoadedProject { root: path.clone(), snapshot: snapshot_project(&path, show_hidden) }
        }));
        match outcome {
            Ok(loaded) => {
                let _ = sender.try_send(loaded);
            }
            Err(_) => {
                crate::logging::error("start_loading_project: panicked — see the panic hook's log line above for details");
            }
        }
    })
    .map(|loaded| Message::ProjectLoaded(Box::new(loaded)))
}

/// Resets every field scoped to the *previous* project — open tabs, search,
/// transient UI, LSP handle — shared by `apply_loaded_project` (switching to
/// a new root) and `close_project` (returning to the welcome screen with no
/// root at all). No confirm-before-discard prompt: the app has no
/// dirty-file guard anywhere else either (`close_tab` already discards
/// silently), so this matches existing precedent rather than introducing a
/// new one.
///
/// Checkpoints the outgoing project's session first (`persist_session`,
/// using `state.root` before it's reassigned/cleared below) — otherwise
/// whatever drifted since the last discrete session-changing action (e.g.
/// the active tab's cursor position) would be lost the moment the tabs it
/// belongs to are cleared.
pub fn reset_project_scoped_state(state: &mut State) {
    persist_session(state);
    state.open_tabs.clear();
    state.active_tab = None;
    state.closed_tabs.clear();
    state.draft = None;
    state.ctx_menu = None;
    state.changes_panel_open = false;
    state.pending_discard = None;
    state.search_query.clear();
    state.search_query_changed_at = None;
    state.search_in_progress = false;
    if let Some(handle) = state.search_task_handle.take() {
        handle.abort();
    }
    state.search_last_query.clear();
    state.search_results.clear();
    state.search_elapsed = Duration::ZERO;
    state.toasts.clear();
    state.flash = None;
    state.projects_open = false;
    // The LSP subscription is keyed on `state.root` (`run_with` in
    // `subscription`), so changing it below tears down the old worker and
    // spawns a fresh one automatically — these just clear the stale handle/
    // status in the meantime. Unless the user has switched the server off
    // entirely, in which case it stays `Disabled` rather than flashing back
    // to `Starting` for a worker that `subscription` won't actually spawn.
    state.lsp_sender = None;
    state.lsp_status = if state.lsp_enabled { LspStatus::default() } else { LspStatus::Disabled };
    // Same idea as the LSP worker above: `chat_worker` is keyed on
    // `(root, chat_session_id, ..)`, so a fresh id here (rather than
    // reusing whatever the previous project was on) is what actually
    // matters — it guarantees a genuinely new session for the new project
    // rather than accidentally colliding with an id that happens to mean
    // something under the old one.
    state.chat_session_id = claude_agent::new_session_id();
    state.chat_sessions.clear();
    state.chat_sessions_open = false;
    state.chat_view_menu_open = false;
    state.chat = ChatThread::default();
}

pub fn apply_loaded_project(state: &mut State, loaded: LoadedProject) {
    let LoadedProject { root, snapshot } = loaded;
    reset_project_scoped_state(state);
    state.repo = Repo::open(&root);
    state.root = root.clone();
    state.tree = snapshot.tree;
    state.collapsed_dirs = snapshot.collapsed_dirs;
    state.changed_files = snapshot.changed_files;
    state.ahead_behind = snapshot.ahead_behind;
    state.welcome_open = false;
    state.loading_project = None;
    recent_projects::touch(&mut state.recent_projects, root);
    state.welcome_rows = compute_welcome_rows(&state.recent_projects);
    restore_session(state);
}

pub fn close_project(state: &mut State) {
    reset_project_scoped_state(state);
    state.repo = None;
    state.root = PathBuf::new();
    state.tree = Vec::new();
    state.collapsed_dirs = HashSet::new();
    state.changed_files = Vec::new();
    state.ahead_behind = None;
    state.welcome_open = true;
    state.loading_project = None;
    state.welcome_rows = compute_welcome_rows(&state.recent_projects);
}

/// Drives an in-progress sidebar drag (see `Message::SidebarResizeStarted`)
/// with window-wide cursor tracking — the resize handle itself is only a
/// few pixels wide, far narrower than a fast drag's mouse movement, so the
/// handle's own `mouse_area` can't be the thing reporting position once the
/// cursor has left it. Only subscribed while `state.sidebar_resizing`, so
/// idle frames don't pay for a global mouse listener.
pub fn sidebar_resize_events(event: iced::Event, _status: iced::event::Status, _window: iced::window::Id) -> Option<Message> {
    match event {
        iced::Event::Mouse(mouse::Event::CursorMoved { position }) => Some(Message::SidebarResizeDragged(position.x)),
        iced::Event::Mouse(mouse::Event::ButtonReleased(_)) => Some(Message::SidebarResizeEnded),
        _ => None,
    }
}
