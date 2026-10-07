use std::time::Instant;

use gpui_kit::{
    AnyWindowHandle, App, AppContext, Bounds, Context, DisplayId, IntoElement, ParentElement,
    Render, Styled, Window, WindowBounds, WindowKind, WindowOptions, div, point, px, rgb, size,
};

use crate::{shell, status_item::Anchor};

const POPOVER_SIZE: (f32, f32) = (360., 480.);

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

struct Popover;

impl Render for Popover {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(0x1e1f22))
            .text_color(rgb(0x9da0a6))
            .p_3()
            .child("Grove popover (empty)")
    }
}

pub fn open_popover(anchor: Anchor, cx: &mut App) -> Option<AnyWindowHandle> {
    let (w, h) = POPOVER_SIZE;
    let x = (anchor.x + anchor.width / 2. - w / 2.).max(8.);
    let y = anchor.y + anchor.height + 4.;
    let bounds = Bounds::new(point(px(x), px(y)), size(px(w), px(h)));
    eprintln!("[grove] popover open: anchor={anchor:?} bounds={bounds:?}");
    let result = gpui_kit::open_window(panel_options(bounds, None), cx, |window, cx| {
        cx.new(|cx| {
            close_on_deactivate(window, cx);
            Popover
        })
    });
    result
        .map(|(handle, _)| handle)
        .inspect_err(|e| eprintln!("[grove] popover: {e:#}"))
        .ok()
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
    if let Ok((handle, _)) = gpui_kit::open_window(options, cx, |_, cx| cx.new(|_| Popover)) {
        shell::close(handle, cx);
    }
    eprintln!("[grove] renderer warm-up {:.1} ms", ms(start));
}
