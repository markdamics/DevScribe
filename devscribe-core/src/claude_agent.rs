//! Embeds the real `claude` CLI (Claude Code) as a long-lived agentic
//! subprocess: one process per chat session, fed prompts over stdin and
//! streaming structured events back over stdout, exactly like `lsp.rs`
//! embeds a language server. Unlike `lsp.rs`, the wire protocol here isn't
//! externally specified (LSP is; Claude Code's `stream-json` isn't), so
//! every shape this module relies on was captured by hand against the real
//! binary (v2.1.246) rather than assumed from documentation — see the
//! project's implementation notes for the raw captures. Treat this module
//! as the single point that would need updating if a future CLI version
//! changes that wire format.
//! 
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use async_process::{Command, Stdio as AsyncStdio};
use futures::channel::mpsc;
use futures::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use futures::{FutureExt, SinkExt, StreamExt};
use serde_json::{json, Value};

/// A command the app sends into a running session (see `run`).
#[derive(Debug, Clone)]
pub enum ClaudeCommand {
    /// A new chat turn from the person using the panel.
    SendPrompt(String),
    /// Answers a pending `ClaudeEvent::PermissionRequest` with the same `id`.
    RespondPermission {
        id: String,
        approve: bool,
        reason: Option<String>,
    },
}

/// An event a running session reports back to the app. Tool-related
/// payloads (`tool_input`/the permission request's `tool_input`/a tool's
/// result) are passed through as raw `serde_json::Value` rather than typed
/// per-tool structs: this module's job is faithfully translating the wire
/// protocol, not deciding how each of Claude Code's many tools should be
/// rendered — that's for whatever consumes these events.
#[derive(Debug, Clone)]
pub enum ClaudeEvent {
    /// A worker is about to spawn (or resume) a session — sent first,
    /// before anything else, specifically so a consumer can reset any
    /// leftover transcript from a *previous* worker instance for the same
    /// session id (e.g. the panel closing and reopening) before this
    /// worker's own history replay (for a resume) or live events (for a
    /// new session) start arriving — otherwise a resume would duplicate
    /// whatever was already on screen from before.
    SessionStarting,
    /// The subprocess is up; `sender` accepts `ClaudeCommand`s.
    Ready(mpsc::Sender<ClaudeCommand>),
    /// Learned once we've seen both the session id (`system`/`init`) and
    /// the model actually in use (only known once the first `assistant`
    /// message arrives — the CLI doesn't echo the requested `--model`
    /// verbatim in `init`).
    SessionInit { session_id: String, model: String },
    /// A completed assistant text block (this module doesn't request
    /// `--include-partial-messages`, so these arrive whole, not streamed
    /// token-by-token).
    AssistantText(String),
    /// The operator's own message, replayed from a resumed session's saved
    /// transcript (see `load_session_history`) — never produced by the live
    /// stdout stream, which only ever echoes the operator's *prior* turns
    /// back as plain-string `user` entries when reading a transcript file,
    /// not during a live session (the live UI already knows what it just
    /// sent, via `ClaudeCommand::SendPrompt`'s own caller).
    OperatorText(String),
    ToolUseStarted { id: String, name: String, input: Value },
    /// `result` is whatever the wire protocol gave back for this tool call
    /// — the top-level `tool_use_result` field when present (richer,
    /// tool-specific: file contents for `Read`, stdout/stderr for `Bash`,
    /// ...), else the tool_result content block itself.
    ToolResult { id: String, is_error: bool, result: Value },
    /// `claude` wants to run `tool_name` with `tool_input` and is waiting
    /// (via the hook bridge) for `ClaudeCommand::RespondPermission{id, ..}`.
    PermissionRequest {
        id: String,
        tool_name: String,
        tool_input: Value,
    },
    /// One turn finished — real cost/usage, replacing any placeholder UI
    /// might show before the first turn completes.
    TurnResult {
        cost_usd: f64,
        input_tokens: u64,
        output_tokens: u64,
    },
    /// The subprocess couldn't be spawned, or the connection died.
    Unavailable(String),
}

/// Removes the generated socket and settings files on drop — critically,
/// including when `run`'s future is *cancelled* mid-`.await` rather than
/// returning normally. That's actually the common case: closing the panel,
/// switching projects, or quitting the app while a session is live all
/// tear this down by dropping the subscription's future outright (iced
/// cancels the old stream when `Subscription::run_with`'s key changes or
/// disappears) — a dropped future never reaches code placed after its
/// suspended `.await` point, so cleanup written as plain statements at the
/// end of `run` would only ever fire on the rare paths that actually
/// `break` out of the loop and fall through. An RAII guard's `Drop` runs
/// either way.
struct TempFiles {
    socket_path: PathBuf,
    settings_path: PathBuf,
}

