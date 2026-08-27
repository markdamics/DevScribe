use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use devscribe_core::diff::DiffLine;
use devscribe_core::git::{ChangeKind, Repo};
use devscribe_core::lsp::{self, CompletionItem, LspCommand, LspEvent, LspLanguage};
use devscribe_core::search::{self, SearchHit};
use devscribe_core::syntax::{self, Span};
use devscribe_core::theme::{Accent, ThemeMode};
use devscribe_core::watcher::{self, WatchEvent};
use devscribe_core::Document;
use iced::futures::channel::mpsc;
use iced::keyboard;
use iced::mouse;

use crate::density::Density;
use crate::fs_tree::{self, Node};
use crate::recent_projects;
use crate::settings;

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
/// (Rust, via `rust-analyzer`). Surfaced in the status bar and the
/// settings panel's Toolchains category.
#[derive(Debug, Clone, Default)]
pub enum LspStatus {
    #[default]
    Starting,
    /// Auto-install is running in the background (`start_server_install`).
    Installing,
    Ready,
    Unavailable(String),
    /// Turned off via the settings panel's "Enable language servers" toggle —
    /// distinct from `Unavailable` (a failed spawn/handshake) so the status
    /// bar doesn't imply something went wrong when the user asked for this.
    Disabled,
}

impl LspStatus {
    /// (status-dot color, label) — the single source of truth both
    /// `status_bar.rs` and `settings_panel.rs`'s Toolchains/About content
    /// render from, so the two never drift apart.
    pub fn describe(&self, server_name: &str, p: devscribe_core::theme::Palette) -> (devscribe_core::theme::Rgba, String) {
        match self {
            LspStatus::Starting => (p.text_muted, format!("{server_name} starting\u{2026}")),
            LspStatus::Installing => (p.text_muted, format!("installing {server_name}\u{2026}")),
            LspStatus::Ready => (p.status_success, format!("{server_name} ready")),
            LspStatus::Unavailable(reason) => (p.status_warning, format!("{server_name} unavailable ({reason})")),
            LspStatus::Disabled => (p.text_muted, format!("{server_name} disabled")),
        }
    }
}

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
    query: String,
    results: Vec<SearchResult>,
    elapsed: Duration,
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
    /// `true` once "Delete" has been clicked once — the menu then shows a
    /// confirm/cancel step instead of the normal rows, rather than
    /// deleting on the first click. Deleting a file/directory here is
    /// permanent (no OS trash/recycle-bin integration), so this is the one
    /// destructive action in the whole app that needs a deliberate second
    /// step before it touches disk.
    pub confirm_delete: bool,
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
    Shortcuts,
    About,
}

impl SettingsCategory {
    pub const ALL: [SettingsCategory; 5] = [
        SettingsCategory::Explorer,
        SettingsCategory::Editor,
        SettingsCategory::Toolchains,
        SettingsCategory::Shortcuts,
        SettingsCategory::About,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SettingsCategory::Explorer => "Explorer",
            SettingsCategory::Editor => "Editor",
            SettingsCategory::Toolchains => "Toolchains",
            SettingsCategory::Shortcuts => "Shortcuts",
            SettingsCategory::About => "About",
        }
    }
}

/// A runnable entry in the command palette: file to open, theme to switch
/// to, action to run.
#[derive(Debug, Clone)]
pub enum PaletteAction {
    OpenFile(PathBuf),
    SetThemeMode(ThemeMode),
    SetAccent(Accent),
    FocusSearchTab,
    /// Opens (or focuses) a diff tab for the currently active file tab.
    ViewDiffOfActiveFile,
    /// Opens (or focuses) a diff tab for "the" working-tree change: the
    /// active file if one's open, else the first entry in `changed_files`.
    /// Distinct from `ViewDiffOfActiveFile` — this one's reachable with
    /// nothing open at all, as long as *something* in the tree has changed.
    ViewWorkingTreeDiff,
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
    /// Opens a blank tab with no file on disk yet ("Untitled-1", ...) — see
    /// `begin_untitled_buffer`. Command-palette only: `⌘N`/`⇧⌘N` are
    /// already the sidebar-draft new-file/new-folder shortcuts, and this is
    /// a deliberately different thing (Phase 5's "New file" writes straight
    /// to disk; this doesn't, until Save As gives it a real location).
    NewUntitledFile,
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
    /// Vertical scroll offset (px from the top) of this tab's editor
    /// canvas, last reported by the `scrollable`'s `on_scroll`. Used only
    /// to virtualize `EditorCanvas::draw` (skip lines outside the visible
    /// range) — not meaningful outside that.
    pub scroll_offset: f32,
    /// Active completion popup: `None` = closed, `Some(items)` = showing.
    pub completions: Option<Vec<CompletionItem>>,
    /// Keyboard-navigation index into `completions`.
    pub completion_selected: usize,
    /// Cursor position when the completion request was sent, used to discard
    /// stale responses that arrive after the cursor moved elsewhere.
    pub completion_anchor: CursorPos,
    /// Undo history — snapshots taken just before an edit that starts a new
    /// undo step (see `record_undo_boundary`). Consecutive same-kind edits
    /// (typing character after character, backspacing run after run)
    /// coalesce into whatever's already on top instead of pushing a new
    /// entry, so `Ctrl+Z` undoes a whole word/paste/deletion at a time
    /// rather than one keystroke.
    undo_stack: Vec<UndoSnapshot>,
    /// Snapshots popped off `undo_stack` by `undo()`, replayed by `redo()`.
    /// Cleared on every new edit, same as any other editor's redo stack.
    redo_stack: Vec<UndoSnapshot>,
    /// The kind of the most recent edit, for `record_undo_boundary`'s
    /// same-kind coalescing. Reset to `None` by any cursor move or
    /// mouse-driven selection change that isn't itself an edit, so typing
    /// never coalesces across an unrelated click or arrow-key jump.
    last_edit_kind: Option<EditKind>,
}

/// A point in `EditorState::undo_stack`/`redo_stack` — the whole buffer plus
/// enough cursor state to put the user back where they were. Cloning
/// `Document` clones its `Rope`, which is cheap (structural sharing), so
/// this is fine to snapshot on every undo boundary rather than diffing.
#[derive(Clone)]
struct UndoSnapshot {
    document: Document,
    cursor: CursorPos,
    selection_anchor: Option<CursorPos>,
}

/// What kind of edit just happened, for `record_undo_boundary`'s
/// same-kind-coalesces-into-one-step logic. `Other` never coalesces, even
/// with itself — each paste/cut is its own undo step.
#[derive(PartialEq, Clone, Copy)]
enum EditKind {
    Insert,
    Delete,
    Other,
}

