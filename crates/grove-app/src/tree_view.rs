//! The checked-out tree for the selected repo: legend, rows, keyboard navigation.

use std::path::PathBuf;

use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, Context, EventEmitter, FocusHandle, Focusable, InteractiveElement, IntoElement,
    KeyDownEvent, ParentElement, Render, ScrollHandle, SharedString, StatefulInteractiveElement,
    Styled, TestSupportExt, Window, div, px, rgb,
};
use grove_core::{NavEffect, NavKey, RepoSnapshot, Role, Row, RowId, RowKind, Tree, navigate};

use crate::icons::{self, icon};
use crate::theme;

/// A prefix folder was collapsed or expanded; the owner persists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollapseChanged {
    pub repo: String,
    pub prefix: String,
    pub collapsed: bool,
}

/// Enter / ⌘Enter or an inline button asked to open a worktree in the editor / terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenRequested {
    pub path: PathBuf,
    pub openable: bool,
    pub role: Role,
}

pub struct TreeView {
    repo: Option<RepoSnapshot>,
    collapsed: Vec<String>,
    tree: Tree,
    selected: Option<RowId>,
    focus: FocusHandle,
    scroll: ScrollHandle,
}

impl EventEmitter<CollapseChanged> for TreeView {}
impl EventEmitter<OpenRequested> for TreeView {}

