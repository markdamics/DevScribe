//! Embeds `copilot-language-server` (the npm package `@github/copilot-language-server`,
//! the same LSP-over-stdio server VS Code/Neovim/JetBrains Copilot plugins run
//! under the hood) as the `ChatProvider::Copilot` backend for the AI Chat
//! Assist panel — the counterpart to `claude_agent` for `ChatProvider::Claude`.
//!
//! Unlike `claude_agent`'s wire format, the base protocol here really is LSP
//! (JSON-RPC with `Content-Length` framing, driven via `async_lsp` exactly
//! like `lsp.rs`), so the handshake (`initialize`/`initialized`) and error
//! handling ride on that crate's generated `LanguageServer`/`LanguageClient`
//! methods. But the chat-specific methods (`signIn`, `conversation/create`,
//! `conversation/turn`, and the `$/progress` notifications a turn streams its
//! reply through) are undocumented custom extensions: the package's own
//! README covers auth and inline completions but says nothing about
//! `conversation/*` at all. Every shape this module relies on for those was
//! read out of the shipped bundle's own runtime parameter validators
//! (`dist/main.js`, package `@github/copilot-language-server@1.539.0`) rather
//! than a spec or documentation — treat this module, like `claude_agent`, as
//! the single point that needs updating if a future server version changes
//! that wire format. In particular: `Params`/`Result` types here are kept as
//! raw `serde_json::Value` rather than typed structs, on purpose — deriving
//! `serde` types for an undocumented, version-drifting shape would just
//! relocate the guesswork into types that look more authoritative than they
//! are.
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::process::Stdio;

use async_lsp::lsp_types::notification::Notification;
use async_lsp::lsp_types::request::Request;
use async_lsp::lsp_types::{
    ClientCapabilities, DidChangeConfigurationParams, ExecuteCommandParams, InitializeParams,
    InitializedParams, Url, WorkDoneProgressParams, WorkspaceClientCapabilities, WorkspaceFolder,
};
use async_lsp::router::Router;
use async_lsp::{LanguageServer, MainLoop};
use futures::channel::mpsc;
use futures::{FutureExt, SinkExt, StreamExt};
use serde_json::{json, Value};

use crate::claude_agent::{ClaudeCommand, ClaudeEvent};

enum SignIn {}
impl Request for SignIn {
    type Params = Value;
    type Result = Value;
    const METHOD: &'static str = "signIn";
}

/// Blocks (server-side) until the device flow started by `SignIn` completes
/// or fails — see `run`'s own doc comment on the auth flow.
enum SignInConfirm {}
impl Request for SignInConfirm {
    type Params = Value;
    type Result = Value;
    const METHOD: &'static str = "signInConfirm";
}

/// Lists the chat models this account can use — required *before*
/// `ConversationCreate`/`ConversationTurn`: despite `modelInfo`/`model` being
/// schema-`Optional` on both (see their own params, read the same way from
/// the bundle), the real server rejects a turn with no model id at all
/// (`"A model id is required: provide modelInfo.id or the deprecated model
/// field"`, jsonrpc error -32603) rather than picking one on its own. Result
/// is a raw JSON array of model objects (`{id, isChatDefault,
/// isChatFallback, ...}` — see `pick_model_id`), not wrapped in an object.
enum CopilotModels {}
impl Request for CopilotModels {
    type Params = Value;
    type Result = Value;
    const METHOD: &'static str = "copilot/models";
}

enum ConversationCreate {}
impl Request for ConversationCreate {
    type Params = Value;
    type Result = Value;
    const METHOD: &'static str = "conversation/create";
}

enum ConversationTurn {}
impl Request for ConversationTurn {
    type Params = Value;
    type Result = Value;
    const METHOD: &'static str = "conversation/turn";
}

/// A turn's reply streams back as `$/progress` notifications keyed by the
/// `workDoneToken` the client supplied in `ConversationCreate`/`ConversationTurn`'s
/// own params — *not* as the request's response, which only resolves once the
/// whole turn (including all streaming) has finished. Registered as a raw
/// `Value` rather than routed through `async_lsp`'s typed `LanguageClient::progress`:
/// that decodes into the *standard* `WorkDoneProgress` shape
/// (`begin`/`report`/`end` with only `title`/`message`/`percentage`/`cancellable`),
/// which would silently drop every field this module actually needs
/// (`reply`, `conversationId`, `turnId`, ...).
enum RawProgress {}
impl Notification for RawProgress {
    type Params = Value;
    const METHOD: &'static str = "$/progress";
}

