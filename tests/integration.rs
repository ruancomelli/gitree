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
        .stdout(predicate::str::contains("gt()"));
}

#[test]
fn env_generates_fish_script() {
    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .args(["env", "fish"])
        .assert()
        .success()
        .stdout(predicate::str::contains("function gt"));
}

#[test]
fn env_generates_posix_script() {
    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .args(["env", "posix"])
        .assert()
        .success()
        .stdout(predicate::str::contains("gt()"));
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
