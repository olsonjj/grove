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

#[cfg(test)]
use crate::removal;
#[cfg(test)]
use gpui_kit::{AnyWindowHandle, Bounds, Pixels, WindowBounds, WindowOptions, px, size};
#[cfg(test)]
use grove_core::{GitError, RepoSnapshot};
#[cfg(test)]
use layout::{
    DETAILS_PANE_MAX_WIDTH, DETAILS_PANE_MIN_WIDTH, DETAILS_PANE_WIDTH, SIDEBAR_W, TREE_MIN_WIDTH,
};
#[cfg(test)]
use sidebar::rows::fetch_label;
#[cfg(test)]
use std::path::Path;
#[cfg(test)]
use window::{MIN_SIZE, open_with};

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
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{
        InputEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Point,
        TestAppContext, point,
    };
    use grove_core::{
        Change, ChangedFile, Fallback, LaunchError, Outcome, Permission, Presence, RowId, RowKind,
        Settings, SettingsFile, Snapshot, SnapshotStore, SortMode, TerminalOpensIn, Upstream,
        WorktreeStatus, relative_time,
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

    /// Resizes the details pane (panel 1) through the shared panel state, then lets the
    /// frame -- and, via `Resized`, the delayed settings write -- land.
    fn resize_pane(
        cx: &mut TestAppContext,
        main: &Entity<MainWindow>,
        w: AnyWindowHandle,
        width: f32,
    ) {
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            let panes = main.read(cx).panes.clone();
            panes.update(cx, |state, cx| state.resize_panel(1, px(width), window, cx))
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(w, |_, window, cx| window.render_frame(cx))
            .unwrap();
    }

    /// Drags the tree/pane divider by `dx` px (positive = rightwards) at mouse rate.
    fn drag_divider(cx: &mut TestAppContext, w: AnyWindowHandle, dx: f32) {
        cx.update_window(w, |_, window, cx| {
            // The handle straddles the pane's left edge; the element itself is
            // not observed, so start the drag at that edge (inside the band).
            let pane = window.find("details").bounds();
            let from = pane.origin;
            let to = Point {
                x: from.x - px(dx),
                y: from.y + pane.size.height / 2.,
            };
            window.drag(from, to, cx);
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn pane_width(cx: &mut TestAppContext, w: AnyWindowHandle) -> f32 {
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            let b = window.find("details").bounds();
            (b.right() - b.left()).as_f32()
        })
        .unwrap()
    }

    /// The tree panel's width: from the sidebar's edge (where the
    /// group starts) to the pane divider.
    fn tree_panel_width(cx: &mut TestAppContext, w: AnyWindowHandle) -> f32 {
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            let pane = window.find("details").bounds();
            pane.left().as_f32() - SIDEBAR_W
        })
        .unwrap()
    }

    fn assert_close(got: Pixels, want: f32, what: &str) {
        assert!(
            (got.as_f32() - want).abs() < 1.,
            "{what}: expected {want} px, got {} px",
            got.as_f32()
        );
    }

    #[gpui_kit::test]
    fn resizable_pane_starts_at_320_and_sidebar_stays_put(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let (w, _) = launch(cx, &file);
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            let details = window.find("details").bounds();
            assert_close(details.right() - details.left(), 320., "fresh pane width");
            // The repo sidebar keeps today's exact footprint:
            // fixed column, flush to the left edge.
            let sidebar = window.find("repo-list").bounds();
            assert_eq!(sidebar.left().as_f32(), 0.);
            assert_close(sidebar.right(), SIDEBAR_W - 1., "sidebar right edge");
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn resizable_drag_trades_width_and_saves_on_release(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        // Wide enough for a 150 px drag not to reach the tree's own minimum.
        let (w, _) = launch_at(cx, &file, canned(), size(px(1500.), px(720.)));

        // Dragging left widens the pane at the tree's expense.
        drag_divider(cx, w, 150.);
        let pane = cx
            .update_window(w, |_, window, cx| {
                window.render_frame(cx);
                window.find("details").bounds()
            })
            .unwrap();
        assert_close(pane.right() - pane.left(), 470., "pane after drag");
        // The tree flexes to exactly what the pane took:
        // pane gained 150 px, tree lost them. Expected tree width comes
        // from the launch above: window width minus sidebar minus pane.
        let tree_after = tree_panel_width(cx, w);
        let tree_before = 1500. - SIDEBAR_W - DETAILS_PANE_WIDTH;
        assert_close(px(tree_after), tree_before - 150., "tree after drag");
        // The write happens once, at drag end, through the saved settings file.
        assert_eq!(
            file.load().unwrap().details_pane_width,
            Some(pane_width(cx, w).round() as i32)
        );
    }

    #[gpui_kit::test]
    fn resizable_clamps_pane_to_260_and_720(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        // Wide enough that the pane's own max (720) and the tree's min (480) can
        // both bind, without one masking the other.
        let (w, _) = launch_at(cx, &file, canned(), size(px(1560.), px(720.)));

        // Far past either stop: the drag lands exactly on the clamp.
        drag_divider(cx, w, 1000.);
        assert_eq!(pane_width(cx, w).round() as i64, 720);
        drag_divider(cx, w, -2000.);
        assert_eq!(pane_width(cx, w).round() as i64, 260);
        // The tree keeps its own minimum in both cases, and the pane's buttons
        // render inside its bounds even at the 260 px minimum (story: nothing
        // overflows or overlaps at any width in the clamp range).
        assert!(tree_panel_width(cx, w) >= 480.);
        let pane = cx
            .update_window(w, |_, window, _| window.find("details").bounds())
            .unwrap();
        for what in [
            "details-open-editor",
            "details-open-terminal",
            "details-copy-path",
        ] {
            let button = cx
                .update_window(w, |_, window, _| window.find(what).bounds())
                .unwrap();
            assert!(
                button.left() >= pane.left() && button.right() <= pane.right(),
                "{what} must sit inside the clamped pane"
            );
        }
    }

    #[gpui_kit::test]
    fn resizable_width_survives_a_selection_change(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let (w, main) = launch_at(cx, &file, canned(), size(px(1500.), px(720.)));
        resize_pane(cx, &main, w, 470.);

        keys(cx, w, &["down"]);
        assert!((pane_width(cx, w) - 470.).abs() < 1.);
    }

    /// Presses the divider, moves the pointer in steps, leaving the drag open
    /// (`released: false`): the events a real drag delivers, split so the middle
    /// can be observed. A press that never moved is no-op for the pane.
    fn pressed_divider_drag(
        cx: &mut TestAppContext,
        w: AnyWindowHandle,
        from: Point<Pixels>,
        to: Point<Pixels>,
    ) {
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            window.dispatch_event(
                MouseMoveEvent {
                    position: from,
                    pressed_button: None,
                    modifiers: Default::default(),
                }
                .to_platform_input(),
                cx,
            );
            window.dispatch_event(
                MouseDownEvent {
                    button: MouseButton::Left,
                    position: from,
                    modifiers: Default::default(),
                    click_count: 1,
                    first_mouse: false,
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
            for step in 1..=8 {
                let fraction = step as f32 / 8.;
                window.dispatch_event(
                    MouseMoveEvent {
                        position: point(
                            from.x + (to.x - from.x) * fraction,
                            from.y + (to.y - from.y) * fraction,
                        ),
                        pressed_button: Some(MouseButton::Left),
                        modifiers: Default::default(),
                    }
                    .to_platform_input(),
                    cx,
                );
                window.render_frame(cx);
            }
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn pane_saves_when_the_drag_ends_not_while_it_moves(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let (w, _) = launch_at(cx, &file, canned(), size(px(1560.), px(720.)));
        let divider = cx
            .update_window(w, |_, window, _| window.find("details").bounds())
            .unwrap()
            .origin;
        let to = point(divider.x - px(150.), divider.y);

        // Press, move, and leave the drag open: no save may have landed while
        // the pointer moves (settings writes happen on background threads, so
        // parking first would flush a mid-move save if one existed).
        pressed_divider_drag(cx, w, divider, to);
        cx.run_until_parked();
        assert_eq!(file.load().unwrap().details_pane_width, None);
        assert_eq!(
            pane_width(cx, w).round() as i64,
            470,
            "pane grew while dragging"
        );

        // Release the still-open drag: the single write carries the final width.
        cx.update_window(w, |_, window, cx| {
            window.dispatch_event(
                MouseUpEvent {
                    button: MouseButton::Left,
                    position: to,
                    modifiers: Default::default(),
                    click_count: 1,
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(file.load().unwrap().details_pane_width, Some(470));
        assert_eq!(pane_width(cx, w).round() as i64, 470);
    }

    #[gpui_kit::test]
    fn resizable_width_is_restored_on_relaunch(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let (w, main) = launch_at(cx, &file, canned(), size(px(1500.), px(720.)));
        resize_pane(cx, &main, w, 470.);
        assert_eq!(file.load().unwrap().details_pane_width, Some(470));

        cx.update(|cx| cx.remove_global::<Library>());
        // Same window size as the first launch, so the saved width fits.
        let (w, _) = launch_at(cx, &file, canned(), size(px(1500.), px(720.)));
        assert!((pane_width(cx, w) - 470.).abs() < 1.);
    }

    #[gpui_kit::test]
    fn saved_max_width_compressed_keep_tree_at_its_minimum(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        // Saved at the pane's maximum, in the wide window it was earned in.
        let file = settings_file(
            &tmp,
            Settings {
                details_pane_width: Some(DETAILS_PANE_MAX_WIDTH as i32),
                ..Settings::default()
            },
        );
        // The app's real default window (1180 px) is too narrow to honor the
        // saved 720: the tree holds its 480 minimum and the pane absorbs the
        // rest rather than overflowing the window.
        let (w, _) = launch_at(cx, &file, canned(), size(px(1180.), px(720.)));
        assert_close(
            px(pane_width(cx, w)),
            1180. - SIDEBAR_W - TREE_MIN_WIDTH,
            "pane at launch",
        );
        assert!(
            tree_panel_width(cx, w) >= TREE_MIN_WIDTH,
            "tree kept its minimum"
        );
        assert!(
            pane_width(cx, w) >= DETAILS_PANE_MIN_WIDTH,
            "pane kept its minimum"
        );
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
        cx.run_until_parked();
        let fields = main.read_with(cx, |m, cx| m.details.read(cx).status_fields());
        assert_eq!(
            fields,
            [
                ("Upstream", "origin/main".into()),
                ("Sync", "2 ahead, 1 behind".into()),
                ("Working tree", "2 changed files".into()),
                ("Size", "0 B".into()),
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
            git(
                &repo_dir,
                &[
                    "-c",
                    "user.name=olsonjj",
                    "commit",
                    "-q",
                    "--allow-empty",
                    "-m",
                    m,
                ],
            );
        }
        let mut repo = canned();
        repo.worktrees[0].path = repo_dir;
        let (w, main) = launch_repo(cx, &file, repo);
        cx.run_until_parked();
        assert!((0..5).all(|i| has(cx, w, "commit", i)));
        assert!(!has(cx, w, "commit", 5));
        // Two-line rows: each commit's unit (its preserved `commit` id) names who wrote
        // it and when, per the pane's own read.
        let commits = main
            .read_with(cx, |m, cx| m.details.read(cx).commits().cloned())
            .unwrap()
            .unwrap();
        let now = grove_core::unix_now();
        for (i, c) in commits.iter().enumerate() {
            let label = cx
                .update_window(w, |_, window, cx| {
                    window.render_frame(cx);
                    window.find(("commit", i)).label().unwrap().to_string()
                })
                .unwrap();
            // The row reads as one unit (VoiceOver): hash, author, time, subject.
            assert!(label.contains(c.short_sha.as_str()), "{label}");
            assert!(label.contains(c.author.as_str()), "{label}");
            assert!(
                label.contains(relative_time(c.time, now).as_str()),
                "{label}"
            );
            assert!(label.contains(c.subject.as_str()), "{label}");
            // And the visible leaves themselves carry what they render: the metadata
            // line's author and time, not just the row's composed label.
            let (author, time_text) = cx
                .update_window(w, |_, window, cx| {
                    window.render_frame(cx);
                    (
                        window
                            .find(("commit-author", i))
                            .label()
                            .unwrap_or_default()
                            .to_string(),
                        window.find(("commit-time", i)).label().unwrap().to_string(),
                    )
                })
                .unwrap();
            assert_eq!(author, "olsonjj");
            assert_eq!(time_text, relative_time(c.time, now));
        }
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
    fn details_show_the_worktree_size(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        let repo_dir = tmp.path().join("api");
        std::fs::create_dir(&repo_dir).unwrap();
        git(&repo_dir, &["init", "-q"]);
        // Three kilobytes across a nested folder, committed so the tree is clean: "3 kB".
        std::fs::create_dir(repo_dir.join("d")).unwrap();
        std::fs::write(repo_dir.join("d/plain.bin"), vec![0u8; 3072]).unwrap();
        git(&repo_dir, &["add", "-A"]);
        git(&repo_dir, &["commit", "-q", "-m", "one"]);
        let mut repo = grove_core::scan(tmp.path()).repos.remove(0);
        repo.worktrees[0].status = grove_core::read_worktree_status(&repo.worktrees[0].path)
            .ok()
            .map(Ok);
        let (_w, main) = launch_repo(cx, &file, repo);
        cx.run_until_parked();
        let fields = main.read_with(cx, |m, cx| m.details.read(cx).status_fields());
        assert_eq!(
            fields,
            [
                ("Upstream", "none".into()),
                ("Sync", "no upstream".into()),
                ("Working tree", "clean".into()),
                ("Size", "3 kB".into()),
            ]
        );
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

    // Branch switch trigger (ticket 25).

    /// Each switch's (worktree path, target branch).
    type Switches = Arc<Mutex<Vec<(PathBuf, String)>>>;

    /// Records each switch, running git with `perform`.
    fn switcher_with(
        cx: &mut TestAppContext,
        perform: impl Fn(&Path, &str) -> Result<(), GitError> + Send + Sync + 'static,
    ) -> Switches {
        let calls: Switches = Default::default();
        let sink = calls.clone();
        cx.update(|cx| {
            cx.set_global(crate::branch::SwitchRunner {
                run: Arc::new(move |path, target| {
                    sink.lock()
                        .unwrap()
                        .push((path.to_path_buf(), target.to_string()));
                    perform(path, target)
                }),
            })
        });
        calls
    }

    /// Records each switch that fails with `fail`'s stderr.
    fn fake_switcher(cx: &mut TestAppContext, fail: Option<&'static str>) -> Switches {
        switcher_with(cx, move |_, _| match fail {
            Some(stderr) => Err(GitError::Failed {
                args: "switch --quiet".into(),
                stderr: stderr.into(),
            }),
            None => Ok(()),
        })
    }

    /// Fakes the branch-list read with `list`.
    fn lister(cx: &mut TestAppContext, list: &'static [&'static str]) {
        cx.update(|cx| {
            cx.set_global(crate::branch::ListRunner {
                run: Arc::new(move |_| Ok(list.iter().map(|branch| branch.to_string()).collect())),
            })
        });
    }

    /// Fakes the branch-list read with a failed read carrying `stderr`.
    fn failing_lister(cx: &mut TestAppContext, stderr: &'static str) {
        cx.update(|cx| {
            cx.set_global(crate::branch::ListRunner {
                run: Arc::new(move |_| {
                    Err(GitError::Failed {
                        args: "for-each-ref refs/heads".into(),
                        stderr: stderr.into(),
                    })
                }),
            })
        });
    }

    fn select_worktree(
        cx: &mut TestAppContext,
        w: AnyWindowHandle,
        main: &Entity<MainWindow>,
        path: &str,
    ) {
        let ix = row_of(cx, main, Path::new(path));
        click(cx, w, ("tree-row", ix));
        cx.run_until_parked();
    }

    /// The switch write in flight: which worktree, to what branch.
    fn switching(cx: &mut TestAppContext, main: &Entity<MainWindow>) -> Option<(PathBuf, String)> {
        main.read_with(cx, |m, cx| {
            m.details
                .read(cx)
                .switching()
                .map(|(p, t)| (p.to_path_buf(), t.to_string()))
        })
    }

    /// What the trigger reads: the committed branch, else the placeholder.
    fn trigger_text(cx: &mut TestAppContext, main: &Entity<MainWindow>) -> Option<String> {
        main.read_with(cx, |m, cx| m.details.read(cx).branch_trigger_text(cx))
    }

    /// The dropdown's rows in order, as their accessible labels.
    fn dropdown(cx: &mut TestAppContext, w: AnyWindowHandle) -> Vec<String> {
        indexed_labels(cx, w, "branch-item")
    }

    #[gpui_kit::test]
    fn branch_trigger_shows_the_worktree_branch_on_main_and_linked(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        lister(cx, &["feature/x", "fix/z", "main", "spare"]);
        let (w, main) = launch_repo(cx, &file, canned());

        // Main worktree, then linked: the branch line sits at the top of the pane for
        // every worktree, with each worktree's own branch as the value.
        select_worktree(cx, w, &main, "/c/api");
        assert!(found(cx, w, "details-branch"));
        assert_eq!(
            label_of(cx, w, "details-branch"),
            "Switch the worktree's branch"
        );
        assert_eq!(trigger_text(cx, &main).as_deref(), Some("main"));
        // The branch line is the trigger now, so the STATUS section leaves it out.
        let fields = main.read_with(cx, |m, cx| m.details.read(cx).status_fields());
        assert_eq!(fields, [("Working tree", "reading…".to_string())]);

        select_worktree(cx, w, &main, "/c/wt/api-x");
        assert_eq!(trigger_text(cx, &main).as_deref(), Some("feature/x"));
    }

    #[gpui_kit::test]
    fn branch_dropdown_lists_read_branches_alphabetically_and_searchable(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        lister(cx, &["spare", "fix/z", "feature/x", "main"]);
        let (w, main) = launch_repo(cx, &file, canned());
        select_worktree(cx, w, &main, "/c/api");
        assert_eq!(trigger_text(cx, &main).as_deref(), Some("main"));

        click(cx, w, "details-branch");

        // Alphabetical; the branches the repo's other worktrees hold are annotated.
        assert_eq!(
            dropdown(cx, w),
            [
                "feature/x, checked out in worktree api-x/",
                "fix/z, checked out in worktree api-z/",
                "main",
                "spare",
            ]
        );

        // The search narrows the list.
        cx.update_window(w, |_, window, cx| window.input("sp", cx))
            .unwrap();
        assert_eq!(dropdown(cx, w), ["spare"]);
        cx.update_window(w, |_, window, cx| window.press("escape", cx))
            .unwrap();
    }

    #[gpui_kit::test]
    fn branch_held_by_another_worktree_is_annotated_and_refused(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        lister(cx, &["main", "feature/x", "fix/z"]);
        let switches = fake_switcher(cx, None);
        let (w, main) = launch_repo(cx, &file, canned());

        // The pane shows the linked worktree on feature/x; the repo's main worktree
        // holds main, so that pick is impossible and starts no write.
        select_worktree(cx, w, &main, "/c/wt/api-x");
        click(cx, w, "details-branch");
        assert_eq!(
            dropdown(cx, w),
            [
                "feature/x",
                "fix/z, checked out in worktree api-z/",
                "main, checked out in worktree api/",
            ]
        );

        click(cx, w, ("branch-item", 2usize));

        assert!(switches.lock().unwrap().is_empty());
        // The trigger keeps the worktree's own branch.
        assert_eq!(trigger_text(cx, &main).as_deref(), Some("feature/x"));
    }

    #[gpui_kit::test]
    fn detached_worktree_shows_the_placeholder_and_picks_a_branch(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        lister(cx, &["main", "feature/x", "next"]);
        let switches = fake_switcher(cx, None);
        let (w, main) = launch_repo(cx, &file, canned());
        select_worktree(cx, w, &main, "/c/wt/bisect");
        // A detached worktree keeps its identity line and shows the placeholder; the
        // STATUS row still says where its detached HEAD is.
        let fields = main.read_with(cx, |m, cx| m.details.read(cx).status_fields());
        assert!(fields.contains(&("Branch", "detached @ a1f3c9e".to_string())));
        assert_eq!(trigger_text(cx, &main).as_deref(), Some("No branch"));

        click(cx, w, "details-branch");
        // The repo's held branches are annotated; only one pick remains possible.
        assert_eq!(
            dropdown(cx, w),
            [
                "feature/x, checked out in worktree api-x/",
                "main, checked out in worktree api/",
                "next",
            ]
        );
        click(cx, w, ("branch-item", 2usize));
        cx.run_until_parked();

        assert_eq!(
            *switches.lock().unwrap(),
            [(PathBuf::from("/c/wt/bisect"), "next".into())]
        );
        let s = status(cx, &main).unwrap();
        assert_eq!(s.text, "Switched to `next`");
    }

    #[gpui_kit::test]
    fn branch_trigger_refuses_a_second_open_while_a_switch_runs(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        lister(cx, &["feature/x", "main", "spare"]);
        let switches = fake_switcher(cx, None);
        let (w, main) = launch_repo(cx, &file, canned());
        select_worktree(cx, w, &main, "/c/api");

        let m = main.clone();
        cx.update_window(w, |_, window, cx| {
            m.update(cx, |m, cx| {
                m.details.update(cx, |d, cx| {
                    d.hold_switch(
                        PathBuf::from("/c/api"),
                        "spare".into(),
                        PathBuf::from("/c/api/.git"),
                        window,
                        cx,
                    );
                });
            });
        })
        .unwrap();
        cx.update_window(w, |_, window, cx| window.render_frame(cx))
            .unwrap();

        // The trigger reads "Switching…", has no committed value, and the dropdown
        // refuses to open while the write runs; no second write starts either way.
        assert_eq!(trigger_text(cx, &main).as_deref(), Some("Switching…"));
        assert_eq!(
            switching(cx, &main),
            Some((PathBuf::from("/c/api"), "spare".into()))
        );
        click(cx, w, "details-branch");
        assert!(dropdown(cx, w).is_empty(), "the dropdown must not reopen");
        assert!(switches.lock().unwrap().is_empty());
    }

    #[gpui_kit::test]
    fn selection_change_during_a_switch_doesnt_cancel_it(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        lister(cx, &["feature/x", "main", "spare"]);
        let switches = fake_switcher(cx, Some("fatal: cannot switch because changes\n"));
        let (w, main) = launch_repo(cx, &file, canned());
        select_worktree(cx, w, &main, "/c/api");

        click(cx, w, "details-branch");
        click(cx, w, ("branch-item", 2usize));

        // Browsing while the write runs: the new row's own branch shows in the trigger,
        // and the running switch is neither cancelled nor repeated.
        let other = row_of(cx, &main, Path::new("/c/wt/api-x"));
        click(cx, w, ("tree-row", other));
        assert_eq!(trigger_text(cx, &main).as_deref(), Some("feature/x"));

        cx.run_until_parked();
        // The outcome notice still lands, exactly once, tied to the switched worktree.
        let s = status(cx, &main).unwrap();
        assert_eq!(
            s.text,
            "Couldn't switch to `spare`: cannot switch because changes"
        );
        assert!(s.error);
        assert_eq!(
            *switches.lock().unwrap(),
            [(PathBuf::from("/c/api"), "spare".into())]
        );
        assert_eq!(trigger_text(cx, &main).as_deref(), Some("feature/x"));
    }

    #[gpui_kit::test]
    fn failed_switch_shows_gits_brief_error_and_restores_the_trigger(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        lister(cx, &["main", "spare"]);
        let switches = fake_switcher(cx, Some("fatal: cannot switch because changes\n"));
        let (w, main) = launch_repo(cx, &file, canned());
        select_worktree(cx, w, &main, "/c/api");

        click(cx, w, "details-branch");
        click(cx, w, ("branch-item", 1usize));

        cx.run_until_parked();
        let s = status(cx, &main).unwrap();
        assert_eq!(
            s.text,
            "Couldn't switch to `spare`: cannot switch because changes"
        );
        assert!(s.error);
        // Nothing changed: the trigger is back on the worktree's branch.
        assert_eq!(trigger_text(cx, &main).as_deref(), Some("main"));
        assert_eq!(switches.lock().unwrap().len(), 1);
    }

    #[gpui_kit::test]
    fn picking_the_current_branch_puts_the_trigger_back_on_the_branch(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        lister(cx, &["feature/x", "main", "spare"]);
        let switches = fake_switcher(cx, None);
        let (w, main) = launch_repo(cx, &file, canned());
        select_worktree(cx, w, &main, "/c/api");

        // `main` is the worktree's own branch: git calls the pick a no-op and Grove
        // reports it like any switch.
        click(cx, w, "details-branch");
        click(cx, w, ("branch-item", 1usize));
        cx.run_until_parked();

        let s = status(cx, &main).unwrap();
        assert_eq!(s.text, "Switched to `main`");
        assert_eq!(
            *switches.lock().unwrap(),
            [(PathBuf::from("/c/api"), "main".into())]
        );
        // The value cleared for the run comes back after the success: the trigger reads
        // the branch, not the placeholder.
        assert_eq!(trigger_text(cx, &main).as_deref(), Some("main"));
    }

    #[gpui_kit::test]
    fn a_running_pull_blocks_the_switcher_until_it_finishes(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        lister(cx, &["feature/x", "main", "spare"]);
        fake_puller(cx, None);
        let switches = fake_switcher(cx, None);
        let (w, main) = launch_repo(cx, &file, pull_repo());
        select_worktree(cx, w, &main, "/c/api");

        // The pull starts and stays in flight (the executor isn't parked).
        click_now(cx, w, "details-pull");
        assert_eq!(label_of(cx, w, "details-pull"), "Pulling…");

        // The branch trigger is disabled while the pull runs: the dropdown stays shut
        // and no switch starts, so pull's fast-forward can't collide with a checkout.
        click_now(cx, w, "details-branch");
        assert!(dropdown(cx, w).is_empty());
        assert!(switches.lock().unwrap().is_empty());

        cx.run_until_parked();
        let s = status(cx, &main).unwrap();
        assert_eq!(s.text, "Pulled main");
        // The pull done, the switcher is back; no switch ever started.
        click(cx, w, "details-branch");
        assert!(!dropdown(cx, w).is_empty());
        assert!(switches.lock().unwrap().is_empty());
    }

    #[gpui_kit::test]
    fn a_running_switch_blocks_pull(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        lister(cx, &["feature/x", "main", "spare"]);
        let pulls = fake_puller(cx, None);
        let (w, main) = launch_repo(cx, &file, pull_repo());
        select_worktree(cx, w, &main, "/c/api");

        // A switch is held in flight, exactly as the pick handler leaves it.
        let m = main.clone();
        cx.update_window(w, |_, window, cx| {
            m.update(cx, |m, cx| {
                m.details.update(cx, |d, cx| {
                    d.hold_switch(
                        PathBuf::from("/c/api"),
                        "spare".into(),
                        PathBuf::from("/c/api/.git"),
                        window,
                        cx,
                    );
                });
            });
        })
        .unwrap();
        cx.update_window(w, |_, window, cx| window.render_frame(cx))
            .unwrap();

        // Pull waits: the button reads Pull, stays disabled, and the click starts no
        // pull — two writes never race on one worktree.
        assert_eq!(label_of(cx, w, "details-pull"), "Pull");
        click_now(cx, w, "details-pull");
        assert!(pulls.lock().unwrap().is_empty());
    }

    /// A failed branch-list read keeps the worktree's own branch as the value and names
    /// the failure in the dropdown, instead of reading as an empty repository.
    #[gpui_kit::test]
    fn failed_branch_read_keeps_the_branch_and_names_the_failure(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        failing_lister(cx, "fatal: not a git repository\n");
        let (w, main) = launch_repo(cx, &file, canned());

        select_worktree(cx, w, &main, "/c/api");
        assert_eq!(trigger_text(cx, &main).as_deref(), Some("main"));
        click(cx, w, "details-branch");
        // The one branch the pane knows is still offered, and it is the value.
        assert_eq!(dropdown(cx, w), ["main"]);
        cx.update_window(w, |_, window, cx| window.press("escape", cx))
            .unwrap();

        // A worktree with no branch of its own gets the read's failure as its empty row,
        // not the same words an empty repository would say.
        select_worktree(cx, w, &main, "/c/wt/bisect");
        assert_eq!(trigger_text(cx, &main).as_deref(), Some("No branch"));
        click(cx, w, "details-branch");
        assert!(dropdown(cx, w).is_empty());
        assert_eq!(label_of(cx, w, "branch-empty"), "Couldn't list branches");
    }

    /// A real repo through the real switch read and write: the worktree re-nests under
    /// its new branch node, the pane follows, and the selection stays row-for-row.
    #[gpui_kit::test]
    fn switched_worktree_renests_and_selection_stays(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let api = tmp.path().join("code/api");
        std::fs::create_dir_all(&api).unwrap();
        git(&api, &["init", "-q"]);
        std::fs::write(api.join("f.txt"), "base\n").unwrap();
        git(&api, &["add", "-A"]);
        git(&api, &["commit", "-q", "-m", "base"]);
        git(&api, &["switch", "-q", "-c", "feature"]);
        // feature's checkout differs in size, so a switch must move the Size row too.
        std::fs::write(api.join("f.txt"), vec![b'x'; 4096]).unwrap();
        git(&api, &["add", "-A"]);
        git(&api, &["commit", "-q", "-m", "theirs"]);
        git(&api, &["switch", "-q", "main"]);
        let mut repo = grove_core::scan(&tmp.path().join("code")).repos.remove(0);
        // Test setup: the scan leaves statuses unread; the app fills them in the background.
        let wt_path = repo.worktrees[0].path.clone();
        repo.worktrees[0].status = Some(grove_core::read_worktree_status(&wt_path));
        let (w, main) = launch_repo(cx, &file, repo);
        select_worktree(cx, w, &main, wt_path.to_str().unwrap());
        cx.run_until_parked();
        assert_eq!(trigger_text(cx, &main).as_deref(), Some("main"));

        /// The pane's Size line as SizeSource keys it, across the switch below.
        fn size_line(cx: &mut TestAppContext, main: &Entity<MainWindow>) -> String {
            main.read_with(cx, |m, cx| {
                m.details
                    .read(cx)
                    .status_fields()
                    .into_iter()
                    .find_map(|(l, v)| (l == "Size").then_some(v))
                    .unwrap()
            })
        }
        assert_eq!(size_line(cx, &main), "5 B");

        click(cx, w, "details-branch");
        assert_eq!(dropdown(cx, w), ["feature", "main"]);
        click(cx, w, ("branch-item", 0usize));
        cx.run_until_parked();

        let s = status(cx, &main).unwrap();
        assert_eq!(s.text, "Switched to `feature`");
        // The worktree re-nested under its new branch node while the selection stayed on
        // the worktree row; trigger and pane agree on the committed branch.
        assert_eq!(trigger_text(cx, &main).as_deref(), Some("feature"));
        let ix = row_of(cx, &main, &wt_path);
        click(cx, w, ("tree-row", ix));
        assert_eq!(
            main.read_with(cx, |m, cx| m.tree.read(cx).selected().cloned()),
            Some(RowId::Worktree(wt_path.clone()))
        );
        assert!(tree_rows_branches(cx, &main).contains(&"feature".to_string()));
        // Recent commits re-read on their own, as HEAD changed.
        let commits = main
            .read_with(cx, |m, cx| m.details.read(cx).commits().cloned())
            .unwrap();
        match commits {
            Ok(commits) => assert_eq!(commits[0].subject, "theirs"),
            err => panic!("expected commits, got {err:?}"),
        }
        // The checkout also moved HEAD, so the size key moved and the walk re-read:
        // feature's f.txt is bigger.
        assert_eq!(size_line(cx, &main), "4 kB");
    }

    /// An edit mid-session re-keys the walk (newest changes mtime); the last landed
    /// size keeps showing until the fresh walk lands, however much the key moves around.
    #[gpui_kit::test]
    fn an_edit_during_size_walks_keeps_the_last_size_showing(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let api = tmp.path().join("code/api");
        std::fs::create_dir_all(&api).unwrap();
        git(&api, &["init", "-q"]);
        std::fs::write(api.join("f.txt"), "base\n").unwrap();
        git(&api, &["add", "-A"]);
        git(&api, &["commit", "-q", "-m", "base"]);
        let mut repo = grove_core::scan(&tmp.path().join("code")).repos.remove(0);
        // Test setup: the scan leaves statuses unread; the app fills them in the background.
        let wt_path = repo.worktrees[0].path.clone();
        repo.worktrees[0].status = Some(grove_core::read_worktree_status(&wt_path));
        let (w, main) = launch_repo(cx, &file, repo);
        /// The pane's Size line, across the edit below.
        fn size_line(cx: &mut TestAppContext, main: &Entity<MainWindow>) -> String {
            main.read_with(cx, |m, cx| {
                m.details
                    .read(cx)
                    .status_fields()
                    .into_iter()
                    .find_map(|(l, v)| (l == "Size").then_some(v))
                    .unwrap()
            })
        }
        select_worktree(cx, w, &main, wt_path.to_str().unwrap());
        cx.run_until_parked();
        assert_eq!(size_line(cx, &main), "5 B");

        // The edit moves the key and starts a fresh walk — still in flight, since the
        // executor isn't parked.
        std::fs::write(wt_path.join("f.txt"), vec![b'x'; 4096]).unwrap();
        let m = main.clone();
        cx.update_window(w, |_, window, cx| {
            m.update(cx, |m, cx| {
                m.details.update(cx, |d, cx| {
                    d.hold_size_walk(cx);
                });
            });
            window.render_frame(cx);
        })
        .unwrap();
        // The last known size shows across the pending read, not the placeholder.
        assert_eq!(size_line(cx, &main), "5 B");

        cx.run_until_parked();
        // The fresh walk lands and the row catches up with the edit.
        assert_eq!(size_line(cx, &main), "4 kB");
    }

    /// The trigger is in the focus order, and the dropdown works by keyboard alone:
    /// Enter opens, arrows move the cursor, Enter picks, and Escape closes without picking.
    #[gpui_kit::test]
    fn branch_trigger_is_keyboard_operable(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        fake_launcher(cx, Ok(Outcome::Opened));
        lister(cx, &["spare", "main"]);
        let switches = fake_switcher(cx, None);
        let (w, main) = launch_repo(cx, &file, canned());
        select_worktree(cx, w, &main, "/c/api");
        assert_eq!(trigger_text(cx, &main).as_deref(), Some("main"));

        // Reach the branch line by focus order, open it, and close it with Escape.
        tab_to(cx, w, "details-branch");
        cx.update_window(w, |_, window, cx| window.press("enter", cx))
            .unwrap();
        assert_eq!(dropdown(cx, w), ["main", "spare"]);
        cx.update_window(w, |_, window, cx| window.press("escape", cx))
            .unwrap();
        assert!(dropdown(cx, w).is_empty());

        // Reopen, move the cursor down to `spare`, and pick with Enter.
        cx.update_window(w, |_, window, cx| {
            window.press("enter", cx);
            window.press("down", cx);
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();

        assert_eq!(
            *switches.lock().unwrap(),
            [(PathBuf::from("/c/api"), "spare".into())]
        );
    }

    /// A branch created in a terminal lands in the dropdown when the repo is reread,
    /// with no Grove action: the watcher's ref hits drive repo-level refreshes.
    #[gpui_kit::test]
    fn branch_created_in_a_terminal_lands_after_repo_refresh(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let api = tmp.path().join("code/api");
        std::fs::create_dir_all(&api).unwrap();
        git(&api, &["init", "-q"]);
        git(&api, &["commit", "-q", "--allow-empty", "-m", "base"]);
        let mut repo = grove_core::scan(&tmp.path().join("code")).repos.remove(0);
        let wt_path = repo.worktrees[0].path.clone();
        let git_dir = repo.repo.git_dir.clone();
        repo.worktrees[0].status = Some(grove_core::read_worktree_status(&wt_path));
        let (w, main) = launch_repo(cx, &file, repo);
        select_worktree(cx, w, &main, wt_path.to_str().unwrap());
        click(cx, w, "details-branch");
        assert_eq!(dropdown(cx, w), ["main"]);
        cx.update_window(w, |_, window, cx| window.press("escape", cx))
            .unwrap();

        // Created outside Grove; the repo reread that follows a ref change rebuilds the
        // list.
        git(&api, &["branch", "later"]);
        cx.update(|cx| shell::reread_repo(git_dir.as_path(), cx));
        cx.run_until_parked();

        click(cx, w, "details-branch");
        assert_eq!(dropdown(cx, w), ["later", "main"]);
        cx.update_window(w, |_, window, cx| window.press("escape", cx))
            .unwrap();

        // Deleted outside Grove, it drops the same way.
        git(&api, &["branch", "-D", "later"]);
        cx.update(|cx| shell::reread_repo(git_dir.as_path(), cx));
        cx.run_until_parked();

        click(cx, w, "details-branch");
        assert_eq!(dropdown(cx, w), ["main"]);
    }

    /// The branches shown as tree rows for the pane's repo.
    fn tree_rows_branches(cx: &mut TestAppContext, main: &Entity<MainWindow>) -> Vec<String> {
        main.read_with(cx, |m, cx| {
            m.tree
                .read(cx)
                .tree()
                .rows
                .iter()
                .filter_map(|r| match &r.id {
                    RowId::Branch(name) => Some(name.clone()),
                    _ => None,
                })
                .collect()
        })
    }
}
