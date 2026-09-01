use super::*;
use futures::channel::mpsc as fmpsc;
use std::time::Duration;

/// Path to the real `devscribe` binary this workspace just built — used as
/// the `--claude-permission-hook` target. Not portable outside this repo's
/// own `target/` layout, which is fine: this test only ever runs by hand,
/// never in CI (see the module doc below).
fn devscribe_exe() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/debug/devscribe")
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

fn find_command_sender(events: &[ClaudeEvent]) -> Option<mpsc::Sender<ClaudeCommand>> {
    events.iter().find_map(|e| match e {
        ClaudeEvent::Ready(sender) => Some(sender.clone()),
        _ => None,
    })
}

fn find_permission_request<'a>(events: &'a [ClaudeEvent], for_path_containing: &str) -> Option<&'a str> {
    events.iter().find_map(|e| match e {
        ClaudeEvent::PermissionRequest { id, tool_input, .. }
            if tool_input.get("file_path").and_then(Value::as_str).is_some_and(|p| p.contains(for_path_containing)) =>
        {
            Some(id.as_str())
        }
        _ => None,
    })
}

/// A scratch project root plus its matching session-transcript directory
/// under `~/.claude/projects/` (the same real location the app itself
/// reads/writes — there's no injectable "home dir" to fake this with, so
/// the safe way to test it is a project path unique enough (PID-suffixed)
/// that it can never collide with a real one, cleaned up on drop.
struct FakeProject {
    root: PathBuf,
    session_dir: PathBuf,
}

impl FakeProject {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!("devscribe-session-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let session_dir = project_session_dir(&root).expect("home dir must resolve on any machine running these tests");
        let _ = std::fs::remove_dir_all(&session_dir);
        std::fs::create_dir_all(&session_dir).unwrap();
        Self { root, session_dir }
    }

    fn write_transcript(&self, session_id: &str, lines: &[Value]) {
        let text = lines.iter().map(|v| v.to_string()).collect::<Vec<_>>().join("\n");
        std::fs::write(self.session_dir.join(format!("{session_id}.jsonl")), text).unwrap();
    }
}

impl Drop for FakeProject {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
        let _ = std::fs::remove_dir_all(&self.session_dir);
    }
}

#[test]
fn new_session_id_produces_distinct_valid_uuids() {
    let a = new_session_id();
    let b = new_session_id();
    assert_ne!(a, b);
    assert!(uuid::Uuid::parse_str(&a).is_ok(), "{a:?} should be a valid UUID");
}

#[test]
fn session_title_prefers_the_latest_ai_title_over_the_first_message() {
    let project = FakeProject::new("title-ai");
    project.write_transcript(
        "s1",
        &[
            json!({"type": "user", "message": {"role": "user", "content": "help me fix this bug"}}),
            json!({"type": "ai-title", "aiTitle": "First guess", "sessionId": "s1"}),
            json!({"type": "ai-title", "aiTitle": "Fixing the null-pointer bug", "sessionId": "s1"}),
        ],
    );
    let title = session_title(&project.session_dir.join("s1.jsonl"));
    assert_eq!(title.as_deref(), Some("Fixing the null-pointer bug"));
}

#[test]
fn session_title_falls_back_to_the_first_operator_message_with_no_ai_title_yet() {
    let project = FakeProject::new("title-fallback");
    project.write_transcript(
        "s1",
        &[
            json!({"type": "queue-operation", "operation": "enqueue"}),
            json!({"type": "user", "message": {"role": "user", "content": "  explain this function  "}}),
        ],
    );
    let title = session_title(&project.session_dir.join("s1.jsonl"));
    assert_eq!(title.as_deref(), Some("explain this function"), "should be trimmed");
}

#[test]
fn session_title_truncates_a_long_first_message() {
    let project = FakeProject::new("title-truncate");
    let long = "x".repeat(200);
    project.write_transcript("s1", &[json!({"type": "user", "message": {"role": "user", "content": long}})]);
    let title = session_title(&project.session_dir.join("s1.jsonl")).unwrap();
    assert!(title.ends_with('\u{2026}'), "a truncated title should end with an ellipsis, got {title:?}");
    assert!(title.chars().count() <= 81, "should be capped near 80 chars, got {} chars", title.chars().count());
}

