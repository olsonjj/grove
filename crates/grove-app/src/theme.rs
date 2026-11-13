//! Grove's palette and shared widgets, from the design prototype. Every text
//! color meets 4.5:1 on every surface (see the tests).
//!
//! [`Palette`] (built by [`palette`]) reads Grove's tokens from the active
//! gpui-kit theme and derives Grove's bespoke tints from that theme's tokens.
//! The theme registered by [`init`] — "Grove Dark", translated from the
//! prototype's fixed palette — makes the active theme carry Grove's values, so
//! every surface follows the active theme and Grove Dark renders exactly the
//! colors the fixed helpers used to carry.

use std::rc::Rc;

use gpui_kit::base::Button;
use gpui_kit::component::highlighter::HighlightTheme;
use gpui_kit::component::theme::{Theme, ThemeConfig, ThemeMode, ThemeRegistry};
use gpui_kit::{
    App, ClickEvent, Div, ElementId, FontWeight, InteractiveElement, ParentElement, Rgba,
    SharedString, Styled, Window, div, px, rgb,
};

/// Grove Dark, translated from the prototype's fixed palette into a gpui-kit
/// theme file. Tokens Grove doesn't map stay on gpui-kit's dark defaults via
/// the theme-color fallbacks, and the registry keeps the built-in "Default
/// Dark" for every token this file does not set.
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

/// The [`palette::palette`] entry point, re-exported at this path so call sites read
/// `theme::palette(cx)` while the `palette` module keeps its internals private.
pub(crate) use palette::palette;

/// The selection fill, exactly as the theme file carries it. gpui-kit's apply
/// path clamps theme selection tokens to 30% alpha (its overlay semantics), so
/// [`init`] restates this opaque value after every apply — see the comment
/// there for when the clamp re-runs.
pub(crate) fn selected() -> Rgba {
    rgb(0x263140)
}

