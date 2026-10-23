use std::path::Path;

use gpui_kit::base::Button;
use gpui_kit::component::input::{Escape, Input, InputEvent, InputState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, AppContext, Context, Entity, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, StatefulInteractiveElement, Styled, Subscription, TestSupportExt, Window,
    deferred, div, px,
};
use grove_core::{RepoSnapshot, repo_order};

use crate::icons::{self, icon};
use crate::shell;
use crate::theme;

use super::MainWindow;
use crate::main_window::layout::SIDEBAR_W;

/// Key context of the open manage-visibility popover: Tab is left to move between the
/// filter field and the eye toggles, so a keyboard user can curate by stepping through rows.
const VISIBILITY_CONTEXT: &str = "VisibilityMenu";

/// The open manage-visibility popover; a fresh name filter per session, cleared on close.
pub(crate) struct VisibilityPopover {
    filter: Entity<InputState>,
    _changes: Subscription,
}

impl MainWindow {
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

    /// The header button that opens the manage-visibility popover.
    pub(crate) fn manage_button(&self, cx: &mut Context<Self>) -> Button {
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

    /// The header chip naming the hidden repos; a click opens the popover.
    pub(crate) fn hidden_chip(&self, hidden: usize, cx: &mut Context<Self>) -> Button {
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
    pub(crate) fn visibility_popover(&self, cx: &mut Context<Self>) -> AnyElement {
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
}
