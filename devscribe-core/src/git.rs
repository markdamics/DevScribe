//! Minimal `gix`-backed read access to the repository: current branch name,
//! a file's content at `HEAD` (for the diff panel), and a coarse working-tree
//! status scan (for the sidebar's Changes panel). No staging or discarding —
//! those are real features that deserve their own pass, not a bolt-on here.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub struct Repo {
    inner: gix::Repository,
    root: PathBuf,
}

/// A coarse per-file working-tree status, matching what the sidebar needs to
/// show as a single badge letter. Doesn't distinguish staged vs. unstaged —
/// `git status --short`'s XY columns collapse to one letter here too, since
/// nothing in the UI acts on the index (no stage/unstage yet).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Modified,
    Added,
    Untracked,
    Deleted,
}

pub struct ChangedFile {
    pub path: PathBuf,
    pub kind: ChangeKind,
}

impl Repo {
    /// Opens `root` as a git repository. Returns `None` if it isn't one (or
    /// can't be opened) — that's an expected, non-error state for any
    /// project that isn't version-controlled.
    pub fn open(root: &Path) -> Option<Self> {
        let inner = gix::open(root).ok()?;
        Some(Self {
            inner,
            root: root.to_path_buf(),
        })
    }

    /// The current branch's short name, or a short commit hash on a detached
    /// `HEAD`. `None` on a repository with no commits yet.
    pub fn branch_name(&self) -> Option<String> {
        if let Ok(Some(name)) = self.inner.head_name() {
            return Some(name.shorten().to_string());
        }
        let commit = self.inner.head_commit().ok()?;
        Some(commit.id().to_string().chars().take(7).collect())
    }

    /// `absolute_path`'s content as it was at `HEAD`. `None` if the path is
    /// outside the repo, untracked, new (no `HEAD` entry yet), or not valid
    /// UTF-8.
    pub fn head_text(&self, absolute_path: &Path) -> Option<String> {
        let relative = absolute_path.strip_prefix(&self.root).ok()?;
        let commit = self.inner.head_commit().ok()?;
        let tree = commit.tree().ok()?;
        let entry = tree.lookup_entry_by_path(relative).ok()??;
        let object = entry.object().ok()?;
        String::from_utf8(object.data.clone()).ok()
    }

    /// A coarse, one-badge-per-file working-tree status scan: `HEAD` vs. the
    /// index (staged changes) and the index vs. the worktree (unstaged
    /// changes + untracked files), merged into one `ChangeKind` per path.
    ///
    /// Deliberately skips `gix`'s rename/copy tracking (left at its default
    /// of "off") — this only needs to answer "is this file different from
    /// `HEAD`, and how," not attribute a rename, so the plain
    /// addition+deletion pair a rename shows up as is an acceptable,
    /// simpler outcome. Returns an empty list (not an error) on any gix
    /// failure — the sidebar treats "no changes" and "couldn't compute
    /// changes" the same way, since there's no UI for surfacing the latter.
    pub fn changed_files(&self) -> Vec<ChangedFile> {
        let Ok(platform) = self.inner.status(gix::progress::Discard) else {
            return Vec::new();
        };
        // `Collapsed` (the default) reports a brand-new directory as a single
        // untracked entry for the directory itself, which wouldn't match any
        // individual file's path — the sidebar wants a badge per file.
        let platform = platform.untracked_files(gix::status::UntrackedFiles::Files);
        let Ok(iter) = platform.into_iter(Vec::new()) else {
            return Vec::new();
        };

        let mut kinds: BTreeMap<PathBuf, ChangeKind> = BTreeMap::new();
        for item in iter.flatten() {
            match item {
                gix::status::Item::TreeIndex(change) => {
                    use gix::diff::index::ChangeRef;
                    let (location, kind) = match &change {
                        ChangeRef::Addition { location, .. } => (location, ChangeKind::Added),
                        ChangeRef::Deletion { location, .. } => (location, ChangeKind::Deleted),
                        ChangeRef::Modification { location, .. } => (location, ChangeKind::Modified),
                        _ => continue,
                    };
                    kinds.entry(self.to_absolute(location)).or_insert(kind);
                }
                gix::status::Item::IndexWorktree(item) => {
                    use gix::status::index_worktree::Item as IwItem;
                    use gix::status::plumbing::index_as_worktree::{Change as WtChange, EntryStatus};
                    match item {
                        IwItem::Modification { rela_path, status, .. } => {
                            let kind = match status {
                                EntryStatus::Change(WtChange::Removed) => Some(ChangeKind::Deleted),
                                EntryStatus::Change(WtChange::Modification { .. } | WtChange::Type { .. }) => {
                                    Some(ChangeKind::Modified)
                                }
                                _ => None,
                            };
                            if let Some(kind) = kind {
                                // Worktree state is more current than a staged-only
                                // classification from the tree-index pass above.
                                kinds.insert(self.to_absolute(&rela_path), kind);
                            }
                        }
                        IwItem::DirectoryContents { entry, .. } => {
                            kinds
                                .entry(self.to_absolute(&entry.rela_path))
                                .or_insert(ChangeKind::Untracked);
                        }
                        IwItem::Rewrite { .. } => {}
                    }
                }
            }
        }

        kinds
            .into_iter()
            .map(|(path, kind)| ChangedFile { path, kind })
            .collect()
    }

