//! The checked-out tree for the selected repo: legend, rows, keyboard navigation.

use std::path::PathBuf;

use gpui_kit::base::Button;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, ParentElement,
    Pixels, Point, Render, Role as A11yRole, ScrollHandle, SharedString,
    StatefulInteractiveElement, Styled, Subscription, TestSupportExt, Window, anchored, deferred,
    div, px, rgb,
};
use grove_core::{
    Badge, LastOpened, NavEffect, NavKey, Presence, RepoSnapshot, Role, Row, RowId, RowKind,
    SortMode, Tree, navigate,
};

use crate::details_view::{LeaveMenu, LeaveMenuBack, MENU_CONTEXT};
use crate::icons::{self, icon};
use crate::launcher::Choice;
use crate::removal::{self, Offer, RemoveRequested};
use crate::sort_menu::{SortChosen, SortMenu, SortMenuIds};
use crate::theme;

/// A prefix folder was collapsed or expanded; the owner persists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollapseChanged {
    pub repo: String,
    pub prefix: String,
    pub collapsed: bool,
}

/// Enter / ⌘Enter or an open button asked to open a worktree in the editor / terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenRequested {
    pub path: PathBuf,
    pub presence: Presence,
    pub role: Role,
    /// One-off app or custom command from a ▾ menu; `None` uses the default for `role`.
    pub target: Option<Choice>,
}

pub struct TreeView {
    repo: Option<RepoSnapshot>,
    collapsed: Vec<String>,
    sort: SortMode,
    last_opened: LastOpened,
    tree: Tree,
    selected: Option<RowId>,
    focus: FocusHandle,
    scroll: ScrollHandle,
    sort_menu: Entity<SortMenu>,
    _sort_chosen: Subscription,
    row_menu: Option<RowMenu>,
    row_menu_item: FocusHandle,
    /// Holds focus when the item is disabled (a disabled button can't), so Esc still closes.
    row_menu_focus: FocusHandle,
}

/// A worktree row's context menu.
struct RowMenu {
    path: PathBuf,
    offer: Offer,
    at: Point<Pixels>,
}

impl EventEmitter<CollapseChanged> for TreeView {}
impl EventEmitter<OpenRequested> for TreeView {}
impl EventEmitter<RemoveRequested> for TreeView {}
/// Forwarded from the header's sort menu; the owner persists it.
impl EventEmitter<SortChosen> for TreeView {}

impl Focusable for TreeView {
    fn focus_handle(&self, _: &gpui_kit::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TreeView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let sort = SortMode::default();
        let ids = SortMenuIds {
            button: "tree-sort",
            menu: "tree-sort-menu",
            choice: "tree-sort-choice",
            what: "branches",
        };
        let sort_menu = cx.new(|cx| SortMenu::new(ids, sort, cx));
        let sort_chosen = cx.subscribe(&sort_menu, |_, _, ev: &SortChosen, cx| cx.emit(*ev));
        Self {
            repo: None,
            collapsed: Vec::new(),
            sort,
            last_opened: LastOpened::new(),
            tree: Tree::default(),
            selected: None,
            // A tab stop so Tab comes back here from the details pane and top bar.
            focus: cx.focus_handle().tab_stop(true),
            scroll: ScrollHandle::new(),
            sort_menu,
            _sort_chosen: sort_chosen,
            row_menu: None,
            row_menu_item: cx.focus_handle(),
            row_menu_focus: cx.focus_handle(),
        }
    }

    /// Reorders by `sort`; the selection stays on its row and is scrolled into view.
    pub fn set_order(&mut self, sort: SortMode, last_opened: LastOpened, cx: &mut Context<Self>) {
        if self.sort == sort && self.last_opened == last_opened {
            return;
        }
        self.sort = sort;
        self.last_opened = last_opened;
        self.sort_menu.update(cx, |m, cx| m.set_mode(sort, cx));
        self.rebuild();
        cx.notify();
    }

    pub fn repo_name(&self) -> Option<&str> {
        self.repo.as_ref().map(|r| r.name())
    }

    pub fn repo(&self) -> Option<&RepoSnapshot> {
        self.repo.as_ref()
    }

