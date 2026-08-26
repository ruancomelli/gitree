//! Thin typed wrapper around `git` invocations.
//!
//! Every method shells out to the real `git` binary, captures stdout/stderr,
//! and returns parsed or trimmed data.  This avoids fragile git crate bindings
//! and keeps gitree a thin ergonomic layer.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::{GitreeError, Result};

/// A handle for running git commands in a specific directory.
#[derive(Debug, Clone)]
pub struct Git {
    /// Working directory for git commands.
    cwd: PathBuf,
}

static VERBOSE: AtomicBool = AtomicBool::new(false);

/// Sets the global verbose flag (shows underlying git commands).
pub fn set_verbose(enabled: bool) {
    VERBOSE.store(enabled, Ordering::Relaxed);
}

impl Git {
    /// Creates a [`Git`] that runs commands in `cwd`.
    #[must_use]
    pub fn new<P: Into<PathBuf>>(cwd: P) -> Self {
        Self { cwd: cwd.into() }
    }

    /// Creates a [`Git`] that runs commands in the current directory.
    #[must_use]
    pub fn cwd() -> Self {
        Self::new(PathBuf::from("."))
    }

    /// Returns the installed git version string (e.g. `git version 2.43.0`).
    ///
    /// # Errors
    ///
    /// Returns [`GitreeError::GitNotFound`] if git is not on PATH, or
    /// [`GitreeError::GitFailed`] if the command exits non-zero.
    pub fn version(&self) -> Result<String> {
        self.run(&["--version"])
    }

    // -----------------------------------------------------------------------
    // Low-level runner
    // -----------------------------------------------------------------------

    /// Runs a git command, returning trimmed stdout on success.
    ///
    /// # Errors
    ///
    /// Returns [`GitreeError::GitNotFound`] if git is not on PATH, or
    /// [`GitreeError::GitFailed`] if git exits non-zero.
    fn run(&self, args: &[&str]) -> Result<String> {
        self.run_with(args, &[])
    }

    /// Runs a git command with extra env vars, returning trimmed stdout.
    fn run_with(&self, args: &[&str], env: &[(&str, &str)]) -> Result<String> {
        let output = self.exec(args, env)?;
        Self::checked(output, &format!("git {}", args.join(" ")))
    }

