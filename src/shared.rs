//! `.shared/` symlink fan-out and gitignore gotcha detection.

use std::fs;
use std::path::Path;

use crate::error::{GitreeError, Result};
use crate::types::SharedDir;

/// Result of linking a single shared item.
#[derive(Debug, Clone)]
pub enum LinkResult {
    /// The symlink was created.
    Linked(String),
    /// The target already exists in the worktree.
    Skipped(String),
}

/// Links all items from `.shared/` into `worktree_path`.
///
/// # Errors
///
/// Returns an error if the `.shared/` directory cannot be read.
pub fn link_shared(shared: &SharedDir, worktree_path: &Path) -> Result<Vec<LinkResult>> {
    if !shared.as_path().is_dir() {
        return Ok(Vec::new());
    }

    let mut results = Vec::new();
    let entries = fs::read_dir(shared.as_path())?;

    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        let source = entry.path();
        let target = worktree_path.join(&name);

        if target.exists() || target.is_symlink() {
            results.push(LinkResult::Skipped(name_str.into_owned()));
        } else {
            let abs_source = fs::canonicalize(&source).unwrap_or(source);
            symlink(&abs_source, &target)?;
            results.push(LinkResult::Linked(name_str.into_owned()));
        }
    }

    Ok(results)
}

/// Checks `.gitignore` files for patterns with trailing slashes that could
/// affect symlinked entries.
///
/// # Errors
///
/// Returns an error if the gitignore file cannot be read.
pub fn check_gitignore_trailing_slash(
    gitignore: &Path,
) -> Result<Vec<(String, std::path::PathBuf)>> {
    if !gitignore.exists() {
        return Ok(Vec::new());
    }

    let content = fs::read_to_string(gitignore)?;
    let mut warnings = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.ends_with('/') {
            warnings.push((trimmed.to_string(), gitignore.to_path_buf()));
        }
    }

    Ok(warnings)
}

/// Creates a symlink, working on both Unix and Windows.
#[cfg(unix)]
fn symlink(source: &Path, target: &Path) -> Result<()> {
    std::os::unix::fs::symlink(source, target).map_err(GitreeError::from)
}

#[cfg(windows)]
fn symlink(source: &Path, target: &Path) -> Result<()> {
    if source.is_dir() {
        std::os::windows::fs::symlink_dir(source, target).map_err(GitreeError::from)
    } else {
        std::os::windows::fs::symlink_file(source, target).map_err(GitreeError::from)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn link_shared_creates_symlinks() {
        let tmp = TempDir::new().unwrap();
        let shared_dir = tmp.path().join(".shared");
        fs::create_dir(&shared_dir).unwrap();
        fs::write(shared_dir.join(".env"), "FOO=bar\n").unwrap();
        fs::create_dir(shared_dir.join(".vscode")).unwrap();

        let worktree = tmp.path().join("main");
        fs::create_dir(&worktree).unwrap();

        let shared = SharedDir::from_path(&shared_dir);
        let results = link_shared(&shared, &worktree).unwrap();

        assert_eq!(results.len(), 2);
        assert!(
            results
                .iter()
                .any(|r| matches!(r, LinkResult::Linked(n) if n == ".env"))
        );
        assert!(
            results
                .iter()
                .any(|r| matches!(r, LinkResult::Linked(n) if n == ".vscode"))
        );

        assert!(worktree.join(".env").is_symlink());
        assert!(worktree.join(".vscode").is_symlink());
    }

    #[test]
    fn link_shared_skips_existing() {
        let tmp = TempDir::new().unwrap();
        let shared_dir = tmp.path().join(".shared");
        fs::create_dir(&shared_dir).unwrap();
        fs::write(shared_dir.join(".env"), "FOO=bar\n").unwrap();

        let worktree = tmp.path().join("main");
        fs::create_dir(&worktree).unwrap();
        fs::write(worktree.join(".env"), "EXISTING\n").unwrap();

        let shared = SharedDir::from_path(&shared_dir);
        let results = link_shared(&shared, &worktree).unwrap();

        assert_eq!(results.len(), 1);
        assert!(
            results
                .iter()
                .all(|r| matches!(r, LinkResult::Skipped(n) if n == ".env"))
        );
    }

    #[test]
    fn link_shared_no_dir() {
        let tmp = TempDir::new().unwrap();
        let shared = SharedDir::from_path(tmp.path().join(".shared"));
        let results = link_shared(&shared, tmp.path()).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn check_gitignore_trailing_slash_detected() {
        let tmp = TempDir::new().unwrap();
        let gitignore = tmp.path().join(".gitignore");
        fs::write(&gitignore, ".myconfig/\n.env\nnode_modules/\n").unwrap();

        let warnings = check_gitignore_trailing_slash(&gitignore).unwrap();
        assert_eq!(warnings.len(), 2);
        assert!(warnings.iter().any(|(p, _)| p == ".myconfig/"));
        assert!(warnings.iter().any(|(p, _)| p == "node_modules/"));
    }

    #[test]
    fn check_gitignore_no_trailing_slash() {
        let tmp = TempDir::new().unwrap();
        let gitignore = tmp.path().join(".gitignore");
        fs::write(&gitignore, ".env\nnode_modules\n*.log\n").unwrap();

        let warnings = check_gitignore_trailing_slash(&gitignore).unwrap();
        assert!(warnings.is_empty());
    }

    #[test]
    fn check_gitignore_nonexistent() {
        let tmp = TempDir::new().unwrap();
        let warnings = check_gitignore_trailing_slash(&tmp.path().join(".gitignore")).unwrap();
        assert!(warnings.is_empty());
    }
}
