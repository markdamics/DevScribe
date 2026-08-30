use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use devscribe_core::claude_agent::{self, ClaudeCommand, ClaudeEvent, PermissionMode};
use devscribe_core::diff::{DiffLine, GutterMark, Hunk};
use devscribe_core::git::{ChangeKind, Repo};
use devscribe_core::lsp::{self, CompletionItem, LspCommand, LspEvent, LspLanguage};
use devscribe_core::outline;
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
use crate::ui::editor_canvas;

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
    /// The AI Chat Assist panel, opened as a full tab instead of docked —
    /// like `Search`, never backed by an `OpenTab` entry; the actual
    /// conversation lives on `State::chat` regardless of presentation.
    Chat,
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
    ChatToggle,
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
    /// `Rc` because `shell.rs` clones this into the canvas program on every
    /// `view()` — and again per layout pass inside `responsive`. A large
    /// file's span list runs to hundreds of thousands of entries, so a real
    /// clone there was megabytes of copying several times a second (the
    /// caret-blink subscription alone redraws 2x/sec).
    pub highlights: Rc<Vec<Span>>,
    highlighter: syntax::Highlighter,
    /// A second, independent parse of the same buffer — see
    /// `devscribe_core::outline`'s module doc for why `highlighter` above
    /// can't supply this itself. `None` for a file with no landmark table
    /// wired for its language, or one that hasn't parsed successfully.
    /// Recomputed alongside `highlights` at settle time; while
    /// `needs_reparse` is set, `breadcrumbs()` doesn't read it — a tree
    /// this stale could point past the live buffer's own end.
    tree: Option<outline::Tree>,
    /// `Rc` for the same reason as `highlights`.
    pub diagnostics: Rc<Vec<EditorDiagnostic>>,
    /// `Some(Ok(_))`/`Some(Err(parse_message))` for `.json` files, `None`
    /// otherwise. Recomputed on every edit, like `highlights`.
    pub json: Option<Result<serde_json::Value, String>>,
    /// Collapsed node paths in the JSON tree view (e.g. `"root.foo[2]"`).
    pub json_collapsed: HashSet<String>,
    /// This file's content at `HEAD` diffed against the live buffer.
    pub diff: DiffStatus,
    /// One entry per buffer line, derived from `diff` — the editor gutter's
    /// per-line added/modified/removed indicator, and what `revert_line`
    /// acts on. Empty when `diff` isn't `Changed`. `Rc` for the same reason
    /// as `highlights`: cloned into the canvas program on every `view()`.
    pub gutter_marks: Rc<Vec<Option<GutterMark>>>,
    /// Same grouping as `gutter_marks`, kept as hunks rather than flattened
    /// per-line — what the diff view's "revert selected changes" selects
    /// and acts on. Empty when `diff` isn't `Changed`, recomputed alongside
    /// it.
    pub hunks: Rc<Vec<Hunk>>,
    /// Hunks currently checked in the diff view, keyed by `Hunk::range.start`
    /// (stable while `diff`/`hunks` don't change, which is the only time
    /// this is read). Cleared whenever `diff` is recomputed, since a hunk at
    /// that index may no longer be the same change once the buffer or `HEAD`
    /// has moved.
    pub diff_selected_hunks: HashSet<usize>,
    /// Whether the diff view's "Revert selected" has been clicked once and
    /// is waiting on its confirm/cancel step — same two-step shape as the
    /// sidebar's `State::pending_discard`.
    pub pending_hunk_revert: bool,
    /// Set by every edit, cleared by `reparse_now`. The expensive derived
    /// views (tree-sitter spans, the JSON tree) are recomputed once the
    /// buffer settles rather than on every keystroke — see `EDIT_SETTLE`.
    needs_reparse: bool,
    /// `Some` while this tab's find widget (Ctrl+F) is open.
    pub find: Option<FindState>,
    /// Vertical scroll offset (px from the top) of this tab's editor
    /// canvas, last reported by the `scrollable`'s `on_scroll`. Used to
    /// virtualize `EditorCanvas::draw` (skip lines outside the visible
    /// range) and, together with `viewport_height`, to decide whether a
    /// Find match is already on-screen before scrolling to it.
    pub scroll_offset: f32,
    /// Height (px) of this tab's editor scroll viewport, last reported
    /// alongside `scroll_offset`. `0.0` until the first `on_scroll` fires
    /// (e.g. a fresh tab that hasn't been scrolled or resized yet) —
    /// `find_step` falls back to an assumed height in that case rather
    /// than refusing to scroll.
    pub viewport_height: f32,
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
        // One materialization, shared with `reparse_json_with` below — see
        // `resync_after_edit`. Skipped entirely for files with no grammar,
        // which are also the files JSON parsing never looks at.
        let text = language.map(|_| document.text().to_string());
        let highlights = match (language, text.as_deref()) {
            (Some(lang), Some(text)) => highlighter.highlight(lang, text),
            _ => Vec::new(),
        };
        let tree = match (language, text.as_deref()) {
            (Some(lang), Some(text)) => outline::parse(lang, text),
            _ => None,
        };
        let mut this = Self {
            document,
            path,
            cursor: CursorPos::default(),
            selection_anchor: None,
            language,
            highlights: Rc::new(highlights),
            highlighter,
            tree,
            diagnostics: Rc::new(Vec::new()),
            json: None,
            json_collapsed: HashSet::new(),
            diff: DiffStatus::default(),
            gutter_marks: Rc::new(Vec::new()),
            hunks: Rc::new(Vec::new()),
            diff_selected_hunks: HashSet::new(),
            pending_hunk_revert: false,
            needs_reparse: false,
            find: None,
            scroll_offset: 0.0,
            viewport_height: 0.0,
            completions: None,
            completion_selected: 0,
            completion_anchor: CursorPos::default(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            last_edit_kind: None,
        };
        this.reparse_json_with(text.as_deref().unwrap_or(""));
        this
    }

    /// Brings the buffer's derived views back in step after an edit.
    ///
    /// Only the cheap half runs here. A full tree-sitter reparse measured
    /// **33 ms** on an 850-line Rust file in a debug build (7.4 ms in
    /// release) — running that, a git blob read and a whole-file LSP
    /// `didChange` on *every keystroke* is what made typing lag and then
    /// wedge the app. Those are deferred to `reparse_now`, which
    /// `Message::EditSettleTick` calls once the buffer stops changing.
    ///
    /// Find matches stay immediate: they are cheap, and the on-screen
    /// highlight would visibly drift from the text otherwise.
    fn resync_after_edit(&mut self) {
        self.needs_reparse = true;
        self.refind();
    }

    /// Recomputes the expensive derived views — syntax spans and the JSON
    /// tree — from the current buffer. A no-op unless an edit is pending, so
    /// it is safe to call speculatively (see `flush_pending_edits`).
    ///
    /// Materializes the buffer as a `String` **once** and shares it; the two
    /// used to call `Rope::to_string()` apiece.
    fn reparse_now(&mut self) {
        if !self.needs_reparse {
            return;
        }
        self.needs_reparse = false;
        let owned = self.language.map(|_| self.document.text().to_string());
        let text = owned.as_deref().unwrap_or("");
        self.rehighlight_with(text);
        self.reparse_json_with(text);
        self.tree = self.language.and_then(|lang| outline::parse(lang, text));
    }

    /// The stack of enclosing scopes at the cursor, for the breadcrumb
    /// strip (`ui::breadcrumb_bar`) — outermost first. Empty while
    /// `needs_reparse` is set: `tree` is up to `EDIT_SETTLE` stale then,
    /// and showing a breadcrumb computed against a buffer the cursor has
    /// since moved out of would be actively wrong, not just late (e.g.
    /// still claiming "inside settle_batch" after the whole function was
    /// just deleted). This is cheap enough to call on every `view()` —
    /// see `devscribe_core::outline`'s module doc for why.
    pub fn breadcrumbs(&self) -> Vec<outline::Crumb> {
        if self.needs_reparse {
            return Vec::new();
        }
        let Some(tree) = self.tree.as_ref() else {
            return Vec::new();
        };
        let Some(lang) = self.language else {
            return Vec::new();
        };
        let byte = self
            .document
            .text()
            .char_to_byte(self.document.char_index(self.cursor.line, self.cursor.col));
        outline::breadcrumbs_at(tree, self.document.text(), byte, lang)
    }

    /// Inserts at `char_idx`, keeping `highlights` aligned — see
    /// `shift_highlights`.
    fn edit_insert(&mut self, char_idx: usize, text: &str) {
        let at = self.document.text().char_to_byte(char_idx);
        self.document.insert(char_idx, text);
        self.shift_highlights(at, 0, text.len());
    }

    /// Removes `range` (chars), keeping `highlights` aligned — see
    /// `shift_highlights`.
    fn edit_remove(&mut self, range: std::ops::Range<usize>) {
        let rope = self.document.text();
        let at = rope.char_to_byte(range.start);
        let removed = rope.char_to_byte(range.end) - at;
        self.document.remove(range);
        self.shift_highlights(at, removed, 0);
    }

    /// Slides existing highlight spans across an edit that replaced
    /// `removed` bytes at `at` with `inserted` bytes.
    ///
    /// Spans are byte offsets into a buffer that has just moved underneath
    /// them, and the real reparse is up to `EDIT_SETTLE` away. Without this
    /// the colouring of everything after the cursor would visibly slide out
    /// of register for the whole of a typing burst — the debounce never
    /// fires while keys keep arriving. This is an approximation (it cannot
    /// know that the token under the cursor just became a keyword), and the
    /// reparse corrects it the moment typing stops.
    fn shift_highlights(&mut self, at: usize, removed: usize, inserted: usize) {
        if self.highlights.is_empty() || (removed == 0 && inserted == 0) {
            return;
        }
        let removed_end = at + removed;
        let shift = |x: usize| {
            if x <= at {
                x
            } else if x >= removed_end {
                x - removed + inserted
            } else {
                // Inside the removed region: collapses onto the edit point.
                at
            }
        };
        let spans = Rc::make_mut(&mut self.highlights);
        for span in spans.iter_mut() {
            span.start = shift(span.start);
            span.end = shift(span.end);
        }
        spans.retain(|s| s.start < s.end);
    }

    /// Recomputes `highlights` from `text`, which must be the current buffer
    /// contents. Cheap relative to a full reparse would suggest otherwise,
    /// but tree-sitter is fast enough that doing this on every edit is fine —
    /// see `devscribe_core::syntax` for why this isn't true incremental
    /// reparsing.
    fn rehighlight_with(&mut self, text: &str) {
        if let Some(lang) = self.language {
            self.highlights = Rc::new(self.highlighter.highlight(lang, text));
        }
    }

    /// Recomputes `json` from `text` (the current buffer contents), for
    /// `.json` files.
    fn reparse_json_with(&mut self, text: &str) {
        self.json = (self.language == Some(syntax::Language::Json)).then(|| {
            serde_json::from_str::<serde_json::Value>(text).map_err(|e| e.to_string())
        });
    }

    /// `refind_with`, materializing the buffer itself. For the two callers
    /// that change the query without editing the document; every edit goes
    /// through `resync_after_edit` instead, which shares one materialization
    /// across all three recomputations.
    fn refind(&mut self) {
        let needs_text = self.find.as_ref().is_some_and(|f| !f.query.is_empty());
        let owned = needs_text.then(|| self.document.text().to_string());
        self.refind_with(owned.as_deref().unwrap_or(""));
    }

    /// Recomputes `find`'s matches from its current query against `text`
    /// (the current buffer contents), for `.find.is_some()` files. Called
    /// both when the query changes and on every edit, like `rehighlight_with`.
    fn refind_with(&mut self, text: &str) {
        let Some(query) = self.find.as_ref().map(|f| f.query.clone()) else {
            return;
        };
        let matches = if query.is_empty() {
            Vec::new()
        } else {
            let query_len = query.chars().count();
            // Same unbounded-memory risk `recompute_search` guards against
            // (every match clones its whole line) — capped here too, even
            // though a single already-open buffer is a smaller blast radius
            // than the whole project.
            search::search_text(text, &query, MAX_SEARCH_RESULTS)
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
            self.edit_remove(start..end);
            self.cursor = self.document.line_col(start).into();
            true
        } else {
            false
        }
    }

    /// The editor gutter's click-to-revert action: restores `line`'s content
    /// (or, for a `RemovedAbove` mark, re-inserts the deleted lines above
    /// it) back to `HEAD`, per whatever `gutter_marks` says about it. `false`
    /// if `line` has no mark — gone stale, e.g. a concurrent external edit —
    /// in which case there's nothing to do and the caller has nothing new to
    /// recompute.
    pub fn revert_line(&mut self, line: usize) -> bool {
        if self.gutter_marks.get(line).and_then(|m| m.as_ref()).is_none() {
            return false;
        }
        self.record_undo_boundary(EditKind::Other);
        self.apply_revert_mark(line);
        self.cursor = self.document.line_col(self.document.char_index(line, 0)).into();
        self.resync_after_edit();
        true
    }

    /// The diff view's "revert selected changes": reverts several new-buffer
    /// lines back to `HEAD`, per `gutter_marks`, as a single undo step —
    /// a batch of hunks undoes together rather than one `Ctrl+Z` per hunk.
    /// `false` (no-op) if none of `lines` still carries a mark.
    ///
    /// Applies in descending line order so that editing a higher-numbered
    /// target — which can shift line numbers below it, e.g. `RemovedAbove`
    /// inserting lines — never moves a not-yet-processed (necessarily
    /// lower-numbered) target out from under it. Callers don't need to sort
    /// `lines` themselves.
    pub fn revert_lines(&mut self, lines: &[usize]) -> bool {
        let mut targets: Vec<usize> = lines
            .iter()
            .copied()
            .filter(|&line| self.gutter_marks.get(line).and_then(|m| m.as_ref()).is_some())
            .collect();
        targets.sort_unstable();
        targets.dedup();
        let Some(&first) = targets.first() else {
            return false;
        };

        self.record_undo_boundary(EditKind::Other);
        for &line in targets.iter().rev() {
            self.apply_revert_mark(line);
        }
        self.cursor = self.document.line_col(self.document.char_index(first, 0)).into();
        self.resync_after_edit();
        true
    }

    /// The actual document edit behind both `revert_line` and
    /// `revert_lines` — restores `line`'s content (or, for a `RemovedAbove`
    /// mark, re-inserts the deleted lines above it) back to `HEAD`, per
    /// whatever `gutter_marks` says about it. A no-op if `line` has no mark.
    fn apply_revert_mark(&mut self, line: usize) {
        let Some(mark) = self.gutter_marks.get(line).and_then(|m| m.clone()) else {
            return;
        };
        match mark {
            GutterMark::Modified { head_text } => {
                let start = self.document.char_index(line, 0);
                let end = start + self.document.line_len_chars(line);
                self.edit_remove(start..end);
                self.edit_insert(start, &head_text);
            }
            GutterMark::Added => {
                let range = self.document.line_char_range_with_terminator(line);
                self.edit_remove(range);
            }
            GutterMark::RemovedAbove { head_lines } => {
                let at = self.document.char_index(line, 0);
                let mut text = String::new();
                for head_line in &head_lines {
                    text.push_str(head_line);
                    text.push('\n');
                }
                self.edit_insert(at, &text);
            }
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
        self.edit_insert(idx, text);
        let new_idx = idx + text.chars().count();
        self.cursor = self.document.line_col(new_idx).into();
        self.resync_after_edit();
    }

    pub fn backspace(&mut self) {
        self.record_undo_boundary(EditKind::Delete);
        if self.delete_selection() {
            self.resync_after_edit();
            return;
        }
        let idx = self.document.char_index(self.cursor.line, self.cursor.col);
        if idx == 0 {
            return;
        }
        self.edit_remove(idx - 1..idx);
        self.cursor = self.document.line_col(idx - 1).into();
        self.resync_after_edit();
    }

    pub fn delete_forward(&mut self) {
        self.record_undo_boundary(EditKind::Delete);
        if self.delete_selection() {
            self.resync_after_edit();
            return;
        }
        let idx = self.document.char_index(self.cursor.line, self.cursor.col);
        if idx >= self.document.text().len_chars() {
            return;
        }
        self.edit_remove(idx..idx + 1);
        self.cursor = self.document.line_col(idx).into();
        self.resync_after_edit();
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
        self.resync_after_edit();
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
        self.resync_after_edit();
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
            self.resync_after_edit();
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

/// AI Chat Assist docked-panel width bounds — same drag-handle idiom as the
/// sidebar's, just anchored to the right edge instead of the left.
pub const CHAT_MIN_WIDTH: f32 = 280.0;
pub const CHAT_MAX_WIDTH: f32 = 560.0;
pub const CHAT_DEFAULT_WIDTH: f32 = 340.0;

/// How the AI Chat Assist panel is currently presented. Replaces the old
/// bare `assist_on: bool` placeholder — there was no panel behind that
/// toggle at all before this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatMode {
    Docked,
    Collapsed,
    Closed,
}

impl ChatMode {
    pub const ALL: [ChatMode; 3] = [ChatMode::Docked, ChatMode::Collapsed, ChatMode::Closed];
}

/// `true` whenever a live `claude` session should exist — either the
/// docked/collapsed presentation is on, *or* it's open as a full tab.
/// Opening as a tab sets `chat_mode` to `Closed` (the docked panel and
/// the tab view are mutually exclusive presentations of the same session),
/// so `chat_mode != Closed` alone isn't the right "is chat active" check —
/// unlike the source mockup's own `chatLamp`, which checks `chatMode`
/// alone and so would show "off" while genuinely live as a tab.
pub fn chat_is_active(state: &State) -> bool {
    state.chat_mode != ChatMode::Closed || state.chat_tab_open
}

/// One entry in the chat transcript. A tool call is a single evolving
/// entry (`Tool`) rather than separate "started"/"result" messages: the
/// wire protocol reports both under the same id (see
/// `devscribe_core::claude_agent`), and a gated call additionally reports
/// a `PermissionRequest` under that *same* id, so keeping one entry keyed
/// by id — rather than a separate `pending_permission` field that could
/// drift out of sync with the transcript — is the simpler, single-source
/// of truth.
#[derive(Debug, Clone)]
pub enum ChatMessage {
    Operator(String),
    /// `streaming` is `true` while this bubble is still being live-typed
    /// from `ClaudeEvent::AssistantTextDelta` chunks — the block's own
    /// `AssistantText` finalizes it (sets this back to `false`) rather than
    /// starting a second bubble. Always `false` for a bubble that arrived
    /// as one complete `AssistantText` with no preceding deltas (e.g.
    /// replayed session history, which never streams — see
    /// `ClaudeEvent::AssistantTextDelta`'s own doc comment).
    Assistant { text: String, streaming: bool },
    Tool(ToolActivity),
}

#[derive(Debug, Clone)]
pub struct ToolActivity {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
    /// `None` for tools that never needed a human decision (Read/Grep/...).
    pub permission: Option<PermissionState>,
    /// `None` while still running.
    pub result: Option<ToolActivityResult>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionState {
    Pending,
    Approved,
    Denied,
}

#[derive(Debug, Clone)]
pub struct ToolActivityResult {
    pub is_error: bool,
    pub result: serde_json::Value,
}

/// Whether the chat subprocess is up yet — drives what the panel shows
/// before the first `Ready` event (or if `claude` isn't on PATH at all).
#[derive(Debug, Clone, Default, PartialEq)]
pub enum ChatStatus {
    #[default]
    Starting,
    Ready,
    Unavailable(String),
}

/// The AI Chat Assist conversation and session bookkeeping — independent
/// of presentation (`State::chat_mode`/`chat_tab_open`), so switching
/// between docked/collapsed/window/tab never loses the thread. Reset to
/// `ChatThread::default()` whenever a new `claude` subprocess is spawned
/// (see `Message::Chat(ClaudeEvent::Ready)`).
#[derive(Debug, Clone, Default)]
pub struct ChatThread {
    pub messages: Vec<ChatMessage>,
    pub session_id: Option<String>,
    pub model: Option<String>,
    pub cost_usd: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// The input bar's current draft — a real multi-line editor
    /// (`iced::widget::text_editor`, not a single-line `text_input`), for
    /// proper cursor movement/selection/Shift+Enter newlines. Not
    /// `#[derive(PartialEq)]`-friendly and doesn't need to be: nothing
    /// compares two `ChatThread`s for equality.
    pub input: iced::widget::text_editor::Content,
    pub sender: Option<mpsc::Sender<ClaudeCommand>>,
    pub status: ChatStatus,
}

impl ChatThread {
    fn find_tool_mut(&mut self, id: &str) -> Option<&mut ToolActivity> {
        self.messages.iter_mut().rev().find_map(|m| match m {
            ChatMessage::Tool(tool) if tool.id == id => Some(tool),
            _ => None,
        })
    }
}

pub struct State {
    pub theme_mode: ThemeMode,
    pub accent: Accent,
    /// Every currently open tab, in the order they appear in the tab bar.
    pub open_tabs: Vec<OpenTab>,
    /// `None` only when `open_tabs` is empty.
    pub active_tab: Option<TabKey>,
    pub chat_mode: ChatMode,
    /// `true` while the AI Chat Assist panel is open as a full tab instead
    /// of docked/collapsed/floating — see `TabKey::Chat`/`chat_is_active`.
    pub chat_tab_open: bool,
    /// Docked chat panel width in logical pixels — same drag-handle idiom
    /// as `sidebar_width`, clamped to `[CHAT_MIN_WIDTH, CHAT_MAX_WIDTH]`.
    pub chat_panel_width: f32,
    /// `true` while the chat panel's resize handle is being dragged — mirrors
    /// `sidebar_resizing`.
    pub chat_resizing: bool,
    /// Bumped to force `subscription()` to tear down and respawn the chat
    /// worker for the *same* session id — reserved for a future "retry
    /// after the process died" action, not currently wired to any UI.
    /// Mirrors `lsp_restart_token`.
    pub chat_restart_token: u64,
    /// The session id `chat_worker` will spawn or resume next — never
    /// `None`: `State::default()`/`reset_project_scoped_state` always seed
    /// a fresh one (see `claude_agent::new_session_id`). Whether the
    /// worker treats it as a brand-new session or resumes an existing one
    /// is decided by the worker itself, by checking whether a transcript
    /// for this id already exists (`claude_agent::session_exists`) — not
    /// tracked as separate state here, so there's nothing to keep in sync
    /// when e.g. reopening the panel after closing it naturally turns into
    /// a resume of the same conversation.
    pub chat_session_id: String,
    /// Past sessions for the current project, most recently active first —
    /// populated on demand (`Message::ChatToggleSessions` opening the
    /// picker), not kept live, since it only reflects a filesystem scan at
    /// the moment it was requested.
    pub chat_sessions: Vec<claude_agent::SessionSummary>,
    pub chat_sessions_open: bool,
    /// The header's "View" popup — lists whichever of Docked/Tab/Collapsed
    /// the panel *isn't* currently presented as, same backdrop-close popup
    /// convention as `tab_bar::overflow_menu`. Closed automatically by
    /// every message that actually switches the presentation
    /// (`ChatDock`/`ChatCollapse`/`ChatOpenTab`/`ChatDockFromTab`), same as
    /// `overflow_open` resets after its own menu actions.
    pub chat_view_menu_open: bool,
    /// The input bar's "+" Actions popup — attach/mention a file, clear the
    /// conversation, switch model, toggle thinking, account & usage. Same
    /// backdrop-close popup convention as `chat_view_menu_open`; every
    /// action closes it on press.
    pub chat_actions_open: bool,
    /// Local-only UI state for the Actions popup's "Thinking" toggle — not
    /// persisted (like `chat_permission_mode`, this only matters for the
    /// currently-running subprocess, if any), and not round-tripped from
    /// `claude` itself: toggling it just sends `/effort high` or `/effort
    /// auto` as a prompt (confirmed against the real CLI: `claude` answers
    /// `/effort` in-place, `num_turns: 0`, no model call), so this bool is
    /// purely "which state did the button last show," not a query of the
    /// session's actual effort level.
    pub chat_thinking_enabled: bool,
    /// The Actions popup's "Shell Access" toggle — whether `Bash` is
    /// allowed at all for this session (see
    /// `claude_agent::SessionOptions::allow_bash`). Off by default every
    /// time, like `chat_thinking_enabled`, and never persisted: running
    /// real shell commands is a materially different risk than editing
    /// files, so it has to be turned on deliberately each session rather
    /// than remembered. Once on, `Bash` is gated by `chat_permission_mode`
    /// exactly like `Edit`/`Write` already are. Part of `chat_worker`'s
    /// subscription key for the same reason `chat_permission_mode` is:
    /// `--disallowedTools` is a spawn-time flag, so toggling this respawns
    /// the subprocess.
    pub chat_shell_access_enabled: bool,
    /// How permission decisions are made for the session — see
    /// `PermissionMode`. Part of `chat_worker`'s subscription key: picking
    /// a different mode respawns the subprocess (`--permission-mode` is a
    /// spawn-time flag, can't change on a running process), resuming the
    /// same session id under the new mode rather than losing the thread.
    pub chat_permission_mode: PermissionMode,
    pub chat: ChatThread,
    /// Current window width in logical pixels — tracked only for the chat
    /// panel's right-edge resize math (`window_width - cursor_x`, since
    /// unlike the sidebar's left-edge handle, the cursor position alone
    /// isn't the width). Updated from `iced::window::Event::Opened`/
    /// `Resized`; starts at `main.rs`'s initial `window_size` request.
    pub window_width: f32,
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
    /// The Changes-panel row currently showing its confirm/cancel step for
    /// "Discard changes" — mirrors `ContextMenu::confirm_delete`'s two-step
    /// pattern for the file tree's Delete action, since discarding a file's
    /// changes is equally destructive and irreversible.
    pub pending_discard: Option<PathBuf>,
    /// `true` while the status bar's Problems dock panel is open, listing
    /// every diagnostic across all open files. Toggled by clicking the
    /// status bar's Problems indicator — see `status_bar.rs`.
    pub problems_panel_open: bool,
    /// When the most recent edit landed, or `None` when nothing is pending.
    /// Drives the `EditSettleTick` subscription, which only exists while
    /// this is `Some` — an unconditional tick would rebuild the entire view
    /// ten times a second forever (the same trap the search debounce
    /// documents).
    edit_settled_at: Option<Instant>,
    /// Files edited since the last settle, in first-edited order. Almost
    /// always one, but a fast tab switch mid-burst can leave two.
    pending_edits: Vec<PathBuf>,
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
    /// Gates the file tree walk (`fs_tree::walk`) itself: when off (the
    /// default), dotfiles/dot-dirs and `fs_tree::SKIP_DIRS` (`.git`,
    /// `target`, `node_modules`, …) are omitted from `state.tree`; when on,
    /// every entry shows. Toggling it re-walks via `refresh_tree`.
    pub show_hidden_files: bool,
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
fn snapshot_project(root: &Path, show_hidden: bool) -> ProjectSnapshot {
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
fn startup(show_hidden: bool) -> Startup {
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
fn startup(_show_hidden: bool) -> Startup {
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
        let settings = startup_settings();
        let Startup { welcome_open, root, snapshot, repo, recent_projects, welcome_rows } = startup(settings.show_hidden_files);

        Self {
            theme_mode: settings.theme_mode,
            accent: settings.accent,
            open_tabs: Vec::new(),
            active_tab: None,
            chat_mode: settings.chat_mode,
            chat_tab_open: false,
            chat_panel_width: settings.chat_panel_width,
            chat_resizing: false,
            chat_restart_token: 0,
            chat_session_id: claude_agent::new_session_id(),
            chat_sessions: Vec::new(),
            chat_sessions_open: false,
            chat_view_menu_open: false,
            chat_actions_open: false,
            chat_thinking_enabled: false,
            chat_shell_access_enabled: false,
            // Matches the behavior this app has always had before mode
            // selection existed at all — every prior end-to-end test was
            // built and verified against "ask for every Edit/Write", so
            // that stays the default rather than silently becoming more
            // permissive under this change.
            chat_permission_mode: PermissionMode::Manual,
            chat: ChatThread::default(),
            window_width: 1280.0,
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
            pending_discard: None,
            problems_panel_open: false,
            edit_settled_at: None,
            pending_edits: Vec::new(),
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
            show_hidden_files: settings.show_hidden_files,
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
    /// Clicked the status bar's Problems indicator — see
    /// `State::problems_panel_open`.
    ToggleProblemsPanel,
    /// Clicked a diagnostic row in the Problems dock panel — opens (or
    /// focuses) `path` and moves the cursor to the diagnostic's start
    /// position, same as clicking a location in any other editor's problems
    /// list.
    OpenDiagnosticAt(PathBuf, CursorPos),
    /// Opens the panel if closed (docked), closes it if it's presented any
    /// other way — mirrors the title-bar button's old `ToggleAssist`
    /// behavior, and the mockup's own `toggleChat`.
    ChatToggle,
    /// Opens or closes the header's "View" popup — see
    /// `State::chat_view_menu_open`'s own doc comment.
    ChatToggleViewMenu,
    ChatDock,
    ChatCollapse,
    /// "Open as tab" — see `TabKey::Chat`/`chat_is_active`.
    ChatOpenTab,
    /// Leaves tab presentation and returns to the docked panel.
    ChatDockFromTab,
    /// The chat tab's own × — unlike `ChatDockFromTab`, this closes the
    /// panel outright rather than re-docking it (matches the mockup: its
    /// tab-bar close handler only clears `chatTabOpen`, leaving `chatMode`
    /// wherever it already was — `Closed`, since opening as a tab set it
    /// there).
    ChatCloseTab,
    /// Enter (or the input bar's send action) — submits `state.chat.input`
    /// as a new turn and clears the draft.
    ChatSubmit,
    /// An event from the running `claude` subprocess (see `chat_worker`).
    Chat(ClaudeEvent),
    ChatApprovePermission(String),
    ChatDenyPermission(String),
    /// Pressed the chat panel's edge resize handle.
    ChatResizeStarted,
    /// Cursor moved while resizing — carries the cursor's window-space X
    /// position; the new width is `window_width - x` since the handle sits
    /// on the panel's *left* edge (the panel itself is docked to the right).
    ChatResizeDragged(f32),
    ChatResizeEnded,
    /// Starts a genuinely fresh session (new random id — see
    /// `claude_agent::new_session_id`), replacing whatever session was
    /// active. The old one isn't lost: it's still on disk, and will show
    /// up in the session list next time it's opened.
    ChatNewSession,
    /// Switches to an existing session by id, picked from `state.chat_sessions`.
    ChatResumeSession(String),
    /// Opens/closes the session-picker list. Opening kicks off a
    /// background scan (`claude_agent::list_sessions`) — see
    /// `start_loading_chat_sessions`.
    ChatToggleSessions,
    ChatSessionsLoaded(Vec<claude_agent::SessionSummary>),
    /// Switches the permission mode (Manual/Auto-Edit/Plan/Auto) — respawns
    /// the worker (see `State::chat_permission_mode`'s own doc comment),
    /// resuming the same session under the new mode.
    ChatSetPermissionMode(PermissionMode),
    /// Opens a real terminal running `claude` so the human can complete
    /// `/design-login` (or anything else that genuinely needs an
    /// interactive TTY) — nothing headless, this session included, can do
    /// that flow itself. See `launch_terminal_running_claude`.
    ChatLaunchDesignLogin,
    /// Opens or closes the input bar's "+" Actions popup — see
    /// `State::chat_actions_open`'s own doc comment.
    ChatToggleActions,
    /// "Attach file…" — opens a native file dialog rooted at the project;
    /// the chosen file (anywhere on disk) is mentioned by its absolute
    /// path. See `Message::ChatFileDialogResult`.
    ChatAttachFileDialog,
    /// "Mention file from this project…" — same dialog as
    /// `ChatAttachFileDialog`, but the chosen file is mentioned relative to
    /// `State::root` when it's actually inside the project.
    ChatMentionFileDialog,
    /// A file was (or wasn't) chosen from either dialog above — `bool` is
    /// whether to mention it relative to the project root (`true`,
    /// `ChatMentionFileDialog`) or by absolute path (`false`,
    /// `ChatAttachFileDialog`). `None` means the dialog was cancelled.
    ChatFileDialogResult(Option<PathBuf>, bool),
    /// "Switch model…" — sends `/model` (with no argument), which `claude`
    /// answers in place with the current model and how to change it,
    /// rather than DevScribe guessing at available model names itself.
    ChatShowModel,
    /// "Account & usage…" — sends `/usage`.
    ChatShowUsage,
    /// The "Thinking" toggle — flips `State::chat_thinking_enabled` and
    /// sends `/effort high` or `/effort auto` accordingly.
    ChatToggleThinking,
    /// The "Shell Access" toggle — flips
    /// `State::chat_shell_access_enabled`. Unlike `ChatToggleThinking`,
    /// there's no live slash-command for this: `--disallowedTools`/the
    /// permission-hook matcher are spawn-time settings, so this relies on
    /// the field being part of `chat_worker`'s subscription key to take
    /// effect (a respawn, not an in-place command).
    ChatToggleShellAccess,
    /// Multi-line input bar: cursor movement, selection, typing — see
    /// `iced::widget::text_editor`. Plain Enter is intercepted separately
    /// (see `chat_panel::input_bar`'s `key_binding`) to submit instead of
    /// inserting a newline; Shift+Enter falls through to here as normal.
    ChatInputAction(iced::widget::text_editor::Action),
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
    /// The editor's `scrollable` reported a new vertical offset (and its
    /// current viewport height) — stored so `EditorCanvas::draw` can skip
    /// lines outside the visible range, and so Find navigation knows
    /// whether a match is already on-screen.
    EditorScrolled { offset: f32, viewport_height: f32 },
    CaretTick,
    /// Fires only while an edit is pending; runs the deferred per-edit work
    /// once the buffer has been still for `EDIT_SETTLE`.
    EditSettleTick,
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
    ToggleShowHiddenFiles,
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
    /// The window regained focus — re-scans git status (`refresh_changed_files`)
    /// and the file tree (`refresh_tree`). The watcher subscription only
    /// covers the project directory (`.git` itself is in `SKIP_DIRS`), so a
    /// `git commit`/`push`/`checkout` run from an external terminal — which
    /// can move `HEAD` or the upstream ref without touching any watched file
    /// — would otherwise leave the Changes panel and ahead/behind count
    /// stale until the next save. Coming back to the window is the one
    /// moment that kind of external change is actually likely to have
    /// happened, so it's a cheap, natural point to catch up. A no-op before
    /// a project is open, same guard the watcher subscription itself uses.
    WindowFocused,
    /// The window was opened or resized — tracked only for the chat
    /// panel's right-edge resize math; see `State::window_width`.
    WindowResized(f32),
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
    /// Context menu's escape hatch for files `SelectFile` would otherwise
    /// redirect elsewhere (currently just Markdown, opened externally by
    /// default) — opens `path` as a DevScribe tab regardless.
    OpenInEditor(PathBuf),
    /// First click on the context menu's "Delete" row — shows the
    /// confirm/cancel step (`ContextMenu::confirm_delete`) rather than
    /// deleting immediately.
    PromptDeletePath,
    /// The confirm step's "Delete" button — actually removes `path` (file
    /// or, recursively, directory) from disk. Permanent: no OS trash/
    /// recycle-bin integration.
    DeletePath(PathBuf),
    /// The Changes panel's rollback icon — shows `path`'s confirm/cancel
    /// step (`State::pending_discard`) rather than discarding immediately.
    PromptDiscardChange(PathBuf),
    CancelDiscardChange,
    /// The confirm step's "Discard changes" button — restores `path`'s
    /// working-tree content from `HEAD` (or deletes it, if it has no `HEAD`
    /// version), via `devscribe_core::git::Repo::discard_file`. Permanent:
    /// this is a `git checkout -- path`, not covered by the editor's own
    /// undo stack.
    ConfirmDiscardChange(PathBuf),
    /// A click on a changed line's gutter marker — reverts just that line
    /// (or, for `GutterMark::RemovedAbove`, re-inserts the deleted lines
    /// above it) back to its `HEAD` content. One undo step.
    RevertLine { line: usize },
    /// A click on a hunk in the diff view — toggles whether it's checked,
    /// keyed by that hunk's `Hunk::range.start` in `EditorState::hunks`.
    ToggleDiffHunkSelected { path: PathBuf, hunk_id: usize },
    /// The diff view's "Revert selected" button — arms the confirm/cancel
    /// step (`EditorState::pending_hunk_revert`) rather than reverting
    /// immediately.
    PromptRevertSelectedHunks(PathBuf),
    CancelRevertSelectedHunks(PathBuf),
    /// Reverts every hunk checked in `path`'s diff view back to `HEAD`, as
    /// one undo step (see `EditorState::revert_lines`).
    ConfirmRevertSelectedHunks(PathBuf),
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
        Message::ChatToggle => toggle_chat(state),
        Message::ChatToggleViewMenu => state.chat_view_menu_open = !state.chat_view_menu_open,
        Message::ChatDock => {
            leave_chat_tab(state);
            state.chat_mode = ChatMode::Docked;
            state.chat_view_menu_open = false;
            persist_settings(state);
        }
        Message::ChatCollapse => {
            leave_chat_tab(state);
            state.chat_mode = ChatMode::Collapsed;
            state.chat_view_menu_open = false;
            persist_settings(state);
        }
        Message::ChatOpenTab => {
            state.chat_tab_open = true;
            state.chat_mode = ChatMode::Closed;
            state.active_tab = Some(TabKey::Chat);
            state.chat_view_menu_open = false;
            persist_settings(state);
        }
        Message::ChatDockFromTab => {
            leave_chat_tab(state);
            state.chat_mode = ChatMode::Docked;
            state.chat_view_menu_open = false;
            persist_settings(state);
        }
        Message::ChatCloseTab => {
            leave_chat_tab(state);
        }
        Message::ChatInputAction(action) => state.chat.input.perform(action),
        Message::ChatSubmit => submit_chat_prompt(state),
        Message::ChatSetPermissionMode(mode) => state.chat_permission_mode = mode,
        Message::ChatLaunchDesignLogin => {
            if launch_terminal_running_claude() {
                push_toast(state, ToastKind::Success, "Opened a terminal \u{2014} run /design-login there, then retry here.");
            } else {
                push_toast(
                    state,
                    ToastKind::Warning,
                    "Couldn't open a terminal automatically \u{2014} open one yourself, run `claude`, then type /design-login.",
                );
            }
        }
        Message::ChatToggleActions => state.chat_actions_open = !state.chat_actions_open,
        Message::ChatAttachFileDialog => {
            state.chat_actions_open = false;
            let dir = state.root.clone();
            return iced::Task::perform(pick_chat_mention_file(dir), |path| Message::ChatFileDialogResult(path, false));
        }
        Message::ChatMentionFileDialog => {
            state.chat_actions_open = false;
            let dir = state.root.clone();
            return iced::Task::perform(pick_chat_mention_file(dir), |path| Message::ChatFileDialogResult(path, true));
        }
        Message::ChatFileDialogResult(path, relative_to_project) => {
            if let Some(path) = path {
                insert_chat_mention(state, &path, relative_to_project);
            }
        }
        Message::ChatShowModel => {
            state.chat_actions_open = false;
            if state.chat.sender.is_some() {
                send_chat_text(state, "/model".to_string());
            }
        }
        Message::ChatShowUsage => {
            state.chat_actions_open = false;
            if state.chat.sender.is_some() {
                send_chat_text(state, "/usage".to_string());
            }
        }
        Message::ChatToggleThinking => {
            state.chat_thinking_enabled = !state.chat_thinking_enabled;
            state.chat_actions_open = false;
            if state.chat.sender.is_some() {
                let cmd = if state.chat_thinking_enabled { "/effort high" } else { "/effort auto" };
                send_chat_text(state, cmd.to_string());
            }
        }
        Message::ChatToggleShellAccess => {
            state.chat_shell_access_enabled = !state.chat_shell_access_enabled;
            state.chat_actions_open = false;
        }
        Message::Chat(event) => return handle_chat_event(state, event),
        Message::ChatApprovePermission(id) => respond_permission(state, id, true, None),
        Message::ChatDenyPermission(id) => respond_permission(state, id, false, Some("denied by user".to_string())),
        Message::ChatResizeStarted => state.chat_resizing = true,
        Message::ChatResizeDragged(x) => {
            if state.chat_resizing {
                state.chat_panel_width = (state.window_width - x).clamp(CHAT_MIN_WIDTH, CHAT_MAX_WIDTH);
            }
        }
        Message::ChatResizeEnded => {
            state.chat_resizing = false;
            persist_settings(state);
        }
        Message::ChatNewSession => {
            state.chat_session_id = claude_agent::new_session_id();
            state.chat = ChatThread::default();
            state.chat_sessions_open = false;
            state.chat_actions_open = false;
        }
        Message::ChatResumeSession(id) => {
            state.chat_session_id = id;
            state.chat = ChatThread::default();
            state.chat_sessions_open = false;
        }
        Message::ChatToggleSessions => {
            state.chat_sessions_open = !state.chat_sessions_open;
            if state.chat_sessions_open {
                return start_loading_chat_sessions(state);
            }
        }
        Message::ChatSessionsLoaded(sessions) => state.chat_sessions = sessions,
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
        Message::ToggleProblemsPanel => state.problems_panel_open = !state.problems_panel_open,
        Message::OpenDiagnosticAt(path, pos) => {
            open_or_focus_file(state, path.clone());
            if let Some(editor) = find_editor_mut(state, &path) {
                editor.click(pos.line, pos.col, false);
            }
        }
        Message::OpenDiffFor(path) => open_or_focus_diff(state, path),
        Message::ViewWorkingTreeDiff => view_working_tree_diff(state),
        Message::SelectFile(path) => {
            if fs_tree::Lang::from_path(&path) == fs_tree::Lang::Md {
                open_externally(&path);
            } else {
                open_or_focus_file(state, path);
            }
        }
        Message::OpenInEditor(path) => {
            state.ctx_menu = None;
            open_or_focus_file(state, path);
        }
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
                mark_edited(state, &path);

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
                        // The `didChange` for the character that triggered
                        // this is still sitting in the settle queue; send it
                        // before asking, or the server completes against a
                        // buffer it has not seen. Only the notification —
                        // running the whole settle here would put the 30 ms
                        // reparse back on the keystroke path, and on exactly
                        // the keystrokes (`.`, `::`) that need to feel fast.
                        // The reparse and diff stay queued.
                        send_did_change_for(state, &path);
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
                mark_edited(state, &path);
            }
        }
        Message::EditorDelete => {
            if let Some(path) = active_file_path(state) {
                if let Some(editor) = find_editor_mut(state, &path) {
                    editor.delete_forward();
                }
                mark_edited(state, &path);
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
                mark_edited(state, &path);
                return iced::clipboard::write(text);
            }
        }
        Message::EditorUndo => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
                && editor.undo()
            {
                mark_edited(state, &path);
            }
        }
        Message::EditorRedo => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
                && editor.redo()
            {
                mark_edited(state, &path);
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
                mark_edited(state, &path);
            }
        }
        Message::EditorScrolled { offset, viewport_height } => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
            {
                editor.scroll_offset = offset;
                editor.viewport_height = viewport_height;
            }
        }
        Message::CaretTick => state.caret_visible = !state.caret_visible,
        Message::EditSettleTick => {
            if state.edit_settled_at.is_some_and(|at| at.elapsed() >= EDIT_SETTLE) {
                flush_pending_edits(state);
            }
        }
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
                    editor.diagnostics =
                        Rc::new(convert_diagnostics(&editor.document, diagnostics));
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
        Message::ToggleShowHiddenFiles => {
            state.show_hidden_files = !state.show_hidden_files;
            persist_settings(state);
            if !state.root.as_os_str().is_empty() {
                refresh_tree(state);
            }
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
                        editor.diagnostics = Rc::new(Vec::new());
                    }
                }
            }
        }
        Message::WindowUnfocused => {
            if state.save_on_focus_loss {
                save_all_dirty_files(state);
            }
        }
        Message::WindowFocused => {
            if !state.welcome_open {
                refresh_tree(state);
                refresh_changed_files(state);
            }
        }
        Message::WindowResized(width) => state.window_width = width,
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
        Message::FindNext => return find_step(state, 1),
        Message::FindPrev => return find_step(state, -1),
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
        Message::PromptDiscardChange(path) => state.pending_discard = Some(path),
        Message::CancelDiscardChange => state.pending_discard = None,
        Message::ConfirmDiscardChange(path) => discard_change(state, path),
        Message::RevertLine { line } => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
                && editor.revert_line(line)
            {
                recompute_diff_for(state, &path);
            }
        }
        Message::ToggleDiffHunkSelected { path, hunk_id } => {
            if let Some(editor) = find_editor_mut(state, &path)
                && !editor.diff_selected_hunks.remove(&hunk_id)
            {
                editor.diff_selected_hunks.insert(hunk_id);
            }
        }
        Message::PromptRevertSelectedHunks(path) => {
            if let Some(editor) = find_editor_mut(state, &path) {
                editor.pending_hunk_revert = true;
            }
        }
        Message::CancelRevertSelectedHunks(path) => {
            if let Some(editor) = find_editor_mut(state, &path) {
                editor.pending_hunk_revert = false;
            }
        }
        Message::ConfirmRevertSelectedHunks(path) => {
            let reverted = find_editor_mut(state, &path).is_some_and(|editor| {
                editor.pending_hunk_revert = false;
                let targets: Vec<usize> = editor
                    .hunks
                    .iter()
                    .filter(|hunk| editor.diff_selected_hunks.contains(&hunk.range.start))
                    .flat_map(|hunk| hunk.marks.iter().map(|(new_line, _)| *new_line))
                    .collect();
                editor.diff_selected_hunks.clear();
                editor.revert_lines(&targets)
            });
            if reverted {
                recompute_diff_for(state, &path);
            }
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
                            // `edit_remove`, not `document.remove`: accepting a
                            // completion is an edit like any other, and its
                            // highlight spans have to slide with it.
                            editor.edit_remove(start..end);
                            editor.cursor = editor.document.line_col(start).into();
                        }
                        editor.insert_text(&text);
                    }
                    mark_edited(state, &path);
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
        PaletteAction::ChatToggle => toggle_chat(state),
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
    // Land any deferred reparse/diff/`didChange` first, so what gets written
    // and what the diff and the language server think was written agree.
    flush_pending_edits(state);
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
    flush_pending_edits(state);
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
        show_hidden_files: state.show_hidden_files,
        problem_lens_enabled: state.problem_lens_enabled,
        save_on_focus_loss: state.save_on_focus_loss,
        lsp_enabled: state.lsp_enabled,
        chat_mode: state.chat_mode,
        chat_panel_width: state.chat_panel_width,
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

