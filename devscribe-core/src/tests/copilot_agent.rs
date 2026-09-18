use super::*;
use futures::channel::mpsc as fmpsc;
use serde_json::json;
use std::time::Duration;

/// `copilot/models`' own shape for a chat model — see `CHAT_SCOPE`.
fn chat_model(id: &str) -> Value {
    json!({"id": id, "scopes": ["chat-panel", "inline"]})
}

#[test]
fn pick_model_id_prefers_the_chat_default() {
    let models = json!([
        {"id": "gpt-4.1", "scopes": ["chat-panel", "inline"], "isChatFallback": true},
        {"id": "claude-sonnet-4", "scopes": ["chat-panel", "inline"], "isChatDefault": true},
        chat_model("o3-mini"),
    ]);
    assert_eq!(pick_model_id(&models), Some("claude-sonnet-4"));
}

#[test]
fn pick_model_id_falls_back_to_the_chat_fallback_model() {
    let models = json!([
        chat_model("o3-mini"),
        {"id": "gpt-4.1", "scopes": ["chat-panel", "inline"], "isChatFallback": true},
    ]);
    assert_eq!(pick_model_id(&models), Some("gpt-4.1"));
}

#[test]
fn pick_model_id_falls_back_to_the_first_model_with_no_flags_at_all() {
    let models = json!([chat_model("o3-mini"), chat_model("gpt-4.1")]);
    assert_eq!(pick_model_id(&models), Some("o3-mini"));
}

/// Regression test for a turn dying with `"No model configuration found for
/// id 'gpt-41-copilot'"`: `copilot/models` lists code-completion models
/// alongside chat ones, and they're never valid for `conversation/*` — even
/// when one sorts first, and even if it somehow carried a chat flag.
#[test]
fn pick_model_id_never_picks_a_code_completion_model() {
    let models = json!([
        {"id": "gpt-41-copilot", "scopes": ["completion"], "isChatDefault": true},
        chat_model("claude-sonnet-4"),
    ]);
    assert_eq!(pick_model_id(&models), Some("claude-sonnet-4"));

    let completion_only = json!([{"id": "gpt-41-copilot", "scopes": ["completion"]}]);
    assert_eq!(pick_model_id(&completion_only), None);
}

#[test]
fn pick_model_id_is_none_for_an_empty_account() {
    assert_eq!(pick_model_id(&json!([])), None);
    assert_eq!(pick_model_id(&json!({"models": []})), None); // wrong shape (object, not array)
    assert_eq!(pick_model_id(&json!([{"id": "no-scopes-at-all"}])), None);
}

