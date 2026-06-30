//! Integration tests for gitree.
//!
//! These tests shell out to real `git` and exercise the full `gitree` binary
//! against temporary repositories.

use std::fs;
use std::path::Path;
use std::process::Command;

use assert_cmd::Command as AssertCommand;
use predicates::prelude::*;
use tempfile::TempDir;

/// Creates a gitree wrapper from a source repo.
///
/// Returns the temp dir (keeps it alive) and the wrapper directory path.
fn create_gitree_repo() -> (TempDir, std::path::PathBuf) {
    let tmp = TempDir::new().unwrap();

    // Create a source repo to clone from.
    let src = tmp.path().join("source.git");
    fs::create_dir(&src).unwrap();
    git(&src, &["init", "--initial-branch=main"]);
    git(&src, &["config", "user.email", "test@test.com"]);
    git(&src, &["config", "user.name", "Test"]);
    git(&src, &["config", "commit.gpgsign", "false"]);
    fs::write(src.join("README.md"), "# Test\n").unwrap();
    git(&src, &["add", "."]);
    git(&src, &["commit", "-m", "initial"]);

    // gitree init from the source repo — creates a wrapper named "source".
    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(tmp.path())
        .args(["init", src.to_str().unwrap()])
        .assert()
        .success();

    // The wrapper directory should be named "source" (derived from the URL).
    let wrapper = tmp.path().join("source");
    assert!(wrapper.join(".bare").is_dir());
    assert!(wrapper.join(".git").is_file());
    assert!(wrapper.join(".shared").is_dir());

    // Verify .git file content.
    let git_content = fs::read_to_string(wrapper.join(".git")).unwrap();
    assert!(git_content.contains("gitdir: ./.bare"));

    // Verify .gitignore has .shared/.
    let gitignore = fs::read_to_string(wrapper.join(".gitignore")).unwrap();
    assert!(gitignore.contains(".shared/"));

    (tmp, wrapper)
}

/// Runs git in a directory.
fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@test.com")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@test.com")
        .env("GPG_TTY", "")
        .output()
        .unwrap();
    if !output.status.success() {
        panic!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

#[test]
fn init_creates_wrapper_structure() {
    let _ = create_gitree_repo();
}

#[test]
fn add_creates_worktree_for_existing_branch() {
    let (_tmp, wrapper) = create_gitree_repo();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["add", "main"])
        .assert()
        .success();

    assert!(wrapper.join("main").is_dir());
    assert!(wrapper.join("main").join("README.md").exists());
}

#[test]
fn add_alias_a_works() {
    let (_tmp, wrapper) = create_gitree_repo();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["a", "main"])
        .assert()
        .success();

    assert!(wrapper.join("main").is_dir());
}

#[test]
fn add_new_branch_creates_worktree() {
    let (_tmp, wrapper) = create_gitree_repo();

    // First add main.
    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["add", "main"])
        .assert()
        .success();

    // Then add a new feature branch.
    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["add", "feature/test", "--new"])
        .assert()
        .success();

    assert!(wrapper.join("feature").join("test").is_dir());
    assert!(
        wrapper
            .join("feature")
            .join("test")
            .join("README.md")
            .exists()
    );
}

#[test]
fn add_symlinks_shared_files() {
    let (_tmp, wrapper) = create_gitree_repo();

    // Create a .shared/.env file.
    fs::write(wrapper.join(".shared").join(".env"), "FOO=bar\n").unwrap();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["add", "main"])
        .assert()
        .success();

    let env_link = wrapper.join("main").join(".env");
    assert!(env_link.is_symlink());
    let content = fs::read_to_string(env_link).unwrap();
    assert_eq!(content, "FOO=bar\n");
}

#[test]
fn list_shows_worktrees() {
    let (_tmp, wrapper) = create_gitree_repo();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["add", "main"])
        .assert()
        .success();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("main"));
}

#[test]
fn list_alias_ls_works() {
    let (_tmp, wrapper) = create_gitree_repo();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["add", "main"])
        .assert()
        .success();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["ls"])
        .assert()
        .success()
        .stdout(predicate::str::contains("main"));
}

#[test]
fn list_json_outputs_valid_json() {
    let (_tmp, wrapper) = create_gitree_repo();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["add", "main"])
        .assert()
        .success();

    let output = AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let parsed: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert!(parsed.is_array());
    assert!(parsed.as_array().unwrap().iter().any(|v| {
        v.get("branch")
            .and_then(|b| b.as_str())
            .is_some_and(|b| b == "main")
    }));
}

#[test]
fn remove_removes_worktree() {
    let (_tmp, wrapper) = create_gitree_repo();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["add", "main"])
        .assert()
        .success();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["remove", "main"])
        .assert()
        .success();

    assert!(!wrapper.join("main").exists());
}

