//! Dark palette and shared widgets, from the design prototype. Every text color here
//! meets 4.5:1 on every surface (see the tests).

use gpui_kit::base::Button;
use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::{
    App, ClickEvent, Div, ElementId, FontWeight, InteractiveElement, ParentElement, Rgba,
    SharedString, Styled, Window, div, px, rgb,
};

/// GPUI Kit's own widgets (tooltips, inputs, scrollbars) in their dark theme too.
pub fn init(cx: &mut App) {
    Theme::change(ThemeMode::Dark, None, cx);
}

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
pub fn card() -> Rgba {
    rgb(0x1b1f26)
}
pub fn disabled() -> Rgba {
    rgb(0x2a2f38)
}
pub fn selected() -> Rgba {
    rgb(0x263140)
}
/// Also badge and hover fill.
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
    rgb(0x929aa7)
}
pub fn link() -> Rgba {
    rgb(0x7db7ff)
}
pub fn accent() -> Rgba {
    rgb(0xf0a847)
}
pub fn accent_hover() -> Rgba {
    rgb(0xe2993a)
}
/// Divider between the halves of an accent split button.
pub fn on_accent_divider() -> Rgba {
    rgb(0xb87a2c)
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
/// Hairline between recent-commit rows in the details pane: half a step above the panel
/// background, well below the card color, so it separates without reading as card chrome.
pub fn commit_divider() -> Rgba {
    rgb(0x191d23)
}
pub fn worktree_text() -> Rgba {
    rgb(0xbfdbff)
}
pub fn connector() -> Rgba {
    rgb(0x4a5462)
}

pub const MONO: &str = "Menlo";
pub const SANS: &str = ".SystemUIFont";

/// Accent bar on a selected row's left edge, so selection isn't shown by fill alone. The
/// row must be `relative()`.
pub fn selection_bar() -> Div {
    div()
        .absolute()
        .left(px(1.))
        .top(px(8.))
        .bottom(px(8.))
        .w(px(3.))
        .rounded(px(2.))
        .bg(accent())
}

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
) -> Button {
    let enabled = on_click.is_some();
    let base = Button::new(id)
        .disabled(!enabled)
        .flex_none()
        .h(px(34.))
        .px(px(14.))
        .rounded(px(8.))
        .border_1()
        .border_color(gpui_kit::transparent_black())
        .focus_visible(|s| s.border_color(link()))
        .text_size(px(13.))
        .child(label.into());
    let styled = match (kind, enabled) {
        (ButtonKind::Primary, true) => base
            .bg(accent())
            .text_color(on_accent())
            .font_weight(FontWeight::SEMIBOLD)
            .hover(|s| s.bg(accent_hover()))
            .focus_visible(|s| s.border_color(text())),
        (ButtonKind::Primary, false) => base
            .bg(disabled())
            .text_color(faint())
            .font_weight(FontWeight::SEMIBOLD),
        (ButtonKind::Secondary, _) => base
            .bg(raised())
            .border_color(border_strong())
            .text_color(text())
            .hover(|s| s.bg(border())),
    };
    match on_click {
        Some(f) => styled.cursor_pointer().on_click(f),
        None => styled,
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::TestAppContext;
    use gpui_kit::component::theme::ActiveTheme;

    use super::*;

    fn luminance(c: Rgba) -> f32 {
        let lin = |v: f32| {
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
    }

    /// WCAG contrast ratio.
    fn contrast(a: Rgba, b: Rgba) -> f32 {
        let (x, y) = (luminance(a), luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    fn assert_readable(pairs: &[(&str, Rgba, &str, Rgba)]) {
        let failing: Vec<String> = pairs
            .iter()
            .map(|(f, fg, b, bg)| (f, b, contrast(*fg, *bg)))
            .filter(|(_, _, r)| *r < 4.5)
            .map(|(f, b, r)| format!("{f} on {b}: {r:.2}"))
            .collect();
        assert!(failing.is_empty(), "below 4.5:1: {failing:?}");
    }

    #[test]
    fn every_text_color_meets_4_5_on_every_surface() {
        let texts = [
            ("text", text()),
            ("text_soft", text_soft()),
            ("muted", muted()),
            ("faint", faint()),
            ("link", link()),
            ("accent", accent()),
            ("warn_text", warn_text()),
            ("worktree_text", worktree_text()),
        ];
        let surfaces = [
            ("bg", bg()),
            ("panel", panel()),
            ("sidebar", sidebar()),
            ("field", field()),
            ("raised", raised()),
            ("card", card()),
            ("selected", selected()),
            ("border", border()),
            ("disabled", disabled()),
            ("worktree_row", worktree_row()),
            ("accent_bg", accent_bg()),
            ("warn_bg", warn_bg()),
        ];
        let mut pairs = Vec::new();
        for (f, fg) in texts {
            for (b, bg) in surfaces {
                pairs.push((f, fg, b, bg));
            }
        }
        pairs.push(("on_accent", on_accent(), "accent", accent()));
        pairs.push(("on_accent", on_accent(), "accent_hover", accent_hover()));
        assert_readable(&pairs);
    }

    /// Tooltips, and placeholders in Grove's inputs.
    #[gpui_kit::test]
    fn gpui_kit_widgets_meet_4_5(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            let t = cx.theme();
            let c = |h: gpui_kit::Hsla| Rgba::from(h);
            let editor = t
                .highlight_theme
                .style
                .editor_background
                .unwrap_or_else(|| t.input_background());
            assert_readable(&[
                ("tooltip", c(t.popover_foreground), "tooltip", c(t.popover)),
                ("input", c(t.foreground), "raised", raised()),
                ("input", c(t.foreground), "editor", c(editor)),
                ("placeholder", c(t.muted_foreground), "raised", raised()),
                ("placeholder", c(t.muted_foreground), "editor", c(editor)),
            ]);
        });
    }
}
