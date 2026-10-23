use std::path::{Path, PathBuf};

pub(super) mod rows;
pub(super) mod visibility;

use gpui_kit::base::Button;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, Context, Focusable, FontWeight, InteractiveElement, IntoElement, KeyDownEvent,
    MouseButton, MouseDownEvent, ParentElement, Pixels, Point, SharedString,
    StatefulInteractiveElement, Styled, TestSupportExt, Window, anchored, deferred, div, px,
};
use grove_core::{RepoSnapshot, repo_order};

use crate::details_view::{self, LeaveMenu, LeaveMenuBack};
use crate::shell;
use crate::theme;

use self::rows::pin_button;
use super::MainWindow;

/// A sidebar row's context menu.
pub(super) struct RepoMenu {
    repo: String,
    path: PathBuf,
    pinned: bool,
    at: Point<Pixels>,
}

impl MainWindow {
    pub(super) fn select_repo(&mut self, name: &str, cx: &mut Context<Self>) {
        shell::select_repo(name, cx);
        if self.tree.read(cx).repo_name() != Some(name) {
            self.show_repo(Some(name), cx);
        }
        cx.notify();
    }

    /// ↑/↓ move the selection through both groups, skipping headers.
    pub(super) fn on_repo_key(
        &mut self,
        ev: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
    pub(super) fn toggle_pin(&mut self, path: &Path, cx: &mut Context<Self>) {
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

    pub(super) fn close_repo_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(menu) = self.repo_menu.take() {
            if let Some([row, _]) = self.repo_focus.get(&menu.repo) {
                window.focus(row, cx);
            }
            cx.notify();
        }
    }
    pub(super) fn repo_row(
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

    pub(super) fn repo_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
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
