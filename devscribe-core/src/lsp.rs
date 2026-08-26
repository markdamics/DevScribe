//! A minimal `async-lsp` client: spawns one language server per language,
//! forwards `didOpen`/`didChange` notifications to it, streams
//! `publishDiagnostics` back out, and handles `textDocument/completion`
//! requests. Full-text sync only (no incremental `TextDocumentContentChangeEvent`
//! ranges) — same "reparse the whole thing" simplification as `syntax`, and
//! always spec-legal regardless of what the server advertises.
use std::collections::HashMap;
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::process::Stdio;

use async_lsp::lsp_types::{
    ClientCapabilities, CompletionClientCapabilities, CompletionParams, CompletionResponse,
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    InitializeParams, InitializedParams, LogMessageParams, PartialResultParams, Position,
    ProgressParams, PublishDiagnosticsParams, ShowMessageParams, TextDocumentClientCapabilities,
    TextDocumentContentChangeEvent, TextDocumentIdentifier, TextDocumentItem,
    TextDocumentPositionParams, VersionedTextDocumentIdentifier, WorkDoneProgressParams,
    WorkspaceFolder,
};
use async_lsp::router::Router;
use async_lsp::{LanguageClient, LanguageServer, MainLoop, ResponseError};
use futures::channel::mpsc;
use futures::{FutureExt, SinkExt, StreamExt};

pub use async_lsp::lsp_types::{CompletionItem, Diagnostic, DiagnosticSeverity, Url};

/// A language DevScribe knows how to talk to a language server for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LspLanguage {
    Rust,
    Java,
    Python,
    JavaScript,
    TypeScript,
    Cpp,
}

impl LspLanguage {
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "rs" => Some(Self::Rust),
            "java" => Some(Self::Java),
            "py" | "pyi" => Some(Self::Python),
            "js" | "mjs" | "cjs" => Some(Self::JavaScript),
            "ts" | "mts" | "cts" | "tsx" => Some(Self::TypeScript),
            "cpp" | "cc" | "cxx" | "c" | "h" | "hpp" | "hxx" => Some(Self::Cpp),
            _ => None,
        }
    }

    pub fn command(self) -> &'static str {
        match self {
            Self::Rust => "rust-analyzer",
            Self::Java => "jdtls",
            Self::Python => "pyright-langserver",
            Self::JavaScript | Self::TypeScript => "typescript-language-server",
            Self::Cpp => "clangd",
        }
    }

    /// Extra args passed after the binary name (e.g. `--stdio` for servers
    /// that require it).
    pub fn args(self) -> &'static [&'static str] {
        match self {
            Self::Python => &["--stdio"],
            Self::JavaScript | Self::TypeScript => &["--stdio"],
            _ => &[],
        }
    }

    pub fn language_id(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Java => "java",
            Self::Python => "python",
            Self::JavaScript => "javascript",
            Self::TypeScript => "typescript",
            Self::Cpp => "cpp",
        }
    }
}

/// A command the app sends into a running worker (see `run`).
#[derive(Debug, Clone)]
pub enum LspCommand {
    DidOpen { uri: Url, text: String },
    DidChange { uri: Url, text: String },
    DidClose { uri: Url },
    Completion { uri: Url, line: u32, character: u32 },
}

/// An event a running worker reports back to the app.
#[derive(Debug, Clone)]
pub enum LspEvent {
    /// The server is initialized; `sender` accepts `LspCommand`s.
    Ready(mpsc::Sender<LspCommand>),
    Diagnostics {
        uri: Url,
        diagnostics: Vec<Diagnostic>,
    },
    Completions {
        uri: Url,
        line: u32,
        character: u32,
        items: Vec<CompletionItem>,
    },
    /// Binary not found anywhere — the app should auto-install it.
    NeedsInstall,
    /// The server binary couldn't be spawned, or the connection died.
    Unavailable(String),
}

struct ClientState {
    diagnostics_tx: mpsc::UnboundedSender<PublishDiagnosticsParams>,
}

impl LanguageClient for ClientState {
    type Error = ResponseError;
    type NotifyResult = ControlFlow<async_lsp::Result<()>>;

