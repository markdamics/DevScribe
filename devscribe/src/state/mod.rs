use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use devscribe_core::claude_agent::{self, ClaudeCommand, ClaudeEvent, PermissionMode};
use devscribe_core::copilot_agent;
use devscribe_core::copilot_completion::{self, CopilotCompletionCommand, CopilotCompletionEvent};
use devscribe_core::diff::{DiffLine, DiffLineKind, GutterMark, Hunk};
use devscribe_core::git::{ChangeKind, Repo};
use devscribe_core::lsp::{self, CompletionItem, LspCommand, LspEvent, LspLanguage};
use devscribe_core::outline;
use devscribe_core::search::{self, SearchHit};
use devscribe_core::syntax::{self, Span};
use devscribe_core::theme::{self, Accent, ThemeMode};
use devscribe_core::watcher::{self, WatchEvent};
use devscribe_core::{Document, Eol};
use iced::futures::channel::mpsc;
use iced::keyboard;
use iced::mouse;

use crate::density::Density;
use crate::fs_tree::{self, Node};
use crate::recent_projects;
use crate::session;
use crate::settings;
use crate::ui::editor_canvas;

mod chat;
mod editor;
mod sidebar;
pub use chat::*;
pub use editor::*;
pub use sidebar::*;

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
    /// The palette's `:N` syntax — see `filtered_palette_entries`. `N` is
    /// 1-based, matching the gutter's own line numbers.
    GoToLine(usize),
}

#[derive(Debug, Clone)]
pub struct PaletteEntry {
    pub label: String,
    pub action: PaletteAction,
}

/// A theme change being previewed, not yet committed — see
/// `State::theme_preview`'s own doc comment. Every field mirrors a
/// committed setting 1:1 and simply overrides it while `Some`
/// (`active_theme` reads this first); nothing here is itself partial.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThemePreview {
    pub theme_mode: ThemeMode,
    pub accent: Accent,
    pub custom_accent: Option<(u8, u8, u8)>,
    pub high_contrast: bool,
}

pub struct State {
    pub theme_mode: ThemeMode,
    pub accent: Accent,
    /// See `settings::Settings::custom_accent`'s own doc comment.
    pub custom_accent: Option<(u8, u8, u8)>,
    /// See `settings::Settings::high_contrast`'s own doc comment.
    pub high_contrast: bool,
    /// The custom accent color picker's own RGB sliders (roadmap item 11)
    /// — live, uncommitted draft values, seeded from `custom_accent` if one
    /// was already set, else a reasonable starting color. Adjusting a
    /// slider updates this *and* sets `theme_preview` so the whole app
    /// reflects it live; nothing here becomes real until "Apply" sends
    /// `Message::SetCustomAccent` with these same values.
    pub custom_accent_draft: (u8, u8, u8),
    /// A theme change the settings panel is showing a live preview of —
    /// hovering a theme-mode/accent-preset/custom-color swatch, or dragging
    /// its RGB sliders, sets this without touching the committed
    /// `theme_mode`/`accent`/`custom_accent`/`high_contrast` fields above,
    /// so the whole app (not just the settings panel itself) reflects the
    /// hovered choice everywhere `active_theme`/`active_palette` is read —
    /// then reverts the instant the preview ends (mouse leaves the swatch,
    /// or the panel closes) with nothing to undo, since nothing was ever
    /// committed. `None` outside of an active preview. Roadmap item 11.
    pub theme_preview: Option<ThemePreview>,
    /// Every currently open tab, in the order they appear in the tab bar.
    pub open_tabs: Vec<OpenTab>,
    /// `None` only when `open_tabs` is empty.
    pub active_tab: Option<TabKey>,
    /// `true` only while `restore_session` is re-opening tabs from a
    /// persisted session — suppresses `persist_session`'s writes for that
    /// duration so restoring a session doesn't immediately re-save a
    /// partially-restored one over top of the file it's reading from. Never
    /// persisted itself.
    restoring_session: bool,
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
    /// worker for the *same* session id — the empty thread's "Try again"
    /// action (`Message::ChatRetryConnection`) after `ChatStatus::Unavailable`.
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
    /// The sessions picker's search box — filters `chat_sessions` by title,
    /// case-insensitively. Cleared whenever the picker closes/reopens
    /// (`Message::ChatToggleSessions`) so a stale filter never hides a
    /// project's whole history behind a search term from last time.
    pub chat_session_filter: String,
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
    /// Whether the message list should keep following new output — `true`
    /// (the default, and reset to `true` any time the transcript itself
    /// resets: a new/resumed session, `ChatFullHistoryLoaded`) until
    /// `ChatScrolled` reports the user isn't within `PIN_TO_BOTTOM_SLACK` of
    /// the bottom, meaning they've scrolled up to read earlier output.
    /// `handle_chat_event` only re-snaps the scroll position to the latest
    /// message while this is `true` — the "stay pinned when reading fresh
    /// output, preserve position when browsing older conversation" half of
    /// the chat-panel UX pass (item 6). Scrolling back down within the
    /// slack re-pins it, same as every chat client's own convention.
    pub chat_pinned_to_bottom: bool,
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
    /// Which backend the panel talks to — see `ChatProvider`. Not persisted,
    /// same as `chat_permission_mode`/`chat_shell_access_enabled`: always
    /// starts back at `Claude` each launch. Part of `chat_worker`'s
    /// subscription key, same reason as those two — picking a different
    /// provider is a spawn-time decision, so it respawns the subprocess.
    pub chat_provider: ChatProvider,
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
    /// In-flight `$/progress` operations the active language server has
    /// reported (rust-analyzer's own "Indexing", "Fetching metadata", ...)
    /// — keyed by the server's own token so a `WorkDoneProgress::End` for
    /// one token clears only that entry, not every operation currently
    /// running. Drives the status bar's progress indicator (visual-feedback
    /// pass, item 8). Cleared outright at every `lsp_status` transition —
    /// a respawn, install, or disable all invalidate whatever token space
    /// a prior connection was tracking.
    pub lsp_progress: std::collections::BTreeMap<String, LspProgressEntry>,
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
    pub copilot_completion_status: CopilotCompletionStatus,
    copilot_completion_sender: Option<mpsc::Sender<CopilotCompletionCommand>>,
    /// Off switches the `copilot_completion_worker` subscription off
    /// entirely, same convention as `lsp_enabled` — no
    /// `copilot-language-server` process gets spawned at all while this is
    /// `false`. Off by default: unlike LSP (core to the editor), inline
    /// completions need an external binary plus a signed-in GitHub Copilot
    /// account, so this is opt-in rather than assumed.
    pub copilot_inline_enabled: bool,
    /// Mirrors `lsp_restart_token` — reserved for a future "retry after the
    /// process died" action, not currently wired to any UI.
    copilot_completion_restart_token: u64,
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
    /// `true` while the status bar's "Background Tasks" popover is open —
    /// a roll-up of the language server's status/progress, the file
    /// watcher, and git, for the visual-feedback pass (roadmap item 8).
    /// Not persisted: it's a transient popover, same as `overflow_open`.
    pub background_tasks_open: bool,
    /// `true` while the status bar's EOL (LF/CRLF) picker popover is open
    /// (roadmap item 9). Not persisted, same as `background_tasks_open`.
    pub eol_picker_open: bool,
    /// `true` while the status bar's language-mode picker popover is open
    /// (roadmap item 9). Not persisted, same as `background_tasks_open`.
    pub language_picker_open: bool,
    /// `true` while the status bar's encoding info popover is open (roadmap
    /// item 9) — `Document` only ever reads/writes UTF-8, so this is
    /// informational rather than a real picker. Not persisted, same as
    /// `background_tasks_open`.
    pub encoding_info_open: bool,
    /// `true` while the Locations dock panel is open — populated by either
    /// "Go to Definition" (when the server names more than one candidate)
    /// or "Find All References", both landing in the same panel; see
    /// `apply_locations`. Not persisted across sessions: a stale set of
    /// locations from a project that's since changed would be actively
    /// misleading, unlike the Problems panel's open/closed state.
    pub references_open: bool,
    /// Header text for the Locations panel (e.g. "References — 4 results")
    /// — set alongside `references_results` so the panel doesn't have to
    /// guess which of the two actions populated it.
    pub references_label: String,
    pub references_results: Vec<LocationEntry>,
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
    /// Spaces inserted per `Tab`/removed per `Shift+Tab` — see
    /// `EditorState::indent`/`dedent`.
    pub tab_size: u8,
    /// Gates the gutter's per-line number text (`editor_canvas.rs`'s
    /// `draw`) — the gutter itself (git-diff marks, revert clicks) stays
    /// regardless, since those aren't "line numbers".
    pub show_line_numbers: bool,
    /// Off by default (matching `Settings::default()`). When on, a buffer
    /// line wider than the editor pane renders as several visual rows
    /// instead of scrolling sideways — see `editor_canvas::wrap_row_starts`
    /// for the wrapping itself and `EditorCanvas::word_wrap`'s own doc
    /// comment for why `draw`/`hit_test` share one code path with the
    /// unwrapped case rather than branching throughout.
    pub word_wrap: bool,
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
    /// `(hovered tab, when the hover started)` — drives the tab bar's hover
    /// preview dwell timer (`TAB_PREVIEW_DWELL` in `ui::tab_bar`), the same
    /// shape as `EditorState::hover_pending`. `None` whenever the mouse
    /// isn't currently resting on a tab.
    pub tab_hover: Option<(TabKey, Instant)>,
    /// Non-`None` while the Ctrl+Tab quick switcher overlay (roadmap item 2)
    /// is showing.
    pub tab_switcher: Option<TabSwitcherState>,
    /// `(hovered breadcrumb's index into the current crumb trail, when the
    /// hover started)` — drives the breadcrumb strip's hover-context
    /// tooltip dwell timer (roadmap item 10), same shape as `tab_hover`.
    /// An index rather than an id: crumbs have no identity of their own
    /// (they're recomputed fresh every `view()` from the cursor position),
    /// but the index is stable for as long as the mouse stays over the same
    /// segment, which is all a dwell timer needs.
    pub breadcrumb_hover: Option<(usize, Instant)>,
}

