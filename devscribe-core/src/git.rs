//! Minimal `gix`-backed read access to the repository: current branch name,
//! a file's content at `HEAD` (for the diff panel), a coarse working-tree
//! status scan (for the sidebar's Changes panel), and discarding a single
//! file's working-tree changes back to `HEAD`. Still no staging — nothing
//! here reads or writes the index.
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

    /// Initializes `root` as a fresh git repository ("New project"'s
    /// git-init step). Returns whether it succeeded — the one caller
    /// (`state::start_loading_project`) treats this as best-effort and
    /// opens the folder either way, so there's no use for `gix::init`'s
    /// real error beyond that (and it's a large type not worth exposing
    /// just to be discarded).
    pub fn init(root: &Path) -> bool {
        gix::init(root).is_ok()
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

    /// Commits the current branch is ahead/behind its upstream (remote-
    /// tracking) branch by — e.g. for a sidebar `▲2 ▼0` indicator. `None`
    /// when there's no upstream to compare against (no remote configured,
    /// the branch isn't tracking one, or `HEAD` is detached) — an expected
    /// state, not an error, same treatment `branch_name()` gives a
    /// commits-free repo. Computed the standard way: the merge-base between
    /// the two tips splits history into "ahead" (reachable from local, not
    /// from the merge-base) and "behind" (reachable from upstream, not from
    /// the merge-base), each counted via `with_hidden([base])` — the
    /// traversal equivalent of `git rev-list <tip> ^<base>`.
    pub fn ahead_behind(&self) -> Option<(usize, usize)> {
        let head_name = self.inner.head_name().ok()??;
        let upstream_name = self
            .inner
            .branch_remote_tracking_ref_name(head_name.as_ref(), gix::remote::Direction::Fetch)?
            .ok()?;

        let local_id = self.inner.head_id().ok()?;
        let upstream_id = self.inner.find_reference(upstream_name.as_ref()).ok()?.into_fully_peeled_id().ok()?;

        if local_id == upstream_id {
            return Some((0, 0));
        }

        let base = self.inner.merge_base(local_id, upstream_id).ok()?.detach();

        let ahead = local_id.ancestors().with_hidden([base]).all().ok()?.filter_map(Result::ok).count();
        let behind = upstream_id.ancestors().with_hidden([base]).all().ok()?.filter_map(Result::ok).count();

        Some((ahead, behind))
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

    /// Reverts `absolute_path`'s working-tree content back to how `HEAD`
    /// (or "doesn't exist") sees it — the Changes panel's "discard changes"
    /// action. Only touches the working tree, never the index: `Modified`/
    /// `Deleted` are restored from `HEAD`'s blob (reusing `head_text`, so a
    /// binary or non-UTF-8 file's discard fails with `NotFound` rather than
    /// corrupting it); `Added`/`Untracked` have no `HEAD` version to restore
    /// to, so discarding one just deletes the working file.
    pub fn discard_file(&self, absolute_path: &Path, kind: ChangeKind) -> std::io::Result<()> {
        match kind {
            ChangeKind::Modified | ChangeKind::Deleted => {
                let content = self.head_text(absolute_path).ok_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::NotFound, "no HEAD version to restore")
                })?;
                std::fs::write(absolute_path, content)
            }
            ChangeKind::Added | ChangeKind::Untracked => std::fs::remove_file(absolute_path),
        }
    }

    /// `bytes` accepts a `BString`, `Cow<BStr>`, or `BStr` by reference —
    /// anything that deref-coerces down to `&[u8]`, which every repository-
    /// relative path type `gix` hands back here does.
    fn to_absolute(&self, bytes: &[u8]) -> PathBuf {
        self.root.join(gix::path::from_bstr(gix::bstr::BStr::new(bytes)))
    }
}

#[cfg(test)]
#[path = "tests/git.rs"]
mod tests;