fn recv_within(rx: &mut fmpsc::Receiver<ClaudeEvent>, timeout: Duration) -> Vec<ClaudeEvent> {
    let deadline = std::time::Instant::now() + timeout;
    let mut events = Vec::new();
    while std::time::Instant::now() < deadline {
        match rx.try_recv() {
            Ok(event) => events.push(event),
            Err(err) if err.is_closed() => break, // sender dropped
            Err(_) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    events
}

/// End-to-end proof that `run` actually reaches a real `copilot-language-server`,
/// completes the LSP handshake, and gets a real answer out of `signIn` — the
/// part of this module that's most likely to have drifted from the bundle's
/// actual behavior, since none of it is documented (see the module's own doc
/// comment). Doesn't complete authentication (that needs a human in a
/// browser); just confirms the protocol assumptions up to that point hold
/// against the real server rather than only against this module's own idea
/// of what the server does.
///
/// **Not run automatically** — talks to GitHub's real device-flow endpoint
/// and requires `copilot-language-server` (`npm install -g
/// @github/copilot-language-server`) on PATH. Run by hand with:
/// `cargo test -p devscribe-core copilot_agent -- --ignored --nocapture`
#[test]
#[ignore = "spawns the real copilot-language-server and talks to GitHub's device-flow endpoint — run manually"]
fn initializes_and_reaches_sign_in_end_to_end() {
    use futures::FutureExt;

    let binary = PathBuf::from("copilot-language-server");
    let dir = std::env::temp_dir().join(format!("devscribe-copilot-agent-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let (tx, mut rx) = fmpsc::channel::<ClaudeEvent>(32);
    let (cancel_tx, cancel_rx) = futures::channel::oneshot::channel::<()>();
    let root = dir.clone();
    let handle = std::thread::spawn(move || {
        futures::executor::block_on(async {
            futures::select! {
                _ = run(root, binary, tx).fuse() => {},
                _ = cancel_rx.fuse() => {}, // bounds the test: signInConfirm would otherwise block forever waiting for a human
            }
        });
    });

    let events = recv_within(&mut rx, Duration::from_secs(20));
    let _ = cancel_tx.send(());
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);

    assert!(!events.is_empty(), "expected at least one event from a real copilot-language-server run");
    assert!(
        !matches!(events.first(), Some(ClaudeEvent::Unavailable(_))),
        "expected the LSP handshake to succeed, got: {events:?}"
    );
    let reached_sign_in = events.iter().any(|e| {
        matches!(e, ClaudeEvent::AssistantText(text) if text.contains("Sign in to GitHub Copilot")) || matches!(e, ClaudeEvent::Ready(_))
    });
    assert!(reached_sign_in, "expected to reach signIn's device-flow prompt or Ready (already signed in), got: {events:?}");
}

/// End-to-end proof that a real turn actually completes — specifically
/// regression coverage for the `modelInfo`/`model` requirement: `signIn`
/// through `Ready` alone (see `initializes_and_reaches_sign_in_end_to_end`)
/// doesn't exercise `conversation/create` at all, so it never would have
/// caught the real server rejecting a turn with no model id
/// (`"A model id is required..."`, jsonrpc error -32603) despite
/// `modelInfo`/`model` both being schema-`Optional` — see `CopilotModels`'s
/// own doc comment.
///
/// **Not run automatically**, same reasons as `claude_agent`'s own billed
/// end-to-end test: this one sends one real prompt to a real, already-
/// signed-in GitHub Copilot account and consumes real usage. Skips itself
/// (rather than failing) if the account isn't already signed in — this test
/// is for the turn/model-id path, not a second copy of the sign-in test.
/// Run by hand with: `cargo test -p devscribe-core copilot_agent -- --ignored --nocapture`
#[test]
#[ignore = "spawns the real copilot-language-server and sends one real, billed prompt — run manually, requires an already-signed-in account"]
fn sends_a_real_turn_and_gets_a_reply_end_to_end() {
    use futures::FutureExt;

    let binary = PathBuf::from("copilot-language-server");
    let dir = std::env::temp_dir().join(format!("devscribe-copilot-agent-turn-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let (tx, mut rx) = fmpsc::channel::<ClaudeEvent>(32);
    let (cancel_tx, cancel_rx) = futures::channel::oneshot::channel::<()>();
    let root = dir.clone();
    let handle = std::thread::spawn(move || {
        futures::executor::block_on(async {
            futures::select! {
                _ = run(root, binary, tx).fuse() => {},
                _ = cancel_rx.fuse() => {},
            }
        });
    });

    // Wait for `Ready` specifically (not just "some events") — if the
    // account needs the device-flow prompt, there's no `Ready` within this
    // window and the test skips rather than hangs/fails.
    let setup_events = recv_within(&mut rx, Duration::from_secs(15));
    let sender = setup_events.iter().find_map(|e| match e {
        ClaudeEvent::Ready(sender) => Some(sender.clone()),
        _ => None,
    });
    let Some(mut sender) = sender else {
        let _ = cancel_tx.send(());
        handle.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        eprintln!("skipping: account isn't already signed in (no Ready within 15s) — got {setup_events:?}");
        return;
    };

    sender.try_send(ClaudeCommand::SendPrompt("Reply with exactly: OK".to_string())).unwrap();
    let turn_events = recv_within(&mut rx, Duration::from_secs(30));
    let _ = cancel_tx.send(());
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);

    for event in &turn_events {
        if let ClaudeEvent::Unavailable(reason) = event {
            panic!("turn failed: {reason}");
        }
    }
    let got_reply = turn_events
        .iter()
        .any(|e| matches!(e, ClaudeEvent::AssistantText(_) | ClaudeEvent::AssistantTextDelta(_)));
    assert!(got_reply, "expected at least one assistant reply chunk, got: {turn_events:?}");
}

fn delta(token: &str, params: &Value, accumulated_is_empty: bool) -> Option<String> {
    extract_progress_delta(token, params, accumulated_is_empty)
}

#[test]
fn extract_progress_delta_reads_a_reply_chunk() {
    let params = json!({"token": "t1", "value": {"kind": "report", "reply": "hello"}});
    assert_eq!(delta("t1", &params, true).as_deref(), Some("hello"));
}

/// Regression test for a turn that streamed its whole answer and showed
/// nothing: a tool-capable model (e.g. `MAI-Code-1.1-Flash`) reports its
/// reply *only* under `editAgentRounds[].reply`, never the top-level
/// `reply` — captured from the real server.
#[test]
fn extract_progress_delta_reads_an_edit_agent_round_reply() {
    let params = json!({
        "token": "t1",
        "value": {"kind": "report", "hideText": false, "editAgentRounds": [{"roundId": 1, "reply": "OK"}]},
    });
    assert_eq!(delta("t1", &params, true).as_deref(), Some("OK"));
}

#[test]
fn extract_progress_delta_concatenates_several_rounds_in_order() {
    let params = json!({
        "token": "t1",
        "value": {"editAgentRounds": [{"roundId": 1, "reply": "one "}, {"roundId": 2, "reply": "two"}]},
    });
    assert_eq!(delta("t1", &params, true).as_deref(), Some("one two"));
}

/// `hideText: true` makes the server send `reply: ""` rather than omitting
/// the round — which must not count as text, or a hidden round would end
/// the turn's terminal-error fallback prematurely.
#[test]
fn extract_progress_delta_treats_a_hidden_round_as_no_text() {
    let params = json!({"token": "t1", "value": {"hideText": true, "editAgentRounds": [{"roundId": 1, "reply": ""}]}});
    assert_eq!(delta("t1", &params, true), None);
}

#[test]
fn extract_progress_delta_ignores_a_different_turns_token() {
    let params = json!({"token": "other-turn", "value": {"kind": "report", "reply": "hello"}});
    assert_eq!(delta("t1", &params, true), None);
}

#[test]
fn extract_progress_delta_ignores_a_report_with_no_reply_field() {
    // e.g. a `tokenUsage`-only report, per the bundle's own turn handler.
    let params = json!({"token": "t1", "value": {"kind": "report", "tokenUsage": {"promptTokens": 5}}});
    assert_eq!(delta("t1", &params, true), None);
}

#[test]
fn extract_progress_delta_surfaces_a_terminal_error_when_nothing_streamed_yet() {
    let params = json!({"token": "t1", "value": {"kind": "end", "error": {"message": "content filtered"}}});
    assert_eq!(delta("t1", &params, true).as_deref(), Some("content filtered"));
}

#[test]
fn extract_progress_delta_does_not_clobber_already_streamed_text_with_a_terminal_error() {
    let params = json!({"token": "t1", "value": {"kind": "end", "error": {"message": "content filtered"}}});
    assert_eq!(delta("t1", &params, false), None);
}

#[test]
fn extract_progress_delta_ignores_a_plain_end_with_no_error() {
    let params = json!({"token": "t1", "value": {"kind": "end"}});
    assert_eq!(delta("t1", &params, true), None);
}

// -----------------------------------------------------------------------
// DevScribe-owned session history
// -----------------------------------------------------------------------

/// A scratch project root plus its matching Copilot-session directory
/// under the real data dir (`dirs::data_dir()/devscribe/copilot_sessions/`)
/// — same "no injectable data dir to fake this with, so use a PID-suffixed
/// unique project path instead" reasoning as `claude_agent`'s own
/// `FakeProject`, cleaned up on drop.
struct FakeCopilotProject {
    root: PathBuf,
    sessions_dir: PathBuf,
}

impl FakeCopilotProject {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!("devscribe-copilot-session-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let sessions_dir = copilot_sessions_dir(&root).expect("data dir must resolve on any machine running these tests");
        let _ = std::fs::remove_dir_all(&sessions_dir);
        Self { root, sessions_dir }
    }
}

impl Drop for FakeCopilotProject {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
        let _ = std::fs::remove_dir_all(&self.sessions_dir);
    }
}

#[test]
fn copilot_session_round_trips_through_save_and_load() {
    let project = FakeCopilotProject::new("round-trip");
    let turns = vec![
        (CopilotSessionRole::Operator, "what does this do?".to_string()),
        (CopilotSessionRole::Assistant, "it does the thing.".to_string()),
    ];
    assert!(!copilot_session_exists(&project.root, "s1"));
    save_copilot_session(&project.root, "s1", &turns);
    assert!(copilot_session_exists(&project.root, "s1"));

    let events = load_copilot_session_history(&project.root, "s1");
    assert_eq!(events.len(), 2);
    assert!(matches!(&events[0], ClaudeEvent::OperatorText(t) if t == "what does this do?"));
    assert!(matches!(&events[1], ClaudeEvent::AssistantText(t) if t == "it does the thing."));
}

#[test]
fn save_copilot_session_is_a_no_op_for_an_empty_transcript() {
    let project = FakeCopilotProject::new("empty");
    save_copilot_session(&project.root, "s1", &[]);
    assert!(!copilot_session_exists(&project.root, "s1"), "an empty turn list shouldn't create a file");
}

#[test]
fn save_copilot_session_overwrites_rather_than_appends() {
    let project = FakeCopilotProject::new("overwrite");
    save_copilot_session(&project.root, "s1", &[(CopilotSessionRole::Operator, "first".to_string())]);
    save_copilot_session(
        &project.root,
        "s1",
        &[(CopilotSessionRole::Operator, "first".to_string()), (CopilotSessionRole::Assistant, "second".to_string())],
    );
    let events = load_copilot_session_history(&project.root, "s1");
    assert_eq!(events.len(), 2, "the second save should replace, not append to, the first");
}

#[test]
fn list_copilot_sessions_is_empty_for_a_project_that_never_had_one() {
    let root = std::env::temp_dir().join(format!("devscribe-copilot-session-test-nonexistent-{}", std::process::id()));
    assert!(list_copilot_sessions(&root).is_empty());
}

#[test]
fn list_copilot_sessions_sorts_most_recently_active_first_and_titles_from_the_first_operator_message() {
    let project = FakeCopilotProject::new("list-sort");
    save_copilot_session(&project.root, "older", &[(CopilotSessionRole::Operator, "first one".to_string())]);
    std::thread::sleep(Duration::from_millis(20)); // ensure a distinct, later mtime
    save_copilot_session(&project.root, "newer", &[(CopilotSessionRole::Operator, "second one".to_string())]);

    let sessions = list_copilot_sessions(&project.root);
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0].id, "newer");
    assert_eq!(sessions[0].title, "second one");
    assert_eq!(sessions[1].id, "older");
    assert_eq!(sessions[1].title, "first one");
}

#[test]
fn list_copilot_sessions_defaults_to_new_session_with_no_operator_message() {
    // Shouldn't normally happen (`save_copilot_session` is a no-op for an
    // empty transcript) but a session that somehow only ever got an
    // assistant turn shouldn't panic or show a blank title.
    let project = FakeCopilotProject::new("no-operator");
    save_copilot_session(&project.root, "s1", &[(CopilotSessionRole::Assistant, "hello".to_string())]);
    let sessions = list_copilot_sessions(&project.root);
    assert_eq!(sessions[0].title, "New session");
}

#[test]
fn list_copilot_sessions_truncates_a_long_first_message() {
    let project = FakeCopilotProject::new("title-truncate");
    let long = "x".repeat(200);
    save_copilot_session(&project.root, "s1", &[(CopilotSessionRole::Operator, long)]);
    let sessions = list_copilot_sessions(&project.root);
    assert!(sessions[0].title.ends_with('\u{2026}'), "a truncated title should end with an ellipsis, got {:?}", sessions[0].title);
    assert!(sessions[0].title.chars().count() <= 61, "should be capped near 60 chars, got {} chars", sessions[0].title.chars().count());
}
