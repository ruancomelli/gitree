//! Wrapper-root discovery and methods.
//!
//! The "wrapper root" is the top-level directory in a gitree-managed layout.
//! It contains:
//!
//! - `.bare/` — the shared git database
//! - `.git` — a *file* (not directory) with `gitdir: ./.bare`
//! - `.shared/` — gitignored files symlinked into each worktree
//! - one subdirectory per worktree (e.g. `main/`, `feature/foo/`)

use std::path::{Path, PathBuf};

use crate::error::{GitreeError, Result};
use crate::git::Git;
use crate::types::{BareDir, BranchName, SharedDir, WorktreePath};

/// The wrapper root directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wrapper {
    path: PathBuf,
}

impl Wrapper {
    /// Discovers the wrapper root by walking up from the current directory.
    ///
    /// # Errors
    ///
    /// Returns [`GitreeError::NotAWrapper`] if no wrapper root can be found.
    pub fn discover() -> Result<Self> {
        Self::from_cwd(&std::env::current_dir()?)
    }

    /// Discovers the wrapper root by walking up from `cwd`.
    pub(crate) fn from_cwd(cwd: &Path) -> Result<Self> {
        // Strategy 1: walk up looking for a .git file pointing at .bare.
        for dir in cwd.ancestors() {
            if Self::is_wrapper_dir(dir) {
                return Ok(Self {
                    path: dir.to_path_buf(),
                });
            }
        }

        // Strategy 2: use git to find the common dir, then its parent.
        let git = Git::new(cwd);
        if let Ok(common_dir) = git.common_dir() {
            let common_abs = if common_dir.is_absolute() {
                common_dir
            } else {
                cwd.join(&common_dir)
            };
            let resolved = common_abs.canonicalize().unwrap_or(common_abs.clone());

            for dir in resolved.ancestors() {
                if Self::is_wrapper_dir(dir) {
                    return Ok(Self {
                        path: dir.to_path_buf(),
                    });
                }
            }

            // If common_dir is `.bare`, its parent might be the wrapper.
            if resolved.file_name().is_some_and(|n| n == ".bare")
                || resolved
                    .parent()
                    .is_some_and(|p| p.file_name().is_some_and(|n| n == ".bare"))
            {
                let bare = if resolved.file_name().is_some_and(|n| n == ".bare") {
                    resolved.clone()
                } else {
                    resolved.parent().unwrap_or(&resolved).to_path_buf()
                };
                if let Some(parent) = bare.parent() {
                    return Ok(Self {
                        path: parent.to_path_buf(),
                    });
                }
            }
        }

        Err(GitreeError::NotAWrapper(cwd.to_path_buf()))
    }

    /// Returns `true` if `dir` contains a `.git` file pointing at `.bare`.
    fn is_wrapper_dir(dir: &Path) -> bool {
        let git_file = dir.join(".git");
        if !git_file.is_file() {
            return false;
        }
        let Ok(content) = std::fs::read_to_string(&git_file) else {
            return false;
        };
        let content = content.trim();
        if let Some(rest) = content.strip_prefix("gitdir:") {
            let path = rest.trim();
            let bare_path = if Path::new(path).is_absolute() {
                PathBuf::from(path)
            } else {
                dir.join(path)
            };
            bare_path.ends_with(".bare")
        } else {
            false
        }
    }

    /// Returns the path to the wrapper root.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the `.bare/` directory.
    #[must_use]
    pub fn bare_dir(&self) -> BareDir {
        BareDir::from_path(self.path.join(".bare"))
    }

    /// Returns the `.shared/` directory.
    #[must_use]
    pub fn shared_dir(&self) -> SharedDir {
        SharedDir::from_path(self.path.join(".shared"))
    }

    /// Returns the [`Git`] handle rooted at the wrapper.
    #[must_use]
    pub fn git(&self) -> Git {
        Git::new(&self.path)
    }

    /// Returns the path for a worktree of the given branch name.
    ///
    /// Branch names with slashes (`feature/foo`) map to nested directories
    /// (`wrapper/feature/foo`).
    #[must_use]
    pub fn worktree_path(&self, branch: &str) -> WorktreePath {
        WorktreePath::from_path(self.path.join(branch))
    }