    fn publish_diagnostics(&mut self, params: PublishDiagnosticsParams) -> Self::NotifyResult {
        let _ = self.diagnostics_tx.unbounded_send(params);
        ControlFlow::Continue(())
    }

    fn progress(&mut self, _params: ProgressParams) -> Self::NotifyResult {
        ControlFlow::Continue(())
    }

    fn show_message(&mut self, _params: ShowMessageParams) -> Self::NotifyResult {
        ControlFlow::Continue(())
    }

    fn log_message(&mut self, _params: LogMessageParams) -> Self::NotifyResult {
        ControlFlow::Continue(())
    }
}

/// Spawns a language server for `root` and drives it until the process dies
/// or `output` is dropped. `binary` is the resolved absolute path (or bare
/// name) of the server executable — callers (see `devscribe`'s `lsp_worker`)
/// are responsible for locating it first and emitting `NeedsInstall` if it
/// isn't found.
pub async fn run(root: PathBuf, language: LspLanguage, binary: PathBuf, mut output: mpsc::Sender<LspEvent>) {
    // Redirect stderr to a per-language log file so startup errors are
    // diagnosable. Path: ~/.local/share/devscribe/logs/<lang>.stderr.log
    let log_path = dirs::data_dir()
        .unwrap_or_else(|| std::env::temp_dir())
        .join("devscribe")
        .join("logs")
        .join(format!("{}.stderr.log", language.language_id()));
    let _ = std::fs::create_dir_all(log_path.parent().unwrap());
    let stderr_cfg = std::fs::File::create(&log_path)
        .map(Stdio::from)
        .unwrap_or_else(|_| Stdio::null());

    // Build the command. Managed jdtls installs pass the install *directory*
    // as `binary`; we launch via `java -jar` directly instead of the Python
    // wrapper, matching how VS Code / Zed integrate jdtls.
    let mut cmd = if language == LspLanguage::Java && binary.is_dir() {
        match jdtls_command(&binary, &root) {
            Some(cmd) => cmd,
            None => {
                let _ = output
                    .send(LspEvent::Unavailable(format!(
                        "jdtls: java not found on PATH or launcher JAR missing \
                         — see {}",
                        log_path.display()
                    )))
                    .await;
                return;
            }
        }
    } else {
        let mut cmd = async_process::Command::new(&binary);
        for arg in language.args() {
            cmd.arg(arg);
        }
        // For managed JS/TS installs, set NODE_PATH so Node can find the
        // typescript peer package. npm -g --prefix puts modules in either
        // <prefix>/lib/node_modules/ (npm ≥7) or <prefix>/node_modules/.
        if matches!(language, LspLanguage::TypeScript | LspLanguage::JavaScript)
            && binary.is_absolute()
        {
            if let Some(prefix) = binary.parent().and_then(|p| p.parent()) {
                for modules_dir in &[
                    prefix.join("lib").join("node_modules"),
                    prefix.join("node_modules"),
                ] {
                    if modules_dir.join("typescript").exists() {
                        cmd.env("NODE_PATH", modules_dir);
                        break;
                    }
                }
            }
        }
        cmd
    };

    cmd.current_dir(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(stderr_cfg)
        .kill_on_drop(true);

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => {
            let _ = output
                .send(LspEvent::Unavailable(format!(
                    "{} not available: {err}",
                    language.command()
                )))
                .await;
            return;
        }
    };
    let child_stdout = child.stdout.take().expect("piped stdout");
    let child_stdin = child.stdin.take().expect("piped stdin");

    let (diagnostics_tx, mut diagnostics_rx) = mpsc::unbounded();
    let (mainloop, mut server) = MainLoop::new_client(|_server| {
        let mut router = Router::from_language_client(ClientState { diagnostics_tx });
        // Ignore any server notifications we don't explicitly handle (e.g.
        // jdtls sends language/status, language/actionableNotification, etc.).
        router.unhandled_notification(|_, _| ControlFlow::Continue(()));
        router
    });

    // async-lsp requires the MainLoop and its `server` handle to live in the
    // same task — splitting them across tokio::spawn causes a "Sender is alive"
    // panic when iced drops the subscription future. We keep both here and use
    // futures::select! to drive the mainloop concurrently with server calls.
    let mainloop_fut = mainloop.run_buffered(child_stdout, child_stdin).fuse();
    futures::pin_mut!(mainloop_fut);

    let Some(root_uri) = Url::from_file_path(&root).ok() else {
        let _ = output
            .send(LspEvent::Unavailable("project root has no file:// URI".into()))
            .await;
        return;
    };

    // Poll mainloop alongside initialize so it can process server messages
    // during the handshake. root_uri is deprecated in favour of
    // workspaceFolders but jdtls still reads it to locate the project.
    let init = {
        let init_fut = server
            .initialize(InitializeParams {
                root_uri: Some(root_uri.clone()),
                workspace_folders: Some(vec![WorkspaceFolder {
                    uri: root_uri,
                    name: "root".into(),
                }]),
                capabilities: ClientCapabilities {
                    text_document: Some(TextDocumentClientCapabilities {
                        completion: Some(CompletionClientCapabilities {
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                ..InitializeParams::default()
            })
            .fuse();
        futures::pin_mut!(init_fut);
        loop {
            futures::select! {
                r = init_fut => break r,
                r = mainloop_fut => {
                    if let Err(err) = r {
                        let line = format!("\n[lsp-mainloop] error: {err:?}\n");
                        let _ = std::fs::OpenOptions::new()
                            .append(true)
                            .open(&log_path)
                            .and_then(|mut f| { use std::io::Write; f.write_all(line.as_bytes()) });
                    }
                    let _ = output
                        .send(LspEvent::Unavailable(format!(
                            "initialize failed: service stopped — see {}",
                            log_path.display()
                        )))
                        .await;
                    return;
                }
            }
        }
    };
    if let Err(err) = init {
        let _ = output
            .send(LspEvent::Unavailable(format!(
                "initialize failed: {err} — see {}",
                log_path.display()
            )))
            .await;
        return;
    }
    if server.initialized(InitializedParams {}).is_err() {
        let _ = output
            .send(LspEvent::Unavailable(format!(
                "server stopped during startup — see {}",
                log_path.display()
            )))
            .await;
        return;
    }

    let (cmd_tx, mut cmd_rx) = mpsc::channel(32);
    if output.send(LspEvent::Ready(cmd_tx)).await.is_err() {
        return;
    }

    // Per-document version counter: `didChange` versions must strictly
    // increase (LSP spec, section 3.1.4) or the server treats the
    // notification as stale and drops it, silently freezing diagnostics
    // after the first edit.
    let mut doc_versions: HashMap<Url, i32> = HashMap::new();

    loop {
        futures::select! {
            r = mainloop_fut => {
                if let Err(err) = r {
                    let line = format!("\n[lsp-mainloop] error: {err:?}\n");
                    let _ = std::fs::OpenOptions::new()
                        .append(true)
                        .open(&log_path)
                        .and_then(|mut f| { use std::io::Write; f.write_all(line.as_bytes()) });
                }
                break;
            }
            diag = diagnostics_rx.next() => {
                let Some(params) = diag else { break };
                let event = LspEvent::Diagnostics {
                    uri: params.uri,
                    diagnostics: params.diagnostics,
                };
                if output.send(event).await.is_err() {
                    break;
                }
            }
            cmd = cmd_rx.next() => {
                let Some(cmd) = cmd else { break };
                match cmd {
                    LspCommand::DidOpen { uri, text } => {
                        doc_versions.insert(uri.clone(), 0);
                        let _ = server.did_open(DidOpenTextDocumentParams {
                            text_document: TextDocumentItem {
                                uri,
                                language_id: language.language_id().into(),
                                version: 0,
                                text,
                            },
                        });
                    }
                    LspCommand::DidChange { uri, text } => {
                        let version = doc_versions.entry(uri.clone()).or_insert(0);
                        *version += 1;
                        let _ = server.did_change(DidChangeTextDocumentParams {
                            text_document: VersionedTextDocumentIdentifier { uri, version: *version },
                            content_changes: vec![TextDocumentContentChangeEvent {
                                range: None,
                                range_length: None,
                                text,
                            }],
                        });
                    }
                    LspCommand::DidClose { uri } => {
                        doc_versions.remove(&uri);
                        let _ = server.did_close(DidCloseTextDocumentParams {
                            text_document: TextDocumentIdentifier { uri },
                        });
                    }
                    LspCommand::Completion { uri, line, character } => {
                        // Drive mainloop alongside the completion request so
                        // the server can process our request and send a reply.
                        let comp_fut = server
                            .completion(CompletionParams {
                                text_document_position: TextDocumentPositionParams {
                                    text_document: TextDocumentIdentifier { uri: uri.clone() },
                                    position: Position { line, character },
                                },
                                work_done_progress_params: WorkDoneProgressParams::default(),
                                partial_result_params: PartialResultParams::default(),
                                context: None,
                            })
                            .fuse();
                        futures::pin_mut!(comp_fut);
                        let result = loop {
                            futures::select! {
                                r = comp_fut => break Some(r),
                                r = mainloop_fut => {
                                    if let Err(err) = r {
                                        let line_str = format!("\n[lsp-mainloop] error: {err:?}\n");
                                        let _ = std::fs::OpenOptions::new()
                                            .append(true)
                                            .open(&log_path)
                                            .and_then(|mut f| { use std::io::Write; f.write_all(line_str.as_bytes()) });
                                    }
                                    break None;
                                }
                            }
                        };
                        let Some(result) = result else { return; };
                        if let Ok(Some(response)) = result {
                            let items = match response {
                                CompletionResponse::Array(v) => v,
                                CompletionResponse::List(l) => l.items,
                            };
                            let capped: Vec<_> = items.into_iter().take(50).collect();
                            let _ = output
                                .send(LspEvent::Completions {
                                    uri,
                                    line,
                                    character,
                                    items: capped,
                                })
                                .await;
                        }
                    }
                }
            }
        }
    }
}

/// Builds a `java -jar` command that launches jdtls the same way VS Code /
/// Zed do — bypassing the Python wrapper script that is fragile to env issues.
///
/// `install_dir` is `~/.local/share/devscribe/servers/jdtls/` (the directory
/// produced by `TarGzDirectory` extraction). Returns `None` if `java` is not
/// on PATH or the launcher JAR cannot be found.
fn jdtls_command(install_dir: &std::path::Path, root: &std::path::Path) -> Option<async_process::Command> {
    // Equinox launcher JAR — filename contains a version number, so we glob.
    let launcher = std::fs::read_dir(install_dir.join("plugins"))
        .ok()?
        .filter_map(|e| e.ok())
        .find(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("org.eclipse.equinox.launcher_")
        })?
        .path();

    let config_dir = install_dir.join(jdtls_config_dir());

    let workspace = dirs::data_dir()
        .unwrap_or_else(|| std::env::temp_dir())
        .join("devscribe")
        .join("jdtls-workspaces")
        .join(fnv_hex(&root.to_string_lossy()));
    let _ = std::fs::create_dir_all(&workspace);

    let mut cmd = async_process::Command::new("java");
    cmd.args([
        "-Declipse.application=org.eclipse.jdt.ls.core.id1",
        "-Dosgi.bundles.defaultStartLevel=4",
        "-Declipse.product=org.eclipse.jdt.ls.core.product",
        "-Dlog.level=ALL",
        "-Xmx1G",
        "--add-modules=ALL-SYSTEM",
        // Two-arg form required; the = form is not universally accepted.
        "--add-opens", "java.base/java.util=ALL-UNNAMED",
        "--add-opens", "java.base/java.lang=ALL-UNNAMED",
        "--add-opens", "java.base/sun.nio.fs=ALL-UNNAMED",
        "-jar",
    ])
    .arg(&launcher)
    .arg("-configuration")
    .arg(&config_dir)
    .arg("-data")
    .arg(&workspace);

    Some(cmd)
}

fn jdtls_config_dir() -> &'static str {
    if cfg!(target_os = "macos") {
        "config_mac"
    } else if cfg!(target_os = "windows") {
        "config_win"
    } else {
        "config_linux"
    }
}

/// FNV-1a hash of `s` as a short hex string — used to derive stable,
/// filesystem-safe directory names from arbitrary paths.
fn fnv_hex(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

#[cfg(test)]
#[path = "tests/lsp.rs"]
mod tests;
