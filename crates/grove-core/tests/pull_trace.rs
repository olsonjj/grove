//! Pull runs exactly the fast-forward command, no auto maintenance. Its own binary: it
//! sets `GIT_TRACE` for the process.

mod common;

use std::fs;

use common::{Fixture, git, init_repo};
use grove_core::git::pull;

/// The command pull runs, traced: `pull --ff-only --quiet` under the writer config.
#[test]
fn pull_runs_ff_only_without_auto_maintenance() {
    let f = Fixture::new();
    let remotes = f.tmp.path().join("remotes");
    fs::create_dir_all(&remotes).unwrap();
    let origin = remotes.join("origin.git");
    git(&remotes, &["init", "-q", "--bare", "origin.git"]);
    let api = init_repo(&f.code().join("api"));
    git(&api, &["remote", "add", "origin", origin.to_str().unwrap()]);
    git(&api, &["push", "-q", "-u", "origin", "main"]);
    let trace = f.tmp.path().join("trace");
    let trace2 = f.tmp.path().join("trace2.json");
    // SAFETY: the only test in this binary, so no other thread reads the environment.
    unsafe {
        std::env::set_var("GIT_TRACE", &trace);
        std::env::set_var("GIT_TRACE2_EVENT", &trace2);
    }

    pull(&api).unwrap();

    let trace = fs::read_to_string(&trace).unwrap();
    assert!(trace.contains("trace: built-in: git pull"), "{trace}");
    // Auto maintenance, which a pull would otherwise start after its fetch, is off.
    assert!(!trace.contains("maintenance run"), "{trace}");
    // GIT_TRACE2's start event records the full argv as run, config overrides included.
    let trace2 = fs::read_to_string(&trace2).unwrap();
    let start = trace2
        .lines()
        .find(|l| l.contains("\"event\":\"start\""))
        .unwrap_or_else(|| panic!("no start event in {trace2}"));
    assert!(
        start
            .contains("\"-c\",\"maintenance.auto=false\",\"-c\",\"gc.auto=0\",\"pull\",\"--ff-only\",\"--quiet\""),
        "{start}"
    );
}
