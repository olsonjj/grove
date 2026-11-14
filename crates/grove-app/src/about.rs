//! About Grove: the small window the Grove menu's About entry opens — the app icon,
//! the version line Settings shows, and the project's GitHub link.

use gpui_kit::IntoElement;
use gpui_kit::base::Button;
use gpui_kit::{
    AnyWindowHandle, App, AppContext, Bounds, ClickEvent, Context, Global, InteractiveElement,
    ParentElement, Render, StatefulInteractiveElement, Styled, Subscription, TestSupportExt,
    TitlebarOptions, Window, WindowBounds, WindowKind, WindowOptions, div, img, px, size,
};
use grove_core::REPO_URL;

use crate::{icons, theme, windows};

/// The app's version line, as Settings shows it — "Grove {version} (build {n})" from
/// the workspace version and the build number, in one place every surface reuses.
pub const VERSION: &str = concat!(
    "Grove ",
    env!("CARGO_PKG_VERSION"),
    " (build ",
    env!("GROVE_BUILD"),
    ")"
);

const SIZE: (f32, f32) = (280., 230.);

/// The open About window and the window-closed observer freeing it; `window(​)`
/// also filters a closed window's stale handle against the windows that still exist.
struct Open {
    window: AnyWindowHandle,
    /// Keeps the window-closed observer alive only while the window is open (dropped
    /// with the global, so open/close cycles don't accumulate observers; the
    /// underscore keeps the never-read lint quiet).
    _closed: Subscription,
}

impl Global for Open {}

pub fn open(cx: &mut App) {
    cx.activate(true);
    if let Some(w) = window(cx) {
        let _ = w.update(cx, |_, window, _| window.activate_window());
        return;
    }
    // When the window closes by any path the slot frees, so the next open builds a
    // new window instead of reactivating the gone one.
    let closed = cx.on_window_closed(|cx, window_id| {
        let gone = cx
            .try_global::<Open>()
            .is_some_and(|open| open.window.window_id() == window_id);
        if gone {
            cx.remove_global::<Open>();
        }
    });
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(SIZE.0), px(SIZE.1)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some("About Grove".into()),
            ..Default::default()
        }),
        focus: true,
        show: true,
        kind: WindowKind::Normal,
        is_resizable: false,
        is_minimizable: false,
        ..Default::default()
    };
    match windows::open_window(options, cx, |_, cx| cx.new(|_| About)) {
        Ok((window, _)) => cx.set_global(Open {
            window,
            _closed: closed,
        }),
        Err(e) => eprintln!("[grove] about: {e:#}"),
    }
}

/// The open About window, if it still exists.
pub fn window(cx: &App) -> Option<AnyWindowHandle> {
    cx.try_global::<Open>()
        .filter(|open| cx.windows().contains(&open.window))
        .map(|open| open.window)
}

/// A tiny static view: nothing is editable, so no draft/Save machinery (the Settings
/// dialog's contrast), and nothing is stateful, so the link is a plain click.
struct About;

