//! `gitree completion`, `gitree __complete`, `gitree __mangen`.

use std::io::Write;
use std::path::Path;

use clap::Command;
use clap_complete::{Shell, generate};
use clap_mangen::Man;

use crate::error::Result;
use crate::repo::Wrapper;

/// Generates and prints shell completion script.
///
/// # Errors
///
/// Returns an error if writing fails.
pub fn run_completion(shell: Shell, mut cmd: Command) -> Result<()> {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    cmd.build();
    generate(shell, &mut cmd, "gitree", &mut out);
    Ok(())
}

/// Dynamic completion: returns branch names for `gitree add <TAB>`.
///
/// # Errors
///
/// Returns an error if git fails or the wrapper cannot be discovered.
pub fn run_complete_branches() -> Result<()> {
    let wrapper = match Wrapper::discover() {
        Ok(w) => w,
        Err(_) => return Ok(()),
    };
    let git = wrapper.git();
    let local = git.local_branches().unwrap_or_default();
    let remote = git.remote_branches().unwrap_or_default();

    let existing_worktrees = git.worktree_list().unwrap_or_default();
    let active: std::collections::HashSet<&str> = existing_worktrees
        .iter()
        .filter_map(|wt| wt.branch.as_deref())
        .collect();

    let local_set: std::collections::HashSet<&str> = local.iter().map(String::as_str).collect();

    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    for b in &local {
        if !active.contains(b.as_str()) {
            writeln!(out, "{b}")?;
        }
    }
    for b in &remote {
        if !active.contains(b.as_str()) && !local_set.contains(b.as_str()) {
            writeln!(out, "{b}")?;
        }
    }
    Ok(())
}

/// Generates manpage(s) into a directory.
///
/// # Errors
///
/// Returns an error if writing fails.
pub fn run_mangen(dir: &Path, mut cmd: Command) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    cmd.build();
    let man = Man::new(cmd);
    let mut buffer: Vec<u8> = Vec::new();
    man.render(&mut buffer)?;

    let path = dir.join("gitree.1");
    let mut file = std::fs::File::create(&path)?;
    file.write_all(&buffer)?;
    eprintln!("Wrote {}", path.display());
    Ok(())
}
