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

#[test]
fn ahead_behind_counts_diverged_commits_against_the_tracked_upstream() {
    let root = std::env::temp_dir().join(format!("devscribe-git-test-ahead-behind-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let remote = root.join("remote");
    let local = root.join("local");
    std::fs::create_dir_all(&remote).unwrap();

    // `-b main` pins the branch name explicitly rather than trusting
    // the ambient `init.defaultBranch` config, so `origin/main` below
    // is guaranteed to exist.
    git(&remote, &["init", "-q", "-b", "main"]);
    std::fs::write(remote.join("f.txt"), "c1\n").unwrap();
    git(&remote, &["add", "."]);
    git(&remote, &["commit", "-q", "-m", "c1"]);

    // A local clone of a non-bare repo works fine for read-only access
    // and sets up the `origin` remote + `main`'s upstream tracking
    // automatically — no bare repo or actual network needed, same
    // "shell out to real git for fixture setup only" approach as
    // `detects_modified_added_untracked_and_deleted` above.
    git(&root, &["clone", "-q", "remote", "local"]);

    // A second commit on the remote that `local` hasn't fetched yet —
    // makes `local` "behind" once it does fetch (updating
    // `refs/remotes/origin/main` without touching local `main`).
    std::fs::write(remote.join("f.txt"), "c2\n").unwrap();
    git(&remote, &["add", "."]);
    git(&remote, &["commit", "-q", "-m", "c2"]);
    git(&local, &["fetch", "-q", "origin"]);

    // A local-only commit — makes `local` "ahead" too, so this
    // exercises genuine divergence (both ahead and behind at once),
    // not just one direction.
    std::fs::write(local.join("g.txt"), "local only\n").unwrap();
    git(&local, &["add", "."]);
    git(&local, &["commit", "-q", "-m", "c3"]);

    let repo = Repo::open(&local).expect("cloned dir is a repo");
    assert_eq!(repo.ahead_behind(), Some((1, 1)));

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn ahead_behind_is_none_without_an_upstream() {
    let dir = std::env::temp_dir().join(format!("devscribe-git-test-no-upstream-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    git(&dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("f.txt"), "c1\n").unwrap();
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "c1"]);

    let repo = Repo::open(&dir).expect("just-initialized dir is a repo");
    assert_eq!(repo.ahead_behind(), None, "no remote was ever configured — there's nothing to compare against");

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn init_makes_a_plain_folder_openable_as_a_repo() {
    let dir = std::env::temp_dir().join(format!("devscribe-git-test-init-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    assert!(Repo::open(&dir).is_none(), "a plain folder isn't a repo yet");
    assert!(Repo::init(&dir), "init on a fresh, writable folder should succeed");
    assert!(Repo::open(&dir).is_some(), "init should make the folder openable as a repo");

    std::fs::remove_dir_all(&dir).ok();
}
