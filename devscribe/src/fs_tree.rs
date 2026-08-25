//! A real (non-hardcoded) walk of a project directory for the sidebar's
//! file tree. Directories that are never useful to browse in an editor
//! (`.git`, `target`, build/dependency dirs, dotfiles) are skipped.
use devscribe_core::theme::{Palette, Rgba};
use iced::Color;
use std::path::{Path, PathBuf};

use crate::color::color;

const SKIP_DIRS: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    ".idea",
    ".vscode",
    ".claude",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Rust,
    Json,
    Cpp,
    Header,
    Java,
    Ts,
    Tsx,
    Toml,
    Md,
    Other,
}

impl Lang {
    pub fn from_path(path: &Path) -> Self {
        match path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref()
        {
            Some("rs") => Lang::Rust,
            Some("json") => Lang::Json,
            Some("cpp" | "cc" | "cxx" | "c") => Lang::Cpp,
            Some("h" | "hpp") => Lang::Header,
            Some("java") => Lang::Java,
            Some("ts") => Lang::Ts,
            Some("tsx" | "jsx") => Lang::Tsx,
            Some("toml") => Lang::Toml,
            Some("md") => Lang::Md,
            _ => Lang::Other,
        }
    }

    /// The short badge label (mockup convention: two uppercase mono chars).
    pub fn code(self, path: &Path) -> String {
        match self {
            Lang::Rust => "RS".into(),
            Lang::Json => "JS".into(),
            Lang::Cpp => "C+".into(),
            Lang::Header => "H".into(),
            Lang::Java => "JV".into(),
            Lang::Ts => "TS".into(),
            Lang::Tsx => "TX".into(),
            Lang::Toml => "TO".into(),
            Lang::Md => "MD".into(),
            Lang::Other => path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_ascii_uppercase().chars().take(2).collect())
                .unwrap_or_else(|| "\u{2022}".into()),
        }
    }

    /// (foreground, background) matching the mockup's per-language badge tints.
    pub fn badge(self, p: Palette) -> (Color, Color) {
        match self {
            Lang::Rust => (color(p.status_warning), tint(p.status_warning, 0.16)),
            Lang::Json => (color(p.status_success), tint(p.status_success, 0.16)),
            Lang::Cpp => (color(p.status_info), tint(p.status_info, 0.18)),
            Lang::Header => (color(p.text_muted), tint(p.status_info, 0.12)),
            Lang::Java => (color(p.status_danger), tint(p.status_danger, 0.16)),
            Lang::Ts => (color(p.accent_solid), tint(p.accent_solid, 0.22)),
            Lang::Tsx => (color(p.status_info), tint(p.accent_solid, 0.22)),
            Lang::Toml => (color(p.text_muted), tint(p.text_muted, 0.14)),
            Lang::Md => (color(p.text_muted), tint(p.text_muted, 0.14)),
            Lang::Other => (color(p.text_muted), tint(p.text_muted, 0.12)),
        }
    }
}

fn tint(c: Rgba, alpha: f32) -> Color {
    color(Rgba { a: alpha, ..c })
}

#[derive(Debug, Clone)]
pub enum Node {
    Dir { name: String, path: PathBuf, children: Vec<Node> },
    File { name: String, path: PathBuf, lang: Lang },
}

fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

/// Walks `root` and builds a `Node` tree: directories first (alphabetical),
/// then files (alphabetical). Returns an empty vec if `root` can't be read.
pub fn walk(root: &Path) -> Vec<Node> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };

    let mut dirs = Vec::new();
    let mut files = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if is_hidden(name) || SKIP_DIRS.contains(&name) {
            continue;
        }

        let Ok(file_type) = entry.file_type() else {
            continue;
        };

        if file_type.is_dir() {
            dirs.push((name.to_string(), path));
        } else if file_type.is_file() {
            files.push((name.to_string(), path));
        }
    }

    dirs.sort_by_key(|a| a.0.to_ascii_lowercase());
    files.sort_by_key(|a| a.0.to_ascii_lowercase());

    let mut nodes: Vec<Node> = dirs
        .into_iter()
        .map(|(name, path)| Node::Dir {
            children: walk(&path),
            name,
            path,
        })
        .collect();

    nodes.extend(files.into_iter().map(|(name, path)| {
        let lang = Lang::from_path(&path);
        Node::File { name, path, lang }
    }));

    nodes
}

/// Every file path under `nodes`, depth-first — used by project-wide search.
pub fn flatten_files(nodes: &[Node]) -> Vec<&Path> {
    let mut files = Vec::new();
    fn walk<'a>(nodes: &'a [Node], out: &mut Vec<&'a Path>) {
        for node in nodes {
            match node {
                Node::Dir { children, .. } => walk(children, out),
                Node::File { path, .. } => out.push(path.as_path()),
            }
        }
    }
    walk(nodes, &mut files);
    files
}

/// Every directory path under `nodes`, at every depth — used to seed
/// `State.collapsed_dirs` so the sidebar tree starts fully collapsed.
pub fn flatten_dirs(nodes: &[Node]) -> Vec<&Path> {
    let mut dirs = Vec::new();
    fn walk<'a>(nodes: &'a [Node], out: &mut Vec<&'a Path>) {
        for node in nodes {
            if let Node::Dir { path, children, .. } = node {
                out.push(path.as_path());
                walk(children, out);
            }
        }
    }
    walk(nodes, &mut dirs);
    dirs
}