/// A snapshot of the open tabs (in tab-bar order) taken the moment the
/// Ctrl+Tab switcher opens, plus which entry is currently highlighted —
/// further Ctrl+Tab/Ctrl+Shift+Tab presses just move `selected` through this
/// same fixed list rather than re-deriving it (and potentially reordering
/// mid-cycle) on every step.
#[derive(Debug, Clone)]
pub struct TabSwitcherState {
    pub entries: Vec<TabKey>,
    pub selected: usize,
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

pub const TAB_SIZE_MIN: u8 = 2;
pub const TAB_SIZE_MAX: u8 = 8;
pub const TAB_SIZE_DEFAULT: u8 = 4;
pub const TAB_SIZE_STEP: u8 = 2;

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

        let mut state = Self {
            theme_mode: settings.theme_mode,
            accent: settings.accent,
            custom_accent: settings.custom_accent,
            high_contrast: settings.high_contrast,
            custom_accent_draft: settings.custom_accent.unwrap_or((124, 156, 224)),
            theme_preview: None,
            open_tabs: Vec::new(),
            active_tab: None,
            restoring_session: false,
            chat_mode: settings.chat_mode,
            chat_tab_open: false,
            chat_panel_width: settings.chat_panel_width,
            chat_resizing: false,
            chat_restart_token: 0,
            chat_session_id: claude_agent::new_session_id(),
            chat_sessions: Vec::new(),
            chat_sessions_open: false,
            chat_session_filter: String::new(),
            chat_view_menu_open: false,
            chat_actions_open: false,
            chat_pinned_to_bottom: true,
            chat_thinking_enabled: false,
            chat_shell_access_enabled: false,
            // Matches the behavior this app has always had before mode
            // selection existed at all — every prior end-to-end test was
            // built and verified against "ask for every Edit/Write", so
            // that stays the default rather than silently becoming more
            // permissive under this change.
            chat_permission_mode: PermissionMode::Manual,
            chat_provider: ChatProvider::Claude,
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
            lsp_progress: std::collections::BTreeMap::new(),
            lsp_sender: None,
            lsp_enabled: settings.lsp_enabled,
            lsp_restart_token: 0,
            copilot_completion_status: CopilotCompletionStatus::default(),
            copilot_completion_sender: None,
            copilot_inline_enabled: settings.copilot_inline_enabled,
            copilot_completion_restart_token: 0,
            repo,
            changed_files: snapshot.changed_files,
            ahead_behind: snapshot.ahead_behind,
            changes_panel_open: false,
            pending_discard: None,
            problems_panel_open: false,
            background_tasks_open: false,
            eol_picker_open: false,
            language_picker_open: false,
            encoding_info_open: false,
            references_open: false,
            references_label: String::new(),
            references_results: Vec::new(),
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
            tab_size: settings.tab_size,
            show_line_numbers: settings.show_line_numbers,
            word_wrap: settings.word_wrap,
            toasts: Vec::new(),
            next_toast_id: 0,
            draft: None,
            ctx_menu: None,
            flash: None,
            closed_tabs: Vec::new(),
            next_untitled_id: 0,
            tab_hover: None,
            tab_switcher: None,
            breadcrumb_hover: None,
        };

