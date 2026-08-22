use ropey::Rope;

/// A single open file's text buffer. Positions are `(line, column)` in
/// `char`s, matching `ropey`'s indexing; byte offsets are derived on demand
/// rather than stored, since `Rope` makes that conversion cheap.
#[derive(Debug, Clone)]
pub struct Document {
    text: Rope,
    path: Option<std::path::PathBuf>,
    dirty: bool,
}

impl Document {
    pub fn empty() -> Self {
        Self {
            text: Rope::new(),
            path: None,
            dirty: false,
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(contents: &str) -> Self {
        Self {
            text: Rope::from_str(contents),
            path: None,
            dirty: false,
        }
    }

    pub fn open(path: impl Into<std::path::PathBuf>) -> std::io::Result<Self> {
        let path = path.into();
        let file = std::fs::File::open(&path)?;
        let text = Rope::from_reader(std::io::BufReader::new(file))?;
        Ok(Self {
            text,
            path: Some(path),
            dirty: false,
        })
    }

    pub fn text(&self) -> &Rope {
        &self.text
    }

    /// Writes the buffer back to `path()`, clearing the dirty flag on
    /// success. Errors (no path, permission denied, etc.) leave the buffer
    /// untouched — the caller decides how to surface that.
    pub fn save(&mut self) -> std::io::Result<()> {
        let path = self
            .path
            .as_ref()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "document has no path"))?;
        let file = std::fs::File::create(path)?;
        self.text.write_to(std::io::BufWriter::new(file))?;
        self.dirty = false;
        Ok(())
    }

    pub fn path(&self) -> Option<&std::path::Path> {
        self.path.as_deref()
    }

    /// Repoints this document at `path` without touching its buffer —
    /// used after a filesystem rename so a subsequent `save()` writes to the
    /// new location instead of the (now nonexistent) old one.
    pub fn set_path(&mut self, path: std::path::PathBuf) {
        self.path = Some(path);
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn line_count(&self) -> usize {
        self.text.len_lines()
    }

    /// Insert `text` at the given char index, marking the document dirty.
    pub fn insert(&mut self, char_idx: usize, text: &str) {
        self.text.insert(char_idx, text);
        self.dirty = true;
    }

    /// Remove the `char_range` (start..end, in chars), marking the document dirty.
    pub fn remove(&mut self, char_range: std::ops::Range<usize>) {
        self.text.remove(char_range);
        self.dirty = true;
    }

    /// Number of chars on `line`, excluding its trailing line terminator
    /// (`\n` or `\r\n`). Out-of-range lines clamp to the last line.
    pub fn line_len_chars(&self, line: usize) -> usize {
        let line = line.min(self.text.len_lines().saturating_sub(1));
        let slice = self.text.line(line);
        let mut len = slice.len_chars();
        if len > 0 && slice.char(len - 1) == '\n' {
            len -= 1;
            if len > 0 && slice.char(len - 1) == '\r' {
                len -= 1;
            }
        }
        len
    }

    /// The text of `line`, excluding its trailing line terminator.
    pub fn line_text(&self, line: usize) -> String {
        let line = line.min(self.text.len_lines().saturating_sub(1));
        let len = self.line_len_chars(line);
        let start = self.text.line_to_char(line);
        self.text.slice(start..start + len).to_string()
    }

    /// Converts a `(line, col)` position (both clamped to valid ranges) into
    /// an absolute char index.
    pub fn char_index(&self, line: usize, col: usize) -> usize {
        let line = line.min(self.text.len_lines().saturating_sub(1));
        let col = col.min(self.line_len_chars(line));
        self.text.line_to_char(line) + col
    }

    /// Converts an absolute char index (clamped to the document's length)
    /// into a `(line, col)` position.
    pub fn line_col(&self, char_idx: usize) -> (usize, usize) {
        let char_idx = char_idx.min(self.text.len_chars());
        let line = self.text.char_to_line(char_idx);
        let col = char_idx - self.text.line_to_char(line);
        (line, col)
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_marks_dirty_and_updates_text() {
        let mut doc = Document::from_str("hello world");
        assert!(!doc.is_dirty());
        doc.insert(5, ",");
        assert!(doc.is_dirty());
        assert_eq!(doc.text().to_string(), "hello, world");
    }

    #[test]
    fn remove_marks_dirty_and_updates_text() {
        let mut doc = Document::from_str("hello, world");
        doc.remove(5..6);
        assert_eq!(doc.text().to_string(), "hello world");
    }

    #[test]
    fn save_without_path_errors() {
        let mut doc = Document::from_str("no path");
        assert!(doc.save().is_err());
    }

    #[test]
    fn save_writes_buffer_and_clears_dirty() {
        let path = std::env::temp_dir().join(format!("devscribe-core-save-test-{}", std::process::id()));
        std::fs::write(&path, "original").unwrap();

        let mut doc = Document::open(&path).unwrap();
        doc.insert(0, "edited ");
        assert!(doc.is_dirty());

        doc.save().unwrap();
        assert!(!doc.is_dirty());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "edited original");

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn line_count_matches_rope() {
        let doc = Document::from_str("a\nb\nc");
        assert_eq!(doc.line_count(), 3);
    }

    #[test]
    fn line_len_chars_excludes_terminator() {
        let doc = Document::from_str("abc\ndef\r\ngh");
        assert_eq!(doc.line_len_chars(0), 3);
        assert_eq!(doc.line_len_chars(1), 3);
        assert_eq!(doc.line_len_chars(2), 2);
    }

    #[test]
    fn line_text_excludes_terminator() {
        let doc = Document::from_str("abc\ndef\r\ngh");
        assert_eq!(doc.line_text(0), "abc");
        assert_eq!(doc.line_text(1), "def");
        assert_eq!(doc.line_text(2), "gh");
    }

    #[test]
    fn char_index_and_line_col_round_trip() {
        let doc = Document::from_str("abc\ndef\ngh");
        assert_eq!(doc.char_index(1, 2), 6);
        assert_eq!(doc.line_col(6), (1, 2));
        // Column past end-of-line clamps to the line's length.
        assert_eq!(doc.char_index(0, 99), 3);
    }
}
