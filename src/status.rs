//! `gitree status` — overview of all worktrees.

use std::io::{IsTerminal, Write};
use std::path::Path;

use crate::error::Result;
use crate::format::{ColorPolicy, PathPolicy};
use crate::git::WorktreeEntry;
use crate::repo::Wrapper;

/// Options for `gitree status`.
#[derive(Debug, Clone)]
pub struct StatusOptions {
    /// Output as JSON.
    pub json: bool,
    /// Color policy.
    pub color: ColorPolicy,
    /// Path display policy.
    pub path: PathPolicy,
}

impl Default for StatusOptions {
    fn default() -> Self {
        Self {
            json: false,
            color: ColorPolicy::Auto,
            path: PathPolicy::Relative,
        }
    }
}

/// A status row for a single worktree.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StatusRow {
    /// The branch name.
    pub branch: String,
    /// The worktree filesystem path, formatted per the requested [`PathPolicy`].
    #[serde(rename = "path")]
    pub path_str: String,
    /// Number of uncommitted changes.
    pub dirty: usize,
    /// Commits ahead of origin.
    pub ahead: usize,
    /// Commits behind origin.
    pub behind: usize,
}

impl StatusRow {
    /// Builds a [`StatusRow`] from a [`WorktreeEntry`].
    ///
    /// `path`, `cwd`, and `home` control how the worktree filesystem path is
    /// rendered (see [`PathPolicy::format`]). `dirty`, `ahead`, and `behind`
    /// are the worktree's change counts relative to its upstream.
    #[must_use]
    pub fn from_entry(
        entry: &WorktreeEntry,
        dirty: usize,
        ahead: usize,
        behind: usize,
        path: PathPolicy,
        cwd: &Path,
        home: Option<&Path>,
    ) -> Self {
        Self {
            branch: entry.branch.clone().unwrap_or_else(|| "(detached)".into()),
            path_str: path.format(&entry.path, cwd, home),
            dirty,
            ahead,
            behind,
        }
    }
}

/// Runs the `status` command.
///
/// # Errors
///
/// Returns an error if git fails.
pub fn run(wrapper: &Wrapper, opts: StatusOptions) -> Result<()> {
    let git = wrapper.git();
    let entries = git.worktree_list()?;

    let cwd = std::env::current_dir()?;
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let home_ref = home.as_deref();

    let rows: Vec<StatusRow> = entries
        .iter()
        .filter(|e| !e.bare)
        .map(|e| {
            let wt_git = wrapper.git_for(e.path.as_path());
            let branch = e.branch.clone().unwrap_or_else(|| "(detached)".into());
            let dirty = wt_git.dirty_count().unwrap_or(0);
            let (ahead, behind) = wt_git.ahead_behind(&branch).unwrap_or((0, 0));
            StatusRow::from_entry(e, dirty, ahead, behind, opts.path, &cwd, home_ref)
        })
        .collect();

    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    if opts.json {
        render_json(&rows, &mut out)?;
    } else {
        let use_color = opts.color.should_color(std::io::stdout().is_terminal());
        render_text(&rows, use_color, &mut out);
    }

    Ok(())
}

/// Renders the status overview as plain text (Git-style).
pub fn render_text(rows: &[StatusRow], use_color: bool, out: &mut impl Write) {
    for row in rows {
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

        if use_color {
            let dirty_part = if row.dirty > 0 {
                format!("\x1b[33m{dirty_marker}\x1b[0m")
            } else {
                dirty_marker
            };
            let ab_part = if row.ahead > 0 || row.behind > 0 {
                format!("\x1b[36m{ab}\x1b[0m")
            } else {
                ab
            };
            let _ = writeln!(
                out,
                "{:<30} {}{}{}",
                row.branch, row.path_str, dirty_part, ab_part
            );
        } else {
            let _ = writeln!(
                out,
                "{:<30} {}{}{}",
                row.branch, row.path_str, dirty_marker, ab
            );
        }
    }
}