        if !state.welcome_open {
            restore_session(&mut state);
        }
        state
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    SetThemeMode(ThemeMode),
    SetAccent(Accent),
    /// Commits a custom accent RGB color (roadmap item 11's color picker),
    /// overriding whichever preset `accent` names — see
    /// `theme::palette_custom`.
    SetCustomAccent(u8, u8, u8),
    /// "Reset to preset" — drops `custom_accent` back to `None`, so
    /// `accent`'s own built-in ramp applies again.
    ClearCustomAccent,
    /// Dragged one of the custom-color picker's RGB sliders — updates
    /// `custom_accent_draft` and live-previews it (`theme_preview`),
    /// without committing (see `custom_accent_draft`'s own doc comment).
    AdjustCustomAccentDraft(u8, u8, u8),
    ToggleHighContrast,
    /// Hovering a theme-mode/accent-preset/custom-color swatch in the
    /// settings panel, or dragging its RGB sliders — live-previews that
    /// choice everywhere (`State::theme_preview`) without committing it.
    /// Roadmap item 11.
    PreviewTheme(ThemePreview),
    /// The mouse left the swatch/slider being previewed, or the settings
    /// panel closed — reverts to the committed theme.
    ClearThemePreview,
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
    /// Clicked the status bar's language-server indicator — opens the
    /// "Background Tasks" popover (`State::background_tasks_open`).
    ToggleBackgroundTasks,
    /// Clicked the status bar's EOL (LF/CRLF) indicator.
    ToggleEolPicker,
    /// Picked a target from the EOL picker — see `EditorState::convert_eol`.
    ConvertEol(Eol),
    /// Clicked the status bar's language-mode indicator.
    ToggleLanguagePicker,
    /// Picked a language from the language-mode picker — see
    /// `EditorState::set_language`.
    SetEditorLanguage(Option<syntax::Language>),
    /// Clicked the status bar's encoding indicator.
    ToggleEncodingInfo,
    /// Clicked a diagnostic row in the Problems dock panel — opens (or
    /// focuses) `path` and moves the cursor to the diagnostic's start
    /// position, same as clicking a location in any other editor's problems
    /// list.
    OpenDiagnosticAt(PathBuf, CursorPos),
    /// Opens the panel if closed (docked), closes it if it's presented any
    /// other way — mirrors the title-bar button's old `ToggleAssist`
    /// behavior, and the mockup's own `toggleChat`.
    ChatToggle,
    /// `⇧⌘I` — opens the chat panel if it isn't active yet (same as
    /// `ChatToggle`'s "closed" branch) and always focuses the composer,
    /// unlike `ChatToggle` which closes an already-open panel instead.
    /// "Focus chat" (chat-panel UX pass, item 6).
    ChatFocus,
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
    /// Sends `String` as a complete new turn, bypassing the draft entirely —
    /// the empty thread's starter prompts (`ask_about_file_prompt`,
    /// `SUMMARIZE_PROJECT_PROMPT`, `fix_bug_prompt`) and the continuation
    /// row's "Retry"/"Regenerate"/"Continue" (chat-panel UX pass, items 7
    /// and 8). A no-op with no live session.
    ChatSendPrompt(String),
    /// "Try again" on the empty thread's failure state — bumps
    /// `chat_restart_token` to force the worker to respawn (see its own doc
    /// comment: reserved for exactly this since it was added).
    ChatRetryConnection,
    /// An event from the running `claude` subprocess (see `chat_worker`).
    Chat(ClaudeEvent),
    ChatApprovePermission(String),
    ChatDenyPermission(String),
    /// Expands/collapses a `permission_card`'s truncated diff preview — see
    /// `ChatThread::expanded_tools`.
    ChatToggleToolExpanded(String),
    /// Copies a single message's raw text (an operator prompt or a settled
    /// assistant reply — see `operator_row`/`assistant_row`'s own "Copy"
    /// buttons) to the system clipboard. Same `push_flash` confirmation as
    /// `CopyPath`.
    ChatCopyText(String),
    /// "Edit" on a past operator message — reloads its text into the
    /// composer, ready to tweak and resend (see the handler's own doc
    /// comment on why this can't rewrite the transcript itself).
    ChatEditMessage(String),
    /// Pressed the chat panel's edge resize handle.
    ChatResizeStarted,
    /// Cursor moved while resizing — carries the cursor's window-space X
    /// position; the new width is `window_width - x` since the handle sits
    /// on the panel's *left* edge (the panel itself is docked to the right).
    ChatResizeDragged(f32),
    ChatResizeEnded,
    /// The message list's own `on_scroll` — carries how many pixels the
    /// viewport currently sits above the true bottom
    /// (`Viewport::absolute_offset_reversed().y`, `0.0` exactly at the
    /// bottom). Drives `State::chat_pinned_to_bottom`.
    ChatScrolled(f32),
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
    /// The sessions picker's search box (`session_list_view`) — filters
    /// `state.chat_sessions` by title as the user types.
    ChatSessionFilterChanged(String),
    /// The chat panel's "Load earlier messages" row — only shown while
    /// `state.chat.history_truncated`. Kicks off `load_earlier_chat_history`,
    /// same background-task shape as `ChatToggleSessions`.
    LoadEarlierChatHistory,
    ChatFullHistoryLoaded(Vec<ClaudeEvent>),
    /// Switches the permission mode (Manual/Auto-Edit/Plan/Auto) — respawns
    /// the worker (see `State::chat_permission_mode`'s own doc comment),
    /// resuming the same session under the new mode.
    ChatSetPermissionMode(PermissionMode),
    /// Switches which backend the panel talks to (see `ChatProvider`) —
    /// respawns the worker under the new provider, same as
    /// `ChatSetPermissionMode`.
    ChatSetProvider(ChatProvider),
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
    /// "Current file" quick action — mentions the active tab's file with no
    /// file-picker round-trip. See `insert_current_file_context`.
    ChatMentionCurrentFile,
    /// "Selected text" quick action — folds the active editor's selection
    /// into the draft as a fenced code block. See `insert_selection_context`.
    ChatMentionSelection,
    /// "Active symbol" quick action — names the innermost enclosing
    /// function/type at the cursor. See `insert_active_symbol_context`.
    ChatMentionActiveSymbol,
    /// "Project root" quick action — names the open project. See
    /// `insert_project_root_context`.
    ChatMentionProjectRoot,
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
    /// A single physical keystroke's character, routed separately from
    /// `EditorInsertText` so auto-pairing (`EditorState::type_char`) only
    /// ever sees live typing — paste (`EditorPasteWithText`) and generated
    /// text (Enter's `"\n"`, Tab-as-indent's four spaces) go through
    /// `EditorInsertText` instead and are never candidates for pairing.
    EditorTypeChar(char),
    EditorBackspace,
    EditorDelete,
    /// `Tab` — block-indents a multi-line selection, else inserts four
    /// spaces at the cursor. See `EditorState::indent`.
    EditorIndent,
    /// `Shift+Tab` — see `EditorState::dedent`.
    EditorDedent,
    /// `Ctrl+/` — see `EditorState::toggle_comment`.
    EditorToggleComment,
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
    /// The editor's `scrollable` reported a new offset (and viewport size),
    /// in both axes now that the canvas scrolls horizontally too — stored so
    /// `EditorCanvas::draw` can skip lines outside the visible range, and so
    /// Find/Go to Line/`scroll_cursor_into_view` know whether the target is
    /// already on-screen.
    EditorScrolled {
        offset: f32,
        viewport_height: f32,
        offset_x: f32,
        viewport_width: f32,
    },
    CaretTick,
    /// Fires only while an edit is pending; runs the deferred per-edit work
    /// once the buffer has been still for `EDIT_SETTLE`.
    EditSettleTick,
    Lsp(LspEvent),
    CopilotCompletion(CopilotCompletionEvent),
    /// `Tab` while a ghost-text suggestion is showing — checked before both
    /// the LSP completion-popup intercept and a real indent, in
    /// `Message::EditorIndent`'s own handler.
    AcceptGhostCompletion,
    /// `Escape` while a ghost-text suggestion is showing — see
    /// `editor_canvas.rs`'s own `handle_key`, the only place this is sent
    /// from (guarded there so a plain Escape with nothing showing still
    /// falls through to the app-wide Escape handling, same as before).
    DismissGhostCompletion,
    /// A debounced batch of on-disk changes from `file_watcher` — an edit
    /// made outside DevScribe (another terminal, `git checkout`, and later
    /// the AI Chat Assist panel's own file edits).
    FilesChanged(Vec<WatchEvent>),
    JsonToggle(String),
    JsonToggleTextMode,
    MarkdownToggleTextMode,
    /// A link clicked in the Markdown preview (`markdown_view.rs`) — handed
    /// to `open_externally`, the OS's default handler for it.
    OpenMarkdownLink(String),
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
    /// `Ctrl+G` — opens the command palette pre-filled with `:`, ready for a
    /// line number; see `filtered_palette_entries`'s `:N` handling. A no-op
    /// with no active file tab, same guard `PaletteAction::GoToLine` itself
    /// applies.
    OpenGoToLine,
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
    /// Turns the `copilot_completion_worker` subscription on/off — same
    /// shape as `ToggleLspEnabled`, for GitHub Copilot inline suggestions
    /// rather than language servers.
    ToggleCopilotInline,
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
    SetTabSize(u8),
    ToggleShowLineNumbers,
    ToggleWordWrap,
    /// The mouse entered/left a tab bar entry — starts/clears the hover
    /// preview's dwell timer (`State::tab_hover`). Mirrors
    /// `EditorHoverMove`/`EditorHoverLeave`'s shape.
    TabHoverStart(TabKey),
    TabHoverEnd(TabKey),
    /// Ticks while `State::tab_hover` is pending, purely to force a `view()`
    /// rebuild once the dwell elapses — same shape as `HoverDebounceTick`.
    TabPreviewTick,
    /// The mouse entered/left a breadcrumb segment — starts/clears the
    /// hover-context tooltip's dwell timer (`State::breadcrumb_hover`,
    /// roadmap item 10). Mirrors `TabHoverStart`/`TabHoverEnd`.
    BreadcrumbHoverStart(usize),
    BreadcrumbHoverEnd(usize),
    /// Ticks while `State::breadcrumb_hover` is pending — same shape as
    /// `TabPreviewTick`.
    BreadcrumbPreviewTick,
    /// Clicked a breadcrumb segment — moves the cursor to that scope's
    /// start and scrolls it into view.
    JumpToBreadcrumb(usize),
    /// Ctrl+Tab (`delta: 1`) / Ctrl+Shift+Tab (`delta: -1`) — opens the quick
    /// switcher (roadmap item 2) if it isn't already showing, snapshotting
    /// the current open-tab order, then steps `selected` through it on every
    /// further press while held.
    StepTabSwitcher(i32),
    /// Fires whenever Ctrl stops being held (`ModifiersChanged`) — commits
    /// whichever entry the switcher had selected and closes it. A no-op
    /// whenever the switcher isn't open, so this is safe to emit
    /// unconditionally from the global modifiers-changed handler.
    ConfirmTabSwitcher,
    /// A direct click on a switcher entry — selects and confirms it in one
    /// step, without needing Ctrl to be released.
    SelectTabSwitcherEntry(TabKey),
    CloseTabSwitcher,
    DismissToast(u64),
    PruneToasts,
    EditorSave,
    ToggleFind,
    CloseFind,
    FindQueryChanged(String),
    FindNext,
    FindPrev,
    ToggleReplace,
    ReplaceQueryChanged(String),
    ReplaceOne,
    /// "Replace All" button — first press asks for confirmation
    /// (`FindState::confirm_replace_all`); doesn't touch the buffer itself.
    ReplaceAll,
    /// The confirmation prompt's "Yes" — actually performs the replacement.
    ConfirmReplaceAll,
    /// The confirmation prompt's "No" — dismisses it without replacing.
    CancelReplaceAll,
    ToggleFindHelp,
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
    /// The first click on a changed line's gutter marker — arms the
    /// confirm/cancel step (`EditorState::pending_revert_line`) rather than
    /// reverting immediately.
    PromptRevertLine { line: usize },
    /// Dismisses `EditorState::pending_revert_line` without reverting —
    /// fired by a click away from the armed line, or by Escape.
    CancelRevertLine,
    /// The confirm step: a second click on the already-armed line's gutter
    /// marker. Reverts just that line (or, for `GutterMark::RemovedAbove`,
    /// re-inserts the deleted lines above it) back to its `HEAD` content.
    /// One undo step.
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
    /// The mouse (not dragging) moved onto a new `(line, col)` cell of the
    /// active editor's canvas — starts/restarts the hover dwell timer. Only
    /// published on an actual cell change (`editor_canvas::CanvasState`
    /// tracks the last cell), not on every raw `CursorMoved` event.
    EditorHoverMove { line: usize, col: usize },
    /// The mouse left the active editor's canvas entirely.
    EditorHoverLeave,
    /// Fires the `LspCommand::Hover` request for whatever position has been
    /// resting long enough (`EditorState::due_hover_request`) — only
    /// subscribed to while some editor actually has a pending hover
    /// position, same shape as `SearchDebounceTick`.
    HoverDebounceTick,
    /// Ctrl/Cmd+Click on `(line, col)`, or `F12` on the cursor's own
    /// position — "Go to Definition". A single result jumps there directly;
    /// more than one opens the Locations panel (`apply_locations`).
    GoToDefinition { line: usize, col: usize },
    /// `Shift+F12` on the cursor's position — "Find All References" across
    /// the whole project.
    FindReferences { line: usize, col: usize },
    ToggleReferencesPanel,
    /// A row in the Locations dock panel was clicked.
    JumpToLocation(PathBuf, CursorPos),
    Noop,
}

