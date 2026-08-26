//! `gitree migrate` — convert a regular clone into a worktree-based layout.
//!
//! Migrating a regular clone (`.git/` is a directory) into the gitree wrapper
//! layout (`.bare/` + `.git` file + `.shared/` + `<branch>/` worktrees).
//!
//! In addition to the bare-rename that the original `migrate` performed, this
//! version also relocates any existing linked worktrees into the wrapper at
//! `<wrapper>/<branch>/` and converts the main worktree into a linked
//! worktree, matching the layout produced by `gitree init` + `gitree add`.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{DirtyEscape, GitreeError, Result};
use crate::git::{Git, WorktreeEntry};
use crate::init;
use crate::shared;
use crate::types::SharedDir;

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
/// Returns an error if any pre-flight check fails, or if the relocation,
/// rename, or verification fails.
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

    execute(&git, &cwd, &report)?;

    eprintln!();
    eprintln!("Migration complete.");
    eprintln!();
    eprintln!("Next steps:");
    eprintln!("  cd {}", report.main_branch);
    for branch in linked_branches(&report) {
        eprintln!("  cd {branch}");
    }
    eprintln!();

    Ok(())
}

/// Pre-flight check report.
#[derive(Debug)]
struct PreflightReport {
    /// Path to the original `.git` directory.
    git_dir: PathBuf,
    /// Branch checked out in the main worktree.
    main_branch: String,
    /// Filesystem path of the main worktree (usually `cwd`, or a subdir if
    /// `core.worktree` was set).
    main_wt_path: PathBuf,
    /// Linked worktrees (every non-bare worktree other than the main one).
    linked_worktrees: Vec<WorktreeEntry>,
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

/// Per-worktree transient files that belong to a single worktree (not shared
/// across worktrees). These are moved from `.bare/` into the worktree's state
/// directory during the main→linked conversion.
const TRANSIENT_FILES: &[&str] = &[
    "COMMIT_EDITMSG",
    "FETCH_HEAD",
    "ORIG_HEAD",
    "REBASE_HEAD",
    "MERGE_HEAD",
    "MERGE_MSG",
    "MERGE_MODE",
    "REVERT_HEAD",
    "CHERRY_PICK_HEAD",
    "BISECT_LOG",
    "BISECT_NAMES",
    "BISECT_TERMS",
    "AUTO_GC",
];

fn preflight(git: &Git, cwd: &Path, opts: &MigrateOptions) -> Result<PreflightReport> {
    // Check 1: must be a regular clone (.git is a directory).
    let git_dir_path = cwd.join(".git");
    if !git_dir_path.is_dir() {
        return Err(GitreeError::PreflightFailed(format!(
            "{} is not a directory — this does not appear to be a regular clone",
            git_dir_path.display()
        )));
    }

    // Check 2: enumerate worktrees. Identify the main worktree (first non-bare
    // entry) and any linked worktrees.
    let worktrees = git.worktree_list()?;
    let non_bare: Vec<&WorktreeEntry> = worktrees.iter().filter(|wt| !wt.bare).collect();
    if non_bare.is_empty() {
        return Err(GitreeError::PreflightFailed(
            "no worktrees found — this does not appear to be a regular clone".into(),
        ));
    }
    let main_wt = non_bare[0];
    let main_wt_path = main_wt.path.clone();
    let main_branch = main_wt.branch.as_deref().ok_or_else(|| {
        GitreeError::PreflightFailed(
            "main worktree has a detached HEAD — cannot determine branch name \
             for relocation. Check out a branch before migrating."
                .into(),
        )
    })?;
    let linked: Vec<WorktreeEntry> = non_bare[1..].iter().map(|e| (*e).clone()).collect();

    // Check 2a: no locked worktrees — they cannot be safely relocated.
    let locked: Vec<&WorktreeEntry> = non_bare.iter().copied().filter(|wt| wt.locked).collect();
    if !locked.is_empty() {
        let names: Vec<String> = locked
            .iter()
            .filter_map(|wt| wt.branch.as_deref().map(String::from))
            .collect();
        return Err(GitreeError::PreflightFailed(format!(
            "locked worktrees cannot be migrated: {} — \
             unlock with `git worktree unlock <path>` first",
            names.join(", ")
        )));
    }

    // Check 3: working tree must be clean.
    let dirty_count = git.dirty_count()?;
    if dirty_count > 0 && !opts.force {
        return Err(GitreeError::DirtyWorktree {
            count: dirty_count,
            branch: Some(main_branch.to_string()),
            path: Some(cwd.to_path_buf()),
            escape: DirtyEscape::Force,
        });
    }

    // Check 4: list untracked files.
    let status = git.run_status_porcelain()?;
    let untracked = status.lines().filter(|line| line.starts_with("??")).count();

    if untracked > 0 && !opts.force {
        eprintln!("warning: {untracked} untracked file(s) found. Use --force to proceed.");
    }

    // Check 5: fsck.
    git.fsck().map_err(|e| {
        GitreeError::PreflightFailed(format!(
            "git fsck failed — repository integrity is questionable: {e}"
        ))
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
        main_branch: main_branch.to_string(),
        main_wt_path,
        linked_worktrees: linked,
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
    eprintln!("  Worktree relocation:");
    eprintln!(
        "    {}: {} → {}/",
        report.main_branch,
        report.main_wt_path.display(),
        cwd.join(&report.main_branch).display()
    );
    for wt in &report.linked_worktrees {
        let branch = wt.branch.as_deref().unwrap_or("?");
        let target = cwd.join(branch);
        if wt.path == target {
            eprintln!("    {branch}: {} (already in place)", wt.path.display());
        } else {
            eprintln!("    {branch}: {} → {}", wt.path.display(), target.display());
        }
    }
    eprintln!();
    eprintln!("  Steps:");
    eprintln!("    1. Relocate linked worktrees into {}/", cwd.display());
    eprintln!("    2. Rename .git → .bare");
    eprintln!("    3. Write .git file (gitdir: ./.bare)");
    eprintln!("    4. Create .shared/ directory");
    eprintln!(
        "    5. Move working files into {}/",
        cwd.join(&report.main_branch).display()
    );
    eprintln!("    6. Convert main worktree into a linked worktree");
    eprintln!("    7. Ensure .gitignore has .shared/");
    eprintln!("    8. Update remote.origin.fetch config");
    eprintln!("    9. Fetch origin");
    eprintln!("    10. Repair worktree pointers");
}

fn execute(git: &Git, cwd: &Path, report: &PreflightReport) -> Result<()> {
    let bare_path = cwd.join(".bare");

    if bare_path.exists() {
        return Err(GitreeError::PathExists(bare_path));
    }

    // Phase 1: relocate linked worktrees into cwd/<branch>/ (before the
    // rename, while `git worktree move` still operates on the live .git).
    relocate_linked_worktrees(git, cwd, report)?;

    // Phase 2: rename .git → .bare (atomic on the same filesystem).
    rename_git_to_bare(&report.git_dir, &bare_path)?;

    // Phase 3: write the .git file and create .shared/.
    write_git_file_and_shared(cwd, &bare_path)?;

    // Phase 4: convert the main worktree into a linked worktree at
    // cwd/<main_branch>/, moving working files and per-worktree state.
    let linked: Vec<&str> = linked_branches(report);
    convert_main_worktree(cwd, &bare_path, &report.main_branch, &linked)?;

    // Phase 5: ensure the wrapper-level .gitignore has .shared/.
    let gitignore = cwd.join(".gitignore");
    if let Err(e) = init::ensure_gitignore_entry(&gitignore, ".shared/") {
        eprintln!("warning: could not update .gitignore: {e}");
    }

    // Phase 6: configure the bare repo. A renamed regular clone has
    // `core.bare = false` (and possibly `core.worktree`); flip it to bare so
    // `.bare` behaves like a bare-clone's database, then configure the remote
    // fetch refs and fetch (non-fatal if offline).
    let bare_git = Git::new(&bare_path);
    if let Err(e) = bare_git.config_set("core.bare", "true") {
        eprintln!("warning: could not set core.bare: {e}");
    }
    if let Err(e) = bare_git.config_unset("core.worktree") {
        eprintln!("warning: could not unset core.worktree: {e}");
    }
    if let Err(e) =
        bare_git.config_set("remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*")
    {
        eprintln!("warning: could not set remote.origin.fetch: {e}");
    }
    eprintln!("Fetching …");
    if let Err(e) = bare_git.fetch() {
        eprintln!("warning: fetch failed (continuing): {e}");
    }

    // Phase 7: repair worktree pointers (fixes linked worktrees' .git files
    // which still reference the pre-rename .git path).
    let cwd_git = Git::new(cwd);
    if let Err(e) = cwd_git.worktree_repair() {
        eprintln!("warning: git worktree repair failed: {e}");
    }

    // Phase 8: link .shared/ items into each worktree.
    link_shared_into_worktrees(cwd, report)?;

    // Phase 9: verify the resulting layout.
    verify(cwd, report)?;

    Ok(())
}

/// Phase 1: moves each linked worktree into `cwd/<branch>/` via
/// `git worktree move`. Worktrees already at the target are skipped.
fn relocate_linked_worktrees(git: &Git, cwd: &Path, report: &PreflightReport) -> Result<()> {
    for wt in &report.linked_worktrees {
        let branch = wt.branch.as_deref().ok_or_else(|| {
            GitreeError::PreflightFailed(
                "a linked worktree has a detached HEAD — cannot determine \
                 branch name for relocation"
                    .into(),
            )
        })?;
        let target = cwd.join(branch);
        if wt.path == target {
            continue;
        }
        if target.exists() {
            return Err(GitreeError::PreflightFailed(format!(
                "target path {} already exists — cannot relocate worktree '{branch}' \
                 (move or remove it first)",
                target.display()
            )));
        }
        eprintln!(
            "Moving worktree '{branch}' {} → {} …",
            wt.path.display(),
            target.display()
        );
        git.worktree_move(&wt.path, &target)?;
    }
    Ok(())
}

/// Phase 2: renames `.git` → `.bare`, rolling back on failure.
fn rename_git_to_bare(git_dir: &Path, bare_path: &Path) -> Result<()> {
    fs::rename(git_dir, bare_path).map_err(|e| {
        GitreeError::PreflightFailed(format!(
            "failed to rename .git → .bare: {e}\nYour original clone is untouched."
        ))
    })
}

/// Phase 3: writes the `.git` file and creates `.shared/`, with rollback.
fn write_git_file_and_shared(cwd: &Path, bare_path: &Path) -> Result<()> {
    let git_file = cwd.join(".git");
    if let Err(e) = fs::write(&git_file, "gitdir: ./.bare\n") {
        let _ = fs::rename(bare_path, git_file);
        return Err(GitreeError::PreflightFailed(format!(
            "failed to write .git file: {e}\nRecovery: renamed .bare → .git"
        )));
    }

    let shared_path = cwd.join(".shared");
    if let Err(e) = fs::create_dir_all(&shared_path) {
        let _ = fs::remove_file(&git_file);
        let _ = fs::rename(bare_path, git_file);
        return Err(GitreeError::PreflightFailed(format!(
            "failed to create .shared/: {e}\nRecovery: removed .git file, renamed .bare → .git"
        )));
    }

    Ok(())
}

/// Phase 4: converts the main worktree into a linked worktree at
/// `cwd/<main_branch>/`.
///
/// This moves every top-level entry of `cwd` (except reserved names and the
/// linked worktree directories) into `cwd/<main_branch>/`, then moves the
/// per-worktree state (`index`, `logs/`, transient files) from `.bare/` into
/// `.bare/worktrees/<main_branch>/`, and writes the `commondir`/`gitdir`/`.git`
/// files that wire the worktree to the bare repo.
fn convert_main_worktree(
    cwd: &Path,
    bare: &Path,
    main_branch: &str,
    linked_branches: &[&str],
) -> Result<()> {
    let main_dir = cwd.join(main_branch);

    // Create the worktree directory.
    fs::create_dir_all(&main_dir).map_err(|e| {
        GitreeError::PreflightFailed(format!("failed to create {}: {e}", main_dir.display()))
    })?;

    // Compute the set of top-level entries to leave in place: the git
    // database, the wrapper-level git/shared dirs, and the directories that
    // hold each worktree (main + linked).
    let skip = skip_top_level_dirs(main_branch, linked_branches);

    // Move every other top-level entry into the worktree directory.
    for entry in fs::read_dir(cwd)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if skip.iter().any(|s| *s == name_str) {
            continue;
        }
        let dest = main_dir.join(&name);
        fs::rename(entry.path(), &dest).map_err(|e| {
            GitreeError::PreflightFailed(format!(
                "failed to move {} → {}: {e}\n\
                 Recovery: move files back into {parent} and run \
                 `mv .bare .git && rm .git`",
                entry.path().display(),
                dest.display(),
                parent = cwd.display()
            ))
        })?;
    }

    // Create the worktree state directory under .bare/worktrees/<basename>.
    //
    // Git names the per-worktree admin directory after the *basename* of the
    // worktree path (not the branch name), so a branch like
    // `feature/backport-adjacency-check` uses the state dir
    // `.bare/worktrees/backport-adjacency-check` — one level deep, matching
    // the `commondir: ../..` that points back at `.bare`.
    let state_name = main_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| main_branch.replace('/', "-"));
    let wt_state = bare.join("worktrees").join(&state_name);
    if wt_state.exists() {
        return Err(GitreeError::PreflightFailed(format!(
            "worktree state directory {} already exists — \
             the branch basename '{state_name}' collides with an existing \
             worktree. Rename the branch or remove the conflicting worktree.",
            wt_state.display()
        )));
    }
    fs::create_dir_all(&wt_state)?;
    fs::create_dir_all(wt_state.join("refs"))?;

    // HEAD: copy (keep .bare/HEAD as the bare repo's default-branch HEAD).
    let bare_head = bare.join("HEAD");
    if bare_head.exists() {
        fs::copy(&bare_head, wt_state.join("HEAD"))?;
    }

    // index: move (per-worktree staging area).
    let bare_index = bare.join("index");
    if bare_index.exists() {
        fs::rename(&bare_index, wt_state.join("index"))?;
    }

    // logs/: move the contents into the worktree's logs/ (only logs/HEAD is
    // truly per-worktree, but moving the whole tree preserves all reflogs).
    let bare_logs = bare.join("logs");
    let wt_logs = wt_state.join("logs");
    if bare_logs.exists() {
        fs::create_dir_all(&wt_logs)?;
        move_dir_contents(&bare_logs, &wt_logs)?;
        let _ = fs::remove_dir_all(&bare_logs);
    }

    // Transient per-worktree files.
    for transient in TRANSIENT_FILES {
        let src = bare.join(transient);
        if src.exists() {
            let _ = fs::rename(&src, wt_state.join(transient));
        }
    }

    // Write the administrative files: commondir, gitdir, and the worktree's
    // .git file. Paths are absolute (matching git's own worktree format).
    fs::write(wt_state.join("commondir"), "../..\n")?;

    let main_abs = main_dir
        .canonicalize()
        .unwrap_or_else(|_| main_dir.to_path_buf());
    fs::write(
        wt_state.join("gitdir"),
        format!("{}\n", main_abs.join(".git").display()),
    )?;

    let bare_abs = bare.canonicalize().unwrap_or_else(|_| bare.to_path_buf());
    fs::write(
        main_dir.join(".git"),
        format!(
            "gitdir: {}\n",
            bare_abs.join("worktrees").join(&state_name).display()
        ),
    )?;

    Ok(())
}

/// Top-level entries of a wrapper that must stay in place during the
/// main-worktree conversion.
const RESERVED_TOP_LEVEL: &[&str] = &[".git", ".bare", ".shared"];

/// Returns the names of top-level entries that must stay in place during the
/// main-worktree conversion: [`RESERVED_TOP_LEVEL`] plus the top-level
/// directory holding each worktree (main + linked).
fn skip_top_level_dirs(main_branch: &str, linked_branches: &[&str]) -> Vec<String> {
    let mut skip: Vec<String> = RESERVED_TOP_LEVEL
        .iter()
        .map(|name| (*name).to_string())
        .collect();
    for branch in std::iter::once(main_branch).chain(linked_branches.iter().copied()) {
        // Worktree directories live at `<wrapper>/<branch>`, so the top-level
        // entry to keep is the branch's first path component.
        let top = Path::new(branch)
            .components()
            .next()
            .filter(|c| matches!(c, std::path::Component::Normal(_)));
        if let Some(top) = top {
            skip.push(top.as_os_str().to_string_lossy().into_owned());
        }
    }
    skip
}

/// Moves every entry of `src` into `dest` (which must exist and be writable).
fn move_dir_contents(src: &Path, dest: &Path) -> Result<()> {
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let target = dest.join(entry.file_name());
        fs::rename(entry.path(), &target).map_err(|e| {
            GitreeError::PreflightFailed(format!(
                "failed to move {} → {}: {e}",
                entry.path().display(),
                target.display()
            ))
        })?;
    }
    Ok(())
}

