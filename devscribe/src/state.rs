use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use devscribe_core::diff::DiffLine;
use devscribe_core::git::{ChangeKind, Repo};
use devscribe_core::lsp::{self, LspCommand, LspEvent};
use devscribe_core::search::{self, SearchHit};
use devscribe_core::syntax::{self, Span};
use devscribe_core::theme::ThemeName;
use devscribe_core::Document;
use iced::futures::channel::mpsc;
use iced::keyboard;

use crate::density::Density;
use crate::fs_tree::{self, Node};

/// Identifies one open tab. Doubles as the dedup/focus key: opening a file
/// or diff that's already open focuses the matching tab instead of opening
/// a duplicate — see `open_or_focus_file`/`open_or_focus_diff`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TabKey {
    File(PathBuf),
    /// A read-only diff view of a specific file against `HEAD` — always
    /// backed by a `File` tab for the same path (see `open_or_focus_diff`),
    /// which is where the actual `DiffStatus` lives.
    Diff(PathBuf),
    Search,
}

/// One entry in `State::open_tabs`. Search isn't one of these — it's a
/// fixed, always-visible icon in the tab bar rather than something that
/// gets opened/closed; see `TabKey::Search` and `tab_bar.rs`.
pub enum OpenTab {
    File(Box<EditorState>),
    Diff(PathBuf),
}

impl OpenTab {
    pub fn key(&self) -> TabKey {
        match self {
            OpenTab::File(editor) => TabKey::File(editor.path.clone()),
            OpenTab::Diff(path) => TabKey::Diff(path.clone()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CursorPos {
    pub line: usize,
    pub col: usize,
}

impl From<(usize, usize)> for CursorPos {
    fn from((line, col): (usize, usize)) -> Self {
        Self { line, col }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
    LineStart,
    LineEnd,
}

/// A diagnostic from the language server, converted into char-based
/// positions (`lsp_types` uses UTF-16 code units) so the editor canvas can
/// place it without re-doing that conversion every frame.
#[derive(Debug, Clone)]
pub struct EditorDiagnostic {
    pub start: CursorPos,
    pub end: CursorPos,
    pub severity: lsp::DiagnosticSeverity,
    pub message: String,
}

/// Status of the language server for the currently supported language
/// (Rust, via `rust-analyzer`). Surfaced in the status bar.
#[derive(Debug, Clone, Default)]
pub enum LspStatus {
    #[default]
    Starting,
    Ready,
    Unavailable(String),
}

/// One project-wide search hit, with the file it was found in.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub path: PathBuf,
    pub hit: SearchHit,
    /// `hit.preview`, broken into syntax-colored runs — same highlighter
    /// the editor canvas uses, run once per file rather than per match.
    /// Empty when the file's language isn't recognized; the UI falls back
    /// to rendering `hit.preview` in one flat color.
    pub segments: Vec<(String, syntax::HighlightKind)>,
    /// `state.search_query`'s length in chars *at the time this result was
    /// computed* — snapshotted so a later, still-unsubmitted query edit
    /// can't desync it from `hit.col`.
    pub query_len_chars: usize,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone)]
pub struct Toast {
    pub id: u64,
    pub kind: ToastKind,
    pub message: String,
    created_at: Instant,
}

const TOAST_LIFETIME: Duration = Duration::from_secs(4);

/// A lighter, non-stacking confirmation pill (`ui/flash.rs`) for immediate
/// direct-action feedback (file/folder created, renamed, path copied, tree
/// collapsed) — distinct from the `Toast` stack above, which is reserved for
/// LSP-readiness and save-result events. Only one shows at a time; starting
/// a new one replaces whatever's currently showing.
#[derive(Debug, Clone)]
pub struct Flash {
    pub text: String,
    created_at: Instant,
}

const FLASH_LIFETIME: Duration = Duration::from_millis(1800);

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
}

/// The settings modal's left-nav categories. Only `Explorer` and `Editor`
/// have real content — `Toolchains`/`Keymap`/`Advanced` are honest
/// "not available yet" placeholders rather than settings that would do
/// nothing (there's no toolchain installer, keymap remapping, or advanced
/// config in DevScribe to back them).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsCategory {
    #[default]
    Explorer,
    Editor,
    Toolchains,
    Keymap,
    Advanced,
}

impl SettingsCategory {
    pub const ALL: [SettingsCategory; 5] = [
        SettingsCategory::Explorer,
        SettingsCategory::Editor,
        SettingsCategory::Toolchains,
        SettingsCategory::Keymap,
        SettingsCategory::Advanced,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SettingsCategory::Explorer => "Explorer",
            SettingsCategory::Editor => "Editor",
            SettingsCategory::Toolchains => "Toolchains",
            SettingsCategory::Keymap => "Keymap",
            SettingsCategory::Advanced => "Advanced",
        }
    }

}

/// A runnable entry in the command palette: file to open, theme to switch
/// to, action to run.
#[derive(Debug, Clone)]
pub enum PaletteAction {
    OpenFile(PathBuf),
    SetTheme(ThemeName),
    FocusSearchTab,
    /// Opens (or focuses) a diff tab for the currently active file tab.
    ViewDiffOfActiveFile,
    CloseActiveTab,
    ToggleAssist,
    ToggleProjects,
    ToggleProblemLens,
    IncreaseEditorFontSize,
    DecreaseEditorFontSize,
    IncreaseUiFontScale,
    DecreaseUiFontScale,
    OpenSettings,
    SaveFile,
}

#[derive(Debug, Clone)]
pub struct PaletteEntry {
    pub label: String,
    pub action: PaletteAction,
}

/// The diff panel's state for the current file, distinguishing the reasons
/// a diff can be empty (worth showing differently) from an actual diff.
#[derive(Debug, Clone, Default)]
pub enum DiffStatus {
    #[default]
    NoRepo,
    /// Tracked, but has no `HEAD` version yet (a new/untracked file).
    Untracked,
    UpToDate,
    Changed(Vec<DiffLine>),
}

/// One match of an in-file "find" query, as an absolute char range — the
/// same coordinate space `EditorState::selection()` and `editor_canvas.rs`
/// already use, so it can be highlighted the same way selection is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FindMatch {
    pub start: usize,
    pub end: usize,
}

/// The current file's in-buffer "find" widget (Ctrl+F) — independent per
/// tab, so switching tabs doesn't lose or bleed one file's find state into
/// another's. Distinct from the project-wide search tab (Ctrl+Shift+F),
/// which searches every file rather than the current buffer.
#[derive(Debug, Clone, Default)]
pub struct FindState {
    pub query: String,
    pub matches: Vec<FindMatch>,
    pub current: usize,
}