    /// Spawns the git command, applying `env` overrides and verbose logging.
    ///
    /// Spawn failures map to [`GitreeError::GitNotFound`] /
    /// [`GitreeError::Io`]; the caller inspects the returned raw output.
    fn exec(&self, args: &[&str], env: &[(&str, &str)]) -> Result<Output> {
        let mut cmd = self.command(args);
        for (key, val) in env {
            cmd.env(key, val);
        }

        if VERBOSE.load(Ordering::Relaxed) {
            eprintln!("git {}", args.join(" "));
        }

        cmd.output().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                GitreeError::GitNotFound
            } else {
                GitreeError::Io(e)
            }
        })
    }

    /// Turns raw command output into trimmed stdout, or a
    /// [`GitreeError::GitFailed`] carrying git's stderr.
    fn checked(output: Output, summary: &str) -> Result<String> {
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(GitreeError::GitFailed {
                summary: summary.to_string(),
                stderr,
            });
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Builds a [`Command`] for git with the given args.
    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new("git");
        cmd.current_dir(&self.cwd);
        cmd.args(args);
        cmd
    }

    // -----------------------------------------------------------------------
    // Repository introspection
    // -----------------------------------------------------------------------

    /// Returns the path to the common git directory (`$GIT_COMMON_DIR`).
    ///
    /// # Errors
    ///
    /// Returns an error if git is unavailable or the path is not inside a
    /// repository.
    pub fn common_dir(&self) -> Result<PathBuf> {
        let out = self.run(&["rev-parse", "--git-common-dir"])?;
        Ok(PathBuf::from(out))
    }

    /// Returns `true` if the working tree has uncommitted changes.
    ///
    /// # Errors
    ///
    /// Returns an error if git fails.
    pub fn is_dirty(&self) -> Result<bool> {
        Ok(self.dirty_count()? > 0)
    }

    /// Counts uncommitted changes (staged + unstaged, one per line).
    ///
    /// # Errors
    ///
    /// Returns an error if git fails.
    pub fn dirty_count(&self) -> Result<usize> {
        let out = self.run(&["status", "--porcelain"])?;
        Ok(out.lines().count())
    }

    // -----------------------------------------------------------------------
    // Branch operations
    // -----------------------------------------------------------------------

    /// Returns all local branch names (without the `refs/heads/` prefix).
    ///
    /// # Errors
    ///
    /// Returns an error if git fails.
    pub fn local_branches(&self) -> Result<Vec<String>> {
        let out = self.run(&["branch", "--list", "--format=%(refname:short)"])?;
        Ok(out.lines().map(String::from).collect())
    }

    /// Returns all remote branch names (without the `refs/remotes/origin/`
    /// prefix), excluding `HEAD` and the `origin/HEAD` symref.
    ///
    /// # Errors
    ///
    /// Returns an error if git fails.
    pub fn remote_branches(&self) -> Result<Vec<String>> {
        let out = self.run(&["branch", "--remote", "--list", "--format=%(refname:short)"])?;
        Ok(out
            .lines()
            // The short name of `refs/remotes/origin/HEAD` is `origin`;
            // skip it before stripping so a real `origin/origin` branch
            // would survive.
            .filter(|l| *l != "origin")
            .map(|l| {
                l.strip_prefix("origin/")
                    .map(String::from)
                    .unwrap_or_else(|| l.to_string())
            })
            .filter(|l| l != "HEAD")
            .collect())
    }

    /// Returns a snapshot of local and remote branch names.
    ///
    /// Use this when both sets are needed; it issues one `git branch`
    /// invocation per set.
    ///
    /// # Errors
    ///
    /// Returns an error if git fails.
    pub fn branches(&self) -> Result<BranchSet> {
        Ok(BranchSet {
            local: self.local_branches()?,
            remote: self.remote_branches()?,
        })
    }

    /// Returns the list of local branches that have no corresponding remote
    /// branch on origin.
    ///
    /// # Errors
    ///
    /// Returns an error if git fails.
    pub fn local_only_branches(&self) -> Result<Vec<String>> {
        let branches = self.branches()?;
        let remote: HashSet<&str> = branches.remote.iter().map(String::as_str).collect();
        Ok(branches
            .local
            .into_iter()
            .filter(|b| !remote.contains(b.as_str()))
            .collect())
    }

    /// Returns ahead/behind counts relative to `origin/<branch>`.
    ///
    /// Returns `(ahead, behind)` or `(0, 0)` if the upstream is not
    /// configured.
    ///
    /// # Errors
    ///
    /// Returns an error if git fails.
    pub fn ahead_behind(&self, branch: &str) -> Result<(usize, usize)> {
        let rev_list = self.run(&[
            "rev-list",
            "--left-right",
            "--count",
            &format!("origin/{branch}...{branch}"),
        ])?;
        let parts: Vec<&str> = rev_list.split_whitespace().collect();
        if parts.len() == 2 {
            let ahead = parts[1].parse().unwrap_or(0);
            let behind = parts[0].parse().unwrap_or(0);
            Ok((ahead, behind))
        } else {
            Ok((0, 0))
        }
    }

    // -----------------------------------------------------------------------
    // Worktree operations
    // -----------------------------------------------------------------------

    /// Runs `git worktree add`.
    ///
    /// # Errors
    ///
    /// Returns an error if git fails.
    pub fn worktree_add(
        &self,
        path: &Path,
        branch: &str,
        new: bool,
        base: Option<&str>,
    ) -> Result<()> {
        let path_str = path.to_string_lossy();
        let mut args: Vec<&str> = vec!["worktree", "add"];
        if new {
            args.push("-b");
            args.push(branch);
        }
        args.push(&path_str);
        if new {
            if let Some(b) = base {
                args.push(b);
            }
        } else {
            args.push(branch);
        }
        self.run(&args)?;
        Ok(())
    }

    /// Removes a worktree at the given path.
    ///
    /// # Errors
    ///
    /// Returns an error if git fails.
    pub fn worktree_remove(&self, path: &Path, force: bool) -> Result<()> {
        let path_str = path.to_string_lossy();
        let mut args: Vec<&str> = vec!["worktree", "remove"];
        if force {
            args.push("--force");
        }
        args.push(&path_str);
        self.run(&args)?;
        Ok(())
    }

    /// Unsets a git config key. No-op if the key is not set.
    ///
    /// # Errors
    ///
    /// Returns an error if git fails for a reason other than the key being
    /// unset.
    pub fn config_unset(&self, key: &str) -> Result<()> {
        match self.config_get(key)? {
            Some(_) => self.run(&["config", "--unset", key]).map(|_| ()),
            None => Ok(()),
        }
    }

    /// Runs `git worktree move <from> <to>`.
    ///
    /// # Errors
    ///
    /// Returns an error if git fails.
    pub fn worktree_move(&self, from: &Path, to: &Path) -> Result<()> {
        let from_str = from.to_string_lossy();
        let to_str = to.to_string_lossy();
        self.run(&["worktree", "move", &from_str, &to_str])?;
        Ok(())
    }

    /// Runs `git worktree repair`, fixing stale `gitdir`/`commondir` pointers
    /// after the `.git` → `.bare` rename or a worktree relocation.
    ///
    /// # Errors
    ///
    /// Returns an error if git fails.
    pub fn worktree_repair(&self) -> Result<()> {
        self.run(&["worktree", "repair"])?;
        Ok(())
    }

    /// Runs `git worktree prune`.
    ///
    /// # Errors
    ///
    /// Returns an error if git fails.
    pub fn worktree_prune(&self) -> Result<()> {
        self.run(&["worktree", "prune"])?;
        Ok(())
    }

    /// Returns parsed `git worktree list --porcelain` output.
    ///
    /// # Errors
    ///
    /// Returns an error if git fails.
    pub fn worktree_list(&self) -> Result<Vec<WorktreeEntry>> {
        let out = self.run(&["worktree", "list", "--porcelain"])?;
        Ok(WorktreeEntry::parse(&out))
    }

    // -----------------------------------------------------------------------
    // Fetch / merge / config
    // -----------------------------------------------------------------------

    /// Runs `git fetch origin`.
    ///
    /// # Errors
    ///
    /// Returns an error if git fails.
    pub fn fetch(&self) -> Result<()> {
        self.run(&["fetch", "origin"])?;
        Ok(())
    }

    /// Runs `git fetch --prune origin`.
    ///
    /// # Errors
    ///
    /// Returns an error if git fails.
    pub fn run_fetch_prune(&self) -> Result<()> {
        self.run(&["fetch", "--prune", "origin"])?;
        Ok(())
    }

    /// Runs `git merge --ff-only <ref>` in the current directory.
    ///
    /// When `autostash` is `true`, passes `--autostash` so that uncommitted
    /// changes are stashed before the merge and popped afterwards.
    ///
    /// # Errors
    ///
    /// Returns an error if git fails.
    pub fn merge_ff_only(&self, refspec: &str, autostash: bool) -> Result<()> {
        let mut args: Vec<&str> = vec!["merge", "--ff-only"];
        if autostash {
            args.push("--autostash");
        }
        args.push(refspec);
        self.run(&args)?;
        Ok(())
    }

    /// Sets a git config value.
    ///
    /// # Errors
    ///
    /// Returns an error if git fails.
    pub fn config_set(&self, key: &str, value: &str) -> Result<()> {
        self.run(&["config", key, value])?;
        Ok(())
    }

    /// Gets a git config value, or `None` if the key is unset.
    ///
    /// Returns an error only for genuine git failures (e.g. corrupt repo),
    /// not for a simply-unset key (`git config --get` exits 1 silently).
    ///
    /// # Errors
    ///
    /// Returns an error if git fails for any reason other than the key being
    /// unset.
    pub fn config_get(&self, key: &str) -> Result<Option<String>> {
        let args = ["config", "--get", key];
        let output = self.exec(&args, &[])?;
        let stderr = String::from_utf8_lossy(&output.stderr);
        match output.status.code() {
            Some(0) => Ok(Some(
                String::from_utf8_lossy(&output.stdout).trim().to_string(),
            )),
            // Exit 1 with empty stderr is git's "key not found" convention;
            // anything else is a genuine failure that must not be swallowed.
            Some(1) if stderr.trim().is_empty() => Ok(None),
            _ => Err(GitreeError::GitFailed {
                summary: format!("git {}", args.join(" ")),
                stderr: stderr.trim().to_string(),
            }),
        }
    }

    /// Runs `git fsck --full`.
    ///
    /// # Errors
    ///
    /// Returns an error if git fails.
    pub fn fsck(&self) -> Result<()> {
        self.run(&["fsck", "--full"])?;
        Ok(())
    }

    /// Runs `git stash list` and returns the count.
    ///
    /// # Errors
    ///
    /// Returns an error if git fails.
    pub fn stash_count(&self) -> Result<usize> {
        let out = self.run(&["stash", "list"])?;
        Ok(out.lines().count())
    }

    /// Returns `git status --porcelain` output (trimmed).
    ///
    /// # Errors
    ///
    /// Returns an error if git fails.
    pub fn run_status_porcelain(&self) -> Result<String> {
        self.run(&["status", "--porcelain"])
    }

    /// Returns `git rev-parse HEAD` output (trimmed).
    ///
    /// # Errors
    ///
    /// Returns an error if git fails or HEAD is unborn.
    pub fn run_rev_parse_head(&self) -> Result<String> {
        self.run(&["rev-parse", "HEAD"])
    }

    /// Deletes a local branch.
    ///
    /// # Errors
    ///
    /// Returns an error if git fails.
    pub fn branch_delete(&self, name: &str, force: bool) -> Result<()> {
        let flag = if force { "-D" } else { "-d" };
        self.run(&["branch", flag, name])?;
        Ok(())
    }

    /// Clones a repository as bare into the given path.
    ///
    /// # Errors
    ///
    /// Returns an error if git fails.
    pub fn clone_bare(url: &str, path: &Path) -> Result<()> {
        let path_str = path.to_string_lossy();
        let cwd = Git::cwd();
        let output = cwd.exec(&["clone", "--bare", url, &path_str], &[])?;
        Git::checked(output, &format!("git clone --bare {url}"))?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// WorktreeEntry — parsed `git worktree list --porcelain`
// ---------------------------------------------------------------------------

/// A snapshot of local and remote branch names.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BranchSet {
    /// Local branches (without the `refs/heads/` prefix).
    pub local: Vec<String>,
    /// Remote-tracking branches from `origin` (without the `origin/` prefix).
    pub remote: Vec<String>,
}

