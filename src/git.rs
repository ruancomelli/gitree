//! Thin typed wrapper around `git` invocations.
//!
//! Every method shells out to the real `git` binary, captures stdout/stderr,
//! and returns parsed or trimmed data.  This avoids fragile git crate bindings
//! and keeps gitree a thin ergonomic layer.

use std::path::{Path, PathBuf};
use std::process::Command;
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
        let mut cmd = self.command(args);
        for (key, val) in env {
            cmd.env(key, val);
        }

        if VERBOSE.load(Ordering::Relaxed) {
            eprintln!("git {}", args.join(" "));
        }

        let output = cmd.output().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                GitreeError::GitNotFound
            } else {
                GitreeError::Io(e)
            }
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let summary = format!("git {}", args.join(" "));
            return Err(GitreeError::GitFailed { summary, stderr });
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

    /// Returns the path to the git directory of the current worktree
    /// (`$GIT_DIR`).
    ///
    /// # Errors
    ///
    /// Returns an error if git is unavailable or the path is not inside a
    /// repository.
    pub fn git_dir(&self) -> Result<PathBuf> {
        let out = self.run(&["rev-parse", "--git-dir"])?;
        Ok(PathBuf::from(out))
    }

    /// Returns the current branch name (short form).
    ///
    /// # Errors
    ///
    /// Returns an error if HEAD is detached or git fails.
    #[allow(dead_code)]
    pub fn current_branch(&self) -> Result<String> {
        self.run(&["rev-parse", "--abbrev-ref", "HEAD"])
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
    /// prefix), excluding `HEAD`.
    ///
    /// # Errors
    ///
    /// Returns an error if git fails.
    pub fn remote_branches(&self) -> Result<Vec<String>> {
        let out = self.run(&["branch", "--remote", "--list", "--format=%(refname:short)"])?;
        Ok(out
            .lines()
            .map(|l| {
                l.strip_prefix("origin/")
                    .map(String::from)
                    .unwrap_or_else(|| l.to_string())
            })
            .filter(|l| l != "HEAD")
            .collect())
    }

    /// Returns `true` if a local branch with the given name exists.
    ///
    /// # Errors
    ///
    /// Returns an error if git fails.
    pub fn has_local_branch(&self, name: &str) -> Result<bool> {
        Ok(self.local_branches()?.iter().any(|b| b == name))
    }

    /// Returns `true` if a remote branch with the given name exists on
    /// `origin`.
    ///
    /// # Errors
    ///
    /// Returns an error if git fails.
    pub fn has_remote_branch(&self, name: &str) -> Result<bool> {
        Ok(self.remote_branches()?.iter().any(|b| b == name))
    }

    /// Returns the list of local branches that have no corresponding remote
    /// branch on origin.
    ///
    /// # Errors
    ///
    /// Returns an error if git fails.
    pub fn local_only_branches(&self) -> Result<Vec<String>> {
        let local = self.local_branches()?;
        let remote = self.remote_branches()?;
        Ok(local
            .iter()
            .filter(|b| !remote.iter().any(|r| r == *b))
            .cloned()
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
    /// # Errors
    ///
    /// Returns an error if git fails.
    pub fn merge_ff_only(&self, refspec: &str) -> Result<()> {
        self.run(&["merge", "--ff-only", refspec])?;
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
    /// not for a simply-unset key (exit code 1).
    ///
    /// # Errors
    ///
    /// Returns an error if git fails for a reason other than the key being
    /// unset.
    #[allow(dead_code)]
    pub fn config_get(&self, key: &str) -> Result<Option<String>> {
        match self.run(&["config", "--get", key]) {
            Ok(val) => Ok(Some(val)),
            Err(GitreeError::GitFailed { stderr, .. }) if stderr.is_empty() => Ok(None),
            Err(e) => Err(e),
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
        let mut cmd = cwd.command(&["clone", "--bare", url, &path_str]);
        if VERBOSE.load(Ordering::Relaxed) {
            eprintln!("git clone --bare {url} {path_str}");
        }
        let output = cmd.output().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                GitreeError::GitNotFound
            } else {
                GitreeError::Io(e)
            }
        })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(GitreeError::GitFailed {
                summary: format!("git clone --bare {url}"),
                stderr,
            });
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// WorktreeEntry — parsed `git worktree list --porcelain`
// ---------------------------------------------------------------------------

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