impl Focusable for TreeView {
    fn focus_handle(&self, _: &gpui_kit::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TreeView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            repo: None,
            collapsed: Vec::new(),
            tree: Tree::default(),
            selected: None,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
        }
    }

    pub fn repo_name(&self) -> Option<&str> {
        self.repo.as_ref().map(|r| r.name())
    }

    #[cfg(test)]
    pub fn tree(&self) -> &Tree {
        &self.tree
    }

    #[cfg(test)]
    pub fn selected(&self) -> Option<&RowId> {
        self.selected.as_ref()
    }

    /// Shows `repo` with the given collapsed prefixes; selects the first row.
    pub fn set_repo(
        &mut self,
        repo: Option<RepoSnapshot>,
        collapsed: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        self.repo = repo;
        self.collapsed = collapsed;
        self.rebuild();
        self.selected = None;
        self.apply(navigate(&self.tree, None, NavKey::Down), cx);
    }

    fn rebuild(&mut self) {
        self.tree = self
            .repo
            .as_ref()
            .map(|r| r.tree(&self.collapsed))
            .unwrap_or_default();
    }

    fn apply(&mut self, effect: NavEffect, cx: &mut Context<Self>) {
        if let Some((prefix, collapse)) = effect.toggle {
            self.set_folder(&prefix, collapse, cx);
        }
        if let Some(id) = effect.select {
            self.select(id);
        }
        cx.notify();
    }

    fn select(&mut self, id: RowId) {
        if let Some(ix) = self.tree.index_of(&id) {
            self.scroll.scroll_to_item(ix);
        }
        self.selected = Some(id);
    }

    fn set_folder(&mut self, prefix: &str, collapse: bool, cx: &mut Context<Self>) {
        let Some(repo) = self.repo_name().map(str::to_string) else {
            return;
        };
        self.collapsed.retain(|p| p != prefix);
        if collapse {
            self.collapsed.push(prefix.to_string());
        }
        self.rebuild();
        // Selection inside a now-hidden folder moves to the folder.
        if self
            .selected
            .as_ref()
            .is_some_and(|id| self.tree.index_of(id).is_none())
        {
            self.selected = Some(RowId::Folder(prefix.to_string()));
        }
        cx.emit(CollapseChanged {
            repo,
            prefix: prefix.to_string(),
            collapsed: collapse,
        });
    }

    fn on_key(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let m = &ev.keystroke.modifiers;
        if m.control || m.alt || m.shift {
            return;
        }
        if m.platform {
            if ev.keystroke.key == "enter" {
                self.request_open(Role::Terminal, cx);
                cx.stop_propagation();
            }
            return;
        }
        let key = match ev.keystroke.key.as_str() {
            "up" => NavKey::Up,
            "down" => NavKey::Down,
            "left" => NavKey::Left,
            "right" => NavKey::Right,
            "enter" => {
                self.request_open(Role::Editor, cx);
                cx.stop_propagation();
                return;
            }
            _ => return,
        };
        let effect = navigate(&self.tree, self.selected.as_ref(), key);
        self.apply(effect, cx);
        cx.stop_propagation();
    }

    /// Opens the selected row's worktree (a branch opens its first one).
    fn request_open(&mut self, role: Role, cx: &mut Context<Self>) {
        let target = self
            .selected
            .as_ref()
            .and_then(|id| self.tree.worktree_for(id));
        if let Some((path, openable)) = target {
            cx.emit(OpenRequested {
                path: path.to_path_buf(),
                openable,
                role,
            });
        }
    }

    /// Dev hook: selects the first worktree row and opens it.
    pub fn dev_open_first(&mut self, cx: &mut Context<Self>) {
        let first = self
            .tree
            .rows
            .iter()
            .find(|r| matches!(r.kind, RowKind::Worktree { .. }));
        if let Some(id) = first.map(|r| r.id.clone()) {
            self.select(id);
            self.request_open(Role::Editor, cx);
            cx.notify();
        }
    }

    fn on_row_click(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        let Some(row) = self.tree.rows.get(ix) else {
            return;
        };
        let mut effect = NavEffect {
            select: Some(row.id.clone()),
            toggle: None,
        };
        if let RowKind::Folder {
            prefix, collapsed, ..
        } = &row.kind
        {
            effect.toggle = Some((prefix.clone(), !collapsed));
        }
        self.apply(effect, cx);
    }

    fn count_line(&self) -> String {
        let t = &self.tree;
        let plural =
            |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        let mut s = format!(
            "{} · {}",
            plural(t.worktrees, "worktree", "worktrees"),
            plural(t.branches, "checked-out branch", "checked-out branches")
        );
        if t.detached > 0 {
            s.push_str(&format!(" + {} detached", t.detached));
        }
        if self.repo.as_ref().is_some_and(|r| r.repo.bare) {
            s = format!("bare · {s}");
        }
        s
    }

    fn render_row(&self, ix: usize, row: &Row, cx: &mut Context<Self>) -> AnyElement {
        if row.kind == RowKind::DetachedSection {
            return div()
                .id(("tree-row", ix))
                .pt(px(18.))
                .pb(px(6.))
                .px(px(10.))
                .text_size(px(11.))
                .text_color(theme::faint())
                .child("DETACHED WORKTREES")
                .into_any_element();
        }
        let selected = self.selected.as_ref() == Some(&row.id);
        let is_wt = matches!(row.kind, RowKind::Worktree { .. });
        let bg = match (selected, is_wt) {
            (true, _) => Some(theme::selected()),
            (false, true) => Some(theme::worktree_row()),
            _ => None,
        };
        let base = div()
            .id(("tree-row", ix))
            .h(px(34.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(8.))
            .pl(px(10. + row.depth as f32 * 20.))
            .pr(px(8.))
            .rounded(px(6.))
            .font_family(theme::MONO)
            .text_size(px(13.))
            .cursor_pointer()
            .when_some(bg, |d, bg| d.bg(bg))
            .when(!selected, |d| d.hover(|s| s.bg(theme::raised())))
            .on_click(cx.listener(move |this, _, window, cx| this.on_row_click(ix, window, cx)));

        match &row.kind {
            RowKind::Folder {
                prefix,
                count,
                collapsed,
            } => base
                .text_color(theme::muted())
                .child(
                    icon(
                        if *collapsed {
                            icons::CHEVRON_RIGHT
                        } else {
                            icons::CHEVRON_DOWN
                        },
                        14.,
                    )
                    .text_color(theme::muted()),
                )
                .child(single_line(prefix.clone()))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(theme::faint())
                        .child(count.to_string()),
                ),
            RowKind::Branch { label, .. } => base
                .text_color(theme::text())
                .child(icon(icons::BRANCH, 15.).text_color(theme::text_soft()))
                .child(single_line(label.clone())),
            RowKind::Worktree {
                display,
                is_main,
                detached_at,
                openable,
                ..
            } => base
                .text_color(theme::worktree_text())
                .child(
                    icon(icons::ELBOW, 14.)
                        .ml(px(-4.))
                        .text_color(theme::connector()),
                )
                .child(icon(icons::FOLDER, 15.).text_color(theme::link()))
                .child(
                    div()
                        .flex()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .child(
                            div()
                                .min_w_0()
                                .flex_shrink(1.)
                                .overflow_hidden()
                                .text_ellipsis_start()
                                .text_color(theme::faint())
                                .child(display.parent.clone()),
                        )
                        .child(div().flex_none().child(display.name.clone())),
                )
                .when(*is_main, |d| d.child(badge("main worktree")))
                .when_some(detached_at.as_ref(), |d, sha| {
                    d.child(badge(format!("detached @ {sha}")))
                })
                .when(selected, |d| {
                    d.child(div().flex_1())
                        .child(self.open_button(ix, *openable, cx))
                        .child(self.terminal_button(ix, *openable, cx))
                }),
            RowKind::DetachedSection => unreachable!(),
        }
        .test_support()
        .into_any_element()
    }
}

