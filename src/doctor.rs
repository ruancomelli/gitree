//! `gitree doctor` — health check for the gitree wrapper.

use std::io::Write;

use crate::error::{GitreeError, Result};
use crate::git::Git;
use crate::repo::Wrapper;

/// A single health check result.
#[derive(Debug)]
pub struct Check {
    /// The name of the check.
    pub name: &'static str,
    /// Whether the check passed.
    pub passed: bool,
    /// Optional message (for failures or informational notes).
    pub message: Option<String>,
}

/// Runs the `doctor` command.
///
/// # Errors
///
/// Returns an error only for unrecoverable failures (not for individual check
/// failures, which are reported as results).
pub fn run(wrapper: &Wrapper) -> Result<()> {
    let checks = vec![
        check_bare_dir(wrapper),
        check_git_file(wrapper),
        check_shared_dir(wrapper),
        check_fsck(wrapper),
        check_git_installed(),
    ];

    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    let mut all_passed = true;
    for check in &checks {
        let status = if check.passed { "ok" } else { "FAIL" };
        let _ = write!(out, "{:<10}  {}", status, check.name);
        if let Some(ref msg) = check.message {
            let _ = write!(out, "  ({msg})");
        }
        let _ = writeln!(out);
        if !check.passed {
            all_passed = false;
        }
    }

    if all_passed {
        let _ = writeln!(out, "\nAll checks passed.");
    } else {
        let _ = writeln!(out, "\nSome checks failed. See messages above.");
    }

    Ok(())
}

fn check_bare_dir(wrapper: &Wrapper) -> Check {
    if wrapper.has_bare_dir() {
        Check {
            name: ".bare/",
            passed: true,
            message: None,
        }
    } else {
        Check {
            name: ".bare/",
            passed: false,
            message: Some("directory missing".into()),
        }
    }
}

fn check_git_file(wrapper: &Wrapper) -> Check {
    if wrapper.has_git_file() {
        Check {
            name: ".git file",
            passed: true,
            message: None,
        }
    } else {
        Check {
            name: ".git file",
            passed: false,
            message: Some("file missing or doesn't point at .bare".into()),
        }
    }
}

fn check_shared_dir(wrapper: &Wrapper) -> Check {
    if wrapper.has_shared_dir() {
        Check {
            name: ".shared/",
            passed: true,
            message: None,
        }
    } else {
        Check {
            name: ".shared/",
            passed: false,
            message: Some("directory missing (run `mkdir .shared`)".into()),
        }
    }
}

fn check_fsck(wrapper: &Wrapper) -> Check {
    match wrapper.git().fsck() {
        Ok(()) => Check {
            name: "git fsck",
            passed: true,
            message: None,
        },
        Err(e) => Check {
            name: "git fsck",
            passed: false,
            message: Some(format!("{e}")),
        },
    }
}

fn check_git_installed() -> Check {
    match Git::cwd().version() {
        Ok(version) => Check {
            name: "git",
            passed: true,
            message: Some(version),
        },
        Err(GitreeError::GitNotFound) => Check {
            name: "git",
            passed: false,
            message: Some("git not found on PATH".into()),
        },
        Err(e) => Check {
            name: "git",
            passed: false,
            message: Some(format!("{e}")),
        },
    }
}