    /// `bytes` accepts a `BString`, `Cow<BStr>`, or `BStr` by reference —
    /// anything that deref-coerces down to `&[u8]`, which every repository-
    /// relative path type `gix` hands back here does.
    fn to_absolute(&self, bytes: &[u8]) -> PathBuf {
        self.root.join(gix::path::from_bstr(gix::bstr::BStr::new(bytes)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    /// Drives the real `git` CLI to build a small fixture repo — simplest
    /// way to get a realistic index/worktree/HEAD combination without
    /// hand-assembling `gix` plumbing objects. `changed_files()` itself
    /// stays pure `gix`; only the test fixture shells out.
    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .status()
            .expect("git must be on PATH for this test");
        assert!(status.success(), "`git {args:?}` failed");
    }

    fn kind_of<'a>(files: &'a [ChangedFile], name: &str) -> Option<&'a ChangeKind> {
        files.iter().find(|f| f.path.file_name().unwrap() == name).map(|f| &f.kind)
    }

    #[test]
    fn detects_modified_added_untracked_and_deleted() {
        let dir = std::env::temp_dir().join(format!("devscribe-git-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        git(&dir, &["init", "-q"]);
        std::fs::write(dir.join("keep.txt"), "unchanged\n").unwrap();
        std::fs::write(dir.join("edit.txt"), "original\n").unwrap();
        std::fs::write(dir.join("gone.txt"), "will be deleted\n").unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "initial"]);

        std::fs::write(dir.join("edit.txt"), "changed\n").unwrap();
        std::fs::remove_file(dir.join("gone.txt")).unwrap();
        std::fs::write(dir.join("new.txt"), "brand new\n").unwrap();
        git(&dir, &["add", "edit.txt", "new.txt"]);
        std::fs::write(dir.join("also_new.txt"), "untracked\n").unwrap();

        let repo = Repo::open(&dir).expect("just-initialized dir is a repo");
        let files = repo.changed_files();

        assert_eq!(kind_of(&files, "edit.txt"), Some(&ChangeKind::Modified));
        assert_eq!(kind_of(&files, "new.txt"), Some(&ChangeKind::Added));
        assert_eq!(kind_of(&files, "also_new.txt"), Some(&ChangeKind::Untracked));
        assert_eq!(kind_of(&files, "gone.txt"), Some(&ChangeKind::Deleted));
        assert_eq!(kind_of(&files, "keep.txt"), None);

        std::fs::remove_dir_all(&dir).ok();
    }
}
