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
    ///
    /// A real project can hold lines with hundreds of thousands of chars
    /// (a minified bundle, a generated blob) — anything rendering only a
    /// prefix of the line should reach for `line_text_capped` instead of
    /// materializing the whole thing.
    pub fn line_text(&self, line: usize) -> String {
        self.line_text_capped(line, usize::MAX)
    }

    /// The first `max_chars` chars of `line`, excluding its trailing line
    /// terminator. Identical to `line_text` for ordinary lines; the point is
    /// pathological ones, where materializing the full line allocates
    /// megabytes per call. The editor canvas cannot show more columns than
    /// fit its width (there is no horizontal scrolling), so it asks only for
    /// what it will actually draw.
    pub fn line_text_capped(&self, line: usize, max_chars: usize) -> String {
        let line = line.min(self.text.len_lines().saturating_sub(1));
        let len = self.line_len_chars(line).min(max_chars);
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

    /// The char range of `line` including its trailing line terminator, so
    /// removing it deletes the line as a whole rather than leaving a blank
    /// one behind. If `line` is the last line and has no terminator of its
    /// own (the common case: files don't end with a blank line), the range
    /// extends to the document's end instead of past it.
    pub fn line_char_range_with_terminator(&self, line: usize) -> std::ops::Range<usize> {
        let line = line.min(self.text.len_lines().saturating_sub(1));
        let start = self.text.line_to_char(line);
        let end = if line + 1 < self.text.len_lines() {
            self.text.line_to_char(line + 1)
        } else {
            self.text.len_chars()
        };
        start..end
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
#[path = "tests/document.rs"]
mod tests;
