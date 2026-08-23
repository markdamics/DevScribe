//! Minimal, always-on error logging to a file. Added while chasing a
//! recurring "the app hangs/crashes" report; kept afterward, scoped down
//! to error conditions only — the verbose per-file/per-search breadcrumb
//! trail that was here during that investigation did its job and was
//! removed once the root cause was found (see
//! `docs/differences-and-roadmap.md`'s search bug-fix writeup).
//!
//! [`init`] installs a process-wide panic hook that writes every panic —
//! on *any* thread, including the background search thread from
//! `state::start_search` — to the log file, in addition to (not instead
//! of) the default stderr hook a terminal launch would already show; the
//! app's usually launched from a desktop icon, not a terminal, so
//! `eprintln!` alone goes nowhere the user can find it. [`error`] is for
//! any other genuine failure worth a permanent record.
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

static LOG_FILE: OnceLock<Mutex<std::fs::File>> = OnceLock::new();
static START: OnceLock<Instant> = OnceLock::new();

/// `<project root>/.devscribe.log` — `CARGO_MANIFEST_DIR` (this crate's own
/// directory, baked in at *compile* time) rather than the *runtime* working
/// directory, so the log lands in the same place every time regardless of
/// how the binary gets launched. That matters here specifically: the
/// desktop-launcher `.desktop` file this project set up has no `Path=` key,
/// so a click-to-launch's working directory defaults to `$HOME`, not this
/// repo — a `current_dir()`-based path would silently land there instead
/// every time it's launched that way.
fn log_path() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let project_root = manifest_dir.parent().unwrap_or(manifest_dir);
    project_root.join(".devscribe.log")
}

/// Truncates any previous run's log (this run's trail is what matters for
/// chasing a fresh repro, not an ever-growing history) and installs the
/// panic hook. Call once, at the very start of `main`. Deliberately
/// silent on success — an empty log file is the expected, healthy state;
/// only [`error`] (including the panic hook this installs) writes to it.
pub fn init() {
    let _ = START.set(Instant::now());
    let path = log_path();
    if let Ok(file) = OpenOptions::new().create(true).write(true).truncate(true).open(&path) {
        let _ = LOG_FILE.set(Mutex::new(file));
    }

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        error(format!("PANIC on thread {:?}: {info}", std::thread::current().name().unwrap_or("<unnamed>")));
        default_hook(info);
    }));
}

/// Appends one timestamped (seconds since [`init`]) line to the log file,
/// and to stderr for a terminal launch. Safe to call from any thread. Only
/// for genuine error conditions — a panic (via the hook [`init`] installs),
/// a background task failing, or similar — not routine/expected events.
pub fn error(message: impl std::fmt::Display) {
    let elapsed = START.get().map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0);
    let line = format!("[+{elapsed:9.3}s] {message}");
    eprintln!("{line}");
    if let Some(mutex) = LOG_FILE.get()
        && let Ok(mut file) = mutex.lock()
    {
        let _ = writeln!(file, "{line}");
        let _ = file.flush();
    }
}
