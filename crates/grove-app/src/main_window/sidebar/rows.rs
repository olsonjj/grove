use std::path::Path;

use gpui_kit::base::Button;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{
    App, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled,
    TestSupportExt, div, px,
};

use crate::icons::{self, icon};
use crate::shell;
use crate::theme;

/// "PINNED · 2", "REPOSITORIES · 40".
pub(crate) fn group_header(id: &'static str, text: String) -> impl Styled + IntoElement {
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
pub(super) fn pin_button(ix: usize, repo: &str, pinned: bool) -> Button {
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
pub(crate) fn top_button(id: &'static str) -> Button {
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
pub(crate) fn link(id: &'static str, text: &'static str, action: fn(&mut App)) -> Button {
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
pub(crate) fn fetch_label(cx: &App) -> String {
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