#[test]
fn session_title_defaults_to_new_session_with_nothing_to_go_on() {
    let project = FakeProject::new("title-empty");
    project.write_transcript("s1", &[json!({"type": "queue-operation", "operation": "enqueue"})]);
    let title = session_title(&project.session_dir.join("s1.jsonl"));
    assert_eq!(title.as_deref(), Some("New session"));
}

#[test]
fn list_sessions_is_empty_for_a_project_that_never_had_one() {
    let root = std::env::temp_dir().join(format!("devscribe-session-test-nonexistent-{}", std::process::id()));
    assert!(list_sessions(&root).is_empty());
}

#[test]
fn list_sessions_sorts_most_recently_active_first_and_ignores_non_transcript_files() {
    let project = FakeProject::new("list-sort");
    project.write_transcript("older", &[json!({"type": "user", "message": {"role": "user", "content": "first one"}})]);
    std::thread::sleep(Duration::from_millis(20)); // ensure a distinct, later mtime
    project.write_transcript("newer", &[json!({"type": "user", "message": {"role": "user", "content": "second one"}})]);
    std::fs::write(project.session_dir.join("not-a-session.txt"), "ignore me").unwrap();

    let sessions = list_sessions(&project.root);

    assert_eq!(sessions.len(), 2, "the stray .txt file must not show up as a session: {sessions:?}");
    assert_eq!(sessions[0].id, "newer");
    assert_eq!(sessions[1].id, "older");
}

#[test]
fn load_session_history_reconstructs_operator_assistant_and_tool_events() {
    let project = FakeProject::new("history");
    project.write_transcript(
        "s1",
        &[
            json!({"type": "user", "message": {"role": "user", "content": "read main.rs"}}),
            json!({
                "type": "assistant",
                "message": {
                    "model": "claude-sonnet-5",
                    "content": [
                        {"type": "tool_use", "id": "toolu_1", "name": "Read", "input": {"file_path": "main.rs"}},
                    ],
                },
            }),
            json!({
                "type": "user",
                "message": {"content": [{"type": "tool_result", "tool_use_id": "toolu_1", "content": "fn main() {}"}]},
                "tool_use_result": {"type": "text", "file": {"content": "fn main() {}"}},
            }),
            json!({
                "type": "assistant",
                "message": {"model": "claude-sonnet-5", "content": [{"type": "text", "text": "It's a minimal main function."}]},
            }),
        ],
    );

    let history = load_session_history(&project.root, "s1");
    let events = history.events;

    assert!(!history.truncated, "well under DEFAULT_HISTORY_LINES, nothing should be capped");
    assert!(matches!(&events[0], ClaudeEvent::OperatorText(t) if t == "read main.rs"));
    assert!(matches!(&events[1], ClaudeEvent::ToolUseStarted { name, .. } if name == "Read"));
    assert!(matches!(&events[2], ClaudeEvent::ToolResult { id, is_error: false, .. } if id == "toolu_1"));
    assert!(matches!(&events[3], ClaudeEvent::AssistantText(t) if t == "It's a minimal main function."));
}

#[test]
fn load_session_history_is_empty_for_a_missing_transcript() {
    let project = FakeProject::new("history-missing");
    let history = load_session_history(&project.root, "does-not-exist");
    assert!(history.events.is_empty());
    assert!(!history.truncated);
}

#[test]
fn load_session_history_caps_to_the_most_recent_lines_and_flags_truncation() {
    // A long-running session's saved transcript can run to thousands of
    // lines; replaying (and forever after re-rendering) all of it on every
    // resume is exactly the unbounded-memory-growth the cap exists to
    // avoid. `DEFAULT_HISTORY_LINES + 50` operator lines, each uniquely
    // numbered, lets this both prove the cap fires and pin *which* lines
    // survive (the most recent ones, not an arbitrary subset).
    let project = FakeProject::new("history-long");
    let total = DEFAULT_HISTORY_LINES + 50;
    let lines: Vec<Value> = (0..total)
        .map(|i| json!({"type": "user", "message": {"role": "user", "content": format!("line {i}")}}))
        .collect();
    project.write_transcript("s1", &lines);

    let capped = load_session_history(&project.root, "s1");
    assert!(capped.truncated, "a transcript longer than the cap must report truncated");
    assert_eq!(capped.events.len(), DEFAULT_HISTORY_LINES);
    assert!(
        matches!(&capped.events[0], ClaudeEvent::OperatorText(t) if t == &format!("line {}", total - DEFAULT_HISTORY_LINES)),
        "capped replay must keep the *most recent* lines, not the earliest: {:#?}",
        capped.events[0]
    );

    let full = load_full_session_history(&project.root, "s1");
    assert_eq!(full.len(), total, "the escape hatch must replay every line, uncapped");
    assert!(matches!(&full[0], ClaudeEvent::OperatorText(t) if t == "line 0"));
}