impl Drop for TempFiles {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket_path);
        let _ = std::fs::remove_file(&self.settings_path);
    }
}

struct PendingPermission {
    decide: std_mpsc::Sender<Decision>,
}

struct Decision {
    approve: bool,
    reason: Option<String>,
}

type PendingMap = Arc<Mutex<HashMap<String, PendingPermission>>>;

/// One connection = one hook invocation = one tool call: `claude` spawns a
/// fresh `--claude-permission-hook` process per gated tool call, and that
/// process makes exactly one connection here, sends exactly one JSON
/// request line, and waits for exactly one JSON response line — so unlike
/// most socket servers, there's no request/response multiplexing to do on
/// the wire itself.
fn handle_permission_connection(stream: UnixStream, pending: PendingMap, requests: mpsc::UnboundedSender<ClaudeEvent>) {
    let mut reader = std::io::BufReader::new(match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    });
    let mut line = String::new();
    if reader.read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    let Ok(request) = serde_json::from_str::<Value>(&line) else {
        return;
    };
    let id = request.get("tool_use_id").and_then(Value::as_str).unwrap_or_default().to_string();
    let tool_name = request.get("tool_name").and_then(Value::as_str).unwrap_or_default().to_string();
    let tool_input = request.get("tool_input").cloned().unwrap_or(Value::Null);
    if id.is_empty() {
        return;
    }

    let (decide, wait) = std_mpsc::channel();
    pending.lock().unwrap().insert(id.clone(), PendingPermission { decide });
    let _ = requests.unbounded_send(ClaudeEvent::PermissionRequest { id: id.clone(), tool_name, tool_input });

    // Blocks this connection's dedicated thread — not the app, not the
    // `claude` subprocess's other work — until `run`'s command loop routes
    // a matching `RespondPermission` here. Fails *closed* (deny) if the
    // sender was ever dropped without deciding (e.g. the chat session
    // ended while this was pending) rather than risk a silent auto-allow.
    let decision = wait.recv().unwrap_or(Decision {
        approve: false,
        reason: Some("devscribe: session ended before this was answered".into()),
    });
    pending.lock().unwrap().remove(&id);

    let response = if decision.approve {
        json!({"decision": "approve"})
    } else {
        json!({"decision": "block", "reason": decision.reason.unwrap_or_else(|| "denied".into())})
    };
    let mut stream = stream;
    let _ = writeln!(stream, "{response}");
}

/// Accepts hook connections on its own OS thread for as long as `socket_path`
/// exists — torn down implicitly when `run` returns and the bound listener
/// (owned by that thread) is dropped along with the process exiting, or
/// explicitly by removing the socket file, whichever comes first.
fn spawn_permission_listener(socket_path: PathBuf, pending: PendingMap, requests: mpsc::UnboundedSender<ClaudeEvent>) -> std::io::Result<()> {
    let listener = UnixListener::bind(&socket_path)?;
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let pending = pending.clone();
            let requests = requests.clone();
            std::thread::spawn(move || handle_permission_connection(stream, pending, requests));
        }
    });
    Ok(())
}

fn generate_settings(devscribe_exe: &std::path::Path, socket_path: &std::path::Path) -> Value {
    let command = format!(
        "\"{}\" --claude-permission-hook \"{}\"",
        devscribe_exe.display(),
        socket_path.display()
    );
    json!({
        "hooks": {
            "PreToolUse": [{
                "matcher": "Edit|Write",
                "hooks": [{ "type": "command", "command": command }]
            }]
        }
    })
}

/// Extracts the richest available description of a tool's result: the
/// top-level `tool_use_result` (tool-specific — e.g. `Read` gives back
/// `{file: {content, ...}}`, `Bash` gives `{stdout, stderr, ...}`) when
/// present, else the plain `tool_result` content block.
fn tool_result_value(message: &Value, block: &Value) -> Value {
    message.get("tool_use_result").cloned().unwrap_or_else(|| block.get("content").cloned().unwrap_or(Value::Null))
}

async fn emit_session_init_if_ready(
    output: &mut mpsc::Sender<ClaudeEvent>,
    session_id: &str,
    model: &Option<String>,
    announced: &mut bool,
) {
    if *announced {
        return;
    }
    if let Some(model) = model {
        *announced = true;
        let _ = output.send(ClaudeEvent::SessionInit { session_id: session_id.to_string(), model: model.clone() }).await;
    }
}