/// Grove's cx-aware palette under the active theme.
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
        legend_chip: Hsla,
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

    /// How strongly the tree-view legend chip blends the link color over the
    /// background. The chip is the swatch behind the worktree folder icon in
    /// the tree-view legend (today's hand-tuned value was `#17222f`), and it
    /// takes the same relative step as [`WORKTREE_ROW_TINT`]; the link hue is
    /// shared with the icon so the swatch reads as its surface in any theme.
    const LEGEND_CHIP_TINT: f32 = 0.08;

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
            legend_chip: blended(bg, t.link, LEGEND_CHIP_TINT),
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

        /// Tree-view legend chip behind the worktree folder icon.
        pub fn legend_chip(&self) -> Hsla {
            self.legend_chip
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
pub fn selection_bar(cx: &App) -> Div {
    div()
        .absolute()
        .left(px(1.))
        .top(px(8.))
        .bottom(px(8.))
        .w(px(3.))
        .rounded(px(2.))
        .bg(palette(cx).accent())
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

/// Button; `on_click: None` renders it disabled. Colors come from the active theme
/// by way of [`palette`].
pub fn button(
    cx: &App,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    kind: ButtonKind,
    on_click: Option<impl Fn(&ClickEvent, &mut Window, &mut App) + 'static>,
) -> Button {
    let palette = palette(cx);
    let enabled = on_click.is_some();
    let base = Button::new(id)
        .disabled(!enabled)
        .flex_none()
        .h(px(34.))
        .px(px(14.))
        .rounded(px(8.))
        .border_1()
        .border_color(gpui_kit::transparent_black())
        .focus_visible(|s| s.border_color(palette.link()))
        .text_size(px(13.))
        .child(label.into());
    let styled = match (kind, enabled) {
        (ButtonKind::Primary, true) => base
            .bg(palette.accent())
            .text_color(palette.on_accent())
            .font_weight(FontWeight::SEMIBOLD)
            .hover(|s| s.bg(palette.accent_hover()))
            .focus_visible(|s| s.border_color(palette.text())),
        (ButtonKind::Primary, false) => base
            .bg(palette.disabled())
            .text_color(palette.faint())
            .font_weight(FontWeight::SEMIBOLD),
        (ButtonKind::Secondary, _) => base
            .bg(palette.raised())
            .border_color(palette.border_strong())
            .text_color(palette.text())
            .hover(|s| s.bg(palette.border())),
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

    /// Today's Grove hex values — the palette the deleted fixed helpers carried
    /// and the source the Grove Dark theme file was translated from, plus the
    /// tree-view legend chip literal. The derivation-tolerance tests run against
    /// these, so the mappings stay true to the palette Grove shipped.
    const TODAY: &[(&str, u32)] = &[
        ("bg", 0x121418),
        ("panel", 0x16191e),
        ("sidebar", 0x15181d),
        ("field", 0x0f1114),
        ("raised", 0x1e222a),
        ("card", 0x1b1f26),
        ("disabled", 0x2a2f38),
        ("selected", 0x263140),
        ("border", 0x262b33),
        ("border_strong", 0x34404f),
        ("connector", 0x4a5462),
        ("text", 0xe7e9ec),
        ("text_soft", 0xc9ced6),
        ("muted", 0xa0a7b2),
        ("faint", 0x929aa7),
        ("link", 0x7db7ff),
        ("accent", 0xf0a847),
        ("accent_hover", 0xe2993a),
        ("on_accent_divider", 0xb87a2c),
        ("accent_bg", 0x2e2414),
        ("on_accent", 0x1a1307),
        ("warn_bg", 0x2a2215),
        ("warn_text", 0xf7c987),
        ("legend_chip", 0x17222f),
        ("worktree_row", 0x151d27),
        ("commit_divider", 0x191d23),
        ("worktree_text", 0xbfdbff),
    ];

    /// A Grove hex from [`TODAY`].
    fn today(name: &str) -> Rgba {
        rgb(TODAY
            .iter()
            .find(|(token, _)| *token == name)
            .expect("known token")
            .1)
    }

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

    /// Every text token over every surface under Grove Dark, computed from the
    /// resolved tokens after the real init path — proving the palette plumbing
    /// is true to today's colors, not just to a detached mapping.
    #[gpui_kit::test]
    fn every_text_color_meets_4_5_on_every_surface(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            let p = palette(cx);
            let to_rgba = |h: Hsla| Rgba::from(h);
            let texts = [
                ("text", to_rgba(p.text())),
                ("text_soft", to_rgba(p.text_soft())),
                ("muted", to_rgba(p.muted())),
                ("faint", to_rgba(p.faint())),
                ("link", to_rgba(p.link())),
                ("accent", to_rgba(p.accent())),
                ("warn_text", to_rgba(p.warn_text())),
                ("worktree_text", to_rgba(p.worktree_text())),
            ];
            let surfaces = [
                ("bg", to_rgba(p.bg())),
                ("panel", to_rgba(p.panel())),
                ("sidebar", to_rgba(p.sidebar())),
                ("field", to_rgba(p.field())),
                ("raised", to_rgba(p.raised())),
                ("card", to_rgba(p.card())),
                ("selected", to_rgba(p.selected())),
                ("border", to_rgba(p.border())),
                ("disabled", to_rgba(p.disabled())),
                ("worktree_row", to_rgba(p.worktree_row())),
                ("legend_chip", to_rgba(p.legend_chip())),
                ("accent_bg", to_rgba(p.accent_bg())),
                ("warn_bg", to_rgba(p.warn_bg())),
            ];
            let mut pairs = cross(&texts, &surfaces);
            pairs.push((
                "on_accent",
                to_rgba(p.on_accent()),
                "accent",
                to_rgba(p.accent()),
            ));
            pairs.push((
                "on_accent",
                to_rgba(p.on_accent()),
                "accent_hover",
                to_rgba(p.accent_hover()),
            ));
            assert_readable(&pairs);
        });
    }

    fn hsla(c: Rgba) -> Hsla {
        Hsla::from(c)
    }

    /// The theme token set Grove's translation carries, built from [`TODAY`],
    /// so the palette mapping can be checked without gpui.
    fn grove_theme_colors() -> ThemeColor {
        ThemeColor {
            background: hsla(today("bg")),
            popover: hsla(today("panel")),
            sidebar: hsla(today("sidebar")),
            input: hsla(today("field")),
            list: hsla(today("raised")),
            group_box: hsla(today("card")),
            muted: hsla(today("disabled")),
            selection: hsla(today("selected")),
            border: hsla(today("border")),
            foreground: hsla(today("text")),
            muted_foreground: hsla(today("muted")),
            link: hsla(today("link")),
            accent: hsla(today("accent")),
            warning: hsla(today("warn_bg")),
            warning_foreground: hsla(today("warn_text")),
            sidebar_foreground: hsla(today("worktree_text")),
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
            ("bg", p.bg(), today("bg")),
            ("panel", p.panel(), today("panel")),
            ("sidebar", p.sidebar(), today("sidebar")),
            ("field", p.field(), today("field")),
            ("raised", p.raised(), today("raised")),
            ("card", p.card(), today("card")),
            ("disabled", p.disabled(), today("disabled")),
            ("selected", p.selected(), today("selected")),
            ("border", p.border(), today("border")),
            ("border_strong", p.border_strong(), today("border_strong")),
            ("connector", p.connector(), today("connector")),
            ("text", p.text(), today("text")),
            ("text_soft", p.text_soft(), today("text_soft")),
            ("muted", p.muted(), today("muted")),
            ("faint", p.faint(), today("faint")),
            ("link", p.link(), today("link")),
            ("accent", p.accent(), today("accent")),
            ("accent_hover", p.accent_hover(), today("accent_hover")),
            ("accent_bg", p.accent_bg(), today("accent_bg")),
            ("on_accent", p.on_accent(), today("on_accent")),
            (
                "on_accent_divider",
                p.on_accent_divider(),
                today("on_accent_divider"),
            ),
            ("warn_bg", p.warn_bg(), today("warn_bg")),
            ("warn_text", p.warn_text(), today("warn_text")),
            ("legend_chip", p.legend_chip(), today("legend_chip")),
            ("worktree_row", p.worktree_row(), today("worktree_row")),
            (
                "commit_divider",
                p.commit_divider(),
                today("commit_divider"),
            ),
            ("worktree_text", p.worktree_text(), today("worktree_text")),
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
            ("legend_chip", p.legend_chip()),
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

    /// The legend chip is the background blended one relative step toward the
    /// link color, the same step the worktree row tint takes.
    #[test]
    fn legend_chip_takes_one_bg_to_link_blend_step() {
        let p = palette_from(&grove_theme_colors());
        let step = p.legend_chip().l - p.bg().l;
        assert!(step > 0.0 && step < 0.1);
        assert!(p.legend_chip().l < p.link().l);
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
            today("accent"),
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
        let bg = hsla(today("bg"));
        let warning = hsla(rgb(0xeab308));
        let warning_fg = hsla(rgb(0xfdf3c7));
        let (pair_bg, pair_fg) = warn_pair(
            bg,
            warning,
            warning_fg,
            hsla(today("accent")),
            hsla(today("text")),
        );
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
        let (pair_bg, pair_fg) = warn_pair(
            bg,
            warning,
            warning_fg,
            hsla(today("accent")),
            hsla(today("text")),
        );
        assert_eq!(pair_bg, blended(bg, hsla(today("accent")), WARN_TINT));
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
                ("input", c(t.foreground), "raised", c(t.colors.list)),
                ("input", c(t.foreground), "editor", c(editor)),
                (
                    "placeholder",
                    c(t.muted_foreground),
                    "raised",
                    c(t.colors.list),
                ),
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
    /// the real init path (`gpui_kit::init` + `init`), every direct mapped token
    /// equals the Grove hex it was translated from. The literal `bg` assertion
    /// fails loudly if the registration ever regresses to gpui-kit's shadcn dark
    /// default, whose background is `#090909`.
    #[gpui_kit::test]
    fn palette_tokens_equal_grove_dark_under_the_startup_init(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            // Grove Dark is the dark default `Theme::change` re-applies.
            assert_eq!(cx.theme().dark_theme.name, "Grove Dark");
            let p = palette(cx);
            let pairs = [
                ("background", p.bg(), today("bg")),
                ("popover", p.panel(), today("panel")),
                ("sidebar", p.sidebar(), today("sidebar")),
                ("input", p.field(), today("field")),
                ("list", p.raised(), today("raised")),
                ("group_box", p.card(), today("card")),
                ("muted", p.disabled(), today("disabled")),
                ("selection", p.selected(), today("selected")),
                ("border", p.border(), today("border")),
                ("foreground", p.text(), today("text")),
                ("muted_foreground", p.muted(), today("muted")),
                ("link", p.link(), today("link")),
                ("accent", p.accent(), today("accent")),
                (
                    "sidebar_foreground",
                    p.worktree_text(),
                    today("worktree_text"),
                ),
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
            // `selected()` is the restatement SOURCE for `theme.selection`
            // (init's opaque-fill fix), so the pair assertion above is
            // tautological w.r.t. `grove-dark.json` for this one token — the
            // registry config is parsed straight from that file, so assert it
            // directly and keep the JSON the guarded single source of truth.
            let grove_dark = ThemeRegistry::global(cx)
                .themes()
                .get("Grove Dark")
                .expect("Grove Dark was registered by init")
                .clone();
            assert_eq!(
                grove_dark.colors.selection.as_deref(),
                Some("#263140"),
                "grove-dark.json's selection drifted from today's palette"
            );
            // The warning pair: the theme's tokens (not the palette's derived
            // `warn_bg`/`warn_text` — those blend with the background and are
            // covered by the ≤0.05-lightness derive test).
            let t = cx.theme();
            assert_eq!(
                color_bytes(Rgba::from(t.warning)),
                color_bytes(today("warn_bg")),
                "theme warning token is not Grove Dark"
            );
            assert_eq!(
                color_bytes(Rgba::from(t.warning_foreground)),
                color_bytes(today("warn_text")),
                "theme warning_foreground token is not Grove Dark"
            );
        });
    }
}
