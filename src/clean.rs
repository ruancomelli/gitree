//! `gitree clean` — remove stale worktrees and delete branches gone from remote.

use std::io::Write;

use crate::error::{GitreeError, Result};
use crate::repo::Wrapper;

/// Options for `gitree clean`.
#[derive(Debug, Clone)]
pub struct CleanOptions {
    /// Delete stale branches without prompting.
    pub force: bool,
}

/// Runs the `clean` command.
///
/// 1. Prunes stale worktree references.
/// 2. Fetches and prunes remote-tracking refs (`git fetch --prune`).
/// 3. Identifies local branches whose remote counterpart is gone.
/// 4. Offers to delete them (interactive unless `--force`).
///
/// # Errors
///
/// Returns an error if git fails.
pub fn run(wrapper: &Wrapper, opts: CleanOptions) -> Result<()> {
    let git = wrapper.git();

    eprintln!("Pruning stale worktree references …");
    git.worktree_prune()?;

    eprintln!("Pruning remote-tracking refs …");
    // Stale-branch detection depends on fresh remote state: a failed fetch
    // leaves the ref list outdated, and deleting against it could remove
    // branches that still exist on the remote.
    if let Err(e) = git.run_fetch_prune() {
        return Err(GitreeError::Other(format!(
            "could not refresh remote-tracking refs ({e}) — \
             refusing to detect stale branches from outdated remote state"
        )));
    }

    // Find local branches whose remote is gone.
    let local_branches = git.local_branches()?;
    let remote_branches = git.remote_branches()?;
    let worktrees = git.worktree_list()?;

    let active_branches: std::collections::HashSet<&str> = worktrees
        .iter()
        .filter_map(|wt| wt.branch.as_deref())
        .collect();

    let stale: Vec<&String> = local_branches
        .iter()
        .filter(|b| {
            // Not on remote and doesn't have an active worktree.
            !remote_branches.iter().any(|r| r == *b) && !active_branches.contains(b.as_str())
        })
        .collect();

    if stale.is_empty() {
        eprintln!("No stale branches found.");
        return Ok(());
    }

    eprintln!("\nFound {} stale branch(es):", stale.len());
    for b in &stale {
        eprintln!("  {b}");
    }

    if opts.force {
        eprintln!("\nDeleting (--force) …");
        for b in &stale {
            match git.branch_delete(b, true) {
                Ok(()) => eprintln!("  Deleted: {b}"),
                Err(e) => eprintln!("  Failed to delete {b}: {e}"),
            }
        }
    } else {
        eprintln!("\nUse --force to delete them.");
    }

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let _ = writeln!(out);
    Ok(())
}
