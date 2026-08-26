//! Error types for gitree.
//!
//! Uses [`thiserror`] for a typed error enum at the library level.
//! [`anyhow`] is used at the application entry point (`main.rs`) for
//! context chaining on operations that don't need typed error handling.

use std::io;
use std::path::PathBuf;
use std::process::ExitCode;

/// A specialised `Result` type used throughout gitree.
pub type Result<T> = std::result::Result<T, GitreeError>;

/// The escape hatch available when a worktree is dirty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirtyEscape {
    /// `--autostash`: stash, ff-merge, pop (used by `pull`).
    Autostash,
    /// `--force`: bypass the dirty pre-check (used by `migrate`).
    Force,
}

/// The main error type for gitree operations.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum GitreeError {
    /// The current directory is not inside a gitree wrapper.
    #[error("not a gitree repository (or any parent directory): {0}")]
    NotAWrapper(PathBuf),

    /// A git command failed. The tuple holds the command summary and stderr.
    #[error("git command failed: {summary}\n{stderr}")]
    GitFailed { summary: String, stderr: String },

    /// Git itself is not installed or not on PATH.
    #[error("git executable not found on PATH")]
    GitNotFound,

    /// A path that was expected to exist does not.
    #[error("path does not exist: {0}")]
    PathMissing(PathBuf),

    /// A path that was expected to be absent already exists.
    #[error("path already exists: {0}")]
    PathExists(PathBuf),

    /// A worktree for the given branch already exists.
    #[error("a worktree for branch '{0}' already exists")]
    WorktreeExists(String),

    /// The branch was not found locally or on any remote.
    #[error("branch '{0}' not found locally or on any remote")]
    BranchNotFound(String),

    /// The working tree is dirty and the operation requires a clean state.
    #[error("working tree{loc} is dirty: {count} uncommitted change(s)", loc = self.dirty_location())]
    DirtyWorktree {
        /// Number of uncommitted changes.
        count: usize,
        /// The branch name of the dirty worktree, if known.
        branch: Option<String>,
        /// The filesystem path of the dirty worktree, if known.
        path: Option<PathBuf>,
        /// The escape hatch available to the user.
        escape: DirtyEscape,
    },

    /// A pre-flight check failed during `migrate`.
    #[error("pre-flight check failed: {0}")]
    PreflightFailed(String),

    /// An I/O error occurred.
    #[error(transparent)]
    Io(#[from] io::Error),

    /// A JSON serialisation/deserialisation error.
    #[error(transparent)]
    Json(#[from] serde_json::Error),

    /// A catch-all for errors that don't fit the above, with context.
    #[error("{0}")]
    Other(String),
}

impl GitreeError {
    /// Returns a formatted location suffix for the `DirtyWorktree` variant,
    /// e.g. ` 'main'` or an empty string when the branch is unknown.
    ///
    /// Lives on `self` so the `#[error(...)]` format string can call it while
    /// matching the same variant.
    fn dirty_location(&self) -> String {
        match self {
            Self::DirtyWorktree { branch, .. } => branch
                .as_deref()
                .map(|b| format!(" '{b}'"))
                .unwrap_or_default(),
            _ => String::new(),
        }
    }

    /// Returns the exit code that should be used when this error is the
    /// top-level failure.
    #[must_use]
    pub fn exit_code(&self) -> ExitCode {
        match self {
            Self::NotAWrapper(_) => ExitCode::from(3),
            _ => ExitCode::from(1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirty_worktree_display_with_branch() {
        let err = GitreeError::DirtyWorktree {
            count: 3,
            branch: Some("main".into()),
            path: Some(PathBuf::from("/home/user/proj/main")),
            escape: DirtyEscape::Autostash,
        };
        assert_eq!(
            err.to_string(),
            "working tree 'main' is dirty: 3 uncommitted change(s)"
        );
    }

    #[test]
    fn dirty_worktree_display_without_branch() {
        let err = GitreeError::DirtyWorktree {
            count: 1,
            branch: None,
            path: None,
            escape: DirtyEscape::Force,
        };
        assert_eq!(
            err.to_string(),
            "working tree is dirty: 1 uncommitted change(s)"
        );
    }
}