/// Phase 8: links `.shared/` items into each worktree directory.
fn link_shared_into_worktrees(cwd: &Path, report: &PreflightReport) -> Result<()> {
    let shared = SharedDir::from_path(cwd.join(".shared"));
    if !shared.as_path().is_dir() {
        return Ok(());
    }
    for wt_path in worktree_target_paths(cwd, report) {
        let results = shared::link_shared(&shared, &wt_path)?;
        for result in results {
            match result {
                shared::LinkResult::Linked(name) => {
                    eprintln!("  {}: linked {name}", wt_path.display());
                }
                shared::LinkResult::Skipped(name) => {
                    eprintln!("  {}: skipped {name} (exists)", wt_path.display());
                }
            }
        }
    }
    Ok(())
}

/// Returns the target paths of every worktree after migration (main + linked).
fn worktree_target_paths(cwd: &Path, report: &PreflightReport) -> Vec<PathBuf> {
    let mut paths = vec![cwd.join(&report.main_branch)];
    for branch in linked_branches(report) {
        paths.push(cwd.join(branch));
    }
    paths
}

/// Phase 9: verifies that every non-bare worktree ended up at `cwd/<branch>`.
fn verify(cwd: &Path, report: &PreflightReport) -> Result<()> {
    let git = Git::new(cwd);
    let worktrees = git.worktree_list()?;
    let cwd_abs = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());

    let mut non_bare = 0;
    for wt in &worktrees {
        if wt.bare {
            continue;
        }
        non_bare += 1;
        let branch = wt.branch.as_deref().unwrap_or("?");
        let expected = cwd_abs.join(branch);
        let actual = wt.path.canonicalize().unwrap_or_else(|_| wt.path.clone());
        if actual != expected {
            return Err(GitreeError::PreflightFailed(format!(
                "verification failed: worktree '{branch}' is at {} but expected {}.\n\
                 Recovery: inspect `git worktree list` and run `git worktree repair`",
                wt.path.display(),
                expected.display()
            )));
        }
    }

    let expected_count = 1 + report.linked_worktrees.len();
    if non_bare != expected_count {
        return Err(GitreeError::PreflightFailed(format!(
            "verification failed: found {non_bare} worktree(s) but expected {expected_count}.\n\
             Recovery: inspect `git worktree list` and run `git worktree repair`"
        )));
    }

    eprintln!("Verification: OK ({non_bare} worktree(s))");
    Ok(())
}

