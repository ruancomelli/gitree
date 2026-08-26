//! `gitree add`, `gitree remove`, `gitree list`, `gitree prune`, `gitree where`.

use std::io::IsTerminal;

use crate::error::{GitreeError, Result};
use crate::format::{self, ColorPolicy, PathPolicy, WorktreeRow};
use crate::git::Git;
use crate::repo::Wrapper;
use crate::shared;
use crate::types::BranchName;

// -----------------------------------------------------------------------
// add
// -----------------------------------------------------------------------

/// Options for `gitree add`.
#[derive(Debug, Clone)]
pub struct AddOptions {
    /// The branch name.
    pub branch: String,
    /// Whether to create a new branch.
    pub new: bool,
    /// The base ref when creating a new branch.
    pub base: Option<String>,
}

/// Runs the `add` command.
///
/// # Errors
///
/// Returns an error if the branch already has a worktree, if the branch
/// doesn't exist and `--new` was not passed, or if git or filesystem
/// operations fail.
pub fn run_add(wrapper: &Wrapper, opts: AddOptions) -> Result<()> {
    let branch = BranchName::new(&opts.branch)?;
    let branch_str = branch.as_str();
    let git = wrapper.git();

    // Check if a worktree already exists for this branch.
    let worktrees = git.worktree_list()?;
    if worktrees
        .iter()
        .any(|wt| wt.branch.as_deref() == Some(branch_str))
    {
        return Err(GitreeError::WorktreeExists(opts.branch));
    }

    let worktree_path = wrapper.worktree_path(branch_str);

    // Determine the base ref for `--new`.
    let base_ref: Option<String> = if opts.new {
        match &opts.base {
            Some(b) => Some(b.clone()),
            None => Some(determine_base_ref(&git, wrapper)?),
        }
    } else {
        // For existing branch: if only remote, create a tracking local.
        let branches = git.branches()?;
        let has_local = branches.local.iter().any(|b| b == branch_str);
        let has_remote = branches.remote.iter().any(|b| b == branch_str);
        if !has_local && !has_remote {
            return Err(GitreeError::BranchNotFound(opts.branch));
        }
        None
    };

    eprintln!(
        "Adding worktree for branch '{branch}' at {path} …",
        branch = branch_str,
        path = worktree_path.as_path().display()
    );

    git.worktree_add(
        worktree_path.as_path(),
        branch_str,
        opts.new,
        base_ref.as_deref(),
    )?;

    // Link .shared/ items.
    if wrapper.has_shared_dir() {
        let results = shared::link_shared(&wrapper.shared_dir(), worktree_path.as_path())?;
        for result in &results {
            match result {
                shared::LinkResult::Linked(name) => eprintln!("  Linked: {name}"),
                shared::LinkResult::Skipped(name) => eprintln!("  Skipped (exists): {name}"),
            }
        }
    }

    // Check gitignore for trailing-slash gotchas.
    let gitignore = worktree_path.as_path().join(".gitignore");
    let warnings = shared::check_gitignore_trailing_slash(&gitignore)?;
    for (pattern, _) in &warnings {
        eprintln!(
            "warning: gitignore pattern '{pattern}' has a trailing slash — \
             symlinks to directories may show as untracked. Remove the slash."
        );
    }

    eprintln!();
    eprintln!("Worktree ready: {}", worktree_path.as_path().display());
    Ok(())
}

/// Determines the base ref for a new branch.
///
/// DWIM logic:
/// - If inside a worktree, use its HEAD.
/// - If at the wrapper level, use `main` (or `master` if `main` doesn't exist).
fn determine_base_ref(git: &Git, wrapper: &Wrapper) -> Result<String> {
    let cwd = std::env::current_dir()?;

    // If CWD is inside a worktree (not the wrapper itself), use HEAD.
    if cwd != *wrapper.path() {
        let worktree_git = Git::new(&cwd);
        if let Ok(head) = worktree_git.run_rev_parse_head() {
            return Ok(head);
        }
    }

    // At wrapper level: prefer main, fallback to master.
    let branches = git.local_branches()?;
    if branches.iter().any(|b| b == "main") {
        return Ok("main".into());
    }
    if branches.iter().any(|b| b == "master") {
        return Ok("master".into());
    }

    Err(GitreeError::Other(
        "cannot determine base branch: neither 'main' nor 'master' exists. Use --base <ref>."
            .into(),
    ))
}

