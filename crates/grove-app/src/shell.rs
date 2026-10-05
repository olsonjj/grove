use std::time::{Duration, Instant};

use global_hotkey::GlobalHotKeyManager;
use gpui_kit::{AnyWindowHandle, App, Global, WindowId};
use objc2::rc::Retained;
use objc2_app_kit::NSRunningApplication;
use tray_icon::TrayIcon;

use crate::{PlatformEvent, macos, windows};

/// A status-item click that arrives right after the popover closed on deactivation is the
/// same click that deactivated it; ignore it so the click toggles instead of reopening.
const REOPEN_GUARD: Duration = Duration::from_millis(300);

struct Shell {
    _tray: TrayIcon,
    _hotkeys: GlobalHotKeyManager,
    popover: Option<AnyWindowHandle>,
    popover_closed_at: Option<Instant>,
    switcher: Option<AnyWindowHandle>,
    main: Option<AnyWindowHandle>,
    prev_app: Option<Retained<NSRunningApplication>>,
}

impl Global for Shell {}

pub fn init(cx: &mut App, tray: TrayIcon, hotkeys: GlobalHotKeyManager) {
    cx.set_global(Shell {
        _tray: tray,
        _hotkeys: hotkeys,
        popover: None,
        popover_closed_at: None,
        switcher: None,
        main: None,
        prev_app: None,
    });
    cx.on_window_closed(on_window_closed).detach();
}

pub fn handle(event: PlatformEvent, cx: &mut App) {
    match event {
        PlatformEvent::StatusItemClicked { anchor } => {
            let shell = cx.global::<Shell>();
            if let Some(popover) = shell.popover {
                close(popover, cx);
            } else if shell
                .popover_closed_at
                .is_none_or(|t| t.elapsed() > REOPEN_GUARD)
            {
                let handle = windows::open_popover(anchor, cx);
                cx.global_mut::<Shell>().popover = handle;
            }
        }
        PlatformEvent::ToggleSwitcher { at } => {
            if let Some(switcher) = cx.global::<Shell>().switcher {
                close(switcher, cx);
                return;
            }
            let prev = macos::frontmost_other_app();
            eprintln!(
                "[grove] switcher: previous frontmost app = {:?}",
                prev.as_deref().map(macos::app_name)
            );
            let handle = windows::open_switcher(at, cx);
            let shell = cx.global_mut::<Shell>();
            shell.prev_app = prev;
            shell.switcher = handle;
        }
        PlatformEvent::ShowMain => {
            cx.activate(true);
            if let Some(main) = cx.global::<Shell>().main {
                let _ = main.update(cx, |_, window, _| window.activate_window());
            } else {
                let handle = windows::open_main(cx);
                cx.global_mut::<Shell>().main = handle;
            }
            eprintln!("[grove] main window shown");
        }
    }
}

pub fn close(window: AnyWindowHandle, cx: &mut App) {
    let _ = window.update(cx, |_, window, _| window.remove_window());
}

fn on_window_closed(cx: &mut App, id: WindowId) {
    let shell = cx.global_mut::<Shell>();
    let is = |h: Option<AnyWindowHandle>| h.is_some_and(|h| h.window_id() == id);
    if is(shell.popover) {
        shell.popover = None;
        shell.popover_closed_at = Some(Instant::now());
        eprintln!("[grove] popover closed");
    } else if is(shell.main) {
        shell.main = None;
    } else if is(shell.switcher) {
        shell.switcher = None;
        let prev = shell.prev_app.take();
        // The switcher is a non-activating panel, so the previous app normally keeps
        // focus on its own; only hand it back if Grove got activated meanwhile.
        if macos::is_app_active()
            && let Some(prev) = &prev
        {
            macos::activate(prev);
        }
        eprintln!("[grove] switcher closed");
        cx.spawn(async move |cx| {
            cx.background_executor()
                .timer(Duration::from_millis(250))
                .await;
            let front = objc2_app_kit::NSWorkspace::sharedWorkspace().frontmostApplication();
            eprintln!(
                "[grove] after switcher close: frontmost app = {:?} (expected {:?})",
                front.as_deref().map(macos::app_name),
                prev.as_deref().map(macos::app_name)
            );
        })
        .detach();
    }
}