    #[cfg(test)]
    pub fn tree(&self) -> &Tree {
        &self.tree
    }

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

    /// Same repo with fresher data (e.g. a status read landed); keeps selection and collapse.
    pub fn update_repo(&mut self, repo: Option<&RepoSnapshot>, cx: &mut Context<Self>) {
        if self.repo.as_ref() == repo {
            return;
        }
        self.repo = repo.cloned();
        let old: Vec<RowId> = self.tree.rows.iter().map(|r| r.id.clone()).collect();
        let old_ix = self
            .selected
            .as_ref()
            .and_then(|id| old.iter().position(|r| r == id));
        self.rebuild();
        if self
            .selected
            .as_ref()
            .is_some_and(|id| self.tree.index_of(id).is_none())
        {
            self.selected = None;
            // The row that took its place: the next surviving one, else the previous.
            let survivor = old_ix.and_then(|ix| {
                let lands = |id: &RowId| {
                    self.tree
                        .index_of(id)
                        .filter(|&i| self.tree.selectable(i))
                        .map(|_| id.clone())
                };
                old[ix + 1..]
                    .iter()
                    .find_map(lands)
                    .or_else(|| old[..ix].iter().rev().find_map(lands))
            });
            match survivor {
                Some(id) => self.select(id),
                None => self.apply(navigate(&self.tree, None, NavKey::Down), cx),
            }
        }
        cx.notify();
    }