// -----------------------------------------------------------------------
// remove
// -----------------------------------------------------------------------

/// Options for `gitree remove`.
#[derive(Debug, Clone)]
pub struct RemoveOptions {
    /// The branch name.
    pub branch: String,
    /// Also delete the local branch after removing the worktree.
    pub delete_branch: bool,
    /// Force removal even if the worktree is dirty.
    pub force: bool,
}

/// Runs the `remove` command.
///
/// # Errors
///
/// Returns an error if the worktree doesn't exist or git fails.
pub fn run_remove(wrapper: &Wrapper, opts: RemoveOptions) -> Result<()> {
    let branch = BranchName::new(&opts.branch)?;
    let worktree_path = wrapper.worktree_path(branch.as_str());

    if !worktree_path.as_path().exists() {
        return Err(GitreeError::PathMissing(worktree_path.into_pathbuf()));
    }

    eprintln!("Removing worktree: {}", worktree_path.as_path().display());
    let git = wrapper.git();
    git.worktree_remove(worktree_path.as_path(), opts.force)?;

    if opts.delete_branch {
        eprintln!("Deleting branch '{branch}' …");
        git.branch_delete(branch.as_str(), opts.force)?;
    }

    eprintln!("Done.");
    Ok(())
}

// -----------------------------------------------------------------------
// list
// -----------------------------------------------------------------------

/// Options for `gitree list`.
#[derive(Debug, Clone)]
pub struct ListOptions {
    /// Output as JSON.
    pub json: bool,
    /// Color policy.
    pub color: ColorPolicy,
    /// Path display policy.
    pub path: PathPolicy,
}

impl Default for ListOptions {
    fn default() -> Self {
        Self {
            json: false,
            color: ColorPolicy::Auto,
            path: PathPolicy::Relative,
        }
    }
}

/// Runs the `list` command.
///
/// # Errors
///
/// Returns an error if git fails.
pub fn run_list(wrapper: &Wrapper, opts: ListOptions) -> Result<()> {
    let git = wrapper.git();
    let entries = git.worktree_list()?;

    let cwd = std::env::current_dir()?;
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let home_ref = home.as_deref();

    let rows: Vec<WorktreeRow> = entries
        .iter()
        .filter(|e| !e.bare)
        .map(|e| {
            let dirty = wrapper
                .git_for(e.path.as_path())
                .is_dirty()
                .unwrap_or(false);
            WorktreeRow::from_entry(e, dirty, opts.path, &cwd, home_ref)
        })
        .collect();

    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    if opts.json {
        format::render_json(&rows, &mut out)?;
    } else {
        let use_color = opts.color.should_color(std::io::stdout().is_terminal());
        format::render_text(&rows, use_color, &mut out);
    }

    Ok(())
}

// -----------------------------------------------------------------------
// prune
// -----------------------------------------------------------------------

/// Runs the `prune` command.
///
/// # Errors
///
/// Returns an error if git fails.
pub fn run_prune(wrapper: &Wrapper) -> Result<()> {
    wrapper.git().worktree_prune()?;
    eprintln!("Pruned stale worktree references.");
    Ok(())
}

// -----------------------------------------------------------------------
// where (path lookup)
// -----------------------------------------------------------------------

/// Runs the `where` command — prints the path of a worktree for the given
/// branch.
///
/// # Errors
///
/// Returns an error if no worktree exists for the branch.
pub fn run_where(wrapper: &Wrapper, branch: &str) -> Result<()> {
    let branch = BranchName::new(branch)?;
    let path = wrapper.worktree_path(branch.as_str());
    if !path.as_path().exists() {
        return Err(GitreeError::PathMissing(path.into_pathbuf()));
    }
    println!("{}", path.as_path().display());
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_options_construction() {
        let opts = AddOptions {
            branch: "feature/test".into(),
            new: false,
            base: None,
        };
        assert_eq!(opts.branch, "feature/test");
        assert!(!opts.new);
    }

    #[test]
    fn list_options_default() {
        let opts = ListOptions::default();
        assert!(!opts.json);
        assert_eq!(opts.color, ColorPolicy::Auto);
        assert_eq!(opts.path, PathPolicy::Relative);
    }
}
