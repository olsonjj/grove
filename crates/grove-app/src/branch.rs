//! The branch switcher: the details pane's control, and the git write and branch-list
//! read behind it (faked in tests).

use std::path::Path;
use std::sync::Arc;

use gpui_kit::component::IndexPath;
use gpui_kit::component::searchable_list::{
    SearchableListDelegate, SearchableListItem, SearchableVec,
};
use gpui_kit::{
    App, Global, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, TestSupportExt, Window, div, px,
};
use grove_core::{GitError, Worktree, git};

use crate::{icons, theme};

/// One branch the details pane's dropdown offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    /// Short branch name.
    pub branch: String,
    /// Another worktree's folder name with the trailing `/` where the branch is already
    /// checked out, which makes the pick impossible; `None` means the pick is possible.
    pub held_by: Option<String>,
}

fn folder_of(path: &Path) -> String {
    format!(
        "{}/",
        path.file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy()
    )
}

/// Another worktree's folder name, with the trailing `/`, where `branch` is checked out —
/// `except` being the worktree whose picker asks — or `None` when the pick is possible.
pub fn held_by(worktrees: &[Worktree], branch: &str, except: &Path) -> Option<String> {
    worktrees
        .iter()
        .filter(|wt| wt.branch.as_deref() == Some(branch) && wt.path != except)
        .min_by(|a, b| a.path.cmp(&b.path))
        .map(|wt| folder_of(&wt.path))
}

/// One dropdown item per local branch, alphabetical, with each branch's checked-out
/// status joined against the snapshot's worktrees. Derived per render, never cached: the
/// snapshot already maps branches to the worktrees holding them.
pub fn items(selected: &Path, list: &[String], worktrees: &[Worktree]) -> Vec<Offer> {
    let mut out: Vec<Offer> = list
        .iter()
        .map(|branch| Offer {
            branch: branch.clone(),
            held_by: held_by(worktrees, branch, selected),
        })
        .collect();
    out.sort_by(|a, b| a.branch.cmp(&b.branch));
    out
}

impl SearchableListItem for Offer {
    type Value = String;

    fn title(&self) -> SharedString {
        self.branch.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.branch
    }
}

/// The branch dropdown's delegate: the offers, searched through the component's
/// [`SearchableVec`], with rows drawn by Grove — the current branch carries the check, and
/// a branch another worktree holds is grayed and annotated with that worktree's folder.
#[derive(Debug, Clone)]
pub struct BranchMenu {
    offers: SearchableVec<Offer>,
}

impl Default for BranchMenu {
    fn default() -> Self {
        Self {
            offers: SearchableVec::new(Vec::new()),
        }
    }
}

impl BranchMenu {
    pub fn new(offers: Vec<Offer>) -> Self {
        Self {
            offers: SearchableVec::new(offers),
        }
    }
}

impl SearchableListDelegate for BranchMenu {
    type Item = Offer;

    fn items_count(&self, section: usize) -> usize {
        self.offers.items_count(section)
    }

    fn item(&self, ix: IndexPath) -> Option<&Offer> {
        self.offers.item(ix)
    }

    fn position<V>(&self, value: &V) -> Option<IndexPath>
    where
        Self::Item: SearchableListItem<Value = V>,
        V: PartialEq,
    {
        self.offers.position(value)
    }

    fn perform_search(
        &mut self,
        query: &str,
        window: &mut Window,
        cx: &mut App,
    ) -> gpui_kit::Task<()> {
        self.offers.perform_search(query, window, cx)
    }

    fn render_item(
        &self,
        ix: IndexPath,
        item: &Offer,
        checked: bool,
        _: &mut Window,
        _: &mut App,
    ) -> Option<gpui_kit::AnyElement> {
        // The row's accessibility label stands on its own: a possible pick is the branch
        // name, an impossible one says which worktree holds it.
        let label = item.held_by.as_ref().map_or_else(
            || item.branch.clone(),
            |held| format!("{}, checked out in worktree {held}", item.branch),
        );
        Some(
            div()
                .id(("branch-item", ix.row))
                .aria_label(label.clone())
                .flex()
                .w_full()
                .min_w_0()
                .items_center()
                .justify_between()
                .gap(px(8.))
                .child(
                    div()
                        .flex()
                        .min_w_0()
                        .items_center()
                        .gap(px(4.))
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_color(if item.held_by.is_none() {
                                    theme::text()
                                } else {
                                    // Grayed: another worktree holds it.
                                    theme::muted()
                                })
                                .child(item.branch.clone()),
                        )
                        .children(item.held_by.clone().map(|held| {
                            div()
                                .flex_none()
                                .text_size(px(11.))
                                .text_color(theme::faint())
                                .child(format!("in `{held}`"))
                        })),
                )
                .children(checked.then(|| {
                    div()
                        .id(("branch-check", ix.row))
                        .test_support()
                        .flex_none()
                        .child(crate::icons::icon(icons::CHECK, 12.).text_color(theme::text()))
                }))
                .test_support()
                .into_any_element(),
        )
    }
}