pub fn update(state: &mut State, message: Message) -> iced::Task<Message> {
    match message {
        Message::SetThemeMode(mode) => set_theme_mode(state, mode),
        Message::SetAccent(accent) => set_accent(state, accent),
        Message::SetCustomAccent(r, g, b) => {
            state.custom_accent = Some((r, g, b));
            state.custom_accent_draft = (r, g, b);
            state.theme_preview = None;
            persist_settings(state);
        }
        Message::ClearCustomAccent => {
            state.custom_accent = None;
            persist_settings(state);
        }
        Message::AdjustCustomAccentDraft(r, g, b) => {
            state.custom_accent_draft = (r, g, b);
            state.theme_preview = Some(ThemePreview {
                theme_mode: state.theme_mode,
                accent: state.accent,
                custom_accent: Some((r, g, b)),
                high_contrast: state.high_contrast,
            });
        }
        Message::ToggleHighContrast => {
            state.high_contrast = !state.high_contrast;
            persist_settings(state);
        }
        Message::PreviewTheme(preview) => {
            state.theme_preview = Some(preview);
        }
        Message::ClearThemePreview => {
            state.theme_preview = None;
        }
        Message::SelectOpenTab(key) => {
            if state.open_tabs.iter().any(|t| t.key() == key) {
                state.active_tab = Some(key);
                persist_session(state);
            }
        }
        Message::CloseTab(key) => close_tab(state, &key),
        Message::CloseActiveTab => {
            if let Some(key) = state.active_tab.clone() {
                close_tab(state, &key);
            }
        }
        Message::TabHoverStart(key) => {
            state.tab_hover = Some((key, Instant::now()));
        }
        Message::TabHoverEnd(key) => {
            if state.tab_hover.as_ref().is_some_and(|(k, _)| *k == key) {
                state.tab_hover = None;
            }
        }
        // No-op on purpose — this tick exists solely to force a `view()`
        // rebuild once `State::tab_hover`'s dwell elapses, the same shape as
        // `HoverDebounceTick`. `ui::tab_bar::hover_preview` is what actually
        // checks the elapsed time and decides whether to render anything.
        Message::TabPreviewTick => {}
        Message::BreadcrumbHoverStart(index) => {
            state.breadcrumb_hover = Some((index, Instant::now()));
        }
        Message::BreadcrumbHoverEnd(index) => {
            if state.breadcrumb_hover.as_ref().is_some_and(|(i, _)| *i == index) {
                state.breadcrumb_hover = None;
            }
        }
        // No-op on purpose, same reasoning as `TabPreviewTick`.
        Message::BreadcrumbPreviewTick => {}
        Message::JumpToBreadcrumb(index) => {
            state.breadcrumb_hover = None;
            let Some(path) = active_file_path(state) else { return iced::Task::none() };
            let font_size = state.editor_font_size;
            let word_wrap = state.word_wrap;
            let Some(editor) = find_editor_mut(state, &path) else { return iced::Task::none() };
            let crumbs = editor.breadcrumbs();
            let Some(crumb) = crumbs.get(index) else { return iced::Task::none() };
            let char_idx = editor.document.text().byte_to_char(crumb.start_byte);
            let (line, col) = editor.document.line_col(char_idx);
            editor.cursor = CursorPos { line, col };
            editor.selection_anchor = None;
            return center_line_in_viewport(editor, font_size, word_wrap, line);
        }
        Message::StepTabSwitcher(delta) => {
            if let Some(switcher) = state.tab_switcher.as_mut() {
                let len = switcher.entries.len() as i32;
                switcher.selected = (switcher.selected as i32 + delta).rem_euclid(len) as usize;
            } else {
                let entries = tab_switcher_entries(state);
                // Nothing to switch *between* with 0 or 1 open tabs.
                if entries.len() >= 2 {
                    let current =
                        entries.iter().position(|k| Some(k) == state.active_tab.as_ref()).unwrap_or(0);
                    let selected = (current as i32 + delta).rem_euclid(entries.len() as i32) as usize;
                    state.tab_switcher = Some(TabSwitcherState { entries, selected });
                }
            }
        }
        Message::ConfirmTabSwitcher => {
            if let Some(switcher) = state.tab_switcher.take()
                && let Some(key) = switcher.entries.get(switcher.selected).cloned()
            {
                return switch_to_tab(state, &key);
            }
        }
        Message::SelectTabSwitcherEntry(key) => {
            state.tab_switcher = None;
            return switch_to_tab(state, &key);
        }
        Message::CloseTabSwitcher => state.tab_switcher = None,
        Message::FocusSearchTab => focus_search(state),
        Message::ChatToggle => toggle_chat(state),
        Message::ChatFocus => {
            if !chat_is_active(state) {
                open_chat_as_tab(state);
                persist_settings(state);
                persist_session(state);
            }
            return focus_chat_input();
        }
        Message::ChatToggleViewMenu => {
            state.chat_view_menu_open = !state.chat_view_menu_open;
            if !state.chat_view_menu_open {
                return focus_chat_input();
            }
        }
        Message::ChatDock => {
            leave_chat_tab(state);
            state.chat_mode = ChatMode::Docked;
            state.chat_view_menu_open = false;
            persist_settings(state);
            persist_session(state);
            return focus_chat_input();
        }
        Message::ChatCollapse => {
            leave_chat_tab(state);
            state.chat_mode = ChatMode::Collapsed;
            state.chat_view_menu_open = false;
            persist_settings(state);
            persist_session(state);
        }
        Message::ChatOpenTab => {
            state.chat_tab_open = true;
            state.chat_mode = ChatMode::Closed;
            state.active_tab = Some(TabKey::Chat);
            state.chat_view_menu_open = false;
            persist_settings(state);
            persist_session(state);
            return focus_chat_input();
        }
        Message::ChatDockFromTab => {
            leave_chat_tab(state);
            state.chat_mode = ChatMode::Docked;
            state.chat_view_menu_open = false;
            persist_settings(state);
            persist_session(state);
            return focus_chat_input();
        }
        Message::ChatCloseTab => {
            leave_chat_tab(state);
            persist_session(state);
        }
        Message::ChatInputAction(action) => state.chat.input.perform(action),
        Message::ChatSubmit => {
            submit_chat_prompt(state);
            // `submit_chat_prompt` (via `send_chat_text`) already re-pins
            // `chat_pinned_to_bottom`, but the actual scroll only happens
            // the next time `handle_chat_event` runs — batch an immediate
            // snap here too so sending doesn't wait on the worker's next
            // event to visibly jump to the bottom.
            return iced::Task::batch([iced::widget::operation::snap_to_end(chat_scroll_id()), focus_chat_input()]);
        }
        Message::ChatSendPrompt(text) => {
            if state.chat.sender.is_some() {
                send_chat_text(state, text);
            }
            return iced::Task::batch([iced::widget::operation::snap_to_end(chat_scroll_id()), focus_chat_input()]);
        }
        Message::ChatRetryConnection => {
            state.chat_restart_token += 1;
            state.chat.status = ChatStatus::Starting;
        }
        Message::ChatSetPermissionMode(mode) => {
            state.chat_permission_mode = mode;
            return focus_chat_input();
        }
        Message::ChatSetProvider(provider) => {
            state.chat_provider = provider;
            return focus_chat_input();
        }
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
        Message::ChatToggleActions => {
            // Guarded on `chat_is_active` (rather than trusting every caller
            // to check first) because `⇧⌘U`'s global shortcut has no way to
            // know whether the panel is even on screen — without this, it'd
            // flip the flag regardless, and the popup would then pop open
            // unprompted the next time the panel *did* open.
            if chat_is_active(state) {
                state.chat_actions_open = !state.chat_actions_open;
                if !state.chat_actions_open {
                    return focus_chat_input();
                }
            }
        }
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
            return focus_chat_input();
        }
        Message::ChatMentionCurrentFile => {
            state.chat_actions_open = false;
            insert_current_file_context(state);
            return focus_chat_input();
        }
        Message::ChatMentionSelection => {
            state.chat_actions_open = false;
            insert_selection_context(state);
            return focus_chat_input();
        }
        Message::ChatMentionActiveSymbol => {
            state.chat_actions_open = false;
            insert_active_symbol_context(state);
            return focus_chat_input();
        }
        Message::ChatMentionProjectRoot => {
            state.chat_actions_open = false;
            insert_project_root_context(state);
            return focus_chat_input();
        }
        Message::ChatShowModel => {
            state.chat_actions_open = false;
            if state.chat.sender.is_some() {
                send_chat_text(state, "/model".to_string());
            }
            return focus_chat_input();
        }
        Message::ChatShowUsage => {
            state.chat_actions_open = false;
            if state.chat.sender.is_some() {
                send_chat_text(state, "/usage".to_string());
            }
            return focus_chat_input();
        }
        Message::ChatToggleThinking => {
            state.chat_thinking_enabled = !state.chat_thinking_enabled;
            state.chat_actions_open = false;
            if state.chat.sender.is_some() {
                let cmd = if state.chat_thinking_enabled { "/effort high" } else { "/effort auto" };
                send_chat_text(state, cmd.to_string());
            }
            return focus_chat_input();
        }
        Message::ChatToggleShellAccess => {
            state.chat_shell_access_enabled = !state.chat_shell_access_enabled;
            state.chat_actions_open = false;
            return focus_chat_input();
        }
        Message::Chat(event) => return handle_chat_event(state, event),
        Message::ChatApprovePermission(id) => {
            respond_permission(state, id, true, None);
            return focus_chat_input();
        }
        Message::ChatDenyPermission(id) => {
            respond_permission(state, id, false, Some("denied by user".to_string()));
            return focus_chat_input();
        }
        Message::ChatToggleToolExpanded(id) => {
            if !state.chat.expanded_tools.remove(&id) {
                state.chat.expanded_tools.insert(id);
            }
        }
        Message::ChatCopyText(text) => {
            push_flash(state, "COPIED TO CLIPBOARD");
            return iced::clipboard::write(text);
        }
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
        Message::ChatScrolled(distance_from_bottom) => {
            state.chat_pinned_to_bottom = distance_from_bottom <= CHAT_SCROLL_PIN_SLACK;
        }
        Message::ChatNewSession => {
            if !chat_is_active(state) {
                open_chat_as_tab(state);
                persist_settings(state);
                persist_session(state);
            }
            state.chat_session_id = claude_agent::new_session_id();
            state.chat = ChatThread::default();
            state.chat_pinned_to_bottom = true;
            state.chat_sessions_open = false;
            state.chat_actions_open = false;
            return focus_chat_input();
        }
        Message::ChatResumeSession(id) => {
            state.chat_session_id = id;
            state.chat = ChatThread::default();
            state.chat_pinned_to_bottom = true;
            state.chat_sessions_open = false;
            return focus_chat_input();
        }
        Message::ChatToggleSessions => {
            state.chat_sessions_open = !state.chat_sessions_open;
            state.chat_session_filter.clear();
            if state.chat_sessions_open {
                return start_loading_chat_sessions(state);
            }
            return focus_chat_input();
        }
        Message::ChatSessionFilterChanged(text) => {
            state.chat_session_filter = text;
        }
        Message::ChatEditMessage(text) => {
            // "Edit" on a past turn can't rewrite `claude`'s own transcript
            // (see `send_chat_text`'s doc comment on why turns are just
            // prompts, not a rewindable state) — the closest useful thing is
            // reloading it into the composer so it's one click away from
            // being sent again, tweaked.
            state.chat.input = iced::widget::text_editor::Content::with_text(&text);
            return focus_chat_input();
        }
        Message::ChatSessionsLoaded(sessions) => state.chat_sessions = sessions,
        Message::LoadEarlierChatHistory => return load_earlier_chat_history(state),
        Message::ChatFullHistoryLoaded(events) => {
            // Replaces rather than prepends: every event a saved transcript
            // can replay through `parse_event_line` only ever appends to
            // `messages` (or mutates an existing entry found by id), so
            // replaying the *complete* history from an empty list lands in
            // the same place a true prepend would, without needing a
            // separate reducer that operates on a detached list first.
            state.chat.messages.clear();
            state.chat.history_truncated = false;
            // "Load earlier messages" is itself a "let me read from the
            // start" action — unpin first so none of the replayed events
            // below (each routed through the same `handle_chat_event` a
            // live turn uses) snap the now much longer transcript back down
            // to the end before the user has seen any of what they asked to
            // load.
            state.chat_pinned_to_bottom = false;
            for event in events {
                let _ = handle_chat_event(state, event);
            }
        }
        Message::ToggleProjects => state.projects_open = !state.projects_open,
        Message::ToggleOverflow => state.overflow_open = !state.overflow_open,
        Message::CollapseSidebar => {
            state.sidebar_collapsed = true;
            // Closing menus that were anchored to sidebar content about to
            // disappear, same as the mockup's own `collapseSidebar` handler.
            state.projects_open = false;
            state.ctx_menu = None;
            persist_session(state);
        }
        Message::ExpandSidebar => {
            state.sidebar_collapsed = false;
            persist_session(state);
        }
        Message::SidebarResizeStarted => state.sidebar_resizing = true,
        Message::SidebarResizeDragged(x) => {
            if state.sidebar_resizing {
                state.sidebar_width = x.clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH);
            }
        }
        Message::SidebarResizeEnded => {
            state.sidebar_resizing = false;
            persist_session(state);
        }
        Message::ToggleChangesPanel => {
            state.changes_panel_open = !state.changes_panel_open;
            persist_session(state);
        }
        Message::ToggleProblemsPanel => {
            state.problems_panel_open = !state.problems_panel_open;
            persist_session(state);
        }
        Message::ToggleBackgroundTasks => {
            let opening = !state.background_tasks_open;
            close_status_bar_popovers(state);
            state.background_tasks_open = opening;
        }
        Message::ToggleEolPicker => {
            let opening = !state.eol_picker_open;
            close_status_bar_popovers(state);
            state.eol_picker_open = opening;
        }
        Message::ConvertEol(target) => {
            state.eol_picker_open = false;
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
            {
                editor.convert_eol(target);
            }
        }
        Message::ToggleLanguagePicker => {
            let opening = !state.language_picker_open;
            close_status_bar_popovers(state);
            state.language_picker_open = opening;
        }
        Message::SetEditorLanguage(language) => {
            state.language_picker_open = false;
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
            {
                editor.set_language(language);
            }
        }
        Message::ToggleEncodingInfo => {
            let opening = !state.encoding_info_open;
            close_status_bar_popovers(state);
            state.encoding_info_open = opening;
        }
        Message::OpenDiagnosticAt(path, pos) => {
            open_or_focus_file(state, path.clone());
            if let Some(editor) = find_editor_mut(state, &path) {
                editor.click(pos.line, pos.col, false);
            }
        }
        Message::OpenDiffFor(path) => open_or_focus_diff(state, path),
        Message::ViewWorkingTreeDiff => view_working_tree_diff(state),
        Message::SelectFile(path) => open_or_focus_file(state, path),
        Message::MarkdownToggleTextMode => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
            {
                editor.markdown_text_mode = !editor.markdown_text_mode;
            }
        }
        Message::OpenMarkdownLink(url) => open_externally(&url),
        Message::EditorInsertText(text) => {
            if let Some(path) = active_file_path(state) {
                // Intercept Enter when the completion popup is open — select
                // the highlighted item instead of inserting a literal
                // newline. Tab has the equivalent intercept in
                // `Message::EditorIndent`, since it no longer routes through
                // here (see `EditorState::indent`).
                let completions_open = find_editor(state, &path)
                    .is_some_and(|e| e.completions.is_some());
                if completions_open && text == "\n" {
                    return update(state, Message::CompletionSelect);
                }

                if let Some(editor) = find_editor_mut(state, &path) {
                    // A trigger char (`.`/`:`) always starts its own fresh
                    // request below, so any popup already open gets closed
                    // outright rather than fuzzy-matched against it. A word
                    // character (letter/digit/`_`) instead narrows whatever
                    // popup is already open — see `refilter_completions`.
                    // Anything else (space, punctuation, a pasted chunk with
                    // any of those in it, ...) closes it: none of that can
                    // ever be part of an identifier the popup would match.
                    if editor.completions_active() && !is_word_text(&text) {
                        editor.close_completions();
                    }
                    editor.clear_hover();
                    editor.insert_text(&text);
                    editor.refilter_completions();
                }
                mark_edited(state, &path);
                maybe_trigger_completion(state, &path, &text);
                maybe_trigger_signature_help(state, &path, &text);
                return scroll_cursor_into_view(state);
            }
        }
        Message::EditorTypeChar(ch) => {
            if let Some(path) = active_file_path(state) {
                if let Some(editor) = find_editor_mut(state, &path) {
                    // Same stale-popup-closing rule as `EditorInsertText`.
                    let text = ch.to_string();
                    if editor.completions_active() && !is_word_text(&text) {
                        editor.close_completions();
                    }
                    editor.clear_hover();
                    editor.type_char(ch);
                    editor.refilter_completions();
                }
                mark_edited(state, &path);
                maybe_trigger_completion(state, &path, &ch.to_string());
                maybe_trigger_signature_help(state, &path, &ch.to_string());
                return scroll_cursor_into_view(state);
            }
        }
        Message::EditorBackspace => {
            if let Some(path) = active_file_path(state) {
                // A Backspace with nothing to delete changed nothing, so it
                // must not arm the settle timer (and the reparse, git diff and
                // LSP `didChange` behind it) either.
                if !find_editor_mut(state, &path).is_some_and(|e| e.backspace()) {
                    return iced::Task::none();
                }
                // Backspacing past the trigger character closes the popup
                // outright (`refilter_completions` handles that); backspacing
                // within the typed prefix instead re-narrows it — the popup
                // must not simply stay frozen at whatever it last showed.
                if let Some(editor) = find_editor_mut(state, &path) {
                    editor.clear_hover();
                    editor.refilter_completions();
                }
                mark_edited(state, &path);
                return scroll_cursor_into_view(state);
            }
        }
        Message::EditorDelete => {
            if let Some(path) = active_file_path(state) {
                // Forward-delete while a completion is open is an unusual
                // enough combination (it deletes text *ahead* of the typed
                // prefix, not part of it) that closing outright is simpler
                // and safer than trying to make it a sensible edit to the
                // prefix.
                if let Some(editor) = find_editor_mut(state, &path) {
                    editor.close_completions();
                    editor.clear_hover();
                }
                if !find_editor_mut(state, &path).is_some_and(|e| e.delete_forward()) {
                    return iced::Task::none();
                }
                mark_edited(state, &path);
                return scroll_cursor_into_view(state);
            }
        }
        Message::EditorIndent => {
            if let Some(path) = active_file_path(state) {
                // A ghost-text suggestion, if showing, wins over both the LSP
                // popup and a real indent — same precedence VS Code gives its
                // own inline suggestions over other Tab behavior. The two
                // can't both be showing at once in practice (see
                // `GhostCompletion`'s own doc comment), but check this first
                // regardless, same as the completions check right below it.
                let ghost_showing = find_editor(state, &path).is_some_and(|e| e.ghost_completion.as_ref().is_some_and(|g| g.at == e.cursor));
                if ghost_showing {
                    return update(state, Message::AcceptGhostCompletion);
                }
                // Same completion-popup intercept `EditorInsertText` gives
                // Enter — Tab with the popup open selects the highlighted
                // item rather than indenting.
                let completions_open = find_editor(state, &path).is_some_and(|e| e.completions.is_some());
                if completions_open {
                    return update(state, Message::CompletionSelect);
                }
                // Tab with a snippet expansion in progress jumps to the next
                // placeholder instead of indenting — see `advance_snippet`.
                if find_editor_mut(state, &path).is_some_and(|e| e.advance_snippet()) {
                    return scroll_cursor_into_view(state);
                }
                let tab_size = state.tab_size;
                if let Some(editor) = find_editor_mut(state, &path) {
                    editor.indent(tab_size);
                }
                mark_edited(state, &path);
                return scroll_cursor_into_view(state);
            }
        }
        Message::EditorDedent => {
            if let Some(path) = active_file_path(state) {
                let tab_size = state.tab_size;
                if !find_editor_mut(state, &path).is_some_and(|e| e.dedent(tab_size)) {
                    return iced::Task::none();
                }
                mark_edited(state, &path);
                return scroll_cursor_into_view(state);
            }
        }
        Message::EditorToggleComment => {
            if let Some(path) = active_file_path(state) {
                if !find_editor_mut(state, &path).is_some_and(|e| e.toggle_comment()) {
                    return iced::Task::none();
                }
                mark_edited(state, &path);
                return scroll_cursor_into_view(state);
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
                            editor.close_completions();
                        }
                    }
                }
                editor.close_snippet();
                editor.move_cursor(dir, extend);
            }
            return scroll_cursor_into_view(state);
        }
        Message::EditorClick { line, col, extend } => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
            {
                editor.close_completions();
                editor.close_snippet();
                editor.clear_hover();
                editor.click(line, col, extend);
            }
        }
        Message::EditorSelectWord { line, col } => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
            {
                editor.close_completions();
                editor.close_snippet();
                editor.select_word_at(line, col);
            }
        }
        Message::EditorSelectLine { line } => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
            {
                editor.close_completions();
                editor.close_snippet();
                editor.select_line_at(line);
            }
        }
        Message::EditorSelectAll => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
            {
                editor.close_completions();
                editor.close_snippet();
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
                return iced::Task::batch([
                    iced::clipboard::write(text),
                    scroll_cursor_into_view(state),
                ]);
            }
        }
        Message::EditorUndo => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
                && editor.undo()
            {
                editor.close_completions();
                editor.close_snippet();
                mark_edited(state, &path);
                // Undo can land the caret anywhere — including a screenful
                // away from wherever the view currently sits.
                return scroll_cursor_into_view(state);
            }
        }
        Message::EditorRedo => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
                && editor.redo()
            {
                editor.close_completions();
                editor.close_snippet();
                mark_edited(state, &path);
                return scroll_cursor_into_view(state);
            }
        }
        Message::EditorPaste => return iced::clipboard::read().map(Message::EditorPasteWithText),
        Message::EditorPasteWithText(text) => {
            if let Some(text) = text
                && let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
            {
                editor.close_completions();
                editor.close_snippet();
                editor.insert_text(&text);
                mark_edited(state, &path);
                return scroll_cursor_into_view(state);
            }
        }
        Message::EditorScrolled { offset, viewport_height, offset_x, viewport_width } => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
            {
                editor.scroll_offset = offset;
                editor.viewport_height = viewport_height;
                editor.scroll_offset_x = offset_x;
                editor.viewport_width = viewport_width;
            }
        }
        Message::CaretTick => state.caret_visible = !state.caret_visible,
        Message::EditSettleTick => {
            if state.edit_settled_at.is_some_and(|at| at.elapsed() >= EDIT_SETTLE) {
                flush_pending_edits(state);
                maybe_trigger_ghost_completion(state);
            }
        }
        Message::Lsp(event) => match event {
            LspEvent::Ready(sender) => {
                let was_starting = matches!(state.lsp_status, LspStatus::Starting);
                state.lsp_status = LspStatus::Ready;
                state.lsp_progress.clear();
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
                    // Discard stale results: only apply if the anchor the
                    // request was sent for is still the one in effect.
                    let anchor = editor.completion_anchor;
                    if anchor.line == line as usize {
                        let line_text = editor.document.line_text(anchor.line);
                        let anchor_utf16 = char_col_to_utf16_col(&line_text, anchor.col);
                        if anchor_utf16 == character {
                            // `set_completions` re-filters against whatever's
                            // been typed since (`editor.cursor`, live) rather
                            // than showing the raw response — the user may
                            // have kept typing past the trigger while this
                            // was in flight.
                            editor.set_completions(items);
                        }
                    }
                }
            }
            LspEvent::Hover { uri, line, character, text } => {
                if let Some(path) = uri.to_file_path().ok()
                    && let Some(editor) = find_editor_mut(state, &path)
                {
                    editor.apply_hover_response(line, character, text);
                }
            }
            LspEvent::SignatureHelp { uri, line, character, signatures, active_signature, active_parameter } => {
                if let Some(path) = uri.to_file_path().ok()
                    && let Some(editor) = find_editor_mut(state, &path)
                {
                    editor.apply_signature_help_response(line, character, signatures, active_signature, active_parameter);
                }
            }
            LspEvent::Progress { token, title, message, percentage, done } => {
                if done {
                    state.lsp_progress.remove(&token);
                } else {
                    let entry = state.lsp_progress.entry(token).or_insert_with(|| LspProgressEntry {
                        title: title.clone().unwrap_or_default(),
                        message: None,
                        percentage: None,
                    });
                    if let Some(title) = title {
                        entry.title = title;
                    }
                    if message.is_some() {
                        entry.message = message;
                    }
                    if percentage.is_some() {
                        entry.percentage = percentage;
                    }
                }
            }
            LspEvent::Definition { locations, .. } => {
                return apply_locations(state, locations, "Definition");
            }
            LspEvent::References { locations, .. } => {
                return apply_locations(state, locations, "References");
            }
            LspEvent::NeedsInstall => {
                // Binary not on PATH and not in the managed dir — kick off
                // a background install and show progress in the status bar.
                if !matches!(state.lsp_status, LspStatus::Installing) {
                    if let Some(lang) = active_lsp_language(state) {
                        state.lsp_status = LspStatus::Installing;
                        state.lsp_progress.clear();
                        return start_server_install(lang);
                    }
                }
            }
            LspEvent::Unavailable(reason) => {
                state.lsp_status = LspStatus::Unavailable(reason.clone());
                state.lsp_progress.clear();
                state.lsp_sender = None;
                let name = active_server_name(state);
                push_toast(state, ToastKind::Warning, format!("{name} unavailable: {reason}"));
            }
        },
        Message::CopilotCompletion(event) => match event {
            CopilotCompletionEvent::Ready(sender) => {
                state.copilot_completion_status = CopilotCompletionStatus::Ready;
                state.copilot_completion_sender = Some(sender);
                for path in open_file_paths(state) {
                    send_copilot_did_open_for(state, &path);
                }
            }
            CopilotCompletionEvent::Suggestion { uri, line, character, item } => {
                if let Some(path) = uri.to_file_path().ok()
                    && let Some(editor) = find_editor_mut(state, &path)
                {
                    // Discard stale results: only apply if the cursor is
                    // still exactly where this request was sent for — see
                    // `GhostCompletion`'s own doc comment.
                    let cursor = editor.cursor;
                    if cursor.line == line as usize {
                        let line_text = editor.document.line_text(cursor.line);
                        if char_col_to_utf16_col(&line_text, cursor.col) == character {
                            editor.ghost_completion = item.and_then(|item| {
                                let insert_text = item.get("insertText").and_then(|v| v.as_str())?.to_string();
                                if insert_text.is_empty() {
                                    return None;
                                }
                                Some(GhostCompletion { at: cursor, insert_text, item })
                            });
                        }
                    }
                }
            }
            CopilotCompletionEvent::Unavailable(reason) => {
                state.copilot_completion_status = CopilotCompletionStatus::Unavailable(reason);
                state.copilot_completion_sender = None;
            }
        },
        Message::AcceptGhostCompletion => {
            if let Some(path) = active_file_path(state) {
                let ghost = find_editor(state, &path).and_then(|editor| {
                    let ghost = editor.ghost_completion.as_ref()?;
                    (ghost.at == editor.cursor).then(|| ghost.clone())
                });
                if let Some(ghost) = ghost {
                    if let Some(editor) = find_editor_mut(state, &path) {
                        editor.close_ghost_completion();
                        editor.insert_text(&ghost.insert_text);
                    }
                    mark_edited(state, &path);
                    if let Some(sender) = state.copilot_completion_sender.as_mut() {
                        let _ = sender.try_send(CopilotCompletionCommand::Accepted { item: ghost.item });
                    }
                    return scroll_cursor_into_view(state);
                }
            }
        }
        Message::DismissGhostCompletion => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
            {
                editor.close_ghost_completion();
            }
        }
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
            persist_session(state);
        }
        Message::JsonToggle(key) => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
                && !editor.json_collapsed.remove(&key)
            {
                editor.json_collapsed.insert(key);
            }
        }
        Message::JsonToggleTextMode => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
            {
                editor.json_text_mode = !editor.json_text_mode;
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
        Message::OpenGoToLine => {
            if active_file_path(state).is_some() {
                state.palette_open = true;
                state.palette_query = ":".to_string();
                state.palette_selected = 0;
                state.settings_open = false;
                return iced::widget::operation::focus(palette_query_id());
            }
        }
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
            state.theme_preview = None;
        }
        Message::CloseSettings => {
            state.settings_open = false;
            state.theme_preview = None;
        }
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
                state.lsp_progress.clear();
            } else {
                // Dropping the subscription tears down the running worker
                // (`kill_on_drop` kills its `rust-analyzer` child); clear
                // the stale sender and every open editor's diagnostics so
                // nothing lingers from before the server went away.
                state.lsp_sender = None;
                state.lsp_status = LspStatus::Disabled;
                state.lsp_progress.clear();
                for tab in &mut state.open_tabs {
                    if let OpenTab::File(editor) = tab {
                        editor.diagnostics = Rc::new(Vec::new());
                    }
                }
            }
        }
        Message::ToggleCopilotInline => {
            state.copilot_inline_enabled = !state.copilot_inline_enabled;
            persist_settings(state);
            if state.copilot_inline_enabled {
                state.copilot_completion_status = CopilotCompletionStatus::Starting;
            } else {
                state.copilot_completion_sender = None;
                state.copilot_completion_status = CopilotCompletionStatus::Disabled;
                for tab in &mut state.open_tabs {
                    if let OpenTab::File(editor) = tab {
                        editor.close_ghost_completion();
                    }
                }
            }
        }
        Message::WindowUnfocused => {
            if state.save_on_focus_loss {
                save_all_dirty_files(state);
            }
            // A checkpoint for whatever drifted since the last discrete
            // session-changing action (`persist_session`'s other call
            // sites) without saving on every keystroke/cursor move — most
            // notably the active tab's cursor position, which otherwise
            // only gets captured on a tab switch/open/close. Unconditional,
            // unlike `save_on_focus_loss` above: that toggle is about
            // writing file *contents* to disk, a materially bigger
            // decision than recording where tabs/cursors/panels are.
            persist_session(state);
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
        Message::SetTabSize(size) => {
            state.tab_size = size.clamp(TAB_SIZE_MIN, TAB_SIZE_MAX);
            persist_settings(state);
        }
        Message::ToggleShowLineNumbers => {
            state.show_line_numbers = !state.show_line_numbers;
            persist_settings(state);
        }
        Message::ToggleWordWrap => {
            state.word_wrap = !state.word_wrap;
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
                    find.just_wrapped = false;
                    find.confirm_replace_all = false;
                }
                editor.refind();
            }
        }
        Message::FindNext => return find_step(state, 1),
        Message::FindPrev => return find_step(state, -1),
        Message::ToggleReplace => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
            {
                if let Some(find) = editor.find.as_mut() {
                    find.replace_open = !find.replace_open;
                    find.confirm_replace_all = false;
                } else {
                    let initial_query = editor
                        .selection()
                        .map(|(start, end)| editor.document.text().slice(start..end).to_string())
                        .unwrap_or_default();
                    editor.find = Some(FindState {
                        query: initial_query,
                        replace_open: true,
                        ..FindState::default()
                    });
                    editor.refind();
                }
                return iced::widget::operation::focus(find_query_id());
            }
        }
        Message::ReplaceQueryChanged(text) => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
                && let Some(find) = editor.find.as_mut()
            {
                find.replace_query = text;
            }
        }
        Message::ReplaceOne => return replace_current_match(state),
        Message::ReplaceAll => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
                && let Some(find) = editor.find.as_mut()
                && !find.matches.is_empty()
            {
                find.confirm_replace_all = true;
            }
        }
        Message::ConfirmReplaceAll => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
                && let Some(find) = editor.find.as_mut()
            {
                find.confirm_replace_all = false;
            }
            replace_all_matches(state);
        }
        Message::CancelReplaceAll => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
                && let Some(find) = editor.find.as_mut()
            {
                find.confirm_replace_all = false;
            }
        }
        Message::ToggleFindHelp => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
                && let Some(find) = editor.find.as_mut()
            {
                find.help_open = !find.help_open;
            }
        }
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
        Message::PromptRevertLine { line } => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
            {
                editor.pending_revert_line = Some(line);
            }
        }
        Message::CancelRevertLine => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
            {
                editor.pending_revert_line = None;
            }
        }
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
                    editor.close_completions();
                }
                return iced::Task::none();
            }
            let signature_help_open = active_file_path(state)
                .and_then(|ref path| find_editor(state, path))
                .is_some_and(|e| e.signature_help.is_some());
            if signature_help_open {
                if let Some(path) = active_file_path(state)
                    && let Some(editor) = find_editor_mut(state, &path)
                {
                    editor.signature_help = None;
                }
                return iced::Task::none();
            }
            let snippet_active = active_file_path(state)
                .and_then(|ref path| find_editor(state, path))
                .is_some_and(|e| e.snippet_active());
            if snippet_active {
                if let Some(path) = active_file_path(state)
                    && let Some(editor) = find_editor_mut(state, &path)
                {
                    editor.close_snippet();
                }
                return iced::Task::none();
            }
            let find_open = active_file_path(state)
                .and_then(|path| find_editor(state, &path))
                .is_some_and(|editor| editor.find.is_some());
            let revert_line_armed = active_file_path(state)
                .and_then(|path| find_editor(state, &path))
                .is_some_and(|editor| editor.pending_revert_line.is_some());
            if find_open {
                if let Some(path) = active_file_path(state)
                    && let Some(editor) = find_editor_mut(state, &path)
                {
                    editor.find = None;
                }
            } else if revert_line_armed {
                if let Some(path) = active_file_path(state)
                    && let Some(editor) = find_editor_mut(state, &path)
                {
                    editor.pending_revert_line = None;
                }
            } else if state.tab_switcher.is_some() {
                state.tab_switcher = None;
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
                    state.lsp_progress.clear();
                    let name = active_server_name(state);
                    push_toast(state, ToastKind::Success, format!("{name} installed"));
                }
                Err(reason) => {
                    state.lsp_status = LspStatus::Unavailable(reason.clone());
                    state.lsp_progress.clear();
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
                let selected = find_editor(state, &path).and_then(|editor| {
                    let items = editor.completions.as_ref()?;
                    let sel = editor.completion_selected.min(items.len().saturating_sub(1));
                    let item = items.get(sel)?.clone();
                    Some((item, editor.completion_anchor, editor.cursor))
                });
                if let Some((item, anchor, cursor)) = selected {
                    if let Some(editor) = find_editor_mut(state, &path) {
                        editor.close_completions();
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
                        let raw = item.insert_text.as_deref().unwrap_or(item.label.as_str());
                        // Snippets (`fn ${1:name}(${2:args}) {\n    $0\n}` and
                        // the like) need their `$`-placeholder syntax parsed
                        // out before insertion — inserting `raw` verbatim
                        // would put the literal dollar-sign syntax in the
                        // buffer. Plain-text items (`insert_text_format`
                        // unset or `PlainText`) just insert as-is, same as
                        // before.
                        if item.insert_text_format == Some(lsp::InsertTextFormat::SNIPPET) {
                            let parsed = crate::snippet::parse(raw);
                            editor.insert_text(&parsed.text);
                            let stops = parsed
                                .tab_stops
                                .into_iter()
                                .map(|s| (start + s.range.0, start + s.range.1))
                                .collect();
                            editor.begin_snippet(stops);
                        } else {
                            editor.insert_text(raw);
                        }
                    }
                    mark_edited(state, &path);
                    return scroll_cursor_into_view(state);
                }
            }
        }
        Message::CloseCompletion => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
            {
                editor.close_completions();
            }
        }
        Message::EditorHoverMove { line, col } => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
            {
                editor.hover_move(line, col);
            }
        }
        Message::EditorHoverLeave => {
            if let Some(path) = active_file_path(state)
                && let Some(editor) = find_editor_mut(state, &path)
            {
                editor.clear_hover();
            }
        }
        Message::HoverDebounceTick => {
            if let Some(path) = active_file_path(state)
                && matches!(state.lsp_status, LspStatus::Ready)
                && let Some(pos) = find_editor(state, &path).and_then(|e| e.due_hover_request())
                && let Some(uri) = lsp_uri(&path)
            {
                let line_text = find_editor(state, &path)
                    .map(|e| e.document.line_text(pos.line))
                    .unwrap_or_default();
                let utf16_char = char_col_to_utf16_col(&line_text, pos.col);
                if let Some(sender) = state.lsp_sender.as_mut() {
                    let _ = sender.try_send(LspCommand::Hover {
                        uri,
                        line: pos.line as u32,
                        character: utf16_char,
                    });
                }
                if let Some(editor) = find_editor_mut(state, &path) {
                    editor.mark_hover_requested(pos);
                }
            }
        }
        Message::GoToDefinition { line, col } => {
            if let Some(path) = active_file_path(state)
                && matches!(state.lsp_status, LspStatus::Ready)
                && let Some(uri) = lsp_uri(&path)
            {
                let line_text = find_editor(state, &path).map(|e| e.document.line_text(line)).unwrap_or_default();
                let character = char_col_to_utf16_col(&line_text, col);
                if let Some(sender) = state.lsp_sender.as_mut() {
                    let _ = sender.try_send(LspCommand::GotoDefinition { uri, line: line as u32, character });
                }
            }
        }
        Message::FindReferences { line, col } => {
            if let Some(path) = active_file_path(state)
                && matches!(state.lsp_status, LspStatus::Ready)
                && let Some(uri) = lsp_uri(&path)
            {
                let line_text = find_editor(state, &path).map(|e| e.document.line_text(line)).unwrap_or_default();
                let character = char_col_to_utf16_col(&line_text, col);
                if let Some(sender) = state.lsp_sender.as_mut() {
                    let _ = sender.try_send(LspCommand::References { uri, line: line as u32, character });
                }
            }
        }
        Message::ToggleReferencesPanel => {
            state.references_open = !state.references_open;
        }
        Message::JumpToLocation(path, pos) => {
            // Left open, same as the Problems panel's `OpenDiagnosticAt` —
            // browsing several results one after another is the common case
            // for both, not a single pick-and-dismiss.
            open_or_focus_file(state, path.clone());
            if let Some(editor) = find_editor_mut(state, &path) {
                editor.click(pos.line, pos.col, false);
            }
            return scroll_cursor_into_view(state);
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
        PaletteAction::GoToLine(line) => return goto_line(state, line),
    }
    iced::Task::none()
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
        custom_accent: state.custom_accent,
        high_contrast: state.high_contrast,
        density: state.density,
        ui_font_scale: state.ui_font_scale,
        editor_font_size: state.editor_font_size,
        git_status_in_tree: state.git_status_in_tree,
        show_hidden_files: state.show_hidden_files,
        problem_lens_enabled: state.problem_lens_enabled,
        save_on_focus_loss: state.save_on_focus_loss,
        lsp_enabled: state.lsp_enabled,
        copilot_inline_enabled: state.copilot_inline_enabled,
        chat_mode: state.chat_mode,
        chat_panel_width: state.chat_panel_width,
        tab_size: state.tab_size,
        show_line_numbers: state.show_line_numbers,
        word_wrap: state.word_wrap,
    });
}

