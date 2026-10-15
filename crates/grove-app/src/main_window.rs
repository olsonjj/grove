use std::collections::HashMap;
use std::path::{Path, PathBuf};

use gpui_kit::base::Button;
use gpui_kit::component::WindowExt;
use gpui_kit::component::input::{Escape, Input, InputEvent, InputState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, AnyWindowHandle, App, AppContext, Bounds, Context, Entity, FocusHandle, Focusable,
    FontWeight, InteractiveElement, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent,
    ParentElement, Pixels, Point, Render, ScrollHandle, SharedString, StatefulInteractiveElement,
    Styled, Subscription, TestSupportExt, TitlebarOptions, WeakEntity, Window, WindowBounds,
    WindowKind, WindowOptions, anchored, deferred, div, px, size,
};
use grove_core::{GitError, RemovalPlan, RepoSnapshot, Worktree, read_removal_plan, repo_order};

use crate::details_view::{self, DetailsView, LeaveMenu, LeaveMenuBack};
use crate::icons::{self, icon};
use crate::removal::{self, RemoveRequested};
use crate::sort_menu::{SortChosen, SortMenu, SortMenuIds};
use crate::tree_view::{CollapseChanged, OpenRequested, TreeView};
use crate::{launcher, shell, theme};

/// Key context of the open manage-visibility popover: Tab is left to move between the
/// filter field and the eye toggles, so a keyboard user can curate by stepping through rows.
const VISIBILITY_CONTEXT: &str = "VisibilityMenu";

/// Sidebar width; also the manage popover's, so it covers the list exactly. Fits
/// "REPOSITORIES · 99" plus the chip, the manage button and the sort menu.
const SIDEBAR_W: f32 = 330.;

const SHORTCUTS: &str = "↑↓ Move   ←→ Collapse / expand   ↵ Editor   ⌘↵ Terminal   ⌘K Jump to";
/// Keeps "Last fetch … ago" current.
const CLOCK_TICK: std::time::Duration = std::time::Duration::from_secs(30);

/// The status bar's confirmation or failure line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub text: String,
    pub error: bool,
    /// Fixable in settings: show the settings link.
    pub settings: bool,
}

struct MainWindow {
    tree: Entity<TreeView>,
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
    _subs: [Subscription; 12],
}

/// A worktree to remove, and its repo.
#[derive(Clone)]
struct Target {
    repo: PathBuf,
    git_dir: PathBuf,
    worktree: Worktree,
}

impl Target {
    async fn read_plan(self) -> RemovalPlan {
        read_removal_plan(&self.git_dir, &self.worktree)
    }
}

#[derive(Clone)]
struct Removal {
    target: Target,
    plan: RemovalPlan,
    confirm: removal::Confirm,
    running: bool,
    cancel: FocusHandle,
    ok: FocusHandle,
}

/// A sidebar row's context menu.
struct RepoMenu {
    repo: String,
    path: PathBuf,
    pinned: bool,
    at: Point<Pixels>,
}

/// The open manage-visibility popover; a fresh name filter per session, cleared on close.
struct VisibilityPopover {
    filter: Entity<InputState>,
    _changes: Subscription,
}

/// The saved repo if it still exists, else the first.
fn selected_repo(cx: &App) -> Option<String> {
    let library = shell::library(cx);
    let snapshot = library.store.snapshot();
    library
        .settings
        .last_selected_repo
        .as_deref()
        .and_then(|name| snapshot.repo(name))
        .or_else(|| {
            repo_order(
                snapshot,
                &library.settings.pinned_repos,
                &library.settings.hidden_repos,
                &library.settings.last_opened,
                library.settings.repo_sort,
            )
            .iter()
            .next()
        })
        .map(|r| r.name().to_string())
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

    /// Pushes fresher snapshot data for the shown repo into the tree, or shows another if
    /// it's gone (e.g. outside a new code folder).
    fn sync_repo(&mut self, cx: &mut Context<Self>) {
        if !cx.has_global::<shell::Library>() {
            return;
        }
        let mode = shell::library(cx).settings.repo_sort;
        self.repo_sort.update(cx, |m, cx| m.set_mode(mode, cx));
        self.sync_tree_order(cx);
        let name = self.tree.read(cx).repo_name().map(str::to_string);
        let snapshot = shell::library(cx).store.snapshot().clone();
        if name.as_deref().and_then(|n| snapshot.repo(n)).is_some() {
            self.tree
                .update(cx, |tree, cx| tree.update_snapshot(snapshot, cx));
        } else {
            self.show_repo(selected_repo(cx).as_deref(), cx);
        }
        cx.notify();
    }

    /// Shows each finished Fetch all's outcome once.
    fn fetch_changed(&mut self, cx: &mut Context<Self>) {
        let fetcher = cx.global::<shell::Fetcher>();
        if fetcher.finished == self.fetch_seen {
            return cx.notify();
        }
        self.fetch_seen = fetcher.finished;
        let Some(report) = &fetcher.report else {
            return cx.notify();
        };
        let (text, error) = (report.message(), !report.failures.is_empty());
        self.open_seq += 1;
        self.set_status(text, error, false, cx);
    }

    fn notice_posted(&mut self, cx: &mut Context<Self>) {
        let notices = cx.global::<shell::Notices>();
        if notices.posted == self.notices_seen {
            return;
        }
        self.notices_seen = notices.posted;
        if let Some(status) = notices.last.clone() {
            shell::notice_shown(cx);
            self.open_seq += 1;
            self.status = Some(status);
            cx.notify();
        }
    }

    /// The shown repo, so a reorder never moves the selection; else the saved one or the first.
    fn selected(&self, cx: &App) -> Option<String> {
        self.tree
            .read(cx)
            .repo_name()
            .filter(|n| shell::library(cx).store.snapshot().repo(n).is_some())
            .map(str::to_string)
            .or_else(|| selected_repo(cx))
    }

    fn sync_tree_order(&self, cx: &mut Context<Self>) {
        let settings = &shell::library(cx).settings;
        let (sort, last_opened) = (settings.tree_sort, settings.last_opened.clone());
        self.tree
            .update(cx, |tree, cx| tree.set_order(sort, last_opened, cx));
    }

    fn show_repo(&mut self, name: Option<&str>, cx: &mut Context<Self>) {
        self.sync_tree_order(cx);
        let library = shell::library(cx);
        let snapshot = library.store.snapshot().clone();
        let repo_name = name
            .filter(|n| snapshot.repo(n).is_some())
            .map(str::to_string);
        let collapsed = repo_name
            .as_deref()
            .map(|n| library.settings.collapsed(n).to_vec())
            .unwrap_or_default();
        self.tree.update(cx, |tree, cx| {
            tree.set_snapshot(Some(snapshot), repo_name, collapsed, cx)
        });
    }
}

impl MainWindow {
    fn set_status(&mut self, text: String, error: bool, settings: bool, cx: &mut Context<Self>) {
        self.status = Some(Status {
            text,
            error,
            settings,
        });
        cx.notify();
    }

    fn open(&mut self, req: &OpenRequested, cx: &mut Context<Self>) {
        self.open_seq += 1;
        let seq = self.open_seq;
        match launcher::open_worktree(&req.path, req.presence, req.role, req.target.as_ref(), cx) {
            Err(e) => self.set_status(e.message, true, e.settings, cx),
            Ok(opening) => {
                self.set_status(opening.message.clone(), false, false, cx);
                cx.spawn(async move |this, cx| {
                    let Some(n) = opening.finish().await else {
                        return;
                    };
                    let _ = this.update(cx, |this, cx| {
                        if this.open_seq == seq {
                            this.set_status(n.text, n.warn, false, cx);
                        }
                    });
                })
                .detach();
            }
        }
    }

    /// Remove worktree… from the tree or details pane: reads the worktree fresh, then
    /// confirms.
    fn request_removal(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        if self.removal.is_some() {
            return;
        }
        let snapshot = shell::library(cx).store.snapshot().clone();
        let Some(target) = snapshot.repos.iter().find_map(|r| {
            let wt = r.worktrees.iter().find(|w| w.path == path)?;
            Some(Target {
                repo: r.repo.path.clone(),
                git_dir: r.repo.git_dir.clone(),
                worktree: wt.clone(),
            })
        }) else {
            return;
        };
        cx.spawn_in(window, async move |this, cx| {
            let plan = cx.background_spawn(target.clone().read_plan()).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.confirm_removal(target, plan, window, cx)
            });
        })
        .detach();
    }

    fn refuse_removal(&mut self, target: &Target, why: &str, cx: &mut Context<Self>) {
        let path = tilde(&target.worktree.path);
        self.open_seq += 1;
        self.set_status(format!("Can't remove {path}: {why}"), true, false, cx);
    }

    fn confirm_removal(
        &mut self,
        target: Target,
        plan: RemovalPlan,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.removal.is_some() {
            return;
        }
        let Some(confirm) = removal::confirm(&target.worktree, &plan) else {
            if let RemovalPlan::NotAllowed(why) = &plan {
                self.refuse_removal(&target, why, cx);
            }
            return;
        };
        let cancel = cx.focus_handle();
        self.removal = Some(Removal {
            target,
            plan,
            confirm,
            running: false,
            cancel: cancel.clone(),
            ok: cx.focus_handle(),
        });
        let main = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, cx| {
            let Some(r) = main.upgrade().and_then(|m| m.read(cx).removal.clone()) else {
                return alert;
            };
            let (on_ok, on_cancel) = (main.clone(), main.clone());
            alert
                .title(r.confirm.title.clone())
                .description(
                    div()
                        .id("remove-message")
                        .aria_label(r.confirm.message.clone())
                        .child(r.confirm.message.clone())
                        .test_support(),
                )
                .footer(removal_footer(&r, main.clone()))
                // The dialog binds Enter to OK; make it press the focused button instead.
                .on_ok(move |_, window, cx| {
                    on_ok
                        .update(cx, |this, cx| {
                            let Some(r) = &this.removal else { return true };
                            if r.cancel.is_focused(window) {
                                this.cancel_removal(cx)
                            } else {
                                if r.ok.is_focused(window) {
                                    this.start_removal(window, cx);
                                }
                                false
                            }
                        })
                        .unwrap_or(true)
                })
                .on_cancel(move |_, _, cx| {
                    on_cancel
                        .update(cx, |this, cx| this.cancel_removal(cx))
                        .unwrap_or(true)
                })
        });
        window.focus(&cancel, cx);
        cx.notify();
    }

    /// False while removal runs: it can't be stopped.
    fn cancel_removal(&mut self, cx: &mut Context<Self>) -> bool {
        if self.removal.as_ref().is_some_and(|r| r.running) {
            return false;
        }
        self.removal = None;
        cx.notify();
        true
    }

    /// Rereads the worktree first: if it changed since the confirm opened, the confirm shows
    /// the new plan instead of removing more than it said.
    fn start_removal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(r) = self.removal.as_mut().filter(|r| !r.running) else {
            return;
        };
        r.running = true;
        let target = r.target.clone();
        window.refresh();
        cx.spawn_in(window, async move |this, cx| {
            let plan = cx.background_spawn(target.clone().read_plan()).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.removal_checked(plan, window, cx)
            });
        })
        .detach();
    }

    fn removal_checked(&mut self, plan: RemovalPlan, window: &mut Window, cx: &mut Context<Self>) {
        let Some(r) = self.removal.as_mut() else {
            return;
        };
        if r.plan == plan {
            let (target, force) = (r.target.clone(), plan.force());
            return self.run_removal(target, force, window, cx);
        }
        match removal::confirm(&r.target.worktree, &plan) {
            Some(confirm) => {
                r.plan = plan;
                r.confirm = confirm;
                r.running = false;
                window.focus(&r.cancel.clone(), cx);
                window.refresh();
            }
            None => {
                let target = r.target.clone();
                self.removal = None;
                window.close_dialog(cx);
                if let RemovalPlan::NotAllowed(why) = &plan {
                    self.refuse_removal(&target, why, cx);
                }
            }
        }
    }

    /// Bookkeeping runs at app level, so closing the window mid-removal doesn't lose it.
    fn run_removal(
        &mut self,
        target: Target,
        force: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let run = removal::runner(cx);
        let handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            let (repo, path) = (target.repo.clone(), target.worktree.path.clone());
            let result = cx
                .background_spawn(async move { run(&repo, &path, force) })
                .await;
            if result.is_ok() {
                cx.update(|cx| shell::worktree_removed(&target.git_dir, &target.worktree.path, cx));
            }
            let _ = handle.update(cx, |_, window, cx| {
                this.update(cx, |this, cx| {
                    this.finish_removal(&target, result, window, cx)
                })
            });
        })
        .detach();
    }

    fn finish_removal(
        &mut self,
        target: &Target,
        result: Result<(), GitError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.removal.take().is_some() {
            window.close_dialog(cx);
        }
        let path = tilde(&target.worktree.path);
        let (text, error) = match result {
            Ok(()) => (format!("Removed {path}"), false),
            Err(e) => (format!("Couldn't remove {path}: {}", e.brief()), true),
        };
        self.open_seq += 1;
        self.set_status(text, error, false, cx);
    }

    fn top_bar(&self, repo: Option<&RepoSnapshot>, cx: &mut Context<Self>) -> AnyElement {
        let fetching = cx.try_global::<shell::Fetcher>().is_some_and(|f| f.running);
        // Stays focusable while fetching (`fetch_all` no-ops), so focus isn't lost.
        let fetch = top_button("fetch-all")
            .text_size(px(13.))
            .on_click(|_, _, cx| shell::fetch_all(cx))
            .when_else(
                fetching,
                |d| d.text_color(theme::faint()).child("Fetching…"),
                |d| {
                    d.text_color(theme::text())
                        .cursor_pointer()
                        .hover(|s| s.bg(theme::border()))
                        .child("Fetch all")
                },
            );
        let jump = top_button("jump-to")
            .gap(px(6.))
            .text_size(px(13.))
            .text_color(theme::text_soft())
            .cursor_pointer()
            .hover(|s| s.bg(theme::border()))
            .accessibility_label("Jump to…")
            .aria_keyshortcuts("Command+K")
            .on_click(|_, _, cx| shell::toggle_switcher(cx))
            .child("Jump to…")
            .child(
                div()
                    .px(px(5.))
                    .rounded(px(4.))
                    .border_1()
                    .border_color(theme::border_strong())
                    .font_family(theme::MONO)
                    .text_size(px(11.))
                    .text_color(theme::muted())
                    .child("⌘K"),
            );
        div()
            .flex_none()
            .h(px(52.))
            .px(px(16.))
            .flex()
            .items_center()
            .gap(px(10.))
            .border_b_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .child(
                div()
                    .text_size(px(16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Grove"),
            )
            .when_some(repo, |d, repo| {
                d.child(div().text_color(theme::faint()).child("/")).child(
                    theme::mono(repo.display_name.clone())
                        .min_w_0()
                        .truncate()
                        .text_size(px(13.)),
                )
            })
            .child(jump)
            .child(div().ml_auto().flex_none().child(fetch))
            .child(
                top_button("open-settings")
                    .accessibility_label("Open-with settings")
                    .size(px(32.))
                    .px_0()
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::border()))
                    .child(icon(icons::SETTINGS, 16.).text_color(theme::text()))
                    .tooltip(|window, cx| Tooltip::new("Open-with settings").build(window, cx))
                    .on_click(|_, _, cx| shell::open_settings(cx)),
            )
            .into_any_element()
    }

    fn status_bar(&self, cx: &App) -> impl IntoElement {
        let line = match &self.status {
            Some(s) => div()
                .flex()
                .min_w_0()
                .gap(px(10.))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_color(if s.error {
                            theme::warn_text()
                        } else {
                            theme::text_soft()
                        })
                        .child(s.text.clone()),
                )
                .when(s.settings, |d| {
                    d.child(link("status-settings", "Settings…", shell::open_settings))
                }),
            None => div(),
        };
        div()
            .id("status-bar")
            .flex_none()
            .h(px(28.))
            .px(px(14.))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(16.))
            .border_t_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .text_size(px(12.))
            .child(
                div()
                    .flex()
                    .min_w_0()
                    .gap(px(16.))
                    .child(
                        div()
                            .flex_none()
                            .text_color(theme::muted())
                            .child(fetch_label(cx)),
                    )
                    .child(line),
            )
            .child(
                div()
                    .flex_none()
                    .text_color(theme::faint())
                    .child(SHORTCUTS),
            )
    }
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let library = shell::library(cx);
        let snapshot = library.store.snapshot().clone();
        let settings = &library.settings;
        let sort = settings.repo_sort;
        let order = repo_order(
            &snapshot,
            &settings.pinned_repos,
            &settings.hidden_repos,
            &settings.last_opened,
            sort,
        );
        // The header counts what the list shows; hidden repos come back as the chip.
        let shown = order.iter().count();
        let hidden_count = snapshot.repos.len() - shown;
        let visibility_menu = if self.visibility.is_some() {
            Some(self.visibility_popover(cx))
        } else {
            None
        };
        let selected = self.selected(cx);
        let top_bar = self.top_bar(selected.as_deref().and_then(|n| snapshot.repo(n)), cx);

        self.repo_focus
            .retain(|name, _| order.iter().any(|r| r.name() == name));
        let mut pinned_rows: Vec<AnyElement> = Vec::new();
        if !order.pinned.is_empty() {
            pinned_rows.push(
                group_header("pinned-header", format!("PINNED · {}", order.pinned.len()))
                    .px(px(8.))
                    .pb(px(6.))
                    .into_any_element(),
            );
        }
        for (ix, repo) in order.pinned.iter().enumerate() {
            let is_selected = selected.as_deref() == Some(repo.name());
            pinned_rows.push(self.repo_row(ix, repo, true, is_selected, window, cx));
        }
        if !order.pinned.is_empty() && !order.others.is_empty() {
            pinned_rows.push(
                div()
                    .flex_none()
                    .h(px(1.))
                    .mx(px(8.))
                    .my(px(6.))
                    .bg(theme::border())
                    .into_any_element(),
            );
        }
        let mut rows: Vec<AnyElement> = Vec::new();
        let mut selected_row = None;
        for (i, repo) in order.others.iter().enumerate() {
            let is_selected = selected.as_deref() == Some(repo.name());
            let ix = order.pinned.len() + i;
            if is_selected {
                selected_row = Some(rows.len());
            }
            rows.push(self.repo_row(ix, repo, false, is_selected, window, cx));
        }
        let names: Vec<String> = order.iter().map(|r| r.name().to_string()).collect();
        if names != self.shown_order {
            if let Some(row) = selected_row {
                self.repo_scroll.scroll_to_item(row);
            }
            self.shown_order = names;
        }
        let repo_menu = self.repo_menu(cx);

        let sidebar = div()
            .w(px(SIDEBAR_W))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .bg(theme::sidebar())
            .border_r_1()
            .border_color(theme::border())
            .child(
                div()
                    .flex_none()
                    .relative()
                    .pl(px(18.))
                    .pr(px(12.))
                    .pt(px(10.))
                    .pb(px(4.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(px(5.))
                            .child(group_header(
                                "repos-header",
                                format!("REPOSITORIES · {shown}"),
                            ))
                            .when(hidden_count > 0, |d| {
                                d.child(self.hidden_chip(hidden_count, cx))
                            }),
                    )
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .child(self.manage_button(cx))
                            .child(self.repo_sort.clone()),
                    )
                    .children(visibility_menu),
            )
            .child(
                div()
                    .on_key_down(cx.listener(Self::on_repo_key))
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    // Pins stay put; only the rest scrolls.
                    .child(
                        div()
                            .id("pinned-list")
                            .flex_none()
                            .px(px(10.))
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .children(pinned_rows)
                            .test_support(),
                    )
                    .child(
                        div()
                            .id("repo-list")
                            .track_scroll(&self.repo_scroll)
                            .flex_1()
                            .overflow_y_scroll()
                            .px(px(10.))
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .children(rows)
                            .test_support(),
                    ),
            )
            .child(
                div()
                    .m(px(10.))
                    .p(px(10.))
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::bg())
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(theme::faint())
                            .child("CODE FOLDER"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(8.))
                            .child(
                                theme::path_text(tilde(&snapshot.code_dir))
                                    .text_size(px(13.))
                                    .text_color(theme::text()),
                            )
                            .child(
                                link("change-code-folder", "Change…", shell::choose_code_dir)
                                    .text_size(px(12.)),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(theme::faint())
                            .child("Grove only looks inside this folder."),
                    ),
            );

        let content = if snapshot.repos.is_empty() {
            div()
                .p(px(16.))
                .text_color(theme::muted())
                .child("No repos")
                .into_any_element()
        } else {
            div()
                .flex_1()
                .min_w_0()
                .h_full()
                .flex()
                .child(self.tree.clone())
                .child(self.details.clone())
                .into_any_element()
        };

        div()
            .on_key_down(cx.listener(|_, ev: &KeyDownEvent, _, cx| {
                let ks = &ev.keystroke;
                if ks.key == "k" && ks.modifiers == gpui_kit::Modifiers::command() {
                    shell::toggle_switcher(cx);
                    cx.stop_propagation();
                }
            }))
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::bg())
            .text_color(theme::text())
            .child(top_bar)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(sidebar)
                    .child(content),
            )
            .child(self.status_bar(cx))
            .children(repo_menu)
    }
}

