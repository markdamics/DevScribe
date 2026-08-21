use std::path::PathBuf;

use devscribe_core::syntax::{self, Span};
use devscribe_core::theme::ThemeName;
use devscribe_core::Document;

use crate::fs_tree::{self, Node};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Code,
    Json,
    Search,
    Diff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CursorPos {
    pub line: usize,
    pub col: usize,
}

impl From<(usize, usize)> for CursorPos {
    fn from((line, col): (usize, usize)) -> Self {
        Self { line, col }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
    LineStart,
    LineEnd,
}

/// An open file: its buffer plus interaction state (cursor, selection). Real
/// keyboard/mouse editing lives here; `editor_canvas.rs` only ever reads it
/// and turns raw input events into the `Message`s that call these methods.
pub struct EditorState {
    pub document: Document,
    pub path: PathBuf,
    pub cursor: CursorPos,
    pub selection_anchor: Option<CursorPos>,
    pub language: Option<syntax::Language>,
    pub highlights: Vec<Span>,
    highlighter: syntax::Highlighter,
}

impl EditorState {
    pub fn new(document: Document, path: PathBuf) -> Self {
        let language = path
            .extension()
            .and_then(|ext| ext.to_str())
            .and_then(syntax::Language::from_extension);
        let mut highlighter = syntax::Highlighter::new();
        let highlights = match language {
            Some(lang) => highlighter.highlight(lang, &document.text().to_string()),
            None => Vec::new(),
        };
        Self {
            document,
            path,
            cursor: CursorPos::default(),
            selection_anchor: None,
            language,
            highlights,
            highlighter,
        }
    }

    /// Recomputes `highlights` from the current buffer contents. Cheap
    /// relative to a full reparse would suggest otherwise, but tree-sitter
    /// is fast enough that doing this on every edit is fine — see
    /// `devscribe_core::syntax` for why this isn't true incremental reparsing.
    fn rehighlight(&mut self) {
        if let Some(lang) = self.language {
            self.highlights = self
                .highlighter
                .highlight(lang, &self.document.text().to_string());
        }
    }

    fn selection_range(&self) -> Option<(usize, usize)> {
        self.selection_anchor.map(|anchor| {
            let a = self.document.char_index(anchor.line, anchor.col);
            let b = self.document.char_index(self.cursor.line, self.cursor.col);
            if a <= b {
                (a, b)
            } else {
                (b, a)
            }
        })
    }

    /// Non-empty (start, end) char range currently selected, if any.
    pub fn selection(&self) -> Option<(usize, usize)> {
        self.selection_range().filter(|(start, end)| start != end)
    }

    /// Deletes the current selection, if any. Returns whether it deleted anything.
    fn delete_selection(&mut self) -> bool {
        let range = self.selection();
        self.selection_anchor = None;
        if let Some((start, end)) = range {
            self.document.remove(start..end);
            self.cursor = self.document.line_col(start).into();
            true
        } else {
            false
        }
    }

    pub fn insert_text(&mut self, text: &str) {
        self.delete_selection();
        let idx = self.document.char_index(self.cursor.line, self.cursor.col);
        self.document.insert(idx, text);
        let new_idx = idx + text.chars().count();
        self.cursor = self.document.line_col(new_idx).into();
        self.rehighlight();
    }

    pub fn backspace(&mut self) {
        if self.delete_selection() {
            self.rehighlight();
            return;
        }
        let idx = self.document.char_index(self.cursor.line, self.cursor.col);
        if idx == 0 {
            return;
        }
        self.document.remove(idx - 1..idx);
        self.cursor = self.document.line_col(idx - 1).into();
        self.rehighlight();
    }

    pub fn delete_forward(&mut self) {
        if self.delete_selection() {
            self.rehighlight();
            return;
        }
        let idx = self.document.char_index(self.cursor.line, self.cursor.col);
        if idx >= self.document.text().len_chars() {
            return;
        }
        self.document.remove(idx..idx + 1);
        self.cursor = self.document.line_col(idx).into();
        self.rehighlight();
    }

