//! The Watcher keeps the Snapshot within 2 s of what's on disk.

mod common;

use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::thread;
use std::time::{Duration, Instant};

use common::{Fixture, add_worktree, git, init_repo};
use grove_core::{Changes, Presence, Snapshot, SnapshotStore, Watcher, Worktree};

const FRESH: Duration = Duration::from_secs(2);
/// How long to watch for refreshes that shouldn't happen.
const QUIET: Duration = Duration::from_millis(1500);

/// A store kept fresh by a running Watcher, the way the app wires them.
struct Live {
    store: Arc<Mutex<SnapshotStore>>,
    batches: Arc<Mutex<Vec<Changes>>>,
    _watcher: Arc<OnceLock<Watcher>>,
}

impl Live {
    fn start(f: &Fixture) -> Self {
        let mut store = SnapshotStore::new(&f.code());
        store.refresh_statuses();
        let snapshot = store.snapshot().clone();
        let store = Arc::new(Mutex::new(store));
        let batches = Arc::new(Mutex::new(Vec::new()));
        let slot = Arc::new(OnceLock::<Watcher>::new());
        let watcher = Watcher::start(&snapshot, {
            let (store, batches, slot) = (store.clone(), batches.clone(), Arc::downgrade(&slot));
            move |changes: Changes| {
                let snapshot = {
                    let mut store = store.lock().unwrap();
                    store.refresh(&changes);
                    store.snapshot().clone()
                };
                batches.lock().unwrap().push(changes);
                if let Some(watcher) = Weak::upgrade(&slot).as_deref().and_then(OnceLock::get) {
                    watcher.update(&snapshot);
                }
            }
        })
        .unwrap();
        assert!(slot.set(watcher).is_ok());
        let live = Self {
            store,
            batches,
            _watcher: slot,
        };
        // Let FSEvents deliver anything left over from building the fixture.
        thread::sleep(Duration::from_millis(500));
        live.batches.lock().unwrap().clear();
        live
    }

    fn snapshot(&self) -> Arc<Snapshot> {
        self.store.lock().unwrap().snapshot().clone()
    }

    /// Waits up to 2 s for the snapshot to satisfy `done`.
    fn within_2s(&self, what: &str, done: impl Fn(&Snapshot) -> bool) {
        let start = Instant::now();
        while start.elapsed() < FRESH {
            if done(&self.snapshot()) {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        panic!("{what} not reflected within 2 s: {:#?}", self.snapshot());
    }

    /// Refresh batches over a quiet spell.
    fn batches_over_quiet_spell(&self) -> Vec<Changes> {
        self.batches.lock().unwrap().clear();
        thread::sleep(QUIET);
        self.batches.lock().unwrap().clone()
    }
}

fn find<'a>(s: &'a Snapshot, folder: &str) -> Option<&'a Worktree> {
    s.repos
        .iter()
        .flat_map(|r| &r.worktrees)
        .find(|w| w.path.file_name().is_some_and(|n| n == folder))
}

fn changed(s: &Snapshot, folder: &str) -> Option<usize> {
    find(s, folder)?
        .status
        .as_ref()?
        .as_ref()
        .ok()
        .map(|st| st.changed())
}

fn branch<'a>(s: &'a Snapshot, folder: &str) -> Option<&'a str> {
    find(s, folder)?.branch.as_deref()
}

fn commit_file(dir: &Path, name: &str, contents: &str) {
    fs::write(dir.join(name), contents).unwrap();
    git(dir, &["add", name]);
    git(dir, &["commit", "-q", "-m", name]);
}

#[test]
fn added_worktree_appears() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    let live = Live::start(&f);
    add_worktree(&api, &f.code().join("api-x"), "x");
    live.within_2s("worktree add", |s| branch(s, "api-x") == Some("x"));
}

#[test]
fn worktree_added_outside_code_dir_appears() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    let live = Live::start(&f);
    fs::create_dir_all(f.outside("")).unwrap();
    add_worktree(&api, &f.outside("api-far"), "far");
    live.within_2s("outside worktree add", |s| {
        branch(s, "api-far") == Some("far")
    });
}

