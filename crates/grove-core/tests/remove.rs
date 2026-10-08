//! Removing linked worktrees, against real git in temp dirs.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{Fixture, add_worktree, git, init_repo};
use grove_core::{RemovalPlan, read_removal_plan, remove_worktree};

/// The plan for the worktree at `path` of repo `api`, from a fresh scan.
fn plan(f: &Fixture, path: &Path) -> RemovalPlan {
    let snapshot = f.scan();
    let repo = snapshot.repo("api").unwrap();
    let wt = repo
        .worktrees
        .iter()
        .find(|w| w.path.canonicalize().ok() == path.canonicalize().ok() || w.path == path)
        .unwrap_or_else(|| panic!("no worktree {} in {repo:#?}", path.display()));
    read_removal_plan(&repo.repo.git_dir, wt)
}

/// `api` with a linked worktree on branch `feature`, outside the code dir.
fn linked(f: &Fixture) -> (PathBuf, PathBuf) {
    let api = init_repo(&f.code().join("api"));
    let wt = f.outside("api-feature");
    fs::create_dir_all(wt.parent().unwrap()).unwrap();
    add_worktree(&api, &wt, "feature");
    (api, wt)
}

fn worktrees(f: &Fixture) -> Vec<PathBuf> {
    f.scan()
        .repo("api")
        .unwrap()
        .worktrees
        .iter()
        .map(|w| w.path.clone())
        .collect()
}

fn not_allowed(why: &str) -> RemovalPlan {
    RemovalPlan::NotAllowed(why.into())
}

#[test]
fn main_worktree_is_not_removable() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));

    assert_eq!(plan(&f, &api), not_allowed("it's the main worktree"));
}

#[test]
fn locked_worktree_is_not_removable() {
    let f = Fixture::new();
    let (api, wt) = linked(&f);
    git(&api, &["worktree", "lock", wt.to_str().unwrap()]);

    assert_eq!(
        plan(&f, &wt),
        not_allowed("it's locked (run git worktree unlock first)")
    );
}

#[test]
fn missing_worktree_removal_drops_only_its_own_entry() {
    let f = Fixture::new();
    let (api, wt) = linked(&f);
    // Another stale entry, e.g. on an unmounted drive: repo-wide prune would drop it too.
    let other = f.outside("api-other");
    add_worktree(&api, &other, "other");
    fs::remove_dir_all(&wt).unwrap();
    fs::remove_dir_all(&other).unwrap();
    let before = worktrees(&f);
    assert_eq!(plan(&f, &before[1]), RemovalPlan::Missing);

    remove_worktree(&api, &before[1], false).unwrap();

    assert_eq!(worktrees(&f), [before[0].clone(), before[2].clone()]);
    git(&api, &["rev-parse", "--verify", "-q", "refs/heads/feature"]);
}

#[test]
fn clean_worktree_plans_plain_removal() {
    let f = Fixture::new();
    let (_, wt) = linked(&f);

    let plan = plan(&f, &wt);

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

    let plan = plan(&f, &wt);

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
        plan(&f, &wt),
        RemovalPlan::Dirty {
            modified: 0,
            untracked: 3
        }
    );
}

/// `api` with an initialized submodule in linked worktree `api-feature`, added with
/// `add_config` (e.g. relative paths).
fn submodule_worktree(f: &Fixture, add_config: &[&str]) -> PathBuf {
    let sub = init_repo(&f.tmp.path().join("sub"));
    let api = init_repo(&f.code().join("api"));
    let allow = ["-c", "protocol.file.allow=always"];
    git(
        &api,
        &[
            &allow[..],
            &["submodule", "-q", "add", sub.to_str().unwrap(), "sub"],
        ]
        .concat(),
    );
    git(&api, &["commit", "-q", "-m", "sub"]);
    let wt = f.outside("api-feature");
    git(
        &api,
        &[
            add_config,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feature",
                wt.to_str().unwrap(),
            ],
        ]
        .concat(),
    );
    git(
        &wt,
        &[&allow[..], &["submodule", "-q", "update", "--init"]].concat(),
    );
    wt
}