/// An open file: its buffer plus interaction state (cursor, selection). Real
/// keyboard/mouse editing lives here; `editor_canvas.rs` only ever reads it
/// and turns raw input events into the `Message`s that call these methods.
pub struct EditorState {
    pub document: Document,
    pub path: PathBuf,
    pub cursor: CursorPos,
    pub selection_anchor: Option<CursorPos>,
    pub language: Option<syntax::Language>,
    pub highlights: Vec<Span>,
    highlighter: syntax::Highlighter,
    pub diagnostics: Vec<EditorDiagnostic>,
    /// `Some(Ok(_))`/`Some(Err(parse_message))` for `.json` files, `None`
    /// otherwise. Recomputed on every edit, like `highlights`.
    pub json: Option<Result<serde_json::Value, String>>,
    /// Collapsed node paths in the JSON tree view (e.g. `"root.foo[2]"`).
    pub json_collapsed: HashSet<String>,
    /// This file's content at `HEAD` diffed against the live buffer.
    pub diff: DiffStatus,
    /// `Some` while this tab's find widget (Ctrl+F) is open.
    pub find: Option<FindState>,
}

impl EditorState {
    pub fn new(document: Document, path: PathBuf) -> Self {
        let language = path
            .extension()
            .and_then(|ext| ext.to_str())
            .and_then(syntax::Language::from_extension);
        let mut highlighter = syntax::Highlighter::new();
        let highlights = match language {
            Some(lang) => highlighter.highlight(lang, &document.text().to_string()),
            None => Vec::new(),
        };
        let mut this = Self {
            document,
            path,
            cursor: CursorPos::default(),
            selection_anchor: None,
            language,
            highlights,
            highlighter,
            diagnostics: Vec::new(),
            json: None,
            json_collapsed: HashSet::new(),
            diff: DiffStatus::default(),
            find: None,
        };
        this.reparse_json();
        this
    }

    /// Recomputes `highlights` from the current buffer contents. Cheap
    /// relative to a full reparse would suggest otherwise, but tree-sitter
    /// is fast enough that doing this on every edit is fine — see
    /// `devscribe_core::syntax` for why this isn't true incremental reparsing.
    fn rehighlight(&mut self) {
        if let Some(lang) = self.language {
            self.highlights = self
                .highlighter
                .highlight(lang, &self.document.text().to_string());
        }
    }

    /// Recomputes `json` from the current buffer contents, for `.json` files.
    fn reparse_json(&mut self) {
        self.json = (self.language == Some(syntax::Language::Json)).then(|| {
            serde_json::from_str::<serde_json::Value>(&self.document.text().to_string())
                .map_err(|e| e.to_string())
        });
    }

    /// Recomputes `find`'s matches from its current query against the
    /// current buffer contents, for `.find.is_some()` files. Called both
    /// when the query changes and on every edit, like `rehighlight`.
    fn refind(&mut self) {
        let Some(query) = self.find.as_ref().map(|f| f.query.clone()) else {
            return;
        };
        let matches = if query.is_empty() {
            Vec::new()
        } else {
            let text = self.document.text().to_string();
            let query_len = query.chars().count();
            search::search_text(&text, &query)
                .into_iter()
                .map(|hit| {
                    let start = self.document.char_index(hit.line, hit.col);
                    FindMatch { start, end: start + query_len }
                })
                .collect()
        };
        if let Some(find) = self.find.as_mut() {
            find.current = find.current.min(matches.len().saturating_sub(1));
            find.matches = matches;
        }
    }

    fn selection_range(&self) -> Option<(usize, usize)> {
        self.selection_anchor.map(|anchor| {
            let a = self.document.char_index(anchor.line, anchor.col);
            let b = self.document.char_index(self.cursor.line, self.cursor.col);
            if a <= b {
                (a, b)
            } else {
                (b, a)
            }
        })
    }

    /// Non-empty (start, end) char range currently selected, if any.
    pub fn selection(&self) -> Option<(usize, usize)> {
        self.selection_range().filter(|(start, end)| start != end)
    }

    /// Deletes the current selection, if any. Returns whether it deleted anything.
    fn delete_selection(&mut self) -> bool {
        let range = self.selection();
        self.selection_anchor = None;
        if let Some((start, end)) = range {
            self.document.remove(start..end);
            self.cursor = self.document.line_col(start).into();
            true
        } else {
            false
        }
    }

    pub fn insert_text(&mut self, text: &str) {
        self.delete_selection();
        let idx = self.document.char_index(self.cursor.line, self.cursor.col);
        self.document.insert(idx, text);
        let new_idx = idx + text.chars().count();
        self.cursor = self.document.line_col(new_idx).into();
        self.rehighlight();
        self.reparse_json();
        self.refind();
    }

    pub fn backspace(&mut self) {
        if self.delete_selection() {
            self.rehighlight();
            self.reparse_json();
            self.refind();
            return;
        }
        let idx = self.document.char_index(self.cursor.line, self.cursor.col);
        if idx == 0 {
            return;
        }
        self.document.remove(idx - 1..idx);
        self.cursor = self.document.line_col(idx - 1).into();
        self.rehighlight();
        self.reparse_json();
        self.refind();
    }

    pub fn delete_forward(&mut self) {
        if self.delete_selection() {
            self.rehighlight();
            self.reparse_json();
            self.refind();
            return;
        }
        let idx = self.document.char_index(self.cursor.line, self.cursor.col);
        if idx >= self.document.text().len_chars() {
            return;
        }
        self.document.remove(idx..idx + 1);
        self.cursor = self.document.line_col(idx).into();
        self.rehighlight();
        self.reparse_json();
        self.refind();
    }

    pub fn move_cursor(&mut self, dir: Direction, extend: bool) {
        if extend {
            if self.selection_anchor.is_none() {
                self.selection_anchor = Some(self.cursor);
            }
        } else {
            self.selection_anchor = None;
        }

        match dir {
            Direction::Left => {
                let idx = self.document.char_index(self.cursor.line, self.cursor.col);
                if idx > 0 {
                    self.cursor = self.document.line_col(idx - 1).into();
                }
            }
            Direction::Right => {
                let idx = self.document.char_index(self.cursor.line, self.cursor.col);
                if idx < self.document.text().len_chars() {
                    self.cursor = self.document.line_col(idx + 1).into();
                }
            }
            Direction::Up => {
                if self.cursor.line > 0 {
                    self.cursor.line -= 1;
                    self.cursor.col = self
                        .cursor
                        .col
                        .min(self.document.line_len_chars(self.cursor.line));
                }
            }
            Direction::Down => {
                if self.cursor.line + 1 < self.document.line_count() {
                    self.cursor.line += 1;
                    self.cursor.col = self
                        .cursor
                        .col
                        .min(self.document.line_len_chars(self.cursor.line));
                }
            }
            Direction::LineStart => self.cursor.col = 0,
            Direction::LineEnd => {
                self.cursor.col = self.document.line_len_chars(self.cursor.line);
            }
        }
    }

    pub fn click(&mut self, line: usize, col: usize, extend: bool) {
        if extend {
            if self.selection_anchor.is_none() {
                self.selection_anchor = Some(self.cursor);
            }
        } else {
            self.selection_anchor = None;
        }
        self.cursor = CursorPos { line, col };
    }
}

