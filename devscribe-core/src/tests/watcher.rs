use super::*;
use futures::channel::mpsc;
use std::time::Duration;

/// Polls a `futures` mpsc receiver with plain sleeps rather than an
/// executor — the watcher itself runs its debounce/coalesce loop on real
/// OS threads (see `run`'s doc comment), so a real wall-clock wait is
/// exactly what's needed here rather than anything async-runtime-specific.
fn recv_within(rx: &mut mpsc::Receiver<Vec<WatchEvent>>, timeout: Duration) -> Option<Vec<WatchEvent>> {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        match rx.try_recv() {
            Ok(batch) => return Some(batch),
            Err(_) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    None
}

#[test]
fn detects_create_and_change_while_filtering_ignored_dirs() {
    let dir = std::env::temp_dir().join(format!("devscribe-watcher-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("target")).unwrap();

    let (tx, mut rx) = mpsc::channel(32);
    let root = dir.clone();
    std::thread::spawn(move || {
        futures::executor::block_on(run(root, tx));
    });

    // Give the watcher a moment to actually register with the OS backend
    // before mutating anything — otherwise the write can race the watch
    // registration and get missed.
    std::thread::sleep(Duration::from_millis(200));

    let new_file = dir.join("new.txt");
    std::fs::write(&new_file, "hi").unwrap();
    std::fs::write(dir.join("target").join("ignored.txt"), "should not surface").unwrap();

    let batch = recv_within(&mut rx, Duration::from_secs(5)).expect("expected a batch of events");
    assert!(
        batch.iter().any(|e| matches!(e, WatchEvent::Created(p) | WatchEvent::Changed(p) if p == &new_file)),
        "expected an event for {new_file:?}, got {batch:?}"
    );
    assert!(
        !batch.iter().any(|e| match e {
            WatchEvent::Created(p) | WatchEvent::Changed(p) | WatchEvent::Removed(p) => {
                p.components().any(|c| c.as_os_str() == "target")
            }
        }),
        "an event under target/ leaked through the ignore filter: {batch:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Regression test for a real bug: inotify's default watch mask fires on a
/// plain *read* too (`IN_OPEN`/`IN_CLOSE_NOWRITE`/`IN_ATTRIB`), not just a
/// write. `run` used to classify every event kind it didn't specifically
/// recognize as `Changed`, so simply *opening* a file for reading (exactly
/// what `Document::open`, the diff view, and LSP file reads all do)
/// generated a false "changed on disk" event — which the app used to react
/// to by reloading the buffer and resetting its cursor to the top, purely
/// because something had *read* the file. Only a real write should surface
/// here.
#[test]
fn a_plain_read_does_not_produce_an_event() {
    let dir = std::env::temp_dir().join(format!("devscribe-watcher-read-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("existing.txt");
    std::fs::write(&file, "original").unwrap();

    let (tx, mut rx) = mpsc::channel(32);
    let root = dir.clone();
    std::thread::spawn(move || {
        futures::executor::block_on(run(root, tx));
    });
    std::thread::sleep(Duration::from_millis(200));

    // A plain read (open + close, no write) of an already-watched file —
    // this alone must not produce a batch.
    let _ = std::fs::read_to_string(&file).unwrap();
    assert!(
        recv_within(&mut rx, Duration::from_millis(600)).is_none(),
        "a plain read of an existing file should not surface as a WatchEvent"
    );

    // A genuine write to the same file still must be reported.
    std::fs::write(&file, "changed").unwrap();
    let batch = recv_within(&mut rx, Duration::from_secs(5)).expect("expected an event for the real write");
    assert!(
        batch.iter().any(|e| matches!(e, WatchEvent::Changed(p) if p == &file)),
        "expected a Changed event for {file:?}, got {batch:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn is_ignored_matches_skip_dirs_anywhere_in_the_path() {
    assert!(is_ignored(Path::new("/repo/target/debug/foo")));
    assert!(is_ignored(Path::new("/repo/node_modules/pkg/index.js")));
    assert!(!is_ignored(Path::new("/repo/src/main.rs")));
}
