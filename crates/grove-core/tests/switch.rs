//! Switching branches through the git writer, against real git in temp dirs.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use common::{Fixture, git, init_repo};
use grove_core::git::{branches, read_worktree_status, switch};

fn head_branch(path: &Path) -> String {
    git_out(path, &["rev-parse", "--abbrev-ref", "HEAD"])
        .trim()
        .to_string()
}

/// `(objectname) (refname)` lines, so a test can see refs created or deleted.
fn refs(path: &Path) -> Vec<String> {
    git_out(path, &["for-each-ref", "--format=%(objectname) %(refname)"])
        .lines()
        .map(str::to_string)
        .collect()
}

fn changed(path: &Path) -> usize {
    read_worktree_status(path).unwrap().changed()
}

fn git_out(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("run git");
    assert!(out.status.success(), "git {args:?} failed");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// `main` carrying `f.txt` = `main\n`, and `feature` carrying `f.txt` = `feature\n` plus a
/// file main doesn't have. HEAD back on `main` to start.
fn two_branch_repo(f: &Fixture) -> PathBuf {
    let api = init_repo(&f.code().join("api"));
    fs::write(api.join("f.txt"), "main\n").unwrap();
    git(&api, &["add", "-A"]);
    git(&api, &["commit", "-q", "-m", "main"]);
    git(&api, &["switch", "-q", "-c", "feature"]);
    fs::write(api.join("f.txt"), "feature\n").unwrap();
    fs::write(api.join("feature-only.txt"), "feature\n").unwrap();
    git(&api, &["add", "-A"]);
    git(&api, &["commit", "-q", "-m", "feature"]);
    git(&api, &["switch", "-q", "main"]);
    api
}

#[test]
fn switch_moves_head_and_files_without_touching_refs() {
    let f = Fixture::new();
    let api = two_branch_repo(&f);
    let refs_before = refs(&api);

    switch(&api, "feature").unwrap();

    assert_eq!(head_branch(&api), "feature");
    assert_eq!(fs::read_to_string(api.join("f.txt")).unwrap(), "feature\n");
    assert!(fs::exists(api.join("feature-only.txt")).unwrap());
    // A switch creates or deletes no refs, including the two HEADs.
    assert_eq!(refs(&api), refs_before);
    assert_eq!(changed(&api), 0);
}

#[test]
fn up_to_date_switch_is_a_successful_no_op() {
    let f = Fixture::new();
    let api = two_branch_repo(&f);
    switch(&api, "feature").unwrap();
    let head = head_branch(&api);

    switch(&api, "feature").unwrap();

    assert_eq!(head_branch(&api), head);
}

#[test]
fn compatible_uncommitted_changes_carry_over() {
    let f = Fixture::new();
    let api = two_branch_repo(&f);
    git(&api, &["switch", "-q", "-c", "next"]);
    git(&api, &["switch", "-q", "main"]);
    // `next` shares main's committed tree, so the edit fits on it too.
    fs::write(api.join("f.txt"), "mine\n").unwrap();
    assert_eq!(changed(&api), 1);

    switch(&api, "next").unwrap();

    assert_eq!(head_branch(&api), "next");
    // Git carried the uncommitted change along instead of discarding or refusing it.
    assert_eq!(fs::read_to_string(api.join("f.txt")).unwrap(), "mine\n");
    assert_eq!(changed(&api), 1);
}

#[test]
fn conflicting_changes_are_refused_and_nothing_changes() {
    let f = Fixture::new();
    let api = two_branch_repo(&f);
    fs::write(api.join("f.txt"), "mine\n").unwrap();

    let err = switch(&api, "feature").unwrap_err();

    // Git's overwrite refusal, in brief form.
    let msg = err.brief();
    assert!(
        msg.contains("would be overwritten by checkout"),
        "unexpected refusal reason: {err}"
    );
    // Nothing changed: HEAD, files, the change still there.
    assert_eq!(head_branch(&api), "main");
    assert_eq!(fs::read_to_string(api.join("f.txt")).unwrap(), "mine\n");
    assert!(!fs::exists(api.join("feature-only.txt")).unwrap());
    assert_eq!(changed(&api), 1);
}

#[test]
fn branch_held_by_another_worktree_is_refused_naming_it() {
    let f = Fixture::new();
    let api = two_branch_repo(&f);
    let linked = f.outside("api-feature");
    fs::create_dir_all(linked.parent().unwrap()).unwrap();
    // A second worktree already checks out `feature`.
    git(
        &api,
        &["worktree", "add", "-q", linked.to_str().unwrap(), "feature"],
    );

    let err = switch(&api, "feature").unwrap_err();

    let msg = err.brief();
    assert!(msg.contains("already"), "{msg}");
    assert!(
        msg.contains("api-feature"),
        "refusal should name the worktree: {msg}"
    );
    assert_eq!(head_branch(&api), "main");
}

#[test]
fn nonexistent_branch_fails_and_nothing_changes() {
    let f = Fixture::new();
    let api = two_branch_repo(&f);
    let refs_before = refs(&api);

    let err = switch(&api, "nope").unwrap_err();

    let msg = err.brief();
    assert!(
        msg.contains("nope"),
        "git's reason should name the branch: {msg}"
    );
    assert_eq!(head_branch(&api), "main");
    assert_eq!(refs(&api), refs_before);
    assert_eq!(changed(&api), 0);
}

#[test]
fn a_deleted_local_branch_isnt_recreated_from_its_remote_branch() {
    // Git's DWIM guessing (on by default) builds a local branch from a matching remote
    // tracking branch; Grove's write is local-only and ref-neutral, so `--no-guess`
    // must refuse instead. A branch deleted in a terminal but still on a stale dropdown
    // is exactly this shape.
    let f = Fixture::new();
    let api = two_branch_repo(&f);
    let origin = f.outside("origin.git");
    fs::create_dir_all(&origin).unwrap();
    git(&origin, &["init", "-q", "--bare"]);
    git(&api, &["remote", "add", "origin", origin.to_str().unwrap()]);
    git(&api, &["push", "-q", "origin", "feature"]);
    git(&api, &["fetch", "-q"]);
    git(&api, &["branch", "-D", "feature"]);
    let refs_before = refs(&api);

    let err = switch(&api, "feature").unwrap_err();

    // Git's own refusal, and nothing came back: no local branch, no ref touched.
    let msg = err.brief();
    assert!(msg.contains("feature"), "unexpected refusal reason: {err}");
    assert_eq!(branches(&api).unwrap(), ["main"]);
    assert_eq!(refs(&api), refs_before);
    assert_eq!(head_branch(&api), "main");
    assert_eq!(changed(&api), 0);
}

/// The switch's recoverability under an interrupt: the run is stopped with SIGTERM (a
/// user's ^C — git then removes its own lock, unlike Grove's SIGKILL timeout) while a
/// slow smudge filter holds the one write the switch has left. Git has recorded nothing:
/// HEAD and the refs are unchanged, and `f.txt` is gone from disk but not from the index
/// — exactly a deletion awaiting the write. A rerun completes the switch. A kill after
/// git has written files does not recover by rerunning (git then refuses with its
/// overwrite reason, and putting that right takes the user's hands); nothing is lost
/// either way, which is what the bounded budget actually promises.
#[test]
fn interrupted_switch_reruns_to_completion() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    // The filter and its attributes are set before any content, so both trees carry
    // them and every checkout of `f.txt` smudges.
    let marker = f.tmp.path().join("marker");
    git(
        &api,
        &[
            "config",
            "filter.slow.smudge",
            &format!("sh -c 'touch {}; sleep 1; cat'", marker.display()),
        ],
    );
    git(&api, &["config", "filter.slow.clean", "cat"]);
    fs::write(api.join(".gitattributes"), "f.txt filter=slow\n").unwrap();
    fs::write(api.join("f.txt"), "main\n").unwrap();
    git(&api, &["add", "-A"]);
    git(&api, &["commit", "-q", "-m", "main"]);
    git(&api, &["switch", "-q", "-c", "feature"]);
    fs::write(api.join("f.txt"), "feature\n").unwrap();
    git(&api, &["add", "-A"]);
    git(&api, &["commit", "-q", "-m", "feature"]);
    git(&api, &["switch", "-q", "main"]);
    // Setup itself smudged; the marker must come from the switch under test.
    fs::remove_file(&marker).unwrap();

    let mut child = Command::new("git")
        .current_dir(&api)
        .args(["switch", "--quiet", "--", "feature"])
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .spawn()
        .unwrap();
    for _ in 0..100 {
        if marker.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(marker.exists(), "filter never started");
    // SAFETY: a plain signal to our own child.
    unsafe { libc::kill(libc::pid_t::try_from(child.id()).unwrap(), libc::SIGTERM) };
    assert!(!child.wait().unwrap().success());
    // Git recorded nothing yet: HEAD and refs unchanged, its lock gone, and `f.txt`
    // removed from disk only because its filtered write was still pending.
    assert_eq!(head_branch(&api), "main");
    assert!(!fs::exists(api.join("f.txt")).unwrap());
    assert!(!fs::exists(api.join(".git").join("index.lock")).unwrap());

    switch(&api, "feature").unwrap();

    // The rerun finished what the interrupt started: HEAD and files moved, tree clean.
    assert_eq!(head_branch(&api), "feature");
    assert_eq!(fs::read_to_string(api.join("f.txt")).unwrap(), "feature\n");
    assert_eq!(changed(&api), 0);
    assert_eq!(branches(&api).unwrap(), ["feature", "main"]);
}
