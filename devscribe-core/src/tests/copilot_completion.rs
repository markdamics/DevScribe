use super::*;
use futures::channel::mpsc as fmpsc;
use serde_json::json;
use std::time::Duration;

fn recv_within(rx: &mut fmpsc::Receiver<CopilotCompletionEvent>, timeout: Duration) -> Vec<CopilotCompletionEvent> {
    let deadline = std::time::Instant::now() + timeout;
    let mut events = Vec::new();
    while std::time::Instant::now() < deadline {
        match rx.try_recv() {
            Ok(event) => events.push(event),
            Err(err) if err.is_closed() => break,
            Err(_) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    events
}

/// End-to-end proof that `run` reaches a real `copilot-language-server`,
/// opens a document, and gets a real (possibly-empty, and that's fine —
/// see below) answer out of `textDocument/inlineCompletion` — the one part
/// of this module that's actually documented (unlike `copilot_agent`'s
/// `conversation/*`), but still worth confirming against the real server:
/// the wire format adds two non-standard fields (`textDocument.version`,
/// `formattingOptions`) on top of the documented shape, and `workspaceFolder`
/// being a `file://` URI rather than a bare path was a real bug caught this
/// same way in `copilot_agent`'s own turn-request params.
///
/// Doesn't assert the suggestion is non-empty: Copilot legitimately has
/// nothing to offer for arbitrary content sometimes, and that's a normal
/// `item: None` response, not a failure — only that a `Suggestion` event
/// comes back at all, rather than `Unavailable`.
///
/// **Not run automatically** — requires `copilot-language-server`
/// (`npm install -g @github/copilot-language-server`) on PATH and an
/// already-signed-in account (see this module's own doc comment on why it
/// doesn't run the device flow itself). Run by hand with:
/// `cargo test -p devscribe-core copilot_completion -- --ignored --nocapture`
#[test]
#[ignore = "spawns the real copilot-language-server — run manually, requires an already-signed-in account"]
fn requests_a_real_suggestion_end_to_end() {
    use futures::FutureExt;

    let binary = PathBuf::from("copilot-language-server");
    let dir = std::env::temp_dir().join(format!("devscribe-copilot-completion-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let (tx, mut rx) = fmpsc::channel::<CopilotCompletionEvent>(32);
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

    let setup_events = recv_within(&mut rx, Duration::from_secs(15));
    let sender = setup_events.iter().find_map(|e| match e {
        CopilotCompletionEvent::Ready(sender) => Some(sender.clone()),
        _ => None,
    });
    let Some(mut sender) = sender else {
        let _ = cancel_tx.send(());
        handle.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        eprintln!("skipping: not signed in, or the handshake failed — got {setup_events:?}");
        return;
    };

    let uri = Url::from_file_path(dir.join("sample.rs")).unwrap();
    let text = "fn add(a: i32, b: i32) -> i32 {\n    \n}\n".to_string();
    sender.try_send(CopilotCompletionCommand::DidOpen { uri: uri.clone(), text }).unwrap();
    // Line 1, col 4 — the blank line inside the function body.
    sender.try_send(CopilotCompletionCommand::Suggest { uri: uri.clone(), line: 1, character: 4 }).unwrap();

    let events = recv_within(&mut rx, Duration::from_secs(20));
    let _ = cancel_tx.send(());
    handle.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);

    for event in &events {
        if let CopilotCompletionEvent::Unavailable(reason) = event {
            panic!("worker reported Unavailable after Ready: {reason}");
        }
    }
    let suggestion = events.iter().find_map(|e| match e {
        CopilotCompletionEvent::Suggestion { uri: u, line, character, item } if *u == uri && *line == 1 && *character == 4 => Some(item),
        _ => None,
    });
    assert!(suggestion.is_some(), "expected a Suggestion event (item may legitimately be None) for the request sent, got: {events:?}");
}

#[test]
fn first_item_reads_the_first_item_of_a_non_empty_list() {
    let result = json!({"items": [{"insertText": "a"}, {"insertText": "b"}]});
    assert_eq!(first_item(&result), Some(json!({"insertText": "a"})));
}

#[test]
fn first_item_is_none_for_an_empty_list() {
    assert_eq!(first_item(&json!({"items": []})), None);
}

#[test]
fn first_item_is_none_when_the_items_field_is_missing_entirely() {
    // The server having nothing to offer at a position is a normal,
    // common response — not a shape this should panic or error on.
    assert_eq!(first_item(&json!({})), None);
}

#[test]
fn first_item_is_none_for_the_wrong_shape() {
    assert_eq!(first_item(&json!({"items": "not an array"})), None);
    assert_eq!(first_item(&json!(null)), None);
}
