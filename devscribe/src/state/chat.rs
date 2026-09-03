//! AI Chat Assist state: `ChatThread`/`ChatMessage`/`ToolActivity`, the `claude` worker
//! subscription, and the message-handling reducer for `ClaudeEvent`s. Split out of the
//! former monolithic `state.rs` — see `super` for `State`/`Message`/`update()`, which
//! this module's functions are called from but never define themselves.

use super::*;

/// Which backend `chat_worker` spawns for the AI Chat Assist panel. `Claude`
/// embeds the `claude` CLI (see `devscribe_core::claude_agent`); `Copilot`
/// embeds `copilot-language-server` (see `devscribe_core::copilot_agent`) —
/// both report `ClaudeEvent::Unavailable` themselves if their binary isn't on
/// PATH, same convention as the LSP servers `lsp_worker` spawns. See
/// docs/roadmap.md #11 for the rest of the plan (more providers, then inline
/// ghost-text completion).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChatProvider {
    Claude,
    Copilot,
}

impl ChatProvider {
    pub const ALL: [ChatProvider; 2] = [ChatProvider::Claude, ChatProvider::Copilot];

    pub fn label(self) -> &'static str {
        match self {
            ChatProvider::Claude => "Claude",
            ChatProvider::Copilot => "Copilot",
        }
    }

    /// Whether this provider is backed by the `claude` CLI, and so supports
    /// the Claude-specific affordances the panel offers on top of plain
    /// chat: saved session transcripts ("THREADS"), `@`-path file mentions,
    /// and the `/model`, `/effort`, `/usage` slash commands the Actions
    /// popup sends as prompts. `copilot_agent` implements none of these —
    /// see its own doc comment on scope — so surfacing them under Copilot
    /// would be UI that looks functional but silently does nothing.
    pub fn is_claude_cli(self) -> bool {
        matches!(self, ChatProvider::Claude)
    }
}

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

/// Deliberately holds no payload — the UI only ever needs to know *whether*
/// a tool call errored, never the result content itself (Read/Bash/Grep
/// output can run to megabytes, and nothing renders it). Dropped at the
/// source in `claude_agent::parse_event_line` too, not just here, so a
/// large result is never even cloned into this process's memory.
#[derive(Debug, Clone)]
pub struct ToolActivityResult {
    pub is_error: bool,
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
    /// `true` once `ClaudeEvent::HistoryTruncated` says a resumed session's
    /// saved transcript has more history than `messages` currently holds —
    /// drives the chat panel's "Load earlier messages" row. Cleared once
    /// `Message::ChatFullHistoryLoaded` replaces `messages` with the
    /// complete replay.
    pub history_truncated: bool,
}

