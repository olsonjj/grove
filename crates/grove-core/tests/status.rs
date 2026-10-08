mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{Fixture, add_worktree, commit_at, git, init_repo};
use grove_core::{
    Badge, GitError, Presence, RowId, Snapshot, SnapshotStore, Worktree, WorktreeStatus,
};

/// Store over the fixture's code dir with every worktree's status read.
fn read(f: &Fixture) -> Snapshot {
    let mut store = SnapshotStore::new(&f.code());
    store.refresh_statuses();
    store.snapshot().as_ref().clone()
}

fn worktree<'a>(s: &'a Snapshot, folder: &str) -> &'a Worktree {
    s.repos
        .iter()
        .flat_map(|r| &r.worktrees)
        .find(|w| w.path.file_name().is_some_and(|n| n == folder))
        .unwrap_or_else(|| panic!("no worktree {folder} in {s:#?}"))
}

fn badge_list(s: &Snapshot, folder: &str) -> Vec<Badge> {
    let wt = worktree(s, folder);
    let repo = s.repos.iter().find(|r| r.worktrees.contains(wt)).unwrap();
    repo.badges(wt)
}

fn badges(s: &Snapshot, folder: &str) -> Vec<String> {
    badge_list(s, folder).iter().map(|b| b.label()).collect()
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
    let sync = &badge_list(&s, "api")[1];
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
        badge_list(&s, "api-fork")[0].description(),
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
    assert!(
        badge_list(&s, "api-x")[0]
            .description()
            .contains("git worktree prune")
    );
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
        badges(store.snapshot(), "api"),
        ["main worktree", "local only"]
    );

    fs::write(api.join("new.txt"), "n").unwrap();
    let path = store.snapshot().repos[0].worktrees[0].path.clone();
    assert!(store.refresh_worktree(&path));
    assert_eq!(
        badges(store.snapshot(), "api"),
        ["main worktree", "1 changed", "local only"]
    );
    assert!(!store.refresh_worktree(Path::new("/nowhere")));
}

#[test]
fn reads_from_a_replaced_snapshot_are_dropped() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    let mut store = SnapshotStore::new(&f.code());
    let stale = store.status_reads();
    store.replace(f.scan());
    let fresh = store.status_reads();
    let status = || grove_core::read_worktree_status(&api);
    let path = store.snapshot().repos[0].worktrees[0].path.clone();
    assert!(!store.apply_status(stale.stamp(), &path, status()));
    assert!(store.snapshot().repos[0].worktrees[0].status.is_none());
    assert!(store.apply_status(fresh.stamp(), &path, status()));
    assert!(store.snapshot().repos[0].worktrees[0].status.is_some());
}

#[test]
fn slow_read_never_overwrites_a_later_one() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    let mut store = SnapshotStore::new(&f.code());
    let path = store.snapshot().repos[0].worktrees[0].path.clone();
    let slow = store.status_reads();
    let clean = grove_core::read_worktree_status(&api);

    fs::write(api.join("new.txt"), "n").unwrap();
    assert!(store.refresh_worktree(&path));
    assert!(!store.apply_status(slow.stamp(), &path, clean));
    assert_eq!(
        badges(store.snapshot(), "api"),
        ["main worktree", "1 changed", "local only"]
    );
}

#[test]
fn rescan_keeps_repos_refreshed_after_it_started() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    let mut store = SnapshotStore::new(&f.code());
    let scan = store.scan_read().run();

    add_worktree(&api, &f.code().join("api-x"), "x");
    let git_dir = store.snapshot().repos[0].repo.git_dir.clone();
    assert!(store.refresh_repo(&git_dir));
    assert!(store.apply_scan(scan));
    assert_eq!(store.snapshot().repos[0].worktree_count(), 2);
    assert_eq!(badges(store.snapshot(), "api-x"), ["local only"]);
}

