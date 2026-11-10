//! `gitree clean` — remove stale worktrees and delete branches whose
//! configured upstream is gone.

use std::collections::HashSet;

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
/// 3. Identifies local branches whose *configured upstream* is gone.
/// 4. Offers to delete them (interactive unless `--force`).
///
/// Branches that were never pushed (no upstream configured) are never
/// considered stale: "no same-name remote branch" is not evidence that a
/// branch was abandoned.
///
/// # Errors
///
/// Returns an error if git fails or any branch deletion fails.
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

    // Stale = the branch's configured upstream no longer exists, and the
    // branch has no active worktree.
    let worktrees = git.worktree_list()?;
    let active_branches: HashSet<&str> = worktrees
        .iter()
        .filter_map(|wt| wt.branch.as_deref())
        .collect();

    let stale: Vec<String> = git
        .upstream_states()?
        .into_iter()
        .filter(|b| b.upstream.is_some() && b.gone && !active_branches.contains(b.name.as_str()))
        .map(|b| b.name)
        .collect();

    if stale.is_empty() {
        eprintln!("No stale branches found.");
        return Ok(());
    }

    eprintln!("\nFound {} stale branch(es):", stale.len());
    for b in &stale {
        let merged = match git.branch_merged_into_head(b) {
            Ok(true) => "fully merged",
            Ok(false) => "has unmerged commits",
            Err(_) => "merged status unknown",
        };
        eprintln!("  {b} ({merged})");
    }

    if !opts.force {
        eprintln!("\nUse --force to delete them.");
        return Ok(());
    }

    eprintln!("\nDeleting (--force) …");
    let mut failures = Vec::new();
    for b in &stale {
        // Prefer the safe delete (refuses to discard unmerged commits);
        // only fall back to -D because --force was given explicitly.
        match git.branch_delete(b, false) {
            Ok(()) => eprintln!("  Deleted: {b}"),
            Err(_) => {
                eprintln!("  {b} is not fully merged — force-deleting");
                match git.branch_delete(b, true) {
                    Ok(()) => eprintln!("  Force-deleted: {b}"),
                    Err(e) => failures.push(format!("{b}: {e}")),
                }
            }
        }
    }

    if !failures.is_empty() {
        return Err(GitreeError::Other(format!(
            "failed to delete {} branch(es): {}",
            failures.len(),
            failures.join("; ")
        )));
    }

    Ok(())
}
