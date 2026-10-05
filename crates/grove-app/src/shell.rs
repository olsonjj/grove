use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use global_hotkey::GlobalHotKeyManager;
use gpui_kit::{AnyWindowHandle, App, AppContext, Global, WindowId};
use grove_core::{Settings, SettingsFile, Snapshot};
use objc2::rc::Retained;
use objc2_app_kit::NSRunningApplication;
use tray_icon::TrayIcon;

use crate::{PlatformEvent, hotkeys, macos, main_window, status_item, windows};

/// A status-item click that arrives right after the popover closed on deactivation is the
/// same click that deactivated it; ignore it so the click toggles instead of reopening.
const REOPEN_GUARD: Duration = Duration::from_millis(300);

/// Settings and snapshot the windows render from. A global of its own so windows don't
/// depend on the platform shell (tests set it alone).
pub struct Library {
    pub settings_file: SettingsFile,
    pub settings: Settings,
    pub snapshot: Arc<Snapshot>,
    pub saves: Saves,
}

impl Global for Library {}

/// Orders settings writes: saves run on background threads, so a newer one can finish
/// first; an older one arriving after it is dropped instead of overwriting it.
#[derive(Default)]
pub struct Saves {
    issued: u64,
    written: Arc<Mutex<u64>>,
}

struct Shell {
    _tray: Option<TrayIcon>,
    _hotkeys: Option<GlobalHotKeyManager>,
    popover: Option<AnyWindowHandle>,
    popover_closed_at: Option<Instant>,
    switcher: Option<AnyWindowHandle>,
    main: Option<AnyWindowHandle>,
    prev_app: Option<Retained<NSRunningApplication>>,
}

impl Global for Shell {}

/// Installs the status item and hotkeys and opens the main window. Only called once setup
/// is complete, so a half-configured Grove has no menu bar icon or shortcuts.
pub fn start(library: Library, cx: &mut App) {
    let (tx, rx) = async_channel::unbounded::<PlatformEvent>();
    // Either can fail, e.g. a second Grove can't take the same hotkeys; keep running.
    let tray = status_item::install(tx.clone())
        .inspect_err(|e| eprintln!("[grove] status item unavailable: {e}"))
        .ok();
    let hotkeys = hotkeys::install(tx)
        .inspect_err(|e| eprintln!("[grove] global hotkeys unavailable: {e}"))
        .ok();
    cx.set_global(library);
    read_statuses(cx);
    crate::launcher::install(cx);
    cx.set_global(Shell {
        _tray: tray,
        _hotkeys: hotkeys,
        popover: None,
        popover_closed_at: None,
        switcher: None,
        main: None,
        prev_app: None,
    });
    windows::warm_up(cx);
    cx.spawn(async move |cx| {
        while let Ok(event) = rx.recv().await {
            cx.update(|cx| handle(event, cx));
        }
    })
    .detach();
    handle(PlatformEvent::ShowMain, cx);
}

pub fn is_running(cx: &App) -> bool {
    cx.has_global::<Shell>()
}

pub fn library(cx: &App) -> &Library {
    cx.global::<Library>()
}

/// Reads every worktree's git status on background threads, applying results to the
/// snapshot as they land so badges fill in without blocking the UI.
pub fn read_statuses(cx: &mut App) {
    let paths = library(cx).snapshot.status_targets();
    let (tx, rx) = async_channel::unbounded();
    cx.background_spawn(async move {
        grove_core::read_statuses(&paths, |path, status| {
            let _ = tx.send_blocking((path.to_path_buf(), status));
        });
    })
    .detach();
    cx.spawn(async move |cx| {
        while let Ok(first) = rx.recv().await {
            let mut batch = vec![first];
            while let Ok(more) = rx.try_recv() {
                batch.push(more);
            }
            cx.update(|cx| {
                let snapshot = &mut cx.global_mut::<Library>().snapshot;
                let snapshot = Arc::make_mut(snapshot);
                for (path, status) in batch {
                    snapshot.set_status(&path, status);
                }
            });
        }
    })
    .detach();
}

pub fn select_repo(name: &str, cx: &mut App) {
    let library = cx.global_mut::<Library>();
    if library.settings.last_selected_repo.as_deref() == Some(name) {
        return;
    }
    library.settings.last_selected_repo = Some(name.to_string());
    save_settings(cx);
}

pub fn set_collapsed(repo: &str, prefix: &str, collapsed: bool, cx: &mut App) {
    let library = cx.global_mut::<Library>();
    if library.settings.set_collapsed(repo, prefix, collapsed) {
        save_settings(cx);
    }
}

fn save_settings(cx: &mut App) {
    let library = cx.global_mut::<Library>();
    library.saves.issued += 1;
    let generation = library.saves.issued;
    let written = library.saves.written.clone();
    let (file, settings) = (library.settings_file.clone(), library.settings.clone());
    cx.background_spawn(async move {
        let mut written = written.lock().unwrap_or_else(|e| e.into_inner());
        if *written > generation {
            return;
        }
        match file.save(&settings) {
            Ok(()) => *written = generation,
            Err(e) => eprintln!("[grove] saving settings to {}: {e}", file.path().display()),
        }
    })
    .detach();
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
                let handle = main_window::open(cx);
                cx.global_mut::<Shell>().main = handle;
            }
            eprintln!("[grove] main window shown");
        }
    }
}

pub fn close(window: AnyWindowHandle, cx: &mut App) {
    let _ = window.update(cx, |_, window, _| window.remove_window());
}

pub fn on_window_closed(cx: &mut App, id: WindowId) {
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