    /// Keeps a moved selection in view, unless the user had scrolled away from it.
    fn rebuild(&mut self) {
        let selected_ix = |tree: &Tree| self.selected.as_ref().and_then(|id| tree.index_of(id));
        let (viewport, dy) = (self.scroll.bounds(), self.scroll.offset().y);
        // Item bounds are unscrolled.
        let before = selected_ix(&self.tree).filter(|&ix| {
            self.scroll.bounds_for_item(ix).is_some_and(|b| {
                b.bottom() + dy > viewport.top() && b.top() + dy < viewport.bottom()
            })
        });
        self.tree = self
            .repo
            .as_ref()
            .map(|r| r.tree(&self.collapsed, self.sort, &self.last_opened))
            .unwrap_or_default();
        let after = selected_ix(&self.tree);
        if let (Some(before), Some(ix)) = (before, after)
            && before != ix
        {
            self.scroll.scroll_to_item(ix);
        }
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

    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let m = &ev.keystroke.modifiers;
        // Not keys bubbling up from the header's sort menu.
        if !self.focus.is_focused(window) || m.control || m.alt || m.shift {
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
        if let Some((path, presence)) = target {
            cx.emit(OpenRequested {
                path: path.to_path_buf(),
                presence,
                role,
                target: None,
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

    /// Selects a worktree row and opens its menu, if it has one (not the main worktree).
    fn open_row_menu(
        &mut self,
        ix: usize,
        at: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(row) = self.tree.rows.get(ix) else {
            return;
        };
        let RowKind::Worktree { path, .. } = &row.kind else {
            return;
        };
        let wt = self
            .repo
            .as_ref()
            .and_then(|r| r.worktrees.iter().find(|w| &w.path == path));
        let offer = wt.and_then(removal::offer);
        let path = path.clone();
        self.select(row.id.clone());
        match offer {
            Some(offer) => {
                let focus = match offer.reason {
                    None => &self.row_menu_item,
                    Some(_) => &self.row_menu_focus,
                };
                window.focus(focus, cx);
                self.row_menu = Some(RowMenu { path, offer, at });
            }
            None => window.focus(&self.focus, cx),
        }
        cx.notify();
    }

    fn close_row_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.row_menu.take().is_some() {
            window.focus(&self.focus, cx);
            cx.notify();
        }
    }

    fn row_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.row_menu.as_ref()?;
        let path = menu.path.clone();
        let enabled = menu.offer.reason.is_none();
        let item = Button::new("tree-menu-remove")
            .track_focus(&self.row_menu_item)
            .tab_stop(false)
            .role(gpui_kit::Role::MenuItem)
            .accessibility_label(menu.offer.label)
            .when_some(menu.offer.reason.clone(), |b, reason| {
                b.aria_description(reason)
            })
            .disabled(!enabled)
            .h(px(28.))
            .px(px(10.))
            .justify_start()
            .rounded(px(5.))
            .focus(|s| s.bg(theme::selected()))
            .when_else(
                enabled,
                |b| {
                    b.cursor_pointer()
                        .text_color(theme::text())
                        .hover(|s| s.bg(theme::selected()))
                },
                |b| b.text_color(theme::faint()),
            )
            .child(menu.offer.label)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.close_row_menu(window, cx);
                cx.emit(RemoveRequested { path: path.clone() });
            }));
        let reason = menu.offer.reason.clone().map(|reason| {
            div()
                .id("tree-menu-reason")
                .aria_label(reason.clone())
                .max_w(px(280.))
                .px(px(10.))
                .pb(px(4.))
                .text_size(px(12.))
                .text_color(theme::muted())
                .child(reason)
                .test_support()
        });
        Some(
            deferred(
                anchored().position(menu.at).snap_to_window().child(
                    div()
                        .id("tree-menu")
                        .track_focus(&self.row_menu_focus)
                        .role(gpui_kit::Role::Menu)
                        .key_context(MENU_CONTEXT)
                        .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                            if ev.keystroke.key == "escape" {
                                this.close_row_menu(window, cx);
                                cx.stop_propagation();
                            }
                        }))
                        .on_action(cx.listener(|this, _: &LeaveMenu, window, cx| {
                            this.close_row_menu(window, cx);
                            window.focus_next(cx);
                        }))
                        .on_action(cx.listener(|this, _: &LeaveMenuBack, window, cx| {
                            this.close_row_menu(window, cx);
                            window.focus_prev(cx);
                        }))
                        // Outside click closes the menu and is swallowed, like a native menu.
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                            this.row_menu = None;
                            cx.stop_propagation();
                            cx.notify();
                        }))
                        .min_w(px(160.))
                        .p(px(4.))
                        .flex()
                        .flex_col()
                        .rounded(px(8.))
                        .border_1()
                        .border_color(theme::border_strong())
                        .bg(theme::raised())
                        .shadow_lg()
                        .occlude()
                        .font_family(theme::SANS)
                        .text_size(px(13.))
                        .child(item)
                        .children(reason)
                        .test_support(),
                ),
            )
            .with_priority(1)
            .into_any_element(),
        )
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
        let unavailable = matches!(
            row.kind,
            RowKind::Worktree {
                presence: Presence::Unavailable,
                ..
            }
        );
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
            .relative()
            .when_some(bg, |d, bg| d.bg(bg))
            .when_else(
                selected,
                |d| d.child(theme::selection_bar()),
                |d| d.hover(|s| s.bg(theme::raised())),
            )
            .role(A11yRole::TreeItem)
            .aria_label(row_label(row))
            .aria_selected(selected)
            .on_click(cx.listener(move |this, _, window, cx| this.on_row_click(ix, window, cx)))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                    this.open_row_menu(ix, ev.position, window, cx);
                    cx.stop_propagation();
                }),
            );

        match &row.kind {
            RowKind::Folder {
                prefix,
                count,
                collapsed,
            } => base
                .aria_expanded(!collapsed)
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
                presence,
                badges,
                ..
            } => base
                // Dimmed by color, not opacity, so its words stay readable.
                .text_color(if unavailable {
                    theme::faint()
                } else {
                    theme::worktree_text()
                })
                .child(
                    icon(icons::ELBOW, 14.)
                        .ml(px(-4.))
                        .text_color(theme::connector()),
                )
                .child(icon(icons::FOLDER, 15.).text_color(if unavailable {
                    theme::faint()
                } else {
                    theme::link()
                }))
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
                .children(badges.iter().enumerate().map(|(i, b)| badge(i, b)))
                .when(*presence == Presence::Missing, |d| {
                    d.child(
                        div()
                            .flex_none()
                            .font_family(theme::SANS)
                            .text_size(px(11.))
                            .text_color(theme::faint())
                            .child("run git worktree prune"),
                    )
                })
                .when(selected, |d| {
                    d.child(div().flex_1())
                        .child(self.open_button(ix, *presence, cx))
                        .child(self.terminal_button(ix, *presence, cx))
                }),
            RowKind::DetachedSection => unreachable!("rendered above"),
        }
        .test_support()
        .into_any_element()
    }
}