#[test]
fn removed_worktree_disappears() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    let wt = f.code().join("api-x");
    add_worktree(&api, &wt, "x");
    let live = Live::start(&f);
    assert!(find(&live.snapshot(), "api-x").is_some());
    git(&api, &["worktree", "remove", wt.to_str().unwrap()]);
    live.within_2s("worktree remove", |s| find(s, "api-x").is_none());
}

#[test]
fn deleted_worktree_folder_turns_missing() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    let wt = f.code().join("api-x");
    add_worktree(&api, &wt, "x");
    let live = Live::start(&f);
    fs::remove_dir_all(&wt).unwrap();
    live.within_2s("folder delete", |s| {
        find(s, "api-x").is_some_and(|w| w.presence == Presence::Missing)
    });
}

#[test]
fn branch_switch_in_main_worktree_moves_it() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    let live = Live::start(&f);
    git(&api, &["switch", "-q", "-c", "feature/y"]);
    live.within_2s("main branch switch", |s| {
        branch(s, "api") == Some("feature/y")
    });
}

#[test]
fn branch_switch_in_linked_worktree_moves_it() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    let wt = f.code().join("api-x");
    add_worktree(&api, &wt, "x");
    let live = Live::start(&f);
    git(&wt, &["switch", "-q", "-c", "fix/z"]);
    live.within_2s("linked branch switch", |s| {
        branch(s, "api-x") == Some("fix/z")
    });
}

#[test]
fn detaching_a_worktree_moves_it() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    let wt = f.code().join("api-x");
    add_worktree(&api, &wt, "x");
    let live = Live::start(&f);
    git(&wt, &["switch", "-q", "--detach"]);
    live.within_2s("detach", |s| find(s, "api-x").is_some_and(|w| w.detached));
}

#[test]
fn file_edit_updates_change_count() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    let live = Live::start(&f);
    assert_eq!(changed(&live.snapshot(), "api"), Some(0));
    fs::write(api.join("new.txt"), "n").unwrap();
    live.within_2s("file edit", |s| changed(s, "api") == Some(1));
}

#[test]
fn file_edit_in_linked_worktree_updates_its_count() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    let wt = f.code().join("api-x");
    add_worktree(&api, &wt, "x");
    let live = Live::start(&f);
    fs::write(wt.join("new.txt"), "n").unwrap();
    live.within_2s("linked file edit", |s| changed(s, "api-x") == Some(1));
    assert_eq!(changed(&live.snapshot(), "api"), Some(0));
}

#[test]
fn staging_updates_status() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    let wt = f.code().join("api-x");
    add_worktree(&api, &wt, "x");
    commit_file(&wt, "a.txt", "a");
    fs::write(wt.join("a.txt"), "b").unwrap();
    let live = Live::start(&f);
    assert_eq!(changed(&live.snapshot(), "api-x"), Some(1));
    git(&wt, &["commit", "-q", "-am", "b"]);
    live.within_2s("commit", |s| changed(s, "api-x") == Some(0));
}

#[test]
fn ignored_edits_trigger_no_refresh() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    commit_file(&api, ".gitignore", "node_modules/\n*.log\n");
    let live = Live::start(&f);

    let deps = api.join("node_modules/left-pad/lib");
    fs::create_dir_all(&deps).unwrap();
    for i in 0..200 {
        fs::write(deps.join(format!("{i}.js")), "x").unwrap();
    }
    fs::write(api.join("debug.log"), "x").unwrap();
    assert_eq!(live.batches_over_quiet_spell(), []);

    // Still watching: a file git sees refreshes.
    fs::write(api.join("seen.txt"), "x").unwrap();
    live.within_2s("edit after ignored edits", |s| changed(s, "api") == Some(1));
}

