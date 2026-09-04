//! Line-level text diffing, independent of `git` — the diff panel feeds it
//! `HEAD`'s blob text (from `git::Repo::head_text`) against the live buffer.
use similar::{ChangeTag, TextDiff, WhitespaceMode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKind {
    Equal,
    Insert,
    Delete,
}

impl From<ChangeTag> for DiffLineKind {
    fn from(tag: ChangeTag) -> Self {
        match tag {
            ChangeTag::Equal => DiffLineKind::Equal,
            ChangeTag::Insert => DiffLineKind::Insert,
            ChangeTag::Delete => DiffLineKind::Delete,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    /// 0-based line number in `old`, when this line came from it.
    pub old_line: Option<usize>,
    /// 0-based line number in `new`, when this line came from it.
    pub new_line: Option<usize>,
    pub text: String,
}

/// A line-by-line diff of `old` against `new`, in document order. Exact
/// whitespace comparison — see `diff_lines_ignoring_whitespace` for the
/// "Ignore Whitespace" diff-view toggle's version of this.
pub fn diff_lines(old: &str, new: &str) -> Vec<DiffLine> {
    diff_lines_with_mode(old, new, WhitespaceMode::Exact)
}

/// Same as `diff_lines`, but a line that only differs from its counterpart
/// by whitespace (leading/trailing/run-length — git's `-w`/`--ignore-all-space`)
/// diffs as `Equal` instead of `Insert`/`Delete`. Line splitting itself is
/// untouched (same tokenizer either way), so `old_line`/`new_line` indices
/// and hunk/gutter-mark grouping downstream behave identically either way —
/// only *which* lines count as changed differs.
pub fn diff_lines_ignoring_whitespace(old: &str, new: &str) -> Vec<DiffLine> {
    diff_lines_with_mode(old, new, WhitespaceMode::IgnoreAll)
}

fn diff_lines_with_mode(old: &str, new: &str, mode: WhitespaceMode) -> Vec<DiffLine> {
    TextDiff::configure()
        .whitespace_mode(mode)
        .diff_lines(old, new)
        .iter_all_changes()
        .map(|change| DiffLine {
            kind: change.tag().into(),
            old_line: change.old_index(),
            new_line: change.new_index(),
            text: change.value().trim_end_matches(['\n', '\r']).to_string(),
        })
        .collect()
}

/// One word (or run of whitespace/punctuation — `similar`'s word tokenizer,
/// not a linguistic one) of a single-line word-level diff — see `diff_words`.
#[derive(Debug, Clone)]
pub struct WordSpan {
    pub kind: DiffLineKind,
    pub text: String,
}

/// A word-level diff of two *single lines* (typically a paired `Delete`
/// then `Insert` from the same `Hunk`) — the change list `diff_lines` would
/// give two whole-line entries for, broken down further so the diff view
/// can highlight just the changed words within an otherwise-similar line
/// instead of tinting the entire line uniformly. `Equal` spans appear on
/// both `old`'s and `new`'s own reconstruction; `Delete` spans only belong
/// to `old`'s, `Insert` spans only to `new`'s — same filtering
/// `diff_row_spans` in `diff_view.rs` does.
pub fn diff_words(old: &str, new: &str) -> Vec<WordSpan> {
    TextDiff::from_words(old, new)
        .iter_all_changes()
        .map(|change| WordSpan {
            kind: change.tag().into(),
            text: change.value().to_string(),
        })
        .collect()
}

/// A gutter marker for one line of the *new* (current) buffer, carrying
/// whatever `HEAD` text a "revert this line" action would need — so drawing
/// the marker and performing the revert both read from the same derived
/// value instead of re-deriving the pairing independently and risking the
/// two disagreeing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GutterMark {
    /// This line has no `HEAD` counterpart at all — reverting removes it.
    Added,
    /// This line replaces `head_text` at the same position — reverting
    /// restores `head_text`.
    Modified { head_text: String },
    /// `head_lines` sat immediately above this line in `HEAD` and don't
    /// survive into the new buffer (or, if the deletion ran off the end of
    /// the file, this is attached to the last line instead) — reverting
    /// re-inserts them above this line.
    RemovedAbove { head_lines: Vec<String> },
}

/// A maximal run of consecutive non-`Equal` lines in a `diff_lines` result —
/// the unit the diff view's "revert selected changes" selects and acts on,
/// and the standard "replace" grouping a unified diff shows as one hunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    /// Range into the `lines` slice `hunks` was called with, covering every
    /// row (`Delete` and/or `Insert`) that belongs to this hunk.
    pub range: std::ops::Range<usize>,
    /// The `(new_line, GutterMark)` pairs this hunk contributes — reverting
    /// the hunk means reverting each of these. Order matches `gutter_marks`'
    /// own derivation, but callers that revert more than one should still
    /// go in descending `new_line` order themselves, so an earlier revert's
    /// edit can't shift a later target's line number out from under it.
    pub marks: Vec<(usize, GutterMark)>,
}

/// Groups `lines` (as returned by `diff_lines(old, new)`) into `Hunk`s,
/// walking it once and pairing up each maximal run of consecutive `Delete`s
/// with the maximal run of consecutive `Insert`s immediately following it.
/// The shorter run's length pairs off as `Modified`; any excess on the
/// insert side is `Added`, any excess on the delete side (or a delete run
/// with no adjacent inserts at all) becomes a `RemovedAbove` on whatever new
/// line comes right after, or the last line if the run reaches end of file.
/// When a line would otherwise get both (more deletes than inserts, so the
/// paired line is `Modified` but also absorbs the run's excess deletes),
/// `RemovedAbove` wins — it carries the deleted lines' text, so reverting it
/// first is what a second click on the newly-restored `Modified` line would
/// need anyway.
///
/// `gutter_marks` is just this, flattened into a per-line array — kept as
/// its own pass so both derive from the same grouping rather than risking
/// the two disagreeing.
pub fn hunks(lines: &[DiffLine], new_line_count: usize) -> Vec<Hunk> {
    let mut out = Vec::new();
    if new_line_count == 0 {
        return out;
    }

    let mut i = 0;
    while i < lines.len() {
        if lines[i].kind == DiffLineKind::Equal {
            i += 1;
            continue;
        }

        let hunk_start = i;
        let mut marks = Vec::new();

        if lines[i].kind == DiffLineKind::Insert {
            // A standalone insert run — one with no delete run immediately
            // before it, since such a run would already have been consumed
            // by the `Delete` branch below — is a pure addition.
            let insert_start = i;
            while i < lines.len() && lines[i].kind == DiffLineKind::Insert {
                i += 1;
            }
            for line in &lines[insert_start..i] {
                if let Some(new_line) = line.new_line {
                    marks.push((new_line, GutterMark::Added));
                }
            }
            out.push(Hunk { range: hunk_start..i, marks });
            continue;
        }

        let delete_start = i;
        while i < lines.len() && lines[i].kind == DiffLineKind::Delete {
            i += 1;
        }
        let delete_lines = &lines[delete_start..i];

        let insert_start = i;
        while i < lines.len() && lines[i].kind == DiffLineKind::Insert {
            i += 1;
        }
        let insert_lines = &lines[insert_start..i];
        let paired = delete_lines.len().min(insert_lines.len());

        for (delete, insert) in delete_lines[..paired].iter().zip(&insert_lines[..paired]) {
            if let Some(new_line) = insert.new_line {
                marks.push((
                    new_line,
                    GutterMark::Modified { head_text: delete.text.clone() },
                ));
            }
        }
        for line in &insert_lines[paired..] {
            if let Some(new_line) = line.new_line {
                marks.push((new_line, GutterMark::Added));
            }
        }

        if delete_lines.len() > paired {
            // Excess deletes have no surviving new-side line of their own —
            // attach them to whatever comes right after the run (the last
            // paired insert, if any, otherwise the next line with a
            // `new_line` at all), or the last line if the run went off the
            // end of the file.
            let head_lines: Vec<String> = delete_lines[paired..].iter().map(|l| l.text.clone()).collect();
            let target = insert_lines
                .last()
                .and_then(|l| l.new_line)
                .or_else(|| lines[i..].iter().find_map(|l| l.new_line))
                .unwrap_or(new_line_count - 1);
            // `RemovedAbove` wins over a `Modified` already recorded for
            // `target` above — same precedence the module doc promises.
            marks.retain(|(new_line, _)| *new_line != target);
            marks.push((target, GutterMark::RemovedAbove { head_lines }));
        }

        out.push(Hunk { range: hunk_start..i, marks });
    }

    out
}

/// Builds one marker per line of `new_line_count` from `lines` — `None` for
/// a line that's unchanged from `HEAD`. See `hunks`, which does the actual
/// grouping; this just flattens every hunk's marks into a per-line array for
/// the editor gutter to index directly.
pub fn gutter_marks(lines: &[DiffLine], new_line_count: usize) -> Vec<Option<GutterMark>> {
    let mut marks = vec![None; new_line_count];
    for hunk in hunks(lines, new_line_count) {
        for (new_line, mark) in hunk.marks {
            marks[new_line] = Some(mark);
        }
    }
    marks
}

#[cfg(test)]
#[path = "tests/diff.rs"]
mod tests;
