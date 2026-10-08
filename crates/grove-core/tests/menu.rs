//! The menu bar popover's rows and one-line states, through the Snapshot store.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{Fixture, add_worktree, commit_at, git, init_repo};
use grove_core::{
    LastOpened, MenuRow, Presence, RowKind, Settings, Snapshot, SnapshotStore, SortMode, StateLine,
    menu,
};

/// Nothing pinned, opened or recent; both lists by name.
fn by_name() -> Settings {
    Settings {
        repo_sort: SortMode::Name,
        tree_sort: SortMode::Name,
        ..Default::default()
    }
}

fn store(f: &Fixture) -> SnapshotStore {
    let mut store = SnapshotStore::new(&f.code());
    store.refresh_statuses();
    store
}

/// The popover row of the worktree in folder `folder`.
fn row(s: &Snapshot, folder: &str) -> MenuRow {
    menu(s, &by_name())
        .into_iter()
        .flat_map(|g| g.rows)
        .find(|r| r.path.file_name().is_some_and(|n| n == folder))
        .unwrap_or_else(|| panic!("no row {folder} in {s:#?}"))
}

fn line(s: &Snapshot, folder: &str) -> StateLine {
    row(s, folder).state
}

fn text(s: &Snapshot, folder: &str) -> String {
    line(s, folder).text
}

fn commit(dir: &Path, msg: &str) {
    git(dir, &["commit", "-q", "--allow-empty", "-m", msg]);
}

/// `api` in the code dir tracking `main` on a bare `origin` outside it, plus a teammate's
/// clone of `origin`.
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

#[test]
fn clean_main_worktree_says_clean() {
    let f = Fixture::new();
    tracked_repo(&f);
    let s = store(&f);
    assert_eq!(
        line(s.snapshot(), "api"),
        StateLine {
            text: "main worktree · clean".into(),
            changed: None,
        }
    );
}

#[test]
fn main_worktree_changes_are_accented_after_its_tag() {
    let f = Fixture::new();
    let (api, _, _) = tracked_repo(&f);
    fs::write(api.join("x"), "x").unwrap();
    let l = line(store(&f).snapshot(), "api");
    assert_eq!(l.text, "main worktree · 1 changed");
    assert_eq!(&l.text[l.changed.unwrap()], "1 changed");
}

#[test]
fn changes_and_sync_share_one_line_and_mark_it_changed() {
    let f = Fixture::new();
    let (api, _, other) = tracked_repo(&f);
    let wt = f.code().join("api-auth");
    add_worktree(&api, &wt, "feature/auth");
    git(&wt, &["push", "-q", "-u", "origin", "feature/auth"]);
    git(&other, &["fetch", "-q"]);
    git(&other, &["switch", "-q", "feature/auth"]);
    commit(&other, "theirs");
    git(&other, &["push", "-q"]);
    commit(&wt, "mine 1");
    commit(&wt, "mine 2");
    git(&wt, &["fetch", "-q"]);
    for name in ["a", "b", "c"] {
        fs::write(wt.join(name), name).unwrap();
    }
    let s = store(&f);
    assert_eq!(
        line(s.snapshot(), "api-auth"),
        StateLine {
            text: "3 changed · ↑2 ↓1".into(),
            changed: Some(0.."3 changed".len()),
        }
    );
}

#[test]
fn behind_main_worktree_still_says_clean() {
    let f = Fixture::new();
    let (api, _, other) = tracked_repo(&f);
    for i in 0..5 {
        commit(&other, &format!("theirs {i}"));
    }
    git(&other, &["push", "-q"]);
    git(&api, &["fetch", "-q"]);
    assert_eq!(
        text(store(&f).snapshot(), "api"),
        "main worktree · clean · ↓5"
    );
}

#[test]
fn clean_linked_worktree_is_just_clean() {
    let f = Fixture::new();
    let (api, _, _) = tracked_repo(&f);
    let wt = f.code().join("api-fix");
    add_worktree(&api, &wt, "fix/rate");
    git(&wt, &["push", "-q", "-u", "origin", "fix/rate"]);
    assert_eq!(text(store(&f).snapshot(), "api-fix"), "clean");
}

#[test]
fn detached_shows_sha_without_clean_and_accents_only_the_changes() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    let wt = f.code().join("bisect");
    git(
        &api,
        &["worktree", "add", "-q", "--detach", wt.to_str().unwrap()],
    );
    fs::write(wt.join("x"), "x").unwrap();
    let s = store(&f);
    let sha = s.snapshot().repos[0]
        .worktrees
        .iter()
        .find(|w| w.path.ends_with("bisect"))
        .and_then(|w| w.head.clone())
        .unwrap();
    let l = line(s.snapshot(), "bisect");
    assert_eq!(l.text, format!("detached @ {} · 1 changed", &sha[..7]));
    assert_eq!(&l.text[l.changed.unwrap()], "1 changed");
    fs::remove_file(wt.join("x")).unwrap();
    assert_eq!(
        text(store(&f).snapshot(), "bisect"),
        format!("detached @ {}", &sha[..7])
    );
}