impl ChatThread {
    fn find_tool_mut(&mut self, id: &str) -> Option<&mut ToolActivity> {
        self.messages.iter_mut().rev().find_map(|m| match m {
            ChatMessage::Tool(tool) if tool.id == id => Some(tool),
            _ => None,
        })
    }
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

/// Shared by the title-bar button, `⌘I`, and the command palette entry.
/// Turns the panel fully off (both the docked/collapsed presentation *and*
/// tab presentation — see `chat_is_active`) if it's on in any form, else
/// opens it as a tab — the default presentation for a freshly opened
/// session (Docked/Collapsed are reached afterward via the "View" popup).
pub fn toggle_chat(state: &mut State) {
    if chat_is_active(state) {
        state.chat_mode = ChatMode::Closed;
        state.chat_tab_open = false;
    } else {
        state.chat_tab_open = true;
        state.chat_mode = ChatMode::Closed;
        state.active_tab = Some(TabKey::Chat);
    }
    persist_settings(state);
    persist_session(state);
}

/// Clears `chat_tab_open` and, if the chat tab was the active one,
/// refocuses whatever's now the first open tab instead of one that no
/// longer exists — the same correction `toggle_chat`'s own doc comment
/// describes, shared by every "View" menu destination
/// (`ChatDock`/`ChatCollapse`/`ChatDockFromTab`/`ChatCloseTab`) now that
/// the menu offers all of them uniformly from every view, tab included.
pub fn leave_chat_tab(state: &mut State) {
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
pub fn submit_chat_prompt(state: &mut State) {
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
pub fn send_chat_text(state: &mut State, text: String) {
    state.chat.messages.push(ChatMessage::Operator(text.clone()));
    if let Some(sender) = state.chat.sender.as_mut() {
        let _ = sender.try_send(ClaudeCommand::SendPrompt(text));
    }
}

pub async fn pick_chat_mention_file(dir: PathBuf) -> Option<PathBuf> {
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
pub fn insert_chat_mention(state: &mut State, path: &Path, relative_to_project: bool) {
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
pub fn respond_permission(state: &mut State, id: String, approve: bool, reason: Option<String>) {
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
pub fn handle_chat_event(state: &mut State, event: ClaudeEvent) -> iced::Task<Message> {
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
        ClaudeEvent::ToolResult { id, is_error } => {
            if let Some(tool) = state.chat.find_tool_mut(&id) {
                tool.result = Some(ToolActivityResult { is_error });
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
        ClaudeEvent::HistoryTruncated => state.chat.history_truncated = true,
    }
    iced::Task::none()
}

/// Scans `~/.claude/projects/...` for this project's past sessions on its
/// own OS thread (`claude_agent::list_sessions` does real filesystem
/// I/O — a directory listing plus reading a chunk of each transcript for
/// its title), same `iced_runtime::task::blocking` vehicle as
/// `start_server_install`, so opening the session picker can never stall
/// the UI even on a project with a long chat history.
pub fn start_loading_chat_sessions(state: &State) -> iced::Task<Message> {
    let root = state.root.clone();
    iced_runtime::task::blocking(move |mut sender| {
        let sessions = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| claude_agent::list_sessions(&root)))
            .unwrap_or_default();
        let _ = sender.try_send(sessions);
    })
    .map(Message::ChatSessionsLoaded)
}

/// Background-thread re-read of the active session's full saved transcript
/// — `Message::LoadEarlierChatHistory`'s task, same
/// `iced_runtime::task::blocking` vehicle as `start_loading_chat_sessions`
/// so a long history can never stall the UI. A no-op `Task::none()` if no
/// session id is set yet (shouldn't happen: the row that sends this
/// message only shows once a resumed session's history has already loaded).
pub fn load_earlier_chat_history(state: &State) -> iced::Task<Message> {
    let root = state.root.clone();
    let session_id = state.chat_session_id.clone();
    iced_runtime::task::blocking(move |mut sender| {
        let events = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            claude_agent::load_full_session_history(&root, &session_id)
        }))
        .unwrap_or_default();
        let _ = sender.try_send(events);
    })
    .map(Message::ChatFullHistoryLoaded)
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
pub fn launch_terminal_running_claude() -> bool {
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

/// Keyed on `(root, session_id, provider, permission_mode, allow_bash,
/// chat_restart_token)`: a project switch, picking a different session
/// (`Message::ChatNewSession`/`ChatResumeSession`), switching providers
/// (`Message::ChatSetProvider`), switching modes
/// (`Message::ChatSetPermissionMode`), or flipping shell access
/// (`Message::ChatToggleShellAccess`) all change the key, so
/// `subscription()` tears down and respawns automatically, same as
/// `lsp_worker`. Binary resolution and `devscribe_exe` (needed so the
/// generated permission hook re-invokes *this* binary) happen inside the
/// async body, same as `lsp_worker` does for its own binary, so the main
/// thread never blocks either.
pub fn chat_worker(
    (root, session_id, provider, mode, allow_bash, _token): &(PathBuf, String, ChatProvider, PermissionMode, bool, u64),
) -> impl iced::futures::Stream<Item = ClaudeEvent> + use<> {
    let root = root.clone();
    let session_id = session_id.clone();
    let provider = *provider;
    let mode = *mode;
    let allow_bash = *allow_bash;
    iced::stream::channel(32, async move |mut output| {
        use iced::futures::SinkExt as _;
        // First, always — see `ClaudeEvent::SessionStarting`'s own doc.
        let _ = output.send(ClaudeEvent::SessionStarting).await;

        match provider {
            ChatProvider::Claude => {
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

                // Whether this id already has a transcript is the sole signal
                // for new-vs-resume (see `claude_agent::session_exists`) — so
                // e.g. reopening the panel after closing it (same id, no
                // explicit "new"/"resume" click in between) naturally becomes
                // a resume, with nothing else needing to track that it
                // should.
                let resume = claude_agent::session_exists(&root, &session_id);
                if resume {
                    let history = claude_agent::load_session_history(&root, &session_id);
                    if history.truncated && output.send(ClaudeEvent::HistoryTruncated).await.is_err() {
                        return;
                    }
                    for event in history.events {
                        if output.send(event).await.is_err() {
                            return;
                        }
                    }
                }
                let options = claude_agent::SessionOptions { session_id, resume, mode, allow_bash };
                claude_agent::run(root, PathBuf::from("claude"), devscribe_exe, options, output).await;
            }
            ChatProvider::Copilot => {
                // `mode`/`allow_bash`/`session_id` don't apply — see
                // `copilot_agent::run`'s own doc comment on scope (no
                // permission modes, no resume-across-restarts yet).
                if !crate::server_install::which_binary("copilot-language-server") {
                    let _ = output
                        .send(ClaudeEvent::Unavailable(
                            "copilot-language-server not found on PATH — install: npm install -g @github/copilot-language-server".to_string(),
                        ))
                        .await;
                    return;
                }
                copilot_agent::run(root, PathBuf::from("copilot-language-server"), output).await;
            }
        }
    })
}

/// Same window-wide cursor tracking as `sidebar_resize_events`, for the
/// chat panel's own drag handle. Only subscribed while `state.chat_resizing`.
pub fn chat_resize_events(event: iced::Event, _status: iced::event::Status, _window: iced::window::Id) -> Option<Message> {
    match event {
        iced::Event::Mouse(mouse::Event::CursorMoved { position }) => Some(Message::ChatResizeDragged(position.x)),
        iced::Event::Mouse(mouse::Event::ButtonReleased(_)) => Some(Message::ChatResizeEnded),
        _ => None,
    }
}
