//! "Recent ▾": a list header's sort mode button and its menu.

use gpui_kit::base::Button;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    Anchor, Context, EventEmitter, FocusHandle, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, Render, StatefulInteractiveElement, Styled, TestSupportExt, Window, anchored,
    deferred, div, px,
};
use grove_core::SortMode;

use crate::details_view::{self, LeaveMenu, LeaveMenuBack};
use crate::icons::{self, icon};
use crate::theme;

/// The user picked a mode; the owner persists it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortChosen(pub SortMode);

/// Element ids and what the button says it sorts.
#[derive(Debug, Clone, Copy)]
pub struct SortMenuIds {
    pub button: &'static str,
    pub menu: &'static str,
    /// Indexed by `MODES`.
    pub choice: &'static str,
    /// "repositories" → "Sort repositories: Recent".
    pub what: &'static str,
}

/// The menu's items, in order.
const MODES: [SortMode; 2] = [SortMode::Recent, SortMode::Name];

pub struct SortMenu {
    ids: SortMenuIds,
    mode: SortMode,
    open: bool,
    button: FocusHandle,
    items: [FocusHandle; 2],
}

impl EventEmitter<SortChosen> for SortMenu {}

impl SortMenu {
    pub fn new(ids: SortMenuIds, mode: SortMode, cx: &mut Context<Self>) -> Self {
        Self {
            ids,
            mode,
            open: false,
            button: cx.focus_handle(),
            items: [cx.focus_handle(), cx.focus_handle()],
        }
    }

    pub fn set_mode(&mut self, mode: SortMode, cx: &mut Context<Self>) {
        if self.mode != mode {
            self.mode = mode;
            cx.notify();
        }
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.open) {
            window.focus(&self.button, cx);
            cx.notify();
        }
    }

    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let at = self.items.iter().position(|h| h.is_focused(window));
        let next = match (ev.keystroke.key.as_str(), at) {
            ("escape", _) => {
                self.close(window, cx);
                return cx.stop_propagation();
            }
            ("down", Some(i)) => (i + 1).min(MODES.len() - 1),
            ("up", Some(i)) => i.saturating_sub(1),
            ("down" | "up", None) => 0,
            _ => return,
        };
        window.focus(&self.items[next], cx);
        cx.stop_propagation();
    }

    fn button(&self, cx: &mut Context<Self>) -> Button {
        let mode = self.mode;
        Button::new(self.ids.button)
            .track_focus(&self.button)
            .accessibility_label(format!("Sort {}: {}", self.ids.what, mode.label()))
            .aria_expanded(self.open)
            .flex_none()
            .h(px(22.))
            .px(px(6.))
            .gap(px(3.))
            .rounded(px(5.))
            .border_1()
            .border_color(gpui_kit::transparent_black())
            .focus_visible(|s| s.border_color(theme::link()))
            .cursor_pointer()
            .hover(|s| s.bg(theme::border()))
            .text_size(px(11.))
            .text_color(theme::muted())
            .child(mode.label())
            .child(icon(icons::CHEVRON_DOWN, 10.).text_color(theme::muted()))
            .on_click(cx.listener(move |this, _, window, cx| {
                if this.open {
                    this.close(window, cx);
                } else {
                    this.open = true;
                    let ix = MODES.iter().position(|m| *m == mode).unwrap_or(0);
                    window.focus(&this.items[ix], cx);
                    cx.notify();
                }
            }))
    }

    fn menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let items = MODES.into_iter().enumerate().map(|(i, mode)| {
            Button::new((self.ids.choice, i))
                .track_focus(&self.items[i])
                .tab_stop(false)
                .role(gpui_kit::Role::MenuItemRadio)
                .aria_toggled((mode == self.mode).into())
                .accessibility_label(mode.label())
                .h(px(28.))
                .px(px(10.))
                .justify_between()
                .gap(px(12.))
                .rounded(px(5.))
                .cursor_pointer()
                .text_color(theme::text())
                .hover(|s| s.bg(theme::selected()))
                .focus(|s| s.bg(theme::selected()))
                .child(mode.label())
                .when(mode == self.mode, |d| {
                    d.child(div().text_color(theme::link()).child("✓"))
                })
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.mode = mode;
                    cx.emit(SortChosen(mode));
                    this.close(window, cx);
                }))
        });
        deferred(
            anchored().anchor(Anchor::TopRight).snap_to_window().child(
                div()
                    .id(self.ids.menu)
                    .role(gpui_kit::Role::Menu)
                    .key_context(details_view::MENU_CONTEXT)
                    .on_key_down(cx.listener(Self::on_key))
                    .on_action(cx.listener(|this, _: &LeaveMenu, window, cx| {
                        this.close(window, cx);
                        window.focus_next(cx);
                    }))
                    .on_action(cx.listener(|this, _: &LeaveMenuBack, window, cx| {
                        this.close(window, cx);
                        window.focus_prev(cx);
                    }))
                    // Outside click closes the menu and is swallowed, like a native menu.
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                        this.open = false;
                        cx.stop_propagation();
                        cx.notify();
                    }))
                    .mt(px(4.))
                    .min_w(px(120.))
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
                    .child(
                        div()
                            .px(px(10.))
                            .py(px(4.))
                            .text_size(px(11.))
                            .text_color(theme::faint())
                            .child("SORT BY"),
                    )
                    .children(items)
                    .test_support(),
            ),
        )
        .with_priority(1)
    }
}

impl Render for SortMenu {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex_none()
            .flex()
            .flex_col()
            .items_end()
            .child(self.button(cx))
            .when(self.open, |d| d.child(self.menu(cx)))
    }
}