fn handle_stdout_line(line: &str, model: &mut Option<String>) -> Vec<ClaudeEvent> {
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return Vec::new();
    };
    parse_event_line(&v, model)
}

/// Shared by the live stdout parser (`handle_stdout_line`, one JSON object
/// per line as it streams) and `load_session_history` (a saved transcript
/// file, same per-line JSON shape for the "assistant"/"user" types that
/// matter here — see that function's own doc comment for the one place
/// the two formats genuinely differ, which this handles too: a `user`
/// entry's `content` is always a tool-result array in the live stream, but
/// can *also* be a plain string in a transcript file — the operator's own
/// past message, replayed as `OperatorText`.
fn parse_event_line(v: &Value, model: &mut Option<String>) -> Vec<ClaudeEvent> {
    let mut events = Vec::new();
    match v.get("type").and_then(Value::as_str) {
        Some("assistant") => {
            let message = v.get("message");
            if model.is_none() {
                if let Some(m) = message.and_then(|m| m.get("model")).and_then(Value::as_str) {
                    *model = Some(m.to_string());
                }
            }
            if let Some(content) = message.and_then(|m| m.get("content")).and_then(Value::as_array) {
                for block in content {
                    match block.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            if let Some(text) = block.get("text").and_then(Value::as_str) {
                                if !text.is_empty() {
                                    events.push(ClaudeEvent::AssistantText(text.to_string()));
                                }
                            }
                        }
                        Some("tool_use") => {
                            let id = block.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
                            let name = block.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
                            let input = block.get("input").cloned().unwrap_or(Value::Null);
                            events.push(ClaudeEvent::ToolUseStarted { id, name, input });
                        }
                        _ => {}
                    }
                }
            }
        }
        Some("user") => {
            if let Some(message) = v.get("message") {
                match message.get("content") {
                    Some(Value::Array(content)) => {
                        for block in content {
                            if block.get("type").and_then(Value::as_str) == Some("tool_result") {
                                let id = block.get("tool_use_id").and_then(Value::as_str).unwrap_or_default().to_string();
                                let is_error = block.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                                let result = tool_result_value(v, block);
                                events.push(ClaudeEvent::ToolResult { id, is_error, result });
                            }
                        }
                    }
                    Some(Value::String(text)) => {
                        let trimmed = text.trim();
                        if !trimmed.is_empty() {
                            events.push(ClaudeEvent::OperatorText(trimmed.to_string()));
                        }
                    }
                    _ => {}
                }
            }
        }
        Some("result") => {
            let cost_usd = v.get("total_cost_usd").and_then(Value::as_f64).unwrap_or(0.0);
            let usage = v.get("usage");
            let input_tokens = usage.and_then(|u| u.get("input_tokens")).and_then(Value::as_u64).unwrap_or(0);
            let output_tokens = usage.and_then(|u| u.get("output_tokens")).and_then(Value::as_u64).unwrap_or(0);
            events.push(ClaudeEvent::TurnResult { cost_usd, input_tokens, output_tokens });
        }
        // `system/status`, `system/thinking_tokens`, `stream_event`,
        // `rate_limit_event`, and anything future/unrecognized: not needed
        // yet, and deliberately not an error — new event types are exactly
        // the kind of wire-format drift this module's doc comment warns
        // about, and a strict match here would turn "unfamiliar" into
        // "crash" instead of "ignore".
        _ => {}
    }
    events
}

/// A fresh, locally-generated session id for a brand-new session — passed
/// to `run` so *DevScribe* controls the id up front (via `--session-id`)
/// rather than learning one after the fact from `system/init`, which is
/// what lets a session be found again later by `list_sessions`/resumed by
/// `load_session_history` even before its first turn completes.
pub fn new_session_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionSummary {
    pub id: String,
    /// Claude Code's own short AI-generated summary of the conversation
    /// (`"type":"ai-title"` in the transcript) when one exists yet, else
    /// the first operator message, else a generic placeholder for a
    /// session that was created but never actually sent a first turn.
    pub title: String,
    pub last_active: SystemTime,
}

/// The directory Claude Code stores this project's session transcripts
/// under — empirically confirmed (not documented) against the installed
/// CLI (v2.1.246): the project's absolute path with every `/` replaced by
/// `-`, under `~/.claude/projects/`. If a future CLI version changes this,
/// `list_sessions`/`load_session_history` degrade to "no sessions found"
/// rather than erroring — see their own doc comments.
fn project_session_dir(root: &Path) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let encoded = root.to_string_lossy().replace('/', "-");
    Some(home.join(".claude").join("projects").join(encoded))
}

