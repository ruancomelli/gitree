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

    // Local config does not survive a clone, so re-establish the
    // no-signing/user settings on the shared database that every worktree
    // inherits from.
    git(
        &wrapper.join(".bare"),
        &["config", "commit.gpgsign", "false"],
    );
    git(
        &wrapper.join(".bare"),
        &["config", "user.email", "test@test.com"],
    );
    git(&wrapper.join(".bare"), &["config", "user.name", "Test"]);

    // Verify .git file content.
    let git_content = fs::read_to_string(wrapper.join(".git")).unwrap();
    assert!(git_content.contains("gitdir: ./.bare"));

    // Verify .gitignore has .shared/.
    let gitignore = fs::read_to_string(wrapper.join(".gitignore")).unwrap();
    assert!(gitignore.contains(".shared/"));

    (tmp, wrapper)
}

/// Runs git in a directory.
///
/// Prepends `-c commit.gpgsign=false` so no test commit ever touches a
/// signing agent, regardless of repo or global git config.
fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(dir)
        .args(["-c", "commit.gpgsign=false"])
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

/// Creates a regular (non-bare) clone suitable as a `gitree migrate` target.
///
/// Returns the temp dir (keeps it alive) and the clone directory path. The
/// clone has one commit on `main`; the source also has a `feature` branch so
/// that worktrees created from the clone aren't flagged as local-only.
fn create_regular_clone() -> (TempDir, std::path::PathBuf) {
    let tmp = TempDir::new().unwrap();

    // Source repo to clone from.
    let src = tmp.path().join("source.git");
    fs::create_dir(&src).unwrap();
    git(&src, &["init", "--initial-branch=main"]);
    git(&src, &["config", "user.email", "test@test.com"]);
    git(&src, &["config", "user.name", "Test"]);
    fs::write(src.join("README.md"), "# Test\n").unwrap();
    git(&src, &["add", "."]);
    git(&src, &["commit", "-m", "initial"]);

    // Add a feature branch in the source so it exists on the remote.
    git(&src, &["branch", "feature"]);

    // Clone into a regular clone (the migration target).
    let clone_dir = tmp.path().join("myclone");
    git(
        tmp.path(),
        &["clone", src.to_str().unwrap(), clone_dir.to_str().unwrap()],
    );
    git(&clone_dir, &["config", "user.email", "test@test.com"]);
    git(&clone_dir, &["config", "user.name", "Test"]);
    git(&clone_dir, &["config", "commit.gpgsign", "false"]);

    (tmp, clone_dir)
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
fn list_path_relative_from_wrapper_root() {
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
        .args(["list", "--path", "relative"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let s = String::from_utf8(output).unwrap();
    let line = s.lines().find(|l| l.contains("main")).unwrap();
    // From the wrapper root the worktree path is just the branch dir.
    assert!(
        line.ends_with("  main"),
        "expected line to end with relative path 'main', got: {line}"
    );
    // Must NOT contain the absolute wrapper path.
    assert!(!line.contains(wrapper.to_str().unwrap()));
}

#[test]
fn list_path_absolute_shows_full_path() {
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
        .args(["list", "--path", "absolute"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let expected = wrapper.join("main");
    let s = String::from_utf8(output).unwrap();
    assert!(
        s.contains(expected.to_str().unwrap()),
        "expected absolute path {} in output: {s}",
        expected.display()
    );
}

#[test]
fn list_path_abbreviated_uses_tilde() {
    let (tmp, wrapper) = create_gitree_repo();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["add", "main"])
        .assert()
        .success();

    // Pretend the temp dir is $HOME so the wrapper lives under it.
    let output = AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["list", "--path", "abbreviated"])
        .env("HOME", tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let s = String::from_utf8(output).unwrap();
    let line = s.lines().find(|l| l.contains("main")).unwrap();
    assert!(
        line.contains("~/"),
        "expected '~/...' in abbreviated output, got: {line}"
    );
    assert!(!line.contains(tmp.path().to_str().unwrap()));
}

#[test]
fn list_json_honors_path_policy() {
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
        .args(["list", "--json", "--path", "relative"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let parsed: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let main_row = parsed
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v.get("branch").and_then(|b| b.as_str()) == Some("main"))
        .unwrap();
    let path = main_row.get("path").and_then(|p| p.as_str()).unwrap();
    assert_eq!(path, "main");
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
fn remove_removes_multiple_worktrees() {
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
        .args(["add", "feature/test", "--new"])
        .assert()
        .success();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["rm", "main", "feature/test"])
        .assert()
        .success();

    assert!(!wrapper.join("main").exists());
    assert!(!wrapper.join("feature/test").exists());
}

#[test]
fn remove_multiple_with_delete_branch_deletes_all_branches() {
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
        .args(["add", "feature/test", "--new"])
        .assert()
        .success();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["rm", "feature/test", "main", "--delete-branch"])
        .assert()
        .success();

    assert!(!wrapper.join("main").exists());
    assert!(!wrapper.join("feature/test").exists());

    let branches = git(&wrapper.join(".bare"), &["branch", "--list"]);
    assert!(
        !branches.contains("main") && !branches.contains("feature/test"),
        "branches should be deleted: {branches}"
    );
}

#[test]
fn remove_multiple_stops_at_first_missing_worktree() {
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
        .args(["rm", "nonexistent", "main"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("error:"));

    assert!(
        wrapper.join("main").exists(),
        "branches after the failure must not be removed"
    );
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
fn switch_rejects_invalid_branch_names() {
    let (_tmp, wrapper) = create_gitree_repo();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["switch", "../escape"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("branch name"));

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["switch", "foo bar"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("branch name"));
}

#[test]
fn where_rejects_invalid_branch_names() {
    let (_tmp, wrapper) = create_gitree_repo();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["where", ".."])
        .assert()
        .failure()
        .stderr(predicate::str::contains("branch name"));

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["where", "foo/../bar"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("branch name"));
}

#[test]
fn remove_rejects_invalid_branch_names() {
    let (_tmp, wrapper) = create_gitree_repo();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["remove", "../escape"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("branch name"));

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["rm", "branch.lock"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("branch name"));
}

#[test]
fn remove_accepts_trailing_slash_directory_form() {
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
        .args(["rm", "main/"])
        .assert()
        .success();

    assert!(!wrapper.join("main").exists());
}

#[test]
fn remove_accepts_directory_and_path_forms_mixed() {
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
        .args(["add", "feature/test", "--new"])
        .assert()
        .success();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["rm", "main/", "./feature/test/"])
        .assert()
        .success();

    assert!(!wrapper.join("main").exists());
    assert!(!wrapper.join("feature/test").exists());
}

#[test]
fn remove_accepts_absolute_worktree_path() {
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
        .args(["rm", wrapper.join("main").to_str().unwrap()])
        .assert()
        .success();

    assert!(!wrapper.join("main").exists());
}

#[test]
fn remove_trailing_slash_works_from_inside_another_worktree() {
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
        .args(["add", "feature/test", "--new"])
        .assert()
        .success();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(wrapper.join("main"))
        .args(["rm", "feature/test/"])
        .assert()
        .success();

    assert!(!wrapper.join("feature/test").exists());
    assert!(wrapper.join("main").exists());
}

#[test]
fn remove_rejects_traversal_with_trailing_slash() {
    let (_tmp, wrapper) = create_gitree_repo();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["rm", "../escape/"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("branch name"));
}

#[test]
fn where_accepts_trailing_slash_directory_form() {
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
        .args(["where", "main/"])
        .assert()
        .stdout(predicate::str::contains("main"))
        .success();
}

#[test]
fn switch_accepts_trailing_slash_directory_form() {
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
        .args(["switch", "main/"])
        .assert()
        .stdout(predicate::str::contains("cd"))
        .stdout(predicate::str::contains("main"))
        .success();
}

#[test]
fn pull_rejects_invalid_branch_override() {
    let (_tmp, wrapper) = create_gitree_repo();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["pull", "--branch", "foo/../bar"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("branch name"));
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
fn prune_reports_no_stale_references() {
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
        .success()
        .stderr(predicate::str::contains("No stale worktree references"));
}

#[test]
fn prune_lists_removed_worktree_references() {
    let (_tmp, wrapper) = create_gitree_repo();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["add", "main"])
        .assert()
        .success();

    // Delete the worktree directory out from under git, leaving a stale
    // reference behind.
    let stale_path = wrapper.join("main");
    fs::remove_dir_all(&stale_path).unwrap();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["prune"])
        .assert()
        .success()
        .stderr(
            predicate::str::contains("Pruned stale worktree references")
                .and(predicate::str::contains(stale_path.to_str().unwrap())),
        );

    // The stale reference no longer appears in the worktree list.
    let listed = git(&wrapper, &["worktree", "list", "--porcelain"]);
    assert!(
        !listed.contains(stale_path.to_str().unwrap()),
        "stale reference still listed after prune: {listed}"
    );
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
fn status_alias_st_works() {
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
        .args(["st"])
        .assert()
        .success()
        .stdout(predicate::str::contains("main"));
}

#[test]
fn status_default_path_is_relative() {
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
        .args(["status"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let s = String::from_utf8(output).unwrap();
    let line = s.lines().find(|l| l.contains("main")).unwrap();
    // From the wrapper root the worktree path is just the branch dir.
    assert!(
        line.ends_with("main"),
        "expected line to end with relative path 'main', got: {line}"
    );
    // Must NOT contain the absolute wrapper path.
    assert!(!line.contains(wrapper.to_str().unwrap()));
}

#[test]
fn status_path_absolute_shows_full_path() {
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
        .args(["status", "--path", "absolute"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let expected = wrapper.join("main");
    let s = String::from_utf8(output).unwrap();
    assert!(
        s.contains(expected.to_str().unwrap()),
        "expected absolute path {} in output: {s}",
        expected.display()
    );
}

#[test]
fn status_path_abbreviated_uses_tilde() {
    let (tmp, wrapper) = create_gitree_repo();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["add", "main"])
        .assert()
        .success();

    // Pretend the temp dir is $HOME so the wrapper lives under it.
    let output = AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["status", "--path", "abbreviated"])
        .env("HOME", tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let s = String::from_utf8(output).unwrap();
    let line = s.lines().find(|l| l.contains("main")).unwrap();
    assert!(
        line.contains("~/"),
        "expected '~/...' in abbreviated output, got: {line}"
    );
    assert!(!line.contains(tmp.path().to_str().unwrap()));
}

#[test]
fn status_json_outputs_valid_json() {
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
        .args(["status", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let parsed: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert!(parsed.is_array());
    let main_row = parsed
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v.get("branch").and_then(|b| b.as_str()) == Some("main"))
        .unwrap();
    // JSON should contain the structured fields.
    assert!(main_row.get("path").is_some());
    assert!(main_row.get("dirty").is_some());
    assert!(main_row.get("ahead").is_some());
    assert!(main_row.get("behind").is_some());
}

#[test]
fn status_json_honors_path_policy() {
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
        .args(["status", "--json", "--path", "relative"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let parsed: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let main_row = parsed
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v.get("branch").and_then(|b| b.as_str()) == Some("main"))
        .unwrap();
    let path = main_row.get("path").and_then(|p| p.as_str()).unwrap();
    assert_eq!(path, "main");
}

#[test]
fn status_color_never_strips_ansi() {
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
        .args(["status", "--color", "never"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let s = String::from_utf8(output).unwrap();
    assert!(!s.contains('\x1b'));
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

#[test]
fn clean_aborts_when_fetch_fails() {
    let (_tmp, wrapper) = create_gitree_repo();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["add", "main"])
        .assert()
        .success();

    // Create a local-only branch that clean would flag as stale.
    let main_wt = wrapper.join("main");
    fs::write(main_wt.join("file.txt"), "content\n").unwrap();
    git(&main_wt, &["add", "."]);
    git(&main_wt, &["commit", "-m", "work"]);
    git(&main_wt, &["branch", "local-only"]);

    // Break the remote so `git fetch --prune` fails.
    git(
        &wrapper.join(".bare"),
        &["remote", "set-url", "origin", "/nonexistent/repo.git"],
    );

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["clean", "--force"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "could not refresh remote-tracking refs",
        ));

    // The branch must survive: no deletion against stale remote state.
    let branches = git(&wrapper.join(".bare"), &["branch", "--list"]);
    assert!(
        branches.contains("local-only"),
        "local-only branch must survive an aborted clean"
    );
}

// ---------------------------------------------------------------------------
// `gitree pull`
// ---------------------------------------------------------------------------

#[test]
fn pull_dirty_worktree_names_branch_and_suggests_autostash() {
    let (_tmp, wrapper) = create_gitree_repo();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["add", "main"])
        .assert()
        .success();

    // Modify a tracked file to make the worktree dirty.
    fs::write(wrapper.join("main").join("README.md"), "modified\n").unwrap();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["pull"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("working tree 'main' is dirty"))
        .stderr(predicate::str::contains("1 uncommitted change"))
        .stderr(predicate::str::contains("--autostash"))
        .stderr(predicate::str::contains("worktree 'main'"));
}

#[test]
fn pull_autostash_succeeds_with_dirty_worktree() {
    let (_tmp, wrapper) = create_gitree_repo();
    let src = _tmp.path().join("source.git");

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["add", "main"])
        .assert()
        .success();

    // Add a second commit to the source so there's something to fast-forward to.
    fs::write(src.join("new-file.txt"), "new\n").unwrap();
    git(&src, &["add", "."]);
    git(&src, &["commit", "-m", "second"]);

    // Modify a tracked file in the worktree.
    let readme = wrapper.join("main").join("README.md");
    fs::write(&readme, "modified\n").unwrap();

    // Stash (triggered by --autostash) creates commits; the shared `.bare`
    // config set by `create_gitree_repo` supplies the no-sign setting and
    // identity for them.
    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["pull", "--autostash"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Fast-forwarding"))
        .stderr(predicate::str::contains("Done"));

    // The modification should be preserved (autostash popped it back).
    assert_eq!(fs::read_to_string(&readme).unwrap(), "modified\n");
    // The new file from the fast-forward should be present.
    assert!(wrapper.join("main").join("new-file.txt").exists());
}

#[test]
fn pull_clean_worktree_fast_forwards() {
    let (_tmp, wrapper) = create_gitree_repo();
    let src = _tmp.path().join("source.git");

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["add", "main"])
        .assert()
        .success();

    // Add a second commit to the source so there's something to fast-forward to.
    fs::write(src.join("new-file.txt"), "new\n").unwrap();
    git(&src, &["add", "."]);
    git(&src, &["commit", "-m", "second"]);

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&wrapper)
        .args(["pull"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Fast-forwarding"))
        .stderr(predicate::str::contains("Done"));

    // The new file from the fast-forward should be present.
    assert!(wrapper.join("main").join("new-file.txt").exists());
}

// ---------------------------------------------------------------------------
// `gitree migrate`
// ---------------------------------------------------------------------------

#[test]
fn migrate_plain_clone_creates_wrapper() {
    let (_tmp, clone) = create_regular_clone();

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&clone)
        .args(["migrate", "--yes"])
        .assert()
        .success();

    // Wrapper layout.
    assert!(clone.join(".bare").is_dir());
    assert!(clone.join(".git").is_file());
    assert!(clone.join(".shared").is_dir());
    // The .git file points at .bare (relative or absolute after repair).
    let git_content = fs::read_to_string(clone.join(".git")).unwrap();
    assert!(
        git_content.contains("gitdir:") && git_content.contains(".bare"),
        "expected .git file to point at .bare, got: {git_content}"
    );

    // The .gitignore at the wrapper level has .shared/.
    let gitignore = fs::read_to_string(clone.join(".gitignore")).unwrap();
    assert!(gitignore.contains(".shared/"));

    // Main worktree relocated into <wrapper>/main/.
    assert!(clone.join("main").is_dir());
    assert!(clone.join("main").join("README.md").exists());

    // .bare is bare.
    let is_bare = git(&clone, &["rev-parse", "--is-bare-repository"]);
    assert_eq!(is_bare, "true");

    // gitree doctor passes and list shows main.
    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&clone)
        .args(["doctor"])
        .assert()
        .success();
    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&clone)
        .args(["list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("main"));
}

#[test]
fn migrate_relocates_linked_worktree_sibling() {
    let (tmp, clone) = create_regular_clone();

    // Create a branch and a sibling linked worktree (outside the clone).
    git(&clone, &["branch", "feature"]);
    let sibling = tmp.path().join("sibling-feature");
    git(
        &clone,
        &["worktree", "add", sibling.to_str().unwrap(), "feature"],
    );

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&clone)
        .args(["migrate", "--yes"])
        .assert()
        .success();

    // The linked worktree is relocated into <wrapper>/feature/.
    assert!(clone.join("feature").is_dir());
    assert!(clone.join("feature").join("README.md").exists());

    // The sibling path is gone.
    assert!(!sibling.exists());

    // Main worktree relocated.
    assert!(clone.join("main").is_dir());

    // git worktree list shows both worktrees at the wrapper.
    let list = git(&clone, &["worktree", "list", "--porcelain"]);
    assert!(
        list.contains(clone.join("main").to_str().unwrap()),
        "expected main worktree path in: {list}"
    );
    assert!(
        list.contains(clone.join("feature").to_str().unwrap()),
        "expected feature worktree path in: {list}"
    );
}

#[test]
fn migrate_locked_worktree_fails() {
    let (tmp, clone) = create_regular_clone();
    git(&clone, &["branch", "feature"]);
    let sibling = tmp.path().join("locked-feature");
    git(
        &clone,
        &["worktree", "add", sibling.to_str().unwrap(), "feature"],
    );
    git(&clone, &["worktree", "lock", sibling.to_str().unwrap()]);

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&clone)
        .args(["migrate", "--yes"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("locked"));

    // The repository is untouched: .git is still a directory.
    assert!(clone.join(".git").is_dir());
    assert!(!clone.join(".bare").exists());
}

#[test]
fn migrate_renames_dir_to_match_branch() {
    let (tmp, clone) = create_regular_clone();
    git(&clone, &["branch", "feature"]);
    // Worktree at a directory whose name differs from its branch.
    let sibling = tmp.path().join("custom-dir-name");
    git(
        &clone,
        &["worktree", "add", sibling.to_str().unwrap(), "feature"],
    );

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&clone)
        .args(["migrate", "--yes"])
        .assert()
        .success();

    // Relocated to <wrapper>/feature/, not <wrapper>/custom-dir-name/.
    assert!(clone.join("feature").is_dir());
    assert!(clone.join("feature").join("README.md").exists());
    assert!(!clone.join("custom-dir-name").exists());
}

#[test]
fn migrate_main_branch_with_slash() {
    let (tmp, clone) = create_regular_clone();
    git(&clone, &["branch", "feature/backport"]);

    // Check out the slash-containing branch in the main worktree.
    git(&clone, &["checkout", "feature/backport"]);
    fs::write(clone.join("work.txt"), "work\n").unwrap();
    git(&clone, &["add", "."]);
    git(&clone, &["commit", "-m", "work on backport"]);

    // Add a linked worktree for a second branch.
    git(&clone, &["branch", "other"]);
    let sibling = tmp.path().join("sibling-other");
    git(
        &clone,
        &["worktree", "add", sibling.to_str().unwrap(), "other"],
    );

    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&clone)
        .args(["migrate", "--yes", "--force"])
        .assert()
        .success();

    // Main worktree relocated to <wrapper>/feature/backport/.
    assert!(clone.join("feature").join("backport").is_dir());
    assert!(
        clone
            .join("feature")
            .join("backport")
            .join("work.txt")
            .exists()
    );

    // The state dir uses the basename (backport), not the full branch, so
    // commondir `../..` resolves correctly to .bare/.
    let state_dir = clone.join(".bare").join("worktrees").join("backport");
    assert!(state_dir.is_dir());
    let commondir = fs::read_to_string(state_dir.join("commondir")).unwrap();
    assert_eq!(commondir.trim(), "../..");

    // Linked worktree also relocated.
    assert!(clone.join("other").is_dir());

    // gitree doctor passes (verifies the full layout is sound).
    AssertCommand::cargo_bin("gitree")
        .unwrap()
        .current_dir(&clone)
        .args(["doctor"])
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
    assert!(
        script.contains("a) sub=\"add\"")
            && script.contains("rm) sub=\"remove\"")
            && script.contains("sw) sub=\"switch\""),
        "bash script should normalize subcommand aliases"
    );
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
    assert!(
        script.contains("__fish_gitree_using_subcommand remove rm")
            && script.contains("__fish_gitree_using_subcommand add a")
            && script.contains("__fish_gitree_using_subcommand switch sw"),
        "fish script should match subcommand aliases"
    );
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