impl TreeView {
    /// Inline "Open in <editor>" on the selected worktree row; disabled when missing.
    fn open_button(&self, ix: usize, presence: Presence, cx: &mut Context<Self>) -> AnyElement {
        let openable = presence.is_openable();
        let editor = crate::launcher::target_name(Role::Editor, cx);
        let label: SharedString = match &editor {
            Some(name) => format!("Open in {name}").into(),
            None => "Open in editor".into(),
        };
        let tip: SharedString = match presence.reason() {
            None => format!("{label} (↵)").into(),
            Some(reason) => format!("Can't open: {reason}").into(),
        };
        inline_button(("open-editor", ix), presence)
            .h(px(24.))
            .px(px(8.))
            .gap(px(6.))
            .font_family(theme::SANS)
            .text_size(px(12.))
            .accessibility_label(label.clone())
            .aria_keyshortcuts("Enter")
            .child(icon(icons::EDITOR, 13.).text_color(if openable {
                theme::link()
            } else {
                theme::faint()
            }))
            .child(label)
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .when_else(
                openable,
                |d| d.text_color(theme::text()),
                |d| d.text_color(theme::faint()),
            )
            .on_click(cx.listener(|this, _, _, cx| this.request_open(Role::Editor, cx)))
            .into_any_element()
    }

    /// Inline icon button "Open in <terminal>" next to the editor button.
    fn terminal_button(&self, ix: usize, presence: Presence, cx: &mut Context<Self>) -> AnyElement {
        let openable = presence.is_openable();
        let label: SharedString = match crate::launcher::target_name(Role::Terminal, cx) {
            Some(name) => format!("Open in {name}").into(),
            None => "Open in terminal".into(),
        };
        let tip: SharedString = match presence.reason() {
            None => format!("{label} (⌘↵)").into(),
            Some(reason) => format!("Can't open: {reason}").into(),
        };
        inline_button(("open-terminal", ix), presence)
            .size(px(24.))
            .accessibility_label(label)
            .aria_keyshortcuts("Command+Enter")
            .child(icon(icons::TERMINAL, 13.).text_color(if openable {
                theme::link()
            } else {
                theme::faint()
            }))
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .on_click(cx.listener(|this, _, _, cx| this.request_open(Role::Terminal, cx)))
            .into_any_element()
    }
}

/// A row's open button. Not a tab stop: Enter / ⌘Enter on the focused tree do the same.
fn inline_button(id: impl Into<gpui_kit::ElementId>, presence: Presence) -> Button {
    let openable = presence.is_openable();
    Button::new(id)
        .focusable(false)
        .disabled(!openable)
        .flex_none()
        .rounded(px(6.))
        .border_1()
        .border_color(theme::border_strong())
        .when_some(presence.reason(), |b, reason| b.aria_description(reason))
        .when(openable, |b| {
            b.bg(theme::raised())
                .cursor_pointer()
                .hover(|s| s.bg(theme::border()))
        })
}