/// `--include-partial-messages`'s live-typing chunks — captured against the
/// real CLI (v2.1.251): `content_block_delta`/`text_delta` is the only
/// delta shape that should surface as `AssistantTextDelta`. A sibling
/// `thinking_delta` (extended-thinking output, not shown in the transcript
/// today) must be ignored rather than mistaken for visible text.
#[test]
fn stream_event_text_delta_becomes_an_assistant_text_delta() {
    let mut model = None;
    let line = json!({
        "type": "stream_event",
        "event": {"type": "content_block_delta", "index": 1, "delta": {"type": "text_delta", "text": "Hel"}},
        "session_id": "s1",
    })
    .to_string();

    let events = handle_stdout_line(&line, &mut model);

    assert!(matches!(events.as_slice(), [ClaudeEvent::AssistantTextDelta(t)] if t == "Hel"));
}

#[test]
fn stream_event_thinking_delta_produces_no_event() {
    let mut model = None;
    let line = json!({
        "type": "stream_event",
        "event": {"type": "content_block_delta", "index": 0, "delta": {"type": "thinking_delta", "thinking": "hmm"}},
        "session_id": "s1",
    })
    .to_string();

    assert!(handle_stdout_line(&line, &mut model).is_empty());
}

/// Real regression test for `TempFiles`: cancels `run`'s future mid-flight
/// (via `select!` racing it against a cancellation signal — the same
/// "future gets dropped mid-`.await`, not run to completion" shape as
/// iced tearing down a subscription) and confirms the generated socket and
/// settings files are gone afterward, not just left behind. Doesn't touch
/// the real `claude` CLI at all — a tiny shell script standing in for it
/// is enough, since only process-lifecycle/file-cleanup is under test
/// here, not protocol behavior (that's `approves_one_edit_and_denies_
/// another_end_to_end`, above).
#[test]
fn temp_files_are_cleaned_up_even_when_run_is_cancelled_mid_await() {
    use futures::FutureExt;

    let dir = std::env::temp_dir().join(format!("devscribe-claude-agent-cleanup-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // Ignores every flag `run` passes it and just blocks reading stdin —
    // stays alive exactly like a real, still-thinking `claude` session
    // would, without needing one.
    let fake_claude = dir.join("fake-claude.sh");
    std::fs::write(&fake_claude, "#!/bin/sh\nexec cat\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake_claude, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let prefix = format!("devscribe-claude-{}-", std::process::id());
    let matching_temp_files = || -> Vec<PathBuf> {
        std::fs::read_dir(std::env::temp_dir())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(&prefix)))
            .collect()
    };

    let (tx, _rx) = fmpsc::channel::<ClaudeEvent>(32);
    let (cancel_tx, cancel_rx) = futures::channel::oneshot::channel::<()>();
    let root = dir.clone();
    let handle = std::thread::spawn(move || {
        futures::executor::block_on(async {
            futures::select! {
                _ = run(root, fake_claude, devscribe_exe(), SessionOptions { session_id: new_session_id(), resume: false, mode: PermissionMode::Manual, allow_bash: false }, tx).fuse() => {},
                _ = cancel_rx.fuse() => {}, // drops `run`'s future here, mid-`.await`
            }
        });
    });

    // Give it time to actually spawn the subprocess and create its files.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while matching_temp_files().is_empty() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let before = matching_temp_files();
    assert!(!before.is_empty(), "expected socket/settings temp files to exist while the session is live (prefix {prefix:?})");

    cancel_tx.send(()).unwrap();
    handle.join().unwrap();

    let after = matching_temp_files();
    assert!(after.is_empty(), "temp files should be cleaned up when the future is cancelled, not just on a normal return: {after:?}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// End-to-end proof that `run` actually works against the real `claude`
/// CLI: spawns a real session in a scratch project, asks it to edit two
/// files, approves the permission request for one and denies the other,
/// and confirms the right file changed and the other didn't.
///
/// **Not run automatically** — this makes real, billed Anthropic API
/// calls and requires `claude` to be installed and authenticated on the
/// machine running it. Run by hand with:
/// `cargo test -p devscribe-core claude_agent -- --ignored --nocapture`
#[test]
#[ignore = "spawns the real, billed `claude` CLI — run manually"]
fn approves_one_edit_and_denies_another_end_to_end() {
    let binary = PathBuf::from("claude");
    let devscribe_exe = devscribe_exe();
    assert!(devscribe_exe.exists(), "build devscribe first: cargo build -p devscribe");

    let dir = std::env::temp_dir().join(format!("devscribe-claude-agent-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("allowed.txt"), "original\n").unwrap();
    std::fs::write(dir.join("secret.txt"), "original\n").unwrap();

    let (tx, mut rx) = fmpsc::channel(64);
    let root = dir.clone();
    std::thread::spawn(move || {
        futures::executor::block_on(run(root, binary, devscribe_exe, SessionOptions { session_id: new_session_id(), resume: false, mode: PermissionMode::Manual, allow_bash: false }, tx));
    });

    let ready_events = recv_within(&mut rx, Duration::from_secs(20));
    let mut sender = find_command_sender(&ready_events).expect("expected ClaudeEvent::Ready");

    futures::executor::block_on(sender.send(ClaudeCommand::SendPrompt(
        "In this directory, append the line 'edited' to allowed.txt, and separately try to append \
         the line 'edited' to secret.txt. Do these as two separate Edit tool calls. Report one \
         short sentence per file about what happened."
            .to_string(),
    )))
    .unwrap();

    // Claude Code executes tool calls (and so fires PreToolUse hooks)
    // sequentially, even though the assistant message that requests both
    // edits arrives as one message with two `tool_use` blocks: the second
    // Edit's hook doesn't even run until the first one has been answered.
    // So permission requests must be answered as they arrive, not
    // collected up front — collecting both first would deadlock, since
    // the second one never shows up until the first is resolved.
    let mut all_events = Vec::new();
    let mut answered_allowed = false;
    let mut answered_secret = false;
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while std::time::Instant::now() < deadline && !(answered_allowed && answered_secret) {
        let batch = recv_within(&mut rx, Duration::from_millis(500));
        if !answered_allowed {
            if let Some(id) = find_permission_request(&batch, "allowed.txt") {
                let id = id.to_string();
                futures::executor::block_on(sender.send(ClaudeCommand::RespondPermission {
                    id,
                    approve: true,
                    reason: None,
                }))
                .unwrap();
                answered_allowed = true;
            }
        }
        if !answered_secret {
            if let Some(id) = find_permission_request(&batch, "secret.txt") {
                let id = id.to_string();
                futures::executor::block_on(sender.send(ClaudeCommand::RespondPermission {
                    id,
                    approve: false,
                    reason: Some("not allowed by policy".to_string()),
                }))
                .unwrap();
                answered_secret = true;
            }
        }
        all_events.extend(batch);
    }
    eprintln!("ALL EVENTS SO FAR:\n{all_events:#?}");
    assert!(answered_allowed, "expected a PermissionRequest for allowed.txt");
    assert!(answered_secret, "expected a PermissionRequest for secret.txt");

    // Let the turn actually finish landing on disk.
    all_events.extend(recv_within(&mut rx, Duration::from_secs(20)));
    std::thread::sleep(Duration::from_secs(2));

    let allowed_contents = std::fs::read_to_string(dir.join("allowed.txt")).unwrap();
    let secret_contents = std::fs::read_to_string(dir.join("secret.txt")).unwrap();
    assert!(allowed_contents.contains("edited"), "allowed.txt should have been edited, got: {allowed_contents:?}");
    assert_eq!(secret_contents, "original\n", "secret.txt should NOT have been edited, got: {secret_contents:?}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// End-to-end proof that the *session* mechanism `chat_worker` relies on
/// actually works against the real CLI, not just at the level individual
/// pieces were spiked at: starts a real session, tells it a fact, closes
/// it (drops `run`'s future, exactly like the panel closing), confirms
/// `session_exists`/`list_sessions`/`load_session_history` now see it
/// correctly from the real transcript `claude` itself wrote, and confirms
/// a second `run` with `resume: true` on that same id both remembers the
/// fact live *and* doesn't duplicate the replayed history.
///
/// **Not run automatically** — real, billed API calls. Run by hand with:
/// `cargo test -p devscribe-core claude_agent::tests::resumes -- --ignored --nocapture`
#[test]
#[ignore = "spawns the real, billed `claude` CLI — run manually"]
fn resumes_a_session_with_real_memory_and_replayed_history() {
    use futures::FutureExt;

    let binary = PathBuf::from("claude");
    let dir = std::env::temp_dir().join(format!("devscribe-claude-agent-resume-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let session_id = new_session_id();

    // --- First run: a fresh session, told a fact, then closed. ---
    assert!(!session_exists(&dir, &session_id), "sanity: shouldn't exist before the first run");
    let (tx1, mut rx1) = fmpsc::channel(64);
    let (cancel_tx, cancel_rx) = futures::channel::oneshot::channel::<()>();
    let root1 = dir.clone();
    let devscribe_exe1 = devscribe_exe();
    let session_id1 = session_id.clone();
    let handle = std::thread::spawn(move || {
        futures::executor::block_on(async {
            futures::select! {
                _ = run(root1, binary, devscribe_exe1, SessionOptions { session_id: session_id1, resume: false, mode: PermissionMode::Manual, allow_bash: false }, tx1).fuse() => {},
                _ = cancel_rx.fuse() => {},
            }
        });
    });
    let ready = recv_within(&mut rx1, Duration::from_secs(20));
    let mut sender = find_command_sender(&ready).expect("expected ClaudeEvent::Ready");
    futures::executor::block_on(sender.send(ClaudeCommand::SendPrompt(
        "Remember this exact phrase for later: pineapple-forty-two. Reply with just: noted.".to_string(),
    )))
    .unwrap();
    let turn1 = recv_within(&mut rx1, Duration::from_secs(30));
    assert!(
        turn1.iter().any(|e| matches!(e, ClaudeEvent::AssistantText(t) if t.contains("noted"))),
        "expected the first turn to actually complete before closing: {turn1:#?}"
    );
    // Close it — same shape as the panel closing (subscription cancelled
    // mid-await), not a graceful `SendPrompt`-less shutdown.
    cancel_tx.send(()).unwrap();
    handle.join().unwrap();

    // --- Between runs: confirm the transcript is discoverable for real. ---
    assert!(session_exists(&dir, &session_id), "claude should have written a transcript for this id");
    let sessions = list_sessions(&dir);
    assert!(sessions.iter().any(|s| s.id == session_id), "list_sessions should find it: {sessions:#?}");
    let history = load_session_history(&dir, &session_id);
    assert!(
        history.events.iter().any(|e| matches!(e, ClaudeEvent::OperatorText(t) if t.contains("pineapple-forty-two"))),
        "expected the first turn's prompt to be in the replayed history: {:#?}",
        history.events
    );

    // --- Second run: resume it, and confirm both real memory and that
    // the live stream doesn't *also* re-emit the first turn (only new
    // turns go out live — replay is `chat_worker`'s own job via
    // `load_session_history`, called separately, not something `run`
    // duplicates internally).
    let (tx2, mut rx2) = fmpsc::channel(64);
    let root2 = dir.clone();
    let devscribe_exe2 = devscribe_exe();
    let session_id2 = session_id.clone();
    std::thread::spawn(move || {
        futures::executor::block_on(run(root2, PathBuf::from("claude"), devscribe_exe2, SessionOptions { session_id: session_id2, resume: true, mode: PermissionMode::Manual, allow_bash: false }, tx2));
    });
    let ready2 = recv_within(&mut rx2, Duration::from_secs(20));
    let mut sender2 = find_command_sender(&ready2).expect("expected ClaudeEvent::Ready on resume");
    assert!(
        !ready2.iter().any(|e| matches!(e, ClaudeEvent::OperatorText(_))),
        "run() itself must not replay history — that's chat_worker's job, calling load_session_history separately: {ready2:#?}"
    );
    futures::executor::block_on(sender2.send(ClaudeCommand::SendPrompt(
        "What was the exact phrase I asked you to remember? Reply with just the phrase.".to_string(),
    )))
    .unwrap();
    let turn2 = recv_within(&mut rx2, Duration::from_secs(30));
    assert!(
        turn2.iter().any(|e| matches!(e, ClaudeEvent::AssistantText(t) if t.contains("pineapple-forty-two"))),
        "expected the resumed session to actually remember the fact from turn 1: {turn2:#?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// End-to-end proof that `PermissionMode::Plan` and `PermissionMode::Auto`
/// actually behave as documented against the real CLI — not just that
/// `generate_settings`/the `--permission-mode` value compile, but that
/// `plan` genuinely can't edit and `bypassPermissions` genuinely never
/// raises a `PermissionRequest` (i.e. `generate_settings`'s empty-hook
/// `json!({})` for non-`Manual` modes is a valid `--settings` payload the
/// CLI actually accepts, not just plausible-looking JSON).
///
/// **Not run automatically** — real, billed API calls. Run by hand with:
/// `cargo test -p devscribe-core claude_agent::tests::plan_and_auto -- --ignored --nocapture`
#[test]
#[ignore = "spawns the real, billed `claude` CLI — run manually"]
fn plan_and_auto_modes_behave_as_documented() {
    let dir = std::env::temp_dir().join(format!("devscribe-claude-agent-modes-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("note.txt"), "original\n").unwrap();

    // --- Plan mode: asked to edit, must not actually touch the file. ---
    let (tx, mut rx) = fmpsc::channel(64);
    let root = dir.clone();
    std::thread::spawn(move || {
        futures::executor::block_on(run(
            root,
            PathBuf::from("claude"),
            devscribe_exe(),
            SessionOptions { session_id: new_session_id(), resume: false, mode: PermissionMode::Plan, allow_bash: false },
            tx,
        ));
    });
    let ready = recv_within(&mut rx, Duration::from_secs(20));
    let mut sender = find_command_sender(&ready).expect("expected ClaudeEvent::Ready");
    futures::executor::block_on(
        sender.send(ClaudeCommand::SendPrompt("Append the line 'edited' to note.txt.".to_string())),
    )
    .unwrap();
    let turn = recv_within(&mut rx, Duration::from_secs(30));
    assert!(
        !turn.iter().any(|e| matches!(e, ClaudeEvent::PermissionRequest { .. })),
        "plan mode should never even attempt a gated edit: {turn:#?}"
    );
    let contents = std::fs::read_to_string(dir.join("note.txt")).unwrap();
    assert_eq!(contents, "original\n", "plan mode must not have actually edited the file: {contents:?}");

    // --- Auto mode: same request, must edit with *no* permission card at all. ---
    let (tx2, mut rx2) = fmpsc::channel(64);
    let root2 = dir.clone();
    std::thread::spawn(move || {
        futures::executor::block_on(run(
            root2,
            PathBuf::from("claude"),
            devscribe_exe(),
            SessionOptions { session_id: new_session_id(), resume: false, mode: PermissionMode::Auto, allow_bash: false },
            tx2,
        ));
    });
    let ready2 = recv_within(&mut rx2, Duration::from_secs(20));
    let mut sender2 = find_command_sender(&ready2).expect("expected ClaudeEvent::Ready");
    futures::executor::block_on(
        sender2.send(ClaudeCommand::SendPrompt("Append the line 'edited' to note.txt.".to_string())),
    )
    .unwrap();
    let turn2 = recv_within(&mut rx2, Duration::from_secs(30));
    assert!(
        !turn2.iter().any(|e| matches!(e, ClaudeEvent::PermissionRequest { .. })),
        "auto mode should never raise a permission card: {turn2:#?}"
    );
    let contents2 = std::fs::read_to_string(dir.join("note.txt")).unwrap();
    assert!(contents2.contains("edited"), "auto mode should have actually edited the file: {contents2:?}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn generate_settings_gates_bash_with_the_same_rule_as_edit_write() {
    let exe = PathBuf::from("/usr/bin/devscribe");
    let socket = PathBuf::from("/tmp/devscribe-test.sock");

    let manual_no_bash = generate_settings(&exe, &socket, PermissionMode::Manual, false);
    assert_eq!(manual_no_bash["hooks"]["PreToolUse"][0]["matcher"], "Edit|Write");

    let manual_with_bash = generate_settings(&exe, &socket, PermissionMode::Manual, true);
    assert_eq!(manual_with_bash["hooks"]["PreToolUse"][0]["matcher"], "Edit|Write|Bash");

    // Non-gating modes stay hook-free regardless of `allow_bash` — the same
    // `json!({})` `generate_settings` has always returned for them.
    for mode in [PermissionMode::EditAuto, PermissionMode::Plan, PermissionMode::Auto] {
        assert_eq!(generate_settings(&exe, &socket, mode, false), json!({}));
        assert_eq!(generate_settings(&exe, &socket, mode, true), json!({}));
    }
}