#[test]
fn remove_alias_rm_works() {
    let (_tmp, wrapper) = create_gitree_repo();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["add", "main"])
        .assert()
        .success();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["rm", "main"])
        .assert()
        .success();

    assert!(!wrapper.join("main").exists());
}

#[test]
fn switch_prints_cd_command() {
    let (_tmp, wrapper) = create_gitree_repo();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["add", "main"])
        .assert()
        .success();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["switch", "main"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("cd "));
}

#[test]
fn switch_alias_sw_works() {
    let (_tmp, wrapper) = create_gitree_repo();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["add", "main"])
        .assert()
        .success();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["sw", "main"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("cd "));
}

#[test]
fn root_prints_wrapper_path() {
    let (_tmp, wrapper) = create_gitree_repo();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["root"])
        .assert()
        .success()
        .stdout(predicate::str::contains("source"));
}

#[test]
fn where_prints_worktree_path() {
    let (_tmp, wrapper) = create_gitree_repo();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["add", "main"])
        .assert()
        .success();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["where", "main"])
        .assert()
        .success()
        .stdout(predicate::str::contains("main"));
}

#[test]
fn add_fails_for_nonexistent_branch_without_new() {
    let (_tmp, wrapper) = create_gitree_repo();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["add", "nonexistent"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

#[test]
fn add_fails_for_duplicate_worktree() {
    let (_tmp, wrapper) = create_gitree_repo();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["add", "main"])
        .assert()
        .success();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["add", "main"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));
}

#[test]
fn not_a_gitree_repo_errors() {
    let tmp = TempDir::new().unwrap();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(tmp.path())
        .args(["list"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not a gitree"));
}

#[test]
fn prune_runs_successfully() {
    let (_tmp, wrapper) = create_gitree_repo();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["add", "main"])
        .assert()
        .success();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["prune"])
        .assert()
        .success();
}

#[test]
fn completion_generates_bash_script() {
    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .args(["completion", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("gitree"));
}

#[test]
fn env_generates_bash_script() {
    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .args(["env", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("gtr()"))
        .stdout(predicate::str::contains("gtrsw"));
}

#[test]
fn env_generates_fish_script() {
    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .args(["env", "fish"])
        .assert()
        .success()
        .stdout(predicate::str::contains("function gtr"))
        .stdout(predicate::str::contains("gtrsw"));
}

#[test]
fn env_generates_posix_script() {
    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .args(["env", "posix"])
        .assert()
        .success()
        .stdout(predicate::str::contains("gtr()"))
        .stdout(predicate::str::contains("gtrsw"));
}

#[test]
fn env_custom_alias() {
    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .args(["env", "bash", "--alias", "mygt"])
        .assert()
        .success()
        .stdout(predicate::str::contains("mygt()"))
        .stdout(predicate::str::contains("mygtsw"))
        .stdout(predicate::str::contains("gtr()").not());
}

#[test]
fn doctor_runs_successfully() {
    let (_tmp, wrapper) = create_gitree_repo();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["doctor"])
        .assert()
        .success()
        .stdout(predicate::str::contains("git fsck"));
}

#[test]
fn status_runs_successfully() {
    let (_tmp, wrapper) = create_gitree_repo();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["add", "main"])
        .assert()
        .success();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("main"));
}

#[test]
fn clean_runs_successfully() {
    let (_tmp, wrapper) = create_gitree_repo();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["add", "main"])
        .assert()
        .success();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["clean"])
        .assert()
        .success();
}

// ---------------------------------------------------------------------------
// Dynamic branch completion (`gitree __complete`)
// ---------------------------------------------------------------------------

/// Helper: creates a gitree repo, adds `main` worktree, and creates a
/// `feature/test` local branch and a `remote-only` branch on the source
/// (fetched into the wrapper).  Returns `(tmp, wrapper, src)`.
fn create_completion_repo() -> (TempDir, std::path::PathBuf, std::path::PathBuf) {
    let (tmp, wrapper) = create_gitree_repo();

    let src = tmp.path().join("source.git");

    // Create a local branch in the bare repo.
    git(&wrapper, &["branch", "feature/test"]);

    // Create a remote-only branch in the source and fetch it.
    git(&src, &["branch", "remote-only"]);
    git(&wrapper, &["fetch", "origin"]);

    // Add the main worktree so it shows up as an active worktree.
    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["add", "main"])
        .assert()
        .success();

    (tmp, wrapper, src)
}