#[test]
fn local_only_gone_and_locked_are_worded() {
    let f = Fixture::new();
    let (api, origin, _) = tracked_repo(&f);
    add_worktree(&api, &f.code().join("api-local"), "local");
    let gone = f.code().join("api-gone");
    add_worktree(&api, &gone, "gone");
    git(&gone, &["push", "-q", "-u", "origin", "gone"]);
    git(&origin, &["branch", "-q", "-D", "gone"]);
    git(&api, &["fetch", "-q", "--prune"]);
    let locked = f.code().join("api-locked");
    add_worktree(&api, &locked, "held");
    git(&api, &["worktree", "lock", locked.to_str().unwrap()]);

    let s = store(&f);
    assert_eq!(text(s.snapshot(), "api-local"), "clean · local only");
    assert_eq!(text(s.snapshot(), "api-gone"), "clean · upstream gone");
    assert_eq!(
        text(s.snapshot(), "api-locked"),
        "clean · local only · locked"
    );
}

#[test]
fn clean_locked_worktree_says_clean_then_locked() {
    let f = Fixture::new();
    let (api, _, _) = tracked_repo(&f);
    let wt = f.code().join("api-held");
    add_worktree(&api, &wt, "held");
    git(&wt, &["push", "-q", "-u", "origin", "held"]);
    git(&api, &["worktree", "lock", wt.to_str().unwrap()]);
    assert_eq!(text(store(&f).snapshot(), "api-held"), "clean · locked");
}

#[test]
fn missing_and_unavailable_say_why() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    let gone = f.code().join("api-x");
    add_worktree(&api, &gone, "x");
    fs::remove_dir_all(&gone).unwrap();
    let away = f.code().join("api-y");
    add_worktree(&api, &away, "y");
    let mut store = store(&f);
    let path = row(store.snapshot(), "api-y").path;
    store.set_presence(&path, Presence::Unavailable);

    let s = store.snapshot();
    assert_eq!(
        text(s, "api-x"),
        "folder is missing (use Remove missing entry…)"
    );
    assert_eq!(text(s, "api-y"), "its drive isn't mounted");
}

#[test]
fn failed_fetch_marks_sync_stale() {
    let f = Fixture::new();
    let (_, origin, _) = tracked_repo(&f);
    fs::remove_dir_all(&origin).unwrap();
    let mut store = store(&f);
    store.fetch_all();
    assert_eq!(
        text(store.snapshot(), "api"),
        "main worktree · clean · stale"
    );
}

#[test]
fn unread_status_says_so() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    add_worktree(&api, &f.code().join("api-x"), "x");
    let locked = f.code().join("api-held");
    add_worktree(&api, &locked, "held");
    git(&api, &["worktree", "lock", locked.to_str().unwrap()]);
    let store = SnapshotStore::new(&f.code());
    let s = store.snapshot();
    assert_eq!(text(s, "api-x"), "checking status…");
    assert_eq!(text(s, "api"), "main worktree · checking status…");
    assert_eq!(text(s, "api-held"), "locked · checking status…");
}

#[test]
fn groups_by_repo_main_first_then_branches_then_detached_and_marks_last_opened() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    add_worktree(&api, &f.code().join("wt-z"), "zeta");
    let bisect = f.code().join("bisect");
    git(
        &api,
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            bisect.to_str().unwrap(),
        ],
    );
    add_worktree(&api, &f.code().join("wt-a"), "Alpha");
    init_repo(&f.code().join("web"));
    let s = store(&f);
    let s = s.snapshot();
    let recent = s.repos[0]
        .worktrees
        .iter()
        .find(|w| w.path.ends_with("wt-z"))
        .unwrap()
        .path
        .clone();

    let groups = menu(
        s,
        &Settings {
            recents: vec![recent, PathBuf::from("/elsewhere")],
            ..by_name()
        },
    );
    let shape: Vec<(String, Vec<(String, bool)>)> = groups
        .iter()
        .map(|g| {
            let rows = g.rows.iter().map(|r| (r.name.clone(), r.last_opened));
            (g.repo.clone(), rows.collect())
        })
        .collect();
    let rows = |v: &[(&str, bool)]| -> Vec<(String, bool)> {
        v.iter().map(|(n, l)| (n.to_string(), *l)).collect()
    };
    assert_eq!(
        shape,
        [
            (
                "api".to_string(),
                rows(&[
                    ("main", false),
                    ("Alpha", false),
                    ("zeta", true),
                    ("bisect", false)
                ])
            ),
            ("web".to_string(), rows(&[("main", false)])),
        ]
    );
}

#[test]
fn pinned_repos_groups_come_first() {
    let f = Fixture::new();
    for name in ["api", "beta", "web", "zeta"] {
        init_repo(&f.code().join(name));
    }
    let s = f.scan();
    let pinned = [
        s.repo("zeta").unwrap().repo.path.clone(),
        s.repo("web").unwrap().repo.path.clone(),
        PathBuf::from("/gone"),
    ];
    let settings = Settings {
        pinned_repos: pinned.to_vec(),
        ..by_name()
    };
    let repos: Vec<String> = menu(&s, &settings).into_iter().map(|g| g.repo).collect();
    assert_eq!(repos, ["web", "zeta", "api", "beta"]);
}

