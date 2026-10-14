mod common;

use std::fs;
use std::path::PathBuf;

use common::{Fixture, add_worktree, git, init_repo};
use grove_core::{
    MAX_RECENTS, Settings, SettingsFile, Snapshot, SnapshotCache, SnapshotStore, SortMode, scan,
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
    s.toggle_pin(&tmp.path().join("code/api"));
    s.toggle_hidden(&tmp.path().join("code/acme"));
    assert!(s.set_repo_sort(SortMode::Name));
    assert!(!s.set_repo_sort(SortMode::Name));
    assert!(s.set_tree_sort(SortMode::Name));
    assert!(!s.set_tree_sort(SortMode::Name));
    file.save(&s).unwrap();
    assert_eq!(file.load().unwrap(), s);
    let json = fs::read_to_string(file.path()).unwrap();
    assert!(json.contains(r#""repo_sort": "name""#), "{json}");
    assert!(json.contains(r#""tree_sort": "name""#), "{json}");
}

#[test]
fn settings_tolerate_missing_fields() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("settings.json");
    fs::write(&path, r#"{"code_dir":"/x"}"#).unwrap();
    let s = SettingsFile::new(&path).load().unwrap();
    assert_eq!(s.code_dir, Some(PathBuf::from("/x")));
    assert!(s.show_menu_bar_icon && s.launch_at_login && !s.background_fetch);
    assert!(s.pinned_repos.is_empty());
    assert!(s.hidden_repos.is_empty());
    assert_eq!(s.repo_sort, SortMode::Recent);
    assert_eq!(s.tree_sort, SortMode::Recent);
}

#[test]
fn pin_and_unpin_round_trip() {
    let tmp = TempDir::new().unwrap();
    let file = SettingsFile::new(tmp.path().join("settings.json"));
    let (api, web) = (PathBuf::from("/c/api"), PathBuf::from("/c/web"));
    let mut s = Settings::default();
    assert!(s.toggle_pin(&api));
    assert!(s.toggle_pin(&web));
    file.save(&s).unwrap();
    let mut s = file.load().unwrap();
    assert!(s.is_pinned(&api) && s.is_pinned(&web));

    assert!(s.toggle_pin(&api));
    file.save(&s).unwrap();
    let s = file.load().unwrap();
    assert!(!s.is_pinned(&api));
    assert_eq!(s.pinned_repos, [web]);
}

#[test]
fn hide_and_unhide_round_trip() {
    let tmp = TempDir::new().unwrap();
    let file = SettingsFile::new(tmp.path().join("settings.json"));
    let (api, web) = (PathBuf::from("/c/api"), PathBuf::from("/c/web"));
    let mut s = Settings::default();
    assert!(s.toggle_hidden(&api));
    assert!(s.toggle_hidden(&web));
    file.save(&s).unwrap();
    let mut s = file.load().unwrap();
    assert!(s.is_hidden(&api) && s.is_hidden(&web));

    assert!(s.toggle_hidden(&api));
    file.save(&s).unwrap();
    let s = file.load().unwrap();
    assert!(!s.is_hidden(&api));
    assert_eq!(s.hidden_repos, [web]);
}

#[test]
fn hiding_a_pinned_repo_keeps_both_and_save_leaves_hidden_untouched() {
    let api = PathBuf::from("/c/api");
    let mut s = Settings::default();
    s.toggle_pin(&api);
    s.toggle_hidden(&api);
    assert!(s.is_pinned(&api) && s.is_hidden(&api));

    let mut edited = s.preferences();
    edited.background_fetch = true;
    s.set_preferences(edited);
    assert!(s.is_pinned(&api) && s.is_hidden(&api));
}

#[test]
fn custom_command_run_in_terminal_defaults_on() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("settings.json");
    let json = r#"{"custom_commands":[{"label":"Neovim","template":"nvim {path}"}]}"#;
    fs::write(&path, json).unwrap();
    let s = SettingsFile::new(&path).load().unwrap();
    assert!(s.custom_commands[0].run_in_terminal);
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
        assert!(s.record_open(&PathBuf::from(format!("/w/{i}")), 1_700_000_000 + i));
    }
    assert_eq!(s.recents.len(), MAX_RECENTS);
    assert_eq!(s.recents[0], PathBuf::from("/w/11"));
    assert_eq!(s.recents[9], PathBuf::from("/w/2"));

    assert!(!s.record_open(&PathBuf::from("/w/11"), 1_700_000_011));
    assert!(s.record_open(&PathBuf::from("/w/5"), 1_700_000_100));
    assert_eq!(s.recents[0], PathBuf::from("/w/5"));
    assert_eq!(s.recents.iter().filter(|p| p.ends_with("5")).count(), 1);
    assert_eq!(s.recents.len(), MAX_RECENTS);
}

#[test]
fn record_open_stamps_last_opened_for_every_worktree() {
    let mut s = Settings::default();
    for i in 0..12 {
        s.record_open(&PathBuf::from(format!("/w/{i}")), 1_700_000_000 + i);
    }
    // Not capped like recents.
    assert_eq!(s.last_opened.len(), 12);
    assert_eq!(s.last_opened[&PathBuf::from("/w/0")], 1_700_000_000);

    // Reopening the most recent one still stamps it.
    assert!(s.record_open(&PathBuf::from("/w/11"), 1_700_000_500));
    assert_eq!(s.last_opened[&PathBuf::from("/w/11")], 1_700_000_500);
}

#[test]
fn removal_forgets_when_the_worktree_was_opened() {
    let mut s = Settings::default();
    s.record_open(&PathBuf::from("/w/a"), 1_700_000_000);
    s.record_open(&PathBuf::from("/w/b"), 1_700_000_000);

    assert!(s.forget_worktree(&PathBuf::from("/w/a")));

    assert_eq!(
        s.last_opened.keys().collect::<Vec<_>>(),
        [&PathBuf::from("/w/b")]
    );
    assert!(!s.forget_worktree(&PathBuf::from("/w/a")));
}

#[test]
fn rescan_prunes_last_opened_for_vanished_worktrees() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    let wt = f.outside("api-x");
    add_worktree(&api, &wt, "x");
    let mut store = SnapshotStore::new(&f.code());
    let paths: Vec<PathBuf> = store.snapshot().repos[0]
        .worktrees
        .iter()
        .map(|w| w.path.clone())
        .collect();
    let mut s = Settings::default();
    for p in &paths {
        s.record_open(p, 1_700_000_000);
    }
    s.record_open(&PathBuf::from("/gone"), 1_700_000_000);
    assert!(s.prune_last_opened(store.snapshot()));
    assert_eq!(
        s.last_opened.keys().collect::<Vec<_>>(),
        paths.iter().collect::<Vec<_>>()
    );
    assert!(
        !s.prune_last_opened(store.snapshot()),
        "nothing left to prune"
    );

    git(&api, &["worktree", "remove", wt.to_str().unwrap()]);
    store.refresh_all();
    s.prune_last_opened(store.snapshot());
    assert_eq!(s.last_opened.keys().collect::<Vec<_>>(), [&paths[0]]);

    // A code dir that's gone (volume unmounted) prunes nothing.
    fs::rename(f.code(), f.tmp.path().join("away")).unwrap();
    store.refresh_all();
    assert!(!store.snapshot().code_dir_exists);
    assert!(!s.prune_last_opened(store.snapshot()));
    assert_eq!(s.last_opened.len(), 1);
}

#[test]
fn rescan_with_an_unreadable_repo_prunes_nothing() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    let wt = f.outside("api-x");
    add_worktree(&api, &wt, "x");
    init_repo(&f.code().join("web"));
    let mut store = SnapshotStore::new(&f.code());
    let mut s = Settings::default();
    for w in store.snapshot().repos.iter().flat_map(|r| &r.worktrees) {
        s.record_open(&w.path, 1_700_000_000);
    }
    s.record_open(&PathBuf::from("/gone"), 1_700_000_000);
    let before = s.last_opened.clone();

    // `git worktree list` fails in api, so its worktrees are unknown, not gone.
    fs::remove_file(api.join(".git/HEAD")).unwrap();
    store.refresh_all();
    let api_repo = store.snapshot().repo("api").unwrap();
    assert!(api_repo.error.is_some() && api_repo.worktrees.is_empty());
    assert!(!s.prune_last_opened(store.snapshot()));
    assert_eq!(s.last_opened, before);
}

