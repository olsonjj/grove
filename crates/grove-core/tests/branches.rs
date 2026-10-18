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