/// Builds the persisted-session shape (`session::Session`) from the live
/// `State` — the inverse of `restore_session`. `TabKey::Search`/`TabKey::Chat`
/// and untitled scratch buffers (`EditorState.document.path().is_none()`)
/// are never recorded — see `session::Session`'s doc for why.
fn capture_session(state: &State) -> session::Session {
    let open_tabs: Vec<session::SessionTab> = state
        .open_tabs
        .iter()
        .filter_map(|tab| match tab {
            OpenTab::File(editor) => {
                editor.document.path()?;
                Some(session::SessionTab {
                    path: editor.path.clone(),
                    is_diff: false,
                    cursor_line: editor.cursor.line,
                    cursor_col: editor.cursor.col,
                })
            }
            OpenTab::Diff(path) => {
                Some(session::SessionTab { path: path.clone(), is_diff: true, cursor_line: 0, cursor_col: 0 })
            }
        })
        .collect();
    let active_tab = state.active_tab.as_ref().and_then(|active| match active {
        TabKey::File(path) => open_tabs.iter().position(|t| !t.is_diff && &t.path == path),
        TabKey::Diff(path) => open_tabs.iter().position(|t| t.is_diff && &t.path == path),
        TabKey::Search | TabKey::Chat => None,
    });
    session::Session {
        open_tabs,
        active_tab,
        sidebar_width: state.sidebar_width,
        sidebar_collapsed: state.sidebar_collapsed,
        collapsed_dirs: state.collapsed_dirs.iter().cloned().collect(),
        changes_panel_open: state.changes_panel_open,
        problems_panel_open: state.problems_panel_open,
        chat_mode: settings::chat_mode_key(state.chat_mode).to_string(),
        chat_tab_open: state.chat_tab_open,
        chat_tab_active: state.active_tab == Some(TabKey::Chat),
    }
}

