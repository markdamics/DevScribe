//! Embeds `copilot-language-server` a *second* time — independent of
//! `copilot_agent`'s chat connection — as the source of VS Code-style
//! inline ("ghost text") completions via `textDocument/inlineCompletion`.
//! Kept as its own always-on-if-enabled worker rather than folded into
//! `copilot_agent`'s chat session: inline completions are useful
//! independent of which chat provider (or whether the chat panel is even
//! open) is active, the same way VS Code's own Copilot completions work
//! regardless of which Copilot Chat participant happens to be selected.
//! Real LSP framing, same `async-lsp`-based pattern as `lsp.rs`.
//!
//! Unlike `copilot_agent`'s `conversation/*` methods, `textDocument/inlineCompletion`
//! itself *is* documented — the package's own README, and even standardized
//! (`lsp_types::request::InlineCompletionRequest` already exists, drafted
//! for LSP 3.18). This module still builds request params as raw JSON
//! rather than using those typed structs, though: the real wire format adds
//! two non-standard fields on top of the spec (`textDocument.version` and
//! `formattingOptions`) that the typed `InlineCompletionParams` has no field
//! for, and matching `copilot_agent`'s own house style (raw `Value` for
//! anything copilot-specific) keeps both modules readable the same way.
//!
//! Scope, deliberately: this worker does not itself run the `signIn` device
//! flow the way `copilot_agent` does for chat — a passive, continuous-as-
//! you-type feature isn't a reasonable place to first ask a human to open a
//! browser and enter a code. If the account isn't already signed in
//! (typically via the chat panel's own Copilot session), this reports
//! `Unavailable` with that instruction rather than blocking on one itself.
use std::collections::HashMap;
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::process::Stdio;

use async_lsp::lsp_types::notification::Notification;
use async_lsp::lsp_types::request::Request;
use async_lsp::lsp_types::{
    ClientCapabilities, DidChangeConfigurationParams, DidChangeTextDocumentParams,
    DidCloseTextDocumentParams, DidOpenTextDocumentParams, ExecuteCommandParams, InitializeParams,
    InitializedParams, TextDocumentContentChangeEvent, TextDocumentIdentifier, TextDocumentItem,
    Url, VersionedTextDocumentIdentifier, WorkDoneProgressParams, WorkspaceClientCapabilities,
    WorkspaceFolder,
};
use async_lsp::router::Router;
use async_lsp::{LanguageServer, MainLoop};
use futures::channel::mpsc;
use futures::stream::FuturesUnordered;
use futures::{FutureExt, SinkExt, StreamExt};
use serde_json::{json, Value};

/// A command the app sends into a running worker (see `run`).
#[derive(Debug, Clone)]
pub enum CopilotCompletionCommand {
    DidOpen { uri: Url, text: String },
    DidChange { uri: Url, text: String },
    DidClose { uri: Url },
    /// The custom `textDocument/didFocus` notification — sent when the
    /// active tab changes, per the package README.
    DidFocus { uri: Url },
    /// Request a suggestion at `line`/`character`. Never blocks behind a
    /// prior still-pending `Suggest` — see `run`'s own doc on why firing
    /// requests without waiting is what actually lets the server's own
    /// "cancel-previous" strategy (confirmed in the README) do its job;
    /// only the most recent request's result is ever emitted regardless.
    Suggest { uri: Url, line: u32, character: u32 },
    /// The user accepted a shown suggestion — replays its own `command` via
    /// `workspace/executeCommand`, which the README says is required for
    /// acceptance telemetry. `item` is the exact `Value` the matching
    /// `CopilotCompletionEvent::Suggestion` carried.
    Accepted { item: Value },
}