pub struct State {
    pub theme: ThemeName,
    /// Every currently open tab, in the order they appear in the tab bar.
    pub open_tabs: Vec<OpenTab>,
    /// `None` only when `open_tabs` is empty.
    pub active_tab: Option<TabKey>,
    pub assist_on: bool,
    pub projects_open: bool,
    pub overflow_open: bool,
    /// Project root the sidebar tree was walked from.
    pub root: PathBuf,
    /// Walked once at startup (filesystem walks are far too slow to redo on
    /// every `view()` — the caret-blink subscription alone redraws 2x/sec).
    pub tree: Vec<Node>,
    /// Directories collapsed in the sidebar tree, keyed by absolute path.
    pub collapsed_dirs: HashSet<PathBuf>,
    pub caret_visible: bool,
    pub lsp_status: LspStatus,
    lsp_sender: Option<mpsc::Sender<LspCommand>>,
    /// `None` when `root` isn't a git repository — an expected, common case.
    pub repo: Option<Repo>,
    /// The sidebar's "CHANGES" panel contents — every file that differs from
    /// `HEAD`, across the whole project (not just open tabs). Empty when
    /// `repo` is `None`. See `refresh_changed_files` for when this updates.
    pub changed_files: Vec<ChangesEntry>,
    pub changes_panel_open: bool,
    pub search_query: String,
    pub search_results: Vec<SearchResult>,
    pub search_elapsed: Duration,
    pub palette_open: bool,
    pub palette_query: String,
    pub palette_selected: usize,
    pub settings_open: bool,
    pub settings_category: SettingsCategory,
    pub density: Density,
    pub problem_lens_enabled: bool,
    /// Gates the file tree's per-file `ChangeKind` badges (`sidebar.rs`).
    /// Doesn't affect the separate "CHANGES" panel, which has its own
    /// collapse toggle.
    pub git_status_in_tree: bool,
    pub editor_font_size: f32,
    /// Multiplies every chrome text size (sidebar, tabs, status bar, title
    /// bar, palette, settings, toasts). Independent of `editor_font_size` —
    /// see `text_scale`.
    pub ui_font_scale: f32,
    pub toasts: Vec<Toast>,
    next_toast_id: u64,
    /// Non-`None` while the sidebar tree has an inline new-file/new-folder/
    /// rename draft open. See `Draft`.
    pub draft: Option<Draft>,
    /// Non-`None` while the tree's right-click context menu is open.
    pub ctx_menu: Option<ContextMenu>,
    /// The single current "flash" confirmation pill, if any. See `Flash`.
    pub flash: Option<Flash>,
    /// LIFO stack of tabs closed via `Message::CloseTab`/`CloseActiveTab`/
    /// `CloseOtherTabs` (not tabs closed only as a side effect, e.g. a
    /// `Diff` tab auto-closed with its backing `File` tab) — `ReopenClosedTab`
    /// pops and reopens the most recent. Capped at `MAX_CLOSED_TABS`.
    pub closed_tabs: Vec<TabKey>,
}

const MAX_CLOSED_TABS: usize = 20;

pub const EDITOR_FONT_SIZE_MIN: f32 = 10.0;
pub const EDITOR_FONT_SIZE_MAX: f32 = 24.0;
pub const EDITOR_FONT_SIZE_DEFAULT: f32 = 13.0;
pub const EDITOR_FONT_SIZE_STEP: f32 = 1.0;

pub const UI_FONT_SCALE_MIN: f32 = 0.8;
pub const UI_FONT_SCALE_MAX: f32 = 1.5;
pub const UI_FONT_SCALE_DEFAULT: f32 = 1.0;
pub const UI_FONT_SCALE_STEP: f32 = 0.1;

