//! Removing a linked worktree: what the tree and details pane offer, the confirm's words, and
//! the git write (faked in tests).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::{App, Global};
use grove_core::{GitError, RemovalPlan, Worktree, remove_worktree};

use crate::main_window::tilde;

/// The tree's menu item or the details pane's button asked to remove (or prune) a worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoveRequested {
    pub path: PathBuf,
}

/// The remove action as offered for a worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub label: &'static str,
    /// Why it's unavailable, shown beside it.
    pub reason: Option<String>,
}

/// `None` on the main worktree: removing it would remove the repo.
pub fn offer(wt: &Worktree) -> Option<Offer> {
    if wt.is_main {
        return None;
    }
    let remove = "Remove worktree…";
    Some(match RemovalPlan::gate(wt) {
        Some(RemovalPlan::Missing) => Offer {
            label: "Remove missing entry…",
            reason: None,
        },
        Some(RemovalPlan::NotAllowed(why)) => Offer {
            label: remove,
            reason: Some(format!("Can't remove: {why}")),
        },
        _ => Offer {
            label: remove,
            reason: None,
        },
    })
}

/// The confirm's words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirm {
    pub title: String,
    pub message: String,
    pub ok: &'static str,
    pub destructive: bool,
}

const IGNORED: &str = "Ignored files (build output, dependencies) are deleted too.";

/// `None` unless `plan` removes the folder.
pub fn confirm(wt: &Worktree, plan: &RemovalPlan) -> Option<Confirm> {
    let path = tilde(&wt.path);
    let kept = match (&wt.branch, &wt.head) {
        (Some(b), _) => format!("Branch {b} is kept."),
        (None, Some(sha)) => format!(
            "It's detached at {}; commits on no branch may become unreachable.",
            sha.get(..7).unwrap_or(sha)
        ),
        (None, None) => String::new(),
    };
    let join = |parts: &[&str]| {
        parts
            .iter()
            .filter(|p| !p.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join(" ")
    };
    match plan {
        RemovalPlan::Clean => Some(Confirm {
            title: "Remove worktree?".into(),
            message: join(&[&format!("Remove {path}?"), &kept, IGNORED]),
            ok: "Remove",
            destructive: false,
        }),
        RemovalPlan::Dirty {
            modified,
            untracked,
        } => Some(Confirm {
            title: "Discard changes and remove worktree?".into(),
            message: join(&[
                &format!("Remove {path}?"),
                &format!(
                    "{} will be permanently deleted.",
                    changes(*modified, *untracked)
                ),
                IGNORED,
                &kept,
            ]),
            ok: "Discard and remove",
            destructive: true,
        }),
        // Nothing on disk to lose, but a detached HEAD's commits can still go unreachable.
        RemovalPlan::Missing => Some(Confirm {
            title: "Remove missing worktree entry?".into(),
            message: join(&[
                &format!("{path} is already gone. Remove Git's record of it?"),
                &kept,
            ]),
            ok: "Remove entry",
            destructive: false,
        }),
        RemovalPlan::NotAllowed(_) => None,
    }
}

/// "3 modified and 2 untracked files", "1 modified file", "1 modified and 1 untracked files".
fn changes(modified: usize, untracked: usize) -> String {
    let files = |n| if n == 1 { "file" } else { "files" };
    match (modified, untracked) {
        (m, 0) => format!("{m} modified {}", files(m)),
        (0, u) => format!("{u} untracked {}", files(u)),
        (m, u) => format!("{m} modified and {u} untracked {}", files(m + u)),
    }
}

/// Removes the worktree at the second path from the repo at the first, forced or not.
pub type Run = Arc<dyn Fn(&Path, &Path, bool) -> Result<(), GitError> + Send + Sync>;

/// Overrides the git write (tests).
pub struct Remover {
    pub run: Run,
}

impl Global for Remover {}

pub fn runner(cx: &App) -> Run {
    cx.try_global::<Remover>()
        .map(|r| r.run.clone())
        .unwrap_or_else(|| Arc::new(run))
}

/// Blocking.
pub fn run(repo: &Path, worktree: &Path, force: bool) -> Result<(), GitError> {
    remove_worktree(repo, worktree, force)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::wt;

    #[test]
    fn clean_confirm_names_path_and_kept_branch() {
        let c = confirm(
            &wt("/w/api-x", Some("feature/x"), false),
            &RemovalPlan::Clean,
        )
        .unwrap();
        assert_eq!(
            c.message,
            "Remove /w/api-x? Branch feature/x is kept. Ignored files (build output, \
             dependencies) are deleted too."
        );
        assert_eq!(c.ok, "Remove");
        assert!(!c.destructive);
    }

    #[test]
    fn dirty_confirm_counts_what_is_lost() {
        let dirty = |m, u| RemovalPlan::Dirty {
            modified: m,
            untracked: u,
        };
        let w = wt("/w/api-x", Some("feature/x"), false);
        let c = confirm(&w, &dirty(3, 2)).unwrap();
        assert_eq!(
            c.message,
            "Remove /w/api-x? 3 modified and 2 untracked files will be permanently deleted. \
             Ignored files (build output, dependencies) are deleted too. Branch feature/x is \
             kept."
        );
        assert_eq!(c.ok, "Discard and remove");
        assert!(c.destructive);
        assert_eq!(changes(1, 0), "1 modified file");
        assert_eq!(changes(0, 1), "1 untracked file");
        assert_eq!(changes(2, 1), "2 modified and 1 untracked files");
        assert_eq!(changes(1, 1), "1 modified and 1 untracked files");
    }

    #[test]
    fn missing_entry_confirm_is_plain_and_keeps_the_branch() {
        let mut w = wt("/w/api-x", Some("feature/x"), false);
        w.presence = grove_core::Presence::Missing;
        assert_eq!(offer(&w).unwrap().label, "Remove missing entry…");
        let c = confirm(&w, &RemovalPlan::Missing).unwrap();
        assert_eq!(
            c.message,
            "/w/api-x is already gone. Remove Git's record of it? Branch feature/x is kept."
        );
        assert_eq!(c.ok, "Remove entry");
        assert!(!c.destructive);
    }
}
