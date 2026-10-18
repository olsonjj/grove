//! Local branch list through the git module, against real git in temp dirs.

mod common;

use std::fs;

use common::{Fixture, add_worktree, git, init_repo};
use grove_core::git::branches;

#[test]
fn lists_all_local_branches_alphabetically() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    // `feature` is checked out in a linked worktree, `spare` was never checked out.
    fs::create_dir_all(f.outside("")).unwrap();
    add_worktree(&api, &f.outside("api-feature"), "feature");
    git(&api, &["branch", "spare"]);

    assert_eq!(branches(&api).unwrap(), ["feature", "main", "spare"]);
}

#[test]
fn empty_list_before_the_first_branch() {
    let f = Fixture::new();
    let fresh = f.code().join("fresh");
    fs::create_dir_all(&fresh).unwrap();
    git(&fresh, &["init", "-q"]);

    assert_eq!(branches(&fresh).unwrap(), Vec::<String>::new());
}

#[test]
fn a_branch_sharing_a_tags_name_lists_under_its_plain_name() {
    // `%(refname:short)` abbreviates a branch that shares a tag's name to
    // `heads/<name>` — a value that matches no worktree's branch and selects nothing
    // when handed to switch. The full refname is used and stripped instead.
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    git(&api, &["tag", "release"]);
    git(&api, &["branch", "release"]);

    assert_eq!(branches(&api).unwrap(), ["main", "release"]);
}