impl MainWindow {
    fn select_repo(&mut self, name: &str, cx: &mut Context<Self>) {
        shell::select_repo(name, cx);
        if self.tree.read(cx).repo_name() != Some(name) {
            self.show_repo(Some(name), cx);
        }
        cx.notify();
    }

    /// ↑/↓ move the selection through both groups, skipping headers.
    fn on_repo_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        let step: isize = match ks.key.as_str() {
            "up" if !ks.modifiers.modified() => -1,
            "down" if !ks.modifiers.modified() => 1,
            _ => return,
        };
        let library = shell::library(cx);
        let names: Vec<String> = repo_order(
            library.store.snapshot(),
            &library.settings.pinned_repos,
            &library.settings.hidden_repos,
            &library.settings.last_opened,
            library.settings.repo_sort,
        )
        .iter()
        .map(|r| r.name().to_string())
        .collect();
        let focused = |n: &String| {
            self.repo_focus
                .get(n)
                .is_some_and(|[row, _]| row.is_focused(window))
        };
        let Some(at) = names.iter().position(focused) else {
            return;
        };
        let next = &names[at.saturating_add_signed(step).min(names.len() - 1)];
        self.select_repo(next, cx);
        if let Some([row, _]) = self.repo_focus.get(next) {
            window.focus(row, cx);
        }
        cx.stop_propagation();
    }

    /// The row under the pointer changes, so hover is re-read from the next layout.
    fn toggle_pin(&mut self, path: &Path, cx: &mut Context<Self>) {
        // Saved first, or an unsaved (fallback) selection would follow the new first row. A
        // saved one naming an absent repo is kept for when it returns.
        if shell::library(cx).settings.last_selected_repo.is_none()
            && let Some(name) = self.selected(cx)
        {
            shell::select_repo(&name, cx);
        }
        shell::toggle_pin(path, cx);
        self.hovered_repo = None;
        cx.notify();
    }

    fn close_repo_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(menu) = self.repo_menu.take() {
            if let Some([row, _]) = self.repo_focus.get(&menu.repo) {
                window.focus(row, cx);
            }
            cx.notify();
        }
    }

    /// Manage visibility: a popover of every repo, each with a show/hide eye. Hiding is a
    /// header action, next to the sort menu.
    fn open_visibility(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.visibility.is_some() {
            return;
        }
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter repositories…"));
        let changes = cx.subscribe(&filter, |_, _, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Change) {
                cx.notify();
            }
        });
        filter.update(cx, |input, cx| input.focus(window, cx));
        self.visibility = Some(VisibilityPopover {
            filter,
            _changes: changes,
        });
        cx.notify();
    }

    fn close_visibility(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.visibility.take().is_some() {
            window.focus(&self.visibility_button, cx);
            cx.notify();
        }
    }

    fn manage_button(&self, cx: &mut Context<Self>) -> Button {
        Button::new("manage-visibility")
            .track_focus(&self.visibility_button)
            .accessibility_label("Manage repository visibility")
            .aria_expanded(self.visibility.is_some())
            .flex_none()
            .h(px(22.))
            .px(px(6.))
            .rounded(px(5.))
            .border_1()
            .border_color(gpui_kit::transparent_black())
            .focus_visible(|s| s.border_color(theme::link()))
            .cursor_pointer()
            .hover(|s| s.bg(theme::border()))
            .tooltip(|window, cx| Tooltip::new("Manage visibility").build(window, cx))
            .child(icon(icons::EYE, 13.).text_color(theme::muted()))
            .on_click(cx.listener(|this, _, window, cx| {
                if this.visibility.is_some() {
                    this.close_visibility(window, cx);
                } else {
                    this.open_visibility(window, cx);
                }
            }))
    }

    fn hidden_chip(&self, hidden: usize, cx: &mut Context<Self>) -> Button {
        Button::new("hidden-chip")
            .accessibility_label(format!("{hidden} hidden"))
            .flex_none()
            .h(px(20.))
            .px(px(4.))
            .rounded(px(5.))
            .cursor_pointer()
            .hover(|s| s.bg(theme::border()))
            .text_size(px(11.))
            .text_color(theme::faint())
            .child(format!("· {hidden} hidden"))
            .on_click(cx.listener(move |this, _, window, cx| this.open_visibility(window, cx)))
    }

    /// One popover row: the repo name, dimmed and struck through when hidden, with an eye
    /// toggle. `ix` indexes the popover's full (unfiltered) list, so a row keeps its id
    /// while its neighbours are filtered out.
    fn visibility_row(
        &self,
        ix: usize,
        name: &str,
        path: &Path,
        hidden: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let eye = Button::new(("visibility-eye", ix))
            .accessibility_label(format!("{} {name}", if hidden { "Show" } else { "Hide" }))
            .flex_none()
            .size(px(22.))
            .rounded(px(5.))
            .cursor_pointer()
            .hover(|s| s.bg(theme::border()))
            .child(
                icon(if hidden { icons::EYE_OFF } else { icons::EYE }, 13.)
                    .text_color(theme::muted()),
            )
            .on_click(cx.listener({
                let path = path.to_path_buf();
                move |_, _, _, cx| {
                    shell::toggle_hidden(&path, cx);
                    cx.notify();
                }
            }));
        div()
            .id(("visibility-row", ix))
            .test_support()
            .aria_label(format!(
                "{name}, {}",
                if hidden { "hidden" } else { "shown" }
            ))
            .flex_none()
            .h(px(26.))
            .px(px(6.))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(8.))
            .rounded(px(5.))
            .text_size(px(13.))
            .child(
                theme::mono(name)
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(if hidden {
                        theme::faint()
                    } else {
                        theme::text_soft()
                    })
                    .when(hidden, |d| d.line_through()),
            )
            .child(eye)
            .into_any_element()
    }

    /// The manage popover, dropping from the header buttons, listing every repo in sidebar
    /// order (pinned first, then the sort mode), including hidden ones.
    fn visibility_popover(&self, cx: &mut Context<Self>) -> AnyElement {
        let (matched, filter) = {
            let library = shell::library(cx);
            let settings = &library.settings;
            let query = self.visibility.as_ref().map_or(String::new(), |v| {
                v.filter.read(cx).value().trim().to_lowercase()
            });
            // The popover's list is the sidebar's order with hidden repos shown: an empty
            // hidden set, since hiding filters before the sort.
            let matches = |repo: &RepoSnapshot| {
                query.is_empty()
                    || repo.display_name.to_lowercase().contains(&query)
                    || repo.name().to_lowercase().contains(&query)
            };
            let matched = repo_order(
                library.store.snapshot(),
                &settings.pinned_repos,
                &[],
                &settings.last_opened,
                settings.repo_sort,
            )
            .iter()
            .enumerate()
            .filter(|(_, repo)| matches(repo))
            .map(|(ix, repo)| {
                (
                    ix,
                    repo.display_name.clone(),
                    repo.repo.path.clone(),
                    settings.is_hidden(&repo.repo.path),
                )
            })
            .collect::<Vec<_>>();
            let filter = self.visibility.as_ref().map(|v| v.filter.clone());
            (matched, filter)
        };
        let rows = matched
            .into_iter()
            .map(|(ix, name, path, hidden)| self.visibility_row(ix, &name, &path, hidden, cx))
            .collect::<Vec<_>>();
        deferred(
            div()
                .id("visibility-menu")
                // Left-aligned with the sidebar and dropping from the header buttons, so
                // repo names get the full sidebar width and never clip at the window edge.
                .absolute()
                .top(px(36.))
                .left(px(0.))
                .w(px(SIDEBAR_W))
                .key_context(VISIBILITY_CONTEXT)
                .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                    if ev.keystroke.key == "escape" {
                        this.close_visibility(window, cx);
                        cx.stop_propagation();
                    }
                }))
                .capture_action(cx.listener(|this, _: &Escape, window, cx| {
                    this.close_visibility(window, cx);
                    cx.stop_propagation();
                }))
                // Outside click closes and is swallowed, like a native menu.
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    if this.visibility.take().is_some() {
                        cx.stop_propagation();
                        cx.notify();
                    }
                }))
                .max_h(px(360.))
                .overflow_y_scroll()
                .p(px(4.))
                .flex()
                .flex_col()
                .gap(px(2.))
                .rounded(px(8.))
                .border_1()
                .border_color(theme::border_strong())
                .bg(theme::raised())
                .shadow_lg()
                .occlude()
                .text_size(px(13.))
                .child(
                    div()
                        .id("visibility-filter")
                        .h(px(26.))
                        .px(px(6.))
                        .mb(px(2.))
                        .rounded(px(5.))
                        .border_1()
                        .border_color(theme::border())
                        .bg(theme::bg())
                        // Column + centering: the input's own height is its line height, so
                        // this keeps the text vertically centered in the field.
                        .flex()
                        .items_center()
                        .when_some(filter, |d, filter| {
                            d.child(
                                Input::new(&filter)
                                    .appearance(false)
                                    .px_0()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(12.)),
                            )
                        })
                        .test_support(),
                )
                .children(rows)
                .test_support(),
        )
        .with_priority(1)
        .into_any_element()
    }

    fn repo_row(
        &mut self,
        ix: usize,
        repo: &RepoSnapshot,
        pinned: bool,
        is_selected: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let name = repo.name().to_string();
        let path = repo.repo.path.clone();
        let count = repo.worktree_count();
        let a11y = format!(
            "{}, {count} worktree{}{}{}",
            repo.display_name,
            if count == 1 { "" } else { "s" },
            if pinned { ", pinned" } else { "" },
            if is_selected { ", selected" } else { "" }
        );
        let [focus, pin_focus] = self
            .repo_focus
            .entry(name.clone())
            .or_insert_with(|| [cx.focus_handle(), cx.focus_handle()])
            .clone();
        // Also while focused, so unpinning from the keyboard doesn't drop focus.
        let show_pin = pinned
            || is_selected
            || self.hovered_repo.as_deref() == Some(&name)
            || pin_focus.is_focused(window);
        let button = Button::new(("repo", ix))
            .track_focus(&focus)
            .accessibility_label(a11y)
            .selected(is_selected)
            .flex_1()
            .min_w_0()
            .h_full()
            .pl(px(10.))
            .pr(px(4.))
            .justify_between()
            .gap(px(8.))
            .rounded(px(6.))
            .border_1()
            .border_color(gpui_kit::transparent_black())
            .focus_visible(|s| s.border_color(theme::link()))
            .cursor_pointer()
            .on_click(cx.listener({
                let name = name.clone();
                move |this, _, window, cx| {
                    this.select_repo(&name, cx);
                    window.focus(&this.tree.focus_handle(cx), cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener({
                    let (name, path) = (name.clone(), path.clone());
                    move |this, ev: &MouseDownEvent, window, cx| {
                        this.repo_menu = Some(RepoMenu {
                            repo: name.clone(),
                            path: path.clone(),
                            pinned,
                            at: ev.position,
                        });
                        window.focus(&this.repo_menu_item, cx);
                        cx.stop_propagation();
                        cx.notify();
                    }
                }),
            )
            .child(theme::mono(repo.display_name.clone()).truncate())
            .child(
                div()
                    .flex_none()
                    .text_size(px(11.))
                    .text_color(theme::link())
                    .child(format!("{count} wt")),
            );
        let pin_slot = div().flex_none().size(px(24.)).mr(px(6.));
        div()
            // By name: a row moved by pinning gets fresh hover state.
            .id(SharedString::from(format!("repo-row-{name}")))
            .flex_none()
            .h(px(34.))
            .flex()
            .items_center()
            .rounded(px(6.))
            .text_size(px(13.))
            .relative()
            .on_hover(cx.listener({
                let name = name.clone();
                move |this, hovered: &bool, _, cx| {
                    if *hovered {
                        this.hovered_repo = Some(name.clone());
                    } else if this.hovered_repo.as_deref() == Some(&name) {
                        this.hovered_repo = None;
                    }
                    cx.notify();
                }
            }))
            .when_else(
                is_selected,
                |d| {
                    d.bg(theme::selected())
                        .text_color(theme::text())
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(theme::selection_bar())
                },
                |d| {
                    d.text_color(theme::text_soft())
                        .hover(|s| s.bg(theme::raised()))
                },
            )
            .child(button)
            .child(if show_pin {
                pin_button(ix, &repo.display_name, pinned)
                    .track_focus(&pin_focus)
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_pin(&path, cx)))
                    .into_any_element()
            } else {
                pin_slot.into_any_element()
            })
            .into_any_element()
    }

    fn repo_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.repo_menu.as_ref()?;
        let (label, path) = (if menu.pinned { "Unpin" } else { "Pin" }, menu.path.clone());
        let item = Button::new("repo-menu-pin")
            .track_focus(&self.repo_menu_item)
            .tab_stop(false)
            .role(gpui_kit::Role::MenuItem)
            .accessibility_label(label)
            .h(px(28.))
            .px(px(10.))
            .justify_start()
            .rounded(px(5.))
            .cursor_pointer()
            .text_color(theme::text())
            .hover(|s| s.bg(theme::selected()))
            .focus(|s| s.bg(theme::selected()))
            .child(label)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.toggle_pin(&path, cx);
                this.close_repo_menu(window, cx);
            }));
        Some(
            deferred(
                anchored().position(menu.at).snap_to_window().child(
                    div()
                        .id("repo-menu")
                        .role(gpui_kit::Role::Menu)
                        .key_context(details_view::MENU_CONTEXT)
                        .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                            if ev.keystroke.key == "escape" {
                                this.close_repo_menu(window, cx);
                                cx.stop_propagation();
                            }
                        }))
                        .on_action(cx.listener(|this, _: &LeaveMenu, window, cx| {
                            this.close_repo_menu(window, cx);
                            window.focus_next(cx);
                        }))
                        .on_action(cx.listener(|this, _: &LeaveMenuBack, window, cx| {
                            this.close_repo_menu(window, cx);
                            window.focus_prev(cx);
                        }))
                        // Outside click closes the menu and is swallowed, like a native menu.
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                            this.repo_menu = None;
                            cx.stop_propagation();
                            cx.notify();
                        }))
                        .min_w(px(140.))
                        .p(px(4.))
                        .flex()
                        .flex_col()
                        .rounded(px(8.))
                        .border_1()
                        .border_color(theme::border_strong())
                        .bg(theme::raised())
                        .shadow_lg()
                        .occlude()
                        .text_size(px(13.))
                        .child(item)
                        .test_support(),
                ),
            )
            .with_priority(1)
            .into_any_element(),
        )
    }
}

