//! Output formatting: color policy, text and JSON rendering for `gitree list`.

use std::io::Write;

use crate::git::WorktreeEntry;
use serde::{Deserialize, Serialize};

/// Color policy for gitree output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ColorPolicy {
    /// Always use color.
    Always,
    /// Never use color.
    Never,
    /// Use color only when output is a terminal.
    #[default]
    Auto,
}

impl ColorPolicy {
    /// Returns `true` if color should be used given `is_tty`.
    #[must_use]
    pub fn should_color(self, is_tty: bool) -> bool {
        match self {
            Self::Always => true,
            Self::Never => false,
            Self::Auto => is_tty,
        }
    }
}

/// A row in the worktree list output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeRow {
    /// The branch name (or `"(detached)"` if no branch).
    pub branch: String,
    /// The worktree filesystem path.
    pub path: String,
    /// The short HEAD hash.
    pub head: String,
    /// Whether the worktree has uncommitted changes.
    pub dirty: bool,
}

impl WorktreeRow {
    /// Builds a [`WorktreeRow`] from a [`WorktreeEntry`].
    #[must_use]
    pub fn from_entry(entry: &WorktreeEntry, dirty: bool) -> Self {
        Self {
            branch: entry.branch.clone().unwrap_or_else(|| "(detached)".into()),
            path: entry.path.display().to_string(),
            head: entry
                .head
                .as_deref()
                .map(|h| h.get(..7).unwrap_or(h).to_string())
                .unwrap_or_default(),
            dirty,
        }
    }
}

/// Renders the worktree list as plain text (Git-style).
pub fn render_text(rows: &[WorktreeRow], use_color: bool, out: &mut impl Write) {
    for row in rows {
        let dirty_marker = if row.dirty { " *" } else { "" };
        if use_color && row.dirty {
            let _ = writeln!(
                out,
                "{}  \x1b[33m{}\x1b[0m  {}{}",
                row.head, row.branch, row.path, dirty_marker
            );
        } else {
            let _ = writeln!(
                out,
                "{}  {}  {}{}",
                row.head, row.branch, row.path, dirty_marker
            );
        }
    }
}

/// Renders the worktree list as JSON.
pub fn render_json(rows: &[WorktreeRow], out: &mut impl Write) -> serde_json::Result<()> {
    serde_json::to_writer_pretty(out, rows)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn sample_rows() -> Vec<WorktreeRow> {
        vec![
            WorktreeRow {
                branch: "main".into(),
                path: "/home/user/project/main".into(),
                head: "abc1234".into(),
                dirty: false,
            },
            WorktreeRow {
                branch: "feature/x".into(),
                path: "/home/user/project/feature/x".into(),
                head: "def5678".into(),
                dirty: true,
            },
        ]
    }

    #[test]
    fn render_text_basic() {
        let rows = sample_rows();
        let mut buf = Vec::new();
        render_text(&rows, false, &mut buf);
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("main"));
        assert!(output.contains("feature/x"));
        assert!(output.contains("*"));
    }

    #[test]
    fn render_json_parses() {
        let rows = sample_rows();
        let mut buf = Vec::new();
        render_json(&rows, &mut buf).unwrap();
        let parsed: Vec<WorktreeRow> = serde_json::from_slice(&buf).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].branch, "main");
        assert!(parsed[1].dirty);
    }

    #[test]
    fn from_entry_with_branch() {
        let entry = WorktreeEntry {
            path: PathBuf::from("/tmp/main"),
            head: Some("abcdef1234567890".into()),
            branch: Some("main".into()),
            bare: false,
            locked: false,
        };
        let row = WorktreeRow::from_entry(&entry, false);
        assert_eq!(row.branch, "main");
        assert_eq!(row.head, "abcdef1");
        assert!(!row.dirty);
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
        let row = WorktreeRow::from_entry(&entry, true);
        assert_eq!(row.branch, "(detached)");
        assert!(row.dirty);
    }

    #[test]
    fn color_policy_should_color() {
        assert!(ColorPolicy::Always.should_color(false));
        assert!(!ColorPolicy::Never.should_color(true));
        assert!(ColorPolicy::Auto.should_color(true));
        assert!(!ColorPolicy::Auto.should_color(false));
    }
}