#[test]
fn refresh_all_reads_statuses_too() {
    let f = Fixture::new();
    init_repo(&f.code().join("api"));
    let mut store = SnapshotStore::new(&f.code());
    store.refresh_all();
    assert_eq!(
        badges(store.snapshot(), "api"),
        ["main worktree", "local only"]
    );
}

#[test]
fn presence_recheck_clears_status_and_blocks_late_reads() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    let wt = f.code().join("api-x");
    add_worktree(&api, &wt, "x");
    let mut store = SnapshotStore::new(&f.code());
    let reads = store.status_reads();
    store.refresh_statuses();
    let path = worktree(store.snapshot(), "api-x").path.clone();
    let late = grove_core::read_worktree_status(&path);

    fs::remove_dir_all(&wt).unwrap();
    assert!(store.set_presence(&path, Presence::of(&path)));
    assert_eq!(badges(store.snapshot(), "api-x"), ["missing"]);
    assert!(!store.apply_status(reads.stamp(), &path, late));
    assert!(worktree(store.snapshot(), "api-x").status.is_none());
    assert!(!store.set_presence(Path::new("/nowhere"), Presence::Missing));
}

#[test]
fn single_read_covers_one_present_worktree() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    let wt = f.code().join("api-x");
    add_worktree(&api, &wt, "x");
    let mut store = SnapshotStore::new(&f.code());
    let path = worktree(store.snapshot(), "api-x").path.clone();
    let landed = std::sync::Mutex::new(Vec::new());
    store
        .status_read(&path)
        .run(|p, _| landed.lock().unwrap().push(p.to_path_buf()));
    assert_eq!(*landed.lock().unwrap(), std::slice::from_ref(&path));

    store.set_presence(&path, Presence::Unavailable);
    store
        .status_read(&path)
        .run(|_, _| panic!("read an unavailable worktree"));
    store
        .status_read(Path::new("/nowhere"))
        .run(|_, _| panic!("read an unknown worktree"));
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

#[test]
fn commit_advances_head_time() {
    let f = Fixture::new();
    let api = f.code().join("api");
    fs::create_dir_all(&api).unwrap();
    git(&api, &["init", "-q"]);
    assert_eq!(worktree(&read(&f), "api").head_time, None, "unborn");

    commit_at(&api, 1_700_000_000);
    let wt = f.code().join("api-x");
    add_worktree(&api, &wt, "x");
    commit_at(&wt, 1_700_000_500);
    let s = read(&f);
    assert_eq!(worktree(&s, "api").head_time, Some(1_700_000_000));
    assert_eq!(worktree(&s, "api-x").head_time, Some(1_700_000_500));

    commit_at(&api, 1_700_001_000);
    assert_eq!(worktree(&read(&f), "api").head_time, Some(1_700_001_000));
}

#[test]
fn reads_that_cant_date_head_keep_its_last_time_until_it_moves() {
    let f = Fixture::new();
    let api = f.code().join("api");
    fs::create_dir_all(&api).unwrap();
    git(&api, &["init", "-q"]);
    commit_at(&api, 1_700_000_000);
    let mut store = SnapshotStore::new(&f.code());
    store.refresh_statuses();
    let head_time = |store: &SnapshotStore| worktree(store.snapshot(), "api").head_time;
    let wt = worktree(store.snapshot(), "api").clone();
    let (api, Some(Ok(read))) = (wt.path, wt.status) else {
        panic!("status not read");
    };
    let mut land = |status| {
        let stamp = store.status_reads().stamp();
        store.apply_status(stamp, &api, status);
        head_time(&store)
    };
    let undated = |head: Option<&str>| {
        Ok(WorktreeStatus {
            head: head.map(str::to_string),
            head_time: None,
            ..read.clone()
        })
    };
    let timeout = || {
        Err(GitError::Timeout {
            args: "status".into(),
        })
    };

    // `git log` timed out on the same commit; then status itself timed out.
    assert_eq!(land(undated(read.head.as_deref())), Some(1_700_000_000));
    assert_eq!(land(timeout()), Some(1_700_000_000));
    // HEAD moved but couldn't be dated: unknown, not the old time.
    assert_eq!(land(Ok(read.clone())), Some(1_700_000_000));
    assert_eq!(land(undated(Some("0123abc"))), None);
    assert_eq!(land(timeout()), None);

    store.refresh_statuses();
    assert_eq!(head_time(&store), Some(1_700_000_000));
    store.set_presence(&api, Presence::Missing);
    assert_eq!(head_time(&store), None, "missing");
}