#[test]
fn own_reads_never_retrigger_the_watcher() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    let wt = f.code().join("api-x");
    add_worktree(&api, &wt, "x");
    for dir in [&api, &wt] {
        commit_file(dir, "a.txt", "a");
    }
    // Stat-dirty but content-clean: a plain `git status` would rewrite the index.
    thread::sleep(Duration::from_millis(1100));
    for dir in [&api, &wt] {
        fs::write(dir.join("a.txt"), "a").unwrap();
    }
    let live = Live::start(&f);

    {
        let mut store = live.store.lock().unwrap();
        store.refresh_all();
        store.refresh_statuses();
    }
    assert_eq!(live.batches_over_quiet_spell(), []);

    // A real change refreshes once, and the refresh's own reads don't start another.
    fs::write(api.join("new.txt"), "n").unwrap();
    live.within_2s("file edit", |s| changed(s, "api") == Some(1));
    assert_eq!(live.batches_over_quiet_spell(), []);
}

#[test]
fn force_added_file_in_ignored_folder_refreshes() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    commit_file(&api, ".gitignore", "dist/\n");
    let live = Live::start(&f);

    fs::create_dir_all(api.join("dist")).unwrap();
    fs::write(api.join("dist/y.js"), "y").unwrap();
    assert_eq!(live.batches_over_quiet_spell(), []);

    fs::write(api.join("dist/x.js"), "x").unwrap();
    git(&api, &["add", "-f", "dist/x.js"]);
    git(&api, &["commit", "-q", "-m", "x"]);
    live.within_2s("force-add commit", |s| changed(s, "api") == Some(0));
    fs::write(api.join("dist/x.js"), "x2").unwrap();
    live.within_2s("edit of force-added file", |s| changed(s, "api") == Some(1));
}

#[test]
fn gitignore_edit_reveals_files() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    commit_file(&api, ".gitignore", "*.log\n");
    fs::write(api.join("a.log"), "x").unwrap();
    let live = Live::start(&f);
    assert_eq!(changed(&live.snapshot(), "api"), Some(0));
    fs::write(api.join(".gitignore"), "").unwrap();
    live.within_2s("gitignore edit", |s| changed(s, "api") == Some(2));
}

#[test]
fn pruned_worktree_outside_code_dir_disappears() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    let wt = f.outside("api-x");
    fs::create_dir_all(f.outside("")).unwrap();
    add_worktree(&api, &wt, "x");
    let live = Live::start(&f);
    fs::remove_dir_all(&wt).unwrap();
    live.within_2s("outside folder delete", |s| {
        find(s, "api-x").is_some_and(|w| w.presence == Presence::Missing)
    });
    // Right after the watch set shrinks, while FSEvents restarts its stream.
    git(&api, &["worktree", "prune"]);
    live.within_2s("prune", |s| find(s, "api-x").is_none());
    assert_eq!(live.batches_over_quiet_spell(), []);
}

#[test]
fn watch_set_change_catches_up_once() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    let live = Live::start(&f);
    fs::create_dir_all(f.outside("")).unwrap();
    add_worktree(&api, &f.outside("api-far"), "far");
    live.within_2s("outside worktree add", |s| {
        branch(s, "api-far") == Some("far")
    });
    // The add, maybe split in two, then one catch-up after the stream restarts.
    thread::sleep(QUIET);
    let batches = live.batches.lock().unwrap().len();
    assert!((1..=3).contains(&batches), "{batches} batches");
    assert_eq!(live.batches_over_quiet_spell(), []);
}

#[test]
fn file_edit_in_worktree_inside_bare_repo_updates_its_count() {
    let f = Fixture::new();
    let src = init_repo(&f.tmp.path().join("src"));
    let bare = f.code().join("api.git");
    git(
        f.tmp.path(),
        &[
            "clone",
            "-q",
            "--bare",
            src.to_str().unwrap(),
            bare.to_str().unwrap(),
        ],
    );
    let wt = bare.join("feat");
    add_worktree(&bare, &wt, "feat");
    let live = Live::start(&f);
    assert_eq!(changed(&live.snapshot(), "feat"), Some(0));
    fs::write(wt.join("new.txt"), "n").unwrap();
    live.within_2s("edit in worktree inside bare repo", |s| {
        changed(s, "feat") == Some(1)
    });
}
