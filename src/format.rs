//! Output formatting: color policy, path policy, text and JSON rendering for `gitree list`.

use std::io::Write;
use std::path::Path;

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

/// How worktree paths are displayed by `gitree list`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PathPolicy {
    /// Paths relative to the current working directory.
    #[default]
    Relative,
    /// Absolute paths.
    Absolute,
    /// Absolute paths with the user's home directory shown as `~`.
    Abbreviated,
}

impl PathPolicy {
    /// Formats `path` for display according to this policy.
    ///
    /// `cwd` is the directory the path should be made relative to when the
    /// policy is [`PathPolicy::Relative`]. `home` is the directory that will
    /// be replaced by `~` when the policy is [`PathPolicy::Abbreviated`].
    ///
    /// When `Relative` and `path` is not reachable from `cwd` (e.g. they
    /// share no common ancestor on Windows), the absolute path is returned
    /// as a fallback.
    #[must_use]
    pub fn format(self, path: &Path, cwd: &Path, home: Option<&Path>) -> String {
        match self {
            Self::Relative => pathdiff::diff_paths(path, cwd)
                .map(|rel| {
                    if rel.as_os_str().is_empty() {
                        ".".into()
                    } else {
                        rel.display().to_string()
                    }
                })
                .unwrap_or_else(|| path.display().to_string()),
            Self::Absolute => path.display().to_string(),
            Self::Abbreviated => match home {
                Some(home) if path.starts_with(home) => {
                    let rest = path.strip_prefix(home).unwrap_or(path);
                    if rest.as_os_str().is_empty() {
                        "~".into()
                    } else {
                        format!("~/{}", rest.display())
                    }
                }
                _ => path.display().to_string(),
            },
        }
    }
}

/// A row in the worktree list output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeRow {
    /// The branch name (or `"(detached)"` if no branch).
    pub branch: String,
    /// The worktree filesystem path, formatted per the requested [`PathPolicy`].
    #[serde(rename = "path")]
    pub path_str: String,
    /// The short HEAD hash.
    pub head: String,
    /// Whether the worktree has uncommitted changes.
    pub dirty: bool,
}

impl WorktreeRow {
    /// Builds a [`WorktreeRow`] from a [`WorktreeEntry`].
    ///
    /// `path`, `cwd`, and `home` control how the worktree filesystem path is
    /// rendered (see [`PathPolicy::format`]).
    #[must_use]
    pub fn from_entry(
        entry: &WorktreeEntry,
        dirty: bool,
        path: PathPolicy,
        cwd: &Path,
        home: Option<&Path>,
    ) -> Self {
        Self {
            branch: entry.branch.clone().unwrap_or_else(|| "(detached)".into()),
            path_str: path.format(&entry.path, cwd, home),
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
        let branch = if use_color && row.dirty {
            format!("\x1b[33m{}\x1b[0m", row.branch)
        } else {
            row.branch.clone()
        };
        let _ = writeln!(
            out,
            "{}  {branch}  {}{dirty_marker}",
            row.head, row.path_str
        );
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
                path_str: "/home/user/project/main".into(),
                head: "abc1234".into(),
                dirty: false,
            },
            WorktreeRow {
                branch: "feature/x".into(),
                path_str: "/home/user/project/feature/x".into(),
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
            prunable: false,
        };
        let row =
            WorktreeRow::from_entry(&entry, false, PathPolicy::Absolute, Path::new("/tmp"), None);
        assert_eq!(row.branch, "main");
        assert_eq!(row.head, "abcdef1");
        assert_eq!(row.path_str, "/tmp/main");
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
            prunable: false,
        };
        let row =
            WorktreeRow::from_entry(&entry, true, PathPolicy::Absolute, Path::new("/tmp"), None);
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

    #[test]
    fn path_policy_relative_subdir() {
        let p = PathPolicy::Relative.format(
            Path::new("/home/user/project/main"),
            Path::new("/home/user/project"),
            None,
        );
        assert_eq!(p, "main");
    }

    #[test]
    fn path_policy_relative_cwd_itself() {
        let p = PathPolicy::Relative.format(
            Path::new("/home/user/project"),
            Path::new("/home/user/project"),
            None,
        );
        assert_eq!(p, ".");
    }

    #[test]
    fn path_policy_relative_sibling() {
        let p = PathPolicy::Relative.format(
            Path::new("/home/user/project/main"),
            Path::new("/home/user/project/feature"),
            None,
        );
        assert_eq!(p, "../main");
    }

    #[test]
    fn path_policy_absolute() {
        let p = PathPolicy::Absolute.format(
            Path::new("/home/user/project/main"),
            Path::new("/tmp"),
            None,
        );
        assert_eq!(p, "/home/user/project/main");
    }

    #[test]
    fn path_policy_abbreviated_under_home() {
        let p = PathPolicy::Abbreviated.format(
            Path::new("/home/user/project/main"),
            Path::new("/tmp"),
            Some(Path::new("/home/user")),
        );
        assert_eq!(p, "~/project/main");
    }

    #[test]
    fn path_policy_abbreviated_home_itself() {
        let p = PathPolicy::Abbreviated.format(
            Path::new("/home/user"),
            Path::new("/tmp"),
            Some(Path::new("/home/user")),
        );
        assert_eq!(p, "~");
    }

    #[test]
    fn path_policy_abbreviated_outside_home() {
        let p = PathPolicy::Abbreviated.format(
            Path::new("/opt/project/main"),
            Path::new("/tmp"),
            Some(Path::new("/home/user")),
        );
        assert_eq!(p, "/opt/project/main");
    }

    #[test]
    fn path_policy_abbreviated_no_home() {
        let p =
            PathPolicy::Abbreviated.format(Path::new("/opt/project/main"), Path::new("/tmp"), None);
        assert_eq!(p, "/opt/project/main");
    }
}
