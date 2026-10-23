use gpui_kit::{
    AnyWindowHandle, App, AppContext, Bounds, Entity, TitlebarOptions, WindowBounds, WindowKind,
    WindowOptions, px, size,
};

use super::MainWindow;

/// The main window's minimum size.
pub(super) const MIN_SIZE: gpui_kit::Size<gpui_kit::Pixels> = size(px(1000.), px(560.));

pub fn open(cx: &mut App) -> Option<AnyWindowHandle> {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(1180.), px(720.)),
            cx,
        ))),
        window_min_size: Some(MIN_SIZE),
        titlebar: Some(TitlebarOptions {
            title: Some("Grove".into()),
            ..Default::default()
        }),
        focus: true,
        show: true,
        kind: WindowKind::Normal,
        ..Default::default()
    };
    open_with(options, cx)
        .map(|(handle, _)| handle)
        .inspect_err(|e| eprintln!("[grove] main: {e:#}"))
        .ok()
}

pub(super) fn open_with(
    options: WindowOptions,
    cx: &mut App,
) -> gpui_kit::Result<(AnyWindowHandle, Entity<MainWindow>)> {
    gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| MainWindow::new(window, cx))
    })
}
