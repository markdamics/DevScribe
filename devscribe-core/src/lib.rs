pub mod claude_agent;
pub mod diff;
pub mod document;
pub mod git;
pub mod lsp;
pub mod outline;
pub mod search;
pub mod syntax;
pub mod theme;
pub mod watcher;

pub use document::Document;
pub use theme::{Accent, Palette, Theme, ThemeMode};
