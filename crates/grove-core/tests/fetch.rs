//! Fetch all through the Snapshot store, against local bare remotes.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{Fixture, git, init_repo};
use grove_core::{FetchReport, Snapshot, SnapshotStore};

fn store(f: &Fixture) -> SnapshotStore {
    let mut store = SnapshotStore::new(&f.code());
    store.refresh_statuses();
    store
}

/// Badge labels of the `api` worktree named `folder`, as the tree shows them.
fn badges(s: &Snapshot, folder: &str) -> Vec<String> {
    let repo = s.repo("api").expect("api repo");
    let wt = repo
        .worktrees
        .iter()
        .find(|w| w.path.file_name().is_some_and(|n| n == folder))
        .unwrap_or_else(|| panic!("no worktree {folder} in {s:#?}"));
    repo.badges(wt).iter().map(|b| b.label()).collect()
}

/// `api` in the code dir tracking `main` on a bare `origin` outside it, plus a second
/// clone of `origin` standing in for a teammate.
fn tracked_repo(f: &Fixture) -> (PathBuf, PathBuf, PathBuf) {
    let remotes = f.tmp.path().join("remotes");
    fs::create_dir_all(&remotes).unwrap();
    let origin = remotes.join("origin.git");
    git(&remotes, &["init", "-q", "--bare", "origin.git"]);
    let api = init_repo(&f.code().join("api"));
    git(&api, &["remote", "add", "origin", origin.to_str().unwrap()]);
    git(&api, &["push", "-q", "-u", "origin", "main"]);
    let other = f.tmp.path().join("other");
    git(
        f.tmp.path(),
        &["clone", "-q", origin.to_str().unwrap(), "other"],
    );
    (api, origin, other)
}

fn push_commits(other: &Path, n: usize) {
    for i in 0..n {
        git(
            other,
            &[
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                &format!("theirs {i}"),
            ],
        );
    }
    git(other, &["push", "-q"]);
}

#[test]
fn fetch_all_shows_upstream_gone_after_remote_branch_deleted() {
    let f = Fixture::new();
    let (api, _, other) = tracked_repo(&f);
    let wt = f.code().join("api-feature");
    git(
        &api,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feature",
            wt.to_str().unwrap(),
        ],
    );
    git(&wt, &["push", "-q", "-u", "origin", "feature"]);
    let mut store = store(&f);
    assert_eq!(
        badges(store.snapshot(), "api-feature"),
        Vec::<String>::new()
    );

    git(&other, &["push", "-q", "origin", "--delete", "feature"]);
    let report = store.fetch_all().unwrap();

    assert_eq!(report.message(), "Fetched 1 repo");
    assert_eq!(badges(store.snapshot(), "api-feature"), ["upstream gone"]);
}

#[test]
fn fetch_all_updates_behind_count() {
    let f = Fixture::new();
    let (_, _, other) = tracked_repo(&f);
    let mut store = store(&f);
    push_commits(&other, 2);
    assert_eq!(badges(store.snapshot(), "api"), ["main worktree"]);

    store.fetch_all();

    assert_eq!(badges(store.snapshot(), "api"), ["main worktree", "↓2"]);
    assert!(store.snapshot().last_fetch.is_some());
}

#[test]
fn failed_fetch_keeps_counts_and_marks_them_stale_until_next_success() {
    let f = Fixture::new();
    let (_, origin, other) = tracked_repo(&f);
    let mut store = store(&f);
    push_commits(&other, 1);
    store.fetch_all();
    let fetched_at = store.snapshot().last_fetch;
    assert!(fetched_at.is_some());
    push_commits(&other, 1);
    // Remote unreachable, as when offline.
    let away = origin.with_extension("away");
    fs::rename(&origin, &away).unwrap();

    let report = store.fetch_all().unwrap();

    assert_eq!(report.fetched, 0);
    assert_eq!(store.snapshot().last_fetch, fetched_at);
    assert!(
        report.message().starts_with("Fetch failed for api: "),
        "{}",
        report.message()
    );
    assert_eq!(
        badges(store.snapshot(), "api"),
        ["main worktree", "↓1", "stale"]
    );

    fs::rename(&away, &origin).unwrap();
    store.fetch_all();
    assert_eq!(badges(store.snapshot(), "api"), ["main worktree", "↓2"]);
}

#[test]
fn stale_mark_survives_rescans_and_repo_rereads() {
    let f = Fixture::new();
    let (_, origin, _) = tracked_repo(&f);
    let mut store = store(&f);
    store.fetch_all();
    fs::remove_dir_all(&origin).unwrap();
    store.fetch_all();
    let last_fetch = store.snapshot().last_fetch;
    assert!(last_fetch.is_some());

    store.refresh_all();
    let git_dir = store.snapshot().repos[0].repo.git_dir.clone();
    store.refresh_repo(&git_dir);
    store.refresh_statuses();

    assert_eq!(badges(store.snapshot(), "api"), ["main worktree", "stale"]);
    assert_eq!(store.snapshot().last_fetch, last_fetch);
}

#[test]
fn fetch_report_names_the_first_failure_and_counts_the_rest() {
    let err = |msg: &str| grove_core::GitError::Failed {
        args: "fetch --all --prune --quiet".into(),
        stderr: format!("fatal: {msg}\n"),
    };
    let report = FetchReport {
        fetched: 1,
        failures: vec![("api".into(), err("offline")), ("web".into(), err("auth"))],
    };
    assert_eq!(
        report.message(),
        "Fetch failed for api: offline (and 1 more repo)"
    );
}