#[test]
fn groups_follow_repo_sort_after_pins() {
    let f = Fixture::new();
    for (name, at) in [("api", 100), ("beta", 300), ("web", 200), ("zeta", 400)] {
        let path = f.code().join(name);
        fs::create_dir_all(&path).unwrap();
        git(&path, &["init", "-q"]);
        commit_at(&path, 1_700_000_000 + at);
    }
    let s = store(&f).snapshot().clone();
    let pinned = [s.repo("api").unwrap().repo.path.clone()];
    let repos = |opened: &LastOpened, sort| -> Vec<String> {
        let settings = Settings {
            pinned_repos: pinned.to_vec(),
            last_opened: opened.clone(),
            repo_sort: sort,
            ..by_name()
        };
        menu(&s, &settings).into_iter().map(|g| g.repo).collect()
    };
    let none = LastOpened::new();
    assert_eq!(
        repos(&none, SortMode::Recent),
        ["api", "zeta", "beta", "web"]
    );
    assert_eq!(repos(&none, SortMode::Name), ["api", "beta", "web", "zeta"]);

    let web = s.repo("web").unwrap().worktrees[0].path.clone();
    let opened = LastOpened::from([(web, 1_700_000_500)]);
    assert_eq!(
        repos(&opened, SortMode::Recent),
        ["api", "web", "zeta", "beta"]
    );
}

#[test]
fn worktrees_within_a_group_follow_tree_sort() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    add_worktree(&api, &f.code().join("wt-a"), "Alpha");
    add_worktree(&api, &f.code().join("wt-b"), "beta");
    add_worktree(&api, &f.code().join("wt-z"), "zeta");
    for name in ["bisect", "bisect2"] {
        let at = f.code().join(name);
        git(
            &api,
            &["worktree", "add", "-q", "--detach", at.to_str().unwrap()],
        );
    }
    // Status unread, so opens are the only activity.
    let s = f.scan();
    let path = |folder: &str| {
        s.repos[0]
            .worktrees
            .iter()
            .find(|w| w.path.ends_with(folder))
            .unwrap()
            .path
            .clone()
    };
    let opened = LastOpened::from([
        (path("wt-z"), 500),
        (path("wt-a"), 100),
        (path("bisect2"), 900),
    ]);
    let rows = |tree_sort| -> Vec<String> {
        let settings = Settings {
            last_opened: opened.clone(),
            tree_sort,
            ..by_name()
        };
        menu(&s, &settings)
            .into_iter()
            .flat_map(|g| g.rows)
            .map(|r| r.name)
            .collect()
    };
    // Main first and detached last either way; beta never active, so after Alpha.
    assert_eq!(
        rows(SortMode::Recent),
        ["main", "zeta", "Alpha", "beta", "bisect2", "bisect"]
    );
    assert_eq!(
        rows(SortMode::Name),
        ["main", "Alpha", "beta", "zeta", "bisect", "bisect2"]
    );
}

#[test]
fn worktrees_within_a_group_match_the_tree() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    add_worktree(&api, &f.code().join("wt-a"), "feature/a");
    add_worktree(&api, &f.code().join("wt-b"), "feature/b");
    add_worktree(&api, &f.code().join("wt-c"), "c");
    let d1 = f.code().join("d1");
    git(
        &api,
        &["worktree", "add", "-q", "--detach", d1.to_str().unwrap()],
    );
    // The main worktree detached too.
    git(&api, &["checkout", "-q", "--detach"]);
    let s = f.scan();
    let path = |folder: &str| {
        s.repos[0]
            .worktrees
            .iter()
            .find(|w| w.path.ends_with(folder))
            .unwrap()
            .path
            .clone()
    };
    let opened = LastOpened::from([
        (path("wt-a"), 10),
        (path("wt-b"), 1),
        (path("wt-c"), 5),
        (path("d1"), 20),
    ]);
    let rows = |tree_sort| -> Vec<String> {
        let settings = Settings {
            last_opened: opened.clone(),
            tree_sort,
            ..by_name()
        };
        let popover: Vec<PathBuf> = menu(&s, &settings)
            .into_iter()
            .flat_map(|g| g.rows)
            .map(|r| r.path)
            .collect();
        let tree: Vec<PathBuf> = s.repos[0]
            .tree(&[], tree_sort, &opened)
            .rows
            .into_iter()
            .filter_map(|r| match r.kind {
                RowKind::Worktree { path, .. } => Some(path),
                _ => None,
            })
            .collect();
        assert_eq!(popover, tree, "{tree_sort:?}");
        popover
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect()
    };
    // The folder is as active as feature/a; the detached main worktree stays first.
    assert_eq!(
        rows(SortMode::Recent),
        ["wt-a", "wt-b", "wt-c", "api", "d1"]
    );
    assert_eq!(rows(SortMode::Name), ["wt-c", "wt-a", "wt-b", "api", "d1"]);
}
