//! The repo order every surface lists repos in, over real repos.

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use common::{Fixture, add_worktree, commit_at, git, init_repo};
use grove_core::{
    RepoOrder, RepoSnapshot, Settings, SettingsFile, Snapshot, SnapshotStore, SortMode, repo_order,
};

fn none() -> BTreeMap<PathBuf, i64> {
    BTreeMap::new()
}

fn names<'a>(order: &RepoOrder<'a>) -> (Vec<&'a str>, Vec<&'a str>) {
    let names = |v: &[&'a RepoSnapshot]| v.iter().map(|r| r.name()).collect();
    (names(&order.pinned), names(&order.others))
}

/// api, Billing, web, zeta, acme/tools.
fn repos(f: &Fixture) {
    for name in ["zeta", "web", "Billing", "api", "acme/tools"] {
        init_repo(&f.code().join(name));
    }
}

#[test]
fn nothing_pinned_is_all_by_name() {
    let f = Fixture::new();
    repos(&f);
    let s = f.scan();
    let order = repo_order(&s, &[], &none(), SortMode::Name);
    assert_eq!(
        names(&order),
        (vec![], vec!["api", "Billing", "acme/tools", "web", "zeta"])
    );
    assert_eq!(order.iter().count(), s.repos.len());
}

#[test]
fn pinned_lead_by_name_and_appear_once() {
    let f = Fixture::new();
    repos(&f);
    let s = f.scan();
    let path = |name: &str| s.repo(name).unwrap().repo.path.clone();
    let mut settings = Settings::default();
    for name in ["zeta", "Billing"] {
        settings.toggle_pin(&path(name));
    }
    let order = repo_order(&s, &settings.pinned_repos, &none(), SortMode::Name);
    assert_eq!(
        names(&order),
        (vec!["Billing", "zeta"], vec!["api", "acme/tools", "web"])
    );
    let all: Vec<&str> = order.iter().map(|r| r.name()).collect();
    assert_eq!(all, ["Billing", "zeta", "api", "acme/tools", "web"]);
}

#[test]
fn pin_of_an_absent_repo_is_ignored_and_kept() {
    let f = Fixture::new();
    repos(&f);
    let file = SettingsFile::new(f.tmp.path().join("settings.json"));
    let s = f.scan();
    let web = s.repo("web").unwrap().repo.path.clone();
    let mut settings = Settings::default();
    settings.toggle_pin(&web);
    file.save(&settings).unwrap();

    let moved = f.tmp.path().join("web-moved");
    fs::rename(&web, &moved).unwrap();
    let s = f.scan();
    let settings = file.load().unwrap();
    let order = repo_order(&s, &settings.pinned_repos, &none(), SortMode::Name);
    assert_eq!(
        names(&order),
        (vec![], vec!["api", "Billing", "acme/tools", "zeta"])
    );
    assert_eq!(settings.pinned_repos, std::slice::from_ref(&web));

    fs::rename(&moved, &web).unwrap();
    let s = f.scan();
    assert_eq!(
        names(&repo_order(
            &s,
            &settings.pinned_repos,
            &none(),
            SortMode::Name
        ))
        .0,
        ["web"]
    );
}

/// Repos committed at the given Unix times (`None`: no commits), statuses read.
fn dated(f: &Fixture, repos: &[(&str, Option<i64>)]) -> Snapshot {
    for (name, at) in repos {
        let path = f.code().join(name);
        fs::create_dir_all(&path).unwrap();
        git(&path, &["init", "-q"]);
        if let Some(at) = at {
            commit_at(&path, *at);
        }
    }
    read(f)
}

fn read(f: &Fixture) -> Snapshot {
    let mut store = SnapshotStore::new(&f.code());
    store.refresh_statuses();
    store.snapshot().as_ref().clone()
}

#[test]
fn recent_is_newest_first_with_ties_by_name_and_unknown_last() {
    let f = Fixture::new();
    let s = dated(
        &f,
        &[
            ("api", Some(1_700_000_100)),
            ("web", Some(1_700_000_300)),
            ("Billing", Some(1_700_000_100)),
            ("unborn", None),
            ("Zeta", Some(1_700_000_200)),
            ("docs", None),
        ],
    );
    assert_eq!(
        names(&repo_order(&s, &[], &none(), SortMode::Recent)),
        (
            vec![],
            vec!["web", "Zeta", "api", "Billing", "docs", "unborn"]
        )
    );
    assert_eq!(
        names(&repo_order(&s, &[], &none(), SortMode::Name)),
        (
            vec![],
            vec!["api", "Billing", "docs", "unborn", "web", "Zeta"]
        )
    );
}

#[test]
fn a_busy_linked_worktree_lifts_its_repo() {
    let f = Fixture::new();
    let s = dated(
        &f,
        &[("api", Some(1_700_000_000)), ("web", Some(1_700_000_100))],
    );
    assert_eq!(
        names(&repo_order(&s, &[], &none(), SortMode::Recent)).1,
        ["web", "api"]
    );

    let wt = f.outside("api-x");
    add_worktree(&f.code().join("api"), &wt, "x");
    commit_at(&wt, 1_700_000_200);
    let s = read(&f);
    assert_eq!(
        names(&repo_order(&s, &[], &none(), SortMode::Recent)).1,
        ["api", "web"]
    );
}

#[test]
fn pinned_stay_by_name_in_recent_mode() {
    let f = Fixture::new();
    let s = dated(
        &f,
        &[
            ("api", Some(1_700_000_100)),
            ("web", Some(1_700_000_300)),
            ("zeta", Some(1_700_000_400)),
            ("beta", None),
            ("gamma", Some(1_700_000_200)),
        ],
    );
    let path = |name: &str| s.repo(name).unwrap().repo.path.clone();
    let pinned = [path("zeta"), path("beta"), path("api")];
    assert_eq!(
        names(&repo_order(&s, &pinned, &none(), SortMode::Recent)),
        (vec!["api", "beta", "zeta"], vec!["web", "gamma"])
    );
    assert_eq!(
        names(&repo_order(&s, &pinned, &none(), SortMode::Name)),
        (vec!["api", "beta", "zeta"], vec!["gamma", "web"])
    );
}

#[test]
fn opens_and_edits_count_as_activity() {
    let f = Fixture::new();
    let s = dated(
        &f,
        &[
            ("api", Some(1_700_000_100)),
            ("web", Some(1_700_000_300)),
            ("docs", None),
        ],
    );
    fn recent(s: &Snapshot, opened: &BTreeMap<PathBuf, i64>) -> Vec<String> {
        let order = repo_order(s, &[], opened, SortMode::Recent);
        order.others.iter().map(|r| r.name().to_string()).collect()
    }
    assert_eq!(recent(&s, &none()), ["web", "api", "docs"]);

    // Opened after web's last commit; docs, never committed, was opened too.
    let main = |name: &str| s.repo(name).unwrap().worktrees[0].path.clone();
    let opened = BTreeMap::from([(main("api"), 1_700_000_400), (main("docs"), 1_700_000_200)]);
    assert_eq!(recent(&s, &opened), ["api", "web", "docs"]);
    assert_eq!(
        names(&repo_order(&s, &[], &opened, SortMode::Name)).1,
        ["api", "docs", "web"]
    );

    // An uncommitted edit in docs, newer than everything else.
    let wip = f.code().join("docs/wip.txt");
    fs::write(&wip, "w").unwrap();
    fs::File::options()
        .write(true)
        .open(&wip)
        .unwrap()
        .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_500))
        .unwrap();
    assert_eq!(recent(&read(&f), &opened), ["docs", "api", "web"]);
}