#[test]
fn worktree_with_submodules_is_not_removable() {
    let f = Fixture::new();
    let wt = submodule_worktree(&f, &[]);

    // Clean as far as status goes, but --force would delete the submodule's repo.
    assert_eq!(
        plan(&f, &wt),
        not_allowed("it contains submodules (remove it with git)")
    );
}

#[test]
fn submodules_are_found_through_relative_gitdirs() {
    let f = Fixture::new();
    let wt = submodule_worktree(&f, &["-c", "worktree.useRelativePaths=true"]);
    // Dirty otherwise, so --force would take the submodule along.
    fs::write(wt.join("wip.txt"), "w").unwrap();

    assert_eq!(
        plan(&f, &wt),
        not_allowed("it contains submodules (remove it with git)")
    );
}

#[test]
fn missing_worktree_with_relative_gitdir_and_submodules_is_not_removable() {
    let f = Fixture::new();
    let wt = submodule_worktree(&f, &["-c", "worktree.useRelativePaths=true"]);
    fs::remove_dir_all(&wt).unwrap();
    let missing = worktrees(&f)[1].clone();

    assert_eq!(
        plan(&f, &missing),
        not_allowed("it contains submodules (remove it with git)")
    );
}

#[test]
fn ignored_folder_holding_another_repo_is_not_removable() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    fs::write(api.join(".gitignore"), "vendor/\nnode_modules/\n").unwrap();
    git(&api, &["add", ".gitignore"]);
    git(&api, &["commit", "-q", "-m", "ignore"]);
    let wt = f.outside("api-feature");
    add_worktree(&api, &wt, "feature");
    fs::create_dir_all(wt.join("node_modules/pkg")).unwrap();
    fs::write(wt.join("node_modules/pkg/index.js"), "").unwrap();
    init_repo(&wt.join("vendor/deep/lib"));

    // Clean to status, so a plain remove would delete the repo unseen.
    assert_eq!(
        plan(&f, &wt),
        not_allowed("vendor/deep/lib/ inside it is a separate Git repository (remove it with git)")
    );
}

#[test]
fn a_missing_folder_that_came_back_is_read_fresh() {
    let f = Fixture::new();
    let (_, wt) = linked(&f);
    let away = f.outside("away");
    fs::rename(&wt, &away).unwrap();
    let snapshot = f.scan();
    let repo = snapshot.repo("api").unwrap();
    let stale = &repo.worktrees[1];
    assert_eq!(
        read_removal_plan(&repo.repo.git_dir, stale),
        RemovalPlan::Missing
    );
    fs::rename(&away, &wt).unwrap();
    fs::write(wt.join("wip.txt"), "w").unwrap();

    assert_eq!(
        read_removal_plan(&repo.repo.git_dir, stale),
        RemovalPlan::Dirty {
            modified: 0,
            untracked: 1
        }
    );
}

#[test]
fn worktree_holding_another_repo_is_not_removable() {
    let f = Fixture::new();
    let (_, wt) = linked(&f);
    init_repo(&wt.join("nested"));

    assert_eq!(
        plan(&f, &wt),
        not_allowed("nested/ inside it is a separate Git repository (remove it with git)")
    );
}

#[test]
fn removing_a_clean_worktree_keeps_its_branch() {
    let f = Fixture::new();
    let (api, wt) = linked(&f);

    remove_worktree(&api, &wt, false).unwrap();

    assert!(!wt.exists());
    assert_eq!(worktrees(&f).len(), 1);
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
    assert_eq!(worktrees(&f).len(), 2);

    remove_worktree(&api, &wt, true).unwrap();

    assert!(!wt.exists());
    assert_eq!(worktrees(&f).len(), 1);
    git(&api, &["rev-parse", "--verify", "-q", "refs/heads/feature"]);
}