/// Renders the status overview as JSON.
pub fn render_json(rows: &[StatusRow], out: &mut impl Write) -> serde_json::Result<()> {
    serde_json::to_writer_pretty(out, rows)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn sample_entry() -> WorktreeEntry {
        WorktreeEntry {
            path: PathBuf::from("/home/user/project/main"),
            head: Some("abcdef1234567890".into()),
            branch: Some("main".into()),
            bare: false,
            locked: false,
        }
    }

    #[test]
    fn from_entry_relative_path() {
        let entry = sample_entry();
        let row = StatusRow::from_entry(
            &entry,
            2,
            0,
            4,
            PathPolicy::Relative,
            Path::new("/home/user/project"),
            None,
        );
        assert_eq!(row.branch, "main");
        assert_eq!(row.path_str, "main");
        assert_eq!(row.dirty, 2);
        assert_eq!(row.ahead, 0);
        assert_eq!(row.behind, 4);
    }

    #[test]
    fn from_entry_absolute_path() {
        let entry = sample_entry();
        let row = StatusRow::from_entry(
            &entry,
            0,
            0,
            0,
            PathPolicy::Absolute,
            Path::new("/tmp"),
            None,
        );
        assert_eq!(row.path_str, "/home/user/project/main");
    }

    #[test]
    fn from_entry_abbreviated_path() {
        let entry = sample_entry();
        let row = StatusRow::from_entry(
            &entry,
            0,
            0,
            0,
            PathPolicy::Abbreviated,
            Path::new("/tmp"),
            Some(Path::new("/home/user")),
        );
        assert_eq!(row.path_str, "~/project/main");
    }

    #[test]
    fn from_entry_detached() {
        let entry = WorktreeEntry {
            path: PathBuf::from("/tmp/detached"),
            head: Some("abcdef1234567890".into()),
            branch: None,
            bare: false,
            locked: false,
        };
        let row = StatusRow::from_entry(
            &entry,
            0,
            0,
            0,
            PathPolicy::Absolute,
            Path::new("/tmp"),
            None,
        );
        assert_eq!(row.branch, "(detached)");
    }

    #[test]
    fn render_text_clean() {
        let rows = vec![StatusRow {
            branch: "main".into(),
            path_str: "main".into(),
            dirty: 0,
            ahead: 0,
            behind: 0,
        }];
        let mut buf = Vec::new();
        render_text(&rows, false, &mut buf);
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("main"));
        assert!(!output.contains("change"));
        assert!(!output.contains("↑"));
    }

    #[test]
    fn render_text_dirty_and_ahead_behind() {
        let rows = vec![StatusRow {
            branch: "feature".into(),
            path_str: "feature".into(),
            dirty: 3,
            ahead: 1,
            behind: 2,
        }];
        let mut buf = Vec::new();
        render_text(&rows, false, &mut buf);
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("(3 changes)"));
        assert!(output.contains("↑1↓2"));
    }

    #[test]
    fn render_text_color_escapes_dirty_and_ab() {
        let rows = vec![StatusRow {
            branch: "feature".into(),
            path_str: "feature".into(),
            dirty: 1,
            ahead: 1,
            behind: 0,
        }];
        let mut buf = Vec::new();
        render_text(&rows, true, &mut buf);
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("\x1b[33m"));
        assert!(output.contains("\x1b[36m"));
    }

    #[test]
    fn render_json_parses() {
        let rows = vec![StatusRow {
            branch: "main".into(),
            path_str: "main".into(),
            dirty: 1,
            ahead: 2,
            behind: 3,
        }];
        let mut buf = Vec::new();
        render_json(&rows, &mut buf).unwrap();
        let parsed: Vec<StatusRow> = serde_json::from_slice(&buf).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].branch, "main");
        assert_eq!(parsed[0].dirty, 1);
        assert_eq!(parsed[0].ahead, 2);
        assert_eq!(parsed[0].behind, 3);
    }
}
