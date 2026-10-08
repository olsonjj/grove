mod common;

use std::fs;
use std::path::PathBuf;

use common::{Fixture, add_worktree, git, init_repo};
use grove_core::{
    MAX_RECENTS, Settings, SettingsFile, Snapshot, SnapshotCache, SnapshotStore, scan,
};
use tempfile::TempDir;

/// (repo name, worktree count) pairs.
fn summary(s: &Snapshot) -> Vec<(String, usize)> {
    s.repos
        .iter()
        .map(|r| {
            assert!(r.error.is_none(), "{}: {:?}", r.name(), r.error);
            (r.name().to_string(), r.worktree_count())
        })
        .collect()
}

fn pairs(v: &[(&str, usize)]) -> Vec<(String, usize)> {
    v.iter().map(|(n, c)| (n.to_string(), *c)).collect()
}

#[test]
fn flat_repo() {
    let f = Fixture::new();
    init_repo(&f.code().join("api"));
    let s = f.scan();
    assert!(s.code_dir_exists);
    assert_eq!(summary(&s), pairs(&[("api", 1)]));
    let main = &s.repos[0].worktrees[0];
    assert!(main.is_main);
    assert_eq!(main.branch.as_deref(), Some("main"));
}

#[test]
fn nested_org_repo() {
    let f = Fixture::new();
    init_repo(&f.code().join("acme/web"));
    init_repo(&f.code().join("tools"));
    assert_eq!(summary(&f.scan()), pairs(&[("tools", 1), ("acme/web", 1)]));
}

#[test]
fn display_names_add_parent_only_on_clash() {
    let f = Fixture::new();
    init_repo(&f.code().join("acme/web"));
    init_repo(&f.code().join("personal/web"));
    init_repo(&f.code().join("acme/api"));
    init_repo(&f.code().join("Tools"));
    let names: Vec<_> = f
        .scan()
        .repos
        .iter()
        .map(|r| r.display_name.clone())
        .collect();
    assert_eq!(names, ["acme/web", "api", "personal/web", "Tools"]);
}

#[test]
fn linked_worktrees_inside_and_outside_code_dir() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    add_worktree(&api, &f.code().join("api-feature"), "feature/login");
    add_worktree(&api, &f.outside("api-hotfix"), "hotfix");
    let s = f.scan();
    // The inside worktree is grouped under its repo, not listed as one; both are counted.
    assert_eq!(summary(&s), pairs(&[("api", 3)]));
    let branches: Vec<_> = s.repos[0]
        .worktrees
        .iter()
        .map(|w| w.branch.clone().unwrap())
        .collect();
    assert_eq!(branches, ["main", "feature/login", "hotfix"]);
}

#[test]
fn linked_worktree_in_nested_folder_is_not_a_repo() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    add_worktree(&api, &f.code().join("worktrees/api-x"), "x");
    assert_eq!(summary(&f.scan()), pairs(&[("api", 2)]));
}

#[test]
fn bare_repo_with_linked_worktrees() {
    let f = Fixture::new();
    let src = init_repo(&f.tmp.path().join("src"));
    let bare = f.code().join("svc.git");
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
    add_worktree(&bare, &f.code().join("svc-a"), "a");
    add_worktree(&bare, &f.outside("svc-b"), "b");
    let s = f.scan();
    assert_eq!(summary(&s), pairs(&[("svc", 2)]));
    assert!(s.repos[0].repo.bare);
    assert!(s.repos[0].worktrees.iter().all(|w| !w.is_main));
}

#[test]
fn bare_repo_without_worktrees_is_listed() {
    let f = Fixture::new();
    git(&f.code(), &["init", "-q", "--bare", "plain.git"]);
    assert_eq!(summary(&f.scan()), pairs(&[("plain", 0)]));
}

#[test]
fn dot_bare_layout_named_after_parent() {
    let f = Fixture::new();
    let d = f.code().join("proj");
    fs::create_dir_all(&d).unwrap();
    let src = init_repo(&f.tmp.path().join("src"));
    git(
        &d,
        &["clone", "-q", "--bare", src.to_str().unwrap(), ".bare"],
    );
    fs::write(d.join(".git"), "gitdir: ./.bare\n").unwrap();
    git(&d, &["worktree", "add", "-q", "-b", "dev", "dev"]);
    assert_eq!(summary(&f.scan()), pairs(&[("proj", 1)]));
}

#[test]
fn empty_folder() {
    let f = Fixture::new();
    fs::create_dir(f.code().join("notes")).unwrap();
    let s = f.scan();
    assert!(s.code_dir_exists);
    assert!(s.repos.is_empty());
}

#[test]
fn missing_folder() {
    let f = Fixture::new();
    let s = scan(&f.tmp.path().join("gone"));
    assert!(!s.code_dir_exists);
    assert!(s.repos.is_empty());
}

#[test]
fn does_not_descend_into_repos() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    init_repo(&api.join("vendored"));
    init_repo(&api.join("node_modules/dep"));
    assert_eq!(summary(&f.scan()), pairs(&[("api", 1)]));
}

#[test]
fn ignores_repos_deeper_than_two_levels() {
    let f = Fixture::new();
    init_repo(&f.code().join("a/b/deep"));
    init_repo(&f.code().join("a/shallow"));
    assert_eq!(summary(&f.scan()), pairs(&[("a/shallow", 1)]));
}

#[test]
fn git_is_available_and_new_enough() {
    let info = grove_core::git::check().expect("git");
    assert!(info.version >= grove_core::git::MIN_VERSION);
}