impl Default for State {
    fn default() -> Self {
        let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let tree = fs_tree::walk(&root);
        // Directories start collapsed — an uncollapsed default would dump
        // the whole project tree open on first launch.
        let collapsed_dirs: HashSet<PathBuf> =
            fs_tree::flatten_dirs(&tree).into_iter().map(Path::to_path_buf).collect();
        let repo = Repo::open(&root);
        let changed_files = compute_changed_files(repo.as_ref());
        Self {
            theme: ThemeName::NullGrid,
            open_tabs: Vec::new(),
            active_tab: None,
            assist_on: true,
            projects_open: false,
            overflow_open: false,
            root,
            tree,
            collapsed_dirs,
            caret_visible: true,
            lsp_status: LspStatus::default(),
            lsp_sender: None,
            repo,
            changed_files,
            changes_panel_open: false,
            search_query: String::new(),
            search_results: Vec::new(),
            search_elapsed: Duration::ZERO,
            palette_open: false,
            palette_query: String::new(),
            palette_selected: 0,
            settings_open: false,
            settings_category: SettingsCategory::default(),
            git_status_in_tree: false,
            density: Density::default(),
            problem_lens_enabled: true,
            editor_font_size: EDITOR_FONT_SIZE_DEFAULT,
            ui_font_scale: UI_FONT_SCALE_DEFAULT,
            toasts: Vec::new(),
            next_toast_id: 0,
            draft: None,
            ctx_menu: None,
            flash: None,
            closed_tabs: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    SetTheme(ThemeName),
    SelectOpenTab(TabKey),
    CloseTab(TabKey),
    CloseActiveTab,
    FocusSearchTab,
    ToggleAssist,
    ToggleProjects,
    ToggleOverflow,
    ToggleChangesPanel,
    /// Opens (or focuses) a diff tab for `path`, from a sidebar Changes row.
    /// Distinct from `PaletteAction::ViewDiffOfActiveFile`, which only ever
    /// targets the currently active tab.
    OpenDiffFor(PathBuf),
    SelectFile(PathBuf),
    EditorInsertText(String),
    EditorBackspace,
    EditorDelete,
    EditorMove { dir: Direction, extend: bool },
    EditorClick { line: usize, col: usize, extend: bool },
    CaretTick,
    Lsp(LspEvent),
    JsonToggle(String),
    ToggleDirCollapsed(PathBuf),
    SearchQueryChanged(String),
    SearchSubmit,
    SearchResultSelected { path: PathBuf, line: usize, col: usize },
    TogglePalette,
    ClosePalette,
    PaletteQueryChanged(String),
    PaletteMove(i32),
    PaletteExecute,
    PaletteRun(PaletteAction),
    ToggleSettings,
    CloseSettings,
    SetDensity(Density),
    ToggleProblemLens,
    SetSettingsCategory(SettingsCategory),
    ToggleGitStatusInTree,
    SetEditorFontSize(f32),
    SetUiFontScale(f32),
    DismissToast(u64),
    PruneToasts,
    EditorSave,
    ToggleFind,
    CloseFind,
    FindQueryChanged(String),
    FindNext,
    FindPrev,
    EscapePressed,
    /// Starts a new-file/new-folder draft in the project root — the
    /// Explorer header buttons and the global Ctrl/Cmd+N / Ctrl/Cmd+Shift+N
    /// shortcuts.
    BeginDraft(DraftKind),
    /// Starts a new-file/new-folder draft in a specific directory — the
    /// tree's right-click context menu, which already knows its target.
    BeginDraftIn(DraftKind, PathBuf),
    /// Starts an in-place rename draft for `path` — context menu only.
    BeginRename(PathBuf),
    DraftTextChanged(String),
    CommitDraft,
    CancelDraft,
    /// Collapses every directory in the tree (real, not decorative — see
    /// `Phase 5` writeup).
    CollapseAllDirs,
    /// Opens the tree's right-click context menu. `None` targets the tree
    /// background/root.
    OpenTreeContext(Option<PathBuf>),
    CloseTreeContext,
    CopyPath(PathBuf),
    CloseOtherTabs,
    RevealActiveInTree,
    ReopenClosedTab,
    Noop,
}

pub fn update(state: &mut State, message: Message) -> iced::Task<Message> {
    match message {
        Message::SetTheme(theme) => state.theme = theme,
        Message::SelectOpenTab(key) => {
            if state.open_tabs.iter().any(|t| t.key() == key) {
                state.active_tab = Some(key);
            }
        }
        Message::CloseTab(key) => close_tab(state, &key),
        Message::CloseActiveTab => {
            if let Some(key) = state.active_tab.clone() {
                close_tab(state, &key);
            }
        }
        Message::FocusSearchTab => focus_search(state),
        Message::ToggleAssist => state.assist_on = !state.assist_on,
        Message::ToggleProjects => state.projects_open = !state.projects_open,
        Message::ToggleOverflow => state.overflow_open = !state.overflow_open,
        Message::ToggleChangesPanel => state.changes_panel_open = !state.changes_panel_open,
        Message::OpenDiffFor(path) => open_or_focus_diff(state, path),
        Message::SelectFile(path) => open_or_focus_file(state, path),
        Message::EditorInsertText(text) => {
            if let Some(path) = active_file_path(state) {
                if let Some(editor) = find_editor_mut(state, &path) {
                    editor.insert_text(&text);
                }
                send_did_change_for(state, &path);
                recompute_diff_for(state, &path);
            }
        }
        Message::EditorBackspace => {
            if let Some(path) = active_file_path(state) {
                if let Some(editor) = find_editor_mut(state, &path) {
                    editor.backspace();
                }
                send_did_change_for(state, &path);
                recompute_diff_for(state, &path);
            }
        }
        Message::EditorDelete => {
            if let Some(path) = active_file_path(state) {
                if let Some(editor) = find_editor_mut(state, &path) {
                    editor.delete_forward();
                }
                send_did_change_for(state, &path);
                recompute_diff_for(state, &path);
            }
        }
        Message::EditorMove { dir, extend } => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
            {
                editor.move_cursor(dir, extend);
            }
        }
        Message::EditorClick { line, col, extend } => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
            {
                editor.click(line, col, extend);
            }
        }
        Message::CaretTick => state.caret_visible = !state.caret_visible,
        Message::Lsp(event) => match event {
            LspEvent::Ready(sender) => {
                let was_starting = matches!(state.lsp_status, LspStatus::Starting);
                state.lsp_status = LspStatus::Ready;
                state.lsp_sender = Some(sender);
                for path in open_file_paths(state) {
                    send_did_open_for(state, &path);
                }
                if was_starting {
                    push_toast(state, ToastKind::Success, "rust-analyzer ready");
                }
            }
            LspEvent::Diagnostics { uri, diagnostics } => {
                if let Some(path) = uri.to_file_path().ok()
                    && let Some(editor) = find_editor_mut(state, &path)
                {
                    editor.diagnostics = convert_diagnostics(&editor.document, diagnostics);
                }
            }
            LspEvent::Unavailable(reason) => {
                state.lsp_status = LspStatus::Unavailable(reason.clone());
                state.lsp_sender = None;
                push_toast(state, ToastKind::Warning, format!("rust-analyzer unavailable: {reason}"));
            }
        },
        Message::ToggleDirCollapsed(path) => {
            if !state.collapsed_dirs.remove(&path) {
                state.collapsed_dirs.insert(path);
            }
        }
        Message::JsonToggle(key) => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
                && !editor.json_collapsed.remove(&key)
            {
                editor.json_collapsed.insert(key);
            }
        }
        Message::SearchQueryChanged(query) => {
            state.search_query = query;
            recompute_search(state);
        }
        Message::SearchSubmit => focus_search(state),
        Message::SearchResultSelected { path, line, col } => {
            open_or_focus_file(state, path.clone());
            if let Some(editor) = find_editor_mut(state, &path) {
                editor.cursor = CursorPos { line, col };
            }
        }
        Message::TogglePalette => {
            state.palette_open = !state.palette_open;
            if state.palette_open {
                state.palette_query.clear();
                state.palette_selected = 0;
                state.settings_open = false;
                return iced::widget::operation::focus(palette_query_id());
            }
        }
        Message::ClosePalette => state.palette_open = false,
        Message::PaletteQueryChanged(query) => {
            state.palette_query = query;
            state.palette_selected = 0;
        }
        Message::PaletteMove(delta) => {
            let len = filtered_palette_entries(state).len();
            state.palette_selected = if len == 0 {
                0
            } else {
                ((state.palette_selected as i32 + delta).rem_euclid(len as i32)) as usize
            };
        }
        Message::PaletteExecute => {
            if let Some(entry) = filtered_palette_entries(state).get(state.palette_selected) {
                run_palette_action(state, entry.action.clone());
            }
            state.palette_open = false;
        }
        Message::PaletteRun(action) => {
            run_palette_action(state, action);
            state.palette_open = false;
        }
        Message::ToggleSettings => {
            state.settings_open = !state.settings_open;
            if state.settings_open {
                state.palette_open = false;
            }
        }
        Message::CloseSettings => state.settings_open = false,
        Message::SetDensity(density) => state.density = density,
        Message::ToggleProblemLens => state.problem_lens_enabled = !state.problem_lens_enabled,
        Message::SetSettingsCategory(category) => state.settings_category = category,
        Message::ToggleGitStatusInTree => state.git_status_in_tree = !state.git_status_in_tree,
        Message::SetEditorFontSize(size) => {
            state.editor_font_size = size.clamp(EDITOR_FONT_SIZE_MIN, EDITOR_FONT_SIZE_MAX);
        }
        Message::SetUiFontScale(scale) => {
            state.ui_font_scale = scale.clamp(UI_FONT_SCALE_MIN, UI_FONT_SCALE_MAX);
        }
        Message::DismissToast(id) => state.toasts.retain(|t| t.id != id),
        Message::PruneToasts => {
            state.toasts.retain(|t| t.created_at.elapsed() < TOAST_LIFETIME);
            if state.flash.as_ref().is_some_and(|f| f.created_at.elapsed() >= FLASH_LIFETIME) {
                state.flash = None;
            }
        }
        Message::EditorSave => save_current_file(state),
        Message::ToggleFind => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
            {
                if editor.find.is_some() {
                    editor.find = None;
                } else {
                    let initial_query = editor
                        .selection()
                        .map(|(start, end)| editor.document.text().slice(start..end).to_string())
                        .unwrap_or_default();
                    editor.find = Some(FindState {
                        query: initial_query,
                        ..FindState::default()
                    });
                    editor.refind();
                    return iced::widget::operation::focus(find_query_id());
                }
            }
        }
        Message::CloseFind => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
            {
                editor.find = None;
            }
        }
        Message::FindQueryChanged(query) => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
            {
                if let Some(find) = editor.find.as_mut() {
                    find.query = query;
                    find.current = 0;
                }
                editor.refind();
            }
        }
        Message::FindNext => find_step(state, 1),
        Message::FindPrev => find_step(state, -1),
        Message::BeginDraft(kind) => return begin_draft(state, kind, state.root.clone()),
        Message::BeginDraftIn(kind, dir) => return begin_draft(state, kind, dir),
        Message::BeginRename(path) => return begin_rename(state, path),
        Message::DraftTextChanged(text) => {
            if let Some(draft) = state.draft.as_mut() {
                draft.text = text;
            }
        }
        Message::CommitDraft => commit_draft(state),
        Message::CancelDraft => state.draft = None,
        Message::CollapseAllDirs => {
            state.collapsed_dirs = fs_tree::flatten_dirs(&state.tree).into_iter().map(Path::to_path_buf).collect();
            push_flash(state, "TREE COLLAPSED");
        }
        Message::OpenTreeContext(target) => {
            state.ctx_menu = Some(ContextMenu { target });
            state.overflow_open = false;
            state.projects_open = false;
        }
        Message::CloseTreeContext => state.ctx_menu = None,
        Message::CopyPath(path) => {
            state.ctx_menu = None;
            let label = path.strip_prefix(&state.root).unwrap_or(&path).display().to_string();
            push_flash(state, format!("PATH COPIED // {label}"));
            return iced::clipboard::write(path.display().to_string());
        }
        Message::CloseOtherTabs => {
            state.overflow_open = false;
            close_other_tabs(state);
        }
        Message::RevealActiveInTree => {
            state.overflow_open = false;
            reveal_active_in_tree(state);
        }
        Message::ReopenClosedTab => {
            state.overflow_open = false;
            reopen_closed_tab(state);
        }
        Message::EscapePressed => {
            let find_open = active_file_path(state)
                .and_then(|path| find_editor(state, &path))
                .is_some_and(|editor| editor.find.is_some());
            if find_open {
                if let Some(path) = active_file_path(state)
                    && let Some(editor) = find_editor_mut(state, &path)
                {
                    editor.find = None;
                }
            } else if state.draft.is_some() {
                state.draft = None;
            } else if state.ctx_menu.is_some() {
                state.ctx_menu = None;
            } else if state.overflow_open {
                state.overflow_open = false;
            } else if state.palette_open {
                state.palette_open = false;
            } else if state.settings_open {
                state.settings_open = false;
            }
        }
        Message::Noop => {}
    }
    iced::Task::none()
}