/// The single place any tab/sidebar/panel layout change gets persisted
/// (`session::save`), keyed by `state.root` — same shape as
/// `persist_settings`. A no-op with no project open (`state.root` is
/// meaningless then) or while `restore_session` is actively re-opening tabs
/// (`state.restoring_session`), so restoring a session can't stomp the very
/// file it's mid-read from with a partially-restored snapshot.
fn persist_session(state: &State) {
    if state.restoring_session || state.welcome_open {
        return;
    }
    session::save(&state.root, &capture_session(state));
}

/// Re-opens `state.root`'s persisted session (`session::load`), if one was
/// ever recorded for it — every open tab (with its cursor position), which
/// one was active, the sidebar/panel layout, and the AI Chat Assist panel's
/// presentation (open/closed, docked/collapsed/tab). Deliberately does *not*
/// restore which `claude` session was live — `state.chat_session_id` stays
/// whatever `reset_project_scoped_state` just minted, so a restored-open
/// panel always starts a brand new conversation rather than resuming the
/// last one. Called after a project's tree/git snapshot is already in place
/// (`apply_loaded_project`, and `State::default()`'s startup auto-reopen),
/// so opening each tab's file can immediately diff it against `HEAD` the
/// same as any other open.
///
/// A no-op, leaving the fresh-snapshot defaults already in `state` (every
/// directory collapsed, no tabs, chat closed), when nothing was ever saved
/// for this root — otherwise a project's very first-ever open would restore
/// an empty-but-not-default session (e.g. an unclamped `sidebar_width` of
/// `0.0`) instead of `snapshot_project`'s/`reset_project_scoped_state`'s
/// intended first-run defaults.
fn restore_session(state: &mut State) {
    let session = session::load(&state.root);
    if session == session::Session::default() {
        return;
    }
    state.restoring_session = true;
    for tab in &session.open_tabs {
        if tab.is_diff {
            open_or_focus_diff(state, tab.path.clone());
            continue;
        }
        open_or_focus_file(state, tab.path.clone());
        if let Some(editor) = find_editor_mut(state, &tab.path) {
            // Clamped rather than trusted outright: the file may have
            // shrunk (or been emptied) on disk since the session was saved.
            let line = tab.cursor_line.min(editor.document.line_count().saturating_sub(1));
            let col = tab.cursor_col.min(editor.document.line_len_chars(line));
            editor.click(line, col, false);
        }
    }
    if let Some(tab) = session.active_tab.and_then(|i| session.open_tabs.get(i)) {
        let key = if tab.is_diff { TabKey::Diff(tab.path.clone()) } else { TabKey::File(tab.path.clone()) };
        if state.open_tabs.iter().any(|t| t.key() == key) {
            state.active_tab = Some(key);
        }
    }
    state.sidebar_width = session.sidebar_width.clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH);
    state.sidebar_collapsed = session.sidebar_collapsed;
    state.collapsed_dirs = session.collapsed_dirs.into_iter().collect();
    state.changes_panel_open = session.changes_panel_open;
    state.problems_panel_open = session.problems_panel_open;
    if let Some(mode) = settings::chat_mode_from_key(&session.chat_mode) {
        state.chat_mode = mode;
    }
    state.chat_tab_open = session.chat_tab_open;
    if session.chat_tab_open && session.chat_tab_active {
        state.active_tab = Some(TabKey::Chat);
    }
    state.restoring_session = false;
}

