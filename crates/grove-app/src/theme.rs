//! Dark palette and shared widgets, from the design prototype. Every text color here
//! meets 4.5:1 on every surface (see the tests).
//!
//! [`Palette`] (built by [`palette`]) is the cx-aware API: it reads Grove's tokens
//! from the active gpui-kit theme and derives Grove's bespoke tints from that theme's
//! tokens. The theme registered by [`init`] — "Grove Dark", translated from the
//! fixed helpers — makes that active theme carry Grove's values; the helpers are the
//! legacy form, the call sites migrate to the palette over tickets 29-31, and ticket
//! 32 deletes them once they're gone.

use std::rc::Rc;

use gpui_kit::base::Button;
use gpui_kit::component::highlighter::HighlightTheme;
use gpui_kit::component::theme::{Theme, ThemeConfig, ThemeMode, ThemeRegistry};
use gpui_kit::{
    App, ClickEvent, Div, ElementId, FontWeight, InteractiveElement, ParentElement, Rgba,
    SharedString, Styled, Window, div, px, rgb,
};

/// Grove Dark, translated from the fixed helpers above into a gpui-kit theme
/// file. Tokens Grove doesn't map stay on gpui-kit's dark defaults via the
/// theme-color fallbacks, and the registry keeps the built-in "Default Dark"
/// for every token this file does not set.
const GROVE_DARK: &str = include_str!("grove-dark.json");

/// The registry name of Grove's own theme (see [`init`]).
const GROVE_DARK_NAME: &str = "Grove Dark";

/// GPUI Kit's own widgets (tooltips, inputs, scrollbars) in Grove dark too.
///
/// Must run after [`gpui_kit::init`]: that call seeds the registry (and its
/// built-in dark default) which the steps below assume and override. The
/// registry must also not yet hold a theme named "Grove Dark" — gpui-kit
/// ships none, and a pre-registered theme of that name would be silently
/// skipped by [`ThemeRegistry::load_themes_from_str`], installing foreign
/// colors under Grove's name. Ticket 33's bundled-theme registration must
/// keep that guarantee.
///
/// Runs in three steps: the registry learns "Grove Dark" by name, so name-based
/// theme lookups resolve to Grove from here on; [`Theme::apply_config`] installs
/// it as the active dark theme, because the registry exposes no way to override
/// its built-in dark default and [`Theme::dark_theme`] is what
/// [`Theme::change`] re-applies; then the kept `Theme::change` startup call
/// loads that dark theme through its normal path, so `cx.theme()` — and every
/// `palette(cx)` call site with it — resolves to Grove's values again.
pub fn init(cx: &mut App) {
    ThemeRegistry::global_mut(cx)
        .load_themes_from_str(GROVE_DARK)
        // The file is embedded, so this is a build-time bug (caught by the
        // runtime parity test), not a runtime failure to recover from.
        .expect("the embedded grove-dark.json parses as a gpui-kit theme set");
    let grove_dark = ThemeRegistry::global(cx)
        .themes()
        .get(GROVE_DARK_NAME)
        .cloned()
        .expect("Grove Dark was just loaded into the registry");
    Theme::update(cx, |theme| {
        let mut config = ThemeConfig::clone(&grove_dark);
        // Grove's palette carries no editor or syntax colors; keep gpui-kit's
        // "Default Dark" highlight so those stay exactly what gpui-kit's own
        // dark default provided before this registration.
        config.highlight = Some(HighlightTheme::default_dark().style.clone());
        theme.apply_config(&Rc::new(config));
    });
    Theme::change(ThemeMode::Dark, None, cx);
    // The theme-file path cannot express Grove's opaque selection fill:
    // `apply_config` clamps the selection token to 30% alpha (its overlay
    // semantics) and the `Theme::change` above re-applies that clamp, so
    // restate the token; `Theme::update`'s reconcile carries it onto the
    // renderable token, keeping `palette(cx)`'s `selected` (and the widgets'
    // selection) at today's opaque fill. The clamp also runs whenever the
    // semantic snapshot recomputes from `colors.selection` — if a later
    // change (ticket 33's launch apply) takes that path, the restatement
    // must happen after it too.
    Theme::update(cx, |theme| theme.selection = selected().into());
}

