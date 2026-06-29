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
    #[error("working tree is dirty: {0} uncommitted change(s)")]
    DirtyWorktree(usize),

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
