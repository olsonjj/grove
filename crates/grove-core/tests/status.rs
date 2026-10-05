mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{Fixture, add_worktree, git, init_repo};
use grove_core::{Presence, RowId, Snapshot, SnapshotStore, Worktree};

/// Store over the fixture's code dir with every worktree's status read.
fn read(f: &Fixture) -> Snapshot {
    let mut store = SnapshotStore::new(&f.code());
    store.refresh_statuses();
    (*store.snapshot()).clone()
}

fn worktree<'a>(s: &'a Snapshot, folder: &str) -> &'a Worktree {
    s.repos
        .iter()
        .flat_map(|r| &r.worktrees)
        .find(|w| w.path.file_name().is_some_and(|n| n == folder))
        .unwrap_or_else(|| panic!("no worktree {folder} in {s:#?}"))
}

fn badges(s: &Snapshot, folder: &str) -> Vec<String> {
    worktree(s, folder)
        .badges()
        .iter()
        .map(|b| b.label())
        .collect()
}

fn commit(dir: &Path, msg: &str) {
    git(dir, &["commit", "-q", "--allow-empty", "-m", msg]);
}

fn bare(at: &Path) -> PathBuf {
    git(
        at.parent().unwrap(),
        &["init", "-q", "--bare", at.to_str().unwrap()],
    );
    at.to_path_buf()
}

/// `api` in the code dir pushing `main` to a bare `origin` outside it, tracking it.
fn tracked_repo(f: &Fixture) -> (PathBuf, PathBuf) {
    fs::create_dir_all(f.tmp.path().join("remotes")).unwrap();
    let origin = bare(&f.tmp.path().join("remotes/origin.git"));
    let api = init_repo(&f.code().join("api"));
    git(&api, &["remote", "add", "origin", origin.to_str().unwrap()]);
    git(&api, &["push", "-q", "-u", "origin", "main"]);
    (api, origin)
}

#[test]
fn clean_tracked_main_worktree_has_only_its_tag() {
    let f = Fixture::new();
    tracked_repo(&f);
    assert_eq!(badges(&read(&f), "api"), ["main worktree"]);
}

#[test]
fn dirty_worktree_counts_changes_in_words() {
    let f = Fixture::new();
    let (api, _) = tracked_repo(&f);
    fs::write(api.join("tracked.txt"), "a").unwrap();
    git(&api, &["add", "tracked.txt"]);
    git(&api, &["commit", "-q", "-m", "add"]);
    git(&api, &["push", "-q"]);
    fs::write(api.join("tracked.txt"), "b").unwrap();
    fs::write(api.join("new.txt"), "n").unwrap();
    fs::write(api.join("staged.txt"), "s").unwrap();
    git(&api, &["add", "staged.txt"]);
    assert_eq!(badges(&read(&f), "api"), ["main worktree", "3 changed"]);
}

#[test]
fn ahead_and_behind_local_bare_remote() {
    let f = Fixture::new();
    let (api, origin) = tracked_repo(&f);
    // Someone else pushes two commits; we commit one locally and fetch.
    let other = f.tmp.path().join("other");
    git(
        f.tmp.path(),
        &[
            "clone",
            "-q",
            origin.to_str().unwrap(),
            other.to_str().unwrap(),
        ],
    );
    commit(&other, "theirs 1");
    commit(&other, "theirs 2");
    git(&other, &["push", "-q"]);
    commit(&api, "mine");
    git(&api, &["fetch", "-q"]);

    let s = read(&f);
    assert_eq!(badges(&s, "api"), ["main worktree", "↑1 ↓2"]);
    let sync = &worktree(&s, "api").badges()[1];
    assert_eq!(sync.description(), "1 ahead, 2 behind origin/main");
}

#[test]
fn fork_tracks_branch_on_another_remote() {
    let f = Fixture::new();
    let (api, _) = tracked_repo(&f);
    let upstream = bare(&f.tmp.path().join("remotes/upstream.git"));
    git(
        &api,
        &["remote", "add", "upstream", upstream.to_str().unwrap()],
    );
    git(&api, &["push", "-q", "upstream", "main:trunk"]);
    git(&api, &["fetch", "-q", "upstream"]);
    let wt = f.code().join("api-fork");
    git(
        &api,
        &[
            "worktree",
            "add",
            "-q",
            "--track",
            "-b",
            "fork",
            wt.to_str().unwrap(),
            "upstream/trunk",
        ],
    );
    commit(&wt, "a");
    commit(&wt, "b");

    let s = read(&f);
    assert_eq!(badges(&s, "api-fork"), ["↑2"]);
    assert_eq!(
        worktree(&s, "api-fork").badges()[0].description(),
        "2 ahead of upstream/trunk"
    );
}