fn run_palette_action(state: &mut State, action: PaletteAction) {
    match action {
        PaletteAction::OpenFile(path) => open_or_focus_file(state, path),
        PaletteAction::SetTheme(theme) => state.theme = theme,
        PaletteAction::FocusSearchTab => focus_search(state),
        PaletteAction::ViewDiffOfActiveFile => {
            if let Some(path) = active_file_path(state) {
                open_or_focus_diff(state, path);
            }
        }
        PaletteAction::CloseActiveTab => {
            if let Some(key) = state.active_tab.clone() {
                close_tab(state, &key);
            }
        }
        PaletteAction::ToggleAssist => state.assist_on = !state.assist_on,
        PaletteAction::ToggleProjects => state.projects_open = !state.projects_open,
        PaletteAction::ToggleProblemLens => state.problem_lens_enabled = !state.problem_lens_enabled,
        PaletteAction::IncreaseEditorFontSize => {
            state.editor_font_size =
                (state.editor_font_size + EDITOR_FONT_SIZE_STEP).clamp(EDITOR_FONT_SIZE_MIN, EDITOR_FONT_SIZE_MAX);
        }
        PaletteAction::DecreaseEditorFontSize => {
            state.editor_font_size =
                (state.editor_font_size - EDITOR_FONT_SIZE_STEP).clamp(EDITOR_FONT_SIZE_MIN, EDITOR_FONT_SIZE_MAX);
        }
        PaletteAction::IncreaseUiFontScale => {
            state.ui_font_scale =
                (state.ui_font_scale + UI_FONT_SCALE_STEP).clamp(UI_FONT_SCALE_MIN, UI_FONT_SCALE_MAX);
        }
        PaletteAction::DecreaseUiFontScale => {
            state.ui_font_scale =
                (state.ui_font_scale - UI_FONT_SCALE_STEP).clamp(UI_FONT_SCALE_MIN, UI_FONT_SCALE_MAX);
        }
        PaletteAction::OpenSettings => state.settings_open = true,
        PaletteAction::SaveFile => save_current_file(state),
    }
}

fn save_current_file(state: &mut State) {
    let Some(path) = active_file_path(state) else {
        return;
    };
    let Some(editor) = find_editor_mut(state, &path) else {
        return;
    };
    let name = editor
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    match editor.document.save() {
        Ok(()) => {
            push_toast(state, ToastKind::Success, format!("Saved {name}"));
            refresh_changed_files(state);
        }
        Err(err) => push_toast(state, ToastKind::Error, format!("Couldn't save {name}: {err}")),
    }
}

fn push_toast(state: &mut State, kind: ToastKind, message: impl Into<String>) {
    let id = state.next_toast_id;
    state.next_toast_id += 1;
    state.toasts.push(Toast {
        id,
        kind,
        message: message.into(),
        created_at: Instant::now(),
    });
}

fn push_flash(state: &mut State, text: impl Into<String>) {
    state.flash = Some(Flash {
        text: text.into(),
        created_at: Instant::now(),
    });
}

/// A stable id for the command palette's search box, so `update()` can focus
/// it (via `iced::widget::operation::focus`) the moment the palette opens.
pub fn palette_query_id() -> iced::widget::Id {
    iced::widget::Id::new("command-palette-query")
}