/// The default background.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "call sites moved to the palette in tickets 29-31; the helper goes in ticket 32"
    )
)]
pub fn bg() -> Rgba {
    rgb(0x121418)
}

/// The [`palette::palette`] entry point, re-exported at this path so call sites read
/// `theme::palette(cx)` while the `palette` module keeps its internals private.
pub(crate) use palette::palette;
pub fn panel() -> Rgba {
    rgb(0x16191e)
}
/// The worktree sidebar.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "call sites moved to the palette in tickets 29-31; the helper goes in ticket 32"
    )
)]
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
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "call sites moved to the palette in tickets 29-31; the helper goes in ticket 32"
    )
)]
pub fn on_accent_divider() -> Rgba {
    rgb(0xb87a2c)
}
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "call sites moved to the palette in tickets 29-31; the helper goes in ticket 32"
    )
)]
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
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "call sites moved to the palette in tickets 29-31; the helper goes in ticket 32"
    )
)]
pub fn worktree_row() -> Rgba {
    rgb(0x151d27)
}
/// Hairline between recent-commit rows in the details pane: half a step above the panel
/// background, well below the card color, so it separates without reading as card chrome.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "call sites moved to the palette in tickets 29-31; the helper goes in ticket 32"
    )
)]
pub fn commit_divider() -> Rgba {
    rgb(0x191d23)
}
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "call sites moved to the palette in tickets 29-31; the helper goes in ticket 32"
    )
)]
pub fn worktree_text() -> Rgba {
    rgb(0xbfdbff)
}
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "call sites moved to the palette in tickets 29-31; the helper goes in ticket 32"
    )
)]
pub fn connector() -> Rgba {
    rgb(0x4a5462)
}

/// Grove's cx-aware palette beside the fixed-color helpers above it; tickets 29-31
/// migrate the call sites over.
///
/// One lint expectation while that migration runs: trees and details already read the
/// palette, so some fixed helpers and some palette tokens are unused until tickets 30-31
/// move the remaining views over and ticket 32 deletes the fixed helpers.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "mid-migration: tickets 30-31 move the last call sites over"
    )
)]
pub(crate) mod palette {
    use gpui_kit::component::theme::{ActiveTheme, Colorize, ThemeColor};
    use gpui_kit::{App, Hsla, Rgba};

    /// Grove's palette, read from the active gpui-kit theme (`cx.theme()`).
    #[derive(Clone, Copy)]
    pub struct Palette {
        bg: Hsla,
        panel: Hsla,
        sidebar: Hsla,
        field: Hsla,
        raised: Hsla,
        card: Hsla,
        disabled: Hsla,
        selected: Hsla,
        border: Hsla,
        border_strong: Hsla,
        text: Hsla,
        connector: Hsla,
        text_soft: Hsla,
        muted: Hsla,
        faint: Hsla,
        link: Hsla,
        accent: Hsla,
        accent_hover: Hsla,
        on_accent_divider: Hsla,
        accent_bg: Hsla,
        on_accent: Hsla,
        warn_bg: Hsla,
        warn_text: Hsla,
        worktree_row: Hsla,
        commit_divider: Hsla,
        worktree_text: Hsla,
    }

    /// Grove's palette over the active theme. Everything is derived here, once per call.
    pub fn palette(cx: &App) -> Palette {
        palette_from(cx.theme())
    }

    /// How strongly the worktree row tint blends the accent over the background; kept
    /// subtle so a folded folder still reads as the background surface.
    const WORKTREE_ROW_TINT: f32 = 0.08;

    /// How strongly the accent background blends the accent over the background.
    const ACCENT_BG_TINT: f32 = 0.12;

    /// The commit divider sits exactly halfway between panel and card.
    const COMMIT_DIVIDER_MID: f32 = 0.5;

    /// Where `text_soft` sits on the text → muted ramp, as the weight on `text`.
    const TEXT_SOFT_STEP: f32 = 0.6;

    /// The lightness step from `muted` down to `faint`.
    const FAINT_DARKEN: f32 = 0.08;

    /// The text weight mixed into the border to raise it to `border_strong`.
    const BORDER_STRONG_STEP: f32 = 0.11;

    /// The text weight mixed into the border to raise it to `connector`, one step
    /// stronger than [`BORDER_STRONG_STEP`].
    const CONNECTOR_STEP: f32 = 0.22;