/// An event a running worker reports back to the app.
#[derive(Debug, Clone)]
pub enum CopilotCompletionEvent {
    /// The subprocess is up; `sender` accepts `CopilotCompletionCommand`s.
    Ready(mpsc::Sender<CopilotCompletionCommand>),
    /// The result of a `Suggest` command — echoes `uri`/`line`/`character`
    /// back so a caller that fired several in a row (the editor moved the
    /// cursor again before this one came back) can tell which request this
    /// answers, same convention `lsp::LspEvent::Completions` already uses.
    /// `item` is `None` when the server had nothing to offer at that
    /// position — a normal, common response, not an error — and is
    /// otherwise the raw `InlineCompletionItem` object (`insertText`,
    /// `range`, `command`, ...), passed through unparsed since nothing here
    /// needs more than to display and, on accept, echo it back.
    Suggestion { uri: Url, line: u32, character: u32, item: Option<Value> },
    /// The subprocess couldn't be spawned, the connection died, or the
    /// account isn't signed in yet (see this module's own doc comment).
    Unavailable(String),
}

enum InlineCompletion {}
impl Request for InlineCompletion {
    type Params = Value;
    type Result = Value;
    const METHOD: &'static str = "textDocument/inlineCompletion";
}

enum SignIn {}
impl Request for SignIn {
    type Params = Value;
    type Result = Value;
    const METHOD: &'static str = "signIn";
}

enum DidFocus {}
impl Notification for DidFocus {
    type Params = Value;
    const METHOD: &'static str = "textDocument/didFocus";
}

enum DidShowCompletion {}
impl Notification for DidShowCompletion {
    type Params = Value;
    const METHOD: &'static str = "textDocument/didShowCompletion";
}

/// Pulls `items[0]` out of an `InlineCompletionList`-shaped result — pure
/// and unit-testable, mirroring `copilot_agent`'s own small parsing helpers.
/// Only the first item is ever used: this module shows one ghost-text
/// suggestion at a time (no VS Code-style cycling through alternates yet).
fn first_item(result: &Value) -> Option<Value> {
    result.get("items")?.as_array()?.first().cloned()
}