/// A single entry from `git worktree list --porcelain`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeEntry {
    /// The filesystem path of the worktree.
    pub path: PathBuf,
    /// The HEAD commit hash.
    pub head: Option<String>,
    /// The branch name (without `refs/heads/`).
    pub branch: Option<String>,
    /// Whether this worktree is bare.
    pub bare: bool,
    /// Whether this worktree is locked.
    pub locked: bool,
}

impl WorktreeEntry {
    /// Parses `git worktree list --porcelain` output.
    #[must_use]
    pub fn parse(output: &str) -> Vec<Self> {
        let mut entries = Vec::new();
        let mut current: Option<Self> = None;

        for line in output.lines() {
            if line.is_empty() {
                if let Some(entry) = current.take() {
                    entries.push(entry);
                }
                continue;
            }
            let (key, value) = line.split_once(' ').unwrap_or((line, ""));
            match key {
                "worktree" => {
                    if let Some(entry) = current.take() {
                        entries.push(entry);
                    }
                    current = Some(Self {
                        path: PathBuf::from(value),
                        head: None,
                        branch: None,
                        bare: false,
                        locked: false,
                    });
                }
                "HEAD" => {
                    if let Some(ref mut entry) = current {
                        entry.head = Some(value.to_string());
                    }
                }
                "branch" => {
                    if let Some(ref mut entry) = current {
                        entry.branch = value
                            .strip_prefix("refs/heads/")
                            .map(String::from)
                            .or_else(|| Some(value.to_string()));
                    }
                }
                "bare" => {
                    if let Some(ref mut entry) = current {
                        entry.bare = true;
                    }
                }
                "locked" => {
                    if let Some(ref mut entry) = current {
                        entry.locked = true;
                    }
                }
                _ => {}
            }
        }
        if let Some(entry) = current {
            entries.push(entry);
        }
        entries
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_get_set_unset_and_errors() {
        let tmp = tempfile::TempDir::new().unwrap();
        run_git(tmp.path(), &["init"]);
        run_git(tmp.path(), &["config", "user.email", "t@example.com"]);
        let git = Git::new(tmp.path());

        assert_eq!(
            git.config_get("user.email").unwrap().as_deref(),
            Some("t@example.com")
        );
        // A distinctive key name avoids collisions with any global config.
        assert_eq!(git.config_get("gitree.test.absent-key").unwrap(), None);
    }

    #[test]
    fn config_get_corrupt_gitfile_is_error() {
        // A malformed `.git` file makes every git invocation fail with a
        // fatal error — this must propagate, not be reported as "unset".
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join(".git"), b"not a gitfile").unwrap();
        let git = Git::new(tmp.path());
        assert!(matches!(
            git.config_get("core.bare"),
            Err(GitreeError::GitFailed { .. })
        ));
    }

