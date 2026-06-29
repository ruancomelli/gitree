//! `gitree migrate` — convert a regular clone into a worktree-based layout.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{GitreeError, Result};
use crate::git::Git;
use crate::init;

/// Options for `gitree migrate`.
#[derive(Debug, Clone)]
pub struct MigrateOptions {
    /// Skip confirmation prompt.
    pub yes: bool,
    /// Allow migration even with warnings (untracked files, local-only
    /// branches).
    pub force: bool,
}

/// Runs the `migrate` command.
///
/// # Errors
///
/// Returns an error if any pre-flight check fails, or if the atomic rename or
/// verification fails.
pub fn run(opts: MigrateOptions) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let git = Git::new(&cwd);

    eprintln!("Running pre-flight checks …");

    let report = preflight(&git, &cwd, &opts)?;

    print_plan(&report, &cwd);

    if !opts.yes {
        eprintln!();
        eprint!("Proceed with migration? [y/N] ");
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            eprintln!("Aborted.");
            return Ok(());
        }
    }

    eprintln!();
    eprintln!("Migrating …");

    execute(&cwd, &report)?;

    eprintln!();
    eprintln!("Migration complete.");
    eprintln!();
    eprintln!("Next steps:");
    eprintln!("  gitree add main");
    eprintln!();

    Ok(())
}

/// Pre-flight check report.
#[derive(Debug)]
struct PreflightReport {
    /// Path to the `.git` directory.
    git_dir: PathBuf,
    /// Number of untracked files.
    untracked: usize,
    /// Whether the working tree is clean.
    clean: bool,
    /// Number of stashes.
    stash_count: usize,
    /// Local-only branches (not on any remote).
    local_only_branches: Vec<String>,
    /// Size of `.git` in bytes.
    git_size: u64,
}

fn preflight(git: &Git, cwd: &Path, opts: &MigrateOptions) -> Result<PreflightReport> {
    // Check 1: must be a regular clone (.git is a directory).
    let git_dir_path = cwd.join(".git");
    if !git_dir_path.is_dir() {
        return Err(GitreeError::PreflightFailed(format!(
            "{} is not a directory — this does not appear to be a regular clone",
            git_dir_path.display()
        )));
    }

    // Check 2: no existing worktrees (regular clones shouldn't have any).
    let worktrees = git.worktree_list()?;
    let non_main = worktrees.iter().filter(|wt| !wt.bare).count();
    if non_main > 1 {
        return Err(GitreeError::PreflightFailed(format!(
            "found {non_main} worktrees — run `git worktree remove` on all worktrees before migrating"
        )));
    }

    // Check 3: working tree must be clean.
    let dirty_count = git.dirty_count()?;
    if dirty_count > 0 && !opts.force {
        return Err(GitreeError::DirtyWorktree(dirty_count));
    }

    // Check 4: list untracked files.
    let status = git.run_status_porcelain()?;
    let untracked = status.lines().filter(|line| line.starts_with("??")).count();

    if untracked > 0 && !opts.force {
        eprintln!("warning: {untracked} untracked file(s) found. Use --force to proceed.");
    }

    // Check 5: fsck.
    git.fsck().map_err(|_| {
        GitreeError::PreflightFailed(
            "git fsck failed — repository integrity is questionable".into(),
        )
    })?;

    // Check 6: local-only branches.
    let local_only = git.local_only_branches()?;
    if !local_only.is_empty() && !opts.force {
        eprintln!(
            "warning: {} local-only branch(es) not on any remote:",
            local_only.len()
        );
        for b in &local_only {
            eprintln!("  {b}");
        }
        eprintln!("Use --force to proceed anyway.");
        return Err(GitreeError::PreflightFailed(format!(
            "{} local-only branch(es) not on any remote",
            local_only.len()
        )));
    }

    // Check 7: stash count (informational).
    let stash_count = git.stash_count().unwrap_or(0);

    // Check 8: size of .git.
    let git_size = dir_size(&git_dir_path);

    Ok(PreflightReport {
        git_dir: git_dir_path,
        untracked,
        clean: dirty_count == 0,
        stash_count,
        local_only_branches: local_only,
        git_size,
    })
}

fn print_plan(report: &PreflightReport, cwd: &Path) {
    eprintln!();
    eprintln!("Migration plan:");
    eprintln!("  Source: {}", cwd.display());
    eprintln!(
        "  .git dir: {} ({})",
        report.git_dir.display(),
        format_size(report.git_size)
    );
    eprintln!(
        "  Working tree: {}",
        if report.clean { "clean" } else { "dirty" }
    );
    if report.untracked > 0 {
        eprintln!("  Untracked files: {}", report.untracked);
    }
    eprintln!("  Stashes: {} (preserved)", report.stash_count);
    if !report.local_only_branches.is_empty() {
        eprintln!("  Local-only branches:");
        for b in &report.local_only_branches {
            eprintln!("    {b}");
        }
    }
    eprintln!();
    eprintln!("  Steps:");
    eprintln!("    1. Rename .git → .bare");
    eprintln!("    2. Write .git file (gitdir: ./.bare)");
    eprintln!("    3. Create .shared/ directory");
    eprintln!("    4. Ensure .gitignore has .shared/");
    eprintln!("    5. Update remote.origin.fetch config");
    eprintln!("    6. Fetch origin");
}