    /// Returns the [`Git`] handle for a specific worktree path.
    #[must_use]
    pub fn git_for(&self, path: &Path) -> Git {
        Git::new(path)
    }

    /// Resolves a user-supplied branch argument to a [`BranchName`].
    ///
    /// Accepts plain branch names, directory-style names (`branch/`,
    /// `./branch/`), and worktree paths, relative or absolute.  Trailing
    /// slashes are common because shells tab-complete worktree directories.
    ///
    /// The fast path validates the trimmed argument as a branch name, which
    /// works from any working directory.  As a fallback the argument is
    /// resolved as a filesystem path and matched against the worktrees git
    /// reports, so only real worktree branches can come back from it.
    ///
    /// # Errors
    ///
    /// Returns the [`BranchName`] validation error when the argument is
    /// neither a valid branch name nor the path of an existing worktree.
    pub fn resolve_branch_arg(&self, raw: &str) -> Result<BranchName> {
        let trimmed = raw.trim_end_matches('/');
        if !trimmed.starts_with('/')
            && let Ok(branch) = BranchName::new(trimmed)
        {
            return Ok(branch);
        }
        if !trimmed.is_empty()
            && let Some(branch) = self.branch_at_path(raw)
        {
            return Ok(branch);
        }
        BranchName::new(trimmed)
    }

    /// Returns the branch of the worktree at `raw` (relative to CWD or
    /// absolute), or `None` when no worktree matches.
    fn branch_at_path(&self, raw: &str) -> Option<BranchName> {
        let candidate = PathBuf::from(raw);
        let candidate = if candidate.is_absolute() {
            candidate
        } else {
            std::env::current_dir().ok()?.join(candidate)
        };
        let resolved = candidate.canonicalize().unwrap_or(candidate);

        self.git()
            .worktree_list()
            .ok()?
            .into_iter()
            .find_map(|entry| {
                let path = entry.path.canonicalize().unwrap_or(entry.path);
                if path != resolved {
                    return None;
                }
                let branch = entry.branch?;
                BranchName::new(&branch).ok()
            })
    }

    /// Returns `true` if `.shared/` exists.
    #[must_use]
    pub fn has_shared_dir(&self) -> bool {
        self.shared_dir().as_path().is_dir()
    }

    /// Returns `true` if `.bare/` exists.
    #[must_use]
    pub fn has_bare_dir(&self) -> bool {
        self.bare_dir().as_path().is_dir()
    }

    /// Returns `true` if the `.git` file exists and points at `.bare`.
    #[must_use]
    pub fn has_git_file(&self) -> bool {
        Self::is_wrapper_dir(&self.path)
    }
}

impl AsRef<Path> for Wrapper {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl std::fmt::Display for Wrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.path.display().fmt(f)
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

    fn create_wrapper(dir: &Path) {
        fs::create_dir_all(dir.join(".bare")).unwrap();
        fs::write(dir.join(".git"), "gitdir: ./.bare\n").unwrap();
    }

    #[test]
    fn is_wrapper_dir_with_git_file() {
        let tmp = TempDir::new().unwrap();
        create_wrapper(tmp.path());
        assert!(Wrapper::is_wrapper_dir(tmp.path()));
    }

    #[test]
    fn is_wrapper_dir_without_git_file() {
        let tmp = TempDir::new().unwrap();
        assert!(!Wrapper::is_wrapper_dir(tmp.path()));
    }