    /// Runs a git command, failing the test on non-zero exit.
    fn run_git(dir: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .current_dir(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn parse_single_worktree() {
        let input = "worktree /home/user/project/main\nHEAD abc123\nbranch refs/heads/main\n\n";
        let entries = WorktreeEntry::parse(input);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, PathBuf::from("/home/user/project/main"));
        assert_eq!(entries[0].head.as_deref(), Some("abc123"));
        assert_eq!(entries[0].branch.as_deref(), Some("main"));
        assert!(!entries[0].bare);
    }

    #[test]
    fn parse_bare_worktree() {
        let input = "worktree /home/user/project/.bare\nbare\n\n";
        let entries = WorktreeEntry::parse(input);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].bare);
        assert!(entries[0].branch.is_none());
    }

    #[test]
    fn parse_multiple_worktrees() {
        let input = "worktree /home/user/project/main\nHEAD abc123\nbranch refs/heads/main\n\nworktree /home/user/project/feature\nHEAD def456\nbranch refs/heads/feature\n\n";
        let entries = WorktreeEntry::parse(input);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].branch.as_deref(), Some("main"));
        assert_eq!(entries[1].branch.as_deref(), Some("feature"));
    }

    #[test]
    fn parse_no_trailing_newline() {
        let input = "worktree /home/user/project/main\nHEAD abc123\nbranch refs/heads/main";
        let entries = WorktreeEntry::parse(input);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].branch.as_deref(), Some("main"));
    }
}