/// Undo history is capped at this many steps per editor (each holding a full
/// document snapshot) so an very long editing session can't grow it without
/// bound.
const MAX_UNDO_ENTRIES: usize = 500;

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
            scroll_offset: 0.0,
            completions: None,
            completion_selected: 0,
            completion_anchor: CursorPos::default(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            last_edit_kind: None,
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
            // Same unbounded-memory risk `recompute_search` guards against
            // (every match clones its whole line) — capped here too, even
            // though a single already-open buffer is a smaller blast radius
            // than the whole project.
            search::search_text(&text, &query, MAX_SEARCH_RESULTS)
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

    /// Pushes a fresh undo snapshot unless this edit is the same `kind` as
    /// the one right before it (typing coalescing into one word, backspaces
    /// coalescing into one run) — always call this *before* mutating
    /// `document`. `Other` never coalesces, so paste/cut are always their
    /// own undo step regardless of what happened right before them.
    fn record_undo_boundary(&mut self, kind: EditKind) {
        let coalesce = kind != EditKind::Other && self.last_edit_kind == Some(kind);
        if !coalesce {
            self.undo_stack.push(UndoSnapshot {
                document: self.document.clone(),
                cursor: self.cursor,
                selection_anchor: self.selection_anchor,
            });
            if self.undo_stack.len() > MAX_UNDO_ENTRIES {
                self.undo_stack.remove(0);
            }
        }
        self.last_edit_kind = Some(kind);
        self.redo_stack.clear();
    }

    pub fn insert_text(&mut self, text: &str) {
        // A lone, non-newline character is ordinary typing and coalesces
        // with adjacent typing into one undo step; anything else (paste,
        // Tab's 4 spaces, Enter's "\n") always starts a fresh one.
        let kind = if text.chars().count() == 1 && text != "\n" {
            EditKind::Insert
        } else {
            EditKind::Other
        };
        self.record_undo_boundary(kind);
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
        self.record_undo_boundary(EditKind::Delete);
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
        self.record_undo_boundary(EditKind::Delete);
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

    /// `Ctrl+Z`. `false` if there was nothing left to undo.
    pub fn undo(&mut self) -> bool {
        let Some(prev) = self.undo_stack.pop() else {
            return false;
        };
        self.redo_stack.push(UndoSnapshot {
            document: self.document.clone(),
            cursor: self.cursor,
            selection_anchor: self.selection_anchor,
        });
        self.document = prev.document;
        self.cursor = prev.cursor;
        self.selection_anchor = prev.selection_anchor;
        self.last_edit_kind = None;
        self.rehighlight();
        self.reparse_json();
        self.refind();
        true
    }

    /// `Ctrl+Shift+Z` / `Ctrl+Y`. `false` if there was nothing left to redo.
    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo_stack.pop() else {
            return false;
        };
        self.undo_stack.push(UndoSnapshot {
            document: self.document.clone(),
            cursor: self.cursor,
            selection_anchor: self.selection_anchor,
        });
        self.document = next.document;
        self.cursor = next.cursor;
        self.selection_anchor = next.selection_anchor;
        self.last_edit_kind = None;
        self.rehighlight();
        self.reparse_json();
        self.refind();
        true
    }

    pub fn move_cursor(&mut self, dir: Direction, extend: bool) {
        self.last_edit_kind = None;
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
        self.last_edit_kind = None;
        if extend {
            if self.selection_anchor.is_none() {
                self.selection_anchor = Some(self.cursor);
            }
        } else {
            self.selection_anchor = None;
        }
        self.cursor = CursorPos { line, col };
    }

    /// Double-click word selection: expands `col` to cover the run of
    /// same-class characters it falls on (word chars, whitespace, or other
    /// punctuation each form their own run), matching the usual editor
    /// convention of selecting "the thing under the cursor" regardless of
    /// what kind of thing that is.
    pub fn select_word_at(&mut self, line: usize, col: usize) {
        self.last_edit_kind = None;
        let text = self.document.line_text(line);
        let chars: Vec<char> = text.chars().collect();
        if chars.is_empty() {
            self.selection_anchor = None;
            self.cursor = CursorPos { line, col: 0 };
            return;
        }
        let idx = col.min(chars.len() - 1);
        let class = char_class(chars[idx]);
        let mut start = idx;
        while start > 0 && char_class(chars[start - 1]) == class {
            start -= 1;
        }
        let mut end = idx + 1;
        while end < chars.len() && char_class(chars[end]) == class {
            end += 1;
        }
        self.selection_anchor = Some(CursorPos { line, col: start });
        self.cursor = CursorPos { line, col: end };
    }

    /// Triple-click line selection: the whole line including its trailing
    /// newline (so it visibly covers the line like `select_word_at` covers a
    /// word), except on the file's last line, which has no newline to take.
    pub fn select_line_at(&mut self, line: usize) {
        self.last_edit_kind = None;
        self.selection_anchor = Some(CursorPos { line, col: 0 });
        self.cursor = if line + 1 < self.document.line_count() {
            CursorPos { line: line + 1, col: 0 }
        } else {
            CursorPos { line, col: self.document.line_len_chars(line) }
        };
    }

    pub fn select_all(&mut self) {
        self.last_edit_kind = None;
        let total_chars = self.document.text().len_chars();
        self.selection_anchor = Some(CursorPos { line: 0, col: 0 });
        self.cursor = self.document.line_col(total_chars).into();
    }

    /// The currently selected text, if any — `Ctrl+C`'s payload.
    pub fn selected_text(&self) -> Option<String> {
        self.selection()
            .map(|(start, end)| self.document.text().slice(start..end).to_string())
    }

    /// `Ctrl+X`: returns the selected text (like `selected_text`) and also
    /// removes it, or `None` (leaving the document untouched) if there was
    /// no selection.
    pub fn cut_selection(&mut self) -> Option<String> {
        let text = self.selected_text();
        if text.is_some() {
            self.record_undo_boundary(EditKind::Other);
            self.delete_selection();
            self.rehighlight();
            self.reparse_json();
            self.refind();
        }
        text
    }
}

#[derive(PartialEq, Eq)]
enum CharClass {
    Word,
    Space,
    Other,
}

fn char_class(c: char) -> CharClass {
    if c.is_alphanumeric() || c == '_' {
        CharClass::Word
    } else if c.is_whitespace() {
        CharClass::Space
    } else {
        CharClass::Other
    }
}

/// Sidebar width bounds for the drag handle — narrow enough to still show
/// file names, wide enough to leave room for the editor.
pub const SIDEBAR_MIN_WIDTH: f32 = 180.0;
pub const SIDEBAR_MAX_WIDTH: f32 = 560.0;
pub const SIDEBAR_DEFAULT_WIDTH: f32 = 272.0;