#[test]
fn no_upstream_is_local_only_without_counts() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    add_worktree(&api, &f.code().join("api-x"), "x");
    commit(&f.code().join("api-x"), "ahead of nothing");
    let s = read(&f);
    assert_eq!(badges(&s, "api"), ["main worktree", "local only"]);
    assert_eq!(badges(&s, "api-x"), ["local only"]);
}

#[test]
fn deleted_remote_branch_is_upstream_gone() {
    let f = Fixture::new();
    let (api, origin) = tracked_repo(&f);
    let wt = f.code().join("api-gone");
    add_worktree(&api, &wt, "gone");
    git(&wt, &["push", "-q", "-u", "origin", "gone"]);
    git(&origin, &["branch", "-q", "-D", "gone"]);
    git(&api, &["fetch", "-q", "--prune"]);
    assert_eq!(badges(&read(&f), "api-gone"), ["upstream gone"]);
}

#[test]
fn detached_worktree_shows_sha_and_no_upstream_badge() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    let wt = f.code().join("bisect");
    git(
        &api,
        &["worktree", "add", "-q", "--detach", wt.to_str().unwrap()],
    );
    let s = read(&f);
    let sha = worktree(&s, "bisect").head.clone().unwrap();
    assert_eq!(badges(&s, "bisect"), [format!("detached @ {}", &sha[..7])]);
}

#[test]
fn locked_worktree_is_badged_and_opens() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    let wt = f.code().join("api-locked");
    add_worktree(&api, &wt, "held");
    git(&api, &["worktree", "lock", wt.to_str().unwrap()]);
    let s = read(&f);
    assert_eq!(badges(&s, "api-locked"), ["local only", "locked"]);
    assert_eq!(worktree(&s, "api-locked").presence, Presence::Present);
    let tree = s.repos[0].tree(&[]);
    let (_, presence) = tree
        .worktree_for(&RowId::Branch("held".into()))
        .expect("row");
    assert!(presence.is_openable());
}

#[test]
fn deleted_folder_is_missing_and_not_openable() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    let wt = f.code().join("api-x");
    add_worktree(&api, &wt, "x");
    fs::remove_dir_all(&wt).unwrap();
    let s = read(&f);
    let w = worktree(&s, "api-x");
    assert_eq!(w.presence, Presence::Missing);
    assert!(w.status.is_none(), "no git call on a missing folder");
    assert_eq!(badges(&s, "api-x"), ["missing"]);
    assert!(w.badges()[0].description().contains("git worktree prune"));
    let tree = s.repos[0].tree(&[]);
    let (_, presence) = tree.worktree_for(&RowId::Branch("x".into())).unwrap();
    assert!(!presence.is_openable());
}

#[test]
fn deleted_locked_folder_is_missing_and_locked() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    let wt = f.code().join("api-x");
    add_worktree(&api, &wt, "x");
    git(&api, &["worktree", "lock", wt.to_str().unwrap()]);
    fs::remove_dir_all(&wt).unwrap();
    assert_eq!(badges(&read(&f), "api-x"), ["missing", "locked"]);
}

#[test]
fn refresh_worktree_rereads_one_status() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    let mut store = SnapshotStore::new(&f.code());
    assert!(store.snapshot().repos[0].worktrees[0].status.is_none());
    store.refresh_statuses();
    assert_eq!(
        badges(&store.snapshot(), "api"),
        ["main worktree", "local only"]
    );

    fs::write(api.join("new.txt"), "n").unwrap();
    let path = store.snapshot().repos[0].worktrees[0].path.clone();
    assert!(store.refresh_worktree(&path));
    assert_eq!(
        badges(&store.snapshot(), "api"),
        ["main worktree", "1 changed", "local only"]
    );
    assert!(!store.refresh_worktree(Path::new("/nowhere")));
}

#[test]
fn status_reads_never_write_the_index() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    let wt = f.code().join("api-x");
    add_worktree(&api, &wt, "x");
    for dir in [&api, &wt] {
        fs::write(dir.join("a.txt"), "a").unwrap();
        git(dir, &["add", "a.txt"]);
        git(dir, &["commit", "-q", "-m", "a"]);
    }
    // Stat-dirty but content-clean: a plain `git status` would refresh and rewrite the index.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    for dir in [&api, &wt] {
        fs::write(dir.join("a.txt"), "a").unwrap();
        fs::write(dir.join("dirty.txt"), "d").unwrap();
    }
    let indexes = [
        api.join(".git/index"),
        api.join(".git/worktrees/api-x/index"),
    ];
    let before: Vec<_> = indexes.iter().map(|p| index_state(p)).collect();

    let s = read(&f);
    assert_eq!(badges(&s, "api-x"), ["1 changed", "local only"]);

    let after: Vec<_> = indexes.iter().map(|p| index_state(p)).collect();
    assert_eq!(before, after);
    assert!(!api.join(".git/index.lock").exists());
}

fn index_state(path: &Path) -> (Vec<u8>, std::time::SystemTime) {
    let bytes = fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    (bytes, fs::metadata(path).unwrap().modified().unwrap())
}
