//! Dark palette and shared widgets, from the design prototype.

use gpui_kit::{
    App, ClickEvent, Div, ElementId, FontWeight, InteractiveElement, IntoElement, ParentElement,
    Rgba, SharedString, StatefulInteractiveElement, Styled, Window, div, px, rgb,
};

pub fn bg() -> Rgba {
    rgb(0x121418)
}
pub fn panel() -> Rgba {
    rgb(0x16191e)
}
pub fn sidebar() -> Rgba {
    rgb(0x15181d)
}
pub fn field() -> Rgba {
    rgb(0x0f1114)
}
pub fn raised() -> Rgba {
    rgb(0x1e222a)
}
pub fn selected() -> Rgba {
    rgb(0x263140)
}
pub fn border() -> Rgba {
    rgb(0x262b33)
}
pub fn border_strong() -> Rgba {
    rgb(0x34404f)
}
pub fn text() -> Rgba {
    rgb(0xe7e9ec)
}
pub fn text_soft() -> Rgba {
    rgb(0xc9ced6)
}
pub fn muted() -> Rgba {
    rgb(0xa0a7b2)
}
pub fn faint() -> Rgba {
    rgb(0x8b93a0)
}
pub fn link() -> Rgba {
    rgb(0x7db7ff)
}
pub fn accent() -> Rgba {
    rgb(0xf0a847)
}
pub fn accent_bg() -> Rgba {
    rgb(0x2e2414)
}
pub fn on_accent() -> Rgba {
    rgb(0x1a1307)
}
pub fn warn_bg() -> Rgba {
    rgb(0x2a2215)
}
pub fn warn_text() -> Rgba {
    rgb(0xf7c987)
}

/// Worktree rows are tinted so a folder never reads as a branch name.
pub fn worktree_row() -> Rgba {
    rgb(0x151d27)
}
pub fn worktree_text() -> Rgba {
    rgb(0xbfdbff)
}
pub fn connector() -> Rgba {
    rgb(0x4a5462)
}

pub const MONO: &str = "Menlo";
pub const SANS: &str = ".SystemUIFont";

pub fn mono(text: impl Into<SharedString>) -> Div {
    div().font_family(MONO).child(text.into())
}

/// Single-line path, elided at the start so the folder name stays visible.
pub fn path_text(text: impl Into<SharedString>) -> Div {
    mono(text)
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis_start()
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    Primary,
    Secondary,
}

/// Button; `on_click: None` renders it disabled.
pub fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    kind: ButtonKind,
    on_click: Option<impl Fn(&ClickEvent, &mut Window, &mut App) + 'static>,
) -> impl IntoElement {
    let enabled = on_click.is_some();
    let base = div()
        .id(id.into())
        .h(px(34.))
        .px(px(14.))
        .flex()
        .items_center()
        .rounded(px(8.))
        .text_size(px(13.))
        .child(label.into());
    let styled = match (kind, enabled) {
        (ButtonKind::Primary, true) => base
            .bg(accent())
            .text_color(on_accent())
            .font_weight(FontWeight::SEMIBOLD)
            .hover(|s| s.bg(rgb(0xe2993a))),
        (ButtonKind::Primary, false) => base
            .bg(rgb(0x2a2f38))
            .text_color(faint())
            .font_weight(FontWeight::SEMIBOLD),
        (ButtonKind::Secondary, _) => base
            .bg(raised())
            .border_1()
            .border_color(border_strong())
            .text_color(text())
            .hover(|s| s.bg(rgb(0x262b33))),
    };
    match on_click {
        Some(f) => styled.cursor_pointer().on_click(f),
        None => styled,
    }
}