pub struct State {
    pub theme_mode: ThemeMode,
    pub accent: Accent,
    /// Every currently open tab, in the order they appear in the tab bar.
    pub open_tabs: Vec<OpenTab>,
    /// `None` only when `open_tabs` is empty.
    pub active_tab: Option<TabKey>,
    pub assist_on: bool,
    pub projects_open: bool,
    pub overflow_open: bool,
    /// `true` collapses the sidebar to a narrow icon rail (project glyph +
    /// Settings + an "expand" button), matching the mockup's `panel-left-close`/
    /// `panel-left-open` toggle. Independent of `welcome_open` — this only
    /// matters once a project is open at all.
    pub sidebar_collapsed: bool,
    /// Sidebar width in logical pixels, dragged via the resize handle on its
    /// right edge — see `Message::SidebarResizeDragged`. Clamped to
    /// `[SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH]`.
    pub sidebar_width: f32,
    /// `true` while the sidebar's resize handle is being dragged — the
    /// window-wide mouse subscription (see `subscription`) only runs while
    /// this is set, so idle frames don't pay for a global cursor listener.
    pub sidebar_resizing: bool,
    /// `true` while no project is open and the welcome screen ("Select a
    /// project") is showing full-window in place of the whole editor —
    /// `shell::view()` short-circuits to `welcome::view` before touching
    /// `title_bar`/`sidebar`/the editor, so `root`/`tree`/etc. below are
    /// meaningless placeholders whenever this is `true`.
    pub welcome_open: bool,
    /// Persisted "recently opened projects," most-recent-first. Loaded once
    /// at startup (`recent_projects::load`) and updated via
    /// `recent_projects::touch` every time a project successfully loads.
    pub recent_projects: Vec<recent_projects::RecentProject>,
    /// Live display data for `recent_projects` — recomputed by
    /// `compute_welcome_rows` whenever it changes, not on every `view()`
    /// (each row costs a `Repo::open` + status scan). Feeds both the
    /// welcome screen's list and the sidebar's projects dropdown.
    pub welcome_rows: Vec<WelcomeRow>,
    /// `Some` only while a background project load (`start_loading_project`)
    /// is in flight — drives the welcome screen's "Loading workspace"
    /// overlay.
    pub loading_project: Option<LoadingProject>,
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
    /// Off switches the LSP subscription off entirely (see `subscription`)
    /// rather than just ignoring its output — no language server process
    /// gets spawned at all while this is `false`. On by default.
    pub lsp_enabled: bool,
    /// Incremented after a successful auto-install to force the subscription
    /// to tear down and respawn `lsp_worker` with the newly installed binary.
    /// Without this, the subscription key `(root, language)` hasn't changed
    /// so iced wouldn't restart the worker automatically.
    lsp_restart_token: u64,
    /// `None` when `root` isn't a git repository — an expected, common case.
    pub repo: Option<Repo>,
    /// The sidebar's "CHANGES" panel contents — every file that differs from
    /// `HEAD`, across the whole project (not just open tabs). Empty when
    /// `repo` is `None`. See `refresh_changed_files` for when this updates.
    pub changed_files: Vec<ChangesEntry>,
    /// `(ahead, behind)` commit counts vs. the current branch's upstream —
    /// the sidebar's `▲2 ▼0` indicator. `None` when `repo` is `None` *or*
    /// there's simply no upstream configured (no remote, or the branch
    /// isn't tracking one) — see `Repo::ahead_behind`. Refreshed alongside
    /// `changed_files`.
    pub ahead_behind: Option<(usize, usize)>,
    pub changes_panel_open: bool,
    /// The live text in the search box — may be ahead of `search_last_query`
    /// while the user is still typing. Search is debounced (see
    /// `search_query_changed_at`) rather than run on every keystroke or
    /// gated on Enter: it starts automatically a short pause after typing
    /// stops, the same shape VSCode's search-as-you-type uses.
    pub search_query: String,
    /// When the search box was last edited, so `Message::SearchDebounceTick`
    /// can tell "the debounce window has elapsed" from "still typing" —
    /// `None` once a search has actually started for the current text (no
    /// point re-checking every tick after that).
    search_query_changed_at: Option<Instant>,
    /// `true` while a background search is in flight — drives the
    /// "Searching…" state in `search_view.rs`.
    pub search_in_progress: bool,
    /// Cancels the in-flight background search, if any, the moment a newer
    /// one starts (`start_search`) — see its doc for what "cancel" can and
    /// can't actually guarantee here.
    search_task_handle: Option<iced::task::Handle>,
    /// The query `search_results` actually reflects, i.e. the query as of
    /// the last completed search. Compared against `search_query` in
    /// `search_view.rs` so a not-yet-searched edit doesn't show stale
    /// results, or a misleading "No matches for {query}" for a query that
    /// was never actually searched.
    pub search_last_query: String,
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
    /// When on, every dirty open file is saved as soon as the app window
    /// loses focus (see `Message::WindowUnfocused`). Off by default —
    /// unlike the other toggles in this struct, this one silently writes to
    /// disk, so it shouldn't turn on a new behavior nobody asked for.
    pub save_on_focus_loss: bool,
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
    /// Monotonic counter for naming untitled buffers ("Untitled-1",
    /// "Untitled-2", ...) — see `begin_untitled_buffer`. Never reused, even
    /// after the tab closes, so two buffers can never end up with the same
    /// name/identity.
    next_untitled_id: u64,
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

/// A project root's derived data — tree, collapsed dirs, and git summary —
/// everything `snapshot_project` computes. `Repo` itself is deliberately
/// not part of this: it isn't `Clone` (wraps a `gix::Repository`), and this
/// gets sent across a background thread as a `Message` payload
/// (`start_loading_project`), which requires `Clone` project-wide. Wherever
/// a snapshot is applied to `State`, `Repo::open` (cheap — just opens
/// refs/HEAD, not a status walk) is called again synchronously alongside it.
#[derive(Debug, Clone)]
struct ProjectSnapshot {
    tree: Vec<Node>,
    collapsed_dirs: HashSet<PathBuf>,
    changed_files: Vec<ChangesEntry>,
    ahead_behind: Option<(usize, usize)>,
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
fn snapshot_project(root: &Path) -> ProjectSnapshot {
    let tree = fs_tree::walk(root);
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
const MAX_WELCOME_ROWS: usize = 8;

/// Computes live display data for up to `MAX_WELCOME_ROWS` of `recent`,
/// via a transient `Repo::open` per entry (not the same handle as
/// `State::repo` — this never touches the currently open project's repo).
/// Called whenever `recent_projects` changes, not on every `view()` (see
/// `WelcomeRow`'s doc).
fn compute_welcome_rows(recent: &[recent_projects::RecentProject]) -> Vec<WelcomeRow> {
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

fn empty_project_snapshot() -> ProjectSnapshot {
    ProjectSnapshot { tree: Vec::new(), collapsed_dirs: HashSet::new(), changed_files: Vec::new(), ahead_behind: None }
}

/// Everything `Default::default()` needs to seed a startup `State`: whether
/// the welcome screen should show, and (if not) the project it auto-reopened.
struct Startup {
    welcome_open: bool,
    root: PathBuf,
    snapshot: ProjectSnapshot,
    repo: Option<Repo>,
    recent_projects: Vec<recent_projects::RecentProject>,
    welcome_rows: Vec<WelcomeRow>,
}

/// Loads the real persisted recent-projects list and auto-reopens the most
/// recently used one that still exists on disk (VSCode-style) — skipping
/// any stale entries for projects since moved or deleted, rather than just
/// failing on the very first one. First run (nothing recorded yet) or
/// every recorded path having vanished both fall through to the welcome
/// screen, same as explicitly closing a project.
#[cfg(not(test))]
fn startup() -> Startup {
    let mut recent_projects = recent_projects::load();
    let reopen = recent_projects.iter().find(|p| p.path.is_dir()).map(|p| p.path.clone());

    match reopen {
        Some(root) => {
            recent_projects::touch(&mut recent_projects, root.clone());
            let snapshot = snapshot_project(&root);
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
fn startup() -> Startup {
    Startup {
        welcome_open: true,
        root: PathBuf::new(),
        snapshot: empty_project_snapshot(),
        repo: None,
        recent_projects: Vec::new(),
        welcome_rows: Vec::new(),
    }
}

/// The persisted settings (`settings::save`, written by `persist_settings`
/// on every settings-changing message), or defaults for whatever wasn't
/// there yet. Global, not project-scoped, so this is independent of
/// `startup()`/`Startup` above.
#[cfg(not(test))]
fn startup_settings() -> settings::Settings {
    settings::load()
}

/// Never reads the real `~/.config/devscribe/settings.json` — same reason
/// `startup()`'s test build never reads `recent_projects.json`: every test
/// that constructs `State::default()` would otherwise depend on whatever
/// settings this machine's user last picked in the real app.
#[cfg(test)]
fn startup_settings() -> settings::Settings {
    settings::Settings::default()
}

impl Default for State {
    fn default() -> Self {
        let Startup { welcome_open, root, snapshot, repo, recent_projects, welcome_rows } = startup();
        let settings = startup_settings();

        Self {
            theme_mode: settings.theme_mode,
            accent: settings.accent,
            open_tabs: Vec::new(),
            active_tab: None,
            assist_on: true,
            projects_open: false,
            overflow_open: false,
            sidebar_collapsed: false,
            sidebar_width: SIDEBAR_DEFAULT_WIDTH,
            sidebar_resizing: false,
            welcome_open,
            recent_projects,
            welcome_rows,
            loading_project: None,
            root,
            tree: snapshot.tree,
            collapsed_dirs: snapshot.collapsed_dirs,
            caret_visible: true,
            lsp_status: LspStatus::default(),
            lsp_sender: None,
            lsp_enabled: settings.lsp_enabled,
            lsp_restart_token: 0,
            repo,
            changed_files: snapshot.changed_files,
            ahead_behind: snapshot.ahead_behind,
            changes_panel_open: false,
            search_query: String::new(),
            search_query_changed_at: None,
            search_in_progress: false,
            search_task_handle: None,
            search_last_query: String::new(),
            search_results: Vec::new(),
            search_elapsed: Duration::ZERO,
            palette_open: false,
            palette_query: String::new(),
            palette_selected: 0,
            settings_open: false,
            settings_category: SettingsCategory::default(),
            git_status_in_tree: settings.git_status_in_tree,
            save_on_focus_loss: settings.save_on_focus_loss,
            density: settings.density,
            problem_lens_enabled: settings.problem_lens_enabled,
            editor_font_size: settings.editor_font_size,
            ui_font_scale: settings.ui_font_scale,
            toasts: Vec::new(),
            next_toast_id: 0,
            draft: None,
            ctx_menu: None,
            flash: None,
            closed_tabs: Vec::new(),
            next_untitled_id: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    SetThemeMode(ThemeMode),
    SetAccent(Accent),
    SelectOpenTab(TabKey),
    CloseTab(TabKey),
    CloseActiveTab,
    FocusSearchTab,
    ToggleAssist,
    ToggleProjects,
    ToggleOverflow,
    CollapseSidebar,
    ExpandSidebar,
    /// Pressed the sidebar's edge resize handle.
    SidebarResizeStarted,
    /// Cursor moved while resizing — carries the cursor's window-space X
    /// position, which becomes the new sidebar width directly (the handle
    /// sits flush against the sidebar's right edge at X == 0).
    SidebarResizeDragged(f32),
    SidebarResizeEnded,
    ToggleChangesPanel,
    /// Opens (or focuses) a diff tab for `path`, from a sidebar Changes row.
    /// Distinct from `PaletteAction::ViewDiffOfActiveFile`, which only ever
    /// targets the currently active tab.
    OpenDiffFor(PathBuf),
    /// Global `⇧⌘D` — see `PaletteAction::ViewWorkingTreeDiff`, which this
    /// shares a handler with.
    ViewWorkingTreeDiff,
    SelectFile(PathBuf),
    EditorInsertText(String),
    EditorBackspace,
    EditorDelete,
    EditorMove { dir: Direction, extend: bool },
    /// A left-button press or drag-move over the canvas — also the plain
    /// (non-double/triple) click path. `extend: true` both for shift-click
    /// and for every drag-move after the initial press, so a mouse drag
    /// just keeps calling `EditorState::click`, which is exactly
    /// `extend`'s existing shift-click behavior (see `EditorCanvas::update`).
    EditorClick { line: usize, col: usize, extend: bool },
    /// Double-click — selects the word (or punctuation/whitespace run) under
    /// the click.
    EditorSelectWord { line: usize, col: usize },
    /// Triple-click — selects the whole line.
    EditorSelectLine { line: usize },
    /// `Ctrl+A`.
    EditorSelectAll,
    /// `Ctrl+Z`.
    EditorUndo,
    /// `Ctrl+Shift+Z` / `Ctrl+Y`.
    EditorRedo,
    /// `Ctrl+C`.
    EditorCopy,
    /// `Ctrl+X`.
    EditorCut,
    /// `Ctrl+V` — kicks off an async clipboard read; the actual insert
    /// happens in `EditorPasteWithText` once it resolves.
    EditorPaste,
    EditorPasteWithText(Option<String>),
    /// The editor's `scrollable` reported a new vertical offset — stored so
    /// `EditorCanvas::draw` can skip lines outside the visible range.
    EditorScrolled(f32),
    CaretTick,
    Lsp(LspEvent),
    /// A debounced batch of on-disk changes from `file_watcher` — an edit
    /// made outside DevScribe (another terminal, `git checkout`, and later
    /// the AI Chat Assist panel's own file edits).
    FilesChanged(Vec<WatchEvent>),
    JsonToggle(String),
    ToggleDirCollapsed(PathBuf),
    SearchQueryChanged(String),
    /// Enter in the search box — starts a search immediately, bypassing
    /// the debounce wait.
    SearchSubmit,
    /// A recurring low-frequency tick (see `subscription`) checking whether
    /// enough time has passed since the last `SearchQueryChanged` to start
    /// searching automatically.
    SearchDebounceTick,
    /// A background search (`start_search`) finished — applied only if
    /// its query still matches `state.search_query`; see `SearchOutcome`.
    SearchCompleted(SearchOutcome),
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
    ToggleSaveOnFocusLoss,
    /// Turns the `rust-analyzer` subscription on/off (see `subscription`).
    /// Switching off drops the running worker (killing its child process,
    /// `kill_on_drop`) and clears every open editor's diagnostics; switching
    /// back on spawns a fresh one for the current project root.
    ToggleLspEnabled,
    /// The window lost focus — saves every dirty open file if
    /// `save_on_focus_loss` is on, else a no-op. Fired from
    /// `iced::window::events()`, not a keybinding.
    WindowUnfocused,
    /// Global `⌘/` — opens Settings straight to the Shortcuts category, in
    /// one step rather than `ToggleSettings` + `SetSettingsCategory`.
    OpenShortcutsHelp,
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
    /// First click on the context menu's "Delete" row — shows the
    /// confirm/cancel step (`ContextMenu::confirm_delete`) rather than
    /// deleting immediately.
    PromptDeletePath,
    /// The confirm step's "Delete" button — actually removes `path` (file
    /// or, recursively, directory) from disk. Permanent: no OS trash/
    /// recycle-bin integration.
    DeletePath(PathBuf),
    CloseOtherTabs,
    RevealActiveInTree,
    ReopenClosedTab,
    /// "Open folder…" — welcome screen or the sidebar projects dropdown.
    OpenFolderDialog,
    /// "New project" — same picker as `OpenFolderDialog`, but the result
    /// gets `git init`ed first if it isn't already a repo (see
    /// `FolderDialogResult`'s second field).
    NewProjectDialog,
    /// The async folder-picker dialog finished. `None` means the user
    /// cancelled it. The `bool` is `OpenFolderDialog` (`false`) vs.
    /// `NewProjectDialog` (`true`) — whether to `git init` the result.
    FolderDialogResult(Option<PathBuf>, bool),
    /// A row in the welcome screen's recent list or the sidebar's projects
    /// dropdown was clicked.
    RecentProjectPicked(PathBuf),
    /// "Close project" — returns to the welcome screen.
    CloseProject,
    /// A background project load (`start_loading_project`) finished.
    /// Boxed since `LoadedProject` carries a whole `ProjectSnapshot`
    /// (file tree included), and `Message` is `Clone` project-wide.
    ProjectLoaded(Box<LoadedProject>),
    /// The "Save As" dialog (`save_file_as`) finished for the tab currently
    /// keyed by the first `PathBuf` (its old, possibly-synthetic-untitled
    /// path) — `None` if the user cancelled, `Some(new_path)` otherwise.
    SaveAsResult(PathBuf, Option<PathBuf>),
    /// A background server install (`start_server_install`) finished.
    /// `Ok(())` = installed successfully; `Err(msg)` = failed with reason.
    ServerInstallComplete(Result<(), String>),
    /// Arrow-key navigation inside the completion popup: +1 down, -1 up.
    CompletionMove(i32),
    /// Tab/Enter while the completion popup is open — inserts the selected item.
    CompletionSelect,
    /// Closes the completion popup without inserting.
    CloseCompletion,
    Noop,
}

pub fn update(state: &mut State, message: Message) -> iced::Task<Message> {
    match message {
        Message::SetThemeMode(mode) => set_theme_mode(state, mode),
        Message::SetAccent(accent) => set_accent(state, accent),
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
        Message::CollapseSidebar => {
            state.sidebar_collapsed = true;
            // Closing menus that were anchored to sidebar content about to
            // disappear, same as the mockup's own `collapseSidebar` handler.
            state.projects_open = false;
            state.ctx_menu = None;
        }
        Message::ExpandSidebar => state.sidebar_collapsed = false,
        Message::SidebarResizeStarted => state.sidebar_resizing = true,
        Message::SidebarResizeDragged(x) => {
            if state.sidebar_resizing {
                state.sidebar_width = x.clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH);
            }
        }
        Message::SidebarResizeEnded => state.sidebar_resizing = false,
        Message::ToggleChangesPanel => state.changes_panel_open = !state.changes_panel_open,
        Message::OpenDiffFor(path) => open_or_focus_diff(state, path),
        Message::ViewWorkingTreeDiff => view_working_tree_diff(state),
        Message::SelectFile(path) => open_or_focus_file(state, path),
        Message::EditorInsertText(text) => {
            if let Some(path) = active_file_path(state) {
                // Intercept Enter ("\n") and Tab ("    ") when the completion
                // popup is open — select the highlighted item instead of
                // inserting the literal text.
                let completions_open = find_editor(state, &path)
                    .is_some_and(|e| e.completions.is_some());
                if completions_open && (text == "\n" || text == "    ") {
                    return update(state, Message::CompletionSelect);
                }

                if let Some(editor) = find_editor_mut(state, &path) {
                    // Any non-trigger keystroke closes a stale popup.
                    if editor.completions.is_some() && text != "." && text != ":" {
                        editor.completions = None;
                    }
                    editor.insert_text(&text);
                }
                send_did_change_for(state, &path);
                recompute_diff_for(state, &path);

                // Trigger completion on '.' or second ':' of '::'
                if matches!(state.lsp_status, LspStatus::Ready) && (text == "." || text == ":") {
                    let trigger_info = find_editor(state, &path).and_then(|editor| {
                        let cursor = editor.cursor;
                        let line_text = editor.document.line_text(cursor.line);
                        let should_trigger = if text == ":" {
                            line_text.ends_with("::")
                        } else {
                            true
                        };
                        if should_trigger {
                            let utf16_char = char_col_to_utf16_col(&line_text, cursor.col);
                            Some((cursor, utf16_char))
                        } else {
                            None
                        }
                    });
                    if let Some((cursor, utf16_char)) = trigger_info {
                        if let Some(uri) = lsp_uri(&path) {
                            if let Some(sender) = state.lsp_sender.as_mut() {
                                let _ = sender.try_send(LspCommand::Completion {
                                    uri,
                                    line: cursor.line as u32,
                                    character: utf16_char,
                                });
                            }
                            if let Some(editor) = find_editor_mut(state, &path) {
                                editor.completion_anchor = cursor;
                            }
                        }
                    }
                }
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
                // Up/Down navigate the completion popup when it's open and
                // the user hasn't started a selection-extend (shift+arrow).
                if editor.completions.is_some() && !extend {
                    match dir {
                        Direction::Up => {
                            editor.completion_selected =
                                editor.completion_selected.saturating_sub(1);
                            return iced::Task::none();
                        }
                        Direction::Down => {
                            let max = editor
                                .completions
                                .as_ref()
                                .map_or(0, |v| v.len().saturating_sub(1));
                            editor.completion_selected =
                                (editor.completion_selected + 1).min(max);
                            return iced::Task::none();
                        }
                        _ => {
                            // Any other cursor movement dismisses the popup.
                            editor.completions = None;
                        }
                    }
                }
                editor.move_cursor(dir, extend);
            }
        }
        Message::EditorClick { line, col, extend } => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
            {
                editor.completions = None;
                editor.click(line, col, extend);
            }
        }
        Message::EditorSelectWord { line, col } => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
            {
                editor.completions = None;
                editor.select_word_at(line, col);
            }
        }
        Message::EditorSelectLine { line } => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
            {
                editor.completions = None;
                editor.select_line_at(line);
            }
        }
        Message::EditorSelectAll => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
            {
                editor.completions = None;
                editor.select_all();
            }
        }
        Message::EditorCopy => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor(state, &path)
                && let Some(text) = editor.selected_text()
            {
                return iced::clipboard::write(text);
            }
        }
        Message::EditorCut => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
                && let Some(text) = editor.cut_selection()
            {
                send_did_change_for(state, &path);
                recompute_diff_for(state, &path);
                return iced::clipboard::write(text);
            }
        }
        Message::EditorUndo => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
                && editor.undo()
            {
                send_did_change_for(state, &path);
                recompute_diff_for(state, &path);
            }
        }
        Message::EditorRedo => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
                && editor.redo()
            {
                send_did_change_for(state, &path);
                recompute_diff_for(state, &path);
            }
        }
        Message::EditorPaste => return iced::clipboard::read().map(Message::EditorPasteWithText),
        Message::EditorPasteWithText(text) => {
            if let Some(text) = text
                && let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
            {
                editor.completions = None;
                editor.insert_text(&text);
                send_did_change_for(state, &path);
                recompute_diff_for(state, &path);
            }
        }
        Message::EditorScrolled(offset) => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
            {
                editor.scroll_offset = offset;
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
                    let name = active_server_name(state);
                    push_toast(state, ToastKind::Success, format!("{name} ready"));
                }
            }
            LspEvent::Diagnostics { uri, diagnostics } => {
                if let Some(path) = uri.to_file_path().ok()
                    && let Some(editor) = find_editor_mut(state, &path)
                {
                    editor.diagnostics = convert_diagnostics(&editor.document, diagnostics);
                }
            }
            LspEvent::Completions { uri, line, character, items } => {
                if let Some(path) = uri.to_file_path().ok()
                    && let Some(editor) = find_editor_mut(state, &path)
                {
                    // Discard stale results: only apply if the cursor hasn't
                    // moved since the request was sent.
                    let anchor = editor.completion_anchor;
                    if anchor.line == line as usize {
                        let line_text = editor.document.line_text(anchor.line);
                        let anchor_utf16 = char_col_to_utf16_col(&line_text, anchor.col);
                        if anchor_utf16 == character {
                            editor.completions = if items.is_empty() { None } else { Some(items) };
                            editor.completion_selected = 0;
                        }
                    }
                }
            }
            LspEvent::NeedsInstall => {
                // Binary not on PATH and not in the managed dir — kick off
                // a background install and show progress in the status bar.
                if !matches!(state.lsp_status, LspStatus::Installing) {
                    if let Some(lang) = active_lsp_language(state) {
                        state.lsp_status = LspStatus::Installing;
                        return start_server_install(lang);
                    }
                }
            }
            LspEvent::Unavailable(reason) => {
                state.lsp_status = LspStatus::Unavailable(reason.clone());
                state.lsp_sender = None;
                let name = active_server_name(state);
                push_toast(state, ToastKind::Warning, format!("{name} unavailable: {reason}"));
            }
        },
        Message::FilesChanged(events) => {
            refresh_tree(state);
            refresh_changed_files(state);
            for event in &events {
                if let WatchEvent::Changed(path) | WatchEvent::Created(path) = event {
                    reload_editor_from_disk(state, path);
                }
            }
        }
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
        // Debounced, like VSCode's search-as-you-type: typing just records
        // *when* (`search_query_changed_at`), not a search itself — the
        // recurring `SearchDebounceTick` starts one once typing pauses.
        // Never live-per-keystroke: `run_search` walks and reads every
        // matching file in the project, and even off the UI thread (see
        // `start_search`), starting that fresh on every single keystroke
        // is wasted, cancelled-a-moment-later work.
        Message::SearchQueryChanged(query) => {
            state.search_query = query;
            if state.search_query.is_empty() {
                // Nothing to debounce — clear immediately rather than
                // waiting a tick to notice there's nothing to search.
                state.search_query_changed_at = None;
                state.search_results.clear();
                state.search_last_query.clear();
                state.search_elapsed = Duration::ZERO;
                state.search_in_progress = false;
                if let Some(handle) = state.search_task_handle.take() {
                    handle.abort();
                }
            } else {
                state.search_query_changed_at = Some(Instant::now());
            }
        }
        Message::SearchSubmit => {
            focus_search(state);
            return start_search(state);
        }
        Message::SearchDebounceTick => {
            let due = state.search_query_changed_at.is_some_and(|at| at.elapsed() >= SEARCH_DEBOUNCE_DELAY);
            if due {
                state.search_query_changed_at = None;
                return start_search(state);
            }
        }
        Message::SearchCompleted(outcome) => {
            // A search superseded by further typing before it finished
            // still runs to completion in the background (see
            // `start_search`'s doc on what "cancel" can guarantee here) —
            // this is what actually discards its result. `search_in_progress`/
            // `search_task_handle` are deliberately untouched when stale:
            // they belong to whatever search is *actually* still running.
            if outcome.query == state.search_query {
                state.search_results = outcome.results;
                state.search_last_query = outcome.query;
                state.search_elapsed = outcome.elapsed;
                state.search_in_progress = false;
                state.search_task_handle = None;
            }
        }
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
            state.palette_open = false;
            if let Some(entry) = filtered_palette_entries(state).get(state.palette_selected) {
                return run_palette_action(state, entry.action.clone());
            }
        }
        Message::PaletteRun(action) => {
            state.palette_open = false;
            return run_palette_action(state, action);
        }
        Message::ToggleSettings => {
            state.settings_open = !state.settings_open;
            if state.settings_open {
                state.palette_open = false;
            }
        }
        Message::CloseSettings => state.settings_open = false,
        Message::SetDensity(density) => {
            state.density = density;
            persist_settings(state);
        }
        Message::ToggleProblemLens => {
            state.problem_lens_enabled = !state.problem_lens_enabled;
            persist_settings(state);
        }
        Message::SetSettingsCategory(category) => state.settings_category = category,
        Message::ToggleGitStatusInTree => {
            state.git_status_in_tree = !state.git_status_in_tree;
            persist_settings(state);
        }
        Message::ToggleSaveOnFocusLoss => {
            state.save_on_focus_loss = !state.save_on_focus_loss;
            persist_settings(state);
        }
        Message::ToggleLspEnabled => {
            state.lsp_enabled = !state.lsp_enabled;
            persist_settings(state);
            if state.lsp_enabled {
                // The subscription (gated on `lsp_enabled`, see
                // `subscription`) picks this up and spawns a fresh worker;
                // `Starting` holds until its `Ready`/`Unavailable` lands.
                state.lsp_status = LspStatus::Starting;
            } else {
                // Dropping the subscription tears down the running worker
                // (`kill_on_drop` kills its `rust-analyzer` child); clear
                // the stale sender and every open editor's diagnostics so
                // nothing lingers from before the server went away.
                state.lsp_sender = None;
                state.lsp_status = LspStatus::Disabled;
                for tab in &mut state.open_tabs {
                    if let OpenTab::File(editor) = tab {
                        editor.diagnostics.clear();
                    }
                }
            }
        }
        Message::WindowUnfocused => {
            if state.save_on_focus_loss {
                save_all_dirty_files(state);
            }
        }
        Message::OpenShortcutsHelp => {
            state.settings_open = true;
            state.settings_category = SettingsCategory::Shortcuts;
            state.palette_open = false;
        }
        Message::SetEditorFontSize(size) => {
            state.editor_font_size = size.clamp(EDITOR_FONT_SIZE_MIN, EDITOR_FONT_SIZE_MAX);
            persist_settings(state);
        }
        Message::SetUiFontScale(scale) => {
            state.ui_font_scale = scale.clamp(UI_FONT_SCALE_MIN, UI_FONT_SCALE_MAX);
            persist_settings(state);
        }
        Message::DismissToast(id) => state.toasts.retain(|t| t.id != id),
        Message::PruneToasts => {
            state.toasts.retain(|t| t.created_at.elapsed() < TOAST_LIFETIME);
            if state.flash.as_ref().is_some_and(|f| f.created_at.elapsed() >= FLASH_LIFETIME) {
                state.flash = None;
            }
        }
        Message::EditorSave => return save_current_file(state),
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
        Message::BeginDraft(kind) => {
            // Pre-existing gap, fixed alongside this feature: `⌘N`/`⇧⌘N`
            // (`global_keys`) aren't otherwise gated on a project being
            // open. Without this, they'd start an invisible draft (the
            // sidebar tree isn't rendered at all during `welcome_open`)
            // targeting `state.root` — which at that point is `PathBuf::new()`,
            // so committing it would write into the process's CWD.
            if !state.welcome_open {
                return begin_draft(state, kind, state.root.clone());
            }
        }
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
            state.ctx_menu = Some(ContextMenu { target, confirm_delete: false });
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
        Message::PromptDeletePath => {
            if let Some(ctx) = state.ctx_menu.as_mut() {
                ctx.confirm_delete = true;
            }
        }
        Message::DeletePath(path) => delete_path(state, path),
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
        Message::OpenFolderDialog => {
            state.projects_open = false;
            if state.loading_project.is_some() {
                return iced::Task::none();
            }
            return iced::Task::perform(pick_folder(), |path| Message::FolderDialogResult(path, false));
        }
        Message::NewProjectDialog => {
            state.projects_open = false;
            if state.loading_project.is_some() {
                return iced::Task::none();
            }
            return iced::Task::perform(pick_folder(), |path| Message::FolderDialogResult(path, true));
        }
        Message::FolderDialogResult(path, init_git) => {
            if let Some(path) = path {
                return start_loading_project(state, path, init_git);
            }
        }
        Message::RecentProjectPicked(path) => {
            state.projects_open = false;
            if state.loading_project.is_none() {
                return start_loading_project(state, path, false);
            }
        }
        Message::CloseProject => close_project(state),
        Message::ProjectLoaded(loaded) => apply_loaded_project(state, *loaded),
        Message::SaveAsResult(old_path, new_path) => {
            if let Some(new_path) = new_path {
                complete_save_as(state, old_path, new_path);
            }
        }
        Message::EscapePressed => {
            let completions_open = active_file_path(state)
                .and_then(|ref path| find_editor(state, path))
                .is_some_and(|e| e.completions.is_some());
            if completions_open {
                if let Some(path) = active_file_path(state)
                    && let Some(editor) = find_editor_mut(state, &path)
                {
                    editor.completions = None;
                }
                return iced::Task::none();
            }
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
            } else if state.projects_open {
                state.projects_open = false;
            } else if state.palette_open {
                state.palette_open = false;
            } else if state.settings_open {
                state.settings_open = false;
            }
        }
        Message::ServerInstallComplete(result) => {
            match result {
                Ok(()) => {
                    // Bump the restart token so the subscription key changes
                    // and iced tears down the idle worker, spawning a fresh
                    // one that will now find the installed binary.
                    state.lsp_restart_token += 1;
                    state.lsp_status = LspStatus::Starting;
                    let name = active_server_name(state);
                    push_toast(state, ToastKind::Success, format!("{name} installed"));
                }
                Err(reason) => {
                    state.lsp_status = LspStatus::Unavailable(reason.clone());
                    let name = active_server_name(state);
                    push_toast(state, ToastKind::Error, format!("{name} install failed: {reason}"));
                }
            }
        }
        Message::CompletionMove(delta) => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
                && editor.completions.is_some()
            {
                let len = editor.completions.as_ref().map_or(0, Vec::len);
                if len > 0 {
                    let sel = editor.completion_selected as i32 + delta;
                    editor.completion_selected = sel.clamp(0, (len - 1) as i32) as usize;
                }
            }
        }
        Message::CompletionSelect => {
            if let Some(path) = active_file_path(state) {
                let insert_text = find_editor(state, &path).and_then(|editor| {
                    let items = editor.completions.as_ref()?;
                    let sel = editor.completion_selected.min(items.len().saturating_sub(1));
                    let item = items.get(sel)?;
                    let text = item.insert_text.as_deref().unwrap_or(&item.label).to_string();
                    Some((text, editor.completion_anchor, editor.cursor))
                });
                if let Some((text, anchor, cursor)) = insert_text {
                    if let Some(editor) = find_editor_mut(state, &path) {
                        editor.completions = None;
                        editor.completion_selected = 0;
                        // Replace text typed since the trigger with the completion.
                        let start = editor.document.char_index(anchor.line, anchor.col);
                        let end = editor.document.char_index(cursor.line, cursor.col);
                        if start <= end {
                            editor.document.remove(start..end);
                            editor.cursor = editor.document.line_col(start).into();
                        }
                        editor.insert_text(&text);
                    }
                    send_did_change_for(state, &path);
                    recompute_diff_for(state, &path);
                }
            }
        }
        Message::CloseCompletion => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
            {
                editor.completions = None;
            }
        }
        Message::Noop => {}
    }
    iced::Task::none()
}

