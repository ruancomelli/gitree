//! `gitree pull` — fetch + fast-forward main (or all worktrees with `--all`).

use crate::error::{DirtyEscape, GitreeError, Result};
use crate::git::MergeResult;
use crate::repo::Wrapper;
use crate::switch::shell_escape;
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
/// Returns an error if the worktree is dirty, the branch doesn't exist,
/// git fails, or the merge outcome is not what a fast-forward promises
/// (e.g. the autostash could not be reapplied cleanly).
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
        eprintln!(
            "Hint: run `gitree add {}` to create one.",
            shell_escape(branch.as_str())
        );
        return Ok(());
    };

    match git.ahead_behind(branch.as_str())? {
        None => {
            eprintln!("No origin/{} — nothing to fast-forward.", branch.as_str());
            return Ok(());
        }
        Some((ahead, _)) if ahead > 0 => {
            return Err(GitreeError::Other(format!(
                "'{branch}' is ahead of origin — refusing to fast-forward"
            )));
        }
        Some((_, 0)) => {
            eprintln!("'{}' is already up to date.", branch.as_str());
            return Ok(());
        }
        Some((_, _)) => {}
    }

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
    let remote_ref = format!("refs/remotes/origin/{}", branch.as_str());
    let result = main_git.merge_ff_only(branch.as_str(), &remote_ref, opts.autostash)?;
    check_merge_outcome(branch.as_str(), &result)?;

    eprintln!("Done.");
    Ok(())
}

/// Fast-forwards every worktree that is behind its origin branch.
///
/// Worktrees are skipped (with a note) when they are dirty, diverged, or
/// no longer have the expected branch checked out; branches without an
/// `origin/<branch>` upstream are skipped silently.
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

        // Revalidate the worktree identity immediately before mutating:
        // the snapshot from `worktree_list` may be stale if the user
        // switched branches in this worktree meanwhile.
        match wt_git.current_branch()? {
            Some(current) if current == branch => {}
            Some(other) => {
                eprintln!("Skipped '{branch}' — the worktree now has '{other}' checked out.");
                continue;
            }
            None => {
                eprintln!("Skipped '{branch}' — the worktree has a detached HEAD now.");
                continue;
            }
        }

        if !autostash {
            let dirty_count = wt_git.dirty_count()?;
            if dirty_count > 0 {
                skipped_dirty.push(format!("{branch} ({dirty_count} change(s))"));
                continue;
            }
        }

        eprintln!("Fast-forwarding '{branch}' …");
        let remote_ref = format!("refs/remotes/origin/{branch}");
        let result = wt_git.merge_ff_only(branch, &remote_ref, autostash)?;
        check_merge_outcome(branch, &result)?;
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

/// Validates that a *successful* merge actually did what a fast-forward
/// promises.
///
/// Two git behaviors must not be reported as success:
///
/// - An autostash whose restoration conflicts: git prints a warning on
///   stderr, keeps the stash, and still exits zero.
/// - A merge that leaves HEAD untouched (e.g. inherited
///   `branch.<name>.mergeOptions=--squash`; gitree clears that config for
///   the merge, so this indicates an unexpected configuration effect).
///
/// # Errors
///
/// Returns an error describing the inconsistency and how to recover.
fn check_merge_outcome(branch: &str, result: &MergeResult) -> Result<()> {
    if result.notes.contains("resulted in conflicts") {
        return Err(GitreeError::Other(format!(
            "fast-forwarded '{branch}', but reapplying the autostash resulted in \
             conflicts — the stash was kept; resolve it with `git stash pop` in \
             the worktree"
        )));
    }
    if result.head_before.is_some() && result.head_before == result.head_after {
        return Err(GitreeError::Other(format!(
            "merge completed but HEAD did not move — check branch.{branch}.mergeOptions \
             and inspect `git status` in the worktree"
        )));
    }
    Ok(())
}