impl Render for About {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = theme::palette(cx);
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(14.))
            .px(px(20.))
            .bg(palette.panel())
            .text_color(palette.text())
            .child(
                div().id("about-icon").test_support().flex_none().child(
                    img(icons::APP_ICON)
                        .size(px(96.))
                        .rounded(px(20.))
                        .flex_none(),
                ),
            )
            .child(
                div()
                    .id("about-version")
                    .test_support()
                    .text_size(px(13.))
                    .text_color(palette.muted())
                    .aria_label(VERSION)
                    .child(VERSION),
            )
            .child(
                Button::new("about-repo")
                    .text_size(px(13.))
                    .text_color(palette.link())
                    .accessibility_label("Grove on GitHub, opens in your browser")
                    .child("Grove on GitHub")
                    .on_click(|_: &ClickEvent, _, cx| cx.open_url(REPO_URL)),
            )
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{OwnedMenuItem, TestAppContext};
    use grove_core::{Outcome, Settings, SettingsFile};
    use tempfile::TempDir;

    use crate::shell;
    use crate::test_support::{canned, fake_launcher, menu_action, set_library, settings_file};
    use crate::{settings_window, theme};

    use super::*;

    /// Grove past setup, with an active window every dispatch runs inside.
    fn running(cx: &mut TestAppContext, file: &SettingsFile) -> AnyWindowHandle {
        fake_launcher(cx, Ok(Outcome::Opened));
        let main = cx.update(|cx| {
            gpui_kit::init(cx);
            theme::init(cx);
            shell::init(cx);
            set_library(file, vec![canned()], cx);
            shell::start_bare(cx);
            theme::apply_choice_from(file, cx);
            shell::show_main(cx);
            cx.windows()[0]
        });
        cx.update_window(main, |_, window, _| window.activate_window())
            .unwrap();
        main
    }

    /// The Grove menu's item names and separators, in order.
    fn menu_items(cx: &mut TestAppContext) -> Vec<MenuShape> {
        let menus = cx.update(|cx| cx.get_menus()).unwrap();
        menus
            .into_iter()
            .find(|menu| menu.name == "Grove")
            .unwrap()
            .items
            .into_iter()
            .map(|item| match item {
                OwnedMenuItem::Action { name, .. } => MenuShape::Action(name),
                OwnedMenuItem::Separator => MenuShape::Separator,
                OwnedMenuItem::Submenu(_) | OwnedMenuItem::SystemMenu(_) => MenuShape::Submenu,
            })
            .collect()
    }

    #[derive(Debug, PartialEq)]
    enum MenuShape {
        Action(String),
        Separator,
        Submenu,
    }

    fn dispatch(cx: &mut TestAppContext, action: &dyn gpui_kit::Action) {
        cx.update(|cx| cx.dispatch_action(action));
        cx.run_until_parked();
    }

    fn about(cx: &mut TestAppContext) -> Box<dyn gpui_kit::Action> {
        menu_action(cx, "Grove", "About Grove")
    }

    fn about_window(cx: &mut App) -> Option<AnyWindowHandle> {
        window(cx)
    }

    fn settings_window_open(cx: &mut App) -> Option<AnyWindowHandle> {
        settings_window::window(cx)
    }

    #[gpui_kit::test]
    fn the_grove_menu_opens_with_about_and_settings(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        running(cx, &file);
        // The menu shape updater.md's "Menu" decision fixes; ticket 38 added the
        // "Check for Updates…" slot between About Grove and Settings….
        assert_eq!(
            menu_items(cx),
            [
                MenuShape::Action("About Grove".into()),
                MenuShape::Separator,
                MenuShape::Action("Check for Updates…".into()),
                MenuShape::Action("Settings…".into()),
                MenuShape::Separator,
                MenuShape::Action("Hide Grove".into()),
                MenuShape::Action("Hide Others".into()),
                MenuShape::Action("Show All".into()),
                MenuShape::Separator,
                MenuShape::Action("Quit Grove".into()),
            ]
        );
    }

    #[gpui_kit::test]
    fn about_shows_the_icon_version_and_repo_link(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let main = running(cx, &file);
        let action = about(cx);
        dispatch(cx, action.as_ref());
        let w = cx.update(about_window).expect("the About window opened");

        let contents = cx
            .update_window(w, |_, window, cx| {
                window.render_frame(cx);
                (
                    window.try_find("about-icon").is_some(),
                    window
                        .try_find("about-version")
                        .and_then(|v| v.label().map(str::to_string)),
                    window
                        .try_find("about-repo")
                        .and_then(|link| link.label().map(str::to_string)),
                )
            })
            .unwrap();
        assert_ne!(w, main);
        assert_eq!(
            contents,
            (
                true,
                Some(VERSION.to_string()),
                Some("Grove on GitHub, opens in your browser".into()),
            )
        );
    }

    #[gpui_kit::test]
    fn the_repo_link_opens_the_project_page(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        running(cx, &file);
        let action = about(cx);
        dispatch(cx, action.as_ref());
        let w = cx.update(about_window).expect("the About window opened");
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            window.click("about-repo", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(cx.opened_url(), Some(REPO_URL.into()));
    }

    #[gpui_kit::test]
    fn about_again_focuses_the_existing_window(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        running(cx, &file);
        let action = about(cx);
        dispatch(cx, action.as_ref());
        let first = cx.update(about_window).expect("the About window opened");
        let windows = cx.update(|cx| cx.windows().len());
        let again = about(cx);
        dispatch(cx, again.as_ref());
        assert_eq!(cx.update(about_window), Some(first));
        assert_eq!(cx.update(|cx| cx.windows().len()), windows, "no second one");
    }

    #[gpui_kit::test]
    fn settings_menu_item_opens_the_settings_dialog(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let main = running(cx, &file);
        let item = menu_action(cx, "Grove", "Settings…");
        dispatch(cx, item.as_ref());
        let opened = cx.update(settings_window_open).expect("settings open");
        assert_ne!(opened, main);
    }

    #[gpui_kit::test]
    fn cmd_comma_opens_the_settings_dialog(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let main = running(cx, &file);
        cx.update_window(main, |_, window, cx| window.press("cmd-,", cx))
            .unwrap();
        cx.run_until_parked();
        assert!(cx.update(settings_window_open).is_some());
    }
}