fn execute(cwd: &Path, report: &PreflightReport) -> Result<()> {
    let bare_path = cwd.join(".bare");

    // Verify .bare doesn't already exist.
    if bare_path.exists() {
        return Err(GitreeError::PathExists(bare_path));
    }

    // Step 1: rename .git → .bare (atomic on same filesystem).
    fs::rename(&report.git_dir, &bare_path).map_err(|e| {
        GitreeError::PreflightFailed(format!(
            "failed to rename .git → .bare: {e}\nYour original clone is untouched."
        ))
    })?;

    // Step 2: write .git file.
    let git_file = cwd.join(".git");
    if let Err(e) = fs::write(&git_file, "gitdir: ./.bare\n") {
        // Recovery: rename .bare back to .git.
        let _ = fs::rename(&bare_path, &report.git_dir);
        return Err(GitreeError::PreflightFailed(format!(
            "failed to write .git file: {e}\nRecovery: renamed .bare → .git"
        )));
    }

    // Step 3: create .shared/.
    let shared_path = cwd.join(".shared");
    if let Err(e) = fs::create_dir_all(&shared_path) {
        let _ = fs::remove_file(&git_file);
        let _ = fs::rename(&bare_path, &report.git_dir);
        return Err(GitreeError::PreflightFailed(format!(
            "failed to create .shared/: {e}\nRecovery: removed .git file, renamed .bare → .git"
        )));
    }

    // Step 4: ensure .shared/ is gitignored.
    let gitignore = cwd.join(".gitignore");
    if let Err(e) = init::ensure_gitignore_entry(&gitignore, ".shared/") {
        eprintln!("warning: could not update .gitignore: {e}");
    }

    // Step 5: configure remote fetch refs.
    let git = Git::new(&bare_path);
    if let Err(e) = git.config_set("remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*") {
        eprintln!("warning: could not set remote.origin.fetch: {e}");
    }

    // Step 6: fetch (non-fatal if offline).
    eprintln!("Fetching …");
    if let Err(e) = git.fetch() {
        eprintln!("warning: fetch failed (continuing): {e}");
    }

    // Verify.
    let verify_git = Git::new(cwd);
    match verify_git.git_dir() {
        Ok(path) if path.ends_with(".bare") => {
            eprintln!("Verification: OK (git dir = {})", path.display());
        }
        Ok(path) => {
            return Err(GitreeError::PreflightFailed(format!(
                "verification failed: git dir is {}, expected .bare\n\
                 Recovery: run `mv .bare .git && rm .git`",
                path.display()
            )));
        }
        Err(e) => {
            return Err(GitreeError::PreflightFailed(format!(
                "verification failed: {e}\n\
                 Recovery: run `mv .bare .git && rm .git`"
            )));
        }
    }

    Ok(())
}

/// Recursively computes the size of a directory in bytes.
/// Does not follow symlinks to prevent infinite loops.
fn dir_size(path: &Path) -> u64 {
    let mut total = 0u64;
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            let path = entry.path();
            // Use symlink_metadata to avoid following symlinks.
            if let Ok(meta) = entry.metadata() {
                if meta.is_dir() && !meta.file_type().is_symlink() {
                    total += dir_size(&path);
                } else {
                    total += meta.len();
                }
            }
        }
    }
    total
}

fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB", "TiB"];
    let mut size = bytes as f64;
    let mut unit_idx = 0;
    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }
    if unit_idx == 0 {
        format!("{bytes} {}", UNITS[0])
    } else {
        format!("{size:.1} {}", UNITS[unit_idx])
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_size_bytes() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(512), "512 B");
    }

    #[test]
    fn format_size_kib() {
        assert_eq!(format_size(1024), "1.0 KiB");
        assert_eq!(format_size(1536), "1.5 KiB");
    }

    #[test]
    fn format_size_mib() {
        assert_eq!(format_size(1048576), "1.0 MiB");
    }

    #[test]
    fn dir_size_calculates() {
        let tmp = tempfile::TempDir::new().unwrap();
        fs::write(tmp.path().join("a.txt"), "hello").unwrap();
        fs::write(tmp.path().join("b.txt"), "world!").unwrap();
        let size = dir_size(tmp.path());
        assert!(size > 0);
    }

    #[test]
    fn dir_size_no_symlink_loop() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().join("dir");
        fs::create_dir(&dir).unwrap();
        // Create a symlink loop.
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&dir, dir.join("loop")).unwrap();
        }
        // Should not hang.
        let _ = dir_size(tmp.path());
    }
}