/// Switching the worktree at `path` to `branch`. Blocking.
pub type SwitchRun = Arc<dyn Fn(&Path, &str) -> Result<(), GitError> + Send + Sync>;

/// Listing the repo at `path`'s local branches. Blocking.
pub type ListRun = Arc<dyn Fn(&Path) -> Result<Vec<String>, GitError> + Send + Sync>;

/// Overrides the git write (tests).
#[allow(dead_code, reason = "fakes install in the test suites that need them")]
pub struct SwitchRunner {
    pub run: SwitchRun,
}

impl Global for SwitchRunner {}

/// Overrides the branch-list read (tests).
#[allow(dead_code, reason = "fakes install in the test suites that need them")]
pub struct ListRunner {
    pub run: ListRun,
}

impl Global for ListRunner {}

/// The switch write, or the installed fake.
pub fn switch_runner(cx: &App) -> SwitchRun {
    cx.try_global::<SwitchRunner>()
        .map(|s| s.run.clone())
        .unwrap_or_else(|| Arc::new(run_switch))
}

/// The branch-list read, or the installed fake.
pub fn list_runner(cx: &App) -> ListRun {
    cx.try_global::<ListRunner>()
        .map(|l| l.run.clone())
        .unwrap_or_else(|| Arc::new(run_list))
}

/// Blocking.
pub fn run_switch(path: &Path, branch: &str) -> Result<(), GitError> {
    git::switch(path, branch)
}

/// Blocking.
pub fn run_list(path: &Path) -> Result<Vec<String>, GitError> {
    git::branches(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::wt;

    #[test]
    fn items_are_alphabetical_and_include_unchecked_out_branches() {
        let list = ["spare", "main", "feature/x"].map(str::to_string);
        let worktrees = [wt("/c/api", Some("main"), true)];

        let offers = items(Path::new("/c/api"), &list, &worktrees);

        assert_eq!(
            offers,
            [
                Offer {
                    branch: "feature/x".into(),
                    held_by: None,
                },
                Offer {
                    branch: "main".into(),
                    held_by: None,
                },
                Offer {
                    branch: "spare".into(),
                    held_by: None,
                },
            ]
        );
    }

    #[test]
    fn a_branch_held_by_another_worktree_names_its_folder() {
        let list = ["main".to_string(), "feature/x".to_string()];
        let worktrees = [
            wt("/c/api", Some("main"), true),
            wt("/c/wt/api-x", Some("feature/x"), false),
        ];

        let offers = items(Path::new("/c/api"), &list, &worktrees);

        assert_eq!(
            offers,
            [
                Offer {
                    branch: "feature/x".into(),
                    held_by: Some("api-x/".into()),
                },
                Offer {
                    branch: "main".into(),
                    held_by: None,
                },
            ]
        );
    }

    #[test]
    fn a_branch_two_worktrees_hold_names_the_other_one() {
        // The selected worktree shares its branch with a linked one; only the other
        // worktree makes further picks of that branch impossible.
        let worktrees = [
            wt("/c/api", Some("main"), true),
            wt("/c/wt/api-a", Some("main"), false),
        ];

        let offers = items(Path::new("/c/wt/api-a"), &["main".to_string()], &worktrees);

        assert_eq!(offers[0].held_by.as_deref(), Some("api/"));
    }

    #[test]
    fn menu_addresses_the_offers_by_row_index() {
        // The menu's delegate stays addressable after selections; the row ids are keyed
        // by row index, which the UI tests click.
        let menu = BranchMenu::new(vec![Offer {
            branch: "main".into(),
            held_by: None,
        }]);
        assert_eq!(menu.items_count(0), 1);
        assert_eq!(
            menu.position(&"main".to_string()),
            Some(IndexPath::default())
        );
    }
}
