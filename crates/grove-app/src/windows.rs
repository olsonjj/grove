use std::time::Instant;

use gpui_kit::{
    AnyWindowHandle, App, AppContext, Bounds, Context, DisplayId, Entity, Global, IntoElement,
    Render, Subscription, Window, WindowBounds, WindowKind, WindowOptions, div, point, px, size,
};

use crate::{shell, theme};

/// `bounds` are relative to `display`, the primary one if `None`.
pub fn panel_options(
    bounds: Bounds<gpui_kit::Pixels>,
    display: Option<DisplayId>,
) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        display_id: display,
        titlebar: None,
        focus: true,
        show: true,
        // PopUp = non-activating NSPanel at popup level: can take key focus without
        // activating Grove, so the frontmost app keeps its activation.
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        ..Default::default()
    }
}

/// Closes `window` when it stops being key (click outside).
pub fn close_on_deactivate<V: 'static>(window: &mut Window, cx: &mut Context<V>) {
    cx.observe_window_activation(window, |_, window, _| {
        if !window.is_window_active() {
            window.remove_window();
        }
    })
    .detach();
}

/// The appearance-changed subscriptions of every window Grove opened, keeping the
/// per-window registration alive for the window's life (spec: Follow System tracks
/// macOS live, per visible window). Windows are few; held until Grove quits.
#[derive(Default)]
struct AppearanceWatch(Vec<Subscription>);

impl Global for AppearanceWatch {}

/// Every window Grove opens: registers gpui's appearance-changed callback at
/// creation — inside the window's build, so it stands before the creation
/// notification gpui defers onto every window flushes — and otherwise exactly
/// `gpui_kit::open_window` (see [`theme::appearance_changed`] for the callback,
/// [`theme::follow_appearance_changed`] for what it applies).
pub fn open_window<V: Render>(
    options: WindowOptions,
    cx: &mut App,
    build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) -> gpui_kit::Result<(AnyWindowHandle, Entity<V>)> {
    let mut watch = None;
    let opened = gpui_kit::open_window(options, cx, |window, cx| {
        let view = build(window, cx);
        watch = Some(window.observe_window_appearance(theme::appearance_changed));
        view
    });
    match opened {
        Ok(opened) => {
            if let Some(subscription) = watch {
                cx.default_global::<AppearanceWatch>().0.push(subscription);
            }
            Ok(opened)
        }
        Err(e) => Err(e),
    }
}

/// Draws nothing; the warm-up window's view.
struct Blank;

impl Render for Blank {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

pub fn ms(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1000.
}

/// The first GPUI window pays ~190 ms of one-time renderer setup; pay it at launch with a
/// hidden throwaway window so the first ⌥⌘K is fast.
pub fn warm_up(cx: &mut App) {
    let start = Instant::now();
    let bounds = Bounds::new(point(px(0.), px(0.)), size(px(1.), px(1.)));
    let options = WindowOptions {
        focus: false,
        show: false,
        ..panel_options(bounds, None)
    };
    if let Ok((handle, _)) = open_window(options, cx, |_, cx| cx.new(|_| Blank)) {
        shell::close(handle, cx);
    }
    eprintln!("[grove] renderer warm-up {:.1} ms", ms(start));
}