    pub fn move_cursor(&mut self, dir: Direction, extend: bool) {
        if extend {
            if self.selection_anchor.is_none() {
                self.selection_anchor = Some(self.cursor);
            }
        } else {
            self.selection_anchor = None;
        }

        match dir {
            Direction::Left => {
                let idx = self.document.char_index(self.cursor.line, self.cursor.col);
                if idx > 0 {
                    self.cursor = self.document.line_col(idx - 1).into();
                }
            }
            Direction::Right => {
                let idx = self.document.char_index(self.cursor.line, self.cursor.col);
                if idx < self.document.text().len_chars() {
                    self.cursor = self.document.line_col(idx + 1).into();
                }
            }
            Direction::Up => {
                if self.cursor.line > 0 {
                    self.cursor.line -= 1;
                    self.cursor.col = self
                        .cursor
                        .col
                        .min(self.document.line_len_chars(self.cursor.line));
                }
            }
            Direction::Down => {
                if self.cursor.line + 1 < self.document.line_count() {
                    self.cursor.line += 1;
                    self.cursor.col = self
                        .cursor
                        .col
                        .min(self.document.line_len_chars(self.cursor.line));
                }
            }
            Direction::LineStart => self.cursor.col = 0,
            Direction::LineEnd => {
                self.cursor.col = self.document.line_len_chars(self.cursor.line);
            }
        }
    }

    pub fn click(&mut self, line: usize, col: usize, extend: bool) {
        if extend {
            if self.selection_anchor.is_none() {
                self.selection_anchor = Some(self.cursor);
            }
        } else {
            self.selection_anchor = None;
        }
        self.cursor = CursorPos { line, col };
    }
}

pub struct State {
    pub theme: ThemeName,
    pub active_tab: Tab,
    pub assist_on: bool,
    pub projects_open: bool,
    pub overflow_open: bool,
    /// Project root the sidebar tree was walked from.
    pub root: PathBuf,
    /// Walked once at startup (filesystem walks are far too slow to redo on
    /// every `view()` — the caret-blink subscription alone redraws 2x/sec).
    pub tree: Vec<Node>,
    pub editor: Option<EditorState>,
    pub caret_visible: bool,
}

impl Default for State {
    fn default() -> Self {
        let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let tree = fs_tree::walk(&root);
        Self {
            theme: ThemeName::NullGrid,
            active_tab: Tab::Code,
            assist_on: true,
            projects_open: false,
            overflow_open: false,
            root,
            tree,
            editor: None,
            caret_visible: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    SetTheme(ThemeName),
    SelectTab(Tab),
    ToggleAssist,
    ToggleProjects,
    ToggleOverflow,
    SelectFile(PathBuf),
    EditorInsertText(String),
    EditorBackspace,
    EditorDelete,
    EditorMove { dir: Direction, extend: bool },
    EditorClick { line: usize, col: usize, extend: bool },
    CaretTick,
}

pub fn update(state: &mut State, message: Message) {
    match message {
        Message::SetTheme(theme) => state.theme = theme,
        Message::SelectTab(tab) => state.active_tab = tab,
        Message::ToggleAssist => state.assist_on = !state.assist_on,
        Message::ToggleProjects => state.projects_open = !state.projects_open,
        Message::ToggleOverflow => state.overflow_open = !state.overflow_open,
        Message::SelectFile(path) => {
            if let Ok(document) = Document::open(&path) {
                state.editor = Some(EditorState::new(document, path));
                state.active_tab = Tab::Code;
            }
        }
        Message::EditorInsertText(text) => {
            if let Some(editor) = state.editor.as_mut() {
                editor.insert_text(&text);
            }
        }
        Message::EditorBackspace => {
            if let Some(editor) = state.editor.as_mut() {
                editor.backspace();
            }
        }
        Message::EditorDelete => {
            if let Some(editor) = state.editor.as_mut() {
                editor.delete_forward();
            }
        }
        Message::EditorMove { dir, extend } => {
            if let Some(editor) = state.editor.as_mut() {
                editor.move_cursor(dir, extend);
            }
        }
        Message::EditorClick { line, col, extend } => {
            if let Some(editor) = state.editor.as_mut() {
                editor.click(line, col, extend);
            }
        }
        Message::CaretTick => state.caret_visible = !state.caret_visible,
    }
}

pub fn subscription(_state: &State) -> iced::Subscription<Message> {
    iced::time::every(std::time::Duration::from_millis(530)).map(|_| Message::CaretTick)
}