#[test]
fn complete_add_lists_branches_without_worktrees() {
    let (_tmp, wrapper, _src) = create_completion_repo();

    let output = AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["__complete", "add"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let branches: Vec<&str> = std::str::from_utf8(&output).unwrap().lines().collect();

    // feature/test and remote-only have no worktree → should appear.
    assert!(
        branches.contains(&"feature/test"),
        "add should list feature/test: {branches:?}"
    );
    assert!(
        branches.contains(&"remote-only"),
        "add should list remote-only: {branches:?}"
    );
    // main has a worktree → should NOT appear.
    assert!(
        !branches.contains(&"main"),
        "add should not list main: {branches:?}"
    );
}

#[test]
fn complete_default_context_matches_add() {
    let (_tmp, wrapper, _src) = create_completion_repo();

    let with_ctx = AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["__complete", "add"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let without_ctx = AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["__complete"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert_eq!(with_ctx, without_ctx);
}

#[test]
fn complete_remove_lists_worktree_branches() {
    let (_tmp, wrapper, _src) = create_completion_repo();

    let output = AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["__complete", "remove"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let branches: Vec<&str> = std::str::from_utf8(&output).unwrap().lines().collect();

    assert!(
        branches.contains(&"main"),
        "remove should list main: {branches:?}"
    );
    assert!(
        !branches.contains(&"feature/test"),
        "remove should not list feature/test: {branches:?}"
    );
    assert!(
        !branches.contains(&"remote-only"),
        "remove should not list remote-only: {branches:?}"
    );
}

#[test]
fn complete_switch_lists_worktree_branches() {
    let (_tmp, wrapper, _src) = create_completion_repo();

    let output = AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["__complete", "switch"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let branches: Vec<&str> = std::str::from_utf8(&output).unwrap().lines().collect();

    assert!(branches.contains(&"main"));
    assert!(!branches.contains(&"feature/test"));
}

#[test]
fn complete_where_lists_worktree_branches() {
    let (_tmp, wrapper, _src) = create_completion_repo();

    let output = AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["__complete", "where"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let branches: Vec<&str> = std::str::from_utf8(&output).unwrap().lines().collect();

    assert!(branches.contains(&"main"));
    assert!(!branches.contains(&"feature/test"));
}

#[test]
fn complete_base_lists_all_branches() {
    let (_tmp, wrapper, _src) = create_completion_repo();

    let output = AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["__complete", "base"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let branches: Vec<&str> = std::str::from_utf8(&output).unwrap().lines().collect();

    // base lists everything, including worktree branches.
    assert!(branches.contains(&"main"));
    assert!(branches.contains(&"feature/test"));
    assert!(branches.contains(&"remote-only"));
}

#[test]
fn complete_outside_wrapper_prints_nothing() {
    let tmp = TempDir::new().unwrap();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(tmp.path())
        .args(["__complete", "add"])
        .assert()
        .success()
        .stdout("");
}

// ---------------------------------------------------------------------------
// Completion script generation contains dynamic overrides
// ---------------------------------------------------------------------------

#[test]
fn completion_bash_contains_dynamic_overrides() {
    let output = AssertCommand::cargo_bin("gitree")
        .unwrap()
        .args(["completion", "bash"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let script = std::str::from_utf8(&output).unwrap();
    assert!(
        script.contains("gitree __complete add"),
        "bash script should reference `gitree __complete add`"
    );
    assert!(script.contains("gitree __complete base"));
    assert!(script.contains("gitree __complete \"${sub}\""));
    assert!(script.contains("__gitree_clap_orig"));
}

#[test]
fn completion_zsh_contains_dynamic_overrides() {
    let output = AssertCommand::cargo_bin("gitree")
        .unwrap()
        .args(["completion", "zsh"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let script = std::str::from_utf8(&output).unwrap();
    assert!(
        script.contains("_gitree_complete_add"),
        "zsh script should define _gitree_complete_add"
    );
    assert!(script.contains("_gitree_complete_remove"));
    assert!(script.contains("_gitree_complete_switch"));
    assert!(script.contains("_gitree_complete_where"));
    assert!(script.contains("_gitree_complete_base"));
    assert!(script.contains("gitree __complete add"));
    // The _default tag should have been replaced.
    assert!(
        !script.contains("Branch name to create a worktree for:_default"),
        "zsh script should not retain the _default action for add"
    );
}

#[test]
fn completion_fish_contains_dynamic_overrides() {
    let output = AssertCommand::cargo_bin("gitree")
        .unwrap()
        .args(["completion", "fish"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let script = std::str::from_utf8(&output).unwrap();
    assert!(
        script.contains("gitree __complete add"),
        "fish script should reference `gitree __complete add`"
    );
    assert!(script.contains("gitree __complete remove"));
    assert!(script.contains("gitree __complete switch"));
    assert!(script.contains("gitree __complete where"));
    assert!(script.contains("gitree __complete base"));
}

#[test]
fn completion_powershell_unchanged() {
    // PowerShell is not in the override list; output should be the raw
    // clap_complete script with no gitree __complete references.
    let output = AssertCommand::cargo_bin("gitree")
        .unwrap()
        .args(["completion", "powershell"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let script = std::str::from_utf8(&output).unwrap();
    assert!(!script.contains("gitree __complete"));
}
