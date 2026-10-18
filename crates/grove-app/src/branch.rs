//! The branch switcher's git write and branch-list read (faked in tests).

use std::path::Path;
use std::sync::Arc;

use gpui_kit::{App, Global};
use grove_core::{GitError, git};

/// Switching the worktree at `path` to `branch`. Blocking.
pub type SwitchRun = Arc<dyn Fn(&Path, &str) -> Result<(), GitError> + Send + Sync>;

/// Listing the repo at `path`'s local branches. Blocking.
pub type ListRun = Arc<dyn Fn(&Path) -> Result<Vec<String>, GitError> + Send + Sync>;

/// Overrides the git write (tests).
#[allow(dead_code, reason = "the branch trigger (ticket 25) consumes the seam")]
pub struct SwitchRunner {
    pub run: SwitchRun,
}

impl Global for SwitchRunner {}

/// Overrides the branch-list read (tests).
#[allow(dead_code, reason = "the branch trigger (ticket 25) consumes the seam")]
pub struct ListRunner {
    pub run: ListRun,
}

impl Global for ListRunner {}

/// The switch write, or the installed fake.
#[allow(dead_code, reason = "the branch trigger (ticket 25) consumes the seam")]
pub fn switch_runner(cx: &App) -> SwitchRun {
    cx.try_global::<SwitchRunner>()
        .map(|s| s.run.clone())
        .unwrap_or_else(|| Arc::new(run_switch))
}

/// The branch-list read, or the installed fake.
#[allow(dead_code, reason = "the branch trigger (ticket 25) consumes the seam")]
pub fn list_runner(cx: &App) -> ListRun {
    cx.try_global::<ListRunner>()
        .map(|l| l.run.clone())
        .unwrap_or_else(|| Arc::new(run_list))
}

/// Blocking.
#[allow(dead_code, reason = "the branch trigger (ticket 25) consumes the seam")]
pub fn run_switch(path: &Path, branch: &str) -> Result<(), GitError> {
    git::switch(path, branch)
}

/// Blocking.
#[allow(dead_code, reason = "the branch trigger (ticket 25) consumes the seam")]
pub fn run_list(path: &Path) -> Result<Vec<String>, GitError> {
    git::branches(path)
}