/// Pulls the incremental reply text (if any) for the in-flight turn
/// identified by `token` out of one raw `$/progress` notification's params
/// — pure and side-effect-free so it's unit-testable without a live server;
/// the one caller (`run`'s three-way select loop) does the actual streaming.
/// `None` for progress belonging to a different token (a stray notification
/// from an earlier, already-finished turn), or one carrying no text at all
/// (a `tokenUsage`-only report, `begin`, ...).
///
/// Reply text arrives in *two* different shapes depending on which internal
/// path the chosen model runs through, and both are load-bearing: a plain
/// chat turn reports `{reply: "<delta>"}`, while a tool-capable model running
/// the agent path reports `{editAgentRounds: [{roundId, reply: "<delta>"}]}`
/// instead — captured from the real server, where a turn answered by
/// `MAI-Code-1.1-Flash` streamed *only* the latter and reading just `reply`
/// silently dropped the entire response. Both carry incremental deltas (the
/// server accumulates them its own side), never a cumulative resend, so
/// concatenating everything present here is correct rather than duplicating.
///
/// A turn that fails or gets content-filtered reports that via `kind:"end"`
/// plus an `error.message`, confirmed from the bundle's own turn handler
/// (`handleTemplateResponse`'s `endProgress({error:{message, ...}})`) rather
/// than a reply chunk — surfaced only when `accumulated_is_empty` (nothing
/// streamed yet for this turn) so a real error message never clobbers actual
/// reply text that already arrived.
fn extract_progress_delta(token: &str, params: &Value, accumulated_is_empty: bool) -> Option<String> {
    if params.get("token").and_then(Value::as_str) != Some(token) {
        return None;
    }
    let payload = params.get("value")?;

    let mut delta = String::new();
    if let Some(reply) = payload.get("reply").and_then(Value::as_str) {
        delta.push_str(reply);
    }
    if let Some(rounds) = payload.get("editAgentRounds").and_then(Value::as_array) {
        for round in rounds {
            if let Some(reply) = round.get("reply").and_then(Value::as_str) {
                delta.push_str(reply);
            }
        }
    }
    if !delta.is_empty() {
        return Some(delta);
    }

    if accumulated_is_empty && payload.get("kind").and_then(Value::as_str) == Some("end") {
        return payload.get("error").and_then(|e| e.get("message")).and_then(Value::as_str).map(str::to_string);
    }
    None
}

/// Every model in `copilot/models`'s list carries a `scopes` array saying
/// what it may be used for: a *chat* model gets `["chat-panel", "inline"]`
/// (plus `"agent-panel"` when it supports tool calls with a large enough
/// context), while a *code-completion* model gets `["completion"]`. Filtering
/// on this is not optional — `copilot/models` deliberately lists completion
/// models too (its `shouldExposeModel` returns `true` for every
/// `capabilities.type === "completion"` entry, and only gates *chat* models
/// behind `model_picker_enabled`), so an unfiltered "first entry"/"default
/// flag" pick happily lands on something like `gpt-41-copilot` and the turn
/// then dies with `"No model configuration found for id 'gpt-41-copilot'"`
/// (jsonrpc error -32603) — the conversation side resolves an id only
/// against models whose `capabilities.type` is `"chat"`.
const CHAT_SCOPE: &str = "chat-panel";

