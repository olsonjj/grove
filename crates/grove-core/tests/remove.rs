//! Removing linked worktrees, against real git in temp dirs.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{Fixture, add_worktree, git, init_repo};
use grove_core::{
    RemovalPlan, Snapshot, Worktree, prune_worktrees, read_removal_plan, remove_worktree,
};

fn worktree<'a>(s: &'a Snapshot, path: &Path) -> &'a Worktree {
    s.repos
        .iter()
        .flat_map(|r| &r.worktrees)
        .find(|w| w.path.canonicalize().ok() == path.canonicalize().ok())
        .unwrap_or_else(|| panic!("no worktree {} in {s:#?}", path.display()))
}

/// `api` with a linked worktree on branch `feature`, outside the code dir.
fn linked(f: &Fixture) -> (PathBuf, PathBuf) {
    let api = init_repo(&f.code().join("api"));
    let wt = f.outside("api-feature");
    fs::create_dir_all(wt.parent().unwrap()).unwrap();
    add_worktree(&api, &wt, "feature");
    (api, wt)
}

fn worktree_count(f: &Fixture) -> usize {
    f.scan().repo("api").unwrap().worktrees.len()
}

#[test]
fn main_worktree_is_not_removable() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));

    let plan = read_removal_plan(worktree(&f.scan(), &api));

    assert_eq!(
        plan,
        RemovalPlan::NotAllowed("it's the main worktree".into())
    );
}

#[test]
fn locked_worktree_is_not_removable() {
    let f = Fixture::new();
    let (api, wt) = linked(&f);
    git(&api, &["worktree", "lock", wt.to_str().unwrap()]);

    assert_eq!(
        read_removal_plan(worktree(&f.scan(), &wt)),
        RemovalPlan::NotAllowed("it's locked (run git worktree unlock first)".into())
    );
}

#[test]
fn missing_worktree_offers_prune_and_prune_drops_the_entry() {
    let f = Fixture::new();
    let (api, wt) = linked(&f);
    fs::remove_dir_all(&wt).unwrap();
    let snapshot = f.scan();
    let missing = snapshot.repo("api").unwrap().worktrees.last().unwrap();
    assert_eq!(read_removal_plan(missing), RemovalPlan::Prune);

    prune_worktrees(&api).unwrap();

    assert_eq!(worktree_count(&f), 1);
}

#[test]
fn clean_worktree_plans_plain_removal() {
    let f = Fixture::new();
    let (_, wt) = linked(&f);

    let plan = read_removal_plan(worktree(&f.scan(), &wt));

    assert_eq!(plan, RemovalPlan::Clean);
    assert!(!plan.force());
}

#[test]
fn modified_files_are_counted() {
    let f = Fixture::new();
    let (api, wt) = linked(&f);
    fs::write(api.join("a.txt"), "a").unwrap();
    fs::write(api.join("b.txt"), "b").unwrap();
    git(&api, &["add", "."]);
    git(&api, &["commit", "-q", "-m", "files"]);
    git(&wt, &["merge", "-q", "main"]);
    fs::write(wt.join("a.txt"), "changed").unwrap();
    fs::write(wt.join("b.txt"), "changed").unwrap();
    git(&wt, &["add", "b.txt"]);

    let plan = read_removal_plan(worktree(&f.scan(), &wt));

    assert_eq!(
        plan,
        RemovalPlan::Dirty {
            modified: 2,
            untracked: 0
        }
    );
    assert!(plan.force());
}

#[test]
fn untracked_files_are_counted_one_by_one_even_in_new_folders() {
    let f = Fixture::new();
    let (_, wt) = linked(&f);
    fs::create_dir_all(wt.join("new/deep")).unwrap();
    fs::write(wt.join("new/one.txt"), "1").unwrap();
    fs::write(wt.join("new/deep/two.txt"), "2").unwrap();
    fs::write(wt.join("three.txt"), "3").unwrap();

    assert_eq!(
        read_removal_plan(worktree(&f.scan(), &wt)),
        RemovalPlan::Dirty {
            modified: 0,
            untracked: 3
        }
    );
}

#[test]
fn removing_a_clean_worktree_keeps_its_branch() {
    let f = Fixture::new();
    let (api, wt) = linked(&f);

    remove_worktree(&api, &wt, false).unwrap();

    assert!(!wt.exists());
    assert_eq!(worktree_count(&f), 1);
    git(&api, &["rev-parse", "--verify", "-q", "refs/heads/feature"]);
}

#[test]
fn dirty_worktree_needs_force_and_unforced_removal_leaves_it() {
    let f = Fixture::new();
    let (api, wt) = linked(&f);
    fs::write(wt.join("wip.txt"), "w").unwrap();

    let err = remove_worktree(&api, &wt, false).unwrap_err();

    assert!(
        err.brief().contains("contains modified or untracked files"),
        "{}",
        err.brief()
    );
    assert!(wt.join("wip.txt").exists());
    assert_eq!(worktree_count(&f), 2);

    remove_worktree(&api, &wt, true).unwrap();

    assert!(!wt.exists());
    assert_eq!(worktree_count(&f), 1);
    git(&api, &["rev-parse", "--verify", "-q", "refs/heads/feature"]);
}
