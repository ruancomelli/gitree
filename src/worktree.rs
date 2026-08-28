//! `gitree add`, `gitree remove`, `gitree list`, `gitree prune`, `gitree where`.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};

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
            None => Some(determine_base_ref(
                &git,
                wrapper,
                &std::env::current_dir()?,
            )?),
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
fn determine_base_ref(git: &Git, wrapper: &Wrapper, cwd: &Path) -> Result<String> {
    // Canonicalize so paths reached through symlinks still compare equal to
    // the wrapper root.
    let resolve = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());

    // If CWD is inside a worktree (not the wrapper itself), use HEAD.
    if resolve(cwd) != resolve(wrapper.path())
        && let Ok(head) = Git::new(cwd).run_rev_parse_head()
    {
        return Ok(head);
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
    /// The branch names.
    pub branches: Vec<String>,
    /// Also delete the local branch after removing the worktree.
    pub delete_branch: bool,
    /// Force removal even if the worktree is dirty.
    pub force: bool,
}

/// Runs the `remove` command.
///
/// Removes one worktree per branch, stopping at the first failure.
///
/// # Errors
///
/// Returns an error if a worktree doesn't exist or git fails.
pub fn run_remove(wrapper: &Wrapper, opts: RemoveOptions) -> Result<()> {
    opts.branches
        .iter()
        .try_for_each(|branch| remove_one(wrapper, branch, &opts))?;
    eprintln!("Done.");
    Ok(())
}

fn remove_one(wrapper: &Wrapper, branch: &str, opts: &RemoveOptions) -> Result<()> {
    let branch = wrapper.resolve_branch_arg(branch)?;
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
/// Reports which stale worktree references were removed, or a friendly
/// note when there were none.
///
/// # Errors
///
/// Returns an error if git fails.
pub fn run_prune(wrapper: &Wrapper) -> Result<()> {
    let git = wrapper.git();
    let stale: Vec<PathBuf> = git
        .worktree_list()?
        .iter()
        // Locked worktrees are never removed by `git worktree prune`.
        .filter(|e| e.prunable && !e.locked)
        .map(|e| e.path.clone())
        .collect();

    git.worktree_prune()?;

    if stale.is_empty() {
        eprintln!("No stale worktree references.");
        return Ok(());
    }

    eprintln!("Pruned stale worktree references:");
    for path in &stale {
        eprintln!("  {}", path.display());
    }
    Ok(())
}

// -----------------------------------------------------------------------
// where (path lookup)
// -----------------------------------------------------------------------

/// Runs the `where` command — prints the path of a worktree for the given
/// branch.
///
/// Accepts plain branch names, directory-style names (`branch/`), and
/// worktree paths.
///
/// # Errors
///
/// Returns an error if no worktree exists for the branch.
pub fn run_where(wrapper: &Wrapper, branch: &str) -> Result<()> {
    let branch = wrapper.resolve_branch_arg(branch)?;
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
    use std::fs;

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

    /// Runs a git command, returning trimmed stdout and failing on error.
    ///
    /// Prepends `-c commit.gpgsign=false` so no test commit ever touches a
    /// signing agent, regardless of repo or global git config.
    fn run_git(dir: &Path, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .current_dir(dir)
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    /// Builds a minimal wrapper whose `.bare` is empty (no branches).
    fn wrapper_with_empty_bare(root: &Path) -> Wrapper {
        fs::create_dir_all(root.join(".bare")).unwrap();
        fs::write(root.join(".git"), "gitdir: ./.bare\n").unwrap();
        Wrapper::from_cwd(root).unwrap()
    }

    #[test]
    fn base_ref_uses_head_inside_linked_directory() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("proj");
        let wrapper = wrapper_with_empty_bare(&root);

        // A worktree directory holding an independent checkout.
        let wt = root.join("main");
        fs::create_dir_all(&wt).unwrap();
        run_git(&wt, &["init", "--initial-branch=main"]);
        run_git(&wt, &["config", "user.email", "t@example.com"]);
        run_git(&wt, &["config", "user.name", "Test"]);
        fs::write(wt.join("f"), "x").unwrap();
        run_git(&wt, &["add", "."]);
        run_git(&wt, &["commit", "-m", "initial"]);
        let expected = run_git(&wt, &["rev-parse", "HEAD"]);

        // Reach the same directory through a symlink into the wrapper.
        #[cfg(unix)]
        {
            let link = tmp.path().join("link");
            std::os::unix::fs::symlink(&root, &link).unwrap();
            let got = determine_base_ref(&wrapper.git(), &wrapper, &link.join("main"));
            assert_eq!(got.unwrap(), expected);
        }
        #[cfg(not(unix))]
        assert_eq!(
            determine_base_ref(&wrapper.git(), &wrapper, &wt).unwrap(),
            expected
        );
    }

    #[test]
    fn base_ref_errors_when_no_main_or_master() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("proj");

        // A working repo whose only branch is not main/master.
        let seed = tmp.path().join("seed");
        fs::create_dir_all(&seed).unwrap();
        run_git(&seed, &["init", "--initial-branch=dev"]);
        run_git(&seed, &["config", "user.email", "t@example.com"]);
        run_git(&seed, &["config", "user.name", "Test"]);
        fs::write(seed.join("f"), "x").unwrap();
        run_git(&seed, &["add", "."]);
        run_git(&seed, &["commit", "-m", "initial"]);

        // Promote its database into the wrapper's bare dir.
        fs::create_dir_all(&root).unwrap();
        fs::rename(seed.join(".git"), root.join(".bare")).unwrap();
        fs::write(root.join(".git"), "gitdir: ./.bare\n").unwrap();
        let wrapper = Wrapper::from_cwd(&root).unwrap();

        let err = determine_base_ref(&wrapper.git(), &wrapper, &root).expect_err("must fail");
        assert!(
            err.to_string().contains("cannot determine base branch"),
            "unexpected error: {err}"
        );
    }
}