/// Returns a `Task` (rather than being `void` like most of `update`'s
/// smaller handlers) only because `SaveFile`/`NewUntitledFile` can now
/// trigger `save_current_file`'s async Save As dialog — every other arm
/// still just mutates `state` and falls through to `Task::none()`.
fn run_palette_action(state: &mut State, action: PaletteAction) -> iced::Task<Message> {
    match action {
        PaletteAction::OpenFile(path) => open_or_focus_file(state, path),
        PaletteAction::SetThemeMode(mode) => set_theme_mode(state, mode),
        PaletteAction::SetAccent(accent) => set_accent(state, accent),
        PaletteAction::FocusSearchTab => focus_search(state),
        PaletteAction::ViewDiffOfActiveFile => {
            if let Some(path) = active_file_path(state) {
                open_or_focus_diff(state, path);
            }
        }
        PaletteAction::ViewWorkingTreeDiff => view_working_tree_diff(state),
        PaletteAction::CloseActiveTab => {
            if let Some(key) = state.active_tab.clone() {
                close_tab(state, &key);
            }
        }
        PaletteAction::ToggleAssist => state.assist_on = !state.assist_on,
        PaletteAction::ToggleProjects => state.projects_open = !state.projects_open,
        PaletteAction::ToggleProblemLens => {
            state.problem_lens_enabled = !state.problem_lens_enabled;
            persist_settings(state);
        }
        PaletteAction::IncreaseEditorFontSize => {
            state.editor_font_size =
                (state.editor_font_size + EDITOR_FONT_SIZE_STEP).clamp(EDITOR_FONT_SIZE_MIN, EDITOR_FONT_SIZE_MAX);
            persist_settings(state);
        }
        PaletteAction::DecreaseEditorFontSize => {
            state.editor_font_size =
                (state.editor_font_size - EDITOR_FONT_SIZE_STEP).clamp(EDITOR_FONT_SIZE_MIN, EDITOR_FONT_SIZE_MAX);
            persist_settings(state);
        }
        PaletteAction::IncreaseUiFontScale => {
            state.ui_font_scale =
                (state.ui_font_scale + UI_FONT_SCALE_STEP).clamp(UI_FONT_SCALE_MIN, UI_FONT_SCALE_MAX);
            persist_settings(state);
        }
        PaletteAction::DecreaseUiFontScale => {
            state.ui_font_scale =
                (state.ui_font_scale - UI_FONT_SCALE_STEP).clamp(UI_FONT_SCALE_MIN, UI_FONT_SCALE_MAX);
            persist_settings(state);
        }
        PaletteAction::OpenSettings => state.settings_open = true,
        PaletteAction::SaveFile => return save_current_file(state),
        PaletteAction::NewUntitledFile => begin_untitled_buffer(state),
    }
    iced::Task::none()
}

