//! `.shared/` symlink fan-out and effective-ignore checks.
//!
//! Links are created *relative* and point at the lexical
//! `.shared/<entry>` path, not at a canonicalized physical location:
//! the wrapper can be moved or copied without breaking every worktree,
//! and retargeting `.shared/<entry>` (e.g. swapping a symlinked profile)
//! reaches all worktrees at once.

use std::fs;
use std::path::Path;

use pathdiff::diff_paths;

use crate::error::{GitreeError, Result};
use crate::git::Git;
use crate::types::SharedDir;

/// Result of linking a single shared item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkResult {
    /// The symlink was created.
    Linked(String),
    /// The target already exists in the worktree.
    Skipped(String),
    /// The path is tracked in the worktree's index. This includes an
    /// unstaged deletion (the file is absent but still indexed) and
    /// sparse-checkout skips — replacing such a path with a symlink would
    /// silently change pending git state.
    SkippedTracked(String),
    /// The entry cannot be shared at all. Currently only `.gitignore`:
    /// Git deliberately does not follow symlinked `.gitignore` files, so
    /// sharing it would silently disable the linked rules.
    Unshareable(String),
}

/// Links all items from `.shared/` into `worktree_path`.
///
/// `git` (rooted at the worktree) is consulted to avoid replacing paths
/// tracked in the worktree's index; `None` skips that check (used by
/// tests on non-git directories).
///
/// # Errors
///
/// Returns an error if the `.shared/` directory cannot be read or a
/// symlink cannot be created.
pub fn link_shared(
    shared: &SharedDir,
    worktree_path: &Path,
    git: Option<&Git>,
) -> Result<Vec<LinkResult>> {
    if !shared.as_path().is_dir() {
        return Ok(Vec::new());
    }

    let mut results = Vec::new();
    let entries = fs::read_dir(shared.as_path())?;

    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy().into_owned();

        if name_str == ".gitignore" {
            results.push(LinkResult::Unshareable(name_str));
            continue;
        }

        let target = worktree_path.join(&name);
        if target.exists() || target.is_symlink() {
            results.push(LinkResult::Skipped(name_str));
            continue;
        }

        if git.is_some_and(|g| g.path_tracked(&name_str).unwrap_or(false)) {
            results.push(LinkResult::SkippedTracked(name_str));
            continue;
        }

        // Link the lexical `.shared/<name>` path, relative to the
        // worktree. Never canonicalize the source: the link must follow
        // the shared entry's identity, not its physical location.
        let source = shared.as_path().join(&name);
        let link_target = diff_paths(&source, worktree_path).unwrap_or_else(|| source.clone());
        symlink(&link_target, &target)?;
        results.push(LinkResult::Linked(name_str));
    }

    Ok(results)
}

/// Returns the subset of `names` that is *not* effectively ignored when
/// viewed from the worktree `git` is rooted at.
///
/// Ignore sources are the worktree's `.gitignore` files,
/// `.git/info/exclude`, and `core.excludesFile` — the wrapper-level
/// `.gitignore` is invisible from inside a worktree.
///
/// Failures of the underlying `check-ignore` are treated as "ignored" so
/// a broken check never produces spurious warnings.
///
/// # Errors
///
/// Returns an error if git fails for a reason other than "not ignored".
pub fn check_shared_ignored(git: &Git, names: &[String]) -> Result<Vec<String>> {
    let mut unignored = Vec::new();
    for name in names {
        if !git.check_ignore(name).unwrap_or(true) {
            unignored.push(name.clone());
        }
    }
    Ok(unignored)
}

/// Creates a symlink, working on both Unix and Windows.
#[cfg(unix)]
fn symlink(source: &Path, target: &Path) -> Result<()> {
    std::os::unix::fs::symlink(source, target).map_err(GitreeError::from)
}