/// Picks which model id to send on every turn out of `copilot/models`'s raw
/// array result — pure and unit-testable, same shape as
/// `extract_progress_delta`. Considers only chat models (see `CHAT_SCOPE`),
/// preferring the account's own `isChatDefault` (what a human gets by never
/// touching Copilot's model picker), then `isChatFallback`, then simply the
/// first chat model rather than leaving the turn with no model id at all.
/// `None` when the account has no chat model available — a real error worth
/// surfacing, not something to paper over with a hardcoded guess that would
/// drift out of date.
fn pick_model_id(models: &Value) -> Option<&str> {
    let is_chat_model = |m: &&Value| {
        m.get("scopes")
            .and_then(Value::as_array)
            .is_some_and(|scopes| scopes.iter().any(|s| s.as_str() == Some(CHAT_SCOPE)))
    };
    let chat_models: Vec<&Value> = models.as_array()?.iter().filter(is_chat_model).collect();

    let by_flag = |flag: &str| chat_models.iter().copied().find(|m| m.get(flag).and_then(Value::as_bool) == Some(true));
    by_flag("isChatDefault")
        .or_else(|| by_flag("isChatFallback"))
        .or_else(|| chat_models.first().copied())
        .and_then(|m| m.get("id"))
        .and_then(Value::as_str)
}