/// Returns the branch names of all linked worktrees.
fn linked_branches(report: &PreflightReport) -> Vec<&str> {
    report
        .linked_worktrees
        .iter()
        .filter_map(|wt| wt.branch.as_deref())
        .collect()
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

    #[test]
    fn skip_top_level_dirs_includes_reserved_and_worktree_dirs() {
        let skip = skip_top_level_dirs("main", &["feature/test", "add-version-command"]);
        assert!(skip.iter().any(|s| s == ".git"));
        assert!(skip.iter().any(|s| s == ".bare"));
        assert!(skip.iter().any(|s| s == ".shared"));
        assert!(skip.iter().any(|s| s == "main"));
        assert!(skip.iter().any(|s| s == "feature"));
        assert!(skip.iter().any(|s| s == "add-version-command"));
        assert!(!skip.iter().any(|s| s == "src"));
    }

    #[test]
    fn move_dir_contents_moves_all_entries() {
        let tmp = tempfile::TempDir::new().unwrap();
        let src = tmp.path().join("src");
        let dest = tmp.path().join("dest");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&dest).unwrap();
        fs::write(src.join("a"), "1").unwrap();
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("sub").join("b"), "2").unwrap();

        move_dir_contents(&src, &dest).unwrap();

        assert!(dest.join("a").exists());
        assert!(dest.join("sub").join("b").exists());
        assert!(!src.join("a").exists());
    }
}