    /// The lightness share `accent_hover` gives up from the accent.
    pub(crate) const ACCENT_HOVER_DARKEN: f32 = 0.06;

    /// How far the accent is darkened for text placed on it, when an inversion does
    /// not reach the 4.5:1 bar.
    const ON_ACCENT_DARKEN: f32 = 0.9;

    /// The share of the accent kept in the divider between the halves of an accent
    /// split button, the rest coming from [`palette_from`]'s `on_accent` text.
    const ON_ACCENT_DIVIDER_ACCENT: f32 = 0.7;

    /// How strongly warning surfaces blend their warning tint over the background.
    pub(crate) const WARN_TINT: f32 = 0.12;

    /// In the accent fallback for the warning pair, the text weight mixed into the
    /// accent, so the fallback text follows its theme's text level.
    const WARN_FALLBACK_TEXT: f32 = 0.55;

    /// In the accent fallback for the warning pair, how far the darker candidate is
    /// darkened for light surfaces the pale candidate cannot reach.
    const WARN_FALLBACK_DARKEN: f32 = 0.55;

    /// The minimum WCAG contrast ratio a derived text token must reach on its surface.
    pub(crate) const MIN_CONTRAST: f32 = 4.5;

    /// Grove's resolved palette over a theme's resolved colors, so [`palette`] and the
    /// derivation tests share one mapping.
    pub(crate) fn palette_from(t: &ThemeColor) -> Palette {
        let bg = t.background;
        let panel = t.popover;
        let card = t.group_box;
        let border = t.border;
        let text = t.foreground;
        let muted = t.muted_foreground;
        let accent = t.accent;
        let on_accent = on_accent_text(accent);
        let (warn_bg, warn_text) = warn_pair(bg, t.warning, t.warning_foreground, accent, text);
        Palette {
            bg,
            panel,
            sidebar: t.sidebar,
            field: t.input,
            raised: t.list,
            card,
            disabled: t.muted,
            selected: t.selection,
            border,
            border_strong: blended(border, text, BORDER_STRONG_STEP),
            text,
            connector: blended(border, text, CONNECTOR_STEP),
            text_soft: blended(text, muted, TEXT_SOFT_STEP),
            muted,
            faint: muted.darken(FAINT_DARKEN),
            link: t.link,
            accent,
            accent_hover: accent.darken(ACCENT_HOVER_DARKEN),
            on_accent_divider: blended(on_accent, accent, ON_ACCENT_DIVIDER_ACCENT),
            accent_bg: blended(bg, accent, ACCENT_BG_TINT),
            on_accent,
            warn_bg,
            warn_text,
            worktree_row: blended(bg, accent, WORKTREE_ROW_TINT),
            commit_divider: blended(panel, card, COMMIT_DIVIDER_MID),
            worktree_text: t.sidebar_foreground,
        }
    }

    /// Blends `toward` into `base` by `factor`, where 0.5 means the halfway point and
    /// 1.0 means `toward` itself.
    pub(crate) fn blended(base: Hsla, toward: Hsla, factor: f32) -> Hsla {
        base.mix(toward, 1.0 - factor)
    }

    /// Text that stays readable on the accent: a lightness-inverted accent or a deeply
    /// darkened one, whichever contrasts better. Dark saturated accents may land under
    /// the bar with both candidates; the closer one is still used.
    pub(crate) fn on_accent_text(accent: Hsla) -> Hsla {
        most_readable(accent.invert_l(), accent.darken(ON_ACCENT_DARKEN), accent)
    }

    /// Warning surface and text pair. Prefers the theme's warning tokens; when the
    /// warning foreground is not readable on the blended surface, falls back to a warm
    /// accent tint with the more readable of a pale and a darkened accent text.
    pub(crate) fn warn_pair(
        bg: Hsla,
        warning: Hsla,
        warning_foreground: Hsla,
        accent: Hsla,
        text: Hsla,
    ) -> (Hsla, Hsla) {
        let warn_bg = blended(bg, warning, WARN_TINT);
        if contrast_ratio(warning_foreground, warn_bg) >= MIN_CONTRAST {
            return (warn_bg, warning_foreground);
        }
        let warn_bg = blended(bg, accent, WARN_TINT);
        let pair_fg = most_readable(
            blended(accent, text, WARN_FALLBACK_TEXT),
            accent.darken(WARN_FALLBACK_DARKEN),
            warn_bg,
        );
        (warn_bg, pair_fg)
    }