/// `⌘S` / palette "Save File". An untitled buffer (`document.path()` still
/// `None`) has nothing to write to yet, so this kicks off `save_file_as`
/// (the native Save As dialog) instead of calling `document.save()` — which
/// would just produce its existing "document has no path" error.
fn save_current_file(state: &mut State) -> iced::Task<Message> {
    let Some(path) = active_file_path(state) else {
        return iced::Task::none();
    };
    let Some(editor) = find_editor_mut(state, &path) else {
        return iced::Task::none();
    };
    if editor.document.path().is_none() {
        return save_file_as(state, path);
    }
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
    iced::Task::none()
}

/// Triggers the native "Save As" dialog for the tab currently keyed by
/// `old_path`, defaulting to the project root and the tab's current
/// (possibly-synthetic-untitled) name — same `Task::perform`-wrapped
/// async-fn shape as `pick_folder`/`save_file_as`'s sibling, the
/// welcome-screen work's folder picker.
fn save_file_as(state: &State, old_path: PathBuf) -> iced::Task<Message> {
    let name = old_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let dir = state.root.clone();
    iced::Task::perform(save_file_dialog(dir, name), move |chosen| Message::SaveAsResult(old_path, chosen))
}

async fn save_file_dialog(dir: PathBuf, name: String) -> Option<PathBuf> {
    rfd::AsyncFileDialog::new()
        .set_directory(dir)
        .set_file_name(name)
        .save_file()
        .await
        .map(|handle| handle.path().to_path_buf())
}