fn truncate_for_title(text: &str, max_chars: usize) -> String {
    let truncated: String = text.chars().take(max_chars).collect();
    if text.chars().count() > max_chars {
        format!("{truncated}\u{2026}")
    } else {
        truncated
    }
}

/// Scans a transcript file for the best available title: the *last*
/// `ai-title` entry (Claude Code appends a fresh one as the conversation
/// evolves, so the latest is the most accurate) if the conversation has
/// gone on long enough for one to exist yet, else the first operator
/// message, truncated. `None` only if the file can't be read at all.
fn session_title(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut ai_title: Option<String> = None;
    let mut first_user_message: Option<String> = None;
    for line in std::io::BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        match v.get("type").and_then(Value::as_str) {
            Some("ai-title") => {
                if let Some(title) = v.get("aiTitle").and_then(Value::as_str) {
                    ai_title = Some(title.to_string());
                }
            }
            Some("user") if first_user_message.is_none() => {
                if let Some(text) = v.get("message").and_then(|m| m.get("content")).and_then(Value::as_str) {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        first_user_message = Some(truncate_for_title(trimmed, 80));
                    }
                }
            }
            _ => {}
        }
    }
    Some(ai_title.or(first_user_message).unwrap_or_else(|| "New session".to_string()))
}

/// Lists past sessions for `root`, most recently active first (by file
/// modification time). Blocking (filesystem reads) — callers must run
/// this off the UI thread, same convention as `devscribe`'s
/// `start_search`/`start_server_install`. Returns an empty list rather
/// than an error if the project has never had a session, or its session
/// directory can't be resolved at all — both are "nothing to show," not a
/// failure worth surfacing.
pub fn list_sessions(root: &Path) -> Vec<SessionSummary> {
    let Some(dir) = project_session_dir(root) else { return Vec::new() };
    let Ok(entries) = std::fs::read_dir(&dir) else { return Vec::new() };

    let mut sessions: Vec<SessionSummary> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().and_then(|e| e.to_str()) == Some("jsonl"))
        .filter_map(|entry| {
            let path = entry.path();
            let id = path.file_stem()?.to_str()?.to_string();
            let last_active = entry.metadata().ok()?.modified().ok()?;
            let title = session_title(&path)?;
            Some(SessionSummary { id, title, last_active })
        })
        .collect();
    sessions.sort_by_key(|s| std::cmp::Reverse(s.last_active));
    sessions
}

/// Reconstructs a resumed session's prior transcript as the same
/// `ClaudeEvent`s live streaming would have produced (run each back
/// through the exact same reducer the live path uses). This exists
/// because `--resume` only streams *new* turns going forward — confirmed
/// against the real CLI, it does not replay history on its own — so
/// without this, resuming a session would silently show an empty
/// transcript despite `claude` itself remembering everything. Blocking,
/// same convention as `list_sessions`; an unreadable/missing transcript
/// degrades to an empty history rather than an error (the session may
/// simply be brand new).
pub fn load_session_history(root: &Path, session_id: &str) -> Vec<ClaudeEvent> {
    let Some(dir) = project_session_dir(root) else { return Vec::new() };
    let Ok(content) = std::fs::read_to_string(dir.join(format!("{session_id}.jsonl"))) else {
        return Vec::new();
    };

    let mut model = None;
    let mut events = Vec::new();
    for line in content.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        events.extend(parse_event_line(&v, &mut model));
    }
    events
}

/// Whether `session_id` already has a saved transcript for `root` — the
/// single source of truth callers (see `devscribe`'s `chat_worker`) use to
/// decide whether to spawn `run` with `resume: false` (a genuinely new id)
/// or `resume: true` (continuing one that already has history), rather
/// than tracking that distinction as separate state that could drift out
/// of sync with what's actually on disk.
pub fn session_exists(root: &Path, session_id: &str) -> bool {
    project_session_dir(root).map(|dir| dir.join(format!("{session_id}.jsonl")).exists()).unwrap_or(false)
}

