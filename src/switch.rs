//! `gitree switch` and `gitree root`.

use std::io::Write;

use crate::error::{GitreeError, Result};
use crate::repo::Wrapper;
use crate::types::BranchName;

/// Shell-escapes a path for use in a `cd '...'` command.
///
/// Replaces any single quote `'` with `'\''` (the standard POSIX escaping
/// pattern) and wraps the entire string in single quotes.
fn shell_escape(s: &str) -> String {
    let escaped = s.replace('\'', "'\\''");
    format!("'{escaped}'")
}

/// Runs the `switch` command — prints a `cd` command for `eval`.
///
/// The path is single-quote-escaped to handle paths containing spaces,
/// quotes, or dollar signs safely.
///
/// # Errors
///
/// Returns an error if no worktree exists for the branch.
pub fn run_switch(wrapper: &Wrapper, branch: &str) -> Result<()> {
    let branch = BranchName::new(branch)?;
    let path = wrapper.worktree_path(branch.as_str());
    if !path.as_path().exists() {
        return Err(GitreeError::PathMissing(path.into_pathbuf()));
    }
    let escaped = shell_escape(&path.as_path().display().to_string());
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    writeln!(out, "cd {escaped}")?;
    Ok(())
}

/// Runs the `root` command — prints the wrapper root path.
pub fn run_root(wrapper: &Wrapper) {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let _ = writeln!(out, "{}", wrapper.path().display());
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_simple_path() {
        assert_eq!(shell_escape("/tmp/project"), "'/tmp/project'");
    }

    #[test]
    fn escape_path_with_space() {
        assert_eq!(shell_escape("/tmp/my project"), "'/tmp/my project'");
    }

    #[test]
    fn escape_path_with_single_quote() {
        assert_eq!(shell_escape("/tmp/it's"), "'/tmp/it'\\''s'");
    }

    #[test]
    fn escape_path_with_dollar() {
        assert_eq!(shell_escape("/tmp/$HOME"), "'/tmp/$HOME'");
    }
}
