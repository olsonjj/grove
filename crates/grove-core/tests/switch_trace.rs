//! Switch runs exactly the quiet switch command, no remote-branch guess and no auto
//! maintenance. Its own binary: it sets `GIT_TRACE` for the process.

mod common;

use std::fs;

use common::{Fixture, git, init_repo};
use grove_core::git::switch;

/// The command switch runs, traced: `switch --quiet --no-guess -- <branch>` under the
/// writer config.
#[test]
fn switch_runs_quiet_without_auto_maintenance() {
    let f = Fixture::new();
    let api = init_repo(&f.code().join("api"));
    git(&api, &["branch", "big"]);
    let trace = f.tmp.path().join("trace");
    let trace2 = f.tmp.path().join("trace2.json");
    // SAFETY: the only test in this binary, so no other thread reads the environment.
    unsafe {
        std::env::set_var("GIT_TRACE", &trace);
        std::env::set_var("GIT_TRACE2_EVENT", &trace2);
    }

    switch(&api, "big").unwrap();

    let trace = fs::read_to_string(&trace).unwrap();
    assert!(trace.contains("trace: built-in: git switch"), "{trace}");
    // Maintenance, which a checkout could otherwise trigger, is off.
    assert!(!trace.contains("maintenance run"), "{trace}");
    // GIT_TRACE2's start event records the full argv as run, config overrides included.
    let trace2 = fs::read_to_string(&trace2).unwrap();
    let start = trace2
        .lines()
        .find(|l| l.contains("\"event\":\"start\""))
        .unwrap_or_else(|| panic!("no start event in {trace2}"));
    assert!(
        start
            .contains("\"-c\",\"maintenance.auto=false\",\"-c\",\"gc.auto=0\",\"switch\",\"--quiet\",\"--no-guess\",\"--\",\"big\""),
        "{start}"
    );
}