impl TreeView {
    /// Inline "Open in <editor>" on the selected worktree row; disabled when missing.
    fn open_button(&self, ix: usize, openable: bool, cx: &mut Context<Self>) -> AnyElement {
        let editor = crate::launcher::target_name(Role::Editor, cx);
        let label: SharedString = match &editor {
            Some(name) => format!("Open in {name}").into(),
            None => "Open in editor".into(),
        };
        let tip: SharedString = if openable {
            format!("{label} (↵)").into()
        } else {
            "Folder is missing; run git worktree prune".into()
        };
        div()
            .id(("open-editor", ix))
            .flex_none()
            .h(px(24.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(6.))
            .rounded(px(6.))
            .border_1()
            .border_color(theme::border_strong())
            .font_family(theme::SANS)
            .text_size(px(12.))
            .child(icon(icons::EDITOR, 13.).text_color(if openable {
                theme::link()
            } else {
                theme::faint()
            }))
            .child(label)
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .when_else(
                openable,
                |d| {
                    d.bg(theme::raised())
                        .text_color(theme::text())
                        .cursor_pointer()
                        .hover(|s| s.bg(theme::border()))
                        .on_click(cx.listener(|this, _, _, cx| this.request_open(Role::Editor, cx)))
                },
                |d| d.text_color(theme::faint()),
            )
            .test_support()
            .into_any_element()
    }
}

impl TreeView {
    /// Inline icon button "Open in <terminal>" next to the editor button.
    fn terminal_button(&self, ix: usize, openable: bool, cx: &mut Context<Self>) -> AnyElement {
        let tip: SharedString = if openable {
            match crate::launcher::target_name(Role::Terminal, cx) {
                Some(name) => format!("Open in {name} (⌘↵)").into(),
                None => "Open in terminal (⌘↵)".into(),
            }
        } else {
            "Folder is missing; run git worktree prune".into()
        };
        div()
            .id(("open-terminal", ix))
            .flex_none()
            .size(px(24.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.))
            .border_1()
            .border_color(theme::border_strong())
            .child(icon(icons::TERMINAL, 13.).text_color(if openable {
                theme::link()
            } else {
                theme::faint()
            }))
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .when(openable, |d| {
                d.bg(theme::raised())
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::border()))
                    .on_click(cx.listener(|this, _, _, cx| this.request_open(Role::Terminal, cx)))
            })
            .test_support()
            .into_any_element()
    }
}

fn single_line(text: String) -> gpui_kit::Div {
    div()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .child(text)
}

fn badge(text: impl Into<SharedString>) -> impl IntoElement {
    div()
        .flex_none()
        .px(px(7.))
        .h(px(18.))
        .flex()
        .items_center()
        .rounded(px(9.))
        .bg(theme::border())
        .text_color(theme::text_soft())
        .font_family(theme::SANS)
        .text_size(px(11.))
        .child(text.into())
}

fn legend_item(icon_el: impl IntoElement, label: &'static str) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .child(icon_el)
        .child(label)
}

