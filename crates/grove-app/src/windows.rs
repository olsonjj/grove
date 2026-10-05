use std::time::Instant;

use gpui_kit::{
    AnyWindowHandle, App, AppContext, Bounds, Context, FocusHandle, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, Render, SharedString, Styled, Window, WindowBounds,
    WindowKind, WindowOptions, div, point, px, rgb, size,
};

use crate::{shell, status_item::Anchor};

const POPOVER_SIZE: (f32, f32) = (360., 480.);
const SWITCHER_SIZE: (f32, f32) = (640., 96.);

fn panel_options(bounds: Bounds<gpui_kit::Pixels>) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
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
fn close_on_deactivate<V: 'static>(window: &mut Window, cx: &mut Context<V>) {
    cx.observe_window_activation(window, |_, window, _| {
        if !window.is_window_active() {
            window.remove_window();
        }
    })
    .detach();
}

// Popover

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
    let result = gpui_kit::open_window(panel_options(bounds), cx, |window, cx| {
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

// Quick switcher

struct Switcher {
    focus: FocusHandle,
    typed: String,
    hotkey_at: Instant,
    rendered: bool,
}

impl Switcher {
    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        eprintln!("[grove] switcher keydown: {ks}");
        match ks.key.as_str() {
            "escape" => {
                window.remove_window();
                return;
            }
            "backspace" => {
                self.typed.pop();
            }
            _ => {
                if let Some(ch) = ks.key_char.as_deref().filter(|_| !ks.modifiers.platform) {
                    self.typed.push_str(ch);
                }
            }
        }
        eprintln!("[grove] switcher typed={:?}", self.typed);
        cx.notify();
    }
}

impl Render for Switcher {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.rendered {
            self.rendered = true;
            let at = self.hotkey_at;
            eprintln!("[grove] latency: hotkey -> first render {:.1} ms", ms(at));
            cx.on_next_frame(window, move |_, _, _| {
                eprintln!("[grove] latency: hotkey -> next frame {:.1} ms", ms(at));
            });
        }
        let text: SharedString = if self.typed.is_empty() {
            "Type to search… (Esc closes)".into()
        } else {
            self.typed.clone().into()
        };
        div()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .size_full()
            .bg(rgb(0x1e1f22))
            .text_color(rgb(0xe6e6e6))
            .text_xl()
            .px_4()
            .flex()
            .items_center()
            .child(text)
    }
}

fn ms(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1000.
}

pub fn open_switcher(hotkey_at: Instant, cx: &mut App) -> Option<AnyWindowHandle> {
    let (w, h) = SWITCHER_SIZE;
    let display = cx.primary_display()?.bounds();
    let x = display.origin.x + (display.size.width - px(w)) / 2.;
    let y = display.origin.y + display.size.height / 4.;
    let bounds = Bounds::new(point(x, y), size(px(w), px(h)));
    eprintln!("[grove] latency: hotkey -> handler {:.1} ms", ms(hotkey_at));
    let result = gpui_kit::open_window(panel_options(bounds), cx, |window, cx| {
        cx.new(|cx| {
            let focus = cx.focus_handle();
            window.focus(&focus, cx);
            close_on_deactivate(window, cx);
            Switcher {
                focus,
                typed: String::new(),
                hotkey_at,
                rendered: false,
            }
        })
    });
    eprintln!(
        "[grove] latency: hotkey -> open_window returned {:.1} ms",
        ms(hotkey_at)
    );
    result
        .map(|(handle, _)| handle)
        .inspect_err(|e| eprintln!("[grove] switcher: {e:#}"))
        .ok()
}

/// The first GPUI window pays ~190 ms of one-time renderer setup; pay it at launch with a
/// hidden throwaway window so the first ⌥⌘K is fast.
pub fn warm_up(cx: &mut App) {
    let start = Instant::now();
    let bounds = Bounds::new(point(px(0.), px(0.)), size(px(1.), px(1.)));
    let options = WindowOptions {
        focus: false,
        show: false,
        ..panel_options(bounds)
    };
    if let Ok((handle, _)) = gpui_kit::open_window(options, cx, |_, cx| cx.new(|_| Popover)) {
        shell::close(handle, cx);
    }
    eprintln!("[grove] renderer warm-up {:.1} ms", ms(start));
}
