//! Fetch starts no other git writes. Its own binary: it sets `GIT_TRACE` for the process.

mod common;

use std::fs;

use common::{Fixture, git, init_repo};
use grove_core::SnapshotStore;

#[test]
fn fetch_all_starts_no_auto_maintenance() {
    let f = Fixture::new();
    let remotes = f.tmp.path().join("remotes");
    fs::create_dir_all(&remotes).unwrap();
    git(&remotes, &["init", "-q", "--bare", "origin.git"]);
    let api = init_repo(&f.code().join("api"));
    let origin = remotes.join("origin.git");
    git(&api, &["remote", "add", "origin", origin.to_str().unwrap()]);
    git(&api, &["push", "-q", "-u", "origin", "main"]);
    let mut store = SnapshotStore::new(&f.code());
    let trace = f.tmp.path().join("trace");
    // SAFETY: the only test in this binary, so no other thread reads the environment.
    unsafe { std::env::set_var("GIT_TRACE", &trace) };

    let report = store.fetch_all().unwrap();

    assert_eq!(report.message(), "Fetched 1 repo");
    let trace = fs::read_to_string(&trace).unwrap();
    assert!(trace.contains("fetch"), "{trace}");
    assert!(!trace.contains("maintenance run"), "{trace}");
}