/// Spawns `copilot-language-server` for `root` and drives it until the
/// process dies or `output` is dropped — the `ChatProvider::Copilot`
/// counterpart to `claude_agent::run`, feeding the very same `ClaudeEvent`
/// stream so `devscribe`'s chat panel/state reducer (`handle_chat_event`)
/// don't need to know which provider produced them at all.
///
/// Scope, deliberately: plain conversational chat only (the server's default
/// "Ask" `chatMode`), one conversation per worker spawn — no resume-across-
/// restarts (unlike Claude, whose sessions are resumed from saved transcript
/// files; Copilot's own `conversation/persistence` is a separate, equally
/// undocumented mechanism not implemented here), and no tool-call/agent-mode
/// support (`ClaudeCommand::RespondPermission` is a no-op — nothing here ever
/// produces a `PermissionRequest` to answer). Extending either is future work,
/// not a limitation of the protocol itself.
///
/// `binary` is the resolved path (or bare name) of `copilot-language-server`;
/// callers (see `devscribe`'s `chat_worker`) are responsible for locating it
/// on PATH first and emitting `Unavailable` if it isn't found, same
/// convention as `claude_agent::run`/`lsp::run`.
pub async fn run(root: PathBuf, binary: PathBuf, mut output: mpsc::Sender<ClaudeEvent>) {
    let log_path = dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("devscribe")
        .join("logs")
        .join("copilot.stderr.log");
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
            let _ = output.send(ClaudeEvent::Unavailable(format!("copilot-language-server not available: {err}"))).await;
            return;
        }
    };
    let child_stdout = child.stdout.take().expect("piped stdout");
    let child_stdin = child.stdin.take().expect("piped stdin");

    let (progress_tx, mut progress_rx) = mpsc::unbounded::<Value>();
    let (mainloop, mut server) = MainLoop::new_client(|_server| {
        let mut router: Router<()> = Router::new(());
        router.notification::<RawProgress>(move |_, params: Value| {
            let _ = progress_tx.unbounded_send(params);
            ControlFlow::Continue(())
        });
        // Everything else server->client (window/logMessage, telemetry/event,
        // didChangeStatus, ...) — not needed for this minimal-scope chat
        // integration; ignored rather than left to the router's own default,
        // which breaks the mainloop for any *non*-`$/`-prefixed method it
        // doesn't recognize (same defensive catch-all `lsp.rs` registers for
        // jdtls's own non-standard chatter).
        router.unhandled_notification(|_, _| ControlFlow::Continue(()));
        router
    });
    let mainloop_fut = mainloop.run_buffered(child_stdout, child_stdin).fuse();
    futures::pin_mut!(mainloop_fut);

    // Drives `fut` alongside `mainloop_fut` (same "pinned future reused
    // across many sequential `select!`s" shape `lsp.rs` uses for its own
    // request handling) — `None` means the connection died while waiting.
    macro_rules! drive {
        ($fut:expr) => {{
            let f = $fut.fuse();
            futures::pin_mut!(f);
            loop {
                futures::select! {
                    r = f => break Some(r),
                    r = mainloop_fut => {
                        if let Err(err) = r {
                            let line = format!("\n[copilot-mainloop] error: {err:?}\n");
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
            let _ = output.send(ClaudeEvent::Unavailable($msg)).await;
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
    // Per the package's README: informs the server of the initial config
    // (defaults are fine for this minimal integration — no proxy/enterprise
    // settings surfaced yet).
    let _ = server.did_change_configuration(DidChangeConfigurationParams { settings: json!({}) });

    // --- Authentication ---------------------------------------------------
    // `signIn` (aliased server-side to `signInInitiate`) returns either
    // `{status:"AlreadySignedIn", user}` or `{status:"PromptUserDeviceFlow",
    // userCode, verificationUri, command}` (the README only documents the
    // latter's `userCode`/`command`; the discriminating `status` field and
    // the already-signed-in case were read out of the bundle's own
    // `handleSignInInitiateChecked`). There's no bespoke UI for the device
    // code here — it's surfaced as a normal assistant bubble, the same
    // precedent `claude`'s own `/model`/`/usage` slash-command replies
    // already establish (a synthetic informational reply rendered exactly
    // like any other `AssistantText`).
    let sign_in = match drive!(server.request::<SignIn>(json!({}))) {
        Some(Ok(result)) => result,
        Some(Err(err)) => {
            unavailable!(format!("copilot sign-in request failed: {err}"));
            return;
        }
        None => {
            unavailable!(format!("copilot-language-server stopped during sign-in — see {}", log_path.display()));
            return;
        }
    };
    if sign_in.get("status").and_then(Value::as_str) == Some("PromptUserDeviceFlow") {
        let user_code = sign_in.get("userCode").and_then(Value::as_str).unwrap_or("?");
        let verification_uri = sign_in.get("verificationUri").and_then(Value::as_str).unwrap_or("https://github.com/login/device");
        let notice = format!(
            "**Sign in to GitHub Copilot** — open {verification_uri} and enter code `{user_code}` \
             (opening your browser now\u{2026})"
        );
        if output.send(ClaudeEvent::AssistantText(notice)).await.is_err() {
            return;
        }
        if let Some(command) = sign_in.get("command") {
            let exec = ExecuteCommandParams {
                command: command.get("command").and_then(Value::as_str).unwrap_or_default().to_string(),
                arguments: command.get("arguments").and_then(Value::as_array).cloned().unwrap_or_default(),
                work_done_progress_params: WorkDoneProgressParams::default(),
            };
            // Best-effort: if opening the browser this way fails, the user
            // can still visit `verification_uri` manually — the message
            // above already gave them everything needed to.
            let _ = drive!(server.execute_command(exec));
        }
        match drive!(server.request::<SignInConfirm>(json!({}))) {
            Some(Ok(_)) => {}
            Some(Err(err)) => {
                unavailable!(format!("GitHub Copilot sign-in failed: {err}"));
                return;
            }
            None => {
                unavailable!(format!("copilot-language-server stopped during sign-in — see {}", log_path.display()));
                return;
            }
        }
        if output.send(ClaudeEvent::AssistantText("Signed in to GitHub Copilot.".to_string())).await.is_err() {
            return;
        }
    }

    // Required up front — see `CopilotModels`'s own doc comment on why an
    // `Optional` schema field still turns out to be mandatory in practice.
    let models = match drive!(server.request::<CopilotModels>(json!({}))) {
        Some(Ok(models)) => models,
        Some(Err(err)) => {
            unavailable!(format!("copilot: couldn't list available models: {err}"));
            return;
        }
        None => {
            unavailable!(format!("copilot-language-server stopped while listing models — see {}", log_path.display()));
            return;
        }
    };
    let Some(model_id) = pick_model_id(&models).map(str::to_string) else {
        unavailable!("copilot: this account has no chat models available".to_string());
        return;
    };

    let (cmd_tx, mut cmd_rx) = mpsc::channel::<ClaudeCommand>(32);
    if output.send(ClaudeEvent::Ready(cmd_tx)).await.is_err() {
        return;
    }

    // A `file://` URI, not a bare path — the server parses this field as a
    // URI and rejects a plain filesystem path with `"Could not parse <path>"`
    // (jsonrpc error -32603), confirmed against the real server. `root`'s
    // own URI already parsed successfully earlier (building `initialize`'s
    // `root_uri`), so this can't newly fail here.
    let workspace_folder = Url::from_file_path(&root).map(|u| u.to_string()).unwrap_or_else(|()| root.to_string_lossy().into_owned());
    let mut conversation_id: Option<String> = None;
    let mut session_init_sent = false;

    loop {
        futures::select! {
            r = mainloop_fut => {
                if let Err(err) = r {
                    let line = format!("\n[copilot-mainloop] error: {err:?}\n");
                    let _ = std::fs::OpenOptions::new().append(true).open(&log_path)
                        .and_then(|mut f| { use std::io::Write; f.write_all(line.as_bytes()) });
                }
                break;
            }
            // Progress that arrives *between* turns (shouldn't normally
            // happen — see the three-way select below, which is what
            // actually drains it while a turn is in flight) — drop rather
            // than crash on an unmatched notification.
            _ = progress_rx.next() => {}
            cmd = cmd_rx.next() => {
                let Some(cmd) = cmd else { break };
                let ClaudeCommand::SendPrompt(text) = cmd else {
                    // `RespondPermission` — no-op, see `run`'s own doc comment.
                    continue;
                };
                let work_done_token = uuid::Uuid::new_v4().to_string();
                let request_fut = match &conversation_id {
                    Some(cid) => server.request::<ConversationTurn>(json!({
                        "workDoneToken": work_done_token,
                        "conversationId": cid,
                        "message": text,
                        "workspaceFolder": workspace_folder,
                        "modelInfo": {"id": model_id},
                    })).left_future(),
                    None => server.request::<ConversationCreate>(json!({
                        "workDoneToken": work_done_token,
                        "workspaceFolder": workspace_folder,
                        "source": "panel",
                        "turns": [{"request": text}],
                        "modelInfo": {"id": model_id},
                    })).right_future(),
                };

                // A three-way race, for the duration of one turn: the
                // request itself (which only *resolves* once the whole
                // reply has finished generating — confirmed from the
                // bundle's own turn handler, which awaits the full
                // processor before returning its response), `$/progress`
                // deltas streaming in the meantime, and the mainloop that
                // both ride on. Awaiting the request alone (the `drive!`
                // macro's shape) would buffer every delta silently until
                // the turn finished, then flush them all at once — no live-
                // typed effect at all.
                let request_fut = request_fut.fuse();
                futures::pin_mut!(request_fut);
                let mut accumulated = String::new();
                let outcome = loop {
                    futures::select! {
                        r = request_fut => break Some(r),
                        r = mainloop_fut => {
                            if let Err(err) = r {
                                let line = format!("\n[copilot-mainloop] error: {err:?}\n");
                                let _ = std::fs::OpenOptions::new().append(true).open(&log_path)
                                    .and_then(|mut f| { use std::io::Write; f.write_all(line.as_bytes()) });
                            }
                            break None;
                        }
                        item = progress_rx.next() => {
                            let Some(value) = item else { continue };
                            if let Some(delta) = extract_progress_delta(&work_done_token, &value, accumulated.is_empty()) {
                                accumulated.push_str(&delta);
                                if output.send(ClaudeEvent::AssistantTextDelta(delta)).await.is_err() {
                                    return;
                                }
                            }
                        }
                    }
                };

                match outcome {
                    Some(Ok(result)) => {
                        if conversation_id.is_none() {
                            let Some(cid) = result.get("conversationId").and_then(Value::as_str) else {
                                unavailable!("copilot: conversation/create response had no conversationId".to_string());
                                return;
                            };
                            conversation_id = Some(cid.to_string());
                        }
                        if !session_init_sent {
                            session_init_sent = true;
                            let model = result.get("modelName").and_then(Value::as_str).unwrap_or(&model_id).to_string();
                            let session_id = conversation_id.clone().unwrap_or_default();
                            if output.send(ClaudeEvent::SessionInit { session_id, model }).await.is_err() {
                                return;
                            }
                        }
                        if !accumulated.is_empty() && output.send(ClaudeEvent::AssistantText(accumulated)).await.is_err() {
                            return;
                        }
                    }
                    Some(Err(err)) => {
                        unavailable!(format!("copilot turn failed: {err}"));
                        return;
                    }
                    None => {
                        unavailable!(format!("copilot-language-server stopped — see {}", log_path.display()));
                        return;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/copilot_agent.rs"]
mod tests;