/// Cancel (focused first) and the confirm, which shows progress while removal runs.
fn removal_footer(r: &Removal, main: WeakEntity<MainWindow>) -> impl IntoElement {
    let cancel_main = main.clone();
    let (bg, fg, hover) = if r.confirm.destructive {
        (theme::warn_text(), theme::bg(), theme::warn_text())
    } else {
        (theme::accent(), theme::on_accent(), theme::accent_hover())
    };
    let button = |id: &'static str| {
        Button::new(id)
            .disabled(r.running)
            .h(px(30.))
            .px(px(14.))
            .rounded(px(7.))
            .border_1()
            .border_color(theme::border_strong())
            .focus_visible(|s| s.border_color(theme::link()))
            .text_size(px(13.))
            .when(!r.running, |b| b.cursor_pointer())
    };
    div()
        .flex()
        .justify_end()
        .gap(px(8.))
        .px(px(16.))
        .child(
            button("remove-cancel")
                .track_focus(&r.cancel)
                .accessibility_label("Cancel")
                .bg(theme::raised())
                .text_color(theme::text())
                .hover(|s| s.bg(theme::border()))
                .child("Cancel")
                .on_click(move |_, window, cx| {
                    if matches!(
                        cancel_main.update(cx, |this, cx| this.cancel_removal(cx)),
                        Ok(true)
                    ) {
                        window.close_dialog(cx);
                    }
                }),
        )
        .child(
            button("remove-confirm")
                .track_focus(&r.ok)
                .accessibility_label(if r.running {
                    "Removing…"
                } else {
                    r.confirm.ok
                })
                .bg(bg)
                .text_color(fg)
                .font_weight(FontWeight::SEMIBOLD)
                .hover(move |s| s.bg(hover))
                .child(if r.running {
                    "Removing…"
                } else {
                    r.confirm.ok
                })
                .on_click(move |_, window, cx| {
                    let _ = main.update(cx, |this, cx| this.start_removal(window, cx));
                }),
        )
}

/// "PINNED · 2", "REPOSITORIES · 40".
fn group_header(id: &'static str, text: String) -> impl Styled + IntoElement {
    div()
        .id(id)
        .test_support()
        .aria_label(text.clone())
        .flex_none()
        .text_size(px(11.))
        .text_color(theme::faint())
        .child(text)
}

/// Shown on hover and selection, and always (filled) on pinned rows.
fn pin_button(ix: usize, repo: &str, pinned: bool) -> Button {
    let label = if pinned { "Unpin" } else { "Pin" };
    Button::new(("repo-pin", ix))
        .accessibility_label(format!("{label} {repo}"))
        .flex_none()
        .size(px(24.))
        .mr(px(6.))
        .rounded(px(6.))
        .border_1()
        .border_color(gpui_kit::transparent_black())
        .focus_visible(|s| s.border_color(theme::link()))
        .cursor_pointer()
        .hover(|s| s.bg(theme::border()))
        .child(
            icon(
                if pinned {
                    icons::PIN_FILLED
                } else {
                    icons::PIN
                },
                13.,
            )
            .text_color(if pinned {
                theme::link()
            } else {
                theme::muted()
            }),
        )
        .tooltip(move |window, cx| Tooltip::new(label).build(window, cx))
}

/// Bordered top bar button.
fn top_button(id: &'static str) -> Button {
    Button::new(id)
        .flex_none()
        .h(px(32.))
        .px(px(12.))
        .rounded(px(8.))
        .border_1()
        .border_color(theme::border_strong())
        .focus_visible(|s| s.border_color(theme::link()))
        .bg(theme::raised())
}

/// Inline text link.
fn link(id: &'static str, text: &'static str, action: fn(&mut App)) -> Button {
    Button::new(id)
        .flex_none()
        .px(px(2.))
        .rounded(px(4.))
        .border_1()
        .border_color(gpui_kit::transparent_black())
        .focus_visible(|s| s.border_color(theme::link()))
        .text_color(theme::link())
        .cursor_pointer()
        .hover(|s| s.underline())
        .child(text)
        .on_click(move |_, _, cx| action(cx))
}

/// "Last fetch 3 minutes ago" or "Not fetched yet". The Fetch all button shows progress.
fn fetch_label(cx: &App) -> String {
    match shell::library(cx).store.snapshot().last_fetch {
        Some(at) => format!(
            "Last fetch {}",
            grove_core::relative_time(at, grove_core::unix_now())
        ),
        None => "Not fetched yet".into(),
    }
}

/// `/Users/me/code` → `~/code`.
pub fn tilde(path: &Path) -> String {
    grove_core::tilde(path, grove_core::home_dir().as_deref())
}

const MIN_SIZE: gpui_kit::Size<gpui_kit::Pixels> = size(px(1000.), px(560.));

pub fn open(cx: &mut App) -> Option<AnyWindowHandle> {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(1180.), px(720.)),
            cx,
        ))),
        window_min_size: Some(MIN_SIZE),
        titlebar: Some(TitlebarOptions {
            title: Some("Grove".into()),
            ..Default::default()
        }),
        focus: true,
        show: true,
        kind: WindowKind::Normal,
        ..Default::default()
    };
    open_with(options, cx)
        .map(|(handle, _)| handle)
        .inspect_err(|e| eprintln!("[grove] main: {e:#}"))
        .ok()
}