#[test]
fn settings_save_keeps_what_changed_while_the_window_was_open() {
    let mut s = Settings::default();
    let mut edited = s.preferences();
    edited.default_editor = Some("zed".into());
    edited.background_fetch = true;
    edited.code_dir = Some("/code/acme".into());
    s.record_open(&PathBuf::from("/w/api"), 1_700_000_000);
    s.set_collapsed("api", "fix/", true);
    s.toggle_pin(&PathBuf::from("/w/api"));
    s.toggle_hidden(&PathBuf::from("/w/acme"));
    s.set_repo_sort(SortMode::Name);
    s.set_tree_sort(SortMode::Name);
    s.set_preferences(edited.clone());
    assert_eq!(s.preferences(), edited);
    assert_eq!(s.recents, [PathBuf::from("/w/api")]);
    assert_eq!(s.collapsed("api"), ["fix/"]);
    assert_eq!(s.pinned_repos, [PathBuf::from("/w/api")]);
    assert_eq!(s.hidden_repos, [PathBuf::from("/w/acme")]);
    assert_eq!(s.repo_sort, SortMode::Name);
    assert_eq!(s.tree_sort, SortMode::Name);
    assert_eq!(s.last_opened[&PathBuf::from("/w/api")], 1_700_000_000);
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
fn snapshot_cache_keeps_activity_times_and_loads_without_them() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    fs::write(api.join("wip.txt"), "w").unwrap();
    let mut store = SnapshotStore::new(&f.code());
    store.refresh_statuses();
    let cache = SnapshotCache::new(f.tmp.path().join("snapshot.json"));
    cache.save(store.snapshot()).unwrap();
    let times = |s: &Snapshot| {
        let wt = &s.repos[0].worktrees[0];
        (wt.head_time, wt.changes_time)
    };
    let cached = cache.load(&f.code()).unwrap();
    assert!(times(&cached).0.is_some() && times(&cached).1.is_some());
    assert_eq!(times(&cached), times(store.snapshot()));

    // As written before HEAD and the activity times were read.
    let json = fs::read_to_string(cache.path()).unwrap();
    let mut old = json.clone();
    for key in [r#","head_time":"#, r#","changes_time":"#, r#","head":"#] {
        let without = without_field(&old, key);
        assert_ne!(without, old, "{key} not in cache");
        old = without;
    }
    fs::write(cache.path(), old).unwrap();
    let cached = cache.load(&f.code()).expect("old cache loads");
    assert_eq!(times(&cached), (None, None));
    let Some(Ok(status)) = &cached.repos[0].worktrees[0].status else {
        panic!("status not cached");
    };
    assert_eq!(
        (
            status.head.as_deref(),
            status.head_time,
            status.changes_time
        ),
        (None, None, None)
    );
}

/// `json` without each `<key><number or string>` field.
fn without_field(json: &str, key: &str) -> String {
    let mut out = String::new();
    let mut rest = json;
    while let Some(at) = rest.find(key) {
        out.push_str(&rest[..at]);
        rest = &rest[at + key.len()..];
        rest = match rest.strip_prefix('"') {
            Some(s) => &s[s.find('"').unwrap() + 1..],
            None => rest.trim_start_matches(|c: char| c.is_ascii_digit()),
        };
    }
    out + rest
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
