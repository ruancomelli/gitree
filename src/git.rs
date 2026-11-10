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
    pub(crate) fn run(&self, args: &[&str]) -> Result<String> {
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
    /// Given `origin/feature/x` and the `origin/HEAD` symref, this yields
    /// `["feature/x"]`: the symref's short name `HEAD` is filtered out below.
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
    /// branch on `origin`.
    ///
    /// Note: the absence of a same-name `origin/<branch>` counterpart does
    /// **not** mean the branch is stale — it may simply never have been
    /// pushed. Use [`Git::upstream_states`] when "the configured upstream
    /// was deleted" is the question.
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

    /// Returns the upstream tracking state of every local branch.
    ///
    /// `gone` is `true` when the branch has a configured upstream whose
    /// remote-tracking ref no longer exists (typically because the branch
    /// was deleted on the remote).
    ///
    /// # Errors
    ///
    /// Returns an error if git fails.
    pub fn upstream_states(&self) -> Result<Vec<UpstreamState>> {
        let out = self.run(&[
            "for-each-ref",
            "--format=%(refname:short)%00%(upstream:short)%00%(upstream:track)",
            "refs/heads",
        ])?;
        Ok(out
            .lines()
            .filter_map(|line| {
                let mut parts = line.split('\0');
                let name = parts.next()?;
                if name.is_empty() {
                    return None;
                }
                let upstream = parts.next().unwrap_or_default();
                let track = parts.next().unwrap_or_default();
                Some(UpstreamState {
                    name: name.to_string(),
                    upstream: (!upstream.is_empty()).then(|| upstream.to_string()),
                    gone: track.contains("gone"),
                })
            })
            .collect())
    }

    /// Returns ahead/behind counts relative to `origin/<branch>`.
    ///
    /// Fully qualified refs are used so that tags or local branches named
    /// like `origin/<branch>` cannot shadow the tracking ref during
    /// revision resolution.
    ///
    /// Returns `None` when the branch has no upstream on `origin`, and the
    /// `(ahead, behind)` counts otherwise.
    ///
    /// # Errors
    ///
    /// Returns an error if git fails.
    pub fn ahead_behind(&self, branch: &str) -> Result<Option<(usize, usize)>> {
        // A quiet existence probe separates "no upstream" (below) from
        // genuine failures in the rev-list call; rev-list would otherwise
        // fail ambiguously for both cases.
        let remote_ref = format!("refs/remotes/origin/{branch}");
        let local_ref = format!("refs/heads/{branch}");
        if self
            .run(&["rev-parse", "--verify", "--quiet", &remote_ref])
            .is_err()
        {
            return Ok(None);
        }

        let rev_list = self.run(&[
            "rev-list",
            "--left-right",
            "--count",
            &format!("{remote_ref}...{local_ref}"),
        ])?;
        let mut parts = rev_list.split_whitespace();
        // Left side is `origin/<branch>`: commits we are behind; right side
        // is `<branch>`: commits we are ahead.
        let behind = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        let ahead = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        Ok(Some((ahead, behind)))
    }

    /// Returns `true` if the branch is an ancestor of HEAD (i.e. fully
    /// merged into the current default branch).
    ///
    /// # Errors
    ///
    /// Returns an error if git fails for a reason other than "not an
    /// ancestor" (which is reported as `false`).
    pub fn branch_merged_into_head(&self, branch: &str) -> Result<bool> {
        let output = self.exec(&["merge-base", "--is-ancestor", branch, "HEAD"], &[])?;
        match output.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(GitreeError::GitFailed {
                summary: format!("git merge-base --is-ancestor {branch} HEAD"),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            }),
        }
    }

    // -----------------------------------------------------------------------
    // Worktree operations
    // -----------------------------------------------------------------------

    /// Runs `git worktree add`.
    ///
    /// `new_branch` (if given) is created with `-b`; `start_point` is the
    /// commit-ish checked out into the new worktree. Passing a fully
    /// qualified branch ref (e.g. `refs/heads/main`) as `start_point` avoids
    /// tag shadowing during revision resolution.
    ///
    /// # Errors
    ///
    /// Returns an error if git fails.
    pub fn worktree_add(
        &self,
        path: &Path,
        new_branch: Option<&str>,
        start_point: Option<&str>,
    ) -> Result<()> {
        let path_str = path.to_string_lossy();
        let mut args: Vec<&str> = vec!["worktree", "add"];
        if let Some(branch) = new_branch {
            args.push("-b");
            args.push(branch);
        }
        args.push(&path_str);
        if let Some(sp) = start_point {
            args.push(sp);
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
    /// `branch` names the branch being updated. Its
    /// `branch.<name>.mergeOptions` config is explicitly cleared for this
    /// invocation: an inherited `--squash` would otherwise let the merge
    /// exit successfully while leaving HEAD untouched.
    ///
    /// The returned [`MergeResult`] carries the HEAD before/after and the
    /// stderr captured on success, so callers can detect a fast-forward
    /// whose autostash restoration conflicted (git exits zero in that case
    /// while keeping the stash).
    ///
    /// # Errors
    ///
    /// Returns an error if git fails.
    pub fn merge_ff_only(
        &self,
        branch: &str,
        refspec: &str,
        autostash: bool,
    ) -> Result<MergeResult> {
        let head_before = self.run_rev_parse_head().ok();
        let mut args: Vec<String> = vec![
            "-c".into(),
            format!("branch.{branch}.mergeOptions="),
            "merge".into(),
            "--ff-only".into(),
        ];
        if autostash {
            args.push("--autostash".into());
        }
        args.push(refspec.into());
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let output = self.exec(&arg_refs, &[])?;
        if !output.status.success() {
            return Err(GitreeError::GitFailed {
                summary: format!("git merge --ff-only {refspec}"),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }
        let head_after = self.run_rev_parse_head().ok();
        Ok(MergeResult {
            head_before,
            head_after,
            notes: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        })
    }

    /// Returns `true` if the worktree's HEAD is a symbolic ref to
    /// `branch`, re-read from the worktree itself.
    ///
    /// Used to revalidate a worktree identity immediately before a
    /// mutating operation: the entry from an earlier `worktree_list()`
    /// snapshot may be stale if the user switched branches meanwhile.
    ///
    /// # Errors
    ///
    /// Returns an error if git fails.
    pub fn current_branch(&self) -> Result<Option<String>> {
        let output = self.exec(&["symbolic-ref", "--short", "-q", "HEAD"], &[])?;
        match output.status.code() {
            Some(0) => Ok(Some(
                String::from_utf8_lossy(&output.stdout).trim().to_string(),
            )),
            // Exit 1 with empty output is the detached-HEAD convention.
            Some(1) => Ok(None),
            _ => Err(GitreeError::GitFailed {
                summary: "git symbolic-ref --short -q HEAD".into(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            }),
        }
    }

    /// Returns `true` if a tag with the given name exists.
    ///
    /// # Errors
    ///
    /// Returns an error if git fails.
    pub fn tag_exists(&self, name: &str) -> Result<bool> {
        Ok(self
            .run(&[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("refs/tags/{name}"),
            ])
            .is_ok())
    }

    /// Runs `git check-ignore -q <path>`.
    ///
    /// Returns `true` when the path is effectively ignored by any ignore
    /// source (.gitignore files, `.git/info/exclude`, `core.excludesFile`).
    ///
    /// # Errors
    ///
    /// Returns an error if git fails for a reason other than "not ignored".
    pub fn check_ignore(&self, path: &str) -> Result<bool> {
        let output = self.exec(&["check-ignore", "-q", path], &[])?;
        match output.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(GitreeError::GitFailed {
                summary: format!("git check-ignore -q {path}"),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            }),
        }
    }

    /// Returns `true` if the path is tracked in the worktree's index.
    ///
    /// This covers files present in the index regardless of their working
    /// tree state, so an unstaged deletion or a sparse-checkout skip is
    /// detected before overwriting the path with e.g. a `.shared` symlink.
    ///
    /// # Errors
    ///
    /// Returns an error if git fails.
    pub fn path_tracked(&self, path: &str) -> Result<bool> {
        let out = self.run(&["ls-files", "--", path])?;
        Ok(!out.is_empty())
    }

    /// Configures the upstream for `branch` on `origin`.
    ///
    /// Sets `branch.<name>.remote` and `branch.<name>.merge` (to
    /// `refs/heads/<name>`), which a bare clone does not establish on its
    /// own. Existing tracking configuration is left untouched.
    ///
    /// # Errors
    ///
    /// Returns an error if git fails.
    pub fn set_branch_upstream(&self, branch: &str) -> Result<()> {
        self.config_set(&format!("branch.{branch}.remote"), "origin")?;
        self.config_set(
            &format!("branch.{branch}.merge"),
            &format!("refs/heads/{branch}"),
        )?;
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

/// The outcome of a successful `git merge --ff-only`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MergeResult {
    /// HEAD before the merge, when it could be resolved.
    pub head_before: Option<String>,
    /// HEAD after the merge, when it could be resolved.
    pub head_after: Option<String>,
    /// stderr captured from a *successful* merge. Git reports autostash
    /// restoration conflicts on stderr while still exiting zero, so
    /// callers must inspect this instead of trusting the exit status.
    pub notes: String,
}

/// The upstream tracking state of a local branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamState {
    /// The branch name (without `refs/heads/`).
    pub name: String,
    /// The configured upstream (e.g. `origin/main`), if any.
    pub upstream: Option<String>,
    /// Whether git reports the upstream as gone (its remote-tracking ref
    /// no longer exists).
    pub gone: bool,
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
    /// Whether git considers this worktree prunable (its gitdir is stale).
    pub prunable: bool,
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
                        prunable: false,
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
                "prunable" => {
                    if let Some(ref mut entry) = current {
                        entry.prunable = true;
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

    #[test]
    fn ahead_behind_without_upstream_is_none() {
        let tmp = tempfile::TempDir::new().unwrap();
        run_git(tmp.path(), &["init", "--initial-branch=main"]);
        run_git(tmp.path(), &["config", "user.email", "t@example.com"]);
        run_git(tmp.path(), &["config", "user.name", "Test"]);
        std::fs::write(tmp.path().join("f"), "x").unwrap();
        run_git(tmp.path(), &["add", "."]);
        run_git(tmp.path(), &["commit", "-m", "initial"]);

        assert_eq!(Git::new(tmp.path()).ahead_behind("main").unwrap(), None);
    }

    #[test]
    fn ahead_behind_counts_commits_vs_origin() {
        // Source repo that acts as the clone's origin.
        let tmp = tempfile::TempDir::new().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir(&src).unwrap();
        run_git(&src, &["init", "--initial-branch=main"]);
        run_git(&src, &["config", "user.email", "t@example.com"]);
        run_git(&src, &["config", "user.name", "Test"]);
        std::fs::write(src.join("f"), "x").unwrap();
        run_git(&src, &["add", "."]);
        run_git(&src, &["commit", "-m", "initial"]);

        // Clone, then commit locally: 1 ahead, 0 behind.
        let clone = tmp.path().join("clone");
        run_git(
            tmp.path(),
            &["clone", src.to_str().unwrap(), clone.to_str().unwrap()],
        );
        std::fs::write(clone.join("g"), "y").unwrap();
        run_git(&clone, &["add", "."]);
        run_git(&clone, &["commit", "-m", "local work"]);

        let git = Git::new(&clone);
        assert_eq!(git.ahead_behind("main").unwrap(), Some((1, 0)));
    }

    /// Runs a git command, failing the test on non-zero exit.
    ///
    /// Prepends `-c commit.gpgsign=false` so no test commit ever touches a
    /// signing agent, and sets an explicit author/committer identity so the
    /// test never depends on global git configuration.
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

    #[test]
    fn parse_prunable_and_locked() {
        let input = "worktree /home/user/project/gone\nHEAD abc123\nbranch refs/heads/feature\nprunable gitdir file points to non-existent location\n\nworktree /home/user/project/locked\nHEAD def456\nbranch refs/heads/locked\nlocked reason\nprunable gitdir file points to non-existent location\n\nworktree /home/user/project/main\nHEAD 789abc\nbranch refs/heads/main\n";
        let entries = WorktreeEntry::parse(input);
        assert_eq!(entries.len(), 3);
        assert!(entries[0].prunable);
        assert!(!entries[0].locked);
        assert!(entries[1].prunable && entries[1].locked);
        assert!(!entries[2].prunable);
        assert!(!entries[2].locked);
    }
}
