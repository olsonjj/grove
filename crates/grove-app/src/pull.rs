//! Fast-forward pulling the selected worktree: when the details pane offers it, and the git
//! write (faked in tests).

use std::path::Path;
use std::sync::Arc;

use gpui_kit::{App, Global};
use grove_core::{GitError, Upstream, Worktree};

use grove_core::git::pull;

/// Pull's button words; `None` unless the worktree is openable, on a branch, and behind its
/// upstream. Stale counts don't block: pull fetches for itself, and `--ff-only` makes even a
/// wrong attempt harmless.
pub fn offer(wt: &Worktree) -> Option<&'static str> {
    if !wt.presence.is_openable() || wt.branch.is_none() || wt.detached {
        return None;
    }
    let status = wt.status.as_ref()?.as_ref().ok()?;
    match &status.upstream {
        Upstream::Tracking { behind, .. } if *behind > 0 => Some("Pull"),
        _ => None,
    }
}

/// Pulling the worktree at `path`.
pub type Run = Arc<dyn Fn(&Path) -> Result<(), GitError> + Send + Sync>;

/// Overrides the git write (tests).
pub struct Puller {
    pub run: Run,
}

impl Global for Puller {}

pub fn runner(cx: &App) -> Run {
    cx.try_global::<Puller>()
        .map(|p| p.run.clone())
        .unwrap_or_else(|| Arc::new(run))
}

/// Blocking.
pub fn run(path: &Path) -> Result<(), GitError> {
    pull(path)
}

#[cfg(test)]
mod tests {
    use grove_core::{Presence, WorktreeStatus};

    use super::*;
    use crate::test_support::wt;

    fn status(upstream: Upstream) -> Option<Result<WorktreeStatus, GitError>> {
        Some(Ok(WorktreeStatus {
            files: vec![],
            upstream,
            head: None,
            head_time: None,
            changes_time: None,
        }))
    }

    fn behind(n: u32) -> Upstream {
        Upstream::Tracking {
            name: "origin/main".into(),
            ahead: 0,
            behind: n,
        }
    }

    /// A linked worktree with `status` instead of the default (none).
    fn wt_with(mut wt: Worktree, status: Option<Result<WorktreeStatus, GitError>>) -> Worktree {
        wt.status = status;
        wt
    }

    #[test]
    fn offered_when_behind_its_upstream() {
        let offered = |wt: &Worktree| offer(wt).unwrap().to_string();
        assert_eq!(
            offered(&wt_with(
                wt("/w/api-x", Some("main"), true),
                status(behind(1))
            )),
            "Pull"
        );
        assert_eq!(
            offered(&wt_with(
                wt("/w/api-x", Some("feature/x"), false),
                status(behind(30))
            )),
            "Pull"
        );
    }

    #[test]
    fn not_offered_without_something_to_pull() {
        let none = |s| offer(&wt_with(wt("/w/api-x", Some("main"), true), s)).is_none();
        let no = |w: Worktree| offer(&wt_with(w, status(behind(1)))).is_none();
        // Up to date, and status still unread ("reading…").
        assert!(none(status(Upstream::Tracking {
            name: "origin/main".into(),
            ahead: 0,
            behind: 0,
        })));
        assert!(none(None));
        // No upstream, upstream gone.
        assert!(none(status(Upstream::None)));
        assert!(none(status(Upstream::Gone {
            name: "origin/main".into(),
        })));

        // Detached, or a status read failed.
        let detached = wt("/w/bisect", None, true);
        assert!(no(detached));
        assert!(none(Some(Err(GitError::Failed {
            args: "status".into(),
            stderr: "fatal: x\n".into(),
        }))));
    }

    #[test]
    fn not_offered_for_unopenable_worktrees() {
        let missing = {
            let mut w = wt("/w/api-x", Some("main"), false);
            w.presence = Presence::Missing;
            w
        };
        assert!(offer(&wt_with(missing, status(behind(1)))).is_none());
    }

    #[test]
    fn main_worktree_pull_needs_only_upstream_counts() {
        // is_main doesn't gate pull: the main worktree pulls like any other.
        assert!(
            offer(&wt_with(
                wt("/w/api", Some("main"), true),
                status(behind(1))
            ))
            .is_some()
        );
    }
}