#[test]
fn settings_round_trip() {
    let tmp = TempDir::new().unwrap();
    let file = SettingsFile::new(tmp.path().join("Grove/settings.json"));
    assert_eq!(file.load().unwrap(), Settings::default());

    let mut s = Settings {
        code_dir: Some(tmp.path().join("code")),
        last_selected_repo: Some("api".into()),
        ..Default::default()
    };
    s.collapsed.insert("api".into(), vec!["feature/".into()]);
    file.save(&s).unwrap();
    assert_eq!(file.load().unwrap(), s);
}

#[test]
fn settings_tolerate_missing_fields() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("settings.json");
    fs::write(&path, r#"{"code_dir":"/x"}"#).unwrap();
    let s = SettingsFile::new(&path).load().unwrap();
    assert_eq!(s.code_dir, Some(PathBuf::from("/x")));
    assert!(s.show_menu_bar_icon && s.launch_at_login && !s.background_fetch);
}

#[test]
fn collapse_state_per_repo() {
    let mut s = Settings::default();
    assert!(s.set_collapsed("api", "fix/", true));
    assert!(s.set_collapsed("api", "feature/", true));
    assert!(!s.set_collapsed("api", "fix/", true));
    assert_eq!(s.collapsed("api"), ["feature/", "fix/"]);
    assert!(s.collapsed("web").is_empty());
    s.set_collapsed("api", "feature/", false);
    s.set_collapsed("api", "fix/", false);
    assert!(s.collapsed.is_empty());
}

#[test]
fn record_open_moves_to_front_and_keeps_last_ten() {
    let mut s = Settings::default();
    for i in 0..12 {
        assert!(s.record_open(&PathBuf::from(format!("/w/{i}"))));
    }
    assert_eq!(s.recents.len(), MAX_RECENTS);
    assert_eq!(s.recents[0], PathBuf::from("/w/11"));
    assert_eq!(s.recents[9], PathBuf::from("/w/2"));

    assert!(!s.record_open(&PathBuf::from("/w/11")));
    assert!(s.record_open(&PathBuf::from("/w/5")));
    assert_eq!(s.recents[0], PathBuf::from("/w/5"));
    assert_eq!(s.recents.iter().filter(|p| p.ends_with("5")).count(), 1);
    assert_eq!(s.recents.len(), MAX_RECENTS);
}

#[test]
fn settings_save_keeps_what_changed_while_the_window_was_open() {
    let mut s = Settings::default();
    let mut edited = s.preferences();
    edited.default_editor = Some("zed".into());
    edited.background_fetch = true;
    edited.code_dir = Some("/code/acme".into());
    s.record_open(&PathBuf::from("/w/api"));
    s.set_collapsed("api", "fix/", true);
    s.set_preferences(edited.clone());
    assert_eq!(s.preferences(), edited);
    assert_eq!(s.recents, [PathBuf::from("/w/api")]);
    assert_eq!(s.collapsed("api"), ["fix/"]);
}

#[test]
fn changing_code_dir_drops_repos_outside_it() {
    let f = Fixture::new();
    init_repo(&f.code().join("api"));
    init_repo(&f.code().join("acme/web"));
    let mut store = SnapshotStore::new(&f.code());
    assert_eq!(
        summary(store.snapshot()),
        pairs(&[("api", 1), ("acme/web", 1)])
    );
    let api_git_dir = store.snapshot().repos[0].repo.git_dir.clone();
    let old_rescan = store.scan_read();

    store.replace(scan(&f.code().join("acme")));
    assert_eq!(summary(store.snapshot()), pairs(&[("web", 1)]));
    assert!(
        !store.apply_scan(old_rescan.run()),
        "old folder's rescan lands"
    );
    assert!(store.repo_read(&api_git_dir).is_none());

    store.refresh_all();
    assert_eq!(summary(store.snapshot()), pairs(&[("web", 1)]));
}

#[test]
fn snapshot_cache_round_trips_with_statuses() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    add_worktree(&api, &f.outside("api-x"), "x");
    fs::write(api.join("new.txt"), "n").unwrap();
    let mut store = SnapshotStore::new(&f.code());
    store.refresh_statuses();
    let cache = SnapshotCache::beside(&SettingsFile::new(f.tmp.path().join("Grove/settings.json")));
    assert_eq!(cache.path(), f.tmp.path().join("Grove/snapshot.json"));
    cache.save(store.snapshot()).unwrap();
    assert_eq!(
        cache.load(&f.code()).as_ref(),
        Some(store.snapshot().as_ref())
    );
}

#[test]
fn unusable_snapshot_cache_is_a_miss() {
    let f = Fixture::new();
    init_repo(&f.code().join("api"));
    let cache = SnapshotCache::new(f.tmp.path().join("snapshot.json"));
    assert!(cache.load(&f.code()).is_none(), "missing");

    cache.save(&f.scan()).unwrap();
    assert!(
        cache.load(&f.tmp.path().join("other")).is_none(),
        "other code dir"
    );

    let json = fs::read_to_string(cache.path()).unwrap();
    fs::write(
        cache.path(),
        json.replacen(r#""version":1"#, r#""version":99"#, 1),
    )
    .unwrap();
    assert!(cache.load(&f.code()).is_none(), "other version");

    fs::write(cache.path(), &json[..json.len() / 2]).unwrap();
    assert!(cache.load(&f.code()).is_none(), "corrupt");

    fs::write(cache.path(), &json).unwrap();
    assert!(cache.load(&f.code()).is_some());
}
