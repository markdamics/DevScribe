//! A minimal `async-lsp` client: spawns one language server per language
//! (currently just `rust-analyzer` for Rust), forwards `didOpen`/`didChange`
//! notifications to it, and streams `publishDiagnostics` back out. Full-text
//! sync only (no incremental `TextDocumentContentChangeEvent` ranges) — same
//! "reparse the whole thing" simplification as `syntax`, and always
//! spec-legal regardless of what the server advertises.
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::process::Stdio;

use async_lsp::lsp_types::{
    ClientCapabilities, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, InitializeParams, InitializedParams, ProgressParams,
    PublishDiagnosticsParams, ShowMessageParams, TextDocumentContentChangeEvent,
    TextDocumentIdentifier, TextDocumentItem, VersionedTextDocumentIdentifier, WorkspaceFolder,
};
use async_lsp::router::Router;
use async_lsp::{LanguageClient, LanguageServer, MainLoop, ResponseError};
use futures::channel::mpsc;
use futures::{SinkExt, StreamExt};

pub use async_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Url};

/// A language DevScribe knows how to talk to a language server for. Kept
/// separate from `syntax::Language` — the two sets will diverge as more
/// grammars than language servers (or vice versa) get wired up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LspLanguage {
    Rust,
}

impl LspLanguage {
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "rs" => Some(Self::Rust),
            _ => None,
        }
    }

    fn command(self) -> &'static str {
        match self {
            Self::Rust => "rust-analyzer",
        }
    }

    fn language_id(self) -> &'static str {
        match self {
            Self::Rust => "rust",
        }
    }
}

/// A command the app sends into a running worker (see `run`).
#[derive(Debug, Clone)]
pub enum LspCommand {
    DidOpen { uri: Url, text: String },
    DidChange { uri: Url, text: String },
    DidClose { uri: Url },
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
}

/// Spawns a language server for `root` and drives it until the process dies
/// or `output` is dropped. Intended to be run as the body of a long-lived
/// stream worker (see `devscribe`'s `subscription()`), not called directly.
pub async fn run(root: PathBuf, mut output: mpsc::Sender<LspEvent>) {
    let language = LspLanguage::Rust;

    let mut child = match async_process::Command::new(language.command())
        .current_dir(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
    {
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
        Router::from_language_client(ClientState { diagnostics_tx })
    });

    tokio::spawn(async move {
        let _ = mainloop.run_buffered(child_stdout, child_stdin).await;
    });

    let Some(root_uri) = Url::from_file_path(&root).ok() else {
        let _ = output
            .send(LspEvent::Unavailable("project root has no file:// URI".into()))
            .await;
        return;
    };

    let init = server
        .initialize(InitializeParams {
            workspace_folders: Some(vec![WorkspaceFolder {
                uri: root_uri,
                name: "root".into(),
            }]),
            capabilities: ClientCapabilities::default(),
            ..InitializeParams::default()
        })
        .await;
    if let Err(err) = init {
        let _ = output
            .send(LspEvent::Unavailable(format!("initialize failed: {err}")))
            .await;
        return;
    }
    if server.initialized(InitializedParams {}).is_err() {
        let _ = output
            .send(LspEvent::Unavailable("server stopped during startup".into()))
            .await;
        return;
    }

    let (cmd_tx, mut cmd_rx) = mpsc::channel(32);
    if output.send(LspEvent::Ready(cmd_tx)).await.is_err() {
        return;
    }

    loop {
        futures::select! {
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
                        let _ = server.did_change(DidChangeTextDocumentParams {
                            text_document: VersionedTextDocumentIdentifier { uri, version: 0 },
                            content_changes: vec![TextDocumentContentChangeEvent {
                                range: None,
                                range_length: None,
                                text,
                            }],
                        });
                    }
                    LspCommand::DidClose { uri } => {
                        let _ = server.did_close(DidCloseTextDocumentParams {
                            text_document: TextDocumentIdentifier { uri },
                        });
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/lsp.rs"]
mod tests;
