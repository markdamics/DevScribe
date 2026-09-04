//! Editor-buffer state: open tabs, `EditorState` (cursor/selection/undo/find-replace/
//! LSP wiring), and everything that opens/closes/saves a file tab. Split out of the
//! former monolithic `state.rs` — see `super` for `State`/`Message`/`update()`, which
//! this module's functions are called from but never define themselves.

use super::*;

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

/// One clickable row in the Locations dock panel (`references_panel.rs`) —
/// "Go to Definition" (when the server names more than one candidate, e.g.
/// several trait impls) and "Find All References" both land here rather
/// than each needing their own panel. Char-based (`col`, not a UTF-16
/// offset) and carrying its own `preview` line, same reasoning as
/// `EditorDiagnostic`: computed once up front so the view doesn't
/// re-convert or re-read the target file on every frame.
#[derive(Debug, Clone)]
pub struct LocationEntry {
    pub path: PathBuf,
    pub line: usize,
    pub col: usize,
    /// The target line's own text, trimmed and length-capped — what the
    /// panel row shows next to the `file:line` label.
    pub preview: String,
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

/// One in-flight `$/progress` operation — see `State::lsp_progress`'s own
/// doc comment.
#[derive(Debug, Clone)]
pub struct LspProgressEntry {
    pub title: String,
    pub message: Option<String>,
    pub percentage: Option<u32>,
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

/// Mirrors `LspStatus`'s own shape for `copilot_completion_worker` — no
/// `Installing` variant, since inline completions have no auto-install path
/// (the binary just has to already be on PATH).
#[derive(Debug, Clone, Default)]
pub enum CopilotCompletionStatus {
    #[default]
    Starting,
    Ready,
    Unavailable(String),
    /// Turned off via the settings panel's toggle — see `LspStatus::Disabled`'s
    /// own doc comment for why this is distinct from `Unavailable`.
    Disabled,
}

impl CopilotCompletionStatus {
    /// (status-dot color, label) — see `LspStatus::describe`.
    pub fn describe(&self, p: devscribe_core::theme::Palette) -> (devscribe_core::theme::Rgba, String) {
        match self {
            CopilotCompletionStatus::Starting => (p.text_muted, "starting\u{2026}".to_string()),
            CopilotCompletionStatus::Ready => (p.status_success, "ready".to_string()),
            CopilotCompletionStatus::Unavailable(reason) => (p.status_warning, format!("unavailable ({reason})")),
            CopilotCompletionStatus::Disabled => (p.text_muted, "disabled".to_string()),
        }
    }
}

/// One GitHub Copilot inline ("ghost text") suggestion, shown after the
/// cursor at `at`. Unlike the LSP dot-completion popup's
/// `completions`/`completions_all` pair, this deliberately does *not* try to
/// locally re-narrow itself as the user keeps typing a matching prefix —
/// every qualifying edit or cursor move instead just invalidates it (see
/// `mark_edited`) and a fresh suggestion is requested once typing settles
/// (`maybe_trigger_ghost_completion`, called from the existing
/// `EditSettleTick` debounce). `at` is checked again at every read site
/// (rendering, Tab-to-accept) rather than trusted, the same defensive
/// pattern `LspEvent::Completions` already uses against `completion_anchor`
/// — belt and suspenders against a suggestion for a position the cursor has
/// since left.
#[derive(Debug, Clone)]
pub struct GhostCompletion {
    pub at: CursorPos,
    /// The literal text to insert at `at` if accepted — never assumed to
    /// share a prefix with anything already on screen.
    pub insert_text: String,
    /// The raw `InlineCompletionItem` `copilot_completion` returned, kept
    /// verbatim so accepting it can hand the *exact same* value back via
    /// `CopilotCompletionCommand::Accepted` — the server's own acceptance
    /// telemetry replays whatever `command` came attached to that specific
    /// item, not a value reconstructed from just `insert_text`.
    pub item: serde_json::Value,
}

/// `textDocument/signatureHelp`'s response, kept as-is (the server already
/// computes `active_signature`/`active_parameter` for the position it was
/// asked about) — shown as a small popup above the cursor while typing a
/// call's argument list. `anchor` is where the request was made for, the
/// same staleness guard `completion_anchor` gives `LspEvent::Completions`.
#[derive(Debug, Clone)]
pub struct SignatureHelpState {
    pub signatures: Vec<lsp::SignatureInformation>,
    pub active_signature: usize,
    pub active_parameter: Option<usize>,
    pub anchor: CursorPos,
}

/// `(sort_text, label)` tuple used to order completion items that tie on
/// fuzzy-match relevance — `sort_text` when the server bothered to set one
/// (most rank by kind there: locals before globals, etc.), the label
/// otherwise, with the label always appended second so items sharing one
/// `sort_text` still land in a stable, readable order.
fn completion_sort_key(item: &CompletionItem) -> (&str, &str) {
    (item.sort_text.as_deref().unwrap_or(item.label.as_str()), item.label.as_str())
}

/// `State::diff_view_mode` — how `diff_view.rs` lays out a changed file's
/// lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiffViewMode {
    /// Old and new lines interleaved in one column, git-diff style.
    #[default]
    Unified,
    /// Old on the left, new on the right, aligned row-for-row so a single
    /// shared `scrollable` keeps both sides in sync automatically — see
    /// `diff_view::side_by_side_rows`.
    SideBySide,
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

/// `State::editor_ctx_menu` — where a right-click landed, both in document
/// terms (`line`/`col`, what the menu's actions actually act on) and screen
/// terms (`x`/`y`, where to draw it).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EditorContextMenu {
    pub line: usize,
    pub col: usize,
    pub x: f32,
    pub y: f32,
}

/// `State::rename_prompt` — "Rename Symbol"'s floating input. `line`/`col`
/// are the position the eventual `LspCommand::Rename` request targets (the
/// same ones `EditorContextMenu` had when "Rename Symbol" was clicked);
/// `query` is the text box's live content, seeded from `EditorState::word_at`.
#[derive(Debug, Clone, PartialEq)]
pub struct RenamePrompt {
    pub line: usize,
    pub col: usize,
    pub query: String,
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
    /// The in-file "replace" row's text — kept even while the row is
    /// collapsed (`replace_open == false`) so re-expanding it doesn't lose
    /// what was typed.
    pub replace_query: String,
    /// Whether the replace row (`find_bar.rs`) is expanded below the find
    /// row. Toggled by the chevron button or Ctrl/Cmd+H.
    pub replace_open: bool,
    /// Set while "Replace All" is asking the user to confirm how many
    /// matches it's about to touch — `find_bar.rs` swaps the replace row's
    /// buttons for a "Replace N matches? Yes/No" prompt instead of firing
    /// the replacement immediately, since it's not undoable per-match the
    /// way a single "Replace" is easy to eyeball first.
    pub confirm_replace_all: bool,
    /// Whether the "?" quick-help popover (find_bar.rs) is open.
    pub help_open: bool,
    /// Set by the most recent `find_step` exactly when it wrapped around
    /// the start/end of `matches` — lets `find_bar.rs` show a "(wrapped)"
    /// hint next to the "N of M" counter so hitting the boundary doesn't
    /// read as the search being stuck.
    pub just_wrapped: bool,
}

/// `Tab`-to-next-placeholder tracking for a snippet completion, from the
/// moment it's inserted (`EditorState::begin_snippet`) until every stop has
/// been visited (`advance_snippet`). `stops` are absolute char ranges into
/// the *live* document — kept correct across the one stop currently being
/// edited via the delta computed in `advance_snippet`, not by re-deriving
/// them from the buffer, so a same-length or different-length retype of the
/// current placeholder both leave every later stop pointing at the right
/// place.
#[derive(Debug, Clone)]
struct ActiveSnippet {
    stops: Vec<(usize, usize)>,
    /// Index into `stops` that the next `advance_snippet` call visits.
    next: usize,
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
    /// `.json` files default to the read-only tree view (`json_view.rs`);
    /// this flips a single tab over to the normal editable `code_area` so
    /// the tree view doesn't have to grow its own editing UI. Ignored for
    /// non-JSON files, which never look at it.
    pub json_text_mode: bool,
    /// `Some(_)` for `.md`/`.markdown` files, `None` otherwise — the source
    /// for the read-only preview panel (`markdown_view.rs`). Recomputed at
    /// settle time alongside `tree`/`json`, not on every keystroke; see
    /// `reparse_now`.
    pub markdown: Option<iced::widget::markdown::Content>,
    /// Same idea as `json_text_mode`, one field over: `.md`/`.markdown`
    /// files default to the rendered preview; this flips a single tab back
    /// to the normal editable `code_area`. Ignored for non-Markdown files.
    pub markdown_text_mode: bool,
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
    /// The buffer line whose gutter marker was clicked once and is now
    /// armed, waiting for a confirming second click (`Message::RevertLine`)
    /// — same two-step shape as `pending_hunk_revert`, but for the canvas
    /// gutter's single-line revert rather than the diff view's multi-hunk
    /// one. A click anywhere else, or Escape, disarms it without reverting.
    pub pending_revert_line: Option<usize>,
    /// Set by every edit, cleared by `reparse_now`. The expensive derived
    /// views (tree-sitter spans, the JSON tree) are recomputed once the
    /// buffer settles rather than on every keystroke — see `EDIT_SETTLE`.
    pub needs_reparse: bool,
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
    /// Horizontal scroll offset (px from the left) — the same idea as
    /// `scroll_offset`, one axis over, now that the canvas is sized to the
    /// document's widest line (`max_line_chars`) rather than always filling
    /// the pane, so long lines scroll into view instead of being clipped.
    pub scroll_offset_x: f32,
    /// Width (px) of the horizontal scroll viewport — the `viewport_height`
    /// of this axis.
    pub viewport_width: f32,
    /// The char length of the document's longest line, capped at
    /// `MAX_RENDERED_LINE_CHARS` — sizes the canvas horizontally (see
    /// `editor_canvas::content_width`). Kept only *grow*-accurate between
    /// settles: `resync_after_edit` bumps it the moment a line gets longer
    /// (so typing past the current edge doesn't visibly clip before the
    /// canvas catches up), but never shrinks it — a shrink (e.g. deleting
    /// the longest line) only self-corrects at the next `reparse_now`, same
    /// EDIT_SETTLE lag `highlights`/`tree` already accept. A full rescan on
    /// every keystroke would put an O(line count) pass back on the hot path
    /// this file has otherwise gone to some lengths to keep O(1).
    max_line_chars: usize,
    /// Active completion popup: `None` = closed, `Some(items)` = showing.
    /// This is the currently *displayed* (fuzzy-filtered, re-sorted) subset —
    /// see `completions_all` for the full LSP response it's filtered from.
    pub completions: Option<Vec<CompletionItem>>,
    /// The unfiltered response `completions` was last derived from —
    /// `refilter_completions` re-scores this on every keystroke rather than
    /// re-requesting the server, so typing past the trigger narrows the
    /// popup instead of just closing it. `None` exactly when `completions`
    /// is (see `close_completions`), except transiently when the current
    /// prefix matches nothing — that keeps `completions` closed while still
    /// letting a `Backspace` bring matches back without a fresh request.
    completions_all: Option<Vec<CompletionItem>>,
    /// Keyboard-navigation index into `completions`.
    pub completion_selected: usize,
    /// Cursor position when the completion request was sent, used to discard
    /// stale responses that arrive after the cursor moved elsewhere, and as
    /// the start of the prefix `refilter_completions` matches against.
    pub completion_anchor: CursorPos,
    /// The current GitHub Copilot inline ("ghost text") suggestion, if any —
    /// independent of `completions`/`completions_all` above (that's the LSP
    /// dot-completion popup; this is `copilot_completion`'s single always-
    /// as-you-type suggestion, and the two can't both be showing at once in
    /// practice since either one dismisses the other on the same triggers).
    /// See `GhostCompletion`'s own doc comment for why this doesn't attempt
    /// local prefix-narrowing the way `completions`/`completions_all` do.
    pub ghost_completion: Option<GhostCompletion>,
    /// Active `textDocument/signatureHelp` popup, shown while typing a
    /// call's argument list — independent of `completions`/`ghost_completion`
    /// above; a signature-help popup and the dot-completion popup can be
    /// showing at once (e.g. `foo(bar.|` — inside a call *and* right after a
    /// dot), so this doesn't get cleared by the same triggers that toggle
    /// those, only by `close_completions`'s broader "typing context changed"
    /// triggers (clicks, cursor moves off the anchor line, etc).
    pub signature_help: Option<SignatureHelpState>,
    /// Cursor position the most recent signature-help request was sent for
    /// — same staleness guard `completion_anchor` gives completions.
    pub signature_help_anchor: CursorPos,
    /// `Tab`-to-next-placeholder state for a snippet completion still being
    /// filled in — see `begin_snippet`/`advance_snippet`. `None` once every
    /// stop has been visited, or the moment any non-typing action (a click,
    /// an arrow key, undo, ...) makes the recorded stop positions no longer
    /// trustworthy.
    active_snippet: Option<ActiveSnippet>,
    /// Where the mouse is currently resting (not dragging) over this
    /// editor's canvas, and since when — set by `Message::EditorHoverMove`
    /// (only on an actual cell change, see `editor_canvas::CanvasState`),
    /// cleared by `Message::EditorHoverLeave` and by `clear_hover`. The
    /// subscription's debounce tick (`due_hover_request`) fires the actual
    /// `LspCommand::Hover` request once this has held still for
    /// `HOVER_DWELL`.
    hover_pending: Option<(CursorPos, Instant)>,
    /// The position `hover_pending` was last turned into an actual request
    /// for — keeps the debounce tick from re-sending the same request every
    /// tick while a reply is still in flight, and lets `apply_hover_response`
    /// discard a reply for a position the mouse has since left.
    hover_requested_for: Option<CursorPos>,
    /// The resolved tooltip and the position it's for — rendered by
    /// `hover_popup.rs`. Distinct from `hover_pending`: this only changes
    /// once an actual response lands (or resolves to "nothing to show").
    pub hover: Option<(CursorPos, String)>,
    /// Undo history — snapshots taken just before an edit that starts a new
    /// undo step (see `record_undo_boundary`). Consecutive same-kind edits
    /// (typing character after character, backspacing run after run)
    /// coalesce into whatever's already on top instead of pushing a new
    /// entry, so `Ctrl+Z` undoes a whole word/paste/deletion at a time
    /// rather than one keystroke.
    pub undo_stack: Vec<UndoSnapshot>,
    /// Snapshots popped off `undo_stack` by `undo()`, replayed by `redo()`.
    /// Cleared on every new edit, same as any other editor's redo stack.
    pub redo_stack: Vec<UndoSnapshot>,
    /// The kind of the most recent edit, for `record_undo_boundary`'s
    /// same-kind coalescing. Reset to `None` by any cursor move or
    /// mouse-driven selection change that isn't itself an edit, so typing
    /// never coalesces across an unrelated click or arrow-key jump.
    last_edit_kind: Option<EditKind>,
    /// The column an unbroken run of Up/Down presses is *trying* to stay in.
    /// Without it, passing through one short line clamps `cursor.col` for
    /// good and the caret walks diagonally down the file instead of straight
    /// — every other editor keeps this "sticky" desired column. `None`
    /// outside such a run: any horizontal move, click, selection or edit
    /// clears it, so the next Up/Down re-seeds from wherever the cursor
    /// actually is.
    goal_col: Option<usize>,
    /// Bumped by every buffer mutation (`edit_insert`/`edit_remove`) and
    /// snapshotted into `UndoSnapshot`, so undo/redo restore *which* revision
    /// the buffer is at rather than only its text. Compared against
    /// `saved_revision` to decide whether the buffer still matches disk.
    revision: u64,
    /// `revision` as of the last successful `save()` (or of the freshly
    /// opened buffer). `revision != saved_revision` is the true dirty test,
    /// and the only thing that can tell "undone back to exactly what's on
    /// disk" apart from "undone past the save point" — see `undo`.
    saved_revision: u64,
}

/// A point in `EditorState::undo_stack`/`redo_stack` — the whole buffer plus
/// enough cursor state to put the user back where they were. Cloning
/// `Document` clones its `Rope`, which is cheap (structural sharing), so
/// this is fine to snapshot on every undo boundary rather than diffing.
#[derive(Clone)]
pub struct UndoSnapshot {
    document: Document,
    cursor: CursorPos,
    selection_anchor: Option<CursorPos>,
    /// `EditorState::revision` when this snapshot was taken — restored
    /// alongside the buffer so the dirty flag can be recomputed against
    /// `saved_revision` instead of being carried back inside `document`.
    revision: u64,
}

/// What kind of edit just happened, for `record_undo_boundary`'s
/// same-kind-coalesces-into-one-step logic. `Other` never coalesces, even
/// with itself — each paste/cut is its own undo step.
#[derive(PartialEq, Clone, Copy)]
pub enum EditKind {
    Insert,
    Delete,
    Other,
}

/// Undo history is capped at this many steps per editor (each holding a full
/// document snapshot) so an very long editing session can't grow it without
/// bound.
pub const MAX_UNDO_ENTRIES: usize = 500;

/// Cap on `EditorState::max_line_chars` — the widest column the canvas will
/// ever actually size itself to and let horizontal scroll reach. A
/// minified bundle or generated blob can hold a single line hundreds of
/// thousands of chars long; sizing the canvas to fit one verbatim would
/// make it (and every hit-test/frame built against its width) as
/// pathological as the "no-cap" line-rendering trap `line_text_capped`
/// exists to avoid on the vertical axis. 2000 columns is already far wider
/// than any realistic terminal or monitor — ordinary code, even quite long
/// lines, stays completely unaffected by this; only the truly pathological
/// case is what the cap actually bites.
pub const MAX_RENDERED_LINE_CHARS: usize = 2000;

/// A one-time full scan for `EditorState::new` — every other call site
/// updates `max_line_chars` incrementally (grow-only in the hot path,
/// reconciled at settle) rather than rescanning, per its own doc comment.
/// `line_len_chars` is O(log n) (no text materialized), so this whole pass
/// is O(line count), the same class as `Document::line_count()` itself.
pub fn scan_max_line_chars(document: &Document) -> usize {
    (0..document.line_count())
        .map(|line| document.line_len_chars(line))
        .max()
        .unwrap_or(0)
        .min(MAX_RENDERED_LINE_CHARS)
}

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
        // A one-time full scan, same cost class as the initial highlight
        // parse above — cheap relative to that (just `len_chars()` per line,
        // no text materialized), and there's no settle-debounced moment
        // before the very first `draw()` to defer it to.
        let max_line_chars = scan_max_line_chars(&document);
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
            json_text_mode: false,
            markdown: None,
            markdown_text_mode: false,
            diff: DiffStatus::default(),
            gutter_marks: Rc::new(Vec::new()),
            hunks: Rc::new(Vec::new()),
            diff_selected_hunks: HashSet::new(),
            pending_hunk_revert: false,
            pending_revert_line: None,
            needs_reparse: false,
            find: None,
            scroll_offset: 0.0,
            viewport_height: 0.0,
            scroll_offset_x: 0.0,
            viewport_width: 0.0,
            max_line_chars,
            completions: None,
            completions_all: None,
            completion_selected: 0,
            completion_anchor: CursorPos::default(),
            ghost_completion: None,
            signature_help: None,
            signature_help_anchor: CursorPos::default(),
            active_snippet: None,
            hover_pending: None,
            hover_requested_for: None,
            hover: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            last_edit_kind: None,
            goal_col: None,
            revision: 0,
            saved_revision: 0,
        };
        let text = text.as_deref().unwrap_or("");
        this.reparse_json_with(text);
        this.reparse_markdown_with(text);
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
    pub fn resync_after_edit(&mut self) {
        self.needs_reparse = true;
        // Every mutation ends here, which makes it the one place that has to
        // break an Up/Down run's sticky column — editing is not vertical
        // motion, so the next Up/Down should re-seed from the new cursor.
        self.goal_col = None;
        // Covers the common single-position edits (typing, backspace,
        // paste, ...) generically via wherever the cursor landed. Multi-line
        // block edits (`indent`/`dedent`/`toggle_comment`) additionally call
        // `note_line_length` per touched line directly, since most of those
        // lines aren't the cursor's own.
        self.note_line_length(self.cursor.line);
        self.refind();
    }

    /// Grows `max_line_chars` if `line`'s current length now exceeds it —
    /// see the field's own doc comment for why this is grow-only.
    fn note_line_length(&mut self, line: usize) {
        let len = self.document.line_len_chars(line).min(MAX_RENDERED_LINE_CHARS);
        if len > self.max_line_chars {
            self.max_line_chars = len;
        }
    }

    /// A full, accurate rescan — corrects any staleness `note_line_length`'s
    /// grow-only tracking left behind (most commonly: the document's longest
    /// line just got shorter or was deleted outright). Called from
    /// `reparse_now` (so ordinary typing self-corrects within one
    /// `EDIT_SETTLE`) and from the handful of discrete, infrequent actions —
    /// undo/redo, line revert — where doing this immediately rather than
    /// waiting for settle is cheap enough (they're not per-keystroke) and
    /// removes any lag between the action and the canvas's width.
    fn recompute_max_line_chars(&mut self) {
        self.max_line_chars = scan_max_line_chars(&self.document);
    }

    /// Recomputes the expensive derived views — syntax spans and the JSON
    /// tree — from the current buffer. A no-op unless an edit is pending, so
    /// it is safe to call speculatively (see `flush_pending_edits`).
    ///
    /// Materializes the buffer as a `String` **once** and shares it; the two
    /// used to call `Rope::to_string()` apiece.
    pub fn reparse_now(&mut self) {
        if !self.needs_reparse {
            return;
        }
        self.needs_reparse = false;
        let owned = self.language.map(|_| self.document.text().to_string());
        let text = owned.as_deref().unwrap_or("");
        self.rehighlight_with(text);
        self.reparse_json_with(text);
        self.reparse_markdown_with(text);
        self.tree = self.language.and_then(|lang| outline::parse(lang, text));
        self.recompute_max_line_chars();
    }

    /// The status bar's "Language Mode" picker (roadmap item 9) — overrides
    /// what `language` this buffer highlights/outlines as, independent of
    /// its real extension. Purely a display choice: LSP routing
    /// (`is_lsp_language`/`active_lsp_language`) stays keyed off the file's
    /// actual extension regardless, since the servers this app talks to are
    /// matched by what a file really is on disk, not how it's being shown —
    /// there's no `textDocument/didOpen` re-send here, and there shouldn't
    /// be. Forces an immediate reparse rather than waiting for the next
    /// edit's debounce, so the switch is visible the moment it's picked.
    pub fn set_language(&mut self, language: Option<syntax::Language>) {
        if self.language == language {
            return;
        }
        self.language = language;
        self.needs_reparse = true;
        self.reparse_now();
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

    /// Inserted/deleted line counts for this file's diff against `HEAD` —
    /// the breadcrumb bar's `+N -N` readout. `None` when `diff` isn't
    /// `Changed` (no repo, untracked, or up to date), same as `gutter_marks`
    /// and `hunks` being empty in those cases.
    pub fn diff_counts(&self) -> Option<(usize, usize)> {
        let DiffStatus::Changed(lines) = &self.diff else {
            return None;
        };
        let inserted = lines.iter().filter(|l| l.kind == DiffLineKind::Insert).count();
        let deleted = lines.iter().filter(|l| l.kind == DiffLineKind::Delete).count();
        Some((inserted, deleted))
    }

    /// Inserts at `char_idx`, keeping `highlights` aligned — see
    /// `shift_highlights`.
    fn edit_insert(&mut self, char_idx: usize, text: &str) {
        let at = self.document.text().char_to_byte(char_idx);
        self.document.insert(char_idx, text);
        self.revision += 1;
        self.shift_highlights(at, 0, text.len());
    }

    /// Removes `range` (chars), keeping `highlights` aligned — see
    /// `shift_highlights`.
    pub fn edit_remove(&mut self, range: std::ops::Range<usize>) {
        let rope = self.document.text();
        let at = rope.char_to_byte(range.start);
        let removed = rope.char_to_byte(range.end) - at;
        self.document.remove(range);
        self.revision += 1;
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

    /// Recomputes `markdown` from `text` (the current buffer contents), for
    /// `.md`/`.markdown` files. Mirrors `reparse_json_with`.
    fn reparse_markdown_with(&mut self, text: &str) {
        self.markdown = (self.language == Some(syntax::Language::Markdown))
            .then(|| iced::widget::markdown::Content::parse(text));
    }

    /// `refind_with`, materializing the buffer itself. For the two callers
    /// that change the query without editing the document; every edit goes
    /// through `resync_after_edit` instead, which shares one materialization
    /// across all three recomputations.
    pub fn refind(&mut self) {
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

    /// Replaces the buffer's current find match (`find.matches[find.current]`)
    /// with `find.replace_query`, as one undo step. No-op if there's no
    /// current match — the caller wiring the "Replace" button already
    /// disables it then. `resync_after_edit` reruns `refind_with`
    /// afterward, which is also what makes `find.current` land on "the next
    /// match" for free: replacing this one drops it out of the results, so
    /// every later match's index shifts down by one to fill the gap.
    pub fn replace_current(&mut self) {
        let Some((target, replacement)) = self
            .find
            .as_ref()
            .and_then(|find| find.matches.get(find.current).map(|m| (*m, find.replace_query.clone())))
        else {
            return;
        };
        self.record_undo_boundary(EditKind::Other);
        self.edit_remove(target.start..target.end);
        self.edit_insert(target.start, &replacement);
        self.cursor = self.document.line_col(target.start + replacement.chars().count()).into();
        self.resync_after_edit();
    }

    /// Replaces every current find match with `find.replace_query`, as a
    /// single undo step covering the whole buffer. No-op if there are no
    /// matches.
    pub fn replace_all(&mut self) {
        let Some((replacement, matches)) =
            self.find.as_ref().map(|find| (find.replace_query.clone(), find.matches.clone()))
        else {
            return;
        };
        if matches.is_empty() {
            return;
        }
        self.record_undo_boundary(EditKind::Other);
        // Back-to-front so earlier matches' char offsets stay valid as
        // later replacements shrink or grow the buffer around them.
        for m in matches.iter().rev() {
            self.edit_remove(m.start..m.end);
            self.edit_insert(m.start, &replacement);
        }
        let landing = (matches[0].start + replacement.chars().count()).min(self.document.text().len_chars());
        self.cursor = self.document.line_col(landing).into();
        self.resync_after_edit();
    }

    /// The status bar EOL indicator's "Convert to LF/CRLF" action (roadmap
    /// item 9) — rewrites every line terminator as one undo step. Goes
    /// straight through `Document::convert_eol` rather than
    /// `edit_insert`/`edit_remove` (unlike every other mutation here): a
    /// terminator conversion touches every line at once, so there's no
    /// single char range to describe, and `(line, col)` cursor positions
    /// stay meaningful regardless — line terminators sit *after*
    /// `line_len_chars`, never inside it, so no line's content or column
    /// numbering shifts. `highlights` is cleared rather than shifted (it'd
    /// otherwise point at stale byte offsets until the reparse `needs_reparse`
    /// queues up actually lands) — a one-frame plain-text flash on an
    /// operation that isn't performance-sensitive, in exchange for never
    /// showing a misaligned highlight.
    pub fn convert_eol(&mut self, target: Eol) {
        if self.document.detect_eol() == target {
            return;
        }
        self.record_undo_boundary(EditKind::Other);
        self.document.convert_eol(target);
        self.highlights = Rc::new(Vec::new());
        self.resync_after_edit();
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

    /// The document's longest line, capped — see the field's own doc
    /// comment. `shell.rs` sizes the canvas horizontally from this.
    pub fn max_line_chars(&self) -> usize {
        self.max_line_chars
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
        self.recompute_max_line_chars();
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
        self.recompute_max_line_chars();
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

    /// Applies a language server's `TextEdit`s (UTF-16 line/character
    /// ranges) as a single undo step — the "Rename Symbol" refactor's
    /// per-file half; `state::apply_rename` calls this once per file the
    /// `WorkspaceEdit` touches. `false` (no-op) for an empty edit list, same
    /// shape as `revert_lines`.
    ///
    /// Every range is converted to a char-offset span against the buffer's
    /// *current* text up front, then applied in descending-start order —
    /// same reasoning as `revert_lines`: an edit can shift the char offsets
    /// of anything after it, so processing highest-offset-first keeps every
    /// not-yet-applied span's own already-computed offsets valid.
    pub fn apply_text_edits(&mut self, edits: &[lsp::TextEdit]) -> bool {
        if edits.is_empty() {
            return false;
        }
        let mut spans: Vec<(usize, usize, &str)> = edits
            .iter()
            .map(|edit| {
                let start_line = edit.range.start.line as usize;
                let start_col = utf16_col_to_char_col(&self.document.line_text(start_line), edit.range.start.character as usize);
                let start = self.document.char_index(start_line, start_col);
                let end_line = edit.range.end.line as usize;
                let end_col = utf16_col_to_char_col(&self.document.line_text(end_line), edit.range.end.character as usize);
                let end = self.document.char_index(end_line, end_col);
                (start, end, edit.new_text.as_str())
            })
            .collect();
        spans.sort_by(|a, b| b.0.cmp(&a.0));

        self.record_undo_boundary(EditKind::Other);
        for (start, end, new_text) in spans {
            self.edit_remove(start..end);
            self.edit_insert(start, new_text);
        }
        // Most of these edits land away from wherever the cursor happens to
        // be (often in a file the user isn't even looking at) — not worth
        // relocating it to any one of them, just keeping it in-bounds.
        self.cursor.line = self.cursor.line.min(self.document.line_count().saturating_sub(1));
        self.cursor.col = self.cursor.col.min(self.document.line_len_chars(self.cursor.line));
        self.resync_after_edit();
        self.recompute_max_line_chars();
        true
    }

    /// Pushes a fresh undo snapshot unless this edit is the same `kind` as
    /// the one right before it (typing coalescing into one word, backspaces
    /// coalescing into one run) — always call this *before* mutating
    /// `document`. `Other` never coalesces, so paste/cut are always their
    /// own undo step regardless of what happened right before them.
    fn record_undo_boundary(&mut self, kind: EditKind) {
        let coalesce = kind != EditKind::Other && self.last_edit_kind == Some(kind);
        if !coalesce {
            let entry = self.snapshot();
            self.undo_stack.push(entry);
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
        // Auto-indent: Enter carries over the current line's leading
        // whitespace, up to wherever the cursor actually sits (so pressing
        // Enter *inside* the indent itself — cursor.col less than the
        // indent's own width — doesn't grab whitespace that the split is
        // about to move onto the new line anyway).
        let owned;
        let text = if text == "\n" {
            owned = format!("\n{}", self.current_line_indent());
            owned.as_str()
        } else {
            text
        };
        let idx = self.document.char_index(self.cursor.line, self.cursor.col);
        self.edit_insert(idx, text);
        let new_idx = idx + text.chars().count();
        self.cursor = self.document.line_col(new_idx).into();
        self.resync_after_edit();
    }

    /// A single live keystroke's character — auto-pairing-aware, unlike
    /// `insert_text` (which stays a literal insert for paste and generated
    /// text like Enter/Tab). Three cases, checked in order:
    ///
    /// 1. No selection, and `ch` is a closing bracket or quote sitting right
    ///    where the cursor already is (i.e. one this same auto-pairing just
    ///    inserted): step over it instead of typing a second one, so closing
    ///    a pair never needs deleting the auto-inserted half first.
    /// 2. `ch` opens a pair (`(`, `[`, `{`, or a quote): pair it — wrapping
    ///    the selection if there is one, else inserting both halves with the
    ///    cursor left in between.
    /// 3. Anything else: ordinary typing.
    pub fn type_char(&mut self, ch: char) {
        if self.selection().is_none() {
            let next = self.document.line_char(self.cursor.line, self.cursor.col);
            if next == Some(ch) && is_closer(ch) {
                self.move_cursor(Direction::Right, false);
                return;
            }
        }
        if let Some(closer) = closer_for(ch) {
            self.insert_pair(ch, closer);
            return;
        }
        self.insert_text(&ch.to_string());
    }

    /// The actual insert behind `type_char`'s pairing case: wraps the
    /// selection in `open`/`close` if there is one (keeping it selected,
    /// around the same original text, not the new brackets), otherwise
    /// inserts both chars at the cursor and leaves the cursor between them.
    fn insert_pair(&mut self, open: char, close: char) {
        let Some((start, end)) = self.selection() else {
            // A lone paired insert coalesces with adjacent typing, same as
            // any other single character — see `insert_text`.
            self.record_undo_boundary(EditKind::Insert);
            let idx = self.document.char_index(self.cursor.line, self.cursor.col);
            self.edit_insert(idx, &format!("{open}{close}"));
            self.cursor = self.document.line_col(idx + 1).into();
            self.resync_after_edit();
            return;
        };
        self.record_undo_boundary(EditKind::Other);
        // Insert the closer first: `start < end`, so inserting there first
        // doesn't shift `start` out from under the second insert. Inserting
        // the opener second then pushes both the closer and the wrapped text
        // right by one, landing the closer at `end + 1` — exactly past the
        // (now-shifted) originally-selected text.
        self.edit_insert(end, &close.to_string());
        self.edit_insert(start, &open.to_string());
        let open_line = self.document.line_col(start).0;
        self.selection_anchor = Some(self.document.line_col(start + 1).into());
        self.cursor = self.document.line_col(end + 1).into();
        self.resync_after_edit();
        // `resync_after_edit`'s own check only sees `cursor.line` (the
        // closer's line); a multi-line wrap grows the opener's line too.
        self.note_line_length(open_line);
    }

    /// The run of leading spaces/tabs on `self.cursor.line`, capped at
    /// `self.cursor.col` — see `insert_text`'s Enter handling.
    fn current_line_indent(&self) -> String {
        let indent_col = self.line_indent_col(self.cursor.line).min(self.cursor.col);
        (0..indent_col)
            .filter_map(|col| self.document.line_char(self.cursor.line, col))
            .collect()
    }

    /// How many leading space/tab chars `line` starts with.
    fn line_indent_col(&self, line: usize) -> usize {
        let len = self.document.line_len_chars(line);
        let mut col = 0;
        while col < len && matches!(self.document.line_char(line, col), Some(' ') | Some('\t')) {
            col += 1;
        }
        col
    }

    /// Whether `line` has `token` starting exactly at `col`.
    fn line_starts_with_at(&self, line: usize, col: usize, token: &str) -> bool {
        token.chars().enumerate().all(|(i, c)| self.document.line_char(line, col + i) == Some(c))
    }

    /// The (inclusive) line range a block operation — indent, dedent,
    /// toggle-comment — should act on: every line the selection touches, or
    /// just the cursor's own line with no selection.
    fn selection_line_range(&self) -> (usize, usize) {
        match self.selection() {
            Some((start, end)) => (
                self.document.line_col(start).0,
                self.document.line_col(end.saturating_sub(1).max(start)).0,
            ),
            None => (self.cursor.line, self.cursor.line),
        }
    }

    /// Slides `cursor` and `selection_anchor` when they sit on `line` at or
    /// after `at_col`, following an edit that inserted (`delta > 0`) or
    /// removed (`delta < 0`) `delta.abs()` chars starting at that column.
    ///
    /// Both are stored as `(line, col)`, not absolute char indices, so —
    /// unlike `shift_highlights` — nothing keeps them in step with an edit
    /// automatically. Every block-editing method that mutates a line out
    /// from under a possibly-stored column has to do this by hand.
    fn shift_cols_after_edit(&mut self, line: usize, at_col: usize, delta: i32) {
        let shift = |col: usize| -> usize {
            if col >= at_col {
                (col as i32 + delta).max(at_col as i32) as usize
            } else {
                col
            }
        };
        if self.cursor.line == line {
            self.cursor.col = shift(self.cursor.col);
        }
        if let Some(anchor) = self.selection_anchor.as_mut()
            && anchor.line == line
        {
            anchor.col = shift(anchor.col);
        }
    }

    /// `Tab`. Block-indents every line touched by an active selection
    /// (inserting `tab_size` spaces at each line's start), even a
    /// single-line one; with no selection at all, behaves like plain typing
    /// and inserts `tab_size` spaces at the cursor. Without the selection
    /// branch, Tab used to *replace* whatever was selected with the indent —
    /// a quietly destructive shortcut for something that should only ever
    /// indent.
    pub fn indent(&mut self, tab_size: u8) {
        let indent = " ".repeat(tab_size as usize);
        if self.selection().is_none() {
            self.insert_text(&indent);
            return;
        }
        let (start_line, end_line) = self.selection_line_range();
        self.record_undo_boundary(EditKind::Other);
        for line in start_line..=end_line {
            let at = self.document.char_index(line, 0);
            self.edit_insert(at, &indent);
            self.shift_cols_after_edit(line, 0, tab_size as i32);
            self.note_line_length(line);
        }
        self.resync_after_edit();
    }

    /// `Shift+Tab`. Removes up to one indent level (a leading tab, or up to
    /// `tab_size` leading spaces) from the start of every line the selection
    /// spans, or just the cursor's line with no selection. `false` if no
    /// targeted line had any leading whitespace to remove, in which case
    /// nothing happened — same no-op-must-not-touch-undo shape as
    /// `backspace`/`delete_forward`.
    pub fn dedent(&mut self, tab_size: u8) -> bool {
        let tab_size = tab_size as usize;
        let (start_line, end_line) = self.selection_line_range();
        let removals: Vec<(usize, usize)> = (start_line..=end_line)
            .filter_map(|line| {
                let len = self.document.line_len_chars(line);
                if len > 0 && self.document.line_char(line, 0) == Some('\t') {
                    return Some((line, 1));
                }
                let mut n = 0;
                while n < tab_size && n < len && self.document.line_char(line, n) == Some(' ') {
                    n += 1;
                }
                (n > 0).then_some((line, n))
            })
            .collect();
        if removals.is_empty() {
            return false;
        }
        self.record_undo_boundary(EditKind::Other);
        for (line, n) in removals {
            let start = self.document.char_index(line, 0);
            self.edit_remove(start..start + n);
            self.shift_cols_after_edit(line, 0, -(n as i32));
        }
        self.resync_after_edit();
        true
    }

    /// `Ctrl+/`. Toggles a line comment on every line the selection spans (or
    /// just the cursor's line with no selection): uncomments every targeted
    /// line if all of them (ignoring blanks) are already commented; otherwise
    /// comments whichever targeted lines aren't already, leaving any that
    /// are alone (so a block with mixed comment state converges to fully
    /// commented in one step, rather than double-commenting a line that
    /// already had a marker). `false` if the language has no line-comment
    /// syntax, or every targeted line is blank, in which case nothing
    /// happened.
    ///
    /// Comments are inserted at each line's own indent column rather than a
    /// single column shared by the whole block, so a block with mixed
    /// indentation ends up with mixed comment columns too — simpler than
    /// hunting for the block's minimum indent, at the cost of the comments
    /// not lining up visually the way some editors' does.
    pub fn toggle_comment(&mut self) -> bool {
        let Some(token) = self.language.and_then(syntax::Language::line_comment) else {
            return false;
        };
        let (start_line, end_line) = self.selection_line_range();
        let mut any_content = false;
        let all_commented = (start_line..=end_line).all(|line| {
            let indent = self.line_indent_col(line);
            if indent >= self.document.line_len_chars(line) {
                true
            } else {
                any_content = true;
                self.line_starts_with_at(line, indent, token)
            }
        });
        if !any_content {
            return false;
        }
        self.record_undo_boundary(EditKind::Other);
        for line in start_line..=end_line {
            let len = self.document.line_len_chars(line);
            let indent = self.line_indent_col(line);
            if indent >= len {
                continue;
            }
            if all_commented {
                let mut remove_len = token.chars().count();
                if self.document.line_char(line, indent + remove_len) == Some(' ') {
                    remove_len += 1;
                }
                let start = self.document.char_index(line, indent);
                self.edit_remove(start..start + remove_len);
                self.shift_cols_after_edit(line, indent, -(remove_len as i32));
            } else if !self.line_starts_with_at(line, indent, token) {
                // A block with mixed comment state (some lines already
                // commented, some not) only comments the lines that need it
                // — otherwise an already-commented line in the selection
                // would double up (`// // like this`) instead of being left
                // alone.
                let at = self.document.char_index(line, indent);
                let inserted = format!("{token} ");
                let inserted_len = inserted.chars().count() as i32;
                self.edit_insert(at, &inserted);
                self.shift_cols_after_edit(line, indent, inserted_len);
                self.note_line_length(line);
            }
        }
        self.resync_after_edit();
        true
    }

    /// `Backspace`. `false` if there was nothing to delete, in which case
    /// nothing at all happened — see the guard below.
    pub fn backspace(&mut self) -> bool {
        let idx = self.document.char_index(self.cursor.line, self.cursor.col);
        // An inert keystroke must not touch the undo history.
        // `record_undo_boundary` unconditionally clears the redo stack and
        // can push a snapshot, so backspacing at the very start of the buffer
        // used to throw away a pending redo *and* leave a phantom entry that
        // made the next Ctrl+Z appear to do nothing.
        if idx == 0 && self.selection().is_none() {
            return false;
        }
        self.record_undo_boundary(EditKind::Delete);
        if self.delete_selection() {
            self.resync_after_edit();
            return true;
        }
        // CRLF-atomic, so backspacing at the start of a line takes the whole
        // terminator rather than leaving an orphaned `\r` behind.
        let prev = self.document.prev_char_index(idx);
        self.edit_remove(prev..idx);
        self.cursor = self.document.line_col(prev).into();
        self.resync_after_edit();
        true
    }

    /// `Delete`. `false` if there was nothing to delete — see `backspace`.
    pub fn delete_forward(&mut self) -> bool {
        let idx = self.document.char_index(self.cursor.line, self.cursor.col);
        if idx >= self.document.text().len_chars() && self.selection().is_none() {
            return false;
        }
        self.record_undo_boundary(EditKind::Delete);
        if self.delete_selection() {
            self.resync_after_edit();
            return true;
        }
        let next = self.document.next_char_index(idx);
        self.edit_remove(idx..next);
        self.cursor = self.document.line_col(idx).into();
        self.resync_after_edit();
        true
    }

    /// `Ctrl+Z`. `false` if there was nothing left to undo.
    pub fn undo(&mut self) -> bool {
        let Some(prev) = self.undo_stack.pop() else {
            return false;
        };
        let entry = self.snapshot();
        self.redo_stack.push(entry);
        self.restore(prev);
        true
    }

    /// `Ctrl+Shift+Z` / `Ctrl+Y`. `false` if there was nothing left to redo.
    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo_stack.pop() else {
            return false;
        };
        let entry = self.snapshot();
        self.undo_stack.push(entry);
        self.restore(next);
        true
    }

    /// The current buffer + cursor as an undo/redo entry.
    fn snapshot(&self) -> UndoSnapshot {
        UndoSnapshot {
            document: self.document.clone(),
            cursor: self.cursor,
            selection_anchor: self.selection_anchor,
            revision: self.revision,
        }
    }

    /// Puts `snapshot` back, shared by `undo` and `redo`.
    ///
    /// The dirty flag is recomputed rather than restored: `snapshot.document`
    /// carries whatever `dirty` was set when it was taken, and for any
    /// snapshot predating a save that is `false` — so undoing across a save
    /// point used to leave the tab claiming "no unsaved changes" while the
    /// buffer and the file on disk disagreed, with no modified dot to warn
    /// anyone. Comparing revisions gets it exactly right in both directions:
    /// undoing back to precisely the saved revision really is clean.
    fn restore(&mut self, snapshot: UndoSnapshot) {
        self.document = snapshot.document;
        self.cursor = snapshot.cursor;
        self.selection_anchor = snapshot.selection_anchor;
        self.revision = snapshot.revision;
        self.document.set_dirty(self.revision != self.saved_revision);
        self.last_edit_kind = None;
        self.resync_after_edit();
        // The whole buffer just changed out from under the grow-only
        // tracking `resync_after_edit` did above (via whatever line the
        // cursor happens to land on) — an infrequent, discrete action, so a
        // full accurate rescan here costs nothing worth avoiding.
        self.recompute_max_line_chars();
    }

    /// Writes the buffer to disk, and on success records the revision that
    /// went out so `undo`/`redo` can tell whether a later state matches it.
    /// Errors propagate untouched, same as `Document::save`.
    pub fn save(&mut self) -> std::io::Result<()> {
        self.document.save()?;
        self.saved_revision = self.revision;
        Ok(())
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

        // Only Up/Down carry a sticky column forward; every other direction
        // is the user deliberately choosing a new one.
        if !matches!(dir, Direction::Up | Direction::Down) {
            self.goal_col = None;
        }

        match dir {
            Direction::Left => {
                let idx = self.document.char_index(self.cursor.line, self.cursor.col);
                if idx > 0 {
                    // `prev_char_index`, not `idx - 1`: on a CRLF buffer the
                    // position between `\r` and `\n` is one `char_index`
                    // clamps straight back onto, so stepping one char at a
                    // time leaves the caret stuck at the line boundary.
                    self.cursor = self.document.line_col(self.document.prev_char_index(idx)).into();
                }
            }
            Direction::Right => {
                let idx = self.document.char_index(self.cursor.line, self.cursor.col);
                if idx < self.document.text().len_chars() {
                    self.cursor = self.document.line_col(self.document.next_char_index(idx)).into();
                }
            }
            Direction::Up => {
                if self.cursor.line > 0 {
                    let goal = *self.goal_col.get_or_insert(self.cursor.col);
                    self.cursor.line -= 1;
                    self.cursor.col = goal.min(self.document.line_len_chars(self.cursor.line));
                }
            }
            Direction::Down => {
                if self.cursor.line + 1 < self.document.line_count() {
                    let goal = *self.goal_col.get_or_insert(self.cursor.col);
                    self.cursor.line += 1;
                    self.cursor.col = goal.min(self.document.line_len_chars(self.cursor.line));
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
        self.goal_col = None;
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
        self.goal_col = None;
        // Scanned straight out of the rope via `line_char` (O(log n), no
        // allocation) rather than materializing the line into a `String` and
        // then a `Vec<char>` — on a minified bundle or a generated blob that
        // was megabytes of copying for every double-click, the same trap
        // `line_text_capped` exists to keep the renderer out of.
        let len = self.document.line_len_chars(line);
        if len == 0 {
            self.selection_anchor = None;
            self.cursor = CursorPos { line, col: 0 };
            return;
        }
        let idx = col.min(len - 1);
        let class_at = |i: usize| self.document.line_char(line, i).map(char_class);
        let Some(class) = class_at(idx) else {
            self.selection_anchor = None;
            self.cursor = CursorPos { line, col: idx };
            return;
        };
        let mut start = idx;
        while start > 0 && class_at(start - 1) == Some(class) {
            start -= 1;
        }
        let mut end = idx + 1;
        while end < len && class_at(end) == Some(class) {
            end += 1;
        }
        self.selection_anchor = Some(CursorPos { line, col: start });
        self.cursor = CursorPos { line, col: end };
    }

    /// The identifier word at `(line, col)`, if any — same word-boundary
    /// rule `select_word_at`'s double-click uses (`char_class`), but
    /// read-only and `None` for anything that isn't `CharClass::Word`
    /// (whitespace, punctuation). Used to pre-fill the context menu's
    /// "Rename Symbol" prompt with whatever's actually under the click,
    /// rather than starting it blank.
    pub fn word_at(&self, line: usize, col: usize) -> Option<String> {
        let len = self.document.line_len_chars(line);
        if len == 0 {
            return None;
        }
        let idx = col.min(len - 1);
        let class_at = |i: usize| self.document.line_char(line, i).map(char_class);
        if class_at(idx) != Some(CharClass::Word) {
            return None;
        }
        let mut start = idx;
        while start > 0 && class_at(start - 1) == Some(CharClass::Word) {
            start -= 1;
        }
        let mut end = idx + 1;
        while end < len && class_at(end) == Some(CharClass::Word) {
            end += 1;
        }
        Some((start..end).filter_map(|i| self.document.line_char(line, i)).collect())
    }

    /// Triple-click line selection: the whole line including its trailing
    /// newline (so it visibly covers the line like `select_word_at` covers a
    /// word), except on the file's last line, which has no newline to take.
    pub fn select_line_at(&mut self, line: usize) {
        self.last_edit_kind = None;
        self.goal_col = None;
        self.selection_anchor = Some(CursorPos { line, col: 0 });
        self.cursor = if line + 1 < self.document.line_count() {
            CursorPos { line: line + 1, col: 0 }
        } else {
            CursorPos { line, col: self.document.line_len_chars(line) }
        };
    }

    pub fn select_all(&mut self) {
        self.last_edit_kind = None;
        self.goal_col = None;
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

    /// Whether a completion response is currently being tracked — `true`
    /// exactly when there's a `completions_all` to `refilter_completions`
    /// against, whether or not `completions` itself has anything to show
    /// right now (see `completions_all`'s doc for why those can differ).
    pub fn completions_active(&self) -> bool {
        self.completions_all.is_some()
    }

    /// Closes the completion popup and discards the last LSP response's
    /// full item list — every action that invalidates "what's on screen
    /// still matches what's in the buffer" (a click, a cursor move, an
    /// undo, ...) calls this rather than just clearing `completions`, so a
    /// stale `completions_all` can't outlive the popup it was filtered for.
    pub fn close_completions(&mut self) {
        self.completions = None;
        self.completions_all = None;
        self.completion_selected = 0;
    }

    /// Dismisses the current ghost-text suggestion, if any — called
    /// unconditionally from `mark_edited` (any edit invalidates it) and from
    /// `Message::DismissGhostCompletion` (Escape, while one is showing).
    pub fn close_ghost_completion(&mut self) {
        self.ghost_completion = None;
    }

    /// Stores a freshly arrived LSP completion response and immediately
    /// filters it against whatever's already been typed since the anchor —
    /// a slow response can land after the user kept typing past the
    /// trigger, so this can't just show the raw list. An empty response
    /// closes the popup outright rather than showing nothing to select.
    pub fn set_completions(&mut self, items: Vec<CompletionItem>) {
        if items.is_empty() {
            self.close_completions();
            return;
        }
        self.completions_all = Some(items);
        self.refilter_completions();
    }

    /// Re-scores `completions_all` against whatever's been typed since
    /// `completion_anchor` (fuzzy-matched against each item's
    /// `filter_text`, falling back to its `label`) and replaces
    /// `completions` with the result, best match first. A no-op if there's
    /// no response being tracked (`completions_active()` is `false`) —
    /// callers don't need to check that themselves first.
    ///
    /// Called after every keystroke while a completion session is active,
    /// so typing narrows the popup instead of just closing it (the
    /// pre-fuzzy-filter behavior: `.` opened it, and the very next
    /// character always dismissed it). Closes the session outright
    /// (`close_completions`) if the cursor has left the anchor's line or
    /// backed up past the anchor itself — deleting the trigger character —
    /// but only sets `completions` to `None` (keeping `completions_all`
    /// around) when the cursor is still in range but nothing currently
    /// matches, so a `Backspace` back to a matching prefix can still bring
    /// the popup back without a fresh round trip to the server.
    pub fn refilter_completions(&mut self) {
        let Some(all) = self.completions_all.as_ref() else {
            return;
        };
        let anchor = self.completion_anchor;
        let cursor = self.cursor;
        if cursor.line != anchor.line || cursor.col < anchor.col {
            self.close_completions();
            return;
        }
        if cursor.col == anchor.col {
            let mut items: Vec<CompletionItem> = all.clone();
            items.sort_by(|a, b| completion_sort_key(a).cmp(&completion_sort_key(b)));
            self.completions = Some(items);
            self.completion_selected = 0;
            return;
        }
        let line_text = self.document.line_text(anchor.line);
        let prefix: String = line_text.chars().skip(anchor.col).take(cursor.col - anchor.col).collect();
        let mut scored: Vec<(i32, &CompletionItem)> = all
            .iter()
            .filter_map(|item| {
                let text = item.filter_text.as_deref().unwrap_or(item.label.as_str());
                crate::fuzzy::score(&prefix, text).map(|s| (s, item))
            })
            .collect();
        // Best fuzzy match first; items tied on relevance fall back to the
        // server's own preferred ordering (`sort_text`, when it bothered to
        // set one — most servers rank by kind there, e.g. locals before
        // globals) and finally to the label, so ties are still deterministic
        // even against a server that never sets `sort_text`.
        scored.sort_by(|a, b| {
            b.0.cmp(&a.0).then_with(|| completion_sort_key(a.1).cmp(&completion_sort_key(b.1)))
        });
        self.completions = if scored.is_empty() {
            None
        } else {
            Some(scored.into_iter().map(|(_, item)| item.clone()).collect())
        };
        self.completion_selected = 0;
    }

    /// Whether a snippet `Tab` walk is currently in progress — `Escape`
    /// cancels it before falling through to anything else Escape would
    /// otherwise close, the same priority `completions_active` gets.
    pub fn snippet_active(&self) -> bool {
        self.active_snippet.is_some()
    }

    /// Clears any in-progress snippet `Tab` walk — called from the same
    /// actions that invalidate it as `close_completions` (a click, an arrow
    /// key, undo, ...), since those make the recorded stop positions no
    /// longer trustworthy.
    pub fn close_snippet(&mut self) {
        self.active_snippet = None;
    }

    /// Places the cursor/selection at the char range `(start, end)` — a
    /// zero-width range just moves the cursor there, a non-empty one
    /// selects it (so typing immediately overtypes the placeholder's
    /// default text, the standard snippet-mode gesture).
    fn place_snippet_stop(&mut self, start: usize, end: usize) {
        let start_pos: CursorPos = self.document.line_col(start).into();
        let end_pos: CursorPos = self.document.line_col(end).into();
        if start == end {
            self.selection_anchor = None;
            self.cursor = start_pos;
        } else {
            self.selection_anchor = Some(start_pos);
            self.cursor = end_pos;
        }
    }

    /// Starts `Tab`-to-next-placeholder tracking for a just-inserted
    /// snippet's `stops` (absolute char ranges, already offset by where the
    /// snippet text landed in the document) and selects the first one — a
    /// snippet's very first placeholder is meant to already be selected the
    /// moment it lands, same as every other editor's snippet mode. A
    /// snippet with at most one stop (just a trailing `$0`, or no stops at
    /// all) isn't worth tracking: there's nothing to jump between, so the
    /// cursor is simply placed there and no `Tab` walk begins.
    pub fn begin_snippet(&mut self, stops: Vec<(usize, usize)>) {
        if stops.len() <= 1 {
            if let Some(&(start, end)) = stops.first() {
                self.place_snippet_stop(start, end);
            }
            return;
        }
        self.place_snippet_stop(stops[0].0, stops[0].1);
        self.active_snippet = Some(ActiveSnippet { stops, next: 1 });
    }

    /// `Tab` while a snippet walk is active: jumps to the next stop.
    /// Returns `false` (consuming nothing) if there's no active walk, so
    /// the caller falls through to a normal indent.
    ///
    /// Before jumping, shifts every stop from here on by exactly how much
    /// the stop just being left changed length — comparing the live cursor
    /// (wherever typing left it) against that stop's original end position.
    /// This only stays correct as long as the cursor never moved some other
    /// way since the stop was selected (a click, an arrow key, ...), which
    /// is exactly the set of actions that already call `close_snippet`
    /// elsewhere — so by the time this runs, every edit since the last
    /// `place_snippet_stop` call has been ordinary typing at this position.
    pub fn advance_snippet(&mut self) -> bool {
        let Some(snippet) = self.active_snippet.as_mut() else {
            return false;
        };
        if snippet.next >= snippet.stops.len() {
            self.active_snippet = None;
            return false;
        }
        let prev_end = snippet.stops[snippet.next - 1].1;
        let current_end = self.document.char_index(self.cursor.line, self.cursor.col);
        let delta = current_end as i64 - prev_end as i64;
        if delta != 0 {
            for stop in &mut snippet.stops[snippet.next..] {
                stop.0 = (stop.0 as i64 + delta).max(0) as usize;
                stop.1 = (stop.1 as i64 + delta).max(0) as usize;
            }
        }
        let (start, end) = snippet.stops[snippet.next];
        snippet.next += 1;
        let done = snippet.next >= snippet.stops.len();
        self.place_snippet_stop(start, end);
        if done {
            self.active_snippet = None;
        }
        true
    }

    /// Mouse now resting over `(line, col)` — called from
    /// `Message::EditorHoverMove`, which only fires on an actual cell
    /// change. Restarts the dwell timer and discards whatever tooltip (or
    /// pending request) was showing for the previous position, since it no
    /// longer applies.
    pub fn hover_move(&mut self, line: usize, col: usize) {
        self.hover_pending = Some((CursorPos { line, col }, Instant::now()));
        self.hover_requested_for = None;
        self.hover = None;
    }

    /// The mouse left the canvas, or some other action (a keypress, an
    /// edit, a click, ...) made a pending or shown hover no longer
    /// meaningful.
    pub fn clear_hover(&mut self) {
        self.hover_pending = None;
        self.hover_requested_for = None;
        self.hover = None;
    }

    /// Whether there's currently a rested-on position at all — what
    /// `subscription()` checks to decide whether the debounce tick needs to
    /// run at all.
    pub fn hover_pending_active(&self) -> bool {
        self.hover_pending.is_some()
    }

    /// The position to send a `LspCommand::Hover` request for, if
    /// `hover_pending` has rested long enough (`HOVER_DWELL`) and hasn't
    /// already been requested — what the debounce tick calls each time it
    /// fires. `None` means "nothing to do this tick", not an error.
    pub fn due_hover_request(&self) -> Option<CursorPos> {
        let (pos, at) = self.hover_pending?;
        if self.hover_requested_for == Some(pos) || at.elapsed() < HOVER_DWELL {
            return None;
        }
        Some(pos)
    }

    /// Marks `pos` as the position a request has now actually been sent
    /// for — called right after sending it, so `due_hover_request` doesn't
    /// fire again for the same position while the reply is in flight.
    pub fn mark_hover_requested(&mut self, pos: CursorPos) {
        self.hover_requested_for = Some(pos);
    }

    /// Applies a `LspEvent::Hover` response — discarded if the mouse has
    /// since left the position it was requested for, the same stale-response
    /// guard `LspEvent::Completions` uses against `completion_anchor`.
    pub fn apply_hover_response(&mut self, line: u32, character: u32, text: Option<String>) {
        let Some(requested) = self.hover_requested_for else {
            return;
        };
        if requested.line != line as usize {
            return;
        }
        let line_text = self.document.line_text(requested.line);
        if char_col_to_utf16_col(&line_text, requested.col) != character {
            return;
        }
        self.hover = text.map(|t| (requested, t));
    }

    /// Applies a `LspEvent::SignatureHelp` response, discarding it if the
    /// cursor has since moved past the position it was requested for (same
    /// staleness check `apply_hover_response` makes). An empty
    /// `signatures` list closes the popup — the server saying "nothing
    /// active here" is exactly what should happen right after `)` closes a
    /// call.
    pub fn apply_signature_help_response(
        &mut self,
        line: u32,
        character: u32,
        signatures: Vec<lsp::SignatureInformation>,
        active_signature: Option<u32>,
        active_parameter: Option<u32>,
    ) {
        let requested = self.signature_help_anchor;
        if requested.line != line as usize {
            return;
        }
        let line_text = self.document.line_text(requested.line);
        if char_col_to_utf16_col(&line_text, requested.col) != character {
            return;
        }
        if signatures.is_empty() {
            self.signature_help = None;
            return;
        }
        let active_signature = active_signature.unwrap_or(0) as usize;
        let active_signature = active_signature.min(signatures.len().saturating_sub(1));
        let active_parameter = active_parameter
            .map(|p| p as usize)
            .or_else(|| signatures[active_signature].active_parameter.map(|p| p as usize));
        self.signature_help = Some(SignatureHelpState {
            signatures,
            active_signature,
            active_parameter,
            anchor: requested,
        });
    }
}

/// How long the mouse has to rest on the same cell, motionless, before a
/// `textDocument/hover` request fires for it — long enough that sweeping the
/// mouse across a line while reading doesn't fire a request per character,
/// short enough that pausing to actually look at something shows docs
/// promptly.
pub const HOVER_DWELL: Duration = Duration::from_millis(300);

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum CharClass {
    Word,
    Space,
    Other,
}

pub fn char_class(c: char) -> CharClass {
    if c.is_alphanumeric() || c == '_' {
        CharClass::Word
    } else if c.is_whitespace() {
        CharClass::Space
    } else {
        CharClass::Other
    }
}

/// The closing half of an auto-paired opener — `None` for anything that
/// isn't one, in particular the closers themselves (`)` doesn't open a pair
/// of its own). See `EditorState::type_char`.
pub fn closer_for(open: char) -> Option<char> {
    match open {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        '"' => Some('"'),
        '\'' => Some('\''),
        '`' => Some('`'),
        _ => None,
    }
}

/// Whether `c` is a closing bracket or a quote — quotes count because they
/// close *themselves*, which is what makes typing a second `"` right before
/// an auto-inserted one a skip-over rather than a double-insert.
pub fn is_closer(c: char) -> bool {
    matches!(c, ')' | ']' | '}' | '"' | '\'' | '`')
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

/// Moves the active file's cursor to the start of `line` (1-based, clamped
/// to the document's own line count) and scrolls it into view — the
/// palette's `:N` syntax (`filtered_palette_entries`) and `Ctrl+G`'s job.
pub fn goto_line(state: &mut State, line: usize) -> iced::Task<Message> {
    let font_size = state.editor_font_size;
    let word_wrap = state.word_wrap;
    let Some(path) = active_file_path(state) else {
        return iced::Task::none();
    };
    let Some(editor) = find_editor_mut(state, &path) else {
        return iced::Task::none();
    };
    let target = line.saturating_sub(1).min(editor.document.line_count().saturating_sub(1));
    editor.cursor = CursorPos { line: target, col: 0 };
    editor.selection_anchor = None;
    center_line_in_viewport(editor, font_size, word_wrap, target)
}

/// `⌘S` / palette "Save File". An untitled buffer (`document.path()` still
/// `None`) has nothing to write to yet, so this kicks off `save_file_as`
/// (the native Save As dialog) instead of calling `document.save()` — which
/// would just produce its existing "document has no path" error.
pub fn save_current_file(state: &mut State) -> iced::Task<Message> {
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
    match editor.save() {
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
pub fn save_file_as(state: &State, old_path: PathBuf) -> iced::Task<Message> {
    let name = old_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let dir = state.root.clone();
    iced::Task::perform(save_file_dialog(dir, name), move |chosen| Message::SaveAsResult(old_path, chosen))
}

pub async fn save_file_dialog(dir: PathBuf, name: String) -> Option<PathBuf> {
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
pub fn save_all_dirty_files(state: &mut State) {
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
        match editor.save() {
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

/// A stable id for the in-file find widget's search box, so `update()` can
/// focus it the moment Ctrl+F opens it.
pub fn find_query_id() -> iced::widget::Id {
    iced::widget::Id::new("find-query")
}

/// A stable id for the primary pane's editor scroll area, so `find_step` can
/// scroll a Find match into view — same pattern as `find_query_id`. Find
/// only ever operates on the primary pane, so this is the one fixed id for
/// it; the split pane (when open) gets its own, `split_editor_scroll_id`.
pub fn editor_scroll_id() -> iced::widget::Id {
    iced::widget::Id::new("editor-scroll-area")
}

/// The split pane's own version of `editor_scroll_id` — distinct so the two
/// panes' `scrollable`s (and `scroll_cursor_into_view`'s `scroll_to` calls)
/// never fight over one widget id when both are showing at once.
pub fn split_editor_scroll_id() -> iced::widget::Id {
    iced::widget::Id::new("split-editor-scroll-area")
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
pub fn complete_save_as(state: &mut State, old_path: PathBuf, new_path: PathBuf) {
    rename_open_tab(state, &old_path, &new_path);
    let Some(editor) = find_editor_mut(state, &new_path) else {
        return;
    };
    editor.language = new_path.extension().and_then(|e| e.to_str()).and_then(syntax::Language::from_extension);
    editor.resync_after_edit();

    let name = new_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    match editor.save() {
        Ok(()) => {
            refresh_tree(state);
            refresh_changed_files(state);
            push_toast(state, ToastKind::Success, format!("Saved {name}"));
        }
        Err(err) => push_toast(state, ToastKind::Error, format!("Couldn't save {name}: {err}")),
    }
}

pub fn close_other_tabs(state: &mut State) {
    let Some(active) = state.active_tab.clone() else {
        return;
    };
    let others: Vec<TabKey> = state.open_tabs.iter().map(|t| t.key()).filter(|k| *k != active).collect();
    for key in others {
        close_tab(state, &key);
    }
}

/// Pops and reopens the most recently closed tab — the tab-bar overflow
/// menu's "Reopen closed tab". Only tabs explicitly closed via `close_tab`
/// are on this stack (see `State::closed_tabs`), not ones closed only as a
/// side effect (a `Diff` tab auto-closed with its backing `File` tab).
pub fn reopen_closed_tab(state: &mut State) {
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
pub const ASSUMED_VIEWPORT_HEIGHT: f32 = 400.0;

/// Moves the active file tab's find selection by `delta` (wrapping), moves
/// the cursor to the newly-current match, and — if that match isn't already
/// within the visible scroll range — scrolls it into view, centered in the
/// viewport.
pub fn find_step(state: &mut State, delta: i32) -> iced::Task<Message> {
    let Some(path) = active_file_path(state) else {
        return iced::Task::none();
    };
    let font_size = state.editor_font_size;
    let word_wrap = state.word_wrap;
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
    let next = (find.current as i32 + delta).rem_euclid(len) as usize;
    find.just_wrapped = if delta > 0 {
        next < find.current
    } else {
        next > find.current
    };
    find.current = next;
    let target = find.matches[find.current];
    let cursor: CursorPos = editor.document.line_col(target.start).into();
    editor.cursor = cursor;
    editor.selection_anchor = None;
    center_line_in_viewport(editor, font_size, word_wrap, cursor.line)
}

/// The active file tab's "Replace" button: replaces the current find match,
/// then scrolls whichever match now sits at `find.current` into view — see
/// `EditorState::replace_current` for why that's already "the next one"
/// without any extra bookkeeping here.
pub fn replace_current_match(state: &mut State) -> iced::Task<Message> {
    let Some(path) = active_file_path(state) else {
        return iced::Task::none();
    };
    let font_size = state.editor_font_size;
    let word_wrap = state.word_wrap;
    let Some(editor) = find_editor_mut(state, &path) else {
        return iced::Task::none();
    };
    editor.replace_current();
    let Some(find) = editor.find.as_ref() else {
        return iced::Task::none();
    };
    let Some(target) = find.matches.get(find.current).copied() else {
        return iced::Task::none();
    };
    let line = editor.document.line_col(target.start).0;
    center_line_in_viewport(editor, font_size, word_wrap, line)
}

/// The active file tab's "Replace All" button.
pub fn replace_all_matches(state: &mut State) {
    let Some(path) = active_file_path(state) else {
        return;
    };
    let Some(editor) = find_editor_mut(state, &path) else {
        return;
    };
    editor.replace_all();
}

/// Scrolls `editor`'s viewport just enough to center `line`, if it isn't
/// already fully visible — shared by Find's "jump to next match" and Go to
/// Line, which both want "leave it alone if visible, otherwise recenter"
/// rather than `scroll_cursor_into_view`'s minimal nudge.
pub fn center_line_in_viewport(
    editor: &mut EditorState,
    font_size: f32,
    word_wrap: bool,
    line: usize,
) -> iced::Task<Message> {
    let line_height = editor_canvas::line_height_px(font_size);
    let line_top = if word_wrap {
        let wrap_cols = editor_canvas::wrap_cols_for_pane(
            if editor.viewport_width > 0.0 { editor.viewport_width } else { ASSUMED_VIEWPORT_WIDTH },
            font_size,
        );
        editor_canvas::row_top_wrapped(&editor.document, wrap_cols, line, 0, font_size)
    } else {
        editor_canvas::line_top(line, font_size)
    };
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
    // Updated by hand, same reasoning as `scroll_cursor_into_view`: the next
    // caller in the same tick (e.g. a held Find-next) must see this scroll
    // as already applied, not recompute against the pre-scroll viewport.
    editor.scroll_offset = target_offset;
    // Also resets horizontal scroll back to the line's start, rather than
    // trying to horizontally center on whatever column the jump landed on —
    // simpler, and right often enough (most lines worth jumping to fit
    // on-screen once scrolled to their start); a match deep inside an
    // unusually long line may still need a manual scroll right afterward.
    editor.scroll_offset_x = 0.0;
    iced::widget::operation::scroll_to(
        editor_scroll_id(),
        iced::widget::scrollable::AbsoluteOffset { x: 0.0, y: target_offset },
    )
}

/// Assumed editor viewport width (px) when `EditorState::viewport_width`
/// hasn't been reported yet — the horizontal sibling of
/// `ASSUMED_VIEWPORT_HEIGHT`, same reasoning.
pub const ASSUMED_VIEWPORT_WIDTH: f32 = 700.0;

/// Scrolls the active file tab's editor just far enough to put the caret on
/// screen, in whichever axis (or both) it isn't already.
///
/// Without this the view never followed the cursor at all — `scroll_to` was
/// reachable only from `find_step` (now `center_line_in_viewport`) — so
/// holding Down, or typing past the bottom or right edge, walked the caret
/// off-screen and left you editing blind.
///
/// Scrolls *minimally*, unlike `center_line_in_viewport`'s deliberate
/// centering: arrowing one line past the edge should nudge the view one
/// line, not fling the caret into the middle of the screen. A no-op in both
/// axes while the caret is already fully visible, which matters because
/// this runs on every cursor move and every keystroke — it must never fight
/// the user's own scrolling.
pub fn scroll_cursor_into_view(state: &mut State, pane: Pane) -> iced::Task<Message> {
    let Some(path) = pane_file_path(state, pane) else {
        return iced::Task::none();
    };
    let font_size = state.editor_font_size;
    let word_wrap = state.word_wrap;
    let Some(editor) = find_editor_mut(state, &path) else {
        return iced::Task::none();
    };
    let scroll_id = match pane {
        Pane::Primary => editor_scroll_id(),
        Pane::Split => split_editor_scroll_id(),
    };

    let line_top = if word_wrap {
        let wrap_cols = editor_canvas::wrap_cols_for_pane(
            if editor.viewport_width > 0.0 { editor.viewport_width } else { ASSUMED_VIEWPORT_WIDTH },
            font_size,
        );
        editor_canvas::row_top_wrapped(&editor.document, wrap_cols, editor.cursor.line, editor.cursor.col, font_size)
    } else {
        editor_canvas::line_top(editor.cursor.line, font_size)
    };
    let line_bottom = line_top + editor_canvas::line_height_px(font_size);
    let viewport_height = if editor.viewport_height > 0.0 {
        editor.viewport_height
    } else {
        ASSUMED_VIEWPORT_HEIGHT
    };
    let visible_top = editor.scroll_offset;
    let visible_bottom = visible_top + viewport_height;
    let target_y = if line_top < visible_top {
        Some(line_top)
    } else if line_bottom > visible_bottom {
        Some(line_bottom - viewport_height)
    } else {
        None
    };

    // Word wrap has no horizontal scroll at all (the canvas is exactly the
    // pane's width — see `shell.rs`'s `code_area`), so there is nothing for
    // this half to do: every column of a wrapped row is already on-screen
    // by construction.
    let (target_x, visible_left) = if word_wrap {
        (None, 0.0)
    } else {
        // Treats the caret as one char wide rather than a zero-width point,
        // so the char just typed at the far edge is fully in view, not just
        // the caret's own leading pixel.
        let char_width = editor_canvas::char_width_px(font_size);
        let col_left = editor_canvas::col_left(editor.cursor.col, font_size);
        let col_right = col_left + char_width;
        let viewport_width = if editor.viewport_width > 0.0 {
            editor.viewport_width
        } else {
            ASSUMED_VIEWPORT_WIDTH
        };
        let visible_left = editor.scroll_offset_x;
        let visible_right = visible_left + viewport_width;
        let target_x = if col_left < visible_left {
            Some(col_left)
        } else if col_right > visible_right {
            Some(col_right - viewport_width)
        } else {
            None
        };
        (target_x, visible_left)
    };

    if target_y.is_none() && target_x.is_none() {
        return iced::Task::none();
    }
    let target_y = target_y.unwrap_or(visible_top).max(0.0);
    let target_x = target_x.unwrap_or(visible_left).max(0.0);
    // The gutter lives in the same scrollable canvas as the text (columns
    // `0..text_x0`), so scrolling to exactly `col_left` of an early column
    // would leave a sliver of it (or all of it, for column 0 itself)
    // permanently hidden to the left of the viewport for no benefit — there
    // is nothing useful *between* 0 and `text_x0` to reveal by stopping
    // short of it. Snap to 0 instead whenever the target would do that.
    let text_x0 = editor_canvas::col_left(0, font_size);
    let target_x = if word_wrap || target_x <= text_x0 { 0.0 } else { target_x };

    // Updated by hand rather than waiting for the scrollable's `on_scroll` to
    // report back: the next keystroke of a held arrow key redoes this same
    // arithmetic, and a `scroll_offset`/`scroll_offset_x` still describing
    // the pre-scroll viewport would make it scroll a second time for
    // something already visible.
    editor.scroll_offset = target_y;
    editor.scroll_offset_x = target_x;
    iced::widget::operation::scroll_to(
        scroll_id,
        iced::widget::scrollable::AbsoluteOffset { x: target_x, y: target_y },
    )
}

pub const MAX_PALETTE_RESULTS: usize = 50;

/// The path of the active tab, if it's a `File` tab (not `Diff` or `Search`).
pub fn active_file_path(state: &State) -> Option<PathBuf> {
    match state.active_tab.as_ref()? {
        TabKey::File(path) => Some(path.clone()),
        _ => None,
    }
}

/// The file a given pane is currently showing — `Pane::Primary` is just
/// `active_file_path`; `Pane::Split` reads `state.split_tab` instead. Every
/// editing `Message` resolves its target buffer through this rather than
/// `active_file_path` directly, so a keystroke in the split pane mutates
/// the split pane's file, not the primary one.
pub fn pane_file_path(state: &State, pane: Pane) -> Option<PathBuf> {
    match pane {
        Pane::Primary => active_file_path(state),
        Pane::Split => match state.split_tab.as_ref()? {
            TabKey::File(path) => Some(path.clone()),
            _ => None,
        },
    }
}

/// `⌘\` / the palette's "Split Editor" — opens the split pane showing
/// whichever other open file tab comes first, or closes it if already open.
/// A no-op (past a flash telling the user why) when there's no primary file
/// to compare against, or no *other* file open to fill the split with —
/// `split_tab` must never equal `active_tab` (see its own doc comment).
pub fn toggle_split_view(state: &mut State) {
    if state.split_tab.is_some() {
        state.split_tab = None;
        return;
    }
    let Some(primary) = active_file_path(state) else {
        return;
    };
    let other = state.open_tabs.iter().find_map(|t| match t {
        OpenTab::File(editor) if editor.path != primary => Some(editor.path.clone()),
        _ => None,
    });
    match other {
        Some(path) => state.split_tab = Some(TabKey::File(path)),
        None => push_flash(state, "OPEN ANOTHER FILE TO SPLIT"),
    }
}

/// Shows `path` in the split pane — from the palette's "Open in Split: ..."
/// entries. Opens it as a tab first if it isn't already one, mirroring
/// `open_or_focus_file`, but sets `split_tab` instead of `active_tab`. A
/// no-op if `path` is already the primary pane's file, same reasoning as
/// `toggle_split_view`.
pub fn open_or_focus_file_in_split(state: &mut State, path: PathBuf) {
    if active_file_path(state).as_deref() == Some(path.as_path()) {
        return;
    }
    let key = TabKey::File(path.clone());
    if !state.open_tabs.iter().any(|t| t.key() == key) {
        let Ok(document) = Document::open(&path) else {
            return;
        };
        state.open_tabs.push(OpenTab::File(Box::new(EditorState::new(document, path.clone()))));
        send_did_open_for(state, &path);
        send_copilot_did_open_for(state, &path);
        recompute_diff_for(state, &path);
    }
    state.split_tab = Some(key);
    persist_session(state);
}

pub fn find_editor<'a>(state: &'a State, path: &Path) -> Option<&'a EditorState> {
    state.open_tabs.iter().find_map(|t| match t {
        OpenTab::File(editor) if editor.path == path => Some(editor.as_ref()),
        _ => None,
    })
}

pub fn find_editor_mut<'a>(state: &'a mut State, path: &Path) -> Option<&'a mut EditorState> {
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
pub fn rebuild_editor_from_disk(state: &mut State, path: &Path) {
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
pub fn reload_editor_from_disk(state: &mut State, path: &Path) {
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
pub fn discard_change(state: &mut State, path: PathBuf) {
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

/// `(theme_mode, accent, custom_accent, high_contrast)` — `state.theme_preview`
/// when the settings panel is live-previewing a change, else the committed
/// settings. The single source every `active_palette` call (and so every
/// `view()` in the app) reads, so a hovered swatch previews everywhere at
/// once rather than just inside the settings panel itself (roadmap item 11).
pub fn active_theme(state: &State) -> (ThemeMode, Accent, Option<(u8, u8, u8)>, bool) {
    match state.theme_preview {
        Some(preview) => (preview.theme_mode, preview.accent, preview.custom_accent, preview.high_contrast),
        None => (state.theme_mode, state.accent, state.custom_accent, state.high_contrast),
    }
}

/// The resolved `Palette` for whatever `active_theme` currently says —
/// every `view()` function should call this rather than
/// `devscribe_core::theme::palette(state.theme_mode, state.accent)`
/// directly, or it won't pick up a live preview (roadmap item 11).
pub fn active_palette(state: &State) -> theme::Palette {
    let (mode, accent, custom, high_contrast) = active_theme(state);
    let p = theme::palette_custom(mode, accent, custom);
    if high_contrast { theme::apply_high_contrast(p) } else { p }
}

/// Every currently open `File` tab's path — used to replay `didOpen` for all
/// of them once the LSP server becomes ready (it may become ready after
/// files were already opened).
pub fn open_file_paths(state: &State) -> Vec<PathBuf> {
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
/// — this is what makes opening a second file additive instead of a
/// replace.
pub fn open_or_focus_file(state: &mut State, path: PathBuf) {
    let key = TabKey::File(path.clone());
    if state.open_tabs.iter().any(|t| t.key() == key) {
        state.active_tab = Some(key);
        persist_session(state);
        return;
    }
    if let Ok(document) = Document::open(&path) {
        state.open_tabs.push(OpenTab::File(Box::new(EditorState::new(document, path.clone()))));
        state.active_tab = Some(key);
        send_did_open_for(state, &path);
        send_copilot_did_open_for(state, &path);
        recompute_diff_for(state, &path);
        persist_session(state);
    }
}

/// Opens `path` as a `File` tab if it isn't one already, same as
/// `open_or_focus_file`'s "ensure it's open" half — but never touches
/// `active_tab`. Used by `apply_rename`: a workspace-wide rename routinely
/// touches files the user never opened (or has open in neither pane), and
/// jumping the primary pane to each one in turn as its edit lands would
/// yank focus away from wherever the user actually is mid-rename.
fn ensure_file_open(state: &mut State, path: &Path) {
    let key = TabKey::File(path.to_path_buf());
    if state.open_tabs.iter().any(|t| t.key() == key) {
        return;
    }
    if let Ok(document) = Document::open(path) {
        state.open_tabs.push(OpenTab::File(Box::new(EditorState::new(document, path.to_path_buf()))));
        send_did_open_for(state, path);
        send_copilot_did_open_for(state, path);
        recompute_diff_for(state, path);
    }
}

/// Applies a `LspEvent::RenameResult`'s edits, one file at a time —
/// opening each as a tab first if needed (`ensure_file_open`) since a
/// project-wide rename routinely touches files well outside whatever's
/// currently open. Each file's edits land as that file's own single undo
/// step (`EditorState::apply_text_edits`); nothing is auto-saved, so the
/// user reviews (and can undo) each touched file same as any other edit —
/// consistent with this app's "save is explicit" model everywhere else.
/// Reports how many files actually changed via a toast, since a rename
/// that silently touches several files scattered across the tree would
/// otherwise be invisible.
pub fn apply_rename(state: &mut State, edits: Vec<(lsp::Url, Vec<lsp::TextEdit>)>) {
    if edits.is_empty() {
        push_toast(state, ToastKind::Warning, "Rename found nothing to change");
        return;
    }
    let mut changed = 0usize;
    for (uri, text_edits) in edits {
        let Ok(path) = uri.to_file_path() else { continue };
        ensure_file_open(state, &path);
        if find_editor_mut(state, &path).is_some_and(|editor| editor.apply_text_edits(&text_edits)) {
            changed += 1;
            mark_edited(state, &path);
        }
    }
    if changed > 0 {
        push_toast(
            state,
            ToastKind::Success,
            format!("Renamed across {changed} file{}", if changed == 1 { "" } else { "s" }),
        );
    } else {
        push_toast(state, ToastKind::Warning, "Rename found nothing to change");
    }
}

/// Hands `target` (a URL, per `Message::OpenMarkdownLink`) to the OS's
/// default application for it — a relative/`file:` link opens whatever
/// handles that path locally, `https:`/`mailto:` etc. go to the OS's usual
/// browser/mail client.
pub fn open_externally(target: &str) {
    if let Err(err) = opener::open(target) {
        crate::logging::error(format!("failed to open {target} externally: {err}"));
    }
}

/// Opens a diff tab for `path`, or focuses it if already open. Always
/// ensures a backing `File` tab exists first (opening one if needed) since
/// that's where the actual `DiffStatus` is computed and cached.
pub fn open_or_focus_diff(state: &mut State, path: PathBuf) {
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
    persist_session(state);
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
pub fn begin_untitled_buffer(state: &mut State) {
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
pub fn view_working_tree_diff(state: &mut State) {
    if let Some(path) = active_file_path(state) {
        open_or_focus_diff(state, path);
    } else if let Some(entry) = state.changed_files.first() {
        open_or_focus_diff(state, entry.path.clone());
    }
}

/// Closes the tab matching `key`. Closing a `File` tab also closes its diff
/// tab, if any (a `Diff` tab has no content without its backing `File`
/// tab), and notifies the LSP server. If the active tab was closed, focuses
/// the tab that's now in its place, or `None` if that was the last one.
pub fn close_tab(state: &mut State, key: &TabKey) {
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
        if let Some(sender) = state.copilot_completion_sender.as_mut() {
            send_copilot_did_close(sender, &editor.path);
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
    persist_session(state);
}

/// The Ctrl+Tab quick switcher's (roadmap item 2) candidate list, in the
/// same order the tab bar itself renders them (`ui::tab_bar::view`): Chat
/// first if it's pinned open, then every `open_tabs` entry. `Search` is
/// deliberately excluded — it's a fixed entry point, not an "open tab" a
/// user would be cycling back to.
pub fn tab_switcher_entries(state: &State) -> Vec<TabKey> {
    let mut entries = Vec::with_capacity(state.open_tabs.len() + 1);
    if state.chat_tab_open {
        entries.push(TabKey::Chat);
    }
    entries.extend(state.open_tabs.iter().map(|t| t.key()));
    entries
}

/// Actually switches to `key` — shared by `ConfirmTabSwitcher` and a direct
/// click on a switcher entry. Mirrors `Message::SelectOpenTab`'s and
/// `Message::ChatOpenTab`'s own handling rather than calling them
/// recursively (`update()` isn't set up for that), so any change to either
/// needs to stay in sync with this.
pub fn switch_to_tab(state: &mut State, key: &TabKey) -> iced::Task<Message> {
    match key {
        TabKey::Chat => {
            state.chat_tab_open = true;
            state.chat_mode = ChatMode::Closed;
            state.active_tab = Some(TabKey::Chat);
            state.chat_view_menu_open = false;
            persist_settings(state);
            persist_session(state);
            focus_chat_input()
        }
        _ => {
            if state.open_tabs.iter().any(|t| &t.key() == key) {
                state.active_tab = Some(key.clone());
                persist_session(state);
            }
            iced::Task::none()
        }
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
pub const EDIT_SETTLE: Duration = Duration::from_millis(90);

/// Records that `path`'s buffer changed, arming the settle timer. The
/// expensive follow-up work happens in `flush_pending_edits`.
pub fn mark_edited(state: &mut State, path: &Path) {
    state.edit_settled_at = Some(Instant::now());
    if !state.pending_edits.iter().any(|p| p == path) {
        state.pending_edits.push(path.to_path_buf());
    }
    // Any edit invalidates whatever ghost-text suggestion was showing —
    // it described text to insert at a position that no longer reflects
    // what's actually in the buffer. A fresh one is requested once typing
    // settles (`maybe_trigger_ghost_completion`, called alongside
    // `flush_pending_edits` from the same `EditSettleTick` debounce this
    // function already arms), not immediately — see `GhostCompletion`'s own
    // doc comment on why this doesn't attempt to locally re-narrow instead.
    if let Some(editor) = find_editor_mut(state, path) {
        editor.close_ghost_completion();
    }
}

/// Runs the deferred per-edit work now, for every buffer waiting on it.
///
/// Called by `EditSettleTick` once typing stops, and directly by anything
/// that needs a buffer's derived state to be current *before* it acts —
/// notably an LSP completion request, which would otherwise be answered
/// against a document the server has not been told about yet.
pub fn flush_pending_edits(state: &mut State) {
    state.edit_settled_at = None;
    for path in std::mem::take(&mut state.pending_edits) {
        if let Some(editor) = find_editor_mut(state, &path) {
            editor.reparse_now();
        }
        send_did_change_for(state, &path);
        send_copilot_did_change_for(state, &path);
        recompute_diff_for(state, &path);
    }
}

pub fn recompute_diff_for(state: &mut State, path: &Path) {
    let Some((current_text, line_count)) =
        find_editor(state, path).map(|e| (e.document.text().to_string(), e.document.line_count()))
    else {
        return;
    };

    let status = match state.repo.as_ref().and_then(|repo| repo.head_text(path)) {
        None if state.repo.is_none() => DiffStatus::NoRepo,
        None => DiffStatus::Untracked,
        Some(old) => {
            let lines = if state.diff_ignore_whitespace {
                devscribe_core::diff::diff_lines_ignoring_whitespace(&old, &current_text)
            } else {
                devscribe_core::diff::diff_lines(&old, &current_text)
            };
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
        // A stale armed line-number after the diff moves would confirm-revert
        // whatever line now sits there, not the one the user meant.
        editor.pending_revert_line = None;
    }
}

pub fn lsp_uri(path: &Path) -> Option<lsp::Url> {
    lsp::Url::from_file_path(path).ok()
}

pub fn is_lsp_language(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .and_then(lsp::LspLanguage::from_extension)
        .is_some()
}

pub fn send_did_open_for(state: &mut State, path: &Path) {
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

/// `copilot_completion`'s own document sync — deliberately not gated by
/// `is_lsp_language` the way `send_did_open_for` is: Copilot suggests for
/// any text file, not just the handful of languages this app has a
/// `LspLanguage`/language-server mapping for.
pub fn send_copilot_did_open_for(state: &mut State, path: &Path) {
    let Some(uri) = lsp_uri(path) else {
        return;
    };
    let Some(text) = find_editor(state, path).map(|e| e.document.text().to_string()) else {
        return;
    };
    if let Some(sender) = state.copilot_completion_sender.as_mut() {
        let _ = sender.try_send(CopilotCompletionCommand::DidOpen { uri, text });
    }
}

pub fn send_copilot_did_change_for(state: &mut State, path: &Path) {
    let Some(uri) = lsp_uri(path) else {
        return;
    };
    let Some(text) = find_editor(state, path).map(|e| e.document.text().to_string()) else {
        return;
    };
    if let Some(sender) = state.copilot_completion_sender.as_mut() {
        let _ = sender.try_send(CopilotCompletionCommand::DidChange { uri, text });
    }
}

pub fn send_copilot_did_close(sender: &mut mpsc::Sender<CopilotCompletionCommand>, path: &Path) {
    if let Some(uri) = lsp_uri(path) {
        let _ = sender.try_send(CopilotCompletionCommand::DidClose { uri });
    }
}

/// Fires the inline-completion request once typing settles — called from
/// `Message::EditSettleTick` right after `flush_pending_edits`, reusing that
/// same debounce rather than a second timer (see `GhostCompletion`'s own
/// doc comment for why "wait for a pause" is the right trigger shape here).
/// Only ever requests for the *active* file: ghost text is a single-cursor,
/// single-view concept, unlike `flush_pending_edits`'s own per-buffer
/// `didChange`/reparse/diff work, which runs for every buffer that changed.
/// Skipped outright while the LSP dot-completion popup is open — showing
/// both at once would visually conflict, and `Tab` can only accept one.
pub fn maybe_trigger_ghost_completion(state: &mut State) {
    if !state.copilot_inline_enabled {
        return;
    }
    let Some(path) = active_file_path(state) else {
        return;
    };
    let Some(editor) = find_editor(state, &path) else {
        return;
    };
    if editor.completions.is_some() {
        return;
    }
    let cursor = editor.cursor;
    let line_text = editor.document.line_text(cursor.line);
    let utf16_char = char_col_to_utf16_col(&line_text, cursor.col);
    let Some(uri) = lsp_uri(&path) else {
        return;
    };
    if let Some(sender) = state.copilot_completion_sender.as_mut() {
        let _ = sender.try_send(CopilotCompletionCommand::Suggest { uri, line: cursor.line as u32, character: utf16_char });
    }
}

/// Whether every char of `text` could plausibly be part of an identifier —
/// used by `EditorInsertText`/`EditorTypeChar` to decide whether typed text
/// should narrow an open completion popup (`refilter_completions`) or close
/// it outright. `false` for an empty string: there's nothing to narrow with.
pub fn is_word_text(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_alphanumeric() || c == '_')
}

/// Fires the LSP completion request that typing `.` or the second `:` of
/// `::` triggers. Shared by `EditorInsertText` and `EditorTypeChar` — a lone
/// `.`/`:` can arrive through either (auto-pairing never touches either
/// char, so `EditorTypeChar` still passes them straight through to a plain
/// insert), and the trigger condition needs to stay identical on both paths.
pub fn maybe_trigger_completion(state: &mut State, path: &Path, text: &str) {
    if !matches!(state.lsp_status, LspStatus::Ready) || (text != "." && text != ":") {
        return;
    }
    let trigger_info = find_editor(state, path).and_then(|editor| {
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
    let Some((cursor, utf16_char)) = trigger_info else {
        return;
    };
    // The `didChange` for the character that triggered this is still sitting
    // in the settle queue; send it before asking, or the server completes
    // against a buffer it has not seen. Only the notification — running the
    // whole settle here would put the 30 ms reparse back on the keystroke
    // path, and on exactly the keystrokes (`.`, `::`) that need to feel
    // fast. The reparse and diff stay queued.
    send_did_change_for(state, path);
    let Some(uri) = lsp_uri(path) else {
        return;
    };
    if let Some(sender) = state.lsp_sender.as_mut() {
        let _ = sender.try_send(LspCommand::Completion {
            uri,
            line: cursor.line as u32,
            character: utf16_char,
        });
    }
    if let Some(editor) = find_editor_mut(state, path) {
        editor.completion_anchor = cursor;
    }
}

/// Fires the LSP signature-help request that typing `(`, `,`, or `)` inside
/// a call triggers — one request per keystroke rather than tracking paren
/// depth client-side, same shape as `maybe_trigger_completion`. `)` is
/// included so leaving a call gets a fresh (likely empty) response that
/// closes the popup, rather than leaving the last argument list's signature
/// stuck on screen.
pub fn maybe_trigger_signature_help(state: &mut State, path: &Path, text: &str) {
    if !matches!(state.lsp_status, LspStatus::Ready) || !matches!(text, "(" | "," | ")") {
        return;
    }
    let Some(editor) = find_editor(state, path) else {
        return;
    };
    let cursor = editor.cursor;
    let line_text = editor.document.line_text(cursor.line);
    let utf16_char = char_col_to_utf16_col(&line_text, cursor.col);
    let Some(uri) = lsp_uri(path) else {
        return;
    };
    // Same reasoning as `maybe_trigger_completion`: flush the pending
    // `didChange` for this keystroke first so the server sees it.
    send_did_change_for(state, path);
    if let Some(sender) = state.lsp_sender.as_mut() {
        let _ = sender.try_send(LspCommand::SignatureHelp {
            uri,
            line: cursor.line as u32,
            character: utf16_char,
        });
    }
    if let Some(editor) = find_editor_mut(state, path) {
        editor.signature_help_anchor = cursor;
    }
}

pub fn send_did_change_for(state: &mut State, path: &Path) {
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

pub fn send_did_close(sender: &mut mpsc::Sender<LspCommand>, path: &Path) {
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
pub fn convert_diagnostics(document: &Document, diagnostics: Vec<lsp::Diagnostic>) -> Vec<EditorDiagnostic> {
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

/// Converts one LSP `Location` into a clickable `LocationEntry` — preferring
/// the already-open editor's live buffer for the target line's text and
/// UTF-16-to-char column conversion when the file has one (so a dirty,
/// unsaved file's own in-progress edits are reflected), and falling back to
/// reading the file straight off disk otherwise, since a definition or
/// reference is very often in a file the user hasn't opened yet. `None` if
/// the location's `uri` isn't a `file://` path, or the on-disk read fails.
fn location_entry(state: &State, loc: &lsp::Location) -> Option<LocationEntry> {
    let path = loc.uri.to_file_path().ok()?;
    let line_idx = loc.range.start.line as usize;
    let line_text = if let Some(editor) = find_editor(state, &path) {
        editor.document.line_text(line_idx)
    } else {
        std::fs::read_to_string(&path).ok()?.lines().nth(line_idx).unwrap_or("").to_string()
    };
    let col = utf16_col_to_char_col(&line_text, loc.range.start.character as usize);
    Some(LocationEntry {
        path,
        line: line_idx,
        col,
        preview: line_text.trim().chars().take(160).collect(),
    })
}

/// Turns `workspace/symbol` results into ready-made palette rows — the
/// palette's `#query` mode. Reuses `location_entry`'s same UTF-16-to-char
/// conversion (open-buffer-aware, disk-fallback) so a symbol in a dirty,
/// unsaved file still resolves against its live content rather than
/// whatever's on disk. Entries whose `uri` doesn't resolve to a real file
/// (or that fail the on-disk read) are silently dropped, same as
/// `apply_locations`' own filtering.
pub fn symbol_palette_entries(state: &State, symbols: &[lsp::SymbolEntry]) -> Vec<PaletteEntry> {
    symbols
        .iter()
        .filter_map(|symbol| {
            let entry = location_entry(state, &symbol.location)?;
            let file_name = entry.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let label = match &symbol.container_name {
                Some(container) => format!("{} \u{2014} {container} \u{2022} {file_name}:{}", symbol.name, entry.line + 1),
                None => format!("{} \u{2014} {file_name}:{}", symbol.name, entry.line + 1),
            };
            Some(PaletteEntry {
                label,
                action: PaletteAction::JumpToSymbolLocation { path: entry.path, line: entry.line, col: entry.col },
            })
        })
        .collect()
}

/// Shared landing logic for both `LspEvent::Definition` and
/// `LspEvent::References`: a single result (by far the common case for Go to
/// Definition — most symbols have exactly one) jumps straight there; several
/// results (an overridden method, a trait with multiple impls, or any real
/// Find-All-References with more than one usage) open the Locations dock
/// panel so the user picks; zero results is a toast rather than a silent
/// no-op — a go-to-definition that quietly does nothing reads as "broken",
/// not "there was nothing to find".
pub fn apply_locations(state: &mut State, locations: Vec<lsp::Location>, label: &'static str) -> iced::Task<Message> {
    let mut entries: Vec<LocationEntry> = locations.iter().filter_map(|loc| location_entry(state, loc)).collect();
    entries.sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));
    match entries.len() {
        0 => {
            push_toast(state, ToastKind::Warning, format!("No {} found", label.to_lowercase()));
            iced::Task::none()
        }
        1 => {
            let entry = entries.remove(0);
            open_or_focus_file(state, entry.path.clone());
            if let Some(editor) = find_editor_mut(state, &entry.path) {
                editor.click(entry.line, entry.col, false);
            }
            scroll_cursor_into_view(state, Pane::Primary)
        }
        n => {
            state.references_label = format!("{label} \u{2014} {n} results");
            state.references_results = entries;
            state.references_open = true;
            iced::Task::none()
        }
    }
}

/// Spawns a background OS thread that installs `language`'s server binary
/// via the method defined in `server_install::spec_for`. Result arrives as
/// `Message::ServerInstallComplete`.
pub fn start_server_install(language: LspLanguage) -> iced::Task<Message> {
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

pub fn utf16_col_to_char_col(line: &str, utf16_col: usize) -> usize {
    let mut utf16_count = 0usize;
    for (char_idx, ch) in line.chars().enumerate() {
        if utf16_count >= utf16_col {
            return char_idx;
        }
        utf16_count += ch.len_utf16();
    }
    line.chars().count()
}

pub fn char_col_to_utf16_col(line: &str, char_col: usize) -> u32 {
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
pub fn lsp_worker((root, language, _token): &(PathBuf, LspLanguage, u64)) -> impl iced::futures::Stream<Item = LspEvent> + use<> {
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

/// One project-wide `copilot-language-server` connection dedicated to inline
/// completions — independent of the AI Chat Assist panel's own Copilot
/// connection (`copilot_agent`/`chat_worker`), same reasoning as
/// `devscribe_core::copilot_completion`'s own doc comment: inline
/// completions are useful regardless of which chat provider (or whether the
/// chat panel is even open) is active. Keyed on `(root, restart_token)` —
/// no per-language keying the way `lsp_worker` has, since one server serves
/// every open file. No auto-install path (unlike `lsp_worker`): if
/// `copilot-language-server` isn't on PATH, this reports `Unavailable`
/// directly rather than trying to fetch it.
pub fn copilot_completion_worker(
    (root, _token): &(PathBuf, u64),
) -> impl iced::futures::Stream<Item = CopilotCompletionEvent> + use<> {
    let root = root.clone();
    iced::stream::channel(32, async move |mut output| {
        use iced::futures::SinkExt as _;
        if !crate::server_install::which_binary("copilot-language-server") {
            let _ = output
                .send(CopilotCompletionEvent::Unavailable(
                    "copilot-language-server not found on PATH — install: npm install -g @github/copilot-language-server".to_string(),
                ))
                .await;
            return;
        }
        copilot_completion::run(root, PathBuf::from("copilot-language-server"), output).await;
    })
}