/// A stable id for the active file's editor scroll area, so `find_step` can
/// scroll a Find match into view. Only one editor pane is ever shown at a
/// time (the active tab's), so a single fixed id is enough — same pattern
/// as `find_query_id`.
pub fn editor_scroll_id() -> iced::widget::Id {
    iced::widget::Id::new("editor-scroll-area")
}

/// A stable id for the chat message-list `scrollable` — Docked and Tab
/// presentation are mutually exclusive (see `chat_panel.rs`'s own module
/// doc comment), so at most one of these is ever actually on screen at a
/// time, making one global id safe to share between them. Used by
/// `handle_chat_event` to snap to the latest message whenever a session
/// (re)connects — a brand-new spawn, a resumed one replaying its saved
/// history, or a respawn from switching modes.
pub fn chat_scroll_id() -> iced::widget::Id {
    iced::widget::Id::new("chat-scroll-area")
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
    state.tree = fs_tree::walk(&state.root, state.show_hidden_files);
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
    editor.resync_after_edit();

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
            TabKey::Search | TabKey::Chat => {}
        }
    }
}

/// Assumed editor viewport height (px) when `EditorState::viewport_height`
/// hasn't been reported yet (no `on_scroll` has fired for this tab) — used
/// so the very first `FindNext`/`FindPrev` still scrolls sensibly instead of
/// treating an unknown height as "nothing is visible".
const ASSUMED_VIEWPORT_HEIGHT: f32 = 400.0;

