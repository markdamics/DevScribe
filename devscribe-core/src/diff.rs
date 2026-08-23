//! Line-level text diffing, independent of `git` — the diff panel feeds it
//! `HEAD`'s blob text (from `git::Repo::head_text`) against the live buffer.
use similar::{ChangeTag, TextDiff};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKind {
    Equal,
    Insert,
    Delete,
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

/// A line-by-line diff of `old` against `new`, in document order.
pub fn diff_lines(old: &str, new: &str) -> Vec<DiffLine> {
    TextDiff::from_lines(old, new)
        .iter_all_changes()
        .map(|change| {
            let kind = match change.tag() {
                ChangeTag::Equal => DiffLineKind::Equal,
                ChangeTag::Insert => DiffLineKind::Insert,
                ChangeTag::Delete => DiffLineKind::Delete,
            };
            DiffLine {
                kind,
                old_line: change.old_index(),
                new_line: change.new_index(),
                text: change.value().trim_end_matches(['\n', '\r']).to_string(),
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "tests/diff.rs"]
mod tests;
