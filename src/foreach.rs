//! `gitree foreach` — run a command in every worktree.

use std::io::Write;
use std::process::Command;

use crate::error::GitreeError;
use crate::error::Result;
use crate::git::WorktreeEntry;
use crate::repo::Wrapper;

/// Options for `gitree foreach`.
#[derive(Debug, Clone)]
pub struct ForeachOptions {
    /// The command to run (passed to `sh -c`).
    pub command: String,
    /// Run in parallel using threads.
    pub parallel: bool,
    /// Filter worktrees by branch glob pattern.
    pub only: Option<String>,
}

/// Runs the `foreach` command.
///
/// # Errors
///
/// Returns an error if the command fails in any worktree.
pub fn run(wrapper: &Wrapper, opts: ForeachOptions) -> Result<()> {
    let git = wrapper.git();
    let entries = git.worktree_list()?;

    let matcher = opts
        .only
        .as_deref()
        .map(|pattern| {
            globset::GlobBuilder::new(pattern)
                .build()
                .map_err(|e| GitreeError::Other(format!("invalid glob pattern '{pattern}': {e}")))
        })
        .transpose()?
        .map(|glob| glob.compile_matcher());

    let worktrees: Vec<&WorktreeEntry> = entries
        .iter()
        .filter(|e| !e.bare)
        .filter(|e| match &matcher {
            Some(m) => {
                let branch = e.branch.as_deref().unwrap_or("");
                m.is_match(branch)
            }
            None => true,
        })
        .collect();

    if opts.parallel {
        run_parallel(&worktrees, &opts.command)
    } else {
        run_sequential(&worktrees, &opts.command)
    }
}

fn run_sequential(worktrees: &[&WorktreeEntry], command: &str) -> Result<()> {
    for wt in worktrees {
        let branch = wt.branch.as_deref().unwrap_or("(detached)");
        eprintln!("=== {branch} ({}) ===", wt.path.display());
        let status = Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(&wt.path)
            .status()?;

        if !status.success() {
            return Err(GitreeError::Other(format!(
                "command failed in '{branch}' with exit code {}",
                status.code().unwrap_or(-1)
            )));
        }
    }
    Ok(())
}

struct ThreadResult {
    branch: String,
    path_display: String,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    error: Option<String>,
}

fn run_parallel(worktrees: &[&WorktreeEntry], command: &str) -> Result<()> {
    let results: Vec<ThreadResult> = std::thread::scope(|s| {
        let handles: Vec<_> = worktrees
            .iter()
            .map(|wt| {
                let branch = wt.branch.as_deref().unwrap_or("(detached)").to_string();
                let path = wt.path.clone();
                let path_display = path.display().to_string();
                let cmd = command.to_string();
                s.spawn(move || -> ThreadResult {
                    let output = Command::new("sh")
                        .arg("-c")
                        .arg(&cmd)
                        .current_dir(&path)
                        .output();
                    match output {
                        Ok(output) if output.status.success() => ThreadResult {
                            branch,
                            path_display,
                            stdout: output.stdout,
                            stderr: output.stderr,
                            error: None,
                        },
                        Ok(output) => ThreadResult {
                            branch: branch.clone(),
                            path_display,
                            stdout: output.stdout,
                            stderr: output.stderr,
                            error: Some(format!(
                                "command failed in '{branch}' with exit code {}",
                                output.status.code().unwrap_or(-1)
                            )),
                        },
                        Err(e) => ThreadResult {
                            branch,
                            path_display,
                            stdout: Vec::new(),
                            stderr: Vec::new(),
                            error: Some(format!("failed to spawn: {e}")),
                        },
                    }
                })
            })
            .collect();

        handles
            .into_iter()
            .map(|h| {
                h.join().unwrap_or_else(|_| ThreadResult {
                    branch: "(unknown)".into(),
                    path_display: "(unknown)".into(),
                    stdout: Vec::new(),
                    stderr: Vec::new(),
                    error: Some("thread panicked".into()),
                })
            })
            .collect()
    });

    let mut had_error = false;
    for result in &results {
        eprintln!("=== {} ({}) ===", result.branch, result.path_display);
        if !result.stdout.is_empty() {
            let _ = std::io::stdout().write_all(&result.stdout);
        }
        if !result.stderr.is_empty() {
            let _ = std::io::stderr().write_all(&result.stderr);
        }
        if let Some(ref err) = result.error {
            eprintln!("error: {err}");
            had_error = true;
        }
    }

    if had_error {
        Err(GitreeError::Other(
            "one or more worktree commands failed".into(),
        ))
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn foreach_options_construction() {
        let opts = ForeachOptions {
            command: "echo hello".into(),
            parallel: false,
            only: Some("feature/*".into()),
        };
        assert_eq!(opts.command, "echo hello");
        assert!(!opts.parallel);
    }
}