/// Moves the active file tab's find selection by `delta` (wrapping), moves
/// the cursor to the newly-current match, and — if that match isn't already
/// within the visible scroll range — scrolls it into view, centered in the
/// viewport.
fn find_step(state: &mut State, delta: i32) -> iced::Task<Message> {
    let Some(path) = active_file_path(state) else {
        return iced::Task::none();
    };
    let font_size = state.editor_font_size;
    let Some(editor) = find_editor_mut(state, &path) else {
        return iced::Task::none();
    };
    let Some(find) = editor.find.as_mut() else {
        return iced::Task::none();
    };
    if find.matches.is_empty() {
        return iced::Task::none();
    }
    let len = find.matches.len() as i32;
    find.current = (find.current as i32 + delta).rem_euclid(len) as usize;
    let target = find.matches[find.current];
    let cursor: CursorPos = editor.document.line_col(target.start).into();
    editor.cursor = cursor;
    editor.selection_anchor = None;

    let line_height = editor_canvas::line_height_px(font_size);
    let line_top = editor_canvas::line_top(cursor.line, font_size);
    let line_bottom = line_top + line_height;
    let viewport_height = if editor.viewport_height > 0.0 {
        editor.viewport_height
    } else {
        ASSUMED_VIEWPORT_HEIGHT
    };
    let visible_top = editor.scroll_offset;
    let visible_bottom = visible_top + viewport_height;

    if line_top >= visible_top && line_bottom <= visible_bottom {
        return iced::Task::none();
    }

    let target_offset = (line_top - viewport_height / 2.0).max(0.0);
    iced::widget::operation::scroll_to(
        editor_scroll_id(),
        iced::widget::scrollable::AbsoluteOffset { x: 0.0, y: target_offset },
    )
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
        action: PaletteAction::ChatToggle,
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

/// Rebuilds `path`'s open `EditorState` from its on-disk content, the same
/// way opening the file fresh would (`open_or_focus_file`) — highlights/undo
/// history/etc. all come along consistently rather than being patched
/// piecemeal. A no-op if `path` isn't open as a tab or can't be read.
/// Unconditional: callers that must not clobber in-progress local edits
/// (see `reload_editor_from_disk`) check `is_dirty()` themselves first.
fn rebuild_editor_from_disk(state: &mut State, path: &Path) {
    if find_editor(state, path).is_none() {
        return;
    }
    let Ok(document) = Document::open(path) else {
        return;
    };
    if let Some(editor) = find_editor_mut(state, path) {
        *editor = EditorState::new(document, path.to_path_buf());
    }
    send_did_change_for(state, path);
    recompute_diff_for(state, path);
}

/// Reloads an open, *unmodified* buffer's content from disk after
/// `Message::FilesChanged` reports an external change to `path` — a dirty
/// buffer is left alone rather than clobbering in-progress local edits;
/// the git-changes panel and the tab's own modified indicator already make
/// that divergence visible without DevScribe silently picking a side.
fn reload_editor_from_disk(state: &mut State, path: &Path) {
    if find_editor(state, path).is_some_and(|e| e.document.is_dirty()) {
        return;
    }
    rebuild_editor_from_disk(state, path);
}

/// The Changes panel's "Discard changes" confirm step — restores `path`'s
/// working-tree content via `Repo::discard_file`, then brings any open tab
/// and the diff/Changes views back in step with the result. Unlike
/// `reload_editor_from_disk`, this always overwrites an open buffer
/// regardless of its dirty state: discarding is the explicit, destructive
/// action the user just confirmed, not an incidental external change to
/// tiptoe around.
fn discard_change(state: &mut State, path: PathBuf) {
    state.pending_discard = None;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let Some(kind) = state.changed_files.iter().find(|e| e.path == path).map(|e| e.kind) else {
        return;
    };
    let result = state.repo.as_ref().map(|repo| repo.discard_file(&path, kind));
    match result {
        Some(Ok(())) => {
            if path.exists() {
                rebuild_editor_from_disk(state, &path);
            } else {
                let key = TabKey::File(path.clone());
                if state.open_tabs.iter().any(|t| t.key() == key) {
                    close_tab(state, &key);
                }
            }
            refresh_changed_files(state);
            push_flash(state, format!("REVERTED // {}", name.to_uppercase()));
        }
        Some(Err(err)) => push_toast(state, ToastKind::Error, format!("Couldn't revert {name}: {err}")),
        None => {}
    }
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
/// `SelectFile`, `SearchResultSelected`, `OpenInEditor`, and the palette's
/// file-open entries — this is what makes opening a second file additive
/// instead of a replace.
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

/// Hands `path` to the OS's default application for it instead of opening a
/// DevScribe tab — used for Markdown files clicked in the sidebar, since
/// they're more often read/reviewed than edited in-place. `OpenInEditor`
/// (the context menu's escape hatch) still goes through `open_or_focus_file`
/// directly.
fn open_externally(path: &Path) {
    if let Err(err) = opener::open(path) {
        crate::logging::error(format!("failed to open {} externally: {err}", path.display()));
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
/// How long the buffer must sit still before the expensive per-edit work
/// runs: a full tree-sitter reparse, a `HEAD` blob read plus whole-file
/// diff, and a full-text LSP `didChange`.
///
/// Measured on an 850-line Rust file in a debug build, those cost ~33 ms,
/// ~3.4 ms and a fresh round of `rust-analyzer` analysis *per keystroke*
/// respectively. Doing that work per character is what made typing lag, and
/// the `didChange` storm is what pushed the load past this app and onto the
/// machine. Short enough to feel immediate, long enough that a burst of
/// typing collapses into one pass.
const EDIT_SETTLE: Duration = Duration::from_millis(90);

/// Records that `path`'s buffer changed, arming the settle timer. The
/// expensive follow-up work happens in `flush_pending_edits`.
fn mark_edited(state: &mut State, path: &Path) {
    state.edit_settled_at = Some(Instant::now());
    if !state.pending_edits.iter().any(|p| p == path) {
        state.pending_edits.push(path.to_path_buf());
    }
}

/// Runs the deferred per-edit work now, for every buffer waiting on it.
///
/// Called by `EditSettleTick` once typing stops, and directly by anything
/// that needs a buffer's derived state to be current *before* it acts —
/// notably an LSP completion request, which would otherwise be answered
/// against a document the server has not been told about yet.
fn flush_pending_edits(state: &mut State) {
    state.edit_settled_at = None;
    for path in std::mem::take(&mut state.pending_edits) {
        if let Some(editor) = find_editor_mut(state, &path) {
            editor.reparse_now();
        }
        send_did_change_for(state, &path);
        recompute_diff_for(state, &path);
    }
}

fn recompute_diff_for(state: &mut State, path: &Path) {
    let Some((current_text, line_count)) =
        find_editor(state, path).map(|e| (e.document.text().to_string(), e.document.line_count()))
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

    // Sized off the buffer's own `line_count()` (which, unlike `diff_lines`'
    // `new_line` indices, counts the trailing empty line `ropey` reports for
    // a file ending in a newline) so the gutter can always index it safely
    // by buffer line number without a bounds check.
    let (gutter_marks, hunks) = match &status {
        DiffStatus::Changed(lines) => (
            Rc::new(devscribe_core::diff::gutter_marks(lines, line_count)),
            devscribe_core::diff::hunks(lines, line_count),
        ),
        _ => (Rc::new(Vec::new()), Vec::new()),
    };

    if let Some(editor) = find_editor_mut(state, path) {
        editor.diff = status;
        editor.gutter_marks = gutter_marks;
        editor.hunks = Rc::new(hunks);
        // A hunk id (`Hunk::range.start`) only means the same change while
        // `hunks` is unchanged — the diff/buffer just moved, so any
        // selection made against the old grouping no longer applies.
        editor.diff_selected_hunks.clear();
        editor.pending_hunk_revert = false;
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
/// both. Run after actions known to change the working tree (saving a file,
/// discarding/reverting a change), after a `FilesChanged` watcher event, and
/// on `WindowFocused` — the watcher itself never fires for a `.git`-only
/// change (a commit, push, or branch switch that doesn't touch a tracked
/// file), so regaining focus is what catches those up.
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
fn reset_project_scoped_state(state: &mut State) {
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

/// Shared by the title-bar button, `⌘I`, and the command palette entry.
/// Turns the panel fully off (both the docked/collapsed presentation *and*
/// tab presentation — see `chat_is_active`) if it's on in any form, else
/// opens it as a tab — the default presentation for a freshly opened
/// session (Docked/Collapsed are reached afterward via the "View" popup).
fn toggle_chat(state: &mut State) {
    if chat_is_active(state) {
        state.chat_mode = ChatMode::Closed;
        state.chat_tab_open = false;
    } else {
        state.chat_tab_open = true;
        state.chat_mode = ChatMode::Closed;
        state.active_tab = Some(TabKey::Chat);
    }
    persist_settings(state);
}

/// Clears `chat_tab_open` and, if the chat tab was the active one,
/// refocuses whatever's now the first open tab instead of one that no
/// longer exists — the same correction `toggle_chat`'s own doc comment
/// describes, shared by every "View" menu destination
/// (`ChatDock`/`ChatCollapse`/`ChatDockFromTab`/`ChatCloseTab`) now that
/// the menu offers all of them uniformly from every view, tab included.
fn leave_chat_tab(state: &mut State) {
    state.chat_tab_open = false;
    if state.active_tab == Some(TabKey::Chat) {
        state.active_tab = state.open_tabs.first().map(|t| t.key());
    }
}

/// Sends the chat input bar's current draft as a new turn: pushes an
/// `Operator` transcript entry immediately (the wire protocol doesn't echo
/// the user's own message back in any way worth waiting for) and clears
/// the draft. A no-op with nothing to send, or with no live session yet
/// (worker still starting, or `claude` unavailable).
fn submit_chat_prompt(state: &mut State) {
    let text = state.chat.input.text().trim().to_string();
    if text.is_empty() || state.chat.sender.is_none() {
        return;
    }
    state.chat.input = iced::widget::text_editor::Content::new();
    send_chat_text(state, text);
}

/// Pushes `text` as an `Operator` transcript entry and forwards it to the
/// running session as a new turn — the shared core of `submit_chat_prompt`
/// (the input bar's own free-text submissions) and the Actions popup's
/// built-in `claude` slash commands (`/model`, `/usage`, `/effort ...`).
/// Those are just prompts like any other from the wire protocol's
/// perspective (see `devscribe_core::claude_agent`): `claude` recognizes
/// and answers them itself without invoking the model — confirmed against
/// the real CLI, a `/model`/`/usage`/`/effort` prompt comes back with
/// `num_turns: 0`, `total_cost_usd: 0`, and a synthetic assistant reply
/// that `handle_chat_event` already renders like any other `AssistantText`.
/// Callers with no live `sender` (e.g. the Actions popup's own callers
/// check first) simply shouldn't call this — there's nothing to forward to.
fn send_chat_text(state: &mut State, text: String) {
    state.chat.messages.push(ChatMessage::Operator(text.clone()));
    if let Some(sender) = state.chat.sender.as_mut() {
        let _ = sender.try_send(ClaudeCommand::SendPrompt(text));
    }
}

async fn pick_chat_mention_file(dir: PathBuf) -> Option<PathBuf> {
    rfd::AsyncFileDialog::new().set_directory(dir).pick_file().await.map(|handle| handle.path().to_path_buf())
}

/// Appends `@<path>` to the chat draft (with a leading space if the draft
/// isn't already empty/whitespace-terminated) — `@`-prefixed paths are
/// `claude`'s own built-in file-reference syntax, confirmed against the
/// real CLI: a prompt containing `@some/file` gets that file's content
/// folded into context automatically, no `Read` tool call needed.
/// `relative_to_project` writes `path` relative to `state.root` when it
/// actually is inside the project (falling back to the absolute path
/// otherwise, e.g. a file `ChatAttachFileDialog` picked from elsewhere on
/// disk).
fn insert_chat_mention(state: &mut State, path: &Path, relative_to_project: bool) {
    let shown = if relative_to_project {
        path.strip_prefix(&state.root).unwrap_or(path).to_string_lossy().into_owned()
    } else {
        path.to_string_lossy().into_owned()
    };
    let existing = state.chat.input.text();
    let needs_space = !existing.is_empty() && !existing.ends_with(char::is_whitespace);
    let mut insertion = String::new();
    if needs_space {
        insertion.push(' ');
    }
    insertion.push('@');
    insertion.push_str(&shown);
    insertion.push(' ');
    state.chat.input.perform(iced::widget::text_editor::Action::Move(iced::widget::text_editor::Motion::DocumentEnd));
    state.chat.input.perform(iced::widget::text_editor::Action::Edit(iced::widget::text_editor::Edit::Paste(std::sync::Arc::new(insertion))));
}

/// Records the human's decision on `state.chat`'s transcript and forwards
/// it to the running session so the blocked permission-hook connection
/// (see `devscribe_core::claude_agent`) can finally answer and let
/// `claude`'s tool call proceed or fail.
fn respond_permission(state: &mut State, id: String, approve: bool, reason: Option<String>) {
    if let Some(tool) = state.chat.find_tool_mut(&id) {
        tool.permission = Some(if approve { PermissionState::Approved } else { PermissionState::Denied });
    }
    if let Some(sender) = state.chat.sender.as_mut() {
        let _ = sender.try_send(ClaudeCommand::RespondPermission { id, approve, reason });
    }
}

/// Applies one event from the running `claude` subprocess to `state.chat`.
/// `Ready` resets the whole thread rather than just recording the new
/// sender: it fires once per subprocess spawn (a fresh project, or the
/// panel re-opening after being fully closed — see `chat_is_active`), and
/// this pass doesn't resume a previous session (no `--resume`/`--session-id`
/// yet), so a prior transcript's messages describe a conversation the new
/// process has no memory of. Leaving them on screen would be misleading,
/// not just stale.
fn handle_chat_event(state: &mut State, event: ClaudeEvent) -> iced::Task<Message> {
    match event {
        // Always the first event from a freshly (re)spawned worker — see
        // its own doc comment. Clearing here, rather than on `Ready`,
        // matters specifically for a resume: history-replay events land
        // *before* `Ready` (the worker only sends `Ready` once the live
        // process is actually up), so clearing on `Ready` would wipe out
        // the very history it just replayed.
        ClaudeEvent::SessionStarting => state.chat = ChatThread::default(),
        ClaudeEvent::Ready(sender) => {
            state.chat.sender = Some(sender);
            state.chat.status = ChatStatus::Ready;
            // Whatever just got replayed (a resumed session's full saved
            // history) or didn't (a brand-new, still-empty one) is now all
            // in `state.chat.messages` — jump to the latest message rather
            // than leaving a resumed conversation scrolled to its start.
            return iced::widget::operation::snap_to_end(chat_scroll_id());
        }
        ClaudeEvent::SessionInit { session_id, model } => {
            state.chat.session_id = Some(session_id);
            state.chat.model = Some(model);
        }
        ClaudeEvent::AssistantText(text) => match state.chat.messages.last_mut() {
            // Finalize the bubble the deltas were building rather than
            // pushing a duplicate — see `ChatMessage::Assistant`'s own doc
            // comment. `text` here is authoritative, so it replaces
            // whatever was accumulated (a safety net against any drift).
            Some(ChatMessage::Assistant { text: existing, streaming }) if *streaming => {
                *existing = text;
                *streaming = false;
            }
            _ => state.chat.messages.push(ChatMessage::Assistant { text, streaming: false }),
        },
        ClaudeEvent::AssistantTextDelta(chunk) => match state.chat.messages.last_mut() {
            Some(ChatMessage::Assistant { text, streaming: true }) => text.push_str(&chunk),
            _ => state.chat.messages.push(ChatMessage::Assistant { text: chunk, streaming: true }),
        },
        ClaudeEvent::OperatorText(text) => state.chat.messages.push(ChatMessage::Operator(text)),
        ClaudeEvent::ToolUseStarted { id, name, input } => {
            state.chat.messages.push(ChatMessage::Tool(ToolActivity { id, name, input, permission: None, result: None }));
        }
        ClaudeEvent::ToolResult { id, is_error, result } => {
            if let Some(tool) = state.chat.find_tool_mut(&id) {
                tool.result = Some(ToolActivityResult { is_error, result });
            }
        }
        ClaudeEvent::PermissionRequest { id, tool_name, tool_input } => {
            if let Some(tool) = state.chat.find_tool_mut(&id) {
                tool.permission = Some(PermissionState::Pending);
            } else {
                // Defensive: every observed real run has `ToolUseStarted`
                // arrive before the matching `PermissionRequest` for the
                // same id, but don't silently drop a real pending
                // permission on the floor if that ordering ever surprises.
                state.chat.messages.push(ChatMessage::Tool(ToolActivity {
                    id,
                    name: tool_name,
                    input: tool_input,
                    permission: Some(PermissionState::Pending),
                    result: None,
                }));
            }
        }
        ClaudeEvent::TurnResult { cost_usd, input_tokens, output_tokens } => {
            state.chat.cost_usd += cost_usd;
            state.chat.input_tokens = input_tokens;
            state.chat.output_tokens = output_tokens;
        }
        ClaudeEvent::Unavailable(reason) => {
            state.chat.status = ChatStatus::Unavailable(reason);
            state.chat.sender = None;
        }
    }
    iced::Task::none()
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

/// Scans `~/.claude/projects/...` for this project's past sessions on its
/// own OS thread (`claude_agent::list_sessions` does real filesystem
/// I/O — a directory listing plus reading a chunk of each transcript for
/// its title), same `iced_runtime::task::blocking` vehicle as
/// `start_server_install`, so opening the session picker can never stall
/// the UI even on a project with a long chat history.
fn start_loading_chat_sessions(state: &State) -> iced::Task<Message> {
    let root = state.root.clone();
    iced_runtime::task::blocking(move |mut sender| {
        let sessions = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| claude_agent::list_sessions(&root)))
            .unwrap_or_default();
        let _ = sender.try_send(sessions);
    })
    .map(Message::ChatSessionsLoaded)
}

/// Best-effort: tries a handful of common Linux terminal emulators in
/// turn, launching each with bare `claude` as its command, stopping at the
/// first one that actually spawns. There's no portable "open the user's
/// terminal" API to call instead — every desktop environment ships (or
/// symlinks) a different one, hence the list rather than one guess.
/// `Command::spawn` here is a detached, fire-and-forget launch (the new
/// terminal process runs independently of DevScribe) — quick to call
/// directly from a message handler, no `iced_runtime::task::blocking`
/// needed the way an actually-blocking operation would.
fn launch_terminal_running_claude() -> bool {
    const CANDIDATES: &[(&str, &[&str])] = &[
        ("x-terminal-emulator", &["-e", "claude"]),
        ("gnome-terminal", &["--", "claude"]),
        ("konsole", &["-e", "claude"]),
        ("xfce4-terminal", &["-e", "claude"]),
        ("alacritty", &["-e", "claude"]),
        ("kitty", &["claude"]),
        ("xterm", &["-e", "claude"]),
    ];
    CANDIDATES.iter().any(|(terminal, args)| std::process::Command::new(terminal).args(*args).spawn().is_ok())
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

/// Keyed on `(root, chat_restart_token)`, mirroring `lsp_worker`'s
/// `(root, language, lsp_restart_token)` keying: a project switch tears
/// down and respawns the session automatically, and bumping the restart
/// token (not currently wired to any UI — reserved for a future "new
/// thread" action) forces a respawn on the same project. Binary resolution
/// and `devscribe_exe` (needed so the generated permission hook re-invokes
/// *this* binary) happen inside the async body, same as `lsp_worker` does
/// for its own binary, so the main thread never blocks either.
/// Keyed on `(root, session_id, permission_mode, allow_bash,
/// chat_restart_token)`: a project switch, picking a different session
/// (`Message::ChatNewSession`/`ChatResumeSession`), switching modes
/// (`Message::ChatSetPermissionMode`), or flipping shell access
/// (`Message::ChatToggleShellAccess`) all change the key, so
/// `subscription()` tears down and respawns automatically, same as
/// `lsp_worker`.
fn chat_worker(
    (root, session_id, mode, allow_bash, _token): &(PathBuf, String, PermissionMode, bool, u64),
) -> impl iced::futures::Stream<Item = ClaudeEvent> + use<> {
    let root = root.clone();
    let session_id = session_id.clone();
    let mode = *mode;
    let allow_bash = *allow_bash;
    iced::stream::channel(32, async move |mut output| {
        use iced::futures::SinkExt as _;
        // First, always — see `ClaudeEvent::SessionStarting`'s own doc.
        let _ = output.send(ClaudeEvent::SessionStarting).await;

        if !crate::server_install::which_binary("claude") {
            let _ = output
                .send(ClaudeEvent::Unavailable(
                    "claude CLI not found on PATH — install: https://claude.ai/download".to_string(),
                ))
                .await;
            return;
        }
        let devscribe_exe = match std::env::current_exe() {
            Ok(path) => path,
            Err(err) => {
                let _ = output.send(ClaudeEvent::Unavailable(format!("couldn't resolve devscribe's own path: {err}"))).await;
                return;
            }
        };

        // Whether this id already has a transcript is the sole signal for
        // new-vs-resume (see `claude_agent::session_exists`) — so e.g.
        // reopening the panel after closing it (same id, no explicit
        // "new"/"resume" click in between) naturally becomes a resume, with
        // nothing else needing to track that it should.
        let resume = claude_agent::session_exists(&root, &session_id);
        if resume {
            for event in claude_agent::load_session_history(&root, &session_id) {
                if output.send(event).await.is_err() {
                    return;
                }
            }
        }
        let options = claude_agent::SessionOptions { session_id, resume, mode, allow_bash };
        claude_agent::run(root, PathBuf::from("claude"), devscribe_exe, options, output).await;
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
                return Message::ChatToggle;
            }
            if c.eq_ignore_ascii_case("u") {
                return Message::ChatAttachFileDialog;
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

/// Maps `iced::window::events()` to `Message::WindowUnfocused`/`WindowFocused`/
/// `WindowResized`, the variants anything here cares about — every other
/// window event (move, redraw…) is a no-op.
fn window_events((_id, event): (iced::window::Id, iced::window::Event)) -> Message {
    match event {
        iced::window::Event::Unfocused => Message::WindowUnfocused,
        iced::window::Event::Focused => Message::WindowFocused,
        iced::window::Event::Opened { size, .. } | iced::window::Event::Resized(size) => Message::WindowResized(size.width),
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

/// Same window-wide cursor tracking as `sidebar_resize_events`, for the
/// chat panel's own drag handle. Only subscribed while `state.chat_resizing`.
fn chat_resize_events(event: iced::Event, _status: iced::event::Status, _window: iced::window::Id) -> Option<Message> {
    match event {
        iced::Event::Mouse(mouse::Event::CursorMoved { position }) => Some(Message::ChatResizeDragged(position.x)),
        iced::Event::Mouse(mouse::Event::ButtonReleased(_)) => Some(Message::ChatResizeEnded),
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
    // No chat worker with no project open, and none while the panel is
    // fully closed (not even as a tab) — see `chat_is_active`. Torn down
    // and respawned (see `chat_worker`'s own doc comment for exactly when)
    // whenever this becomes true again, the project changes, the target
    // session changes, or `chat_restart_token` is bumped.
    if !state.welcome_open && chat_is_active(state) {
        let key = (
            state.root.clone(),
            state.chat_session_id.clone(),
            state.chat_permission_mode,
            state.chat_shell_access_enabled,
            state.chat_restart_token,
        );
        subs.push(iced::Subscription::run_with(key, chat_worker).map(Message::Chat));
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
    // Same shape, and for the same reason, as the search debounce below:
    // subscribed only while there is actually an edit waiting to settle.
    if state.edit_settled_at.is_some() {
        subs.push(iced::time::every(EDIT_SETTLE / 3).map(|_| Message::EditSettleTick));
    }
    if state.search_query_changed_at.is_some() {
        subs.push(iced::time::every(Duration::from_millis(100)).map(|_| Message::SearchDebounceTick));
    }
    if state.sidebar_resizing {
        subs.push(iced::event::listen_with(sidebar_resize_events));
    }
    if state.chat_resizing {
        subs.push(iced::event::listen_with(chat_resize_events));
    }
    iced::Subscription::batch(subs)
}

#[cfg(test)]
#[path = "tests/state.rs"]
mod tests;