    /// The candidate with the better contrast on `surface`; ties keep the first.
    fn most_readable(a: Hsla, b: Hsla, surface: Hsla) -> Hsla {
        if contrast_ratio(a, surface) >= contrast_ratio(b, surface) {
            a
        } else {
            b
        }
    }

    /// WCAG relative luminance.
    fn relative_luminance(c: Hsla) -> f32 {
        let lin = |v: f32| {
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        let rgb = Rgba::from(c);
        0.2126 * lin(rgb.r) + 0.7152 * lin(rgb.g) + 0.0722 * lin(rgb.b)
    }

    /// WCAG contrast ratio between two colors.
    pub(crate) fn contrast_ratio(a: Hsla, b: Hsla) -> f32 {
        let (x, y) = (relative_luminance(a), relative_luminance(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    impl Palette {
        /// The default background.
        pub fn bg(&self) -> Hsla {
            self.bg
        }

        /// Popover and sheet surfaces.
        pub fn panel(&self) -> Hsla {
            self.panel
        }

        /// The worktree sidebar.
        pub fn sidebar(&self) -> Hsla {
            self.sidebar
        }

        /// Input and select fields.
        pub fn field(&self) -> Hsla {
            self.field
        }

        /// Raised surfaces, such as secondary buttons and list rows.
        pub fn raised(&self) -> Hsla {
            self.raised
        }

        /// Card and panel-section chrome.
        pub fn card(&self) -> Hsla {
            self.card
        }

        /// Disabled control fills.
        pub fn disabled(&self) -> Hsla {
            self.disabled
        }

        /// Selection fills.
        pub fn selected(&self) -> Hsla {
            self.selected
        }

        /// Default border, as well as badge and hover fills.
        pub fn border(&self) -> Hsla {
            self.border
        }

        /// One step stronger than [`Self::border`] for chrome on chrome.
        pub fn border_strong(&self) -> Hsla {
            self.border_strong
        }

        /// The default text color.
        pub fn text(&self) -> Hsla {
            self.text
        }

        /// Tree connectors, one border step stronger than [`Self::border_strong`].
        pub fn connector(&self) -> Hsla {
            self.connector
        }

        /// Secondary text, between [`Self::text`] and [`Self::muted`].
        pub fn text_soft(&self) -> Hsla {
            self.text_soft
        }

        /// Muted text, such as placeholders and captions.
        pub fn muted(&self) -> Hsla {
            self.muted
        }

        /// A step below [`Self::muted`], for the least important text and disabled
        /// labels.
        pub fn faint(&self) -> Hsla {
            self.faint
        }

        /// Link text.
        pub fn link(&self) -> Hsla {
            self.link
        }

        /// The accent color for primary controls and highlights.
        pub fn accent(&self) -> Hsla {
            self.accent
        }

        /// Hovered accent controls.
        pub fn accent_hover(&self) -> Hsla {
            self.accent_hover
        }

        /// Divider between the halves of an accent split button.
        pub fn on_accent_divider(&self) -> Hsla {
            self.on_accent_divider
        }

        /// Soft accent fill behind accent-tinted content.
        pub fn accent_bg(&self) -> Hsla {
            self.accent_bg
        }

        /// Text on accent-colored controls.
        pub fn on_accent(&self) -> Hsla {
            self.on_accent
        }

        /// Warning surface fill.
        pub fn warn_bg(&self) -> Hsla {
            self.warn_bg
        }

        /// Warning text, readable on [`Self::warn_bg`].
        pub fn warn_text(&self) -> Hsla {
            self.warn_text
        }

        /// Worktree rows are tinted so a folder never reads as a branch name.
        pub fn worktree_row(&self) -> Hsla {
            self.worktree_row
        }

        /// Hairline between recent-commit rows in the details pane: exactly halfway
        /// between the panel background and the card color, so it separates without
        /// reading as card chrome.
        pub fn commit_divider(&self) -> Hsla {
            self.commit_divider
        }

        /// Text over the worktree sidebar.
        pub fn worktree_text(&self) -> Hsla {
            self.worktree_text
        }
    }
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
    use gpui_kit::Hsla;
    use gpui_kit::TestAppContext;
    use gpui_kit::component::theme::{ActiveTheme, ThemeColor};

    use super::palette::{
        ACCENT_HOVER_DARKEN, MIN_CONTRAST, WARN_TINT, blended, contrast_ratio, on_accent_text,
        palette, palette_from, warn_pair,
    };

    use super::*;

    /// The sRGB bytes of a color, rounded to the nearest byte per channel.
    /// gpui carries theme colors as f32 Hsla, and some sRGB byte triples don't
    /// convert back to Rgba bit-for-bit, so palette parity is asserted at byte
    /// granularity (`u32::from(Rgba)` truncates and would flip the low bytes
    /// instead).
    fn color_bytes(c: Rgba) -> u32 {
        ((c.r * 255.0).round() as u32) << 24
            | ((c.g * 255.0).round() as u32) << 16
            | ((c.b * 255.0).round() as u32) << 8
            | ((c.a * 255.0).round() as u32)
    }

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

    /// Every text over every surface, as `(text, fg, surface, bg)` rows.
    fn cross<'a>(
        texts: &'a [(&'a str, Rgba)],
        surfaces: &'a [(&'a str, Rgba)],
    ) -> Vec<(&'a str, Rgba, &'a str, Rgba)> {
        texts
            .iter()
            .flat_map(|(f, fg)| surfaces.iter().map(move |(b, bg)| (*f, *fg, *b, *bg)))
            .collect()
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
        let mut pairs = cross(&texts, &surfaces);
        pairs.push(("on_accent", on_accent(), "accent", accent()));
        pairs.push(("on_accent", on_accent(), "accent_hover", accent_hover()));
        assert_readable(&pairs);
    }

    fn hsla(c: Rgba) -> Hsla {
        Hsla::from(c)
    }

    /// The theme token set a Grove Dark translation would carry, built from today's
    /// fixed helpers, so the palette mapping can be checked without gpui.
    fn grove_theme_colors() -> ThemeColor {
        ThemeColor {
            background: hsla(bg()),
            popover: hsla(panel()),
            sidebar: hsla(sidebar()),
            input: hsla(field()),
            list: hsla(raised()),
            group_box: hsla(card()),
            muted: hsla(disabled()),
            selection: hsla(selected()),
            border: hsla(border()),
            foreground: hsla(text()),
            muted_foreground: hsla(muted()),
            link: hsla(link()),
            accent: hsla(accent()),
            warning: hsla(warn_bg()),
            warning_foreground: hsla(warn_text()),
            sidebar_foreground: hsla(worktree_text()),
            ..ThemeColor::default()
        }
    }

    /// The mapping is true to today's palette: derived tokens sit within a small
    /// lightness step of the fixed helpers, and every derived text/surface pair
    /// keeps the 4.5:1 bar.
    #[test]
    fn grove_tokens_derive_close_to_today_s_palette() {
        let p = palette_from(&grove_theme_colors());
        let pairs = [
            ("bg", p.bg(), bg()),
            ("panel", p.panel(), panel()),
            ("sidebar", p.sidebar(), sidebar()),
            ("field", p.field(), field()),
            ("raised", p.raised(), raised()),
            ("card", p.card(), card()),
            ("disabled", p.disabled(), disabled()),
            ("selected", p.selected(), selected()),
            ("border", p.border(), border()),
            ("border_strong", p.border_strong(), border_strong()),
            ("connector", p.connector(), connector()),
            ("text", p.text(), text()),
            ("text_soft", p.text_soft(), text_soft()),
            ("muted", p.muted(), muted()),
            ("faint", p.faint(), faint()),
            ("link", p.link(), link()),
            ("accent", p.accent(), accent()),
            ("accent_hover", p.accent_hover(), accent_hover()),
            ("accent_bg", p.accent_bg(), accent_bg()),
            ("on_accent", p.on_accent(), on_accent()),
            (
                "on_accent_divider",
                p.on_accent_divider(),
                on_accent_divider(),
            ),
            ("warn_bg", p.warn_bg(), warn_bg()),
            ("warn_text", p.warn_text(), warn_text()),
            ("worktree_row", p.worktree_row(), worktree_row()),
            ("commit_divider", p.commit_divider(), commit_divider()),
            ("worktree_text", p.worktree_text(), worktree_text()),
        ];
        for (name, derived, fixed) in pairs {
            let dl = (derived.l - hsla(fixed).l).abs();
            assert!(dl <= 0.05, "{name} lightness off by {dl:.4}");
        }
        let foregrounds = [
            ("text", p.text()),
            ("text_soft", p.text_soft()),
            ("muted", p.muted()),
            ("faint", p.faint()),
            ("link", p.link()),
            ("accent", p.accent()),
            ("warn_text", p.warn_text()),
            ("worktree_text", p.worktree_text()),
        ];
        let derived_surfaces = [
            ("bg", p.bg()),
            ("panel", p.panel()),
            ("sidebar", p.sidebar()),
            ("field", p.field()),
            ("raised", p.raised()),
            ("card", p.card()),
            ("selected", p.selected()),
            ("border", p.border()),
            ("disabled", p.disabled()),
            ("worktree_row", p.worktree_row()),
            ("accent_bg", p.accent_bg()),
            ("warn_bg", p.warn_bg()),
        ];
        let to_rgba = |h: Hsla| Rgba::from(h);
        let mut readable: Vec<(&str, Rgba, &str, Rgba)> = foregrounds
            .iter()
            .flat_map(|(f, fg)| {
                derived_surfaces
                    .iter()
                    .map(move |(b, bg)| (*f, to_rgba(*fg), *b, to_rgba(*bg)))
            })
            .collect();
        readable.push((
            "on_accent",
            Rgba::from(p.on_accent()),
            "accent",
            Rgba::from(p.accent()),
        ));
        readable.push((
            "on_accent",
            Rgba::from(p.on_accent()),
            "accent_hover",
            Rgba::from(p.accent_hover()),
        ));
        assert_readable(&readable);
    }

    #[test]
    fn text_soft_and_faint_step_between_text_and_muted() {
        let p = palette_from(&grove_theme_colors());
        let (l_text, l_soft, l_muted, l_faint) =
            (p.text().l, p.text_soft().l, p.muted().l, p.faint().l);
        assert!(l_text > l_soft && l_soft > l_muted);
        assert!((l_soft - l_muted).abs() < (l_text - l_muted).abs());
        assert!(l_faint < l_muted && (l_muted - l_faint).abs() < 0.15);
    }

    #[test]
    fn commit_divider_is_halfway_between_panel_and_card() {
        let p = palette_from(&grove_theme_colors());
        let mid = (p.panel().l + p.card().l) / 2.0;
        assert!((p.commit_divider().l - mid).abs() < 0.01);
    }

    #[test]
    fn connector_is_one_stronger_border_step_than_border_strong() {
        let p = palette_from(&grove_theme_colors());
        let strong_step = p.border_strong().l - p.border().l;
        let connector_step = p.connector().l - p.border().l;
        assert!(strong_step > 0.0);
        assert!(connector_step > strong_step);
    }

    #[test]
    fn worktree_row_tint_stays_close_to_the_background() {
        let p = palette_from(&grove_theme_colors());
        let step = p.worktree_row().l - p.bg().l;
        assert!(step > 0.0 && step < 0.1);
        assert!(p.worktree_row().l < p.accent().l);
    }

    #[test]
    fn accent_hover_takes_one_small_darkening_step() {
        let p = palette_from(&grove_theme_colors());
        let step = p.accent().l - p.accent_hover().l;
        assert!(step > 0.0 && (step - ACCENT_HOVER_DARKEN * p.accent().l).abs() < 0.001);
    }

    /// The accents span Grove's orange and pale, vivid, light and dark shapes, the
    /// kinds the bundled themes ship.
    #[test]
    fn on_accent_is_readable_on_representative_accents() {
        let accents = [
            accent(),
            rgb(0xeab308),
            rgb(0x3b82f6),
            rgb(0x7db7ff),
            rgb(0x22c55e),
        ];
        for a in accents {
            let on = on_accent_text(hsla(a));
            assert!(
                contrast_ratio(on, hsla(a)) >= MIN_CONTRAST,
                "on_accent not readable on {:?}",
                hsla(a)
            );
        }
    }

    #[test]
    fn warn_pair_prefers_the_theme_s_tokens() {
        let bg = hsla(bg());
        let warning = hsla(rgb(0xeab308));
        let warning_fg = hsla(rgb(0xfdf3c7));
        let (pair_bg, pair_fg) = warn_pair(bg, warning, warning_fg, hsla(accent()), hsla(text()));
        assert_eq!(pair_fg, warning_fg);
        assert_eq!(pair_bg, blended(bg, warning, WARN_TINT));
    }

    #[test]
    fn warn_pair_falls_back_to_a_readable_accent_tint() {
        let bg = hsla(rgb(0x2a2a2a));
        let warning = hsla(rgb(0x808080));
        let warning_fg = hsla(rgb(0x858585));
        let first_bg = blended(bg, warning, WARN_TINT);
        assert!(contrast_ratio(warning_fg, first_bg) < MIN_CONTRAST);
        let (pair_bg, pair_fg) = warn_pair(bg, warning, warning_fg, hsla(accent()), hsla(text()));
        assert_eq!(pair_bg, blended(bg, hsla(accent()), WARN_TINT));
        assert!(contrast_ratio(pair_fg, pair_bg) >= MIN_CONTRAST);
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

    /// `palette(cx)` projects the active theme's tokens one-to-one; the derivations
    /// are covered by the pure tests above.
    #[gpui_kit::test]
    fn palette_reads_the_active_theme(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            let t = cx.theme();
            let p = palette(cx);
            assert_eq!(p.accent(), t.accent);
            assert_eq!(p.bg(), t.background);
            assert_eq!(p.panel(), t.popover);
            assert_eq!(p.sidebar(), t.sidebar);
            assert_eq!(p.field(), t.input);
            assert_eq!(p.raised(), t.colors.list);
            assert_eq!(p.card(), t.group_box);
            assert_eq!(p.disabled(), t.muted);
            assert_eq!(p.selected(), t.selection);
            assert_eq!(p.border(), t.border);
            assert_eq!(p.text(), t.foreground);
            assert_eq!(p.muted(), t.muted_foreground);
            assert_eq!(p.link(), t.link);
            assert_eq!(p.worktree_text(), t.sidebar_foreground);
        });
    }

    /// Registering Grove Dark as the dark default keeps today's palette: after
    /// the real init path (`gpui_kit::init` + `init`), every direct mapped
    /// token equals its fixed helper exactly. The literal `bg` assertion fails
    /// loudly if the registration ever regresses to gpui-kit's shadcn dark
    /// default, whose background is `#090909`.
    #[gpui_kit::test]
    fn palette_tokens_equal_the_fixed_helpers_under_the_startup_init(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            // Grove Dark is the dark default `Theme::change` re-applies.
            assert_eq!(cx.theme().dark_theme.name, "Grove Dark");
            let p = palette(cx);
            let pairs = [
                ("background", p.bg(), bg()),
                ("popover", p.panel(), panel()),
                ("sidebar", p.sidebar(), sidebar()),
                ("input", p.field(), field()),
                ("list", p.raised(), raised()),
                ("group_box", p.card(), card()),
                ("muted", p.disabled(), disabled()),
                ("selection", p.selected(), selected()),
                ("border", p.border(), border()),
                ("foreground", p.text(), text()),
                ("muted_foreground", p.muted(), muted()),
                ("link", p.link(), link()),
                ("accent", p.accent(), accent()),
                ("sidebar_foreground", p.worktree_text(), worktree_text()),
            ];
            for (token, themed, fixed) in pairs {
                assert_eq!(
                    color_bytes(Rgba::from(themed)),
                    color_bytes(fixed),
                    "{token} token is not Grove Dark"
                );
            }
            // `#121418` survives gpui's Hsla round trip bit-for-bit, so the
            // anchor holds at the raw color too, not just at byte granularity.
            assert_eq!(Rgba::from(p.bg()), rgb(0x121418));
            // The warning pair: the theme's tokens (not the palette's derived
            // `warn_bg`/`warn_text` — those blend with the background and are
            // covered by the ≤0.05-lightness derive test).
            let t = cx.theme();
            assert_eq!(
                color_bytes(Rgba::from(t.warning)),
                color_bytes(warn_bg()),
                "theme warning token is not Grove Dark"
            );
            assert_eq!(
                color_bytes(Rgba::from(t.warning_foreground)),
                color_bytes(warn_text()),
                "theme warning_foreground token is not Grove Dark"
            );
        });
    }
}
