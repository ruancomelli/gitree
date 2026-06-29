//! Newtypes for domain primitives.
//!
//! Each type wraps a path or string and centralises validation so that the rest
//! of the codebase can work with strongly-typed values rather than raw
//! [`PathBuf`] / [`String`].

use std::ffi::OsStr;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::error::{GitreeError, Result};

// ---------------------------------------------------------------------------
// BranchName
// ---------------------------------------------------------------------------

/// A validated git branch name.
///
/// Branch names in git cannot start with `-`, contain `..`, `~`, `^`, `:`,
/// `\\`, `?`, `[`, `*`, or have a trailing `/`.  See
/// <https://git-scm.com/docs/git-check-ref-format>.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BranchName(String);

impl BranchName {
    /// Validates and creates a [`BranchName`].
    ///
    /// # Errors
    ///
    /// Returns [`GitreeError::Other`] if the name violates git's ref-name
    /// rules.
    pub fn new(name: &str) -> Result<Self> {
        Self::validate(name)?;
        Ok(Self(name.to_string()))
    }

    /// Returns the branch name as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn validate(name: &str) -> Result<()> {
        if name.is_empty() {
            return Err(GitreeError::Other("branch name is empty".into()));
        }
        if name.starts_with('-') || name.starts_with('.') {
            return Err(GitreeError::Other(format!(
                "branch name '{name}' must not start with '-' or '.'"
            )));
        }
        if name.ends_with('/') {
            return Err(GitreeError::Other(format!(
                "branch name '{name}' must not end with '/'"
            )));
        }
        if name.ends_with(".lock") {
            return Err(GitreeError::Other(format!(
                "branch name '{name}' must not end with '.lock'"
            )));
        }
        for (i, ch) in name.chars().enumerate() {
            if matches!(ch, ' ' | '~' | '^' | ':' | '?' | '*' | '[' | '\\') {
                return Err(GitreeError::Other(format!(
                    "branch name '{name}' contains invalid character '{ch}' at position {i}"
                )));
            }
        }
        if name.contains("..") {
            return Err(GitreeError::Other(format!(
                "branch name '{name}' must not contain '..'"
            )));
        }
        if name.contains("//") {
            return Err(GitreeError::Other(format!(
                "branch name '{name}' must not contain '//'"
            )));
        }
        if name.contains("@{") {
            return Err(GitreeError::Other(format!(
                "branch name '{name}' must not contain '@{{'"
            )));
        }
        Ok(())
    }
}

impl AsRef<str> for BranchName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl AsRef<OsStr> for BranchName {
    fn as_ref(&self) -> &OsStr {
        self.0.as_ref()
    }
}

impl fmt::Display for BranchName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl PartialEq<str> for BranchName {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

// ---------------------------------------------------------------------------
// Path newtypes
// ---------------------------------------------------------------------------

macro_rules! path_newtype {
    ($name:ident, $doc:expr) => {
        #[doc = $doc]
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct $name(PathBuf);

        impl $name {
            /// Creates a new instance from any [`AsRef<Path>`].
            #[must_use]
            pub fn from_path<P: AsRef<Path>>(path: P) -> Self {
                Self(path.as_ref().to_path_buf())
            }

            /// Returns a reference to the inner [`Path`].
            #[must_use]
            pub fn as_path(&self) -> &Path {
                &self.0
            }

            /// Consumes and returns the inner [`PathBuf`].
            #[allow(dead_code)]
            pub fn into_pathbuf(self) -> PathBuf {
                self.0
            }
        }

        impl AsRef<Path> for $name {
            fn as_ref(&self) -> &Path {
                &self.0
            }
        }

        impl From<PathBuf> for $name {
            fn from(path: PathBuf) -> Self {
                Self(path)
            }
        }

        impl From<&Path> for $name {
            fn from(path: &Path) -> Self {
                Self(path.to_path_buf())
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.display().fmt(f)
            }
        }
    };
}

path_newtype!(
    BareDir,
    "The `.bare/` directory inside the wrapper — the shared git database."
);
path_newtype!(
    SharedDir,
    "The `.shared/` directory inside the wrapper — holds gitignored files symlinked into each worktree."
);
path_newtype!(WorktreePath, "The filesystem path of a single worktree.");

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_branch_names() {
        assert!(BranchName::new("main").is_ok());
        assert!(BranchName::new("feature/my-feature").is_ok());
        assert!(BranchName::new("release-1.0").is_ok());
        assert!(BranchName::new("bugfix/fix-issue-123").is_ok());
    }

    #[test]
    fn invalid_branch_names() {
        assert!(BranchName::new("").is_err());
        assert!(BranchName::new("-branch").is_err());
        assert!(BranchName::new(".hidden").is_err());
        assert!(BranchName::new("branch/").is_err());
        assert!(BranchName::new("branch..name").is_err());
        assert!(BranchName::new("branch//name").is_err());
        assert!(BranchName::new("branch name").is_err());
        assert!(BranchName::new("branch~name").is_err());
        assert!(BranchName::new("branch:name").is_err());
        assert!(BranchName::new("branch.lock").is_err());
    }

    #[test]
    fn branch_name_as_str_and_display() {
        let name = BranchName::new("feature/test").unwrap();
        assert_eq!(name.as_str(), "feature/test");
        assert_eq!(name.to_string(), "feature/test");
    }

    #[test]
    fn path_newtypes() {
        let bare = BareDir::from_path("/tmp/project/.bare");
        assert_eq!(bare.as_path(), Path::new("/tmp/project/.bare"));
        assert_eq!(bare.to_string(), "/tmp/project/.bare");
    }
}