fn open_with(
    options: WindowOptions,
    cx: &mut App,
) -> gpui_kit::Result<(AnyWindowHandle, Entity<MainWindow>)> {
    gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| MainWindow::new(window, cx))
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{Point, TestAppContext};
    use grove_core::{
        Change, ChangedFile, Fallback, LaunchError, Outcome, Permission, Presence, RowId, RowKind,
        Settings, SettingsFile, Snapshot, SnapshotStore, SortMode, TerminalOpensIn, Upstream,
        WorktreeStatus,
    };
    use tempfile::TempDir;

    use super::*;
    use crate::pull;
    use crate::shell::Library;
    use crate::test_support::{
        Launches, canned, code, fake_launcher, fake_presence, indexed_labels, set_library,
        settings_file, tab_to, terminal_window,
    };

    fn launch(
        cx: &mut TestAppContext,
        file: &SettingsFile,
    ) -> (AnyWindowHandle, Entity<MainWindow>) {
        launch_repo(cx, file, canned())
    }

    fn launch_repo(
        cx: &mut TestAppContext,
        file: &SettingsFile,
        repo: grove_core::RepoSnapshot,
    ) -> (AnyWindowHandle, Entity<MainWindow>) {
        launch_at(cx, file, repo, size(px(1180.), px(720.)))
    }

    fn launch_at(
        cx: &mut TestAppContext,
        file: &SettingsFile,
        repo: grove_core::RepoSnapshot,
        window: gpui_kit::Size<gpui_kit::Pixels>,
    ) -> (AnyWindowHandle, Entity<MainWindow>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::details_view::init(cx);
            set_library(file, vec![repo], cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: window,
                })),
                ..Default::default()
            };
            open_with(options, cx).unwrap()
        })
    }

    fn folder_collapsed(cx: &mut TestAppContext, main: &Entity<MainWindow>, prefix: &str) -> bool {
        main.read_with(cx, |m, cx| {
            m.tree.read(cx).tree().rows.iter().any(|r| {
                r.id == RowId::Folder(prefix.into())
                    && matches!(
                        r.kind,
                        RowKind::Folder {
                            collapsed: true,
                            ..
                        }
                    )
            })
        })
    }

    #[gpui_kit::test]
    fn collapse_by_key_is_saved_and_restored(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = SettingsFile::new(tmp.path().join("settings.json"));
        file.save(&Settings {
            last_selected_repo: Some("api".into()),
            ..Default::default()
        })
        .unwrap();

        let (w, main) = launch(cx, &file);
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            // main → main worktree → feature/, then collapse it.
            for k in ["down", "down", "left"] {
                window.press(k, cx);
            }
        })
        .unwrap();
        cx.run_until_parked();
        assert!(folder_collapsed(cx, &main, "feature/"));
        assert_eq!(file.load().unwrap().collapsed("api"), ["feature/"]);

        cx.update_window(w, |_, window, cx| {
            window.press("right", cx);
            window.press("left", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(file.load().unwrap().collapsed("api"), ["feature/"]);
        cx.update_window(w, |_, window, cx| window.press("right", cx))
            .unwrap();
        cx.run_until_parked();
        assert!(file.load().unwrap().collapsed("api").is_empty());
        cx.update_window(w, |_, window, cx| window.press("left", cx))
            .unwrap();
        cx.run_until_parked();
        cx.update(|cx| cx.remove_global::<Library>());

        // Fresh load from the same file restores it.
        let (_, main) = launch(cx, &file);
        assert!(folder_collapsed(cx, &main, "feature/"));
        assert!(!folder_collapsed(cx, &main, "fix/"));
    }

    fn keys(cx: &mut TestAppContext, w: AnyWindowHandle, keys: &[&str]) {
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            for k in keys {
                window.press(k, cx);
            }
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn has(cx: &mut TestAppContext, w: AnyWindowHandle, id: &'static str, ix: usize) -> bool {
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            window.try_find((id, ix)).is_some()
        })
        .unwrap()
    }

    fn status(cx: &mut TestAppContext, main: &Entity<MainWindow>) -> Option<Status> {
        main.read_with(cx, |m, _| m.status.clone())
    }

    #[gpui_kit::test]
    fn enter_opens_worktree_and_branch_rows_in_default_editor(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        let (w, main) = launch(cx, &file);

        // Branch row `main` opens its worktree.
        keys(cx, w, &["enter"]);
        assert_eq!(*launches.lock().unwrap(), [code("/c/api")]);
        let s = status(cx, &main).unwrap();
        assert_eq!(s.text, "Opening /c/api in VS Code…");
        assert!(!s.error);

        // main → wt → feature/ → feature/x → its worktree.
        keys(cx, w, &["down", "down", "down", "enter", "down", "enter"]);
        assert_eq!(
            launches.lock().unwrap()[1..],
            [code("/c/wt/api-x"), code("/c/wt/api-x")]
        );
        assert_eq!(
            status(cx, &main).unwrap().text,
            "Opening /c/wt/api-x in VS Code…"
        );

        // A folder row opens nothing; every open is a new launch, never deduped.
        keys(cx, w, &["up", "up", "enter"]);
        assert_eq!(launches.lock().unwrap().len(), 3);
    }

    #[gpui_kit::test]
    fn configured_editor_is_used(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(
            &tmp,
            Settings {
                default_editor: Some("zed".into()),
                ..Default::default()
            },
        );
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        let (w, main) = launch(cx, &file);
        keys(cx, w, &["enter"]);
        assert_eq!(
            *launches.lock().unwrap(),
            [vec!["/fake/bin/zed", "--new", "/c/api"]]
        );
        assert_eq!(status(cx, &main).unwrap().text, "Opening /c/api in Zed…");
    }

    #[gpui_kit::test]
    fn launch_failure_shows_reason(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let fail = LaunchError::Exit {
            program: "code".into(),
            status: "exit status: 1".into(),
            stderr: "bad option".into(),
        };
        let launches = fake_launcher(cx, Err(fail));
        let (w, main) = launch(cx, &file);
        keys(cx, w, &["enter"]);
        assert_eq!(launches.lock().unwrap().len(), 1);
        let s = status(cx, &main).unwrap();
        assert_eq!(s.text, "code failed: bad option");
        assert!(s.error);
    }

    fn with_commands(tmp: &TempDir, default_editor: &str) -> SettingsFile {
        let cmd = |label: &str, template: &str, run_in_terminal| grove_core::CustomCommand {
            label: label.into(),
            template: template.into(),
            run_in_terminal,
        };
        settings_file(
            tmp,
            Settings {
                default_editor: Some(default_editor.into()),
                custom_commands: vec![
                    cmd("Neovim", "nvim {path}", true),
                    cmd("Sublime", "subl -n {path}", false),
                ],
                ..Default::default()
            },
        )
    }

    #[gpui_kit::test]
    fn custom_commands_open_from_enter_and_the_menu(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = with_commands(&tmp, "custom:Sublime");
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        let (w, main) = launch(cx, &file);

        keys(cx, w, &["enter"]);
        assert_eq!(
            *launches.lock().unwrap(),
            [vec!["/bin/sh", "-c", "subl -n '/c/api'"]]
        );
        assert_eq!(
            status(cx, &main).unwrap().text,
            "Opening /c/api in Sublime…"
        );

        // Detected editors first, then the commands.
        click(cx, w, "details-menu-editor");
        assert!(has(cx, w, "editor-choice", 3));
        click(cx, w, ("editor-choice", 2usize));
        let launch = launches.lock().unwrap()[1].clone();
        assert_eq!(launch[0], "osascript");
        assert!(
            launch[1].contains(r#"do script "cd '/c/api' && nvim '/c/api'""#),
            "{}",
            launch[1]
        );
        assert_eq!(status(cx, &main).unwrap().text, "Opening /c/api in Neovim…");
    }

    #[gpui_kit::test]
    fn custom_command_failures_show_in_status_line(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = with_commands(&tmp, "custom:Sublime");
        let fail = LaunchError::Exit {
            program: "sh".into(),
            status: "exit status: 127".into(),
            stderr: "sh: subl: command not found".into(),
        };
        fake_launcher(cx, Err(fail));
        let (w, main) = launch(cx, &file);
        keys(cx, w, &["enter"]);
        let s = status(cx, &main).unwrap();
        assert_eq!(s.text, "Sublime failed: sh: subl: command not found");
        assert!(s.error);
    }

    #[gpui_kit::test]
    fn custom_command_in_terminal_reports_fallback_and_warp(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = with_commands(&tmp, "custom:Neovim");
        let denied = Outcome::FellBack(Fallback::Denied(Permission::Automation));
        let launches = fake_launcher(cx, Ok(denied));
        let (w, main) = launch(cx, &file);
        keys(cx, w, &["enter"]);
        assert_eq!(
            status(cx, &main).unwrap().text,
            "Opened /c/api in a new Terminal window without running Neovim. For commands, \
             allow Grove in System Settings → Privacy & Security → Automation"
        );

        cx.update(|cx| cx.global_mut::<Library>().settings.default_terminal = Some("warp".into()));
        keys(cx, w, &["enter"]);
        assert_eq!(launches.lock().unwrap().len(), 1);
        let s = status(cx, &main).unwrap();
        assert!(
            s.text.starts_with("Warp can't run custom commands"),
            "{}",
            s.text
        );
        assert!(s.error && s.settings);
    }

    #[gpui_kit::test]
    fn uninstalled_default_editor_is_an_error(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(
            &tmp,
            Settings {
                default_editor: Some("cursor".into()),
                ..Default::default()
            },
        );
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        let (w, main) = launch(cx, &file);
        keys(cx, w, &["enter"]);
        assert!(launches.lock().unwrap().is_empty());
        assert_eq!(
            status(cx, &main).unwrap(),
            Status {
                text: "Your default editor, Cursor, isn't installed anymore".into(),
                error: true,
                settings: true,
            }
        );
    }

    #[gpui_kit::test]
    fn inline_button_on_selected_worktree_only(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        let (w, main) = launch(cx, &file);
        let has_button = |cx: &mut TestAppContext, ix: usize| has(cx, w, "open-editor", ix);
        // Branch row selected: no button anywhere.
        assert!(!(0..10).any(|ix| has_button(cx, ix)));
        keys(cx, w, &["down"]);
        assert!(has_button(cx, 1));
        assert!(!has_button(cx, 4));

        cx.update_window(w, |_, window, cx| window.click(("open-editor", 1usize), cx))
            .unwrap();
        cx.run_until_parked();
        assert_eq!(*launches.lock().unwrap(), [code("/c/api")]);
        assert_eq!(
            status(cx, &main).unwrap().text,
            "Opening /c/api in VS Code…"
        );
    }

    fn row_badges(cx: &mut TestAppContext, main: &Entity<MainWindow>, path: &str) -> Vec<String> {
        main.read_with(cx, |m, cx| {
            m.tree
                .read(cx)
                .tree()
                .rows
                .iter()
                .find_map(|r| match &r.kind {
                    RowKind::Worktree {
                        path: p, badges, ..
                    } if p == Path::new(path) => Some(badges.iter().map(|b| b.label()).collect()),
                    _ => None,
                })
                .unwrap()
        })
    }

    #[gpui_kit::test]
    fn status_reads_landing_update_badges_and_keep_selection(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let (w, main) = launch(cx, &file);
        keys(cx, w, &["down", "down", "down", "down"]);
        let before = main.read_with(cx, |m, cx| m.tree.read(cx).selected().cloned());
        assert_eq!(before, Some(RowId::Worktree("/c/wt/api-x".into())));
        assert!(row_badges(cx, &main, "/c/wt/api-x").is_empty());

        cx.update(|cx| {
            let store = &mut cx.global_mut::<Library>().store;
            let stamp = store.status_reads().stamp();
            store.apply_status(
                stamp,
                Path::new("/c/wt/api-x"),
                Ok(WorktreeStatus {
                    files: changed_files(3),
                    upstream: Upstream::Tracking {
                        name: "origin/feature/x".into(),
                        ahead: 2,
                        behind: 1,
                    },
                    head: None,
                    head_time: None,
                    changes_time: None,
                }),
            );
        });
        cx.run_until_parked();
        assert_eq!(row_badges(cx, &main, "/c/wt/api-x"), ["3 changed", "↑2 ↓1"]);
        let after = main.read_with(cx, |m, cx| m.tree.read(cx).selected().cloned());
        assert_eq!(after, before);
    }

    #[gpui_kit::test]
    fn unavailable_worktree_cannot_be_opened(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        fake_presence(cx, "/c/wt/api-x", Presence::Unavailable);
        let mut repo = canned();
        repo.worktrees[1].presence = Presence::Unavailable;
        let (w, main) = launch_repo(cx, &file, repo);
        assert_eq!(row_badges(cx, &main, "/c/wt/api-x"), ["unavailable"]);
        keys(cx, w, &["down", "down", "down", "down", "enter"]);
        assert!(launches.lock().unwrap().is_empty());
        assert_eq!(
            status(cx, &main).unwrap().text,
            "Can't open /c/wt/api-x: its drive isn't mounted"
        );
    }

    #[gpui_kit::test]
    fn missing_worktree_cannot_be_opened(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        fake_presence(cx, "/c/wt/api-x", Presence::Missing);
        let mut repo = canned();
        repo.worktrees[1].presence = Presence::Missing; // /c/wt/api-x
        let (w, main) = launch_repo(cx, &file, repo);

        // feature/x branch, then its worktree; Enter and the button both refuse.
        keys(cx, w, &["down", "down", "down", "enter", "down", "enter"]);
        cx.update_window(w, |_, window, cx| window.click(("open-editor", 4usize), cx))
            .unwrap();
        cx.run_until_parked();
        assert!(launches.lock().unwrap().is_empty());
        let s = status(cx, &main).unwrap();
        assert_eq!(
            s.text,
            "Can't open /c/wt/api-x: folder is missing (use Remove missing entry…)"
        );
        assert!(s.error);
    }

    #[gpui_kit::test]
    fn folder_deleted_after_scan_is_refused_and_badged(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        let (w, main) = launch(cx, &file);
        assert!(row_badges(cx, &main, "/c/wt/api-x").is_empty());
        fake_presence(cx, "/c/wt/api-x", Presence::Missing);

        keys(cx, w, &["down", "down", "down", "down", "enter"]);
        assert!(launches.lock().unwrap().is_empty());
        assert_eq!(
            status(cx, &main).unwrap().text,
            "Can't open /c/wt/api-x: folder is missing (use Remove missing entry…)"
        );
        assert_eq!(row_badges(cx, &main, "/c/wt/api-x"), ["missing"]);
        keys(cx, w, &["cmd-enter"]);
        assert!(launches.lock().unwrap().is_empty());
    }

    #[gpui_kit::test]
    fn remounted_worktree_opens_and_rereads_status(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        let mut repo = canned();
        repo.worktrees[1].presence = Presence::Unavailable; // /c/wt/api-x
        let (w, main) = launch_repo(cx, &file, repo);
        assert_eq!(row_badges(cx, &main, "/c/wt/api-x"), ["unavailable"]);

        keys(cx, w, &["down", "down", "down", "down", "enter"]);
        assert_eq!(*launches.lock().unwrap(), [code("/c/wt/api-x")]);
        let wt = cx.update(|cx| {
            shell::library(cx).store.snapshot().repos[0]
                .worktrees
                .iter()
                .find(|w| w.path == Path::new("/c/wt/api-x"))
                .cloned()
                .unwrap()
        });
        assert_eq!(wt.presence, Presence::Present);
        // The fake path isn't a repo, so the reread lands as a failure, not unread.
        assert!(matches!(wt.status, Some(Err(_))));
    }

    #[gpui_kit::test]
    fn cmd_enter_opens_worktree_and_branch_rows_in_default_terminal(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        let (w, main) = launch(cx, &file);

        // Branch row `main` opens its worktree in a new Terminal tab.
        keys(cx, w, &["cmd-enter"]);
        let got = launches.lock().unwrap().clone();
        assert_eq!(got.len(), 1);
        terminal_window(&got[0], "/c/api");
        let s = status(cx, &main).unwrap();
        assert_eq!(s.text, "Opening /c/api in Terminal…");
        assert!(!s.error);

        // feature/x branch, then its worktree row; each open is a new launch.
        keys(
            cx,
            w,
            &["down", "down", "down", "cmd-enter", "down", "cmd-enter"],
        );
        let got = launches.lock().unwrap().clone();
        assert_eq!(got.len(), 3);
        terminal_window(&got[1], "/c/wt/api-x");
        terminal_window(&got[2], "/c/wt/api-x");
        assert_eq!(
            status(cx, &main).unwrap().text,
            "Opening /c/wt/api-x in Terminal…"
        );

        // Folder row: nothing. Plain Enter still goes to the editor.
        keys(cx, w, &["up", "up", "cmd-enter", "down", "enter"]);
        let got = launches.lock().unwrap().clone();
        assert_eq!(got.len(), 4);
        assert_eq!(got[3], code("/c/wt/api-x"));
    }

    #[gpui_kit::test]
    fn configured_terminal_and_window_setting(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(
            &tmp,
            Settings {
                default_terminal: Some("warp".into()),
                terminal_opens_in: Some(TerminalOpensIn::Window),
                ..Default::default()
            },
        );
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        let (w, main) = launch(cx, &file);
        keys(cx, w, &["cmd-enter"]);
        assert_eq!(
            *launches.lock().unwrap(),
            [vec![
                "/usr/bin/open",
                "warp://action/new_window?path=/c/api"
            ]]
        );
        assert_eq!(status(cx, &main).unwrap().text, "Opening /c/api in Warp…");
    }

    #[gpui_kit::test]
    fn automation_denied_shows_permission_hint(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let denied = Outcome::FellBack(Fallback::Denied(Permission::Automation));
        let launches = fake_launcher(cx, Ok(denied));
        let (w, main) = launch(cx, &file);
        keys(cx, w, &["cmd-enter"]);
        assert_eq!(launches.lock().unwrap().len(), 1);
        let s = status(cx, &main).unwrap();
        assert_eq!(
            s.text,
            "Opened /c/api in a new Terminal window. For tabs, allow Grove in System Settings → Privacy & Security → Automation"
        );
        assert!(s.error);
    }

    #[gpui_kit::test]
    fn terminal_launch_failure_shows_reason(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let fail = LaunchError::Exit {
            program: "open".into(),
            status: "exit status: 1".into(),
            stderr: "Unable to find application named 'Terminal'".into(),
        };
        fake_launcher(cx, Err(fail));
        let (w, main) = launch(cx, &file);
        keys(cx, w, &["cmd-enter"]);
        let s = status(cx, &main).unwrap();
        assert_eq!(
            s.text,
            "open failed: Unable to find application named 'Terminal'"
        );
        assert!(s.error);
    }

    #[gpui_kit::test]
    fn uninstalled_default_terminal_is_an_error(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(
            &tmp,
            Settings {
                default_terminal: Some("iterm2".into()),
                ..Default::default()
            },
        );
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        let (w, main) = launch(cx, &file);
        keys(cx, w, &["cmd-enter"]);
        assert!(launches.lock().unwrap().is_empty());
        assert_eq!(
            status(cx, &main).unwrap(),
            Status {
                text: "Your default terminal, iTerm2, isn't installed anymore".into(),
                error: true,
                settings: true,
            }
        );
    }

    #[gpui_kit::test]
    fn inline_terminal_button_opens_and_refuses_missing(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        fake_presence(cx, "/c/wt/api-x", Presence::Missing);
        let mut repo = canned();
        repo.worktrees[1].presence = Presence::Missing; // /c/wt/api-x
        let (w, main) = launch_repo(cx, &file, repo);
        let has_button = |cx: &mut TestAppContext, ix: usize| has(cx, w, "open-terminal", ix);
        assert!(!(0..10).any(|ix| has_button(cx, ix)));
        keys(cx, w, &["down"]);
        assert!(has_button(cx, 1));
        assert!(!has_button(cx, 4));

        cx.update_window(w, |_, window, cx| {
            window.click(("open-terminal", 1usize), cx)
        })
        .unwrap();
        cx.run_until_parked();
        let got = launches.lock().unwrap().clone();
        assert_eq!(got.len(), 1);
        terminal_window(&got[0], "/c/api");
        assert_eq!(
            status(cx, &main).unwrap().text,
            "Opening /c/api in Terminal…"
        );

        // Missing worktree: the button is inert and ⌘Enter refuses.
        keys(cx, w, &["down", "down", "down"]);
        assert!(has_button(cx, 4));
        cx.update_window(w, |_, window, cx| {
            window.click(("open-terminal", 4usize), cx)
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(launches.lock().unwrap().len(), 1);
        keys(cx, w, &["cmd-enter"]);
        assert_eq!(launches.lock().unwrap().len(), 1);
        assert_eq!(
            status(cx, &main).unwrap().text,
            "Can't open /c/wt/api-x: folder is missing (use Remove missing entry…)"
        );
    }

    #[gpui_kit::test]
    fn inline_buttons_fit_at_min_window_size(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        let mut repo = canned();
        repo.worktrees[1].status = Some(Ok(WorktreeStatus {
            files: changed_files(12),
            upstream: Upstream::Tracking {
                name: "origin/feature/x".into(),
                ahead: 12,
                behind: 30,
            },
            head: None,
            head_time: None,
            changes_time: None,
        }));
        let (w, _) = launch_at(cx, &file, repo, MIN_SIZE);
        keys(cx, w, &["down"; 4]);
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            let row = window.find(("tree-row", 4usize)).bounds();
            let details = window.find("details").bounds();
            let editor = window.find(("open-editor", 4usize)).bounds();
            let terminal = window.find(("open-terminal", 4usize)).bounds();
            assert!(editor.left() >= row.left());
            assert!(terminal.right() <= row.right());
            assert!(terminal.right() <= details.left());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn default_app_names_follow_detection_order(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        launch(cx, &file);
        cx.update(|cx| {
            assert_eq!(
                launcher::target_name(grove_core::Role::Terminal, cx).as_deref(),
                Some("Terminal")
            );
            assert_eq!(
                launcher::target_name(grove_core::Role::Editor, cx).as_deref(),
                Some("VS Code")
            );
        });
    }

    /// Status after ⌘Enter on `main` when the executor returns `outcome`.
    fn terminal_status(cx: &mut TestAppContext, settings: Settings, outcome: Outcome) -> Status {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, settings);
        fake_launcher(cx, Ok(outcome));
        let (w, main) = launch(cx, &file);
        keys(cx, w, &["cmd-enter"]);
        status(cx, &main).unwrap()
    }

    #[gpui_kit::test]
    fn permission_timeout_is_reported(cx: &mut TestAppContext) {
        let s = terminal_status(cx, Settings::default(), Outcome::TimedOut);
        assert_eq!(
            s.text,
            "Still waiting for permission to control Terminal. To open it, allow Grove in System Settings → Privacy & Security → Automation"
        );
        assert!(s.error);
    }

    #[gpui_kit::test]
    fn old_ghostty_tab_falls_back_softly(cx: &mut TestAppContext) {
        let settings = Settings {
            default_terminal: Some("ghostty".into()),
            ..Default::default()
        };
        let unsupported = Outcome::FellBack(Fallback::Unsupported(
            "Expected end of line but found identifier. (-2741)".into(),
        ));
        let s = terminal_status(cx, settings, unsupported);
        assert_eq!(
            s.text,
            "Opened /c/api in a new Ghostty window (tabs need Ghostty 1.3 or later)"
        );
        assert!(!s.error);
    }

    #[gpui_kit::test]
    fn cold_launch_fallback_keeps_confirmation(cx: &mut TestAppContext) {
        let settings = Settings {
            default_terminal: Some("ghostty".into()),
            ..Default::default()
        };
        let s = terminal_status(cx, settings, Outcome::FellBack(Fallback::NotRunning));
        assert_eq!(s.text, "Opening /c/api in Ghostty…");
        assert!(!s.error);
    }

    #[gpui_kit::test]
    fn tab_script_failure_names_the_reason(cx: &mut TestAppContext) {
        let failed =
            Outcome::FellBack(Fallback::Failed("Terminal didn't come to the front".into()));
        let s = terminal_status(cx, Settings::default(), failed);
        assert_eq!(
            s.text,
            "Opened /c/api in a new Terminal window (new tab failed: Terminal didn't come to the front)"
        );
        assert!(s.error);
    }

    fn changed_files(n: usize) -> Vec<ChangedFile> {
        (0..n)
            .map(|i| ChangedFile {
                change: Change::Modified,
                path: format!("f{i}.txt"),
                from: None,
            })
            .collect()
    }

    fn click(cx: &mut TestAppContext, w: AnyWindowHandle, id: impl Into<gpui_kit::ElementId>) {
        let id = id.into();
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx);
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn found(cx: &mut TestAppContext, w: AnyWindowHandle, id: &'static str) -> bool {
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).is_some()
        })
        .unwrap()
    }

    /// The details pane's title and kind line.
    fn details(cx: &mut TestAppContext, main: &Entity<MainWindow>) -> (String, String) {
        main.read_with(cx, |m, cx| {
            let d = m.details.read(cx).details().cloned().unwrap();
            (d.title, d.kind)
        })
    }

    fn pair(title: &str, kind: &str) -> (String, String) {
        (title.into(), kind.into())
    }

    #[gpui_kit::test]
    fn details_show_name_and_kind_for_branch_and_worktree(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        let (w, main) = launch(cx, &file);
        assert!(found(cx, w, "details-kind"));
        assert_eq!(details(cx, &main), pair("main", "Branch · has worktree"));
        keys(cx, w, &["down"]);
        assert_eq!(details(cx, &main), pair("api", "Worktree · main worktree"));
        keys(cx, w, &["down", "down"]);
        assert_eq!(
            details(cx, &main),
            pair("feature/x", "Branch · has worktree")
        );
        keys(cx, w, &["down"]);
        assert_eq!(
            details(cx, &main),
            pair("api-x", "Worktree · linked worktree")
        );
        keys(cx, w, &["down"; 9]);
        assert_eq!(
            details(cx, &main),
            pair("bisect", "Worktree · detached HEAD")
        );
    }

    #[gpui_kit::test]
    fn details_show_upstream_sync_and_changed_files(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        let mut repo = canned();
        repo.worktrees[0].status = Some(Ok(WorktreeStatus {
            files: changed_files(2),
            upstream: Upstream::Tracking {
                name: "origin/main".into(),
                ahead: 2,
                behind: 1,
            },
            head: None,
            head_time: None,
            changes_time: None,
        }));
        let (w, main) = launch_repo(cx, &file, repo);
        let fields = main.read_with(cx, |m, cx| m.details.read(cx).status_fields());
        assert_eq!(
            fields,
            [
                ("Branch", "main".to_string()),
                ("Upstream", "origin/main".into()),
                ("Sync", "2 ahead, 1 behind".into()),
                ("Working tree", "2 changed files".into()),
            ]
        );
        assert!(has(cx, w, "changed-file", 0));
        assert!(has(cx, w, "changed-file", 1));
        assert!(!has(cx, w, "changed-file", 2));
        keys(cx, w, &["down", "down", "down", "down"]);
        assert!(!has(cx, w, "changed-file", 0));
    }

    #[gpui_kit::test]
    fn menu_open_uses_chosen_app_once_without_changing_default(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        let (w, main) = launch(cx, &file);

        click(cx, w, "details-menu-editor");
        assert!(found(cx, w, "editor-menu"));
        click(cx, w, ("editor-choice", 1usize));
        assert_eq!(
            *launches.lock().unwrap(),
            [vec!["/fake/bin/zed", "--new", "/c/api"]]
        );
        assert_eq!(status(cx, &main).unwrap().text, "Opening /c/api in Zed…");
        assert!(!found(cx, w, "editor-menu"));

        // The default is untouched: the primary button and Enter still use VS Code.
        click(cx, w, "details-open-editor");
        keys(cx, w, &["enter"]);
        assert_eq!(
            launches.lock().unwrap()[1..],
            [code("/c/api"), code("/c/api")]
        );
        assert_eq!(
            cx.update(|cx| shell::library(cx).settings.default_editor.clone()),
            None
        );
        assert_eq!(file.load().unwrap().default_editor, None);

        click(cx, w, "details-menu-terminal");
        click(cx, w, ("terminal-choice", 1usize));
        assert_eq!(
            launches.lock().unwrap()[3],
            ["/usr/bin/open", "warp://action/new_tab?path=/c/api"]
        );
        click(cx, w, "details-open-terminal");
        terminal_window(&launches.lock().unwrap()[4], "/c/api");
        assert_eq!(file.load().unwrap().default_terminal, None);
    }

    #[gpui_kit::test]
    fn uninstalled_default_falls_back_to_menu(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(
            &tmp,
            Settings {
                default_editor: Some("cursor".into()),
                ..Default::default()
            },
        );
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        let (w, main) = launch(cx, &file);
        click(cx, w, "details-open-editor");
        assert!(launches.lock().unwrap().is_empty());
        assert!(status(cx, &main).unwrap().settings);
        assert!(found(cx, w, "editor-menu"));
        click(cx, w, ("editor-choice", 0usize));
        assert_eq!(*launches.lock().unwrap(), [code("/c/api")]);
    }

    #[gpui_kit::test]
    fn copy_cd_and_path_copy_shell_quoted_command_and_raw_path(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        let mut repo = canned();
        repo.worktrees[1].path = "/c/wt/it's here".into();
        let (w, main) = launch_repo(cx, &file, repo);
        keys(cx, w, &["down", "down", "down", "down"]);
        let clipboard = |cx: &mut TestAppContext| cx.read_from_clipboard().and_then(|c| c.text());

        click(cx, w, "details-copy-cd");
        assert_eq!(clipboard(cx).as_deref(), Some(r"cd '/c/wt/it'\''s here'"));
        assert_eq!(
            status(cx, &main).unwrap().text,
            "Copied cd command for /c/wt/it's here"
        );

        click(cx, w, "details-copy-path");
        assert_eq!(clipboard(cx).as_deref(), Some("/c/wt/it's here"));
        assert_eq!(
            status(cx, &main).unwrap().text,
            "Copied path /c/wt/it's here"
        );

        click(cx, w, "details-reveal");
        assert_eq!(
            status(cx, &main).unwrap().text,
            "Revealed /c/wt/it's here in Finder"
        );
    }

    /// Selects `/c/wt/api-x` with `presence` and tries every open control in the pane.
    fn try_details_opens(cx: &mut TestAppContext, presence: Presence) -> (Launches, Status) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        fake_presence(cx, "/c/wt/api-x", presence);
        let mut repo = canned();
        repo.worktrees[1].presence = presence;
        let (w, main) = launch_repo(cx, &file, repo);
        keys(cx, w, &["down", "down", "down", "down"]);
        for id in [
            "details-open-editor",
            "details-menu-editor",
            "details-open-terminal",
            "details-menu-terminal",
        ] {
            click(cx, w, id);
        }
        assert!(!found(cx, w, "editor-menu") && !found(cx, w, "terminal-menu"));
        click(cx, w, "details-reveal");
        (launches, status(cx, &main).unwrap())
    }

    #[gpui_kit::test]
    fn details_open_disabled_for_missing_worktree(cx: &mut TestAppContext) {
        let (launches, s) = try_details_opens(cx, Presence::Missing);
        assert!(launches.lock().unwrap().is_empty());
        assert_eq!(
            s.text,
            "Can't reveal /c/wt/api-x: folder is missing (use Remove missing entry…)"
        );
        assert!(s.error);
    }

    #[gpui_kit::test]
    fn details_open_disabled_for_unavailable_worktree(cx: &mut TestAppContext) {
        let (launches, s) = try_details_opens(cx, Presence::Unavailable);
        assert!(launches.lock().unwrap().is_empty());
        assert_eq!(s.text, "Can't reveal /c/wt/api-x: its drive isn't mounted");
    }

    #[gpui_kit::test]
    fn details_open_refused_when_folder_vanished_after_scan(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        let (w, main) = launch(cx, &file);
        keys(cx, w, &["down", "down", "down", "down"]);
        fake_presence(cx, "/c/wt/api-x", Presence::Missing);
        click(cx, w, "details-menu-editor");
        click(cx, w, ("editor-choice", 1usize));
        assert!(launches.lock().unwrap().is_empty());
        assert_eq!(
            status(cx, &main).unwrap().text,
            "Can't open /c/wt/api-x: folder is missing (use Remove missing entry…)"
        );
        // The refusal rechecked presence, so the pane now shows it disabled.
        click(cx, w, "details-open-editor");
        assert!(launches.lock().unwrap().is_empty());
        assert_eq!(row_badges(cx, &main, "/c/wt/api-x"), ["missing"]);
    }

    fn git(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .current_dir(dir)
            .args(["-c", "user.name=T", "-c", "user.email=t@t.invalid"])
            .args([
                "-c",
                "commit.gpgsign=false",
                "-c",
                "init.defaultBranch=main",
            ])
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[gpui_kit::test]
    fn details_list_last_five_commits_in_background(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        let repo_dir = tmp.path().join("api");
        std::fs::create_dir(&repo_dir).unwrap();
        git(&repo_dir, &["init", "-q"]);
        for m in ["one", "two", "three", "four", "five", "six"] {
            git(&repo_dir, &["commit", "-q", "--allow-empty", "-m", m]);
        }
        let mut repo = canned();
        repo.worktrees[0].path = repo_dir;
        let (w, main) = launch_repo(cx, &file, repo);
        cx.run_until_parked();
        assert!((0..5).all(|i| has(cx, w, "commit", i)));
        assert!(!has(cx, w, "commit", 5));
        // A linked worktree that isn't a repo shows the error instead of stale commits.
        keys(cx, w, &["down", "down", "down", "down"]);
        let err = main.read_with(cx, |m, cx| match m.details.read(cx).commits() {
            Some(Err(e)) => e.to_string(),
            other => panic!("expected an error, got {other:?}"),
        });
        assert!(
            err.starts_with("git log -5") && err.contains("failed:"),
            "{err}"
        );
        assert!(!has(cx, w, "commit", 0));
    }

    #[gpui_kit::test]
    fn fetch_all_failure_shows_repo_and_error_and_marks_counts_stale(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        // `main` two behind `origin/main`, whose remote is unreachable.
        let code = tmp.path().join("code");
        let api = code.join("api");
        std::fs::create_dir_all(&api).unwrap();
        git(&api, &["init", "-q"]);
        for m in ["one", "two", "three"] {
            git(&api, &["commit", "-q", "--allow-empty", "-m", m]);
        }
        let gone = tmp.path().join("gone.git");
        git(&api, &["remote", "add", "origin", gone.to_str().unwrap()]);
        git(&api, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
        git(&api, &["branch", "-q", "-u", "origin/main"]);
        git(&api, &["reset", "-q", "--hard", "HEAD~2"]);
        let mut repo = grove_core::scan(&code).repos.remove(0);
        let path = repo.worktrees[0].path.clone();
        repo.worktrees[0].status = grove_core::read_worktree_status(&path).ok().map(Ok);
        let (w, main) = launch_repo(cx, &file, repo);
        assert_eq!(cx.update(|cx| fetch_label(cx)), "Not fetched yet");
        let path = path.to_str().unwrap();
        assert_eq!(row_badges(cx, &main, path), ["main worktree", "↓2"]);

        click(cx, w, "fetch-all");

        let s = status(cx, &main).unwrap();
        assert!(s.text.starts_with("Fetch failed for api: "), "{}", s.text);
        assert!(s.error);
        assert_eq!(
            row_badges(cx, &main, path),
            ["main worktree", "↓2", "stale"]
        );
        // Nothing fetched, so no fetch time over the stale counts.
        assert_eq!(cx.update(|cx| fetch_label(cx)), "Not fetched yet");
    }

    #[gpui_kit::test]
    fn fetch_report_shows_once_in_status_line(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let (_, main) = launch(cx, &file);
        cx.update(|cx| {
            let fetcher = cx.default_global::<shell::Fetcher>();
            fetcher.finished += 1;
            fetcher.report = Some(grove_core::FetchReport {
                fetched: 3,
                failures: vec![],
            });
        });
        cx.run_until_parked();
        assert_eq!(
            status(cx, &main).unwrap(),
            Status {
                text: "Fetched 3 repos".into(),
                error: false,
                settings: false,
            }
        );

        // A later fetch starting doesn't repeat the old report.
        main.update(cx, |m, _| m.status = None);
        cx.update(|cx| cx.global_mut::<shell::Fetcher>().running = true);
        cx.run_until_parked();
        assert_eq!(status(cx, &main), None);
        assert_eq!(cx.update(|cx| fetch_label(cx)), "Not fetched yet");
    }

    #[gpui_kit::test]
    fn status_bar_shows_last_fetch_time(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        launch(cx, &file);
        cx.update(|cx| {
            let snapshot = Snapshot {
                last_fetch: Some(grove_core::unix_now() - 180),
                ..shell::library(cx).store.snapshot().as_ref().clone()
            };
            cx.global_mut::<Library>().store = SnapshotStore::from_snapshot(snapshot);
        });
        assert_eq!(cx.update(|cx| fetch_label(cx)), "Last fetch 3 minutes ago");
    }

    #[gpui_kit::test]
    fn background_fetch_runs_every_15_minutes_only_when_on(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        launch(cx, &file);
        cx.update(shell::start_background_fetch);
        let fetches = |cx: &mut TestAppContext| {
            cx.update(|cx| cx.try_global::<shell::Fetcher>().map_or(0, |f| f.finished))
        };
        let quarter_hour = std::time::Duration::from_secs(15 * 60);

        cx.executor().advance_clock(quarter_hour);
        cx.run_until_parked();
        assert_eq!(fetches(cx), 0);

        cx.update(|cx| cx.global_mut::<Library>().settings.background_fetch = true);
        cx.executor()
            .advance_clock(quarter_hour - std::time::Duration::from_secs(1));
        cx.run_until_parked();
        assert_eq!(fetches(cx), 0);
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(1));
        cx.run_until_parked();
        assert_eq!(fetches(cx), 1);
        cx.executor().advance_clock(quarter_hour);
        cx.run_until_parked();
        assert_eq!(fetches(cx), 2);
    }

    fn switcher_open(cx: &mut TestAppContext) -> bool {
        cx.update(|cx| crate::switcher::window(cx).is_some_and(|w| cx.windows().contains(&w)))
    }

    #[gpui_kit::test]
    fn cmd_k_and_jump_to_open_the_switcher(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        let (w, _) = launch(cx, &file);
        assert!(!switcher_open(cx));
        keys(cx, w, &["cmd-k"]);
        assert!(switcher_open(cx));
        let switcher = cx.update(|cx| crate::switcher::window(cx).unwrap());
        keys(cx, switcher, &["escape"]);
        assert!(!switcher_open(cx));

        click(cx, w, "jump-to");
        assert!(switcher_open(cx));
        // Again closes it: one switcher, toggled.
        click(cx, w, "jump-to");
        assert!(!switcher_open(cx));
    }

    #[gpui_kit::test]
    fn switcher_launch_outcomes_reach_the_status_bar(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let fail = LaunchError::Exit {
            program: "code".into(),
            status: "exit status: 1".into(),
            stderr: "bad option".into(),
        };
        fake_launcher(cx, Err(fail));
        let (w, main) = launch(cx, &file);
        keys(cx, w, &["cmd-k"]);
        let switcher = cx.update(|cx| crate::switcher::window(cx).unwrap());
        keys(cx, switcher, &["enter"]);
        assert_eq!(
            status(cx, &main),
            Some(Status {
                text: "code failed: bad option".into(),
                error: true,
                settings: false,
            })
        );
    }

    #[gpui_kit::test]
    fn notice_posted_with_no_main_window_shows_when_one_opens(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let hint = Status {
            text: "Allow Grove to control Terminal".into(),
            error: true,
            settings: false,
        };
        cx.update(|cx| shell::post(hint.clone(), cx));
        let (_, main) = launch(cx, &file);
        assert_eq!(status(cx, &main), Some(hint.clone()));
        // Once shown, the next window doesn't repeat it.
        let (_, again) = launch(cx, &file);
        assert_eq!(status(cx, &again), None);

        // Nor a stale one.
        cx.update(|cx| {
            shell::post(hint, cx);
            let at = Instant::now()
                .checked_sub(Duration::from_secs(120))
                .unwrap();
            cx.global_mut::<shell::Notices>().posted_at = Some(at);
        });
        let (_, late) = launch(cx, &file);
        assert_eq!(status(cx, &late), None);
    }

    #[gpui_kit::test]
    fn tree_and_details_opens_are_recorded_to_recents_and_last_opened(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        fake_presence(cx, "/c/wt/api-y", Presence::Missing);
        let (w, _) = launch(cx, &file);
        let recents = || file.load().unwrap().recents;
        let opened = || {
            let last_opened = file.load().unwrap().last_opened;
            let since = grove_core::unix_now() - 60;
            assert!(last_opened.values().all(|&t| t >= since), "{last_opened:?}");
            last_opened.into_keys().collect::<Vec<_>>()
        };
        keys(cx, w, &["enter"]);
        assert_eq!(recents(), [PathBuf::from("/c/api")]);
        assert_eq!(opened(), [PathBuf::from("/c/api")]);

        keys(cx, w, &["down", "down", "down", "down"]);
        click(cx, w, "details-open-terminal");
        assert_eq!(
            recents(),
            [PathBuf::from("/c/wt/api-x"), PathBuf::from("/c/api")]
        );
        assert_eq!(
            opened(),
            [PathBuf::from("/c/api"), PathBuf::from("/c/wt/api-x")]
        );

        // A refused open isn't one.
        keys(cx, w, &["down", "down", "enter"]);
        assert_eq!(recents().len(), 2);
        assert_eq!(opened().len(), 2);
    }

    fn two_repos(cx: &mut TestAppContext, file: &SettingsFile) -> AnyWindowHandle {
        let mut web = canned();
        web.display_name = "web".into();
        web.repo.name = "web".into();
        web.repo.path = "/c/web".into();
        web.repo.git_dir = "/c/web/.git".into();
        web.worktrees.truncate(1);
        web.worktrees[0].path = "/c/web".into();
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::details_view::init(cx);
            set_library(file, vec![canned(), web], cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(1180.), px(720.)),
                })),
                ..Default::default()
            };
            open_with(options, cx).unwrap().0
        })
    }

    /// The header row (count, chip, manage button, sort menu) must fit inside the sidebar
    /// even with a two-digit hidden count — widening the sidebar came from this review.
    #[gpui_kit::test]
    fn the_header_and_chip_fit_inside_the_sidebar(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        let names: Vec<String> = (0..12).map(|i| format!("r{i:02}")).collect();
        let repos = names.iter().map(|n| repo_named(n)).collect();
        let (w, _) = launch_repos(cx, &file, repos, size(px(1280.), px(720.)));
        open_visibility(cx, w);
        // Hide ten repos, so the chip reads "· 10 hidden".
        for i in 0..10usize {
            click(cx, w, ("visibility-eye", i));
        }
        assert_eq!(label_of(cx, w, "hidden-chip"), "10 hidden");
        keys(cx, w, &["escape"]);
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            let sidebar_right = window.find("repo-list").bounds().right();
            for id in [
                "repos-header",
                "hidden-chip",
                "manage-visibility",
                "repo-sort",
            ] {
                assert!(
                    window.find(id).bounds().right() <= sidebar_right,
                    "{id} overflows the sidebar"
                );
            }
            // The manage button keeps its breathing room before the sort menu.
            let manage = window.find("manage-visibility").bounds().right();
            let sort_left = window.find("repo-sort").bounds().left();
            assert!(
                manage < sort_left,
                "manage button crowded against the sort menu"
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn tab_reaches_every_control_and_comes_back_to_the_tree(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        let w = two_repos(cx, &file);
        let ids: [gpui_kit::ElementId; 15] = [
            "jump-to".into(),
            "fetch-all".into(),
            "open-settings".into(),
            "repo-sort".into(),
            "tree-sort".into(),
            ("repo", 0usize).into(),
            ("repo", 1usize).into(),
            "change-code-folder".into(),
            "details-copy-path".into(),
            "details-copy-cd".into(),
            "details-reveal".into(),
            "details-open-editor".into(),
            "details-menu-editor".into(),
            "details-open-terminal".into(),
            "details-menu-terminal".into(),
        ];
        for id in ids {
            tab_to(cx, w, id);
            tab_to(cx, w, "tree-view");
        }
    }

    #[gpui_kit::test]
    fn keyboard_switches_repo_and_lands_in_its_tree(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        let w = two_repos(cx, &file);
        tab_to(cx, w, ("repo", 1usize));
        keys(cx, w, &["enter"]);
        let label = |cx: &mut TestAppContext, ix: usize| {
            cx.update_window(w, |_, window, cx| {
                window.render_frame(cx);
                window.find(("repo", ix)).label().unwrap().to_string()
            })
            .unwrap()
        };
        assert_eq!(label(cx, 1), "web, 1 worktree, selected");
        assert_eq!(label(cx, 0), "api, 5 worktrees");
        // Focus moved into the tree: Enter opens the new repo's worktree.
        keys(cx, w, &["enter"]);
        assert_eq!(*launches.lock().unwrap(), [code("/c/web")]);
        assert_eq!(
            file.load().unwrap().last_selected_repo.as_deref(),
            Some("web")
        );
    }

    #[gpui_kit::test]
    fn open_with_menu_works_from_the_keyboard(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        let (w, main) = launch(cx, &file);
        let focused = |cx: &mut TestAppContext, id: gpui_kit::ElementId| {
            cx.update_window(w, |_, window, cx| {
                window.render_frame(cx);
                window.find(id).focused() == Some(true)
            })
            .unwrap()
        };

        tab_to(cx, w, "details-menu-editor");
        keys(cx, w, &["space"]);
        assert!(found(cx, w, "editor-menu"));
        assert!(focused(cx, ("editor-choice", 0usize).into()));
        keys(cx, w, &["escape"]);
        assert!(!found(cx, w, "editor-menu"));
        assert!(focused(cx, "details-menu-editor".into()));

        keys(cx, w, &["enter", "down", "down", "up", "up", "down"]);
        assert!(focused(cx, ("editor-choice", 1usize).into()));
        keys(cx, w, &["enter"]);
        assert_eq!(
            *launches.lock().unwrap(),
            [vec!["/fake/bin/zed", "--new", "/c/api"]]
        );
        assert!(!found(cx, w, "editor-menu"));
        assert!(focused(cx, "details-menu-editor".into()));

        // Tab leaves the menu and closes it.
        keys(cx, w, &["enter", "tab"]);
        assert!(!found(cx, w, "editor-menu"));
        assert!(focused(cx, "details-open-terminal".into()));

        tab_to(cx, w, "details-copy-path");
        keys(cx, w, &["enter"]);
        assert_eq!(status(cx, &main).unwrap().text, "Copied path /c/api");
    }

    #[gpui_kit::test]
    fn empty_fallback_menu_still_closes_on_escape(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        cx.update(|cx| {
            let l = cx.global_mut::<crate::launcher::Launcher>();
            l.targets
                .as_mut()
                .unwrap()
                .retain(|t| !matches!(t.kind, grove_core::TargetKind::Editor(_)));
        });
        let (w, _) = launch(cx, &file);
        tab_to(cx, w, "details-open-editor");
        keys(cx, w, &["enter"]);
        assert!(launches.lock().unwrap().is_empty());
        assert!(found(cx, w, "editor-menu"));
        keys(cx, w, &["escape"]);
        assert!(!found(cx, w, "editor-menu"));
        tab_to(cx, w, "tree-view");
    }

    #[gpui_kit::test]
    fn tree_rows_and_icon_buttons_have_accessible_labels(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        let (w, _) = launch(cx, &file);
        keys(cx, w, &["down"]);
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            let row = window.find(("tree-row", 1usize));
            assert_eq!(
                row.label(),
                Some("/c/api, worktree, The repo's original clone")
            );
            assert_eq!(row.selected(), Some(true));
            assert_eq!(
                window.find(("open-terminal", 1usize)).label(),
                Some("Open in Terminal")
            );
            assert_eq!(
                window.find("open-settings").label(),
                Some("Open-with settings")
            );
            assert_eq!(
                window.find("details-menu-editor").label(),
                Some("Open in another editor")
            );
        })
        .unwrap();
    }

    fn label_of(
        cx: &mut TestAppContext,
        w: AnyWindowHandle,
        id: impl Into<gpui_kit::ElementId>,
    ) -> String {
        let id = id.into();
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            window.find(id).label().unwrap().to_string()
        })
        .unwrap()
    }

    #[gpui_kit::test]
    fn pin_button_and_context_menu_pin_persist_and_keep_selection(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        let w = two_repos(cx, &file);
        let pins = || file.load().unwrap().pinned_repos;
        assert!(!has(cx, w, "repo-pin", 1), "only on hover or selection");
        assert_eq!(label_of(cx, w, ("repo-pin", 0usize)), "Pin api");

        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            window.hover(("repo", 1usize), cx);
        })
        .unwrap();
        click(cx, w, ("repo-pin", 1usize));
        assert_eq!(pins(), [PathBuf::from("/c/web")]);
        assert_eq!(label_of(cx, w, ("repo", 0usize)), "web, 1 worktree, pinned");
        assert_eq!(label_of(cx, w, ("repo-pin", 0usize)), "Unpin web");
        assert_eq!(
            label_of(cx, w, ("repo", 1usize)),
            "api, 5 worktrees, selected"
        );

        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            window.right_click(("repo", 0usize), cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(label_of(cx, w, "repo-menu-pin"), "Unpin");
        click(cx, w, "repo-menu-pin");
        assert!(pins().is_empty());
        assert!(!found(cx, w, "repo-menu"));
        assert_eq!(
            label_of(cx, w, ("repo", 0usize)),
            "api, 5 worktrees, selected"
        );

        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            window.right_click(("repo", 0usize), cx);
        })
        .unwrap();
        click(cx, w, "repo-menu-pin");
        assert_eq!(pins(), [PathBuf::from("/c/api")]);
        assert_eq!(
            label_of(cx, w, ("repo", 0usize)),
            "api, 5 worktrees, pinned, selected"
        );
    }

    #[gpui_kit::test]
    fn repo_menu_closes_on_escape_tab_and_outside_click(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        let w = two_repos(cx, &file);
        let open = |cx: &mut TestAppContext| {
            cx.update_window(w, |_, window, cx| {
                window.render_frame(cx);
                window.right_click(("repo", 1usize), cx);
            })
            .unwrap();
            cx.run_until_parked();
            assert!(found(cx, w, "repo-menu"));
        };
        let focused = |cx: &mut TestAppContext, id: gpui_kit::ElementId| {
            cx.update_window(w, |_, window, cx| {
                window.render_frame(cx);
                window.find(id).focused() == Some(true)
            })
            .unwrap()
        };

        open(cx);
        assert!(focused(cx, "repo-menu-pin".into()));
        keys(cx, w, &["escape"]);
        assert!(!found(cx, w, "repo-menu"));
        assert!(focused(cx, ("repo", 1usize).into()));

        open(cx);
        keys(cx, w, &["tab"]);
        assert!(!found(cx, w, "repo-menu"));

        open(cx);
        click(cx, w, "repos-header");
        assert!(!found(cx, w, "repo-menu"));
        assert!(file.load().unwrap().pinned_repos.is_empty());
    }

    #[gpui_kit::test]
    fn arrow_keys_cross_from_pinned_into_the_rest(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(
            &tmp,
            Settings {
                pinned_repos: vec!["/c/web".into()],
                ..Default::default()
            },
        );
        fake_launcher(cx, Ok(Outcome::Opened));
        let w = two_repos(cx, &file);
        tab_to(cx, w, ("repo", 0usize));
        keys(cx, w, &["down"]);
        assert_eq!(
            label_of(cx, w, ("repo", 1usize)),
            "api, 5 worktrees, selected"
        );
        keys(cx, w, &["up"]);
        assert_eq!(
            label_of(cx, w, ("repo", 0usize)),
            "web, 1 worktree, pinned, selected"
        );
        assert_eq!(
            file.load().unwrap().last_selected_repo.as_deref(),
            Some("web")
        );
        // Focus stayed in the sidebar, on the selected row.
        keys(cx, w, &["down", "down"]);
        assert_eq!(
            label_of(cx, w, ("repo", 1usize)),
            "api, 5 worktrees, selected"
        );
    }

    fn header_texts(cx: &mut TestAppContext, w: AnyWindowHandle) -> Vec<String> {
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            ["pinned-header", "repos-header"]
                .into_iter()
                .filter_map(|id| Some(window.try_find(id)?.label()?.to_string()))
                .collect()
        })
        .unwrap()
    }

    #[gpui_kit::test]
    fn all_pinned_still_counts_every_repo_and_selects_the_first_pin(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(
            &tmp,
            Settings {
                pinned_repos: vec!["/c/web".into(), "/c/api".into()],
                ..Default::default()
            },
        );
        fake_launcher(cx, Ok(Outcome::Opened));
        let w = two_repos(cx, &file);
        assert_eq!(header_texts(cx, w), ["PINNED · 2", "REPOSITORIES · 2"]);
        assert_eq!(
            label_of(cx, w, ("repo", 0usize)),
            "api, 5 worktrees, pinned, selected"
        );
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            let list: gpui_kit::ElementId = "repo-list".into();
            let in_list = |id: gpui_kit::ElementId| window.find(id).path().contains(&list);
            // REPOSITORIES and PINNED stay fixed above the scrolling list.
            assert!(!in_list("repos-header".into()));
            assert!(!in_list("pinned-header".into()));
            assert!(!in_list(("repo", 0usize).into()));
            assert!(
                window.find("repos-header").bounds().origin.y
                    < window.find("pinned-header").bounds().origin.y
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn unpinning_from_the_keyboard_keeps_focus_on_the_moved_button(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(
            &tmp,
            Settings {
                pinned_repos: vec!["/c/web".into()],
                last_selected_repo: Some("api".into()),
                ..Default::default()
            },
        );
        fake_launcher(cx, Ok(Outcome::Opened));
        let w = two_repos(cx, &file);
        assert_eq!(header_texts(cx, w), ["PINNED · 1", "REPOSITORIES · 2"]);
        tab_to(cx, w, ("repo-pin", 0usize));
        keys(cx, w, &["enter"]);
        assert!(file.load().unwrap().pinned_repos.is_empty());
        assert_eq!(header_texts(cx, w), ["REPOSITORIES · 2"]);
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            let pin = window.find(("repo-pin", 1usize));
            assert_eq!(pin.label(), Some("Pin web"));
            assert_eq!(pin.focused(), Some(true));
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn pinning_keeps_a_saved_selection_of_an_absent_repo(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(
            &tmp,
            Settings {
                last_selected_repo: Some("gone".into()),
                ..Default::default()
            },
        );
        fake_launcher(cx, Ok(Outcome::Opened));
        let w = two_repos(cx, &file);
        click(cx, w, ("repo-pin", 0usize));
        let s = file.load().unwrap();
        assert_eq!(s.pinned_repos, [PathBuf::from("/c/api")]);
        assert_eq!(s.last_selected_repo.as_deref(), Some("gone"));
    }

    /// The manage popover's rows: every repo, in sidebar order, with its hidden state.
    fn visibility_rows(cx: &mut TestAppContext, w: AnyWindowHandle) -> Vec<String> {
        indexed_labels(cx, w, "visibility-row")
    }

    fn open_visibility(cx: &mut TestAppContext, w: AnyWindowHandle) {
        click(cx, w, "manage-visibility");
        assert!(found(cx, w, "visibility-menu"));
    }

    #[gpui_kit::test]
    fn manage_popover_lists_every_repo_and_closes_on_escape_outside_and_button(
        cx: &mut TestAppContext,
    ) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(
            &tmp,
            Settings {
                pinned_repos: vec!["/c/web".into()],
                ..Default::default()
            },
        );
        fake_launcher(cx, Ok(Outcome::Opened));
        let w = two_repos(cx, &file);
        assert_eq!(header_texts(cx, w), ["PINNED · 1", "REPOSITORIES · 2"]);
        assert!(!found(cx, w, "hidden-chip"), "nothing hidden yet");
        assert_eq!(
            cx.update_window(w, |_, window, cx| {
                window.render_frame(cx);
                Some(window.find("manage-visibility").label()?.to_string())
            })
            .unwrap(),
            Some("Manage repository visibility".into())
        );

        open_visibility(cx, w);
        assert_eq!(visibility_rows(cx, w), ["web, shown", "api, shown"]);

        // The filter field starts focused, so Escape goes to the popover.
        keys(cx, w, &["escape"]);
        assert!(!found(cx, w, "visibility-menu"));

        open_visibility(cx, w);
        click(cx, w, "repos-header");
        assert!(!found(cx, w, "visibility-menu"));

        open_visibility(cx, w);
        click(cx, w, "manage-visibility");
        assert!(!found(cx, w, "visibility-menu"));
    }

    #[gpui_kit::test]
    fn hiding_applies_at_once_persists_and_the_chip_reopens_the_popover(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        let w = two_repos(cx, &file);

        open_visibility(cx, w);
        click(cx, w, ("visibility-eye", 0usize));
        // api leaves the sidebar at once; the count follows the list and the chip appears.
        assert_eq!(header_texts(cx, w), ["REPOSITORIES · 1"]);
        // The hidden api keeps the selection, so the visible web row is not selected.
        assert_eq!(label_of(cx, w, ("repo", 0usize)), "web, 1 worktree");
        assert_eq!(file.load().unwrap().hidden_repos, [PathBuf::from("/c/api")]);
        // The popover stays open, api now hidden.
        assert_eq!(visibility_rows(cx, w), ["api, hidden", "web, shown"]);

        // The chip reopens the same popover.
        keys(cx, w, &["escape"]);
        click(cx, w, "hidden-chip");
        assert!(found(cx, w, "visibility-menu"));

        click(cx, w, ("visibility-eye", 0usize));
        assert_eq!(header_texts(cx, w), ["REPOSITORIES · 2"]);
        assert!(!found(cx, w, "hidden-chip"));
        assert!(file.load().unwrap().hidden_repos.is_empty());
        assert_eq!(visibility_rows(cx, w), ["api, shown", "web, shown"]);
        assert_eq!(
            label_of(cx, w, ("repo", 0usize)),
            "api, 5 worktrees, selected"
        );
    }

    #[gpui_kit::test]
    fn hiding_a_pinned_repo_leaves_pinned_and_returns_it_intact(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(
            &tmp,
            Settings {
                pinned_repos: vec!["/c/web".into()],
                last_selected_repo: Some("api".into()),
                ..Default::default()
            },
        );
        fake_launcher(cx, Ok(Outcome::Opened));
        let mut web = canned();
        web.display_name = "web".into();
        web.repo.name = "web".into();
        web.repo.path = "/c/web".into();
        web.repo.git_dir = "/c/web/.git".into();
        web.worktrees.truncate(1);
        web.worktrees[0].path = "/c/web".into();
        let (w, main) = launch_repos(cx, &file, vec![web, canned()], size(px(1180.), px(720.)));

        open_visibility(cx, w);
        // Rows are pinned web first, then api.
        assert_eq!(visibility_rows(cx, w), ["web, shown", "api, shown"]);
        let before = details(cx, &main);
        click(cx, w, ("visibility-eye", 0usize));
        assert_eq!(header_texts(cx, w), ["REPOSITORIES · 1"]);
        assert_eq!(
            file.load().unwrap().pinned_repos,
            [PathBuf::from("/c/web")],
            "the pin is preserved"
        );
        // The selected repo keeps the details pane.
        assert_eq!(details(cx, &main), before);

        // Hidden rows are out of arrow-key navigation: down from api lands back on api.
        tab_to(cx, w, ("repo", 0usize));
        keys(cx, w, &["down"]);
        assert_eq!(
            label_of(cx, w, ("repo", 0usize)),
            "api, 5 worktrees, selected"
        );

        click(cx, w, ("visibility-eye", 0usize));
        assert_eq!(header_texts(cx, w), ["PINNED · 1", "REPOSITORIES · 2"]);
        assert_eq!(label_of(cx, w, ("repo", 0usize)), "web, 1 worktree, pinned");
    }

    #[gpui_kit::test]
    fn the_filter_narrows_rows_and_resets_on_close(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        let w = two_repos(cx, &file);

        open_visibility(cx, w);
        click(cx, w, "visibility-filter");
        cx.update_window(w, |_, window, cx| window.input("ap", cx))
            .unwrap();
        cx.run_until_parked();
        assert_eq!(visibility_rows(cx, w), ["api, shown"]);

        keys(cx, w, &["escape"]);
        open_visibility(cx, w);
        assert_eq!(visibility_rows(cx, w), ["api, shown", "web, shown"]);
    }

    #[gpui_kit::test]
    fn visibility_toggles_have_accessible_labels(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        let w = two_repos(cx, &file);
        open_visibility(cx, w);
        assert_eq!(label_of(cx, w, ("visibility-eye", 0usize)), "Hide api");
        click(cx, w, ("visibility-eye", 0usize));
        assert_eq!(label_of(cx, w, ("visibility-eye", 0usize)), "Show api");
    }

    /// A one-worktree repo at `/c/<name>`.
    fn repo_named(name: &str) -> grove_core::RepoSnapshot {
        let mut repo = canned();
        repo.display_name = name.into();
        repo.repo.name = name.into();
        repo.repo.path = format!("/c/{name}").into();
        repo.repo.git_dir = format!("/c/{name}/.git").into();
        repo.worktrees.truncate(1);
        repo.worktrees[0].path = format!("/c/{name}").into();
        repo
    }

    fn launch_repos(
        cx: &mut TestAppContext,
        file: &SettingsFile,
        repos: Vec<grove_core::RepoSnapshot>,
        window: gpui_kit::Size<gpui_kit::Pixels>,
    ) -> (AnyWindowHandle, Entity<MainWindow>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::details_view::init(cx);
            set_library(file, repos, cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: window,
                })),
                ..Default::default()
            };
            open_with(options, cx).unwrap()
        })
    }

    /// A status read for `/c/<repo>` landing with HEAD committed at `head_time`.
    fn commit_lands(cx: &mut TestAppContext, repo: &str, head_time: i64) {
        cx.update(|cx| {
            let store = &mut cx.global_mut::<Library>().store;
            let stamp = store.status_reads().stamp();
            store.apply_status(
                stamp,
                Path::new(&format!("/c/{repo}")),
                Ok(WorktreeStatus {
                    files: vec![],
                    upstream: Upstream::None,
                    head: Some(format!("{head_time:x}")),
                    head_time: Some(head_time),
                    changes_time: None,
                }),
            );
        });
        cx.run_until_parked();
    }

    fn sidebar(cx: &mut TestAppContext, w: AnyWindowHandle, n: usize) -> Vec<String> {
        (0..n).map(|ix| label_of(cx, w, ("repo", ix))).collect()
    }

    #[gpui_kit::test]
    fn opening_a_worktree_lifts_its_repo_in_recent_order(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(
            &tmp,
            Settings {
                last_selected_repo: Some("api".into()),
                ..Default::default()
            },
        );
        fake_launcher(cx, Ok(Outcome::Opened));
        let mut web = repo_named("web");
        web.worktrees[0].head_time = Some(1_700_000_000);
        let (w, _) = launch_repos(
            cx,
            &file,
            vec![repo_named("api"), web],
            size(px(1180.), px(720.)),
        );
        assert_eq!(
            sidebar(cx, w, 2),
            ["web, 1 worktree", "api, 1 worktree, selected"]
        );

        keys(cx, w, &["enter"]);
        assert!(
            file.load()
                .unwrap()
                .last_opened
                .contains_key(Path::new("/c/api"))
        );
        assert_eq!(
            sidebar(cx, w, 2),
            ["api, 1 worktree, selected", "web, 1 worktree"]
        );
    }

    #[gpui_kit::test]
    fn sort_menu_changes_order_persists_and_keeps_selection(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(
            &tmp,
            Settings {
                last_selected_repo: Some("api".into()),
                ..Default::default()
            },
        );
        fake_launcher(cx, Ok(Outcome::Opened));
        let mut web = repo_named("web");
        web.worktrees[0].head_time = Some(1_700_000_000);
        let (w, main) = launch_repos(
            cx,
            &file,
            vec![repo_named("api"), web],
            size(px(1180.), px(720.)),
        );
        let recent = ["web, 1 worktree", "api, 1 worktree, selected"];
        let by_name = ["api, 1 worktree, selected", "web, 1 worktree"];
        assert_eq!(sidebar(cx, w, 2), recent);
        assert_eq!(label_of(cx, w, "repo-sort"), "Sort repositories: Recent");

        click(cx, w, "repo-sort");
        assert!(found(cx, w, "repo-sort-menu"));
        click(cx, w, ("repo-sort-choice", 1usize));
        assert!(!found(cx, w, "repo-sort-menu"));
        assert_eq!(sidebar(cx, w, 2), by_name);
        assert_eq!(label_of(cx, w, "repo-sort"), "Sort repositories: Name");
        assert_eq!(file.load().unwrap().repo_sort, SortMode::Name);
        assert_eq!(
            main.read_with(cx, |m, cx| m.tree.read(cx).repo_name().map(str::to_string)),
            Some("api".into())
        );

        // From the keyboard: opens on the current mode; Escape closes back to the button.
        let focused = |cx: &mut TestAppContext, id: gpui_kit::ElementId| {
            cx.update_window(w, |_, window, cx| {
                window.render_frame(cx);
                window.find(id).focused() == Some(true)
            })
            .unwrap()
        };
        tab_to(cx, w, "repo-sort");
        keys(cx, w, &["enter"]);
        assert!(focused(cx, ("repo-sort-choice", 1usize).into()));
        keys(cx, w, &["escape"]);
        assert!(!found(cx, w, "repo-sort-menu"));
        assert!(focused(cx, "repo-sort".into()));
        keys(cx, w, &["enter", "up", "enter"]);
        assert!(!found(cx, w, "repo-sort-menu"));
        assert_eq!(sidebar(cx, w, 2), recent);
        assert_eq!(file.load().unwrap().repo_sort, SortMode::Recent);

        // Tab leaves the menu and closes it.
        keys(cx, w, &["enter", "tab"]);
        assert!(!found(cx, w, "repo-sort-menu"));
    }

    #[gpui_kit::test]
    fn live_reorder_keeps_selection_and_scrolls_it_into_view(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        let names: Vec<String> = (0..30).map(|i| format!("r{i:02}")).collect();
        let repos = names.iter().map(|n| repo_named(n)).collect();
        let (w, main) = launch_repos(cx, &file, repos, size(px(1180.), px(400.)));
        let in_view = |cx: &mut TestAppContext, ix: usize| {
            cx.update_window(w, |_, window, cx| {
                window.render_frame(cx);
                let list = window.find("repo-list").bounds();
                let row = window.find(("repo", ix)).bounds();
                row.top() >= list.top() && row.bottom() <= list.bottom()
            })
            .unwrap()
        };
        // Nothing saved: the first row is selected.
        assert_eq!(
            label_of(cx, w, ("repo", 0usize)),
            "r00, 1 worktree, selected"
        );
        assert!(in_view(cx, 0));
        assert!(!in_view(cx, 29), "fixture must overflow");

        for (i, name) in names.iter().enumerate().skip(1) {
            commit_lands(cx, name, 1_700_000_000 + i as i64);
        }
        assert_eq!(label_of(cx, w, ("repo", 0usize)), "r29, 1 worktree");
        assert_eq!(
            label_of(cx, w, ("repo", 29usize)),
            "r00, 1 worktree, selected"
        );
        assert!(in_view(cx, 29));
        assert_eq!(
            main.read_with(cx, |m, cx| m.tree.read(cx).repo_name().map(str::to_string)),
            Some("r00".into())
        );
    }

    /// Branch rows top to bottom.
    fn tree_branches(cx: &mut TestAppContext, main: &Entity<MainWindow>) -> Vec<String> {
        main.read_with(cx, |m, cx| {
            m.tree
                .read(cx)
                .tree()
                .rows
                .iter()
                .filter_map(|r| match &r.kind {
                    RowKind::Branch { name, .. } => Some(name.clone()),
                    _ => None,
                })
                .collect()
        })
    }

    #[gpui_kit::test]
    fn tree_sort_menu_reorders_persists_and_keeps_selection(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        // Saved from a previous run.
        let file = settings_file(
            &tmp,
            Settings {
                tree_sort: SortMode::Name,
                ..Default::default()
            },
        );
        let launches = fake_launcher(cx, Ok(Outcome::Opened));
        let mut repo = canned();
        repo.worktrees[2].head_time = Some(1_600_000_000); // feature/y
        repo.worktrees[3].head_time = Some(1_700_000_000); // fix/z
        let (w, main) = launch_repo(cx, &file, repo);
        let by_name = ["main", "feature/x", "feature/y", "fix/z"];
        let recent = ["main", "fix/z", "feature/y", "feature/x"];
        assert_eq!(tree_branches(cx, &main), by_name);
        assert_eq!(label_of(cx, w, "tree-sort"), "Sort branches: Name");

        // main, its worktree, feature/, feature/x, its worktree, feature/y.
        keys(cx, w, &["down"; 5]);
        let selected = |cx: &mut TestAppContext| {
            main.read_with(cx, |m, cx| m.tree.read(cx).selected().cloned())
        };
        let y = Some(RowId::Branch("feature/y".into()));
        assert_eq!(selected(cx), y);

        click(cx, w, "tree-sort");
        assert!(found(cx, w, "tree-sort-menu"));
        click(cx, w, ("tree-sort-choice", 0usize));
        assert!(!found(cx, w, "tree-sort-menu"));
        assert_eq!(tree_branches(cx, &main), recent);
        assert_eq!(selected(cx), y);
        assert_eq!(label_of(cx, w, "tree-sort"), "Sort branches: Recent");
        assert_eq!(file.load().unwrap().tree_sort, SortMode::Recent);
        // The repo sort is separate.
        assert_eq!(file.load().unwrap().repo_sort, SortMode::Recent);

        // From the keyboard; keys meant for the menu never reach the tree.
        tab_to(cx, w, "tree-sort");
        keys(cx, w, &["down"]);
        assert_eq!(selected(cx), y);
        keys(cx, w, &["up"]);
        assert_eq!(selected(cx), y);
        keys(cx, w, &["enter", "down", "enter"]);
        assert!(!found(cx, w, "tree-sort-menu"));
        assert_eq!(tree_branches(cx, &main), by_name);
        assert_eq!(selected(cx), y);
        assert_eq!(file.load().unwrap().tree_sort, SortMode::Name);
        keys(cx, w, &["enter", "escape"]);
        assert!(!found(cx, w, "tree-sort-menu"));
        assert_eq!(selected(cx), y);
        assert!(launches.lock().unwrap().is_empty());
    }

    #[gpui_kit::test]
    fn recent_tree_reorders_live_and_keeps_selection(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        let (w, main) = launch(cx, &file);
        assert_eq!(
            tree_branches(cx, &main),
            ["main", "feature/x", "feature/y", "fix/z"]
        );
        // main, its worktree, feature/, feature/x.
        keys(cx, w, &["down"; 3]);
        // Opening fix/z's worktree from elsewhere makes it the newest.
        cx.update(|cx| shell::record_open(Path::new("/c/wt/api-z"), cx));
        cx.run_until_parked();
        assert_eq!(
            tree_branches(cx, &main),
            ["main", "fix/z", "feature/x", "feature/y"]
        );
        assert_eq!(
            main.read_with(cx, |m, cx| m.tree.read(cx).selected().cloned()),
            Some(RowId::Branch("feature/x".into()))
        );
    }

    /// Index of the tree row for the worktree at `path`.
    fn row_of(cx: &mut TestAppContext, main: &Entity<MainWindow>, path: &Path) -> usize {
        main.read_with(cx, |m, cx| {
            m.tree
                .read(cx)
                .tree()
                .index_of(&RowId::Worktree(path.into()))
                .unwrap_or_else(|| panic!("no row for {}", path.display()))
        })
    }

    fn right_click(
        cx: &mut TestAppContext,
        w: AnyWindowHandle,
        id: impl Into<gpui_kit::ElementId>,
    ) {
        let id = id.into();
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            window.right_click(id, cx);
        })
        .unwrap();
        cx.run_until_parked();
    }

    #[gpui_kit::test]
    fn remove_is_offered_on_linked_worktrees_only(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        let mut repo = canned();
        repo.worktrees[2].locked = true; // api-y
        repo.worktrees[3].presence = Presence::Missing; // api-z
        fake_presence(cx, "/c/wt/api-z", Presence::Missing);
        let (w, main) = launch_repo(cx, &file, repo);
        let row = |cx: &mut TestAppContext, p: &str| row_of(cx, &main, Path::new(p));

        let ix = row(cx, "/c/api");
        click(cx, w, ("tree-row", ix));
        assert!(!found(cx, w, "details-remove"));
        right_click(cx, w, ("tree-row", ix));
        assert!(!found(cx, w, "tree-menu"));

        let ix = row(cx, "/c/wt/api-x");
        click(cx, w, ("tree-row", ix));
        assert_eq!(label_of(cx, w, "details-remove"), "Remove worktree…");
        assert!(!found(cx, w, "details-remove-reason"));
        right_click(cx, w, ("tree-row", ix));
        assert_eq!(label_of(cx, w, "tree-menu-remove"), "Remove worktree…");
        keys(cx, w, &["escape"]);
        assert!(!found(cx, w, "tree-menu"));
        // A click outside closes it too, handing focus back to the tree.
        right_click(cx, w, ("tree-row", ix));
        click(cx, w, "details-remove");
        assert!(!found(cx, w, "tree-menu"));
        assert!(focused(cx, w, "tree-view"));
        assert!(main.read_with(cx, |m, _| m.removal.is_none()));

        let ix = row(cx, "/c/wt/api-y");
        click(cx, w, ("tree-row", ix));
        let locked = "Can't remove: it's locked (run git worktree unlock first)";
        assert_eq!(label_of(cx, w, "details-remove-reason"), locked);
        // Disabled: clicks do nothing.
        click(cx, w, "details-remove");
        right_click(cx, w, ("tree-row", ix));
        assert_eq!(label_of(cx, w, "tree-menu-reason"), locked);
        click(cx, w, "tree-menu-remove");
        assert!(main.read_with(cx, |m, _| m.removal.is_none()));
        assert_eq!(status(cx, &main), None);
        keys(cx, w, &["escape"]);
        assert!(!found(cx, w, "tree-menu"));

        let ix = row(cx, "/c/wt/api-z");
        click(cx, w, ("tree-row", ix));
        assert_eq!(label_of(cx, w, "details-remove"), "Remove missing entry…");
        right_click(cx, w, ("tree-row", ix));
        assert_eq!(label_of(cx, w, "tree-menu-remove"), "Remove missing entry…");
    }

    /// Each removal's path and whether it was forced.
    type Removals = Arc<Mutex<Vec<(PathBuf, bool)>>>;

    /// Records each removal, then runs it for real (temp repos only) or fails with `fail`.
    fn fake_remover(cx: &mut TestAppContext, fail: Option<&'static str>) -> Removals {
        fake_remover_with(cx, fail, |_| {})
    }

    /// `fake_remover`, calling `before` with the worktree first.
    fn fake_remover_with(
        cx: &mut TestAppContext,
        fail: Option<&'static str>,
        before: impl Fn(&Path) + Send + Sync + 'static,
    ) -> Removals {
        let calls: Removals = Default::default();
        let sink = calls.clone();
        cx.update(|cx| {
            cx.set_global(removal::Remover {
                run: Arc::new(move |repo, path, force| {
                    sink.lock().unwrap().push((path.to_path_buf(), force));
                    before(path);
                    match fail {
                        Some(stderr) => Err(GitError::Failed {
                            args: "worktree remove".into(),
                            stderr: stderr.into(),
                        }),
                        None => removal::run(repo, path, force),
                    }
                }),
            })
        });
        calls
    }

    /// Real repo `api` in `tmp/code` with clean worktrees `a` and `c` and `b` holding one
    /// modified and two untracked files, under `tmp/wt`. Trees sort by name.
    fn removal_fixture(
        cx: &mut TestAppContext,
        tmp: &TempDir,
        settings: Settings,
    ) -> (AnyWindowHandle, Entity<MainWindow>, SettingsFile, PathBuf) {
        removal_fixture_with(cx, tmp, settings, |_| {})
    }

    /// `removal_fixture`, with `before_scan` given the worktrees' folder.
    fn removal_fixture_with(
        cx: &mut TestAppContext,
        tmp: &TempDir,
        settings: Settings,
        before_scan: impl FnOnce(&Path),
    ) -> (AnyWindowHandle, Entity<MainWindow>, SettingsFile, PathBuf) {
        let file = settings_file(
            tmp,
            Settings {
                tree_sort: SortMode::Name,
                ..settings
            },
        );
        fake_launcher(cx, Ok(Outcome::Opened));
        let code = tmp.path().join("code");
        let api = code.join("api");
        std::fs::create_dir_all(&api).unwrap();
        git(&api, &["init", "-q"]);
        std::fs::write(api.join("readme"), "hi\n").unwrap();
        git(&api, &["add", "readme"]);
        git(&api, &["commit", "-q", "-m", "one"]);
        let wt = tmp.path().join("wt");
        for b in ["a", "b", "c"] {
            let dir = wt.join(b);
            git(
                &api,
                &["worktree", "add", "-q", "-b", b, dir.to_str().unwrap()],
            );
        }
        let b = wt.join("b");
        std::fs::write(b.join("readme"), "changed\n").unwrap();
        std::fs::create_dir(b.join("new")).unwrap();
        std::fs::write(b.join("new/one"), "1").unwrap();
        std::fs::write(b.join("new/two"), "2").unwrap();
        before_scan(&wt);
        let repo = grove_core::scan(&code).repos.remove(0);
        let wt = repo.worktrees[1].path.parent().unwrap().to_path_buf();
        let (w, main) = launch_repo(cx, &file, repo);
        // The confirm's slide-in otherwise moves its buttons between frames.
        cx.update(|cx| cx.set_reduce_motion(true));
        (w, main, file, wt)
    }

    fn selected_row(cx: &mut TestAppContext, main: &Entity<MainWindow>) -> Option<RowId> {
        main.read_with(cx, |m, cx| m.tree.read(cx).selected().cloned())
    }

    fn focused(cx: &mut TestAppContext, w: AnyWindowHandle, id: &'static str) -> bool {
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            window.find(id).focused() == Some(true)
        })
        .unwrap()
    }

    #[gpui_kit::test]
    fn clean_remove_confirms_then_removes_without_force(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let removals = fake_remover(cx, None);
        let (w, main, file, wt) = removal_fixture(cx, &tmp, Settings::default());
        let a = wt.join("a");
        cx.update(|cx| shell::record_open(&a, cx));
        cx.run_until_parked();
        assert!(file.load().unwrap().last_opened.contains_key(&a));

        let ix = row_of(cx, &main, &a);
        click(cx, w, ("tree-row", ix));
        click(cx, w, "details-remove");
        let message = label_of(cx, w, "remove-message");
        assert!(
            message.starts_with(&format!("Remove {}?", a.display())),
            "{message}"
        );
        assert!(message.contains("Branch a is kept."), "{message}");
        assert!(!message.contains("permanently"), "{message}");
        assert_eq!(label_of(cx, w, "remove-confirm"), "Remove");
        assert!(focused(cx, w, "remove-cancel"));

        click(cx, w, "remove-confirm");
        assert_eq!(*removals.lock().unwrap(), [(a.clone(), false)]);
        assert!(!found(cx, w, "remove-message"));
        assert!(!a.exists());
        let s = status(cx, &main).unwrap();
        assert_eq!(s.text, format!("Removed {}", tilde(&a)));
        assert!(!s.error);
        // Its rows are gone; the next row (b's branch) takes the selection.
        assert!(main.read_with(cx, |m, cx| {
            m.tree
                .read(cx)
                .tree()
                .index_of(&RowId::Worktree(a.clone()))
                .is_none()
        }));
        assert_eq!(selected_row(cx, &main), Some(RowId::Branch("b".into())));
        assert!(!file.load().unwrap().last_opened.contains_key(&a));
    }

    #[gpui_kit::test]
    fn dirty_remove_counts_changes_and_forces_after_cancel_and_esc(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let removals = fake_remover(cx, None);
        let (w, main, _file, wt) = removal_fixture(cx, &tmp, Settings::default());
        let b = wt.join("b");
        let ix = row_of(cx, &main, &b);
        let open_confirm = |cx: &mut TestAppContext| {
            right_click(cx, w, ("tree-row", ix));
            click(cx, w, "tree-menu-remove");
        };

        open_confirm(cx);
        let message = label_of(cx, w, "remove-message");
        assert!(
            message.contains("1 modified and 2 untracked files will be permanently deleted"),
            "{message}"
        );
        assert_eq!(label_of(cx, w, "remove-confirm"), "Discard and remove");
        assert!(focused(cx, w, "remove-cancel"));
        // Enter on the focused Cancel, Esc, and a Cancel click each leave it be.
        keys(cx, w, &["enter"]);
        assert!(!found(cx, w, "remove-message"));
        open_confirm(cx);
        keys(cx, w, &["escape"]);
        assert!(!found(cx, w, "remove-message"));
        open_confirm(cx);
        click(cx, w, "remove-cancel");
        assert!(!found(cx, w, "remove-message"));
        assert!(removals.lock().unwrap().is_empty());
        assert!(b.join("new/one").exists());
        assert_eq!(status(cx, &main), None);

        open_confirm(cx);
        click(cx, w, "remove-confirm");
        assert_eq!(*removals.lock().unwrap(), [(b.clone(), true)]);
        assert!(!b.exists());
        // The next row is c's branch.
        assert_eq!(selected_row(cx, &main), Some(RowId::Branch("c".into())));
    }

    #[gpui_kit::test]
    fn last_row_removed_selects_the_previous(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        fake_remover(cx, None);
        let (w, main, _file, wt) = removal_fixture(cx, &tmp, Settings::default());
        let c = wt.join("c");
        let ix = row_of(cx, &main, &c);
        click(cx, w, ("tree-row", ix));
        click(cx, w, "details-remove");
        assert!(found(cx, w, "remove-message"));
        keys(cx, w, &["tab", "enter"]);
        assert!(!c.exists());
        assert_eq!(selected_row(cx, &main), Some(RowId::Worktree(wt.join("b"))));
    }

    #[gpui_kit::test]
    fn failed_remove_shows_gits_error(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let removals = fake_remover(cx, Some("fatal: 'x' is locked\n"));
        let (w, main, _file, wt) = removal_fixture(cx, &tmp, Settings::default());
        let a = wt.join("a");
        let ix = row_of(cx, &main, &a);
        click(cx, w, ("tree-row", ix));
        click(cx, w, "details-remove");
        assert!(found(cx, w, "remove-message"));
        click(cx, w, "remove-confirm");
        assert_eq!(removals.lock().unwrap().len(), 1);
        assert!(!found(cx, w, "remove-message"));
        assert!(a.exists());
        let s = status(cx, &main).unwrap();
        assert_eq!(
            s.text,
            format!("Couldn't remove {}: 'x' is locked", tilde(&a))
        );
        assert!(s.error);
        assert_eq!(selected_row(cx, &main), Some(RowId::Worktree(a)));
    }

    #[gpui_kit::test]
    fn missing_worktree_removes_only_its_entry_after_a_plain_confirm(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let removals = fake_remover(cx, None);
        let (w, main, _file, wt) = removal_fixture_with(cx, &tmp, Settings::default(), |wt| {
            std::fs::remove_dir_all(wt.join("a")).unwrap();
            std::fs::remove_dir_all(wt.join("c")).unwrap();
        });
        let c = wt.join("c");
        let ix = row_of(cx, &main, &c);
        click(cx, w, ("tree-row", ix));
        assert_eq!(label_of(cx, w, "details-remove"), "Remove missing entry…");
        click(cx, w, "details-remove");
        let message = label_of(cx, w, "remove-message");
        assert!(message.contains("is already gone"), "{message}");
        assert_eq!(label_of(cx, w, "remove-confirm"), "Remove entry");
        click(cx, w, "remove-confirm");
        assert_eq!(*removals.lock().unwrap(), [(c.clone(), false)]);
        assert_eq!(
            status(cx, &main).unwrap().text,
            format!("Removed {}", tilde(&c))
        );
        // a, also missing, keeps its entry.
        let rows = main.read_with(cx, |m, cx| {
            let tree = m.tree.read(cx).tree();
            [wt.join("a"), c.clone()].map(|p| tree.index_of(&RowId::Worktree(p)).is_some())
        });
        assert_eq!(rows, [true, false]);
    }

    #[gpui_kit::test]
    fn confirm_refreshes_instead_of_removing_when_the_worktree_changed(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let removals = fake_remover(cx, None);
        let (w, main, _file, wt) = removal_fixture(cx, &tmp, Settings::default());
        let a = wt.join("a");
        let ix = row_of(cx, &main, &a);
        click(cx, w, ("tree-row", ix));
        click(cx, w, "details-remove");
        assert_eq!(label_of(cx, w, "remove-confirm"), "Remove");

        std::fs::write(a.join("late"), "x").unwrap();
        click(cx, w, "remove-confirm");

        assert!(removals.lock().unwrap().is_empty());
        let message = label_of(cx, w, "remove-message");
        assert!(message.contains("1 untracked file will be"), "{message}");
        assert_eq!(label_of(cx, w, "remove-confirm"), "Discard and remove");
        // Back on Cancel, so a second Enter doesn't discard what it just revealed.
        assert!(focused(cx, w, "remove-cancel"));
        click(cx, w, "remove-confirm");
        assert_eq!(*removals.lock().unwrap(), [(a.clone(), true)]);
        assert!(!a.exists());
    }

    #[gpui_kit::test]
    fn worktree_dirtied_after_a_clean_confirm_is_kept(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        // Dirtied after the confirm's last look, just before git runs.
        let removals = fake_remover_with(cx, None, |path| {
            std::fs::write(path.join("late"), "x").unwrap()
        });
        let (w, main, _file, wt) = removal_fixture(cx, &tmp, Settings::default());
        let a = wt.join("a");
        let ix = row_of(cx, &main, &a);
        click(cx, w, ("tree-row", ix));
        click(cx, w, "details-remove");
        assert!(found(cx, w, "remove-message"));
        click(cx, w, "remove-confirm");

        assert_eq!(*removals.lock().unwrap(), [(a.clone(), false)]);
        assert!(!found(cx, w, "remove-message"));
        assert!(a.join("late").exists());
        let s = status(cx, &main).unwrap();
        assert!(
            s.text
                .starts_with(&format!("Couldn't remove {}: ", tilde(&a)))
                && s.text.contains("contains modified or untracked files"),
            "{}",
            s.text
        );
        assert!(s.error);
    }

    #[gpui_kit::test]
    fn submodules_refuse_removal_in_the_status_line(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let removals = fake_remover(cx, None);
        let (w, main, _file, wt) = removal_fixture(cx, &tmp, Settings::default());
        let a = wt.join("a");
        // What an initialized submodule leaves in the worktree's admin folder.
        let admin = tmp.path().join("code/api/.git/worktrees/a/modules");
        std::fs::create_dir_all(admin).unwrap();
        let ix = row_of(cx, &main, &a);
        click(cx, w, ("tree-row", ix));
        click(cx, w, "details-remove");
        assert!(!found(cx, w, "remove-message"));
        assert!(removals.lock().unwrap().is_empty());
        assert_eq!(
            status(cx, &main).unwrap().text,
            format!(
                "Can't remove {}: it contains submodules (remove it with git)",
                tilde(&a)
            )
        );
    }

    /// Each pull's worktree path.
    type Pulls = Arc<Mutex<Vec<PathBuf>>>;

    /// Records each pull, running git with `perform`.
    fn puller_with(
        cx: &mut TestAppContext,
        perform: impl Fn(&Path) -> Result<(), GitError> + Send + Sync + 'static,
    ) -> Pulls {
        let calls: Pulls = Default::default();
        let sink = calls.clone();
        cx.update(|cx| {
            cx.set_global(pull::Puller {
                run: Arc::new(move |path| {
                    sink.lock().unwrap().push(path.to_path_buf());
                    perform(path)
                }),
            })
        });
        calls
    }

    /// Records each pull that fails with `fail`'s stderr. Canned paths have no git behind
    /// them, so a pull that succeeds is a no-op; the fixture test installs the real one.
    fn fake_puller(cx: &mut TestAppContext, fail: Option<&'static str>) -> Pulls {
        puller_with(cx, move |_| match fail {
            Some(stderr) => Err(GitError::Failed {
                args: "pull --ff-only".into(),
                stderr: stderr.into(),
            }),
            None => Ok(()),
        })
    }

    /// Clicks `id` without parking the executor, so background tasks can't finish yet.
    fn click_now(cx: &mut TestAppContext, w: AnyWindowHandle, id: impl Into<gpui_kit::ElementId>) {
        let id = id.into();
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx);
        })
        .unwrap();
    }

    /// `id`'s label as rendered, without parking the executor.
    fn label_now(cx: &mut TestAppContext, w: AnyWindowHandle, id: &'static str) -> String {
        label_of(cx, w, id)
    }

    /// Clicks `id` `clicks` times and reads its label back, all before the background task
    /// a parked executor would finish may run: the pane's synchronous state is asserted.
    fn click_and_look(
        cx: &mut TestAppContext,
        w: AnyWindowHandle,
        id: &'static str,
        clicks: usize,
    ) -> String {
        for _ in 0..clicks {
            click_now(cx, w, id);
        }
        label_now(cx, w, id)
    }

    fn upstream_status(upstream: Upstream) -> Option<Result<WorktreeStatus, GitError>> {
        Some(Ok(WorktreeStatus {
            files: vec![],
            upstream,
            head: None,
            head_time: None,
            changes_time: None,
        }))
    }

    /// Canned repo with per-worktree upstreams: main behind 1, api-x up to date, api-y with
    /// no upstream, api-z gone, bisect detached.
    fn pull_repo() -> RepoSnapshot {
        let mut repo = canned();
        let set = |repo: &mut RepoSnapshot, ix: usize, upstream| {
            repo.worktrees[ix].status = upstream_status(upstream);
        };
        set(
            &mut repo,
            0,
            Upstream::Tracking {
                name: "origin/main".into(),
                ahead: 0,
                behind: 1,
            },
        );
        set(
            &mut repo,
            1,
            Upstream::Tracking {
                name: "origin/feature/x".into(),
                ahead: 0,
                behind: 0,
            },
        );
        set(&mut repo, 2, Upstream::None);
        set(
            &mut repo,
            3,
            Upstream::Gone {
                name: "origin/fix/z".into(),
            },
        );
        set(&mut repo, 4, Upstream::None);
        repo
    }

    /// The details pane's Sync line, as the status fields show it.
    fn sync_line(cx: &mut TestAppContext, main: &Entity<MainWindow>) -> String {
        main.read_with(cx, |m, cx| {
            m.details
                .read(cx)
                .status_fields()
                .into_iter()
                .find_map(|(l, v)| (l == "Sync").then_some(v))
                .unwrap()
        })
    }

    #[gpui_kit::test]
    fn pull_button_offers_per_upstream_state(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        let repo = pull_repo();
        let (w, main) = launch_repo(cx, &file, repo);
        let row = |cx: &mut TestAppContext, p: &str| row_of(cx, &main, Path::new(p));

        // main: behind 1 → Pull beside the Sync field, whose line shows the same counts.
        let ix = row(cx, "/c/api");
        click(cx, w, ("tree-row", ix));
        assert_eq!(label_of(cx, w, "details-pull"), "Pull");
        assert_eq!(sync_line(cx, &main), "1 behind");

        for (path, why) in [
            ("/c/wt/api-x", "up to date"),
            ("/c/wt/api-y", "no upstream"),
            ("/c/wt/api-z", "upstream gone"),
            ("/c/wt/bisect", "detached"),
        ] {
            let ix = row(cx, path);
            click(cx, w, ("tree-row", ix));
            assert!(!found(cx, w, "details-pull"), "Pull offered for {why}");
        }
    }

    #[gpui_kit::test]
    fn failed_fetch_counts_dont_block_the_button(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        let mut repo = pull_repo();
        repo.fetch_error = Some(GitError::Failed {
            args: "fetch --all --prune --quiet".into(),
            stderr: "fatal: unable to access: offline\n".into(),
        });
        let (w, main) = launch_repo(cx, &file, repo);

        let ix = row_of(cx, &main, Path::new("/c/api"));
        click(cx, w, ("tree-row", ix));
        assert_eq!(label_of(cx, w, "details-pull"), "Pull");
    }

    #[gpui_kit::test]
    fn pull_click_runs_once_and_posts_outcome(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        let pulls = fake_puller(cx, None);
        let (w, main) = launch_repo(cx, &file, pull_repo());
        let ix = row_of(cx, &main, Path::new("/c/api"));
        click(cx, w, ("tree-row", ix));
        // Runs in the background (“Pulling…”), disabled; a second click is refused.
        assert_eq!(click_and_look(cx, w, "details-pull", 2), "Pulling…");

        cx.run_until_parked();
        assert_eq!(*pulls.lock().unwrap(), [PathBuf::from("/c/api")]);
        assert_eq!(status(cx, &main).unwrap().text, "Pulled main");
        assert!(!status(cx, &main).unwrap().error);
    }

    /// Records each pull and runs git for real.
    fn real_puller(cx: &mut TestAppContext) -> Pulls {
        puller_with(cx, pull::run)
    }

    #[gpui_kit::test]
    fn pulled_worktree_rereads_its_status_badge_and_commits(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let pulls = real_puller(cx);
        // Bare origin plus a teammate clone, as fetch's tests set up: a `feature` worktree
        // one behind its upstream.
        let remotes = tmp.path().join("remotes");
        std::fs::create_dir_all(&remotes).unwrap();
        let origin = remotes.join("origin.git");
        git(&remotes, &["init", "-q", "--bare", "origin.git"]);
        let api = tmp.path().join("code/api");
        std::fs::create_dir_all(&api).unwrap();
        git(&api, &["init", "-q"]);
        std::fs::write(api.join("f.txt"), "base\n").unwrap();
        git(&api, &["add", "-A"]);
        git(&api, &["commit", "-q", "-m", "base"]);
        git(&api, &["remote", "add", "origin", origin.to_str().unwrap()]);
        git(&api, &["push", "-q", "-u", "origin", "main"]);
        let wt = tmp.path().join("wt/api-feature");
        git(
            &api,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feature",
                wt.to_str().unwrap(),
            ],
        );
        // The scan reports canonical paths; keep ours comparable.
        let wt = wt.canonicalize().unwrap();
        git(&wt, &["push", "-q", "-u", "origin", "feature"]);
        let other = tmp.path().join("other");
        git(
            tmp.path(),
            &["clone", "-q", origin.to_str().unwrap(), "other"],
        );
        git(&other, &["checkout", "-q", "feature"]);
        git(&other, &["commit", "-q", "--allow-empty", "-m", "theirs"]);
        git(&other, &["push", "-q"]);
        // Test setup: refresh the tracking refs so the scan shows the worktree behind.
        git(&wt, &["fetch", "-q"]);
        let mut repo = grove_core::scan(&tmp.path().join("code")).repos.remove(0);
        // Test setup: the scan leaves statuses unread; the app fills them in the background.
        let entry = repo.worktrees.iter_mut().find(|w| w.path == wt).unwrap();
        entry.status = Some(grove_core::read_worktree_status(&wt));
        let (w, main) = launch_repo(cx, &file, repo);
        let ix = row_of(cx, &main, &wt);
        click(cx, w, ("tree-row", ix));
        assert_eq!(sync_line(cx, &main), "1 behind");
        assert_eq!(row_badges(cx, &main, wt.to_str().unwrap()), ["\u{2193}1"]);

        // Runs in the background (“Pulling…”), disabled; a second click is refused.
        assert_eq!(click_and_look(cx, w, "details-pull", 2), "Pulling…");
        cx.run_until_parked();

        assert_eq!(*pulls.lock().unwrap(), vec![wt.clone()]);
        assert_eq!(status(cx, &main).unwrap().text, "Pulled feature");
        assert!(!status(cx, &main).unwrap().error);
        // The reread after success leaves the worktree up to date: the badge clears, the
        // sync line agrees, and the button is gone (nothing left to pull).
        assert_eq!(
            row_badges(cx, &main, wt.to_str().unwrap()),
            Vec::<String>::new()
        );
        assert_eq!(sync_line(cx, &main), "up to date");
        assert!(!found(cx, w, "details-pull"));
        // Recent commits re-read on their own, as HEAD changed.
        let commits = main
            .read_with(cx, |m, cx| m.details.read(cx).commits().cloned())
            .unwrap();
        match commits {
            Ok(commits) => assert_eq!(commits[0].subject, "theirs"),
            err => panic!("expected commits, got {err:?}"),
        }
    }

    #[gpui_kit::test]
    fn selection_change_during_pull_doesnt_cancel_it(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        let pulls = fake_puller(cx, None);
        let mut repo = pull_repo();
        // A second pullable worktree, whose button must refuse while one runs.
        repo.worktrees[3].status = upstream_status(Upstream::Tracking {
            name: "origin/fix/z".into(),
            ahead: 0,
            behind: 2,
        });
        let (w, main) = launch_repo(cx, &file, repo);
        let ix = row_of(cx, &main, Path::new("/c/api"));
        click(cx, w, ("tree-row", ix));
        // Start the pull and hold it in flight: nothing parked yet.
        assert_eq!(click_and_look(cx, w, "details-pull", 2), "Pulling…");

        // Moving the selection away doesn't cancel it or start another: the button on the
        // also-behind worktree is there but refuses the click.
        let other = row_of(cx, &main, Path::new("/c/wt/api-z"));
        click_now(cx, w, ("tree-row", other));
        assert_eq!(click_and_look(cx, w, "details-pull", 2), "Pull");

        cx.run_until_parked();
        // The first pull's outcome still lands, and nothing else was pulled.
        assert_eq!(*pulls.lock().unwrap(), [PathBuf::from("/c/api")]);
        assert_eq!(status(cx, &main).unwrap().text, "Pulled main");

        // With no pull running, the still-behind worktree pulls once, its own second
        // click refused while it runs.
        click_now(cx, w, "details-pull");
        assert_eq!(click_and_look(cx, w, "details-pull", 2), "Pulling…");
        cx.run_until_parked();
        assert_eq!(
            *pulls.lock().unwrap(),
            [PathBuf::from("/c/api"), PathBuf::from("/c/wt/api-z")]
        );
        assert_eq!(status(cx, &main).unwrap().text, "Pulled fix/z");
    }

    #[gpui_kit::test]
    fn failed_pull_shows_gits_brief_error_and_changes_nothing(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        let pulls = fake_puller(cx, Some("fatal: Not possible to fast-forward, aborting.\n"));
        let (w, main) = launch_repo(cx, &file, pull_repo());
        let ix = row_of(cx, &main, Path::new("/c/api"));
        click(cx, w, ("tree-row", ix));

        click(cx, w, "details-pull");

        assert_eq!(pulls.lock().unwrap().len(), 1);
        let s = status(cx, &main).unwrap();
        assert_eq!(
            s.text,
            "Couldn't pull main: Not possible to fast-forward, aborting."
        );
        assert!(s.error);
        // Nothing changed: the counts are as they were and the button is back.
        assert_eq!(sync_line(cx, &main), "1 behind");
        assert_eq!(label_of(cx, w, "details-pull"), "Pull");
    }
}
