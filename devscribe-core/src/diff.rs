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
mod tests {
    use super::*;

    #[test]
    fn identical_text_is_all_equal() {
        let lines = diff_lines("a\nb\nc\n", "a\nb\nc\n");
        assert!(lines.iter().all(|l| l.kind == DiffLineKind::Equal));
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn detects_insert_and_delete() {
        let lines = diff_lines("a\nb\nc\n", "a\nx\nc\n");
        let kinds: Vec<_> = lines.iter().map(|l| (l.kind, l.text.as_str())).collect();
        assert_eq!(
            kinds,
            vec![
                (DiffLineKind::Equal, "a"),
                (DiffLineKind::Delete, "b"),
                (DiffLineKind::Insert, "x"),
                (DiffLineKind::Equal, "c"),
            ]
        );
    }

    #[test]
    fn pure_addition_has_no_old_line() {
        let lines = diff_lines("a\n", "a\nb\n");
        let added = lines
            .iter()
            .find(|l| l.kind == DiffLineKind::Insert)
            .expect("insert present");
        assert_eq!(added.old_line, None);
        assert_eq!(added.new_line, Some(1));
    }
}
