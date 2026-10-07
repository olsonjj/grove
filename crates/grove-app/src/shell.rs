use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use global_hotkey::GlobalHotKeyManager;
use gpui_kit::{AnyWindowHandle, App, AppContext, Global, WindowId};
use grove_core::{
    Changes, FetchReport, Settings, SettingsFile, Snapshot, SnapshotCache, SnapshotStore,
    StatusReads, Watcher,
};
use objc2::rc::Retained;
use objc2_app_kit::NSRunningApplication;
use tray_icon::TrayIcon;

use crate::{PlatformEvent, hotkeys, macos, main_window, status_item, windows};

/// A status-item click that arrives right after the popover closed on deactivation is the
/// same click that deactivated it; ignore it so the click toggles instead of reopening.
const REOPEN_GUARD: Duration = Duration::from_millis(300);
/// Window shows and activations within this of the last full refresh don't start another;
/// the watcher covers the gap.
const RESCAN_GUARD: Duration = Duration::from_secs(2);
/// Snapshot changes are written to the cache at most this often.
const CACHE_DELAY: Duration = Duration::from_secs(2);
const BACKGROUND_FETCH: Duration = Duration::from_secs(15 * 60);

/// Settings and snapshot the windows render from. A global of its own so windows don't
/// depend on the platform shell (tests set it alone).
pub struct Library {
    pub settings_file: SettingsFile,
    pub settings: Settings,
    pub store: SnapshotStore,
    pub saves: Saves,
}

impl Global for Library {}

/// Orders writes of one file (settings, cache): saves run on background threads, so a newer one can finish
/// first; an older one arriving after it is dropped instead of overwriting it.
#[derive(Default)]
pub struct Saves {
    issued: u64,
    written: Arc<Mutex<u64>>,
}

/// Fetch all's progress and outcome, for the status bar.
#[derive(Default)]
pub struct Fetcher {
    pub running: bool,
    /// Bumped per finished fetch, so windows show each report once.
    pub finished: u64,
    pub report: Option<FetchReport>,
}

impl Global for Fetcher {}

/// Keeps the snapshot fresh: the watcher, full refreshes, and the on-disk cache.
struct Live {
    /// Snapshots for the watcher to follow; `None` if it couldn't start.
    watch: Option<async_channel::Sender<Arc<Snapshot>>>,
    cache: SnapshotCache,
    cache_saves: Saves,
    cache_pending: bool,
    scanning: bool,
    last_scan: Option<Instant>,
}

impl Global for Live {}

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

/// Installs the status item and hotkeys, starts watching, and opens the main window. Only
/// called once setup is complete, so a half-configured Grove has no menu bar icon or
/// shortcuts. A `cached` snapshot is shown as is while a full refresh runs.
pub fn start(library: Library, cached: bool, cx: &mut App) {
    let (tx, rx) = async_channel::unbounded::<PlatformEvent>();
    // Either can fail, e.g. a second Grove can't take the same hotkeys; keep running.
    let tray = status_item::install(tx.clone())
        .inspect_err(|e| eprintln!("[grove] status item unavailable: {e}"))
        .ok();
    let hotkeys = hotkeys::install(tx)
        .inspect_err(|e| eprintln!("[grove] global hotkeys unavailable: {e}"))
        .ok();
    cx.set_global(Live {
        watch: None,
        cache: SnapshotCache::beside(&library.settings_file),
        cache_saves: Saves::default(),
        cache_pending: false,
        scanning: false,
        last_scan: None,
    });
    cx.set_global(library);
    if cached {
        rescan(cx);
    } else {
        cx.global_mut::<Live>().last_scan = Some(Instant::now());
        read_statuses(cx);
    }
    start_watching(cx);
    start_background_fetch(cx);
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
/// snapshot as they land so badges fill in without blocking the UI. Results for a snapshot
/// that's since been replaced are dropped.
pub fn read_statuses(cx: &mut App) {
    let reads = cx.global_mut::<Library>().store.status_reads();
    run_reads(reads, cx);
}

/// Rereads one worktree's status in the background, e.g. after its folder came back.
pub fn read_status(path: &Path, cx: &mut App) {
    let reads = cx.global_mut::<Library>().store.status_read(path);
    run_reads(reads, cx);
}

fn run_reads(reads: StatusReads, cx: &mut App) {
    let stamp = reads.stamp();
    let (tx, rx) = async_channel::unbounded();
    cx.background_spawn(async move {
        reads.run(|path, status| {
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
                let store = &mut cx.global_mut::<Library>().store;
                let mut changed = false;
                for (path, status) in batch {
                    changed |= store.apply_status(stamp, &path, status);
                }
                if changed {
                    save_cache_soon(cx);
                }
            });
        }
    })
    .detach();
}

/// Full refresh (rescan plus every status) in the background, unless one is running or
/// just ran. Shown data stays until the result swaps in.
pub fn refresh_all(cx: &mut App) {
    let Some(live) = cx.try_global::<Live>() else {
        return;
    };
    if live.scanning || live.last_scan.is_some_and(|t| t.elapsed() < RESCAN_GUARD) {
        return;
    }
    rescan(cx);
}

fn rescan(cx: &mut App) {
    let live = cx.global_mut::<Live>();
    live.scanning = true;
    live.last_scan = Some(Instant::now());
    let read = cx.global_mut::<Library>().store.scan_read();
    cx.spawn(async move |cx| {
        let result = cx.background_spawn(async move { read.run() }).await;
        cx.update(|cx| {
            cx.global_mut::<Live>().scanning = false;
            if cx.global_mut::<Library>().store.apply_scan(result) {
                snapshot_changed(cx);
            }
        });
    })
    .detach();
}

