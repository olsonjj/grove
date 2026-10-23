use std::collections::HashMap;

use crate::details_view::{self, DetailsView};
use crate::removal::RemoveRequested;
use crate::sort_menu::{SortChosen, SortMenu, SortMenuIds};
use crate::tree_view::{CollapseChanged, OpenRequested, TreeView};
use crate::{launcher, shell};
use gpui_kit::component::{ResizablePanelEvent, ResizableState};
use gpui_kit::{
    AppContext, Context, Entity, FocusHandle, Focusable, ScrollHandle, Subscription, Window,
};

mod app_model;
mod layout;
mod removal_flow;
mod sidebar;
mod window;

pub use layout::Status;
pub use sidebar::rows::tilde;
pub use window::open;

use app_model::selected_repo;

use removal_flow::Removal;
use sidebar::RepoMenu;
use sidebar::visibility::VisibilityPopover;
/// Keeps "Last fetch … ago" current.
const CLOCK_TICK: std::time::Duration = std::time::Duration::from_secs(30);

struct MainWindow {
    tree: Entity<TreeView>,
    /// Shared sizes of the branch-tree / details-pane split; its `Resized` event
    /// persists the pane's width once per drag or programmatic resize.
    panes: Entity<ResizableState>,
    details: Entity<DetailsView>,
    status: Option<Status>,
    /// Bumped per open or notice so a slow earlier launch can't overwrite a newer status.
    open_seq: u64,
    /// The last Fetch all report shown (`Fetcher::finished`).
    fetch_seen: u64,
    /// The last `Notices::posted` shown.
    notices_seen: u64,
    /// Sidebar rows' focus (row, pin button) by repo name, so it follows a row when pinning
    /// moves it.
    repo_focus: HashMap<String, [FocusHandle; 2]>,
    hovered_repo: Option<String>,
    repo_menu: Option<RepoMenu>,
    repo_menu_item: FocusHandle,
    repo_sort: Entity<SortMenu>,
    /// The manage-visibility button, so closing the popover refocuses it.
    visibility_button: FocusHandle,
    /// The open manage-visibility popover.
    visibility: Option<VisibilityPopover>,
    repo_scroll: ScrollHandle,
    /// The sidebar order last rendered, to scroll the selection back into view on a reorder.
    shown_order: Vec<String>,
    /// The open remove confirm.
    removal: Option<Removal>,
    _subs: [Subscription; 13],
}

impl MainWindow {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let tree = cx.new(TreeView::new);
        let collapse = cx.subscribe(&tree, |_, _, ev: &CollapseChanged, cx| {
            shell::set_collapsed(&ev.repo, &ev.prefix, ev.collapsed, cx);
        });
        let open = cx.subscribe(&tree, |this, _, ev: &OpenRequested, cx| {
            this.open(ev, cx);
        });
        let tree_sort = cx.subscribe(&tree, |_, _, ev: &SortChosen, cx| {
            shell::set_tree_sort(ev.0, cx);
        });
        let ids = SortMenuIds {
            button: "repo-sort",
            menu: "repo-sort-menu",
            choice: "repo-sort-choice",
            what: "repositories",
        };
        let mode = shell::library(cx).settings.repo_sort;
        let repo_sort = cx.new(|cx| SortMenu::new(ids, mode, cx));
        let repo_sort_chosen = cx.subscribe(&repo_sort, |_, _, ev: &SortChosen, cx| {
            shell::set_repo_sort(ev.0, cx);
        });
        let details = cx.new(|cx| DetailsView::new(tree.clone(), cx));
        let panes = cx.new(|_| ResizableState::default());
        let pane_saved = cx.subscribe(&panes, |_, panes, ev: &ResizablePanelEvent, cx| {
            if let (ResizablePanelEvent::Resized, Some(width)) = (ev, panes.read(cx).sizes().get(1))
            {
                shell::set_details_pane_width(width.round().as_f32() as i32, cx);
            }
        });
        let details_open = cx.subscribe(&details, |this, _, ev: &OpenRequested, cx| {
            this.open(ev, cx);
        });
        let tree_remove = cx.subscribe_in(&tree, window, |this, _, ev: &RemoveRequested, w, cx| {
            this.request_removal(&ev.path, w, cx);
        });
        let details_remove =
            cx.subscribe_in(&details, window, |this, _, ev: &RemoveRequested, w, cx| {
                this.request_removal(&ev.path, w, cx);
            });
        let notice = cx.subscribe(&details, |this, _, ev: &details_view::Notice, cx| {
            this.open_seq += 1;
            this.set_status(ev.text.clone(), ev.error, false, cx);
        });
        let library = cx.observe_global::<shell::Library>(|this, cx| this.sync_repo(cx));
        let fetcher = cx.observe_global::<shell::Fetcher>(|this, cx| this.fetch_changed(cx));
        let notices = cx.observe_global::<shell::Notices>(|this, cx| this.notice_posted(cx));
        let activation = cx.observe_window_activation(window, |_, window, cx| {
            if window.is_window_active() {
                shell::refresh_all(cx);
            }
        });
        window.focus(&tree.focus_handle(cx), cx);
        // E.g. a switcher launch's follow-up that landed while no main window was open.
        let status = shell::unseen_notice(cx);
        if status.is_some() {
            shell::notice_shown(cx);
        }
        let mut this = Self {
            tree,
            panes,
            details,
            status,
            open_seq: 0,
            fetch_seen: cx.try_global::<shell::Fetcher>().map_or(0, |f| f.finished),
            notices_seen: cx.try_global::<shell::Notices>().map_or(0, |n| n.posted),
            repo_focus: HashMap::new(),
            hovered_repo: None,
            repo_menu: None,
            repo_menu_item: cx.focus_handle(),
            repo_sort,
            visibility_button: cx.focus_handle(),
            visibility: None,
            repo_scroll: ScrollHandle::new(),
            shown_order: Vec::new(),
            removal: None,
            _subs: [
                collapse,
                open,
                tree_sort,
                repo_sort_chosen,
                details_open,
                notice,
                library,
                fetcher,
                notices,
                activation,
                tree_remove,
                details_remove,
                pane_saved,
            ],
        };
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(CLOCK_TICK).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();
        this.show_repo(selected_repo(cx).as_deref(), cx);
        if std::env::var_os("GROVE_DEV_OPEN").is_some() {
            this.dev_open(cx);
        }
        this
    }

    /// Dev: opens the first worktree once editors are detected, so the inline button and
    /// status bar can be checked without synthetic input.
    fn dev_open(&self, cx: &mut Context<Self>) {
        let tree = self.tree.clone();
        cx.spawn(async move |_, cx| {
            while !cx.update(|cx| launcher::ready(cx)) {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(50))
                    .await;
            }
            tree.update(cx, |tree, cx| tree.dev_open_first(cx));
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    mod badges_status;
    mod branches;
    mod common;
    mod details;
    mod launch;
    mod pane;
    mod pull;
    mod removal;
    mod sidebar;
    mod visibility;
}