/// A stable id for the in-file find widget's search box, so `update()` can
/// focus it the moment Ctrl+F opens it.
pub fn find_query_id() -> iced::widget::Id {
    iced::widget::Id::new("find-query")
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
fn refresh_tree(state: &mut State) {
    state.tree = fs_tree::walk(&state.root);
}

fn begin_draft(state: &mut State, kind: DraftKind, dir: PathBuf) -> iced::Task<Message> {
    state.draft = Some(Draft {
        kind,
        dir,
        target: None,
        text: String::new(),
    });
    state.ctx_menu = None;
    iced::widget::operation::focus(draft_input_id())
}

fn begin_rename(state: &mut State, path: PathBuf) -> iced::Task<Message> {
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
fn commit_draft(state: &mut State) {
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
fn rename_open_tab(state: &mut State, old_path: &Path, new_path: &Path) {
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
}

/// Closes every open tab except `active_tab` — the tab-bar overflow menu's
/// "Close others". Reuses `close_tab` per key (rather than a bulk `retain`)
/// so LSP `didClose` notifications and diff-tab cleanup still happen.
fn close_other_tabs(state: &mut State) {
    let Some(active) = state.active_tab.clone() else {
        return;
    };
    let others: Vec<TabKey> = state.open_tabs.iter().map(|t| t.key()).filter(|k| *k != active).collect();
    for key in others {
        close_tab(state, &key);
    }
}

/// Expands every ancestor directory of the active file so it's visible in
/// the tree — the tab-bar overflow menu's "Reveal in tree". Doesn't scroll
/// the tree to it: `sidebar.rs`'s tree `scrollable` has no stable `.id()`
/// wired up yet, the same known gap as Ctrl+F's auto-scroll-to-match.
fn reveal_active_in_tree(state: &mut State) {
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

/// Pops and reopens the most recently closed tab — the tab-bar overflow
/// menu's "Reopen closed tab". Only tabs explicitly closed via `close_tab`
/// are on this stack (see `State::closed_tabs`), not ones closed only as a
/// side effect (a `Diff` tab auto-closed with its backing `File` tab).
fn reopen_closed_tab(state: &mut State) {
    while let Some(key) = state.closed_tabs.pop() {
        match key {
            TabKey::File(path) => {
                open_or_focus_file(state, path);
                return;
            }
            TabKey::Diff(path) => {
                open_or_focus_diff(state, path);
                return;
            }
            TabKey::Search => {}
        }
    }
}

/// Moves the active file tab's find selection by `delta` (wrapping) and
/// moves the cursor to the newly-current match.
fn find_step(state: &mut State, delta: i32) {
    let Some(path) = active_file_path(state) else {
        return;
    };
    let Some(editor) = find_editor_mut(state, &path) else {
        return;
    };
    let Some(find) = editor.find.as_mut() else {
        return;
    };
    if find.matches.is_empty() {
        return;
    }
    let len = find.matches.len() as i32;
    find.current = (find.current as i32 + delta).rem_euclid(len) as usize;
    let target = find.matches[find.current];
    editor.cursor = editor.document.line_col(target.start).into();
    editor.selection_anchor = None;
}

const MAX_PALETTE_RESULTS: usize = 50;

fn all_palette_entries(state: &State) -> Vec<PaletteEntry> {
    let mut entries = Vec::new();

    for path in fs_tree::flatten_files(&state.tree) {
        let label = path
            .strip_prefix(&state.root)
            .unwrap_or(path)
            .display()
            .to_string();
        entries.push(PaletteEntry {
            label: format!("Open: {label}"),
            action: PaletteAction::OpenFile(path.to_path_buf()),
        });
    }

    for theme in ThemeName::ALL {
        entries.push(PaletteEntry {
            label: format!("Theme: {}", theme.label()),
            action: PaletteAction::SetTheme(theme),
        });
    }

    entries.push(PaletteEntry {
        label: "Go to: Search".to_string(),
        action: PaletteAction::FocusSearchTab,
    });
    if let Some(path) = active_file_path(state) {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        entries.push(PaletteEntry {
            label: format!("View Diff: {name} \u{2194} HEAD"),
            action: PaletteAction::ViewDiffOfActiveFile,
        });
    }
    if state.active_tab.is_some() {
        entries.push(PaletteEntry {
            label: "Close active tab".to_string(),
            action: PaletteAction::CloseActiveTab,
        });
    }

    entries.push(PaletteEntry {
        label: "Toggle Assist panel".to_string(),
        action: PaletteAction::ToggleAssist,
    });
    entries.push(PaletteEntry {
        label: "Toggle Projects panel".to_string(),
        action: PaletteAction::ToggleProjects,
    });
    entries.push(PaletteEntry {
        label: format!(
            "{} inline problem hints",
            if state.problem_lens_enabled { "Hide" } else { "Show" }
        ),
        action: PaletteAction::ToggleProblemLens,
    });
    entries.push(PaletteEntry {
        label: "Increase editor font size".to_string(),
        action: PaletteAction::IncreaseEditorFontSize,
    });
    entries.push(PaletteEntry {
        label: "Decrease editor font size".to_string(),
        action: PaletteAction::DecreaseEditorFontSize,
    });
    entries.push(PaletteEntry {
        label: "Increase UI font size".to_string(),
        action: PaletteAction::IncreaseUiFontScale,
    });
    entries.push(PaletteEntry {
        label: "Decrease UI font size".to_string(),
        action: PaletteAction::DecreaseUiFontScale,
    });
    entries.push(PaletteEntry {
        label: "Open Settings".to_string(),
        action: PaletteAction::OpenSettings,
    });
    if active_file_path(state).is_some() {
        entries.push(PaletteEntry {
            label: "Save File".to_string(),
            action: PaletteAction::SaveFile,
        });
    }

    entries
}

/// The palette entries matching `state.palette_query`, in the same order
/// both the view and `PaletteExecute`/`PaletteMove` use — keeping them in
/// sync is what makes "press Enter" run the entry actually shown selected.
pub fn filtered_palette_entries(state: &State) -> Vec<PaletteEntry> {
    let query = state.palette_query.to_ascii_lowercase();
    all_palette_entries(state)
        .into_iter()
        .filter(|entry| query.is_empty() || entry.label.to_ascii_lowercase().contains(&query))
        .take(MAX_PALETTE_RESULTS)
        .collect()
}

/// The path of the active tab, if it's a `File` tab (not `Diff` or `Search`).
pub fn active_file_path(state: &State) -> Option<PathBuf> {
    match state.active_tab.as_ref()? {
        TabKey::File(path) => Some(path.clone()),
        _ => None,
    }
}

pub fn find_editor<'a>(state: &'a State, path: &Path) -> Option<&'a EditorState> {
    state.open_tabs.iter().find_map(|t| match t {
        OpenTab::File(editor) if editor.path == path => Some(editor.as_ref()),
        _ => None,
    })
}

fn find_editor_mut<'a>(state: &'a mut State, path: &Path) -> Option<&'a mut EditorState> {
    state.open_tabs.iter_mut().find_map(|t| match t {
        OpenTab::File(editor) if editor.path == path => Some(editor.as_mut()),
        _ => None,
    })
}

/// The active tab's `EditorState`, if the active tab is a `File`.
pub fn active_editor(state: &State) -> Option<&EditorState> {
    find_editor(state, &active_file_path(state)?)
}

/// Every currently open `File` tab's path — used to replay `didOpen` for all
/// of them once the LSP server becomes ready (it may become ready after
/// files were already opened).
fn open_file_paths(state: &State) -> Vec<PathBuf> {
    state
        .open_tabs
        .iter()
        .filter_map(|t| match t {
            OpenTab::File(editor) => Some(editor.path.clone()),
            _ => None,
        })
        .collect()
}