/// Spawns `copilot-language-server` for `root` and drives it until the
/// process dies or `output` is dropped — the inline-completion counterpart
/// to `copilot_agent::run`. `binary` is the resolved path (or bare name) of
/// `copilot-language-server`; callers (see `devscribe`'s
/// `copilot_completion_worker`) are responsible for locating it on PATH
/// first and emitting `Unavailable` if it isn't found, same convention as
/// every other worker in this codebase.
pub async fn run(root: PathBuf, binary: PathBuf, mut output: mpsc::Sender<CopilotCompletionEvent>) {
    let log_path = dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("devscribe")
        .join("logs")
        .join("copilot-completion.stderr.log");
    let _ = std::fs::create_dir_all(log_path.parent().unwrap());
    let stderr_cfg = std::fs::File::create(&log_path).map(Stdio::from).unwrap_or_else(|_| Stdio::null());

    let mut cmd = async_process::Command::new(&binary);
    cmd.arg("--stdio")
        .current_dir(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(stderr_cfg)
        .kill_on_drop(true);

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => {
            let _ = output.send(CopilotCompletionEvent::Unavailable(format!("copilot-language-server not available: {err}"))).await;
            return;
        }
    };
    let child_stdout = child.stdout.take().expect("piped stdout");
    let child_stdin = child.stdin.take().expect("piped stdin");

    let (mainloop, mut server) = MainLoop::new_client(|_server| {
        let mut router: Router<()> = Router::new(());
        // No `$/progress`/chat-shaped notifications expected on this
        // connection at all — window/logMessage, telemetry/event, etc. are
        // all safe to ignore, same defensive catch-all `lsp.rs` registers
        // for jdtls's own non-standard chatter.
        router.unhandled_notification(|_, _| ControlFlow::Continue(()));
        router
    });
    let mainloop_fut = mainloop.run_buffered(child_stdout, child_stdin).fuse();
    futures::pin_mut!(mainloop_fut);

    macro_rules! drive {
        ($fut:expr) => {{
            let f = $fut.fuse();
            futures::pin_mut!(f);
            loop {
                futures::select! {
                    r = f => break Some(r),
                    r = mainloop_fut => {
                        if let Err(err) = r {
                            let line = format!("\n[copilot-completion-mainloop] error: {err:?}\n");
                            let _ = std::fs::OpenOptions::new().append(true).open(&log_path)
                                .and_then(|mut f| { use std::io::Write; f.write_all(line.as_bytes()) });
                        }
                        break None;
                    }
                }
            }
        }};
    }

    macro_rules! unavailable {
        ($msg:expr) => {{
            let _ = output.send(CopilotCompletionEvent::Unavailable($msg)).await;
        }};
    }

    let Some(root_uri) = Url::from_file_path(&root).ok() else {
        unavailable!("project root has no file:// URI".to_string());
        return;
    };

    let init = drive!(server.initialize(InitializeParams {
        workspace_folders: Some(vec![WorkspaceFolder { uri: root_uri, name: "root".into() }]),
        initialization_options: Some(json!({
            "editorInfo": {"name": "DevScribe", "version": env!("CARGO_PKG_VERSION")},
            "editorPluginInfo": {"name": "DevScribe", "version": env!("CARGO_PKG_VERSION")},
        })),
        capabilities: ClientCapabilities {
            workspace: Some(WorkspaceClientCapabilities { workspace_folders: Some(true), ..Default::default() }),
            ..Default::default()
        },
        ..InitializeParams::default()
    }));
    match init {
        Some(Ok(_)) => {}
        Some(Err(err)) => {
            unavailable!(format!("initialize failed: {err} — see {}", log_path.display()));
            return;
        }
        None => {
            unavailable!(format!("copilot-language-server stopped during startup — see {}", log_path.display()));
            return;
        }
    }
    if server.initialized(InitializedParams {}).is_err() {
        unavailable!(format!("copilot-language-server stopped during startup — see {}", log_path.display()));
        return;
    }
    let _ = server.did_change_configuration(DidChangeConfigurationParams { settings: json!({}) });

    // Deliberately just a status *check*, never the device-flow prompt
    // `copilot_agent` shows for chat — see this module's own doc comment.
    let sign_in = match drive!(server.request::<SignIn>(json!({}))) {
        Some(Ok(result)) => result,
        Some(Err(err)) => {
            unavailable!(format!("copilot: sign-in check failed: {err}"));
            return;
        }
        None => {
            unavailable!(format!("copilot-language-server stopped during startup — see {}", log_path.display()));
            return;
        }
    };
    if sign_in.get("status").and_then(Value::as_str) != Some("AlreadySignedIn") {
        unavailable!("not signed in to GitHub Copilot yet — sign in via the AI Chat Assist panel first (switch its provider to Copilot)".to_string());
        return;
    }

    let (cmd_tx, mut cmd_rx) = mpsc::channel::<CopilotCompletionCommand>(32);
    if output.send(CopilotCompletionEvent::Ready(cmd_tx)).await.is_err() {
        return;
    }

    let mut doc_versions: HashMap<Url, i32> = HashMap::new();
    // Requests in flight for `Suggest`, tagged with the generation counter
    // below so a response that's no longer the most recent request can be
    // dropped rather than shown — see `CopilotCompletionCommand::Suggest`'s
    // own doc comment on why these are never awaited one-at-a-time.
    let mut inflight = FuturesUnordered::new();
    let mut latest_suggest: u64 = 0;

    loop {
        // `futures::select!` (unlike `tokio::select!`) has no `, if cond`
        // guard syntax, and an unguarded `inflight.next()` would busy-spin
        // the loop while empty (`FuturesUnordered::next()` resolves
        // `Ready(None)` on every poll when empty, rather than pending like
        // an mpsc channel does) — `Either` swaps in a future that never
        // resolves at all for that case instead.
        let next_result = if inflight.is_empty() {
            futures::future::Either::Left(futures::future::pending())
        } else {
            futures::future::Either::Right(inflight.next())
        };
        futures::pin_mut!(next_result);

        futures::select! {
            r = mainloop_fut => {
                if let Err(err) = r {
                    let line = format!("\n[copilot-completion-mainloop] error: {err:?}\n");
                    let _ = std::fs::OpenOptions::new().append(true).open(&log_path)
                        .and_then(|mut f| { use std::io::Write; f.write_all(line.as_bytes()) });
                }
                break;
            }
            cmd = cmd_rx.next() => {
                let Some(cmd) = cmd else { break };
                match cmd {
                    CopilotCompletionCommand::DidOpen { uri, text } => {
                        doc_versions.insert(uri.clone(), 0);
                        let _ = server.did_open(DidOpenTextDocumentParams {
                            text_document: TextDocumentItem { uri, language_id: String::new(), version: 0, text },
                        });
                    }
                    CopilotCompletionCommand::DidChange { uri, text } => {
                        let version = doc_versions.entry(uri.clone()).or_insert(0);
                        *version += 1;
                        let _ = server.did_change(DidChangeTextDocumentParams {
                            text_document: VersionedTextDocumentIdentifier { uri, version: *version },
                            content_changes: vec![TextDocumentContentChangeEvent { range: None, range_length: None, text }],
                        });
                    }
                    CopilotCompletionCommand::DidClose { uri } => {
                        doc_versions.remove(&uri);
                        let _ = server.did_close(DidCloseTextDocumentParams { text_document: TextDocumentIdentifier { uri } });
                    }
                    CopilotCompletionCommand::DidFocus { uri } => {
                        let _ = server.notify::<DidFocus>(json!({"textDocument": {"uri": uri}}));
                    }
                    CopilotCompletionCommand::Suggest { uri, line, character } => {
                        latest_suggest += 1;
                        let generation = latest_suggest;
                        let version = *doc_versions.get(&uri).unwrap_or(&0);
                        let params = json!({
                            "textDocument": {"uri": uri.to_string(), "version": version},
                            "position": {"line": line, "character": character},
                            "context": {"triggerKind": 2},
                            "formattingOptions": {"tabSize": 4, "insertSpaces": true},
                        });
                        // `ServerSocket::request` is `async fn(&self, ...)`, so
                        // the future it returns borrows `server` for its whole
                        // lifetime — storing several of those in `inflight`
                        // across loop iterations would keep `server` borrowed
                        // immutably for as long as any of them are still
                        // pending, which conflicts with the `&mut self` calls
                        // (`did_open`/`did_change`/...) elsewhere in this same
                        // match. Cloning `ServerSocket` (cheap — it's a thin
                        // handle around a channel sender) and moving the clone
                        // into its own `async move` block sidesteps that: the
                        // block owns its clone outright, independent of the
                        // outer `server` this loop keeps using.
                        let cloned = server.clone();
                        let request = async move { cloned.request::<InlineCompletion>(params).await };
                        inflight.push(request.map(move |result| (generation, uri, line, character, result)));
                    }
                    CopilotCompletionCommand::Accepted { item } => {
                        if let Some(command) = item.get("command") {
                            let exec = ExecuteCommandParams {
                                command: command.get("command").and_then(Value::as_str).unwrap_or_default().to_string(),
                                arguments: command.get("arguments").and_then(Value::as_array).cloned().unwrap_or_default(),
                                work_done_progress_params: WorkDoneProgressParams::default(),
                            };
                            // Best-effort acceptance telemetry — awaited (via
                            // `drive!`, so `mainloop_fut` keeps running
                            // alongside it) rather than fired-and-dropped: an
                            // async-fn future that's never polled at all never
                            // actually sends its request either (the send
                            // happens on first poll, not on the call), so a
                            // bare `let _ = server.execute_command(exec);`
                            // here would silently never reach the server.
                            let _ = drive!(server.execute_command(exec));
                        }
                        let _ = server.notify::<DidShowCompletion>(json!({"item": item}));
                    }
                }
            }
            result = next_result => {
                let Some((generation, uri, line, character, result)) = result else { continue };
                if generation != latest_suggest {
                    continue; // superseded by a newer `Suggest` — see the field's own doc
                }
                let item = match result {
                    Ok(value) => first_item(&value),
                    Err(_) => None, // cancelled server-side, or errored — either way, no suggestion
                };
                if output.send(CopilotCompletionEvent::Suggestion { uri, line, character, item }).await.is_err() {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/copilot_completion.rs"]
mod tests;