#[test]
fn repo_and_full_rereads_keep_head_time_when_status_fails() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    let wt = f.code().join("api-x");
    add_worktree(&api, &wt, "x");
    commit_at(&wt, 1_700_000_500);
    let mut store = SnapshotStore::new(&f.code());
    store.refresh_statuses();
    let x = |store: &SnapshotStore| worktree(store.snapshot(), "api-x").clone();
    assert_eq!(x(&store).head_time, Some(1_700_000_500));

    // The worktree's `.git` file points nowhere, so `git status` in it fails.
    fs::write(wt.join(".git"), "gitdir: /nowhere\n").unwrap();
    let git_dir = store.snapshot().repos[0].repo.git_dir.clone();
    assert!(store.refresh_repo(&git_dir));
    assert!(matches!(x(&store).status, Some(Err(_))));
    assert_eq!(x(&store).head_time, Some(1_700_000_500));

    store.refresh_all();
    assert!(matches!(x(&store).status, Some(Err(_))));
    assert_eq!(x(&store).head_time, Some(1_700_000_500));
}

fn mtime(path: &Path) -> i64 {
    let t = fs::metadata(path).unwrap().modified().unwrap();
    t.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64
}

#[test]
fn editing_a_file_sets_changes_time() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    fs::write(api.join("old.txt"), "x").unwrap();
    git(&api, &["add", "old.txt"]);
    commit(&api, "old");
    assert_eq!(worktree(&read(&f), "api").changes_time, None, "clean");

    fs::write(api.join("new.txt"), "x").unwrap();
    let s = read(&f);
    assert!(worktree(&s, "api").changes_time >= Some(mtime(&api.join("new.txt"))));

    // A deleted path has no time; the untracked one still counts.
    fs::remove_file(api.join("old.txt")).unwrap();
    let s = read(&f);
    assert_eq!(
        worktree(&s, "api").changes_time,
        Some(mtime(&api.join("new.txt")))
    );

    fs::remove_file(api.join("new.txt")).unwrap();
    assert_eq!(
        worktree(&read(&f), "api").changes_time,
        None,
        "only deletions"
    );
}

#[test]
fn editing_an_already_changed_file_moves_changes_time() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    let wip = api.join("wip.txt");
    fs::write(&wip, "1").unwrap();
    set_mtime(&wip, 1_700_000_000);
    let mut store = SnapshotStore::new(&f.code());
    store.refresh_statuses();
    assert_eq!(
        worktree(store.snapshot(), "api").changes_time,
        Some(1_700_000_000)
    );

    // Same `git status` output, newer file.
    set_mtime(&wip, 1_700_000_100);
    store.refresh_statuses();
    assert_eq!(
        worktree(store.snapshot(), "api").changes_time,
        Some(1_700_000_100)
    );
}

fn set_mtime(path: &Path, secs: u64) {
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs))
        .unwrap();
}

/// A clock-skewed or tool-stamped file would otherwise keep its repo on top for years.
#[test]
fn files_dated_in_the_future_are_ignored() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    let (past, future) = (api.join("past.txt"), api.join("future.txt"));
    fs::write(&past, "p").unwrap();
    fs::write(&future, "f").unwrap();
    set_mtime(&past, 1_700_000_000);
    set_mtime(&future, 4_000_000_000);
    assert_eq!(worktree(&read(&f), "api").changes_time, Some(1_700_000_000));

    fs::remove_file(&past).unwrap();
    assert_eq!(worktree(&read(&f), "api").changes_time, None);
}
