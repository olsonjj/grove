mod hotkeys;
mod icons;
mod macos;
mod main_window;
mod setup;
mod shell;
mod status_item;
mod theme;
mod tree_view;
mod windows;

use std::time::Instant;

use gpui_kit::{App, QuitMode};

/// Events from platform callbacks (status item, hotkeys), forwarded into GPUI's foreground loop.
pub enum PlatformEvent {
    StatusItemClicked { anchor: status_item::Anchor },
    ToggleSwitcher { at: Instant },
    ShowMain,
}

fn main() {
    gpui_kit::application()
        .with_assets(icons::Assets)
        .with_quit_mode(QuitMode::Explicit)
        .run(|cx: &mut App| {
            gpui_kit::init(cx);
            macos::use_dark_appearance();
            cx.on_window_closed(|cx, id| {
                if shell::is_running(cx) {
                    shell::on_window_closed(cx, id);
                } else {
                    // Setup closed before finishing: nothing else can bring Grove back.
                    cx.quit();
                }
            })
            .detach();
            setup::launch(cx);
        });
}