fn set_theme_mode(state: &mut State, mode: ThemeMode) {
    state.theme_mode = mode;
    persist_settings(state);
}

fn set_accent(state: &mut State, accent: Accent) {
    state.accent = accent;
    // A preset always wins outright over a leftover custom color — same
    // "the override wins, not blended" relationship the other direction
    // (`Message::SetCustomAccent`) already has with `accent`.
    state.custom_accent = None;
    persist_settings(state);
}

/// Closes every status-bar popover (Background Tasks, EOL, Language,
/// Encoding) — all four anchor to roughly the same corner (see
/// `status_bar.rs`'s own doc comments on why exact per-segment positioning
/// isn't worth chasing here), so each one's own toggle closes the rest
/// first rather than risking two stacked on top of each other.
fn close_status_bar_popovers(state: &mut State) {
    state.background_tasks_open = false;
    state.eol_picker_open = false;
    state.language_picker_open = false;
    state.encoding_info_open = false;
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
///
/// A query starting with `:` is Go to Line, not a text filter — `:42` never
/// substring-matches a command label anyway, so this is purely additive: it
/// turns what used to be an always-empty result into the one synthetic entry
/// that actually jumps there.
pub fn filtered_palette_entries(state: &State) -> Vec<PaletteEntry> {
    let query = state.palette_query.trim();
    if let Some(rest) = query.strip_prefix(':') {
        return match rest.trim().parse::<usize>() {
            Ok(line) if line > 0 && active_file_path(state).is_some() => vec![PaletteEntry {
                label: format!("Go to line {line}"),
                action: PaletteAction::GoToLine(line),
            }],
            _ => Vec::new(),
        };
    }
    let query = query.to_ascii_lowercase();
    all_palette_entries(state)
        .into_iter()
        .filter(|entry| query.is_empty() || entry.label.to_ascii_lowercase().contains(&query))
        .take(MAX_PALETTE_RESULTS)
        .collect()
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
    // Literal Ctrl, not `modifiers.command()` — `command()` is Cmd on macOS,
    // where Cmd+Tab is the OS's own app switcher and never reaches this
    // handler at all. Ctrl+Tab is the one binding that means "cycle tabs" on
    // every platform, so it's checked directly rather than through the
    // per-OS `command()` mapping the rest of this function uses.
    if let keyboard::Event::KeyPressed { key, modifiers, .. } = &event
        && modifiers.control()
        && matches!(key, keyboard::Key::Named(keyboard::key::Named::Tab))
    {
        return Message::StepTabSwitcher(if modifiers.shift() { -1 } else { 1 });
    }
    // Ctrl released while the switcher is open commits the highlighted
    // entry, the same "hold to browse, release to pick" gesture Alt-Tab
    // uses — see `Message::ConfirmTabSwitcher`'s own doc for why this is
    // safe to emit unconditionally rather than only while the switcher is
    // actually open.
    if let keyboard::Event::ModifiersChanged(modifiers) = &event
        && !modifiers.control()
    {
        return Message::ConfirmTabSwitcher;
    }
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
                return if modifiers.shift() {
                    Message::ChatFocus
                } else if modifiers.alt() {
                    Message::ChatNewSession
                } else {
                    Message::ChatToggle
                };
            }
            if c.eq_ignore_ascii_case("u") {
                return if modifiers.shift() { Message::ChatToggleActions } else { Message::ChatAttachFileDialog };
            }
            if c.eq_ignore_ascii_case("g") {
                return Message::OpenGoToLine;
            }
            if c.eq_ignore_ascii_case("h") {
                return Message::ToggleReplace;
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
    // No inline-completion worker with no project open, or while the
    // settings toggle is off — see `copilot_completion_worker`.
    if !state.welcome_open && state.copilot_inline_enabled {
        let key = (state.root.clone(), state.copilot_completion_restart_token);
        subs.push(iced::Subscription::run_with(key, copilot_completion_worker).map(Message::CopilotCompletion));
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
            state.chat_provider,
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
    // Same shape again: only ticks while the active editor actually has a
    // rested-on position waiting on `HOVER_DWELL`/a reply, not permanently.
    let hover_pending = active_file_path(state)
        .and_then(|path| find_editor(state, &path))
        .is_some_and(|e| e.hover_pending_active());
    if hover_pending {
        subs.push(iced::time::every(Duration::from_millis(80)).map(|_| Message::HoverDebounceTick));
    }
    // Same shape again: only ticks while the mouse is actually resting on a
    // tab, waiting on `ui::tab_bar::TAB_PREVIEW_DWELL` — see
    // `Message::TabPreviewTick`'s own doc.
    if state.tab_hover.is_some() {
        subs.push(iced::time::every(Duration::from_millis(80)).map(|_| Message::TabPreviewTick));
    }
    // Same shape again: only ticks while the mouse is resting on a
    // breadcrumb segment, waiting on `ui::breadcrumb_bar::HOVER_DWELL`.
    if state.breadcrumb_hover.is_some() {
        subs.push(iced::time::every(Duration::from_millis(80)).map(|_| Message::BreadcrumbPreviewTick));
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
#[path = "../tests/state.rs"]
mod tests;