#[cfg(windows)]
fn symlink(source: &Path, target: &Path) -> Result<()> {
    if source.is_dir() {
        std::os::windows::fs::symlink_dir(source, target).map_err(GitreeError::from)
    } else {
        std::os::windows::fs::symlink_file(source, target).map_err(GitreeError::from)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Runs a git command, failing the test on error, with an explicit
    /// test identity.
    fn run_git(dir: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .current_dir(dir)
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@test.com")
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@test.com")
            .env("GPG_TTY", "")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn link_shared_creates_relative_symlinks() {
        let tmp = TempDir::new().unwrap();
        let shared_dir = tmp.path().join(".shared");
        fs::create_dir(&shared_dir).unwrap();
        fs::write(shared_dir.join(".env"), "FOO=bar\n").unwrap();
        fs::create_dir(shared_dir.join(".vscode")).unwrap();

        let worktree = tmp.path().join("main");
        fs::create_dir(&worktree).unwrap();

        let shared = SharedDir::from_path(&shared_dir);
        let results = link_shared(&shared, &worktree, None).unwrap();

        assert_eq!(results.len(), 2);
        assert!(
            results
                .iter()
                .any(|r| matches!(r, LinkResult::Linked(n) if n == ".env"))
        );
        assert!(
            results
                .iter()
                .any(|r| matches!(r, LinkResult::Linked(n) if n == ".vscode"))
        );

        let env_link = fs::read_link(worktree.join(".env")).unwrap();
        assert_eq!(env_link, std::path::PathBuf::from("../.shared/.env"));
        assert!(worktree.join(".env").is_symlink());
        assert!(worktree.join(".vscode").is_symlink());
    }

    #[test]
    fn link_shared_skips_existing() {
        let tmp = TempDir::new().unwrap();
        let shared_dir = tmp.path().join(".shared");
        fs::create_dir(&shared_dir).unwrap();
        fs::write(shared_dir.join(".env"), "FOO=bar\n").unwrap();

        let worktree = tmp.path().join("main");
        fs::create_dir(&worktree).unwrap();
        fs::write(worktree.join(".env"), "EXISTING\n").unwrap();

        let shared = SharedDir::from_path(&shared_dir);
        let results = link_shared(&shared, &worktree, None).unwrap();

        assert_eq!(results.len(), 1);
        assert!(
            results
                .iter()
                .all(|r| matches!(r, LinkResult::Skipped(n) if n == ".env"))
        );
        assert_eq!(
            fs::read_to_string(worktree.join(".env")).unwrap(),
            "EXISTING\n"
        );
    }

    #[test]
    fn link_shared_refuses_gitignore() {
        let tmp = TempDir::new().unwrap();
        let shared_dir = tmp.path().join(".shared");
        fs::create_dir(&shared_dir).unwrap();
        fs::write(shared_dir.join(".gitignore"), "node_modules\n").unwrap();

        let worktree = tmp.path().join("main");
        fs::create_dir(&worktree).unwrap();

        let shared = SharedDir::from_path(&shared_dir);
        let results = link_shared(&shared, &worktree, None).unwrap();

        assert_eq!(results, vec![LinkResult::Unshareable(".gitignore".into())]);
        assert!(!worktree.join(".gitignore").exists());
    }

    #[test]
    fn link_shared_skips_tracked_deletion() {
        // A tracked file deleted from the working tree (unstaged) must
        // not be replaced by a symlink: that would convert a pending
        // deletion into a type change.
        let tmp = TempDir::new().unwrap();
        let shared_dir = tmp.path().join(".shared");
        fs::create_dir(&shared_dir).unwrap();
        fs::write(shared_dir.join(".env"), "FOO=bar\n").unwrap();

        let worktree = tmp.path().join("main");
        fs::create_dir(&worktree).unwrap();
        run_git(&worktree, &["init", "--initial-branch=main"]);
        run_git(&worktree, &["config", "user.email", "t@example.com"]);
        run_git(&worktree, &["config", "user.name", "Test"]);
        fs::write(worktree.join(".env"), "TRACKED\n").unwrap();
        run_git(&worktree, &["add", "."]);
        run_git(&worktree, &["commit", "-m", "initial"]);
        fs::remove_file(worktree.join(".env")).unwrap();

        let git = Git::new(&worktree);
        assert!(git.path_tracked(".env").unwrap());

        let shared = SharedDir::from_path(&shared_dir);
        let results = link_shared(&shared, &worktree, Some(&git)).unwrap();

        assert_eq!(results.len(), 1);
        assert!(
            results
                .iter()
                .all(|r| matches!(r, LinkResult::SkippedTracked(n) if n == ".env"))
        );
        assert!(!worktree.join(".env").exists());
    }

    #[test]
    fn link_shared_no_dir() {
        let tmp = TempDir::new().unwrap();
        let shared = SharedDir::from_path(tmp.path().join(".shared"));
        let results = link_shared(&shared, tmp.path(), None).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn check_shared_ignored_reports_unignored_names() {
        let tmp = TempDir::new().unwrap();
        let wt = tmp.path().join("wt");
        fs::create_dir(&wt).unwrap();
        run_git(&wt, &["init", "--initial-branch=main"]);
        fs::write(wt.join(".gitignore"), "secret.env\n").unwrap();

        let git = Git::new(&wt);
        let unignored = check_shared_ignored(&git, &["secret.env".into(), ".env".into()]).unwrap();
        assert_eq!(unignored, vec![".env".to_string()]);
    }
}
