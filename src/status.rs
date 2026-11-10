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
    /// Why the counts could not be determined, if applicable. A row with
    /// an error is *not* known to be clean and synchronized — the counts
    /// are placeholders.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The live counts and any inspection failure for a single worktree.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorktreeCounts {
    /// Number of uncommitted changes.
    pub dirty: usize,
    /// Commits ahead of origin.
    pub ahead: usize,
    /// Commits behind origin.
    pub behind: usize,
    /// Why the counts could not be determined, if applicable.
    pub error: Option<String>,
}

impl StatusRow {
    /// Builds a [`StatusRow`] from a [`WorktreeEntry`].
    ///
    /// `path`, `cwd`, and `home` control how the worktree filesystem path is
    /// rendered (see [`PathPolicy::format`]). `counts` carries the worktree's
    /// change counts relative to its upstream and any inspection failure.
    #[must_use]
    pub fn from_entry(
        entry: &WorktreeEntry,
        counts: WorktreeCounts,
        path: PathPolicy,
        cwd: &Path,
        home: Option<&Path>,
    ) -> Self {
        Self {
            branch: entry.branch.clone().unwrap_or_else(|| "(detached)".into()),
            path_str: path.format(&entry.path, cwd, home),
            dirty: counts.dirty,
            ahead: counts.ahead,
            behind: counts.behind,
            error: counts.error,
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
            let mut error: Option<String> = None;

            // Individual worktrees may be unreadable (missing directory,
            // corrupt index) — the overview degrades per row instead of
            // failing wholesale, but the failure is reported.
            let dirty = match wt_git.dirty_count() {
                Ok(count) => count,
                Err(err) => {
                    error = Some(format!("could not inspect worktree: {err}"));
                    0
                }
            };

            // No upstream on origin is a normal state, not an error: the
            // counts stay zero without an error annotation.
            let (ahead, behind) = if error.is_some() {
                (0, 0)
            } else {
                match wt_git.ahead_behind(&branch) {
                    Ok(Some(counts)) => counts,
                    Ok(None) => (0, 0),
                    Err(err) => {
                        error = Some(format!("could not compare with origin: {err}"));
                        (0, 0)
                    }
                }
            };

            StatusRow::from_entry(
                e,
                WorktreeCounts {
                    dirty,
                    ahead,
                    behind,
                    error,
                },
                opts.path,
                &cwd,
                home_ref,
            )
        })
        .collect();

    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    if opts.json {
        render_json(&rows, &mut out)?;
    } else {
        let use_color = opts.color.should_color(std::io::stdout().is_terminal());
        render_text(&rows, use_color, &mut out)?;
    }

    Ok(())
}

/// Renders the status overview as plain text (Git-style).
///
/// # Errors
///
/// Returns an error if writing to the output fails.
pub fn render_text(
    rows: &[StatusRow],
    use_color: bool,
    out: &mut impl Write,
) -> std::io::Result<()> {
    for row in rows {
        let changes = (row.dirty > 0).then(|| {
            let plural = if row.dirty == 1 { "" } else { "s" };
            format!(" ({} change{plural})", row.dirty)
        });
        let offsets =
            (row.ahead > 0 || row.behind > 0).then(|| format!(" ↑{}↓{}", row.ahead, row.behind));
        let error = row.error.as_ref().map(|e| format!(" (error: {e})"));

        let paint = |code: &str, part: Option<String>| match part {
            Some(s) if use_color => format!("\x1b[{code}m{s}\x1b[0m"),
            Some(s) => s,
            None => String::new(),
        };

        writeln!(
            out,
            "{:<30} {}{}{}{}",
            row.branch,
            row.path_str,
            paint("33", changes),
            paint("36", offsets),
            paint("31", error),
        )?;
    }
    Ok(())
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
            prunable: false,
        }
    }

    #[test]
    fn from_entry_relative_path() {
        let entry = sample_entry();
        let row = StatusRow::from_entry(
            &entry,
            WorktreeCounts {
                dirty: 2,
                ahead: 0,
                behind: 4,
                error: None,
            },
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
            WorktreeCounts::default(),
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
            WorktreeCounts::default(),
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
            prunable: false,
        };
        let row = StatusRow::from_entry(
            &entry,
            WorktreeCounts::default(),
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
            error: None,
        }];
        let mut buf = Vec::new();
        render_text(&rows, false, &mut buf).unwrap();
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
            error: None,
        }];
        let mut buf = Vec::new();
        render_text(&rows, false, &mut buf).unwrap();
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
            error: None,
        }];
        let mut buf = Vec::new();
        render_text(&rows, true, &mut buf).unwrap();
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
            error: None,
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

    #[test]
    fn render_text_shows_error_row() {
        let rows = vec![StatusRow {
            branch: "main".into(),
            path_str: "main".into(),
            dirty: 0,
            ahead: 0,
            behind: 0,
            error: Some("could not inspect worktree: boom".into()),
        }];
        let mut buf = Vec::new();
        render_text(&rows, false, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("(error: could not inspect worktree: boom)"));
    }

    #[test]
    fn render_json_omits_absent_error() {
        let rows = vec![
            StatusRow {
                branch: "main".into(),
                path_str: "main".into(),
                dirty: 0,
                ahead: 0,
                behind: 0,
                error: None,
            },
            StatusRow {
                branch: "broken".into(),
                path_str: "broken".into(),
                dirty: 0,
                ahead: 0,
                behind: 0,
                error: Some("boom".into()),
            },
        ];
        let mut buf = Vec::new();
        render_json(&rows, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert!(!output.contains("\"error\": null"));
        assert!(output.contains("\"error\": \"boom\""));
    }
}
