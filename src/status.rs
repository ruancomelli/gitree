//! `gitree status` — overview of all worktrees.

use std::io::Write;

use crate::error::Result;
use crate::repo::Wrapper;

/// A status row for a single worktree.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StatusRow {
    /// The branch name.
    pub branch: String,
    /// The worktree path.
    pub path: String,
    /// Number of uncommitted changes.
    pub dirty: usize,
    /// Commits ahead of origin.
    pub ahead: usize,
    /// Commits behind origin.
    pub behind: usize,
}

/// Runs the `status` command.
///
/// # Errors
///
/// Returns an error if git fails.
pub fn run(wrapper: &Wrapper) -> Result<()> {
    let git = wrapper.git();
    let entries = git.worktree_list()?;

    let rows: Vec<StatusRow> = entries
        .iter()
        .filter(|e| !e.bare)
        .map(|e| {
            let wt_git = wrapper.git_for(e.path.as_path());
            let branch = e.branch.clone().unwrap_or_else(|| "(detached)".into());
            let dirty = wt_git.dirty_count().unwrap_or(0);
            let (ahead, behind) = wt_git.ahead_behind(&branch).unwrap_or((0, 0));
            StatusRow {
                branch,
                path: e.path.display().to_string(),
                dirty,
                ahead,
                behind,
            }
        })
        .collect();

    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    for row in &rows {
        let dirty_marker = if row.dirty > 0 {
            format!(
                " ({} change{})",
                row.dirty,
                if row.dirty == 1 { "" } else { "s" }
            )
        } else {
            String::new()
        };
        let ab = if row.ahead > 0 || row.behind > 0 {
            format!(" ↑{}↓{}", row.ahead, row.behind)
        } else {
            String::new()
        };
        let _ = writeln!(out, "{:<30} {}{}{}", row.branch, row.path, dirty_marker, ab);
    }

    Ok(())
}
