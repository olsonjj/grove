//! Fast-forward pull through the git writer, against real git in temp dirs.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{Fixture, add_worktree, git, init_repo, push_commits};
use grove_core::git::{pull, read_worktree_status};

/// `api` in the code dir tracking `main` on a bare `origin` outside it, plus a second
/// clone of `origin` standing in for a teammate, as the fetch tests set up.
fn tracked_repo(f: &Fixture) -> (PathBuf, PathBuf, PathBuf) {
    let remotes = f.tmp.path().join("remotes");
    fs::create_dir_all(&remotes).unwrap();
    let origin = remotes.join("origin.git");
    git(&remotes, &["init", "-q", "--bare", "origin.git"]);
    let api = init_repo(&f.code().join("api"));
    fs::write(api.join("f.txt"), "base\n").unwrap();
    git(&api, &["add", "-A"]);
    git(&api, &["commit", "-q", "-m", "base"]);
    git(&api, &["remote", "add", "origin", origin.to_str().unwrap()]);
    git(&api, &["push", "-q", "-u", "origin", "main"]);
    let other = f.tmp.path().join("other");
    git(
        f.tmp.path(),
        &["clone", "-q", origin.to_str().unwrap(), "other"],
    );
    (api, origin, other)
}

fn ahead_behind(path: &Path) -> (u32, u32) {
    match read_worktree_status(path).unwrap().upstream {
        grove_core::Upstream::Tracking { ahead, behind, .. } => (ahead, behind),
        u => panic!("expected tracking upstream, got {u:?}"),
    }
}

fn head(path: &Path) -> Option<String> {
    read_worktree_status(path).unwrap().head
}

#[test]
fn behind_worktree_fast_forwards() {
    let f = Fixture::new();
    let (api, _, _) = tracked_repo(&f);
    push_commits(f.tmp.path().join("other").as_path(), 2);
    // Test setup: refresh the tracking refs the pre-pull counts read.
    git(&api, &["fetch", "-q"]);
    let (_, behind_before) = ahead_behind(&api);
    assert_eq!(behind_before, 2);

    pull(&api).unwrap();

    let (_, behind) = ahead_behind(&api);
    assert_eq!(behind, 0);
    assert_eq!(ahead_behind(&api).0, 0);
    // The teammate's commits landed and HEAD is a fast-forward, not a merge.
    let log = git_output(&api, &["log", "--format=%s", "-3"]);
    assert_eq!(log.lines().next().unwrap(), "theirs 1");
    assert!(!fs::exists(api.join(".git").join("MERGE_HEAD")).unwrap());
}

#[test]
fn up_to_date_pull_is_a_successful_no_op() {
    let f = Fixture::new();
    let (api, _, _) = tracked_repo(&f);
    let before = head(&api);

    pull(&api).unwrap();

    assert_eq!(head(&api), before);
    assert_eq!(ahead_behind(&api), (0, 0));
}

#[test]
fn diverged_branch_is_refused_and_nothing_changes() {
    let f = Fixture::new();
    let (api, _, other) = tracked_repo(&f);
    git(&api, &["commit", "-q", "--allow-empty", "-m", "local"]);
    let (ahead, _) = ahead_behind(&api);
    assert_eq!(ahead, 1);
    push_commits(&other, 1);
    let before_head = head(&api);

    let err = pull(&api).unwrap_err();

    // Git's fast-forward refusal, in brief form.
    let msg = err.brief();
    assert!(
        msg.to_lowercase().contains("fast-forward"),
        "unexpected refusal reason: {err}"
    );
    // Nothing changed: HEAD, counts, files.
    assert_eq!(head(&api), before_head);
    assert_eq!(ahead_behind(&api), (1, 1));
    assert!(!fs::exists(api.join(".git").join("MERGE_HEAD")).unwrap());
}

#[test]
fn local_change_to_incoming_file_is_refused_with_gits_error() {
    let f = Fixture::new();
    let (api, _, other) = tracked_repo(&f);
    fs::write(other.join("f.txt"), "theirs\n").unwrap();
    git(&other, &["add", "-A"]);
    git(&other, &["commit", "-q", "-m", "touch f.txt"]);
    git(&other, &["push", "-q"]);
    // Local uncommitted change to the same file.
    fs::write(api.join("f.txt"), "mine\n").unwrap();

    let err = pull(&api).unwrap_err();

    let msg = err.brief();
    assert!(
        msg.contains("would be overwritten by merge"),
        "unexpected refusal reason: {err}"
    );
    // Git's refusal left the local edit in place.
    assert_eq!(fs::read_to_string(api.join("f.txt")).unwrap(), "mine\n");
    let (_, behind) = ahead_behind(&api);
    assert_eq!(behind, 1);
}

#[test]
fn local_change_to_untouched_file_does_not_block_the_pull() {
    let f = Fixture::new();
    let (api, _, other) = tracked_repo(&f);
    fs::write(api.join("other.txt"), "mine\n").unwrap();
    fs::write(other.join("f.txt"), "theirs\n").unwrap();
    git(&other, &["add", "-A"]);
    git(&other, &["commit", "-q", "-m", "touch f.txt"]);
    git(&other, &["push", "-q"]);

    pull(&api).unwrap();

    // The pull landed and kept the unrelated local edit.
    assert_eq!(fs::read_to_string(api.join("f.txt")).unwrap(), "theirs\n");
    assert_eq!(fs::read_to_string(api.join("other.txt")).unwrap(), "mine\n");
    assert_eq!(ahead_behind(&api), (0, 0));
}

#[test]
fn worktree_without_upstream_fails() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    let before_head = head(&api);

    let err = pull(&api).unwrap_err();

    assert!(
        matches!(err, grove_core::GitError::Failed { .. }),
        "unexpected error: {err}"
    );
    assert_eq!(head(&api), before_head);
}

#[test]
fn detached_worktree_fails() {
    let f = Fixture::new();
    let (api, _, other) = tracked_repo(&f);
    push_commits(&other, 1);
    let wt = f.outside("api-detached");
    fs::create_dir_all(wt.parent().unwrap()).unwrap();
    add_worktree(&api, &wt, "feature");
    git(&wt, &["checkout", "-q", "--detach"]);
    let before_head = head(&wt);

    let err = pull(&wt).unwrap_err();

    let msg = err.brief();
    assert!(
        msg.contains("not currently on a branch"),
        "unexpected refusal reason: {err}"
    );
    assert_eq!(head(&wt), before_head);
}

fn git_output(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("run git");
    assert!(out.status.success());
    String::from_utf8_lossy(&out.stdout).into_owned()
}
