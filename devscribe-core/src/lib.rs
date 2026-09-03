pub mod claude_agent;
pub mod copilot_agent;
pub mod copilot_completion;
pub mod diff;
pub mod document;
pub mod git;
pub mod lsp;
pub mod outline;
pub mod search;
pub mod syntax;
pub mod theme;
pub mod watcher;

pub use document::{Document, Eol};
pub use theme::{Accent, Palette, Theme, ThemeMode};