/// Opens `path` as a `File` tab, or focuses it if already open. Shared by
/// `SelectFile`, `SearchResultSelected`, and the palette's file-open entries
/// — this is what makes opening a second file additive instead of a replace.
fn open_or_focus_file(state: &mut State, path: PathBuf) {
    let key = TabKey::File(path.clone());
    if state.open_tabs.iter().any(|t| t.key() == key) {
        state.active_tab = Some(key);
        return;
    }
    if let Ok(document) = Document::open(&path) {
        state.open_tabs.push(OpenTab::File(Box::new(EditorState::new(document, path.clone()))));
        state.active_tab = Some(key);
        send_did_open_for(state, &path);
        recompute_diff_for(state, &path);
    }
}

/// Opens a diff tab for `path`, or focuses it if already open. Always
/// ensures a backing `File` tab exists first (opening one if needed) since
/// that's where the actual `DiffStatus` is computed and cached.
fn open_or_focus_diff(state: &mut State, path: PathBuf) {
    open_or_focus_file(state, path.clone());
    if find_editor(state, &path).is_none() {
        // The file failed to open (e.g. deleted on disk) — nothing to diff.
        return;
    }
    let key = TabKey::Diff(path.clone());
    if !state.open_tabs.iter().any(|t| t.key() == key) {
        state.open_tabs.push(OpenTab::Diff(path));
    }
    state.active_tab = Some(key);
}

/// Opens the (singleton) search tab, or focuses it if already open.
fn focus_search(state: &mut State) {
    state.active_tab = Some(TabKey::Search);
}

/// Closes the tab matching `key`. Closing a `File` tab also closes its diff
/// tab, if any (a `Diff` tab has no content without its backing `File`
/// tab), and notifies the LSP server. If the active tab was closed, focuses
/// the tab that's now in its place, or `None` if that was the last one.
fn close_tab(state: &mut State, key: &TabKey) {
    let Some(pos) = state.open_tabs.iter().position(|t| &t.key() == key) else {
        return;
    };
    state.closed_tabs.retain(|k| k != key);
    state.closed_tabs.push(key.clone());
    if state.closed_tabs.len() > MAX_CLOSED_TABS {
        state.closed_tabs.remove(0);
    }
    let removed = state.open_tabs.remove(pos);
    if let OpenTab::File(editor) = &removed {
        if let Some(sender) = state.lsp_sender.as_mut() {
            send_did_close(sender, &editor.path);
        }
        let diff_key = TabKey::Diff(editor.path.clone());
        state.open_tabs.retain(|t| t.key() != diff_key);
    }

    // The active tab may no longer exist — either it was the one closed, or
    // (if it was a `Diff` tab) it was closed as a side effect of its
    // backing `File` tab closing just above.
    let active_still_open = state
        .active_tab
        .as_ref()
        .is_some_and(|active| state.open_tabs.iter().any(|t| &t.key() == active));
    if !active_still_open {
        state.active_tab = state
            .open_tabs
            .get(pos.min(state.open_tabs.len().saturating_sub(1)))
            .map(|t| t.key());
    }
}

/// Recomputes `path`'s diff against its `HEAD` version, if it's open.
fn recompute_diff_for(state: &mut State, path: &Path) {
    let Some(current_text) = find_editor(state, path).map(|e| e.document.text().to_string())
    else {
        return;
    };

    let status = match state.repo.as_ref().and_then(|repo| repo.head_text(path)) {
        None if state.repo.is_none() => DiffStatus::NoRepo,
        None => DiffStatus::Untracked,
        Some(old) => {
            let lines = devscribe_core::diff::diff_lines(&old, &current_text);
            if lines.iter().all(|l| l.kind == devscribe_core::diff::DiffLineKind::Equal) {
                DiffStatus::UpToDate
            } else {
                DiffStatus::Changed(lines)
            }
        }
    };

    if let Some(editor) = find_editor_mut(state, path) {
        editor.diff = status;
    }
}

/// Computes the sidebar's "CHANGES" panel contents: every file `repo`
/// reports as differing from `HEAD`, with insertion/deletion counts from
/// `devscribe_core::diff::diff_lines` run against `HEAD`'s blob and the
/// file's current *on-disk* content — not the live buffer, so this covers
/// files that aren't even open as a tab, matching `changed_files()` itself
/// scanning the whole working tree rather than just open files.
fn compute_changed_files(repo: Option<&Repo>) -> Vec<ChangesEntry> {
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

/// Re-scans the working tree for `state.changed_files`. Not run on every
/// keystroke — a full `gix` status walk plus a per-file diff isn't cheap —
/// only after actions known to change the working tree, i.e. saving a file.
/// Edits made outside DevScribe won't be reflected until the next save;
/// there's no file-watcher wired up yet (same limitation `tree`/`fs_tree`
/// already has for the file browser).
fn refresh_changed_files(state: &mut State) {
    state.changed_files = compute_changed_files(state.repo.as_ref());
}

/// Naive project-wide search: reads every file the sidebar already walked
/// and scans it. Capped so a broad query on a large project can't stall the
/// UI thread indefinitely — see `devscribe_core::search` for the "start
/// naive, index later if slow" rationale.
const MAX_SEARCH_RESULTS: usize = 200;

fn recompute_search(state: &mut State) {
    state.search_results.clear();
    if state.search_query.is_empty() {
        state.search_elapsed = Duration::ZERO;
        return;
    }
    let query_len_chars = state.search_query.chars().count();
    let started = Instant::now();

    'files: for path in fs_tree::flatten_files(&state.tree) {
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        let hits = search::search_text(&content, &state.search_query);
        if hits.is_empty() {
            continue;
        }

        // Highlight the whole file once per file with results, not once per
        // match — reuses the same `Highlighter` the editor canvas uses.
        let language = path
            .extension()
            .and_then(|ext| ext.to_str())
            .and_then(syntax::Language::from_extension);
        let file_spans = language.map(|lang| {
            let mut highlighter = syntax::Highlighter::new();
            highlighter.highlight(lang, &content)
        });
        let document = Document::from_str(&content);

        for hit in hits {
            let segments = file_spans
                .as_ref()
                .map(|spans| line_segments(&document, spans, hit.line))
                .unwrap_or_default();
            state.search_results.push(SearchResult {
                path: path.to_path_buf(),
                hit,
                segments,
                query_len_chars,
            });
            if state.search_results.len() >= MAX_SEARCH_RESULTS {
                break 'files;
            }
        }
    }

    state.search_elapsed = started.elapsed();
}

/// The syntax-colored runs covering `line`'s full text, in order — the same
/// span-slicing `editor_canvas.rs` does per visible line, reused here for
/// one search-result preview line instead of a whole open buffer.
fn line_segments(document: &Document, spans: &[Span], line: usize) -> Vec<(String, syntax::HighlightKind)> {
    let line_text = document.line_text(line);
    if line_text.is_empty() {
        return Vec::new();
    }
    let line_start_byte = document.text().line_to_byte(line);
    let line_end_byte = line_start_byte + line_text.len();

    let mut out = Vec::new();
    for span in spans {
        if span.end <= line_start_byte || span.start >= line_end_byte {
            continue;
        }
        let seg_start = span.start.max(line_start_byte) - line_start_byte;
        let seg_end = (span.end.min(line_end_byte) - line_start_byte).min(line_text.len());
        if seg_start < seg_end {
            out.push((line_text[seg_start..seg_end].to_string(), span.kind));
        }
    }
    out
}

