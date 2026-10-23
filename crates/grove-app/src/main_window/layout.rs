use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{h_resizable, resizable_panel};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, App, Context, FontWeight, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, Render, StatefulInteractiveElement, Styled, TestSupportExt, Window, div, px,
};
use grove_core::{RepoSnapshot, repo_order};

use crate::icons::{self, icon};
use crate::shell;
use crate::theme;

use super::MainWindow;
use super::sidebar::rows::{fetch_label, group_header, link, tilde, top_button};

/// Sidebar width; also the manage popover's, so it covers the list exactly. Fits
/// "REPOSITORIES · 99" plus the chip, the manage button and the sort menu.
pub(super) const SIDEBAR_W: f32 = 330.;

/// The details pane's width on a fresh install (no saved width yet).
pub(super) const DETAILS_PANE_WIDTH: f32 = 320.;
/// Details-pane clamps the divider enforces.
pub(super) const DETAILS_PANE_MIN_WIDTH: f32 = 260.;
/// Upper clamp of the divider's pane range; see DETAILS_PANE_MIN_WIDTH.
pub(super) const DETAILS_PANE_MAX_WIDTH: f32 = 720.;
/// The tree never gets narrower than this, even with the pane at its maximum.
/// Matches `TreeView`'s own min width: the selected row's inline buttons need
/// the room, and a narrower tree column just lets them bleed under the pane.
pub(super) const TREE_MIN_WIDTH: f32 = 480.;
const SHORTCUTS: &str = "↑↓ Move   ←→ Collapse / expand   ↵ Editor   ⌘↵ Terminal   ⌘K Jump to";
/// The status bar's confirmation or failure line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub text: String,
    pub error: bool,
    /// Fixable in settings: show the settings link.
    pub settings: bool,
}

impl MainWindow {
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
            // Pane width is seeded from settings so a relaunched window opens at
            // the width the last drag left; the tree panel carries no size, so it
            // flexes first.
            let pane_width = shell::library(cx)
                .settings
                .details_pane_width
                .unwrap_or(DETAILS_PANE_WIDTH as i32) as f32;
            let split = h_resizable("tree-details")
                .with_state(&self.panes)
                .child(
                    resizable_panel()
                        .size_range(px(TREE_MIN_WIDTH)..px(f32::MAX))
                        .child(self.tree.clone()),
                )
                .child(
                    resizable_panel()
                        .size(px(pane_width))
                        .size_range(px(DETAILS_PANE_MIN_WIDTH)..px(DETAILS_PANE_MAX_WIDTH))
                        .child(self.details.clone()),
                );
            // The group's own root is `size_full`, so it needs the flexing
            // container the plain row had to share the row with the sidebar.
            div()
                .flex_1()
                .min_w_0()
                .h_full()
                .child(split)
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
