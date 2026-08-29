//! `gitree pull` — fetch + fast-forward main (or all worktrees with `--all`).

use crate::error::{DirtyEscape, GitreeError, Result};
use crate::repo::Wrapper;
use crate::types::BranchName;

/// Options for `gitree pull`.
#[derive(Debug, Clone)]
pub struct PullOptions {
    /// Fast-forward every worktree behind its origin branch.
    pub all: bool,
    /// Override the branch to fast-forward (default: main, fallback master).
    pub branch: Option<String>,
    /// Stash uncommitted changes before merging, pop afterwards.
    pub autostash: bool,
}

/// Runs the `pull` command.
///
/// Fetches from origin, then fast-forwards the main worktree (or the
/// specified branch's worktree), or every behind worktree with `--all`.
///
/// # Errors
///
/// Returns an error if the worktree is dirty, the branch doesn't exist, or
/// git fails.
pub fn run(wrapper: &Wrapper, opts: PullOptions) -> Result<()> {
    eprintln!("Fetching origin …");
    wrapper.git().fetch()?;

    if opts.all {
        run_all(wrapper, opts.autostash)
    } else {
        run_single(wrapper, &opts)
    }
}

/// Fast-forwards a single branch's worktree (default: main, fallback master).
fn run_single(wrapper: &Wrapper, opts: &PullOptions) -> Result<()> {
    let git = wrapper.git();

    let requested = opts.branch.clone().unwrap_or_else(|| {
        let local = git.local_branches().unwrap_or_default();
        if local.iter().any(|b| b == "main") {
            "main".into()
        } else {
            "master".into()
        }
    });
    let branch = BranchName::new(&requested)?;

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

    if !opts.autostash {
        let dirty_count = main_git.dirty_count()?;
        if dirty_count > 0 {
            return Err(GitreeError::DirtyWorktree {
                count: dirty_count,
                branch: Some(branch.to_string()),
                path: Some(main_wt.path.clone()),
                escape: DirtyEscape::Autostash,
            });
        }
    }

    eprintln!("Fast-forwarding '{branch}' …");
    main_git.merge_ff_only(&format!("origin/{}", branch.as_str()), opts.autostash)?;

    eprintln!("Done.");
    Ok(())
}

/// Fast-forwards every worktree that is behind its origin branch.
///
/// Worktrees are skipped (with a note) when they are dirty or not
/// fast-forwardable; branches without an `origin/<branch>` upstream are
/// skipped silently.
fn run_all(wrapper: &Wrapper, autostash: bool) -> Result<()> {
    let git = wrapper.git();
    let worktrees = git.worktree_list()?;

    let mut updated = 0usize;
    let mut skipped_diverged: Vec<String> = Vec::new();
    let mut skipped_dirty: Vec<String> = Vec::new();

    for wt in worktrees
        .iter()
        .filter(|wt| !wt.bare && wt.branch.is_some())
    {
        let branch = wt.branch.as_deref().unwrap_or_default();

        let Some((ahead, behind)) = git.ahead_behind(branch)? else {
            continue;
        };
        if ahead > 0 {
            skipped_diverged.push(branch.to_string());
            continue;
        }
        if behind == 0 {
            continue;
        }

        let wt_git = wrapper.git_for(wt.path.as_path());
        if !autostash {
            let dirty_count = wt_git.dirty_count()?;
            if dirty_count > 0 {
                skipped_dirty.push(format!("{branch} ({dirty_count} change(s))"));
                continue;
            }
        }

        eprintln!("Fast-forwarding '{branch}' …");
        wt_git.merge_ff_only(&format!("origin/{branch}"), autostash)?;
        updated += 1;
    }

    for branch in &skipped_diverged {
        eprintln!("Skipped '{branch}' — ahead of origin, not fast-forwardable.");
    }
    for entry in &skipped_dirty {
        eprintln!("Skipped '{entry}' — dirty worktree (use --autostash to stash and pull).");
    }
    eprintln!("Updated {updated} worktree(s).");
    Ok(())
}