fn lsp_uri(path: &Path) -> Option<lsp::Url> {
    lsp::Url::from_file_path(path).ok()
}

fn is_lsp_language(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .and_then(lsp::LspLanguage::from_extension)
        .is_some()
}

fn send_did_open_for(state: &mut State, path: &Path) {
    if !is_lsp_language(path) {
        return;
    }
    let Some(uri) = lsp_uri(path) else {
        return;
    };
    let Some(text) = find_editor(state, path).map(|e| e.document.text().to_string()) else {
        return;
    };
    if let Some(sender) = state.lsp_sender.as_mut() {
        let _ = sender.try_send(LspCommand::DidOpen { uri, text });
    }
}

fn send_did_change_for(state: &mut State, path: &Path) {
    if !is_lsp_language(path) {
        return;
    }
    let Some(uri) = lsp_uri(path) else {
        return;
    };
    let Some(text) = find_editor(state, path).map(|e| e.document.text().to_string()) else {
        return;
    };
    if let Some(sender) = state.lsp_sender.as_mut() {
        let _ = sender.try_send(LspCommand::DidChange { uri, text });
    }
}

fn send_did_close(sender: &mut mpsc::Sender<LspCommand>, path: &Path) {
    if !is_lsp_language(path) {
        return;
    }
    if let Some(uri) = lsp_uri(path) {
        let _ = sender.try_send(LspCommand::DidClose { uri });
    }
}

/// Converts `lsp_types::Diagnostic`s (UTF-16 positions) into char-based
/// `EditorDiagnostic`s against the document's *current* text. If edits have
/// raced ahead of a stale diagnostics batch, positions are clamped rather
/// than panicking — see `Document::line_text`.
fn convert_diagnostics(document: &Document, diagnostics: Vec<lsp::Diagnostic>) -> Vec<EditorDiagnostic> {
    diagnostics
        .into_iter()
        .map(|d| {
            let start_line = d.range.start.line as usize;
            let end_line = d.range.end.line as usize;
            let start_col =
                utf16_col_to_char_col(&document.line_text(start_line), d.range.start.character as usize);
            let end_col =
                utf16_col_to_char_col(&document.line_text(end_line), d.range.end.character as usize);
            EditorDiagnostic {
                start: CursorPos { line: start_line, col: start_col },
                end: CursorPos { line: end_line, col: end_col },
                severity: d.severity.unwrap_or(lsp::DiagnosticSeverity::ERROR),
                message: d.message,
            }
        })
        .collect()
}

fn utf16_col_to_char_col(line: &str, utf16_col: usize) -> usize {
    let mut utf16_count = 0usize;
    for (char_idx, ch) in line.chars().enumerate() {
        if utf16_count >= utf16_col {
            return char_idx;
        }
        utf16_count += ch.len_utf16();
    }
    line.chars().count()
}

/// Zero-arg by design: `Subscription::run` needs a plain `fn` pointer (no
/// captures), and DevScribe doesn't support changing the project root at
/// runtime yet, so rediscovering it here (same as `State::default`) is
/// simpler than threading it through as subscription data.
fn lsp_worker() -> impl iced::futures::Stream<Item = LspEvent> {
    let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    iced::stream::channel(32, async move |output| {
        lsp::run(root, output).await;
    })
}

/// Ctrl/Cmd+K (palette), Ctrl/Cmd+S (save), and Escape (close whatever
/// overlay is open) — global shortcuts that work regardless of which widget
/// has focus. `keyboard::listen()` only sees events no focused widget
/// captured, so this never steals a keystroke the editor canvas wants (e.g.
/// typing a literal "k").
fn global_keys(event: keyboard::Event) -> Message {
    if let keyboard::Event::KeyPressed { key, modifiers, .. } = event {
        if modifiers.command()
            && let keyboard::Key::Character(c) = key.as_ref()
        {
            if c.eq_ignore_ascii_case("k") {
                return Message::TogglePalette;
            }
            if c.eq_ignore_ascii_case("s") {
                return Message::EditorSave;
            }
            if c.eq_ignore_ascii_case("f") {
                return if modifiers.shift() {
                    Message::FocusSearchTab
                } else {
                    Message::ToggleFind
                };
            }
            if c.eq_ignore_ascii_case("w") {
                return if modifiers.alt() {
                    Message::CloseOtherTabs
                } else {
                    Message::CloseActiveTab
                };
            }
            if c.eq_ignore_ascii_case("n") {
                return Message::BeginDraft(if modifiers.shift() {
                    DraftKind::NewFolder
                } else {
                    DraftKind::NewFile
                });
            }
            if modifiers.shift() && c.eq_ignore_ascii_case("e") {
                return Message::RevealActiveInTree;
            }
            if modifiers.shift() && c.eq_ignore_ascii_case("t") {
                return Message::ReopenClosedTab;
            }
        }
        if matches!(key, keyboard::Key::Named(keyboard::key::Named::Escape)) {
            return Message::EscapePressed;
        }
        // The editor canvas captures these itself while focused (see
        // `editor_canvas::handle_key`), so these only fire when something
        // else has focus — harmless no-ops unless the palette is open.
        if matches!(key, keyboard::Key::Named(keyboard::key::Named::ArrowUp)) {
            return Message::PaletteMove(-1);
        }
        if matches!(key, keyboard::Key::Named(keyboard::key::Named::ArrowDown)) {
            return Message::PaletteMove(1);
        }
    }
    Message::Noop
}

pub fn subscription(_state: &State) -> iced::Subscription<Message> {
    iced::Subscription::batch([
        iced::time::every(std::time::Duration::from_millis(530)).map(|_| Message::CaretTick),
        iced::time::every(Duration::from_secs(1)).map(|_| Message::PruneToasts),
        iced::Subscription::run(lsp_worker).map(Message::Lsp),
        iced::keyboard::listen().map(global_keys),
    ])
}

#[cfg(test)]
mod tests {
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
    fn line_segments_matches_real_highlighter_output() {
        let source = "fn main() {\n    let x = 1;\n}\n";
        let document = Document::from_str(source);
        let mut highlighter = syntax::Highlighter::new();
        let spans = highlighter.highlight(syntax::Language::Rust, source);

        // Line 1 is `    let x = 1;` — `let` should come back as its own
        // Keyword-colored segment, not merged into the surrounding text.
        let segments = line_segments(&document, &spans, 1);
        let joined: String = segments.iter().map(|(text, _)| text.as_str()).collect();
        assert_eq!(joined, document.line_text(1), "segments must reconstruct the full line with no gaps");

        let let_segment = segments
            .iter()
            .find(|(text, _)| text == "let")
            .expect("`let` should be its own segment");
        assert_eq!(let_segment.1, syntax::HighlightKind::Keyword);
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
}
