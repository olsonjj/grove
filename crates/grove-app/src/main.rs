mod hotkeys;
mod macos;
mod shell;
mod status_item;
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
    macos::force_accessory_activation_policy();
    gpui_kit::application()
        .with_quit_mode(QuitMode::Explicit)
        .run(|cx: &mut App| {
            gpui_kit::init(cx);
            macos::set_accessory_activation_policy();

            let (tx, rx) = async_channel::unbounded::<PlatformEvent>();
            let tray = status_item::install(tx.clone()).expect("status item");
            let hotkeys = hotkeys::install(tx).expect("global hotkeys");
            shell::init(cx, tray, hotkeys);
            windows::warm_up(cx);

            cx.spawn(async move |cx| {
                while let Ok(event) = rx.recv().await {
                    cx.update(|cx| shell::handle(event, cx));
                }
            })
            .detach();
        });
}