/// Saves every dirty open `File` tab — the "Save on focus loss" toggle's
/// handler. Deliberately separate from `save_current_file` rather than
/// looping it over every path: saving 5 files shouldn't stack 5 "Saved x"
/// toasts, so successes are collapsed into one summary toast and only
/// failures (rare — permission errors, a file deleted out from under an
/// open tab) get their own.
fn save_all_dirty_files(state: &mut State) {
    // Untitled buffers are deliberately skipped here, not just "no path
    // means save() would error" — this fires on a window-focus-loss
    // background event, and popping a blocking native Save As dialog the
    // moment the user alt-tabs away would be a genuinely bad surprise, not
    // just a missed save. They're saved (via the dialog) whenever the user
    // explicitly asks, same as any other dirty file.
    let dirty_paths: Vec<PathBuf> = state
        .open_tabs
        .iter()
        .filter_map(|t| match t {
            OpenTab::File(editor) if editor.document.is_dirty() && editor.document.path().is_some() => {
                Some(editor.path.clone())
            }
            _ => None,
        })
        .collect();
    if dirty_paths.is_empty() {
        return;
    }

    let mut saved = 0usize;
    let mut errors: Vec<String> = Vec::new();
    for path in &dirty_paths {
        let Some(editor) = find_editor_mut(state, path) else {
            continue;
        };
        match editor.document.save() {
            Ok(()) => saved += 1,
            Err(err) => {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                errors.push(format!("{name}: {err}"));
            }
        }
    }

    if saved > 0 {
        refresh_changed_files(state);
        let summary = if saved == 1 { "Saved 1 file".to_string() } else { format!("Saved {saved} files") };
        push_toast(state, ToastKind::Success, summary);
    }
    for err in errors {
        push_toast(state, ToastKind::Error, format!("Couldn't save {err}"));
    }
}