fn legend() -> impl IntoElement {
    let folder_chip = div()
        .size(px(18.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .bg(rgb(0x17222f))
        .child(icon(icons::FOLDER, 12.).text_color(theme::link()));
    div()
        .flex()
        .flex_wrap()
        .gap(px(16.))
        .text_size(px(12.))
        .text_color(theme::muted())
        .child(legend_item(
            icon(icons::BRANCH, 13.).text_color(theme::text_soft()),
            "Branch",
        ))
        .child(legend_item(folder_chip, "Worktree (folder on disk)"))
}

impl Render for TreeView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows: Vec<AnyElement> = self
            .tree
            .rows
            .iter()
            .enumerate()
            .map(|(ix, row)| self.render_row(ix, row, cx))
            .collect();
        let empty = rows.is_empty();
        let error = self
            .repo
            .as_ref()
            .and_then(|r| r.error.as_ref())
            .map(|e| e.to_string());
        div()
            .id("tree-view")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap(px(8.))
                    .pt(px(14.))
                    .pb(px(10.))
                    .px(px(16.))
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(theme::muted())
                            .child(self.count_line()),
                    )
                    .child(legend()),
            )
            .children(error.map(|e| {
                div()
                    .mx(px(16.))
                    .mb(px(8.))
                    .p(px(10.))
                    .rounded(px(6.))
                    .bg(theme::warn_bg())
                    .text_color(theme::warn_text())
                    .text_size(px(13.))
                    .child(e)
            }))
            .child(
                div()
                    .id("tree-rows")
                    .track_scroll(&self.scroll)
                    .flex_1()
                    .overflow_y_scroll()
                    .px(px(10.))
                    .pb(px(16.))
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .when(empty, |d| {
                        d.child(
                            div()
                                .p(px(6.))
                                .text_size(px(13.))
                                .text_color(theme::muted())
                                .child("No worktrees"),
                        )
                    })
                    .children(rows)
                    .test_support(),
            )
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::rc::Rc;

    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{
        AppContext, Bounds, Entity, Point, TestAppContext, WindowBounds, WindowOptions, size,
    };
    use grove_core::{RepoRef, Worktree};

    use super::*;

    fn wt(path: &str, branch: Option<&str>, is_main: bool) -> Worktree {
        Worktree {
            path: PathBuf::from(path),
            branch: branch.map(str::to_string),
            head: Some("a1f3c9e5d2".into()),
            detached: branch.is_none(),
            is_main,
            locked: false,
            prunable: false,
        }
    }

    /// main, feature/{x,y}, fix/z, one detached.
    pub(crate) fn canned() -> RepoSnapshot {
        RepoSnapshot {
            display_name: "api".into(),
            repo: RepoRef {
                name: "api".into(),
                path: "/c/api".into(),
                git_dir: "/c/api/.git".into(),
                bare: false,
            },
            worktrees: vec![
                wt("/c/api", Some("main"), true),
                wt("/c/wt/api-x", Some("feature/x"), false),
                wt("/c/wt/api-y", Some("feature/y"), false),
                wt("/c/wt/api-z", Some("fix/z"), false),
                wt("/c/wt/bisect", None, false),
            ],
            error: None,
        }
    }

    fn branch(name: &str) -> Option<RowId> {
        Some(RowId::Branch(name.into()))
    }
    fn folder(prefix: &str) -> Option<RowId> {
        Some(RowId::Folder(prefix.into()))
    }
    fn worktree(path: &str) -> Option<RowId> {
        Some(RowId::Worktree(path.into()))
    }

    fn open(
        cx: &mut TestAppContext,
        collapsed: &[&str],
    ) -> (gpui_kit::AnyWindowHandle, Entity<TreeView>) {
        cx.update(gpui_kit::init);
        let collapsed: Vec<String> = collapsed.iter().map(|s| s.to_string()).collect();
        cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(720.), px(600.)),
                })),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| {
                    let mut view = TreeView::new(cx);
                    view.set_repo(Some(canned()), collapsed, cx);
                    window.focus(&view.focus, cx);
                    view
                })
            })
            .unwrap()
        })
    }

    fn press(cx: &mut TestAppContext, w: gpui_kit::AnyWindowHandle, keys: &[&str]) {
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            for k in keys {
                window.press(k, cx);
            }
        })
        .unwrap();
    }

    fn selected(cx: &mut TestAppContext, view: &Entity<TreeView>) -> Option<RowId> {
        view.read_with(cx, |v, _| v.selected().cloned())
    }

    #[gpui_kit::test]
    fn arrows_move_selection_across_visible_rows(cx: &mut TestAppContext) {
        let (w, view) = open(cx, &[]);
        assert_eq!(selected(cx, &view), branch("main"));
        press(cx, w, &["down"]);
        assert_eq!(selected(cx, &view), worktree("/c/api"));
        press(cx, w, &["down", "down"]);
        assert_eq!(selected(cx, &view), branch("feature/x"));
        press(cx, w, &["up"]);
        assert_eq!(selected(cx, &view), folder("feature/"));
        // The detached section header is skipped.
        press(cx, w, &["down"; 9]);
        assert_eq!(selected(cx, &view), worktree("/c/wt/bisect"));
        press(cx, w, &["up"]);
        assert_eq!(selected(cx, &view), worktree("/c/wt/api-z"));
    }

    #[gpui_kit::test]
    fn left_right_collapse_expand_and_report(cx: &mut TestAppContext) {
        let (w, view) = open(cx, &[]);
        let events = Rc::new(RefCell::new(Vec::new()));
        let sink = events.clone();
        cx.update(|cx| {
            cx.subscribe(&view, move |_, ev: &CollapseChanged, _| {
                sink.borrow_mut().push((ev.prefix.clone(), ev.collapsed))
            })
            .detach()
        });

        // On feature/x: ← moves to the folder, ← again collapses it.
        press(cx, w, &["down", "down", "down", "left"]);
        assert_eq!(selected(cx, &view), folder("feature/"));
        press(cx, w, &["left"]);
        assert_eq!(selected(cx, &view), folder("feature/"));
        // Collapsed: ↓ skips its children.
        press(cx, w, &["down"]);
        assert_eq!(selected(cx, &view), folder("fix/"));
        press(cx, w, &["up", "right", "down"]);
        assert_eq!(selected(cx, &view), branch("feature/x"));
        // → on a branch steps into its worktree; ← back out.
        press(cx, w, &["right"]);
        assert_eq!(selected(cx, &view), worktree("/c/wt/api-x"));
        press(cx, w, &["left"]);
        assert_eq!(selected(cx, &view), branch("feature/x"));

        assert_eq!(
            *events.borrow(),
            [
                ("feature/".to_string(), true),
                ("feature/".to_string(), false)
            ]
        );
    }

    /// main plus 40 linked worktrees on `b00`..`b39`.
    fn long_repo() -> RepoSnapshot {
        let mut repo = canned();
        repo.worktrees.truncate(1);
        for i in 0..40 {
            repo.worktrees.push(wt(
                &format!("/c/wt/b{i:02}"),
                Some(&format!("b{i:02}")),
                false,
            ));
        }
        repo
    }

    /// Whether row `ix` lies fully inside the scrolling viewport.
    fn row_in_view(cx: &mut TestAppContext, w: gpui_kit::AnyWindowHandle, ix: usize) -> bool {
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            let viewport = window.find("tree-rows").bounds();
            let row = window.find(("tree-row", ix)).bounds();
            row.top() >= viewport.top() && row.bottom() <= viewport.bottom()
        })
        .unwrap()
    }

    #[gpui_kit::test]
    fn keyboard_selection_scrolls_into_view(cx: &mut TestAppContext) {
        let (w, view) = open(cx, &[]);
        cx.update_window(w, |_, window, cx| {
            window.resize(size(px(720.), px(320.)));
            view.update(cx, |v, cx| v.set_repo(Some(long_repo()), Vec::new(), cx));
        })
        .unwrap();
        let last = 1 + 2 * 40;
        assert!(row_in_view(cx, w, 0));
        assert!(!row_in_view(cx, w, last), "fixture must overflow");

        press(cx, w, &["down"; 81]);
        assert_eq!(selected(cx, &view), worktree("/c/wt/b39"));
        assert!(row_in_view(cx, w, last));
        assert!(!row_in_view(cx, w, 0));

        press(cx, w, &["up"; 81]);
        assert_eq!(selected(cx, &view), branch("main"));
        assert!(row_in_view(cx, w, 0));
    }

    #[gpui_kit::test]
    fn restored_collapse_state_hides_children(cx: &mut TestAppContext) {
        let (w, view) = open(cx, &["fix/"]);
        // main, wt, feature/, x, wt, y, wt, fix/ (collapsed), section, bisect.
        press(cx, w, &["down"; 7]);
        assert_eq!(selected(cx, &view), folder("fix/"));
        press(cx, w, &["down"]);
        assert_eq!(selected(cx, &view), worktree("/c/wt/bisect"));
    }
}
