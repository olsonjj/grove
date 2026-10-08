mod details_view;
mod hotkeys;
mod icons;
mod launcher;
mod login_item;
mod macos;
mod main_window;
mod popover;
mod settings_window;
mod setup;
mod shell;
mod status_item;
mod switcher;
#[cfg(test)]
mod test_support;
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
    let app = gpui_kit::application()
        .with_assets(icons::Assets)
        .with_quit_mode(QuitMode::Explicit);
    app.on_reopen(shell::reopen);
    app.run(|cx: &mut App| {
        gpui_kit::init(cx);
        macos::use_dark_appearance();
        shell::init(cx);
        setup::launch(cx);
    });
}
