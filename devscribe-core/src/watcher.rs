//! A general-purpose filesystem watcher, independent of any one feature.
//! Currently consumed by `devscribe`'s sidebar tree / git-changed-files
//! refresh; the AI Chat Assist panel is expected to consume the same
//! stream later, since an edit Claude Code makes on disk is indistinguishable
//! from any other external change.
//!
//! Built on `notify`'s recommended (OS-native, e.g. inotify on Linux)
//! backend, which delivers events from its own background thread via an
//! `EventHandler` callback. That thread hands raw events to a second,
//! dedicated thread (spawned here) that debounces/coalesces bursts using a
//! plain blocking `std::sync::mpsc::Receiver::recv_timeout` — no async
//! runtime timer required — before forwarding a batch into the async
//! world via `futures::channel::mpsc::UnboundedSender::unbounded_send`,
//! which is safe to call from any thread.
use std::path::{Path, PathBuf};
use std::sync::mpsc as std_mpsc;
use std::time::Duration;

use futures::channel::mpsc;
use futures::{SinkExt, StreamExt};
use notify::event::ModifyKind;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

/// Directories never worth watching (or walking — see `devscribe`'s
/// `fs_tree::SKIP_DIRS`, which mirrors this list for the sidebar walk).
pub const SKIP_DIRS: &[&str] = &[".git", "target", "node_modules", ".idea", ".vscode", ".claude"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEvent {
    Changed(PathBuf),
    Created(PathBuf),
    Removed(PathBuf),
}

/// How long to keep coalescing events after the first one in a burst before
/// forwarding a batch — long enough to fold a save's multiple raw OS events
/// (truncate + write + metadata touch) into one, short enough to still feel
/// immediate.
const DEBOUNCE: Duration = Duration::from_millis(200);

fn is_ignored(path: &Path) -> bool {
    path.components().any(|c| {
        c.as_os_str()
            .to_str()
            .map(|s| SKIP_DIRS.contains(&s))
            .unwrap_or(false)
    })
}

/// Watches `root` and forwards debounced, filtered batches of events into
/// `output` until either the underlying watcher errors out or `output`'s
/// receiver is dropped (project switch — see `devscribe`'s `file_watcher`
/// subscription, keyed on the project root so switching projects respawns
/// this). Never blocks the caller's async task: the actual OS-thread
/// blocking (notify's own thread, plus the debounce thread below) is
/// entirely off to the side.
pub async fn run(root: PathBuf, mut output: mpsc::Sender<Vec<WatchEvent>>) {
    let (raw_tx, raw_rx) = std_mpsc::channel::<notify::Result<Event>>();

    let mut watcher = match RecommendedWatcher::new(raw_tx, notify::Config::default()) {
        Ok(w) => w,
        Err(_) => return,
    };
    if watcher.watch(&root, RecursiveMode::Recursive).is_err() {
        return;
    }

    let (batch_tx, mut batch_rx) = mpsc::unbounded::<Vec<WatchEvent>>();
    std::thread::spawn(move || {
        // Keeping `watcher` alive for this thread's lifetime is the point —
        // dropping it unregisters the underlying inotify watches.
        let _watcher = watcher;
        let mut pending: Vec<WatchEvent> = Vec::new();
        loop {
            let recv = if pending.is_empty() {
                raw_rx.recv().map_err(|_| ())
            } else {
                raw_rx.recv_timeout(DEBOUNCE).map_err(|_| ())
            };
            match recv {
                Ok(Ok(event)) => {
                    // inotify's default mask (what `notify` registers) includes
                    // `IN_OPEN`/`IN_CLOSE_*`/`IN_ATTRIB`, so a plain *read* of a
                    // watched file — `Document::open`ing a tab, the git-diff
                    // code's `std::fs::read_to_string`, an LSP server reading
                    // the file — fires an event too, surfaced here as
                    // `EventKind::Access(_)`/`Modify(Metadata(_))`. Only a real
                    // content change is worth reacting to; anything else used
                    // to be misclassified as `Changed`, which reloaded (and so
                    // reset the cursor of) the very file someone just opened
                    // or DevScribe itself just read for the diff view.
                    let is_relevant = matches!(
                        event.kind,
                        EventKind::Create(_)
                            | EventKind::Remove(_)
                            | EventKind::Modify(ModifyKind::Data(_) | ModifyKind::Name(_))
                    );
                    if is_relevant {
                        for path in event.paths {
                            if is_ignored(&path) {
                                continue;
                            }
                            pending.push(match event.kind {
                                EventKind::Create(_) => WatchEvent::Created(path),
                                EventKind::Remove(_) => WatchEvent::Removed(path),
                                _ => WatchEvent::Changed(path),
                            });
                        }
                    }
                }
                Ok(Err(_)) => {}
                Err(()) if !pending.is_empty() => {
                    if batch_tx.unbounded_send(std::mem::take(&mut pending)).is_err() {
                        return; // nothing reads this anymore — stream was dropped
                    }
                }
                Err(()) => return, // `raw_rx` disconnected: the watcher itself is gone
            }
        }
    });

    while let Some(batch) = batch_rx.next().await {
        if output.send(batch).await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
#[path = "tests/watcher.rs"]
mod tests;