/// Spawns `claude` for `root` and drives it — reading its stream-json
/// stdout, writing prompts as stream-json stdin, and answering its
/// permission-hook connections — until the process dies or `output` is
/// dropped. `binary` and `devscribe_exe` are resolved paths; callers (see
/// `devscribe`'s `chat_worker`) are responsible for locating `claude` on
/// PATH first and emitting `Unavailable` if it isn't found, same
/// convention as `lsp::run`. `session_id` is always DevScribe's own choice
/// (see `new_session_id`) — passed as `--session-id` for a brand-new
/// session (`resume: false`) or `--resume` to continue an existing one
/// (`resume: true`), never left for the CLI to pick on its own.
pub async fn run(
    root: PathBuf,
    binary: PathBuf,
    devscribe_exe: PathBuf,
    session_id: String,
    resume: bool,
    mut output: mpsc::Sender<ClaudeEvent>,
) {
    let run_id = format!("{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default());
    let socket_path = std::env::temp_dir().join(format!("devscribe-claude-{run_id}.sock"));
    let settings_path = std::env::temp_dir().join(format!("devscribe-claude-{run_id}-settings.json"));
    let _ = std::fs::remove_file(&socket_path);
    // Held for the rest of this function, including across every `.await`
    // — see `TempFiles`'s own doc comment for why that matters.
    let _cleanup = TempFiles { socket_path: socket_path.clone(), settings_path: settings_path.clone() };

    let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
    let (perm_tx, mut perm_rx) = mpsc::unbounded::<ClaudeEvent>();
    if let Err(err) = spawn_permission_listener(socket_path.clone(), pending.clone(), perm_tx) {
        let _ = output.send(ClaudeEvent::Unavailable(format!("couldn't start the permission-hook socket: {err}"))).await;
        return;
    }

    let settings = generate_settings(&devscribe_exe, &socket_path);
    let Ok(settings_json) = serde_json::to_string(&settings) else {
        let _ = output.send(ClaudeEvent::Unavailable("couldn't serialize the generated hook settings".into())).await;
        return;
    };
    if std::fs::write(&settings_path, settings_json).is_err() {
        let _ = output.send(ClaudeEvent::Unavailable("couldn't write the generated hook settings file".into())).await;
        return;
    }

    let mut command = Command::new(&binary);
    command
        .arg("--print")
        .arg("--input-format")
        .arg("stream-json")
        .arg("--output-format")
        .arg("stream-json")
        .arg("--verbose")
        .arg("--permission-mode")
        .arg("acceptEdits")
        .arg("--disallowedTools")
        .arg("Bash")
        .arg("--strict-mcp-config")
        .arg("--settings")
        .arg(&settings_path);
    if resume {
        command.arg("--resume").arg(&session_id);
    } else {
        command.arg("--session-id").arg(&session_id);
    }
    let mut child = match command
        .current_dir(&root)
        .stdin(AsyncStdio::piped())
        .stdout(AsyncStdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            let _ = output.send(ClaudeEvent::Unavailable(format!("claude not available: {err}"))).await;
            return; // `_cleanup` (in scope since before `settings_path` was written) handles both files
        }
    };
    let child_stdout = child.stdout.take().expect("piped stdout");
    let mut child_stdin = child.stdin.take().expect("piped stdin");
    let mut lines = BufReader::new(child_stdout).lines();

    let (command_tx, mut command_rx) = mpsc::channel::<ClaudeCommand>(32);
    let _ = output.send(ClaudeEvent::Ready(command_tx)).await;

    let mut model: Option<String> = None;
    let mut session_init_announced = false;

    loop {
        futures::select! {
            line = lines.next().fuse() => {
                match line {
                    Some(Ok(line)) => {
                        for event in handle_stdout_line(&line, &mut model) {
                            if output.send(event).await.is_err() {
                                let _ = child.kill();
                                break;
                            }
                        }
                        emit_session_init_if_ready(&mut output, &session_id, &model, &mut session_init_announced).await;
                    }
                    Some(Err(_)) | None => {
                        let _ = output.send(ClaudeEvent::Unavailable("claude's output stream ended".into())).await;
                        break;
                    }
                }
            }
            perm_event = perm_rx.next().fuse() => {
                match perm_event {
                    Some(event) => { if output.send(event).await.is_err() { break; } }
                    None => {} // listener thread panicked/exited — permission requests just won't surface anymore
                }
            }
            command = command_rx.next().fuse() => {
                match command {
                    Some(ClaudeCommand::SendPrompt(text)) => {
                        let line = json!({"type": "user", "message": {"role": "user", "content": text}}).to_string();
                        if child_stdin.write_all(line.as_bytes()).await.is_err()
                            || child_stdin.write_all(b"\n").await.is_err()
                            || child_stdin.flush().await.is_err()
                        {
                            let _ = output.send(ClaudeEvent::Unavailable("couldn't send the prompt to claude".into())).await;
                            break;
                        }
                    }
                    Some(ClaudeCommand::RespondPermission { id, approve, reason }) => {
                        if let Some(slot) = pending.lock().unwrap().remove(&id) {
                            let _ = slot.decide.send(Decision { approve, reason });
                        }
                    }
                    None => break, // app dropped its command sender — session over
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/claude_agent.rs"]
mod tests;