/// The single place any settings-panel-controlled field gets persisted
/// (`settings::save`) so it survives closing and reopening the app, same
/// shape as `recent_projects::touch` persisting on every project load.
/// Called at the end of every settings-changing `Message` arm below, so none
/// of them can silently forget to save.
fn persist_settings(state: &State) {
    settings::save(&settings::Settings {
        theme_mode: state.theme_mode,
        accent: state.accent,
        density: state.density,
        ui_font_scale: state.ui_font_scale,
        editor_font_size: state.editor_font_size,
        git_status_in_tree: state.git_status_in_tree,
        problem_lens_enabled: state.problem_lens_enabled,
        save_on_focus_loss: state.save_on_focus_loss,
        lsp_enabled: state.lsp_enabled,
    });
}

fn set_theme_mode(state: &mut State, mode: ThemeMode) {
    state.theme_mode = mode;
    persist_settings(state);
}

fn set_accent(state: &mut State, accent: Accent) {
    state.accent = accent;
    persist_settings(state);
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

/// Finishes a Save As (`save_file_as`'s dialog result): repoints the tab
/// from `old_path` to `new_path` in place — no close/reopen, so no lost
/// cursor/undo/find state — then actually writes the content, since
/// (unlike a real on-disk rename) nothing existed at `new_path` before.
///
/// Reuses `rename_open_tab` for the repointing rather than duplicating it:
/// it already does exactly "update `editor.path`/`editor.document`'s path,
/// re-notify LSP old-close/new-open, fix up `active_tab`/any matching
/// `Diff` tab key," which is precisely the bookkeeping "turn a
/// synthetic-path tab into a real-path tab" needs too. The one thing it
/// doesn't do — `EditorState::new` only derives `language`/`highlights`
/// (and JSON parsing) once, at construction, from whatever path was
/// current then — matters much more here than for an ordinary rename
/// (going from no-extension-so-no-highlighting to a real language is the
/// whole point of this flow), so those get explicitly recomputed after.
fn complete_save_as(state: &mut State, old_path: PathBuf, new_path: PathBuf) {
    rename_open_tab(state, &old_path, &new_path);
    let Some(editor) = find_editor_mut(state, &new_path) else {
        return;
    };
    editor.language = new_path.extension().and_then(|e| e.to_str()).and_then(syntax::Language::from_extension);
    editor.rehighlight();
    editor.reparse_json();

    let name = new_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    match editor.document.save() {
        Ok(()) => {
            refresh_tree(state);
            refresh_changed_files(state);
            push_toast(state, ToastKind::Success, format!("Saved {name}"));
        }
        Err(err) => push_toast(state, ToastKind::Error, format!("Couldn't save {name}: {err}")),
    }
}

/// Closes every open tab except `active_tab` — the tab-bar overflow menu's
/// "Close others". Reuses `close_tab` per key (rather than a bulk `retain`)
/// so LSP `didClose` notifications and diff-tab cleanup still happen.
/// Removes `target` (file, or recursively for a directory) from disk —
/// the confirmed "Delete" action. Closes every open tab under `target`
/// first (reusing `close_tab`, so LSP `didClose`/diff-tab cleanup/
/// `active_tab` reassignment all happen the same way `CloseActiveTab`
/// already does), then does the actual removal.
fn delete_path(state: &mut State, target: PathBuf) {
    state.ctx_menu = None;
    let is_dir = target.is_dir();
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();

    let affected: Vec<TabKey> = state
        .open_tabs
        .iter()
        .map(|t| t.key())
        .filter(|key| match key {
            TabKey::File(path) | TabKey::Diff(path) => path == &target || path.starts_with(&target),
            TabKey::Search => false,
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

    for mode in ThemeMode::ALL {
        entries.push(PaletteEntry {
            label: format!("Theme: {}", mode.label()),
            action: PaletteAction::SetThemeMode(mode),
        });
    }
    for accent in Accent::ALL {
        entries.push(PaletteEntry {
            label: format!("Accent: {}", accent.label()),
            action: PaletteAction::SetAccent(accent),
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
    // Surfaces whenever the query happens to contain "diff" (plain substring
    // filtering in `filtered_palette_entries` — no special-casing needed
    // here), matching the mockup's contextual "Diff:" command row. Gated on
    // there being something to actually diff, same as `ViewDiffOfActiveFile`
    // above — an always-present entry that's usually a no-op would be a fake
    // control.
    if active_file_path(state).is_some() || !state.changed_files.is_empty() {
        entries.push(PaletteEntry {
            label: "Diff: open working tree changes".to_string(),
            action: PaletteAction::ViewWorkingTreeDiff,
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
    entries.push(PaletteEntry {
        label: "New untitled file".to_string(),
        action: PaletteAction::NewUntitledFile,
    });

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

/// Reloads an open, *unmodified* buffer's content from disk after
/// `Message::FilesChanged` reports an external change to `path` — a dirty
/// buffer is left alone rather than clobbering in-progress local edits;
/// the git-changes panel and the tab's own modified indicator already make
/// that divergence visible without DevScribe silently picking a side.
/// Rebuilds the `EditorState` the same way opening the file fresh would
/// (`open_or_focus_file`), so highlights/undo history/etc. all stay
/// consistent with the new content instead of being patched piecemeal.
fn reload_editor_from_disk(state: &mut State, path: &Path) {
    let Some(editor) = find_editor_mut(state, path) else {
        return;
    };
    if editor.document.is_dirty() {
        return;
    }
    let Ok(document) = Document::open(path) else {
        return;
    };
    *editor = EditorState::new(document, path.to_path_buf());
    send_did_change_for(state, path);
    recompute_diff_for(state, path);
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

/// Opens a blank tab with no file on disk yet — command palette's "New
/// untitled file." Mirrors `open_or_focus_file`'s shape but starts from
/// `Document::empty()` (already `path: None` internally) instead of
/// `Document::open`.
///
/// `EditorState.path` still gets a real, unique `PathBuf` — just a
/// synthetic one (`Untitled-N`, a bare name with no directory component,
/// so it can never collide with a real project file: those are always
/// absolute, from `fs_tree::walk(&state.root)`). This keeps every existing
/// path-keyed mechanism (tab identity/lookup/close/reopen) working
/// unchanged; "untitled-ness" is tracked separately, via
/// `editor.document.path().is_none()`, checked only where it actually
/// matters (`save_current_file`). No LSP `didOpen` (no extension ⇒
/// `is_lsp_language` already says no) and no diff tab — both correctly
/// inapplicable to a buffer with no disk identity yet.
fn begin_untitled_buffer(state: &mut State) {
    state.next_untitled_id += 1;
    let name = format!("Untitled-{}", state.next_untitled_id);
    let path = PathBuf::from(name);
    let key = TabKey::File(path.clone());
    state.open_tabs.push(OpenTab::File(Box::new(EditorState::new(Document::empty(), path))));
    state.active_tab = Some(key);
}

/// The global `⇧⌘D` / palette "Diff: open working tree changes" handler:
/// diffs the active file if one's open, else falls back to the first entry
/// in the sidebar's Changes list — so the command is reachable (and does
/// something useful) even with no file tab open at all, not just a faster
/// path to the same thing `ViewDiffOfActiveFile` already covers. A no-op
/// when there's neither an active file nor any changed file to fall back to
/// (clean tree, or no repo).
fn view_working_tree_diff(state: &mut State) {
    if let Some(path) = active_file_path(state) {
        open_or_focus_diff(state, path);
    } else if let Some(entry) = state.changed_files.first() {
        open_or_focus_diff(state, entry.path.clone());
    }
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

/// Re-scans the working tree for `state.changed_files`, and the upstream
/// comparison for `state.ahead_behind` alongside it — both are cheap-ish
/// `gix` walks with the same staleness story, so one refresh point covers
/// both. Not run on every keystroke — only after actions known to change
/// the working tree, i.e. saving a file. Edits made outside DevScribe (a
/// `git commit`/`git fetch` in another terminal, say) won't be reflected
/// until the next save; there's no file-watcher wired up yet (same
/// limitation `tree`/`fs_tree` already has for the file browser).
fn refresh_changed_files(state: &mut State) {
    state.changed_files = compute_changed_files(state.repo.as_ref());
    state.ahead_behind = state.repo.as_ref().and_then(Repo::ahead_behind);
}

/// How long the search box has to sit still before `SearchDebounceTick`
/// starts a search for it — VSCode-style search-as-you-type, not a search
/// per keystroke. `SearchSubmit` (Enter) bypasses this entirely.
const SEARCH_DEBOUNCE_DELAY: Duration = Duration::from_millis(300);

/// Naive project-wide search: reads every file the sidebar already walked
/// and scans it. Capped so even one search can't run away — see
/// `devscribe_core::search` for the "start naive, index later if slow"
/// rationale. Briefly lowered to 50 while chasing what turned out to be an
/// unrelated, now-fixed root cause (see `SearchHit::preview`'s doc and the
/// roadmap's search bug-fix writeup, "ninth pass") — restored to 200 now
/// that every result's render cost is genuinely bounded regardless of the
/// underlying line's real length, so there's no reason left to show fewer.
const MAX_SEARCH_RESULTS: usize = 200;

/// Files larger than this are skipped entirely rather than read and
/// scanned. This is naive search — unlike an indexed tool (ripgrep, an
/// LSP), it has no way to bound the cost of one huge file (a lockfile, a
/// bundle, a log) other than not reading it. `MAX_SEARCH_RESULTS`/
/// `search_text`'s own `max_hits` cap the *match count*, but a
/// many-megabyte file that matches nothing would still pay the full
/// read+scan cost without this.
const MAX_SEARCH_FILE_BYTES: u64 = 2 * 1024 * 1024;

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
const MAX_SEARCH_FILES_SCANNED: usize = 3_000;

/// The actual file-reading/scanning work, decoupled from `State` so it can
/// run on a background thread (see `start_search`) — takes owned inputs
/// and returns an owned outcome instead of borrowing app state, since
/// nothing here can hold a reference across a thread boundary.
fn run_search(files: &[PathBuf], query: &str) -> SearchOutcome {
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
fn start_search(state: &mut State) -> iced::Task<Message> {
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
        // Belt-and-suspenders alongside `logging::init`'s process-wide panic
        // hook: that hook already logs *that* a panic happened on this
        // thread, but `catch_unwind` here lets this specific call site say
        // *what it was doing* (a plain "search thread panicked" beats
        // hunting for which of several background threads a hook fired on).
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
async fn pick_folder() -> Option<PathBuf> {
    rfd::AsyncFileDialog::new().pick_folder().await.map(|handle| handle.path().to_path_buf())
}

/// Starts a background project load: optionally `git init`s `path` (for
/// "New project", when it isn't already a repo), then computes a full
/// `snapshot_project`. Guarded by the caller checking `loading_project` is
/// already `None` — unlike search, this is a one-shot action with no
/// cancel-and-restart behavior, so a simple "ignore while one's in flight"
/// guard is enough (no `Handle`/`abortable` needed).
fn start_loading_project(state: &mut State, path: PathBuf, init_git: bool) -> iced::Task<Message> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());
    state.loading_project = Some(LoadingProject { name, path: path.clone() });

    iced_runtime::task::blocking(move |mut sender| {
        // Same belt-and-suspenders `catch_unwind` as `start_search`, on top
        // of `logging::init`'s process-wide panic hook.
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if init_git && Repo::open(&path).is_none() {
                // Best-effort: if `git init` fails, still open the folder
                // as a non-repo project rather than losing the pick.
                Repo::init(&path);
            }
            LoadedProject { root: path.clone(), snapshot: snapshot_project(&path) }
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
fn reset_project_scoped_state(state: &mut State) {
    state.open_tabs.clear();
    state.active_tab = None;
    state.closed_tabs.clear();
    state.draft = None;
    state.ctx_menu = None;
    state.changes_panel_open = false;
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
}

fn apply_loaded_project(state: &mut State, loaded: LoadedProject) {
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
}

fn close_project(state: &mut State) {
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

/// Spawns a background OS thread that installs `language`'s server binary
/// via the method defined in `server_install::spec_for`. Result arrives as
/// `Message::ServerInstallComplete`.
fn start_server_install(language: LspLanguage) -> iced::Task<Message> {
    iced_runtime::task::blocking(move |mut sender| {
        let spec = crate::server_install::spec_for(language);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::server_install::install(&spec)
        }))
        .unwrap_or_else(|_| Err("install panicked unexpectedly".into()));
        let _ = sender.try_send(result);
    })
    .map(Message::ServerInstallComplete)
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

fn char_col_to_utf16_col(line: &str, char_col: usize) -> u32 {
    line.chars().take(char_col).map(|c| c.len_utf16() as u32).sum()
}

/// Language of the currently active file, if it maps to a known LSP language.
pub fn active_lsp_language(state: &State) -> Option<LspLanguage> {
    active_file_path(state)?
        .extension()
        .and_then(|ext| ext.to_str())
        .and_then(LspLanguage::from_extension)
}

/// Display name of the active language server (for status bar / toasts).
pub fn active_server_name(state: &State) -> &'static str {
    active_lsp_language(state)
        .map(LspLanguage::command)
        .unwrap_or("language server")
}

/// Keyed on `(root, language, restart_token)` so that:
/// - a project switch (new `root`) tears down the old worker automatically
/// - switching the active file to a different language does the same
/// - a successful auto-install increments `restart_token`, forcing iced to
///   respawn the worker so it picks up the newly installed binary
///
/// Binary resolution happens inside the async stream body so the main thread
/// never blocks: if the binary isn't found, `NeedsInstall` is emitted and
/// the worker exits cleanly.
fn lsp_worker((root, language, _token): &(PathBuf, LspLanguage, u64)) -> impl iced::futures::Stream<Item = LspEvent> + use<> {
    let root = root.clone();
    let language = *language;
    iced::stream::channel(32, async move |mut output| {
        use iced::futures::SinkExt as _;
        let spec = crate::server_install::spec_for(language);
        match crate::server_install::resolve_binary(&spec) {
            Some(binary) => lsp::run(root, language, binary, output).await,
            None => { let _ = output.send(LspEvent::NeedsInstall).await; }
        }
    })
}

/// Keyed on `root` alone, mirroring `lsp_worker`'s keying — switching
/// projects tears down and respawns the watcher on the new root.
fn file_watcher(root: &PathBuf) -> impl iced::futures::Stream<Item = Vec<WatchEvent>> + use<> {
    let root = root.clone();
    iced::stream::channel(32, async move |output| {
        watcher::run(root, output).await;
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
            if modifiers.shift() && c.eq_ignore_ascii_case("d") {
                return Message::ViewWorkingTreeDiff;
            }
            if c.eq_ignore_ascii_case("i") {
                return Message::ToggleAssist;
            }
            if c.eq_ignore_ascii_case("/") {
                return Message::OpenShortcutsHelp;
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

/// Maps `iced::window::events()` to `Message::WindowUnfocused` on the one
/// variant `save_on_focus_loss` cares about — every other window event
/// (resize, move, redraw…) is a no-op here.
fn window_events((_id, event): (iced::window::Id, iced::window::Event)) -> Message {
    match event {
        iced::window::Event::Unfocused => Message::WindowUnfocused,
        _ => Message::Noop,
    }
}

/// Drives an in-progress sidebar drag (see `Message::SidebarResizeStarted`)
/// with window-wide cursor tracking — the resize handle itself is only a
/// few pixels wide, far narrower than a fast drag's mouse movement, so the
/// handle's own `mouse_area` can't be the thing reporting position once the
/// cursor has left it. Only subscribed while `state.sidebar_resizing`, so
/// idle frames don't pay for a global mouse listener.
fn sidebar_resize_events(event: iced::Event, _status: iced::event::Status, _window: iced::window::Id) -> Option<Message> {
    match event {
        iced::Event::Mouse(mouse::Event::CursorMoved { position }) => Some(Message::SidebarResizeDragged(position.x)),
        iced::Event::Mouse(mouse::Event::ButtonReleased(_)) => Some(Message::SidebarResizeEnded),
        _ => None,
    }
}

pub fn subscription(state: &State) -> iced::Subscription<Message> {
    let mut subs = vec![
        iced::time::every(std::time::Duration::from_millis(530)).map(|_| Message::CaretTick),
        iced::time::every(Duration::from_secs(1)).map(|_| Message::PruneToasts),
        iced::keyboard::listen().map(global_keys),
        iced::window::events().map(window_events),
    ];
    // No LSP worker while no project is open or LSP is disabled. The key
    // includes `lsp_restart_token` so incrementing it (after a successful
    // auto-install) forces iced to respawn the worker — see `lsp_worker`.
    if !state.welcome_open && state.lsp_enabled {
        if let Some(lang) = active_lsp_language(state) {
            let key = (state.root.clone(), lang, state.lsp_restart_token);
            subs.push(iced::Subscription::run_with(key, lsp_worker).map(Message::Lsp));
        }
    }
    // No file watcher with no project open — keyed on `root` so switching
    // projects tears down and respawns it on the new one (see `file_watcher`).
    if !state.welcome_open {
        subs.push(iced::Subscription::run_with(state.root.clone(), file_watcher).map(Message::FilesChanged));
    }
    // Only ticks while there's an actual pending debounce to check
    // (`search_query_changed_at` goes back to `None` the moment a search
    // starts — see `Message::SearchDebounceTick`/`SearchSubmit`), *not* a
    // permanent 10Hz subscription. A found-and-fixed regression from the
    // debounce rewrite: an unconditional 100ms tick forces a full `view()`
    // rebuild ten times a second for the *entire app*, forever, not just
    // while search is in use — with a couple hundred result rows sitting on
    // screen, that's real, sustained, unbounded-duration load, easily
    // enough to look like the app hanging even though nothing is actually
    // stuck (see the roadmap's search bug-fix writeup, "seventh pass").
    if state.search_query_changed_at.is_some() {
        subs.push(iced::time::every(Duration::from_millis(100)).map(|_| Message::SearchDebounceTick));
    }
    if state.sidebar_resizing {
        subs.push(iced::event::listen_with(sidebar_resize_events));
    }
    iced::Subscription::batch(subs)
}

#[cfg(test)]
#[path = "tests/state.rs"]
mod tests;