/// `git fetch --all --prune` in every repo, concurrently and off the UI thread, then
/// rereads every status for the new ahead/behind and upstream gone. No-op while one runs.
pub fn fetch_all(cx: &mut App) {
    if !cx.has_global::<Library>() || cx.default_global::<Fetcher>().running {
        return;
    }
    cx.global_mut::<Fetcher>().running = true;
    let read = cx.global_mut::<Library>().store.fetch_read();
    cx.spawn(async move |cx| {
        let result = cx.background_spawn(async move { read.run() }).await;
        cx.update(|cx| {
            let report = cx.global_mut::<Library>().store.apply_fetch(result);
            let fetcher = cx.global_mut::<Fetcher>();
            fetcher.running = false;
            let Some(report) = report else {
                return;
            };
            fetcher.finished += 1;
            fetcher.report = Some(report);
            save_cache_soon(cx);
        });
    })
    .detach();
}

/// Fetch all every 15 minutes while background fetch is on in settings.
pub(crate) fn start_background_fetch(cx: &mut App) {
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(BACKGROUND_FETCH).await;
            cx.update(|cx| {
                if library(cx).settings.background_fetch {
                    fetch_all(cx);
                }
            });
        }
    })
    .detach();
}

/// Starts the watcher off the UI thread; its batches come back here to be read in the
/// background and applied.
fn start_watching(cx: &mut App) {
    let (changes_tx, changes_rx) = async_channel::unbounded::<Changes>();
    let (watch_tx, watch_rx) = async_channel::unbounded::<Arc<Snapshot>>();
    let first = library(cx).store.snapshot().clone();
    cx.background_spawn(async move {
        let watcher = Watcher::start(&first, move |changes| {
            let _ = changes_tx.send_blocking(changes);
        });
        let watcher = match watcher {
            Ok(w) => w,
            Err(e) => return eprintln!("[grove] {e}"),
        };
        // One task, so watch-set updates apply in order; only the newest matters.
        while let Ok(mut snapshot) = watch_rx.recv().await {
            while let Ok(newer) = watch_rx.try_recv() {
                snapshot = newer;
            }
            watcher.update(&snapshot);
        }
    })
    .detach();
    cx.global_mut::<Live>().watch = Some(watch_tx);
    cx.spawn(async move |cx| {
        while let Ok(changes) = changes_rx.recv().await {
            cx.update(|cx| apply_changes(changes, cx));
        }
    })
    .detach();
}

fn apply_changes(changes: Changes, cx: &mut App) {
    for git_dir in &changes.repos {
        let Some(read) = cx.global_mut::<Library>().store.repo_read(git_dir) else {
            continue;
        };
        cx.spawn(async move |cx| {
            let result = cx.background_spawn(async move { read.run() }).await;
            cx.update(|cx| {
                if cx.global_mut::<Library>().store.apply_repo(result) {
                    snapshot_changed(cx);
                }
            });
        })
        .detach();
    }
    for path in &changes.worktrees {
        read_status(path, cx);
    }
}

/// After repos or worktrees may have come or gone: re-aim the watcher, update the cache.
fn snapshot_changed(cx: &mut App) {
    let snapshot = library(cx).store.snapshot().clone();
    if let Some(watch) = &cx.global::<Live>().watch {
        let _ = watch.try_send(snapshot);
    }
    save_cache_soon(cx);
}

fn save_cache_soon(cx: &mut App) {
    let Some(live) = cx.try_global::<Live>() else {
        return;
    };
    if live.cache_pending {
        return;
    }
    cx.global_mut::<Live>().cache_pending = true;
    cx.spawn(async move |cx| {
        cx.background_executor().timer(CACHE_DELAY).await;
        cx.update(|cx| {
            cx.global_mut::<Live>().cache_pending = false;
            let snapshot = library(cx).store.snapshot().clone();
            let live = cx.global_mut::<Live>();
            let cache = live.cache.clone();
            live.cache_saves.issue().write(cx, move || {
                cache.save(&snapshot).inspect_err(|e| {
                    eprintln!("[grove] caching to {}: {e}", cache.path().display())
                })
            });
        });
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
    let (file, settings) = (library.settings_file.clone(), library.settings.clone());
    library.saves.issue().write(cx, move || {
        file.save(&settings)
            .inspect_err(|e| eprintln!("[grove] saving settings to {}: {e}", file.path().display()))
    });
}

impl Saves {
    fn issue(&mut self) -> SaveTicket {
        self.issued += 1;
        SaveTicket {
            generation: self.issued,
            written: self.written.clone(),
        }
    }
}

struct SaveTicket {
    generation: u64,
    written: Arc<Mutex<u64>>,
}

impl SaveTicket {
    /// Runs `write` on a background thread, unless a later-issued write already finished.
    fn write(self, cx: &App, write: impl FnOnce() -> io::Result<()> + Send + 'static) {
        cx.background_spawn(async move {
            let mut written = self.written.lock().unwrap_or_else(|e| e.into_inner());
            if *written <= self.generation && write().is_ok() {
                *written = self.generation;
            }
        })
        .detach();
    }
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
                refresh_all(cx);
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
            refresh_all(cx);
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
