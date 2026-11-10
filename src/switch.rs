//! `gitree switch` and `gitree root`.

use std::io::Write;

use crate::error::Result;
use crate::repo::Wrapper;

/// Shell-escapes a path for use in a `cd '...'` command in POSIX-style
/// shells (bash, zsh, dash).
///
/// Replaces any single quote `'` with `'\''` (the standard POSIX escaping
/// pattern) and wraps the entire string in single quotes. This is *not*
/// safe for fish, which treats backslash escapes inside single quotes
/// differently — fish consumers must receive the destination as data (see
/// `gitree env fish`, which `cd`s the path returned by `gitree where`).
pub(crate) fn shell_escape(s: &str) -> String {
    let escaped = s.replace('\'', "'\\''");
    format!("'{escaped}'")
}

/// Runs the `switch` command — prints a `cd` command for `eval`.
///
/// Accepts plain branch names, directory-style names (`branch/`), and
/// worktree paths.  The path is the one git registers for the worktree,
/// single-quote-escaped to handle paths containing spaces, quotes, or
/// dollar signs safely.
///
/// # Errors
///
/// Returns an error if no worktree exists for the branch.
pub fn run_switch(wrapper: &Wrapper, branch: &str) -> Result<()> {
    let entry = wrapper.resolve_worktree(branch)?;
    let escaped = shell_escape(&entry.path.display().to_string());
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    writeln!(out, "cd {escaped}")?;
    Ok(())
}

/// Runs the `root` command — prints the wrapper root path.
///
/// # Errors
///
/// Returns an error if writing to stdout fails.
pub fn run_root(wrapper: &Wrapper) -> Result<()> {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    writeln!(out, "{}", wrapper.path().display())?;
    Ok(())
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