/// What a screen reader says for `row`.
fn row_label(row: &Row) -> String {
    match &row.kind {
        RowKind::Folder { prefix, count, .. } => format!("{prefix}, folder of {count} branches"),
        RowKind::Branch { label, .. } => format!("{label}, branch"),
        RowKind::Worktree {
            display, badges, ..
        } => {
            let mut s = format!("{}{}, worktree", display.parent, display.name);
            for b in badges {
                s.push_str(", ");
                s.push_str(&b.description());
            }
            s
        }
        RowKind::DetachedSection => "Detached worktrees".into(),
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

/// Worded state badge; color only reinforces the words. Tooltip gives the long form.
fn badge(i: usize, b: &Badge) -> impl IntoElement {
    let (bg, fg) = match b {
        Badge::Changed(_) => (theme::accent_bg(), theme::accent()),
        Badge::Missing | Badge::UpstreamGone(_) | Badge::StatusFailed(_) => {
            (theme::warn_bg(), theme::warn_text())
        }
        _ => (theme::border(), theme::text_soft()),
    };
    let tip: SharedString = b.description().into();
    div()
        .id(("badge", i))
        .flex_none()
        .px(px(7.))
        .h(px(18.))
        .flex()
        .items_center()
        .gap(px(4.))
        .rounded(px(9.))
        .bg(bg)
        .text_color(fg)
        .font_family(theme::SANS)
        .text_size(px(11.))
        .when(*b == Badge::Locked, |d| {
            d.child(icon(icons::LOCK, 11.).text_color(fg))
        })
        .child(b.label())
        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
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
        let row_menu = self.row_menu(cx);
        let error = self
            .repo
            .as_ref()
            .and_then(|r| r.error.as_ref())
            .map(|e| e.to_string());
        div()
            .id("tree-view")
            .test_support()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .flex_1()
            // Room for the selected row's inline buttons; the details pane gives way first.
            .min_w(px(480.))
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
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .child(legend())
                            .child(self.sort_menu.clone()),
                    ),
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
                    .role(A11yRole::Tree)
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
            .children(row_menu)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{
        AppContext, Bounds, Entity, Point, TestAppContext, WindowBounds, WindowOptions, size,
    };

    use super::*;
    use crate::test_support::{canned, wt};

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
    fn reorder_keeps_selection_and_scrolls_it_into_view(cx: &mut TestAppContext) {
        let (w, view) = open(cx, &[]);
        cx.update_window(w, |_, window, cx| {
            window.resize(size(px(720.), px(320.)));
            view.update(cx, |v, cx| v.set_repo(Some(long_repo()), Vec::new(), cx));
        })
        .unwrap();
        press(cx, w, &["down", "down"]);
        assert_eq!(selected(cx, &view), branch("b00"));
        assert!(row_in_view(cx, w, 2));

        // Every other branch active: b00, never active, sorts last.
        let opened = (1..40)
            .map(|i| (PathBuf::from(format!("/c/wt/b{i:02}")), 1_700_000_000 + i))
            .collect();
        cx.update_window(w, |_, _, cx| {
            view.update(cx, |v, cx| v.set_order(SortMode::Recent, opened, cx))
        })
        .unwrap();
        // After main, its worktree and 39 branches with one worktree each.
        let last_branch = 2 + 2 * 39;
        assert_eq!(selected(cx, &view), branch("b00"));
        assert_eq!(
            view.read_with(cx, |v, _| v.tree().rows[last_branch].id.clone()),
            RowId::Branch("b00".into())
        );
        assert!(row_in_view(cx, w, last_branch));
        assert!(!row_in_view(cx, w, 2));
    }

    #[gpui_kit::test]
    fn reorder_leaves_the_view_alone_when_scrolled_away(cx: &mut TestAppContext) {
        let (w, view) = open(cx, &[]);
        cx.update_window(w, |_, window, cx| {
            window.resize(size(px(720.), px(320.)));
            view.update(cx, |v, cx| v.set_repo(Some(long_repo()), Vec::new(), cx));
        })
        .unwrap();
        press(cx, w, &["down", "down"]);
        assert_eq!(selected(cx, &view), branch("b00"));
        view.read_with(cx, |v, _| v.scroll.scroll_to_bottom());
        let last = 1 + 2 * 40;
        assert!(row_in_view(cx, w, last));
        assert!(!row_in_view(cx, w, 2));

        // b01 moves above b00; b00 is still off screen, so nothing scrolls.
        let opened = LastOpened::from([(PathBuf::from("/c/wt/b01"), 1_700_000_000)]);
        cx.update_window(w, |_, _, cx| {
            view.update(cx, |v, cx| v.set_order(SortMode::Recent, opened, cx))
        })
        .unwrap();
        assert_eq!(
            view.read_with(cx, |v, _| v.tree().index_of(&RowId::Branch("b00".into()))),
            Some(4)
        );
        assert!(row_in_view(cx, w, last));
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