    #[test]
    fn is_wrapper_dir_with_git_directory() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir(tmp.path().join(".git")).unwrap();
        assert!(!Wrapper::is_wrapper_dir(tmp.path()));
    }

    #[test]
    fn discover_from_cwd() {
        let tmp = TempDir::new().unwrap();
        create_wrapper(tmp.path());
        let wrapper = Wrapper::from_cwd(tmp.path()).unwrap();
        assert_eq!(wrapper.path(), tmp.path());
    }

    #[test]
    fn discover_from_subdirectory() {
        let tmp = TempDir::new().unwrap();
        create_wrapper(tmp.path());
        let subdir = tmp.path().join("main/src");
        fs::create_dir_all(&subdir).unwrap();
        let wrapper = Wrapper::from_cwd(&subdir).unwrap();
        assert_eq!(wrapper.path(), tmp.path());
    }

    #[test]
    fn discover_fails_outside_wrapper() {
        let tmp = TempDir::new().unwrap();
        let result = Wrapper::from_cwd(tmp.path());
        assert!(result.is_err());
    }

    #[test]
    fn worktree_path_with_slashes() {
        let tmp = TempDir::new().unwrap();
        create_wrapper(tmp.path());
        let wrapper = Wrapper {
            path: tmp.path().to_path_buf(),
        };
        let path = wrapper.worktree_path("feature/my-feature");
        assert_eq!(path.as_path(), tmp.path().join("feature/my-feature"));
    }

    #[test]
    fn bare_and_shared_dirs() {
        let tmp = TempDir::new().unwrap();
        create_wrapper(tmp.path());
        fs::create_dir_all(tmp.path().join(".shared")).unwrap();
        let wrapper = Wrapper {
            path: tmp.path().to_path_buf(),
        };
        assert!(wrapper.bare_dir().as_path().exists());
        assert!(wrapper.has_shared_dir());
        assert!(wrapper.has_bare_dir());
        assert!(wrapper.has_git_file());
    }

    /// Runs a git command, returning trimmed stdout and failing on error.
    ///
    /// Prepends `-c commit.gpgsign=false` so no test commit ever touches a
    /// signing agent, regardless of repo or global git config.
    fn run_git(dir: &Path, args: &[&str]) -> String {
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
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    /// Builds a wrapper with real `main` and `feature/test` worktrees.
    fn wrapper_with_worktrees(root: &Path) -> Wrapper {
        let seed = root.join("seed");
        fs::create_dir_all(&seed).unwrap();
        run_git(&seed, &["init", "--initial-branch=main"]);
        run_git(&seed, &["config", "user.email", "t@example.com"]);
        run_git(&seed, &["config", "user.name", "Test"]);
        fs::write(seed.join("f"), "x").unwrap();
        run_git(&seed, &["add", "."]);
        run_git(&seed, &["commit", "-m", "initial"]);

        // Bare clone (config does not survive a clone), then the .git file.
        let proj = root.join("proj");
        fs::create_dir_all(&proj).unwrap();
        run_git(
            root,
            &[
                "clone",
                "--bare",
                seed.to_str().unwrap(),
                proj.join(".bare").to_str().unwrap(),
            ],
        );
        run_git(
            &proj.join(".bare"),
            &["config", "user.email", "t@example.com"],
        );
        run_git(&proj.join(".bare"), &["config", "user.name", "Test"]);
        fs::write(proj.join(".git"), "gitdir: ./.bare\n").unwrap();
        let wrapper = Wrapper::from_cwd(&proj).unwrap();

        run_git(&proj, &["worktree", "add", "main", "main"]);
        run_git(
            &proj,
            &["worktree", "add", "feature/test", "-b", "feature/test"],
        );
        wrapper
    }

    #[test]
    fn resolve_branch_arg_accepts_trailing_slash() {
        let tmp = TempDir::new().unwrap();
        let wrapper = wrapper_with_worktrees(tmp.path());
        assert_eq!(
            wrapper.resolve_branch_arg("main/").unwrap().as_str(),
            "main"
        );
    }

    #[test]
    fn resolve_branch_arg_resolves_worktree_path() {
        let tmp = TempDir::new().unwrap();
        let wrapper = wrapper_with_worktrees(tmp.path());
        let abs = wrapper.path().join("main").display().to_string();
        assert_eq!(wrapper.resolve_branch_arg(&abs).unwrap().as_str(), "main");
    }

    #[test]
    fn resolve_branch_arg_resolves_nested_worktree_path() {
        let tmp = TempDir::new().unwrap();
        let wrapper = wrapper_with_worktrees(tmp.path());
        let nested = wrapper.path().join("feature/test").display().to_string();
        assert_eq!(
            wrapper.resolve_branch_arg(&nested).unwrap().as_str(),
            "feature/test"
        );
    }

    #[test]
    fn resolve_branch_arg_errors_for_invalid_names() {
        let tmp = TempDir::new().unwrap();
        let wrapper = wrapper_with_worktrees(tmp.path());
        for bad in ["../escape/", "branch.lock/", ""] {
            let err = wrapper.resolve_branch_arg(bad).expect_err("must fail");
            assert!(
                err.to_string().contains("branch name"),
                "unexpected error for {bad}: {err}"
            );
        }
    }
}
