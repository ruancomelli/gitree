//! `gitree init` — fresh bare-clone setup.

use std::fs;
use std::path::Path;

use crate::error::{GitreeError, Result};
use crate::git::Git;
use crate::shared;

/// Options for `gitree init`.
#[derive(Debug, Clone)]
pub struct InitOptions {
    /// The remote URL to clone from.
    pub url: String,
    /// The wrapper directory name (defaults to repo name from URL).
    pub name: Option<String>,
}

/// Executes the `init` command.
///
/// # Errors
///
/// Returns an error if the target directory exists and is non-empty, or if any
/// git/filesystem operation fails.
pub fn run(opts: InitOptions) -> Result<()> {
    let name = opts.name.clone().unwrap_or_else(|| derive_name(&opts.url));
    let wrapper_path = std::env::current_dir()?.join(&name);

    // Pre-flight: directory must not exist or be empty.
    if wrapper_path.exists() {
        let is_empty = fs::read_dir(&wrapper_path)?.next().is_none();
        if !is_empty {
            return Err(GitreeError::PathExists(wrapper_path));
        }
    }

    let bare_path = wrapper_path.join(".bare");

    eprintln!("Cloning {url} as bare into {name}/.bare …", url = opts.url);

    fs::create_dir_all(&wrapper_path)?;
    Git::clone_bare(&opts.url, &bare_path)?;

    // Write .git file pointing at .bare.
    let git_file = wrapper_path.join(".git");
    fs::write(&git_file, "gitdir: ./.bare\n")?;

    // Configure remote fetch refs.
    let git = Git::new(&bare_path);
    git.config_set("remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*")?;

    eprintln!("Fetching …");
    if let Err(e) = git.fetch() {
        eprintln!("warning: fetch failed (continuing): {e}");
    }

    // Create .shared/ directory.
    let shared_dir = wrapper_path.join(".shared");
    fs::create_dir_all(&shared_dir)?;

    // Ensure .shared/ is in .gitignore.
    let gitignore = wrapper_path.join(".gitignore");
    ensure_gitignore_entry(&gitignore, ".shared/")?;

    eprintln!();
    eprintln!("Repository initialised at: {}", wrapper_path.display());
    eprintln!();
    eprintln!("Next steps:");
    eprintln!("  cd {name}");
    eprintln!("  gitree add main");
    eprintln!();

    // Warn about gitignore trailing-slash gotchas.
    let warnings = shared::check_gitignore_trailing_slash(&gitignore)?;
    for (pattern, _) in &warnings {
        eprintln!(
            "warning: gitignore pattern '{pattern}' has a trailing slash — \
             symlinks to directories may show as untracked. Remove the slash."
        );
    }

    Ok(())
}

/// Derives a wrapper directory name from a git URL.
fn derive_name(url: &str) -> String {
    let trimmed = url.trim_end_matches('/').trim_end_matches(".git");
    let name = trimmed.rsplit(['/', ':']).next().unwrap_or("repo");
    name.to_string()
}

/// Ensures `entry` is present in the `.gitignore` file at `path`.
pub(crate) fn ensure_gitignore_entry(path: &Path, entry: &str) -> Result<()> {
    let content = if path.exists() {
        fs::read_to_string(path)?
    } else {
        String::new()
    };

    if content.lines().any(|line| line.trim() == entry) {
        return Ok(());
    }

    let mut new_content = content;
    if !new_content.is_empty() && !new_content.ends_with('\n') {
        new_content.push('\n');
    }
    new_content.push_str(entry);
    new_content.push('\n');
    fs::write(path, new_content)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_name_https() {
        assert_eq!(derive_name("https://github.com/foo/bar.git"), "bar");
    }

    #[test]
    fn derive_name_ssh() {
        assert_eq!(derive_name("git@github.com:foo/bar.git"), "bar");
    }

    #[test]
    fn derive_name_no_git_suffix() {
        assert_eq!(derive_name("https://github.com/foo/bar"), "bar");
    }

    #[test]
    fn derive_name_trailing_slash() {
        assert_eq!(derive_name("https://github.com/foo/bar.git/"), "bar");
    }

    #[test]
    fn ensure_gitignore_creates_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let gitignore = tmp.path().join(".gitignore");
        ensure_gitignore_entry(&gitignore, ".shared/").unwrap();
        let content = fs::read_to_string(&gitignore).unwrap();
        assert!(content.contains(".shared/"));
    }

    #[test]
    fn ensure_gitignore_appends() {
        let tmp = tempfile::TempDir::new().unwrap();
        let gitignore = tmp.path().join(".gitignore");
        fs::write(&gitignore, "node_modules\n").unwrap();
        ensure_gitignore_entry(&gitignore, ".shared/").unwrap();
        let content = fs::read_to_string(&gitignore).unwrap();
        assert!(content.contains("node_modules"));
        assert!(content.contains(".shared/"));
    }

    #[test]
    fn ensure_gitignore_no_duplicate() {
        let tmp = tempfile::TempDir::new().unwrap();
        let gitignore = tmp.path().join(".gitignore");
        fs::write(&gitignore, ".shared/\nnode_modules\n").unwrap();
        ensure_gitignore_entry(&gitignore, ".shared/").unwrap();
        let content = fs::read_to_string(&gitignore).unwrap();
        assert_eq!(content.matches(".shared/").count(), 1);
    }
}
