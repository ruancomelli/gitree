//! `gitree pull` — fetch + fast-forward main.

use crate::error::{GitreeError, Result};
use crate::repo::Wrapper;

/// Options for `gitree pull`.
#[derive(Debug, Clone)]
pub struct PullOptions {
    /// Override the branch to fast-forward (default: main, fallback master).
    pub branch: Option<String>,
}

/// Runs the `pull` command.
///
/// Fetches from origin, then fast-forwards the main worktree (or the
/// specified branch's worktree) if it is clean.
///
/// # Errors
///
/// Returns an error if the worktree is dirty, the branch doesn't exist, or
/// git fails.
pub fn run(wrapper: &Wrapper, opts: PullOptions) -> Result<()> {
    let git = wrapper.git();

    eprintln!("Fetching origin …");
    git.fetch()?;

    let branch = opts.branch.unwrap_or_else(|| {
        let local = git.local_branches().unwrap_or_default();
        if local.iter().any(|b| b == "main") {
            "main".into()
        } else {
            "master".into()
        }
    });

    let worktrees = git.worktree_list()?;
    let main_wt = worktrees
        .iter()
        .find(|wt| wt.branch.as_deref() == Some(branch.as_str()));

    let Some(main_wt) = main_wt else {
        eprintln!("No worktree for '{branch}' — nothing to fast-forward.");
        eprintln!("Hint: run `gitree add {branch}` to create one.");
        return Ok(());
    };

    let main_git = wrapper.git_for(main_wt.path.as_path());

    let dirty_count = main_git.dirty_count()?;
    if dirty_count > 0 {
        return Err(GitreeError::DirtyWorktree(dirty_count));
    }

    eprintln!("Fast-forwarding '{branch}' …");
    main_git.merge_ff_only(&format!("origin/{branch}"))?;

    eprintln!("Done.");
    Ok(())
}
