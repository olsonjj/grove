use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_channel::Sender;
use global_hotkey::GlobalHotKeyManager;
use gpui_kit::component::input::{Copy, Cut, Paste, Redo, SelectAll, Undo};
use gpui_kit::{
    AnyWindowHandle, App, AppContext, Global, KeyBinding, Menu, MenuItem, OsAction, Window,
    WindowId,
};
use grove_core::{
    Changes, FetchReport, Preferences, Settings, SettingsFile, Snapshot, SnapshotCache,
    SnapshotStore, SortMode, StatusReads, Watcher,
};
use objc2::rc::Retained;
use objc2_app_kit::NSRunningApplication;
use tray_icon::TrayIcon;

use crate::{
    PlatformEvent, hotkeys, login_item, macos, main_window, popover, settings_window, setup,
    status_item, switcher, windows,
};

/// A status-item click that arrives right after the popover closed on deactivation is the
/// same click that deactivated it; ignore it so the click toggles instead of reopening.
pub(crate) const REOPEN_GUARD: Duration = Duration::from_millis(300);
/// Window shows and activations within this of the last full refresh don't start another;
/// the watcher covers the gap.
const RESCAN_GUARD: Duration = Duration::from_secs(2);
/// Snapshot changes are written to the cache at most this often.
const CACHE_DELAY: Duration = Duration::from_secs(2);
/// A notice posted while no window could show it is shown by the next one within this.
const NOTICE_FRESH: Duration = Duration::from_secs(60);
const BACKGROUND_FETCH: Duration = Duration::from_secs(15 * 60);

gpui_kit::actions!(
    grove,
    [Quit, Hide, HideOthers, ShowAll, CloseWindow, Minimize]
);

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

/// Completed single-repo rereads: what repo, how many — the Fetcher pattern. A watcher
/// ref hit's reread may change nothing in the repo snapshot yet still change its branch
/// list, so the details pane follows this to keep the branch switcher's dropdown honest.
#[derive(Default)]
pub struct RepoReads {
    /// Bumped per completed reread, so windows read each bump once.
    pub finished: u64,
    /// The repo the last reread was for.
    pub repo: Option<String>,
}

impl Global for RepoReads {}

/// Lines for the main window's status bar from windows that may close before their launch
/// finishes (switcher, popover).
#[derive(Default)]
pub struct Notices {
    /// Bumped per post, so windows show each once.
    pub posted: u64,
    pub last: Option<main_window::Status>,
    pub posted_at: Option<Instant>,
    /// `last` has been on screen.
    pub shown: bool,
}

impl Global for Notices {}

pub fn post(status: main_window::Status, cx: &mut App) {
    let notices = cx.default_global::<Notices>();
    notices.posted += 1;
    notices.last = Some(status);
    notices.posted_at = Some(Instant::now());
    notices.shown = false;
}

/// The last notice, if no window showed it and it's still news.
pub fn unseen_notice(cx: &App) -> Option<main_window::Status> {
    let notices = cx.try_global::<Notices>()?;
    let fresh = notices
        .posted_at
        .is_some_and(|t| t.elapsed() < NOTICE_FRESH);
    notices.last.clone().filter(|_| fresh && !notices.shown)
}

pub fn notice_shown(cx: &mut App) {
    cx.default_global::<Notices>().shown = true;
}

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
    /// `None` while the menu bar icon is off in settings (or couldn't be installed).
    tray: Option<TrayIcon>,
    /// For a status item installed later; `None` in tests.
    events: Option<Sender<PlatformEvent>>,
    _hotkeys: Option<GlobalHotKeyManager>,
    popover: Option<AnyWindowHandle>,
    popover_closed_at: Option<Instant>,
    main: Option<AnyWindowHandle>,
    prev_app: Option<Retained<NSRunningApplication>>,
}

impl Global for Shell {}

impl Shell {
    fn new(
        tray: Option<TrayIcon>,
        events: Option<Sender<PlatformEvent>>,
        hotkeys: Option<GlobalHotKeyManager>,
    ) -> Self {
        Self {
            tray,
            events,
            _hotkeys: hotkeys,
            popover: None,
            popover_closed_at: None,
            main: None,
            prev_app: None,
        }
    }
}

/// Installs the status item and hotkeys, starts watching, and opens the main window. Only
/// called once setup is complete, so a half-configured Grove has no menu bar icon or
/// shortcuts. A `cached` snapshot is shown as is while a full refresh runs.
pub fn start(library: Library, cached: bool, cx: &mut App) {
    let (tx, rx) = async_channel::unbounded::<PlatformEvent>();
    // Either can fail, e.g. a second Grove can't take the same hotkeys; keep running.
    let tray = library
        .settings
        .show_menu_bar_icon
        .then(|| install_tray(tx.clone()))
        .flatten();
    let hotkeys = hotkeys::install(tx.clone())
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
    crate::login_item::sync(cx);
    if cached {
        rescan(cx);
    } else {
        cx.global_mut::<Live>().last_scan = Some(Instant::now());
        read_statuses(cx);
    }
    start_watching(cx);
    start_background_fetch(cx);
    crate::launcher::install(cx);
    cx.set_global(Shell::new(tray, Some(tx), hotkeys));
    windows::warm_up(cx);
    cx.spawn(async move |cx| {
        while let Ok(event) = rx.recv().await {
            cx.update(|cx| handle(event, cx));
        }
    })
    .detach();
    handle(PlatformEvent::ShowMain, cx);
}

/// App-wide hooks, set before any window opens: window bookkeeping, the app menu with ⌘Q,
/// and a flush however Grove quits.
pub fn init(cx: &mut App) {
    cx.on_window_closed(|cx, id| {
        if is_running(cx) {
            on_window_closed(cx, id);
        } else {
            // Setup closed before finishing: nothing else can bring Grove back.
            quit(cx);
        }
    })
    .detach();
    cx.on_app_quit(|cx| {
        flush(cx);
        async {}
    })
    .detach();
    cx.on_action(|_: &Quit, cx| quit(cx));
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    // Closing the main window keeps Grove running (see `on_window_closed`).
    // Popover and switcher have the keys while open, but as non-activating panels they're
    // never GPUI's active (main) window; the main window behind them is.
    cx.on_action(|_: &CloseWindow, cx| {
        if let Some(open) = open_switcher(cx) {
            switcher::dismiss(open, cx);
        } else if let Some(popover) = open_popover(cx) {
            on_window(popover, cx, Window::remove_window);
        } else if let Some(window) = cx.active_window() {
            on_window(window, cx, Window::remove_window);
        }
    });
    cx.on_action(|_: &Minimize, cx| {
        if open_switcher(cx).is_none()
            && open_popover(cx).is_none()
            && let Some(window) = cx.active_window()
        {
            on_window(window, cx, |w| w.minimize_window());
        }
    });
    cx.bind_keys([
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("alt-cmd-h", HideOthers, None),
        KeyBinding::new("cmd-w", CloseWindow, None),
        KeyBinding::new("cmd-m", Minimize, None),
    ]);
    cx.set_menus([
        Menu::new("Grove").items([
            MenuItem::action("Hide Grove", Hide),
            MenuItem::action("Hide Others", HideOthers),
            MenuItem::action("Show All", ShowAll),
            MenuItem::separator(),
            MenuItem::action("Quit Grove", Quit),
        ]),
        Menu::new("Edit").items([
            MenuItem::os_action("Undo", Undo, OsAction::Undo),
            MenuItem::os_action("Redo", Redo, OsAction::Redo),
            MenuItem::separator(),
            MenuItem::os_action("Cut", Cut, OsAction::Cut),
            MenuItem::os_action("Copy", Copy, OsAction::Copy),
            MenuItem::os_action("Paste", Paste, OsAction::Paste),
            MenuItem::os_action("Select All", SelectAll, OsAction::SelectAll),
        ]),
        Menu::new("Window").items([
            MenuItem::action("Minimize", Minimize),
            MenuItem::action("Close Window", CloseWindow),
        ]),
    ]);
}

fn install_tray(tx: Sender<PlatformEvent>) -> Option<TrayIcon> {
    status_item::install(tx)
        .inspect_err(|e| eprintln!("[grove] status item unavailable: {e}"))
        .ok()
}

/// Shows or removes the menu bar icon per settings. The main window and hotkeys don't
/// depend on it.
fn apply_menu_bar_icon(cx: &mut App) {
    let show = library(cx).settings.show_menu_bar_icon;
    if !is_running(cx) {
        return;
    }
    let shell = cx.global_mut::<Shell>();
    if show && shell.tray.is_none() {
        shell.tray = shell.events.clone().and_then(install_tray);
    } else if !show && shell.tray.is_some() {
        shell.tray = None;
        eprintln!("[grove] status item removed");
        if let Some(popover) = shell.popover {
            close(popover, cx);
        }
    }
}

fn open_switcher(cx: &App) -> Option<AnyWindowHandle> {
    switcher::window(cx).filter(|w| cx.windows().contains(w))
}

fn open_popover(cx: &App) -> Option<AnyWindowHandle> {
    cx.try_global::<Shell>().and_then(|s| s.popover)
}

/// Deferred: menu and key actions arrive inside the key window's own update.
fn on_window(window: AnyWindowHandle, cx: &mut App, f: fn(&mut Window)) {
    cx.defer(move |cx| {
        let _ = window.update(cx, |_, window, _| f(window));
    });
}

/// Dock icon clicked while no window shows (GPUI doesn't report other clicks, so with only
/// the popover or switcher up it does nothing). Before setup finishes, setup's window is the
/// only one, e.g. minimized.
pub fn reopen(cx: &mut App) {
    if is_running(cx) {
        handle(PlatformEvent::ShowMain, cx);
    } else if let Some(setup) = cx.windows().into_iter().next() {
        cx.activate(true);
        let _ = setup.update(cx, |_, window, _| window.activate_window());
    }
}

/// Running as after `start`, minus the status item, hotkeys and background work.
#[cfg(test)]
pub fn start_bare(cx: &mut App) {
    cx.set_global(Shell::new(None, None, None));
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
            let library = cx.global_mut::<Library>();
            if library.store.apply_scan(result) {
                let snapshot = library.store.snapshot().clone();
                if library.settings.prune_last_opened(&snapshot) {
                    save_settings(cx);
                }
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
            cx.update(|cx| apply_changes(&changes, cx));
        }
    })
    .detach();
}

fn apply_changes(changes: &Changes, cx: &mut App) {
    for git_dir in &changes.repos {
        reread_repo(git_dir, cx);
    }
    for path in &changes.worktrees {
        read_status(path, cx);
    }
}

/// Rereads one repo's worktrees and statuses in the background: the single-repo refresh
/// that re-derives branch names from the worktree list, so a switched worktree re-nests
/// under its new branch node and the snapshot cache saves. Grove removal and branch
/// switching both end by calling this on their repo.
pub fn reread_repo(git_dir: &Path, cx: &mut App) {
    let Some(read) = cx.global_mut::<Library>().store.repo_read(git_dir) else {
        return;
    };
    let git_dir = git_dir.to_path_buf();
    cx.spawn(async move |cx| {
        let result = cx.background_spawn(async move { read.run() }).await;
        cx.update(|cx| {
            if cx.global_mut::<Library>().store.apply_repo(result) {
                snapshot_changed(cx);
            }
            // A branch deleted or created outside Grove leaves the snapshot unchanged,
            // but the repo's branch list moves, so every completed reread is announced.
            let repo = cx
                .global_mut::<Library>()
                .store
                .snapshot()
                .repos
                .iter()
                .find(|r| r.repo.git_dir == git_dir)
                .map(|r| r.repo.name.clone());
            let reads = cx.default_global::<RepoReads>();
            reads.finished += 1;
            reads.repo = repo;
        });
    })
    .detach();
}

/// After Grove removed or pruned `worktree`: forgets when it was opened and rereads its repo.
pub fn worktree_removed(git_dir: &Path, worktree: &Path, cx: &mut App) {
    if cx
        .global_mut::<Library>()
        .settings
        .forget_worktree(worktree)
    {
        save_settings(cx);
    }
    reread_repo(git_dir, cx);
}

/// Rereads one worktree's status off the UI thread and applies it, e.g. right after Grove
/// pulled it. Like [`read_status`] but for a single path, and without its reader threads
/// waking the UI as results land: one read, one apply.
pub fn reread_status(path: &Path, cx: &mut App) {
    let reads = cx.global_mut::<Library>().store.status_read(path);
    let stamp = reads.stamp();
    cx.spawn(async move |cx| {
        let results = cx.background_spawn(async move { reads.collect() }).await;
        cx.update(|cx| {
            let store = &mut cx.global_mut::<Library>().store;
            let mut changed = false;
            for (path, status) in results {
                changed |= store.apply_status(stamp, &path, status);
            }
            if changed {
                save_cache_soon(cx);
            }
        });
    })
    .detach();
}

/// After repos or worktrees may have come or gone: re-aim the watcher, update the cache.
fn snapshot_changed(cx: &mut App) {
    let snapshot = library(cx).store.snapshot().clone();
    if let Some(watch) = cx.try_global::<Live>().and_then(|l| l.watch.as_ref()) {
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

pub fn set_repo_sort(mode: SortMode, cx: &mut App) {
    if cx.global_mut::<Library>().settings.set_repo_sort(mode) {
        save_settings(cx);
    }
}

pub fn set_tree_sort(mode: SortMode, cx: &mut App) {
    if cx.global_mut::<Library>().settings.set_tree_sort(mode) {
        save_settings(cx);
    }
}

pub fn toggle_pin(repo: &Path, cx: &mut App) {
    if cx.global_mut::<Library>().settings.toggle_pin(repo) {
        save_settings(cx);
    }
}

/// Hides or unhides the repo at `repo` from the desktop sidebar; display-only.
pub fn toggle_hidden(repo: &Path, cx: &mut App) {
    if cx.global_mut::<Library>().settings.toggle_hidden(repo) {
        save_settings(cx);
    }
}

/// Puts `worktree` first in recents (the switcher's empty search), stamps it opened now
/// (Recent sort) and saves.
pub fn record_open(worktree: &Path, cx: &mut App) {
    let now = grove_core::unix_now();
    if cx
        .global_mut::<Library>()
        .settings
        .record_open(worktree, now)
    {
        save_settings(cx);
    }
}

/// ⌘K and Jump to…: the same switcher as ⌥⌘K.
pub fn toggle_switcher(cx: &mut App) {
    let at = Instant::now();
    if is_running(cx) {
        handle(PlatformEvent::ToggleSwitcher { at }, cx);
    } else {
        switcher::toggle(at, cx);
    }
}

/// Open Grove and ⌥⌘G: brings up the main window, opening it if it's closed.
pub fn show_main(cx: &mut App) {
    if is_running(cx) {
        handle(PlatformEvent::ShowMain, cx);
    } else {
        main_window::open(cx);
    }
}

/// Settings… (top bar, popover, status-line links).
pub fn open_settings(cx: &mut App) {
    settings_window::open(cx);
}

/// The sidebar's Change…: the first-run picker; Continue switches right away.
pub fn choose_code_dir(cx: &mut App) {
    setup::change_code_dir(change_code_dir, cx);
}

/// Switches to another code folder, given its fresh scan: repos outside it drop out of
/// every surface, and the watcher and cache follow.
pub fn change_code_dir(snapshot: Snapshot, cx: &mut App) {
    switch_code_dir(snapshot, cx);
    if cx.has_global::<Live>() {
        cx.global_mut::<Live>().last_scan = Some(Instant::now());
    }
}

/// As `change_code_dir`, but `snapshot` may be old, so the next refresh still rescans.
fn switch_code_dir(snapshot: Snapshot, cx: &mut App) {
    let library = cx.global_mut::<Library>();
    if library.settings.code_dir.as_ref() != Some(&snapshot.code_dir) {
        library.settings.code_dir = Some(snapshot.code_dir.clone());
        save_settings(cx);
    }
    eprintln!("[grove] code dir now {}", snapshot.code_dir.display());
    cx.global_mut::<Library>().store.replace(snapshot);
    read_statuses(cx);
    snapshot_changed(cx);
}

/// Settings Save: every surface picks the change up at once. `scanned` is the new code
/// folder's scan, if it changed.
pub fn save_preferences(prefs: Preferences, scanned: Option<Snapshot>, cx: &mut App) {
    cx.global_mut::<Library>().settings.set_preferences(prefs);
    save_settings(cx);
    if let Some(snapshot) = scanned {
        switch_code_dir(snapshot, cx);
    }
    apply_menu_bar_icon(cx);
    login_item::apply(cx);
}

pub(crate) fn save_settings(cx: &mut App) {
    let library = cx.global_mut::<Library>();
    let (file, settings) = (library.settings_file.clone(), library.settings.clone());
    library.saves.issue().write(cx, move || {
        file.save(&settings)
            .inspect_err(|e| eprintln!("[grove] saving settings to {}: {e}", file.path().display()))
    });
}

/// Quit, Grove's only way out (popover, ⌘Q, app menu). The Dock menu's Quit and logout
/// terminate without it, so `init` flushes on any quit too.
pub fn quit(cx: &mut App) {
    flush(cx);
    cx.quit();
}

/// Writes what's still pending, since background saves and the debounced cache die with the
/// process. Writes nothing if called again.
fn flush(cx: &mut App) {
    if cx.has_global::<Library>() {
        let library = cx.global_mut::<Library>();
        let (file, settings) = (library.settings_file.clone(), library.settings.clone());
        if let Some(ticket) = library.saves.unwritten() {
            ticket.write_now(|| {
                file.save(&settings).inspect_err(|e| {
                    eprintln!("[grove] saving settings to {}: {e}", file.path().display())
                })
            });
        }
    }
    if cx.try_global::<Live>().is_some_and(|l| l.cache_pending) {
        let snapshot = library(cx).store.snapshot().clone();
        let live = cx.global_mut::<Live>();
        live.cache_pending = false;
        let cache = live.cache.clone();
        live.cache_saves.issue().write_now(|| {
            cache
                .save(&snapshot)
                .inspect_err(|e| eprintln!("[grove] caching to {}: {e}", cache.path().display()))
        });
    }
}

impl Saves {
    /// A ticket for a final write if an issued one hasn't landed (in flight or failed).
    fn unwritten(&mut self) -> Option<SaveTicket> {
        let written = *self.written.lock().unwrap_or_else(|e| e.into_inner());
        (written < self.issued).then(|| self.issue())
    }

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

    /// Writes on this thread, after any in-flight write (it holds the lock while writing).
    fn write_now(self, write: impl FnOnce() -> io::Result<()>) {
        let mut written = self.written.lock().unwrap_or_else(|e| e.into_inner());
        if *written <= self.generation && write().is_ok() {
            *written = self.generation;
        }
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
                let handle = popover::open(anchor, cx);
                cx.global_mut::<Shell>().popover = handle;
                refresh_all(cx);
            }
        }
        PlatformEvent::ToggleSwitcher { at } => {
            let prev = macos::frontmost_other_app();
            if switcher::toggle(at, cx) {
                cx.global_mut::<Shell>().prev_app = prev;
            }
        }
        PlatformEvent::ShowMain => {
            refresh_all(cx);
            macos::unhide();
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

fn on_window_closed(cx: &mut App, id: WindowId) {
    let is = |h: Option<AnyWindowHandle>| h.is_some_and(|h| h.window_id() == id);
    let switcher = switcher::closed(id, cx);
    let shell = cx.global_mut::<Shell>();
    if is(shell.popover) {
        shell.popover = None;
        shell.popover_closed_at = Some(Instant::now());
        eprintln!("[grove] popover closed");
    } else if is(shell.main) {
        shell.main = None;
    } else if let Some(dismissed) = switcher {
        let prev = shell.prev_app.take();
        // The switcher is a non-activating panel, so the previous app normally keeps focus
        // on its own; hand it back if Grove got activated meanwhile. Not after a click
        // elsewhere: that click chose what's frontmost.
        if dismissed
            && macos::is_app_active()
            && let Some(prev) = &prev
        {
            macos::activate(prev);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{OwnedMenuItem, TestAppContext};
    use grove_core::{Outcome, Settings, SettingsFile};
    use tempfile::TempDir;

    use super::*;
    use crate::test_support::{canned, fake_launcher, set_library, settings_file};

    /// Grove past setup with its main window open and an unsaved settings change.
    fn running(cx: &mut TestAppContext, file: &SettingsFile) -> AnyWindowHandle {
        fake_launcher(cx, Ok(Outcome::Opened));
        let main = cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            set_library(file, vec![canned()], cx);
            start_bare(cx);
            show_main(cx);
            record_open(Path::new("/c/wt/api-y"), cx);
            cx.global::<Shell>().main.unwrap()
        });
        cx.update_window(main, |_, window, _| window.activate_window())
            .unwrap();
        // Not parked: the background settings save hasn't run.
        main
    }

    fn recents(file: &SettingsFile) -> Vec<PathBuf> {
        file.load().unwrap().recents
    }

    fn main_window(cx: &mut TestAppContext) -> Option<AnyWindowHandle> {
        cx.update(|cx| cx.global::<Shell>().main)
    }

    #[gpui_kit::test]
    fn closing_main_window_keeps_grove_running(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let main = running(cx, &file);
        cx.update(|cx| close(main, cx));
        cx.run_until_parked();
        assert!(cx.update(|cx| is_running(cx)));
        assert_eq!(main_window(cx), None);
    }

    #[gpui_kit::test]
    fn show_main_reopens_a_closed_main_window(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let main = running(cx, &file);
        cx.update(|cx| close(main, cx));
        cx.run_until_parked();
        cx.update(|cx| handle(PlatformEvent::ShowMain, cx));
        let reopened = main_window(cx).expect("main window reopened");
        assert!(cx.update(|cx| cx.windows().contains(&reopened)));
    }

    #[gpui_kit::test]
    fn dock_reopen_shows_main_window(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let main = running(cx, &file);
        cx.update(|cx| close(main, cx));
        cx.run_until_parked();
        cx.update(reopen);
        assert!(main_window(cx).is_some());
    }

    #[gpui_kit::test]
    fn dock_reopen_before_setup_finishes_opens_nothing(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            reopen(cx);
        });
        assert!(cx.update(|cx| cx.windows().is_empty()));
    }

    #[gpui_kit::test]
    fn dock_reopen_before_setup_finishes_brings_up_setup(cx: &mut TestAppContext) {
        let setup = cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            let setup = cx
                .open_window(Default::default(), |_, cx| cx.new(|_| gpui_kit::EmptyView))
                .unwrap();
            reopen(cx);
            setup
        });
        cx.run_until_parked();
        assert_eq!(cx.update(|cx| cx.active_window()), Some(setup.into()));
    }

    #[gpui_kit::test]
    fn cmd_q_writes_pending_settings_first(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let main = running(cx, &file);
        cx.update_window(main, |_, window, cx| {
            window.render_frame(cx);
            window.press("cmd-q", cx);
        })
        .unwrap();
        assert_eq!(recents(&file), [PathBuf::from("/c/wt/api-y")]);
    }

    /// The action behind `menu`'s `item`.
    fn menu_action(cx: &mut TestAppContext, menu: &str, item: &str) -> Box<dyn gpui_kit::Action> {
        let menus = cx.update(|cx| cx.get_menus()).unwrap();
        menus
            .into_iter()
            .find(|m| m.name == menu)
            .unwrap()
            .items
            .into_iter()
            .find_map(|i| match i {
                OwnedMenuItem::Action { name, action, .. } if name == item => Some(action),
                _ => None,
            })
            .unwrap()
    }

    #[gpui_kit::test]
    fn menu_quit_writes_pending_settings_first(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        running(cx, &file);
        let quit = menu_action(cx, "Grove", "Quit Grove");
        cx.update(|cx| cx.dispatch_action(quit.as_ref()));
        assert_eq!(recents(&file), [PathBuf::from("/c/wt/api-y")]);
    }

    #[gpui_kit::test]
    fn cmd_w_closes_main_window_and_grove_keeps_running(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let main = running(cx, &file);
        cx.update_window(main, |_, window, cx| {
            window.render_frame(cx);
            window.press("cmd-w", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert!(!cx.update(|cx| cx.windows().contains(&main)));
        assert!(cx.update(|cx| is_running(cx)));
        assert_eq!(main_window(cx), None);
    }

    /// Like Esc: no reopen guard, so the shortcut right after opens it again.
    #[gpui_kit::test]
    fn cmd_w_dismisses_the_switcher(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        running(cx, &file);
        let w = cx.update(|cx| {
            switcher::toggle(Instant::now(), cx);
            switcher::window(cx).unwrap()
        });
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            window.press("cmd-w", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert!(!cx.update(|cx| cx.windows().contains(&w)));
        assert!(cx.update(|cx| switcher::toggle(Instant::now(), cx)));
    }

    #[gpui_kit::test]
    fn cmd_w_closes_the_popover_not_the_main_window_behind_it(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let main = running(cx, &file);
        let anchor = status_item::Anchor {
            x: 600.,
            y: 0.,
            width: 24.,
            height: 24.,
            screen: None,
        };
        let popover = cx.update(|cx| {
            handle(PlatformEvent::StatusItemClicked { anchor }, cx);
            open_popover(cx).unwrap()
        });
        cx.update_window(popover, |_, window, cx| {
            window.render_frame(cx);
            window.press("cmd-w", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(cx.update(|cx| cx.active_window()), Some(main));
        assert_eq!(cx.update(|cx| open_popover(cx)), None);
        assert!(!cx.update(|cx| cx.windows().contains(&popover)));
        assert_eq!(main_window(cx), Some(main));
    }

    #[gpui_kit::test]
    fn menu_close_window_closes_main_window(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let main = running(cx, &file);
        let close = menu_action(cx, "Window", "Close Window");
        cx.update_window(main, |_, window, cx| {
            window.render_frame(cx);
            window.dispatch_action(close, cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(main_window(cx), None);
    }

    /// The Dock menu's Quit and logout terminate the app without going through `quit`.
    #[gpui_kit::test]
    fn system_quit_writes_pending_settings_first(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        running(cx, &file);
        cx.quit();
        assert_eq!(recents(&file), [PathBuf::from("/c/wt/api-y")]);
    }

    /// After each full rescan of a code folder holding a bare repo `web` (no worktrees, so no
    /// status reads), `last_opened` as saved.
    #[gpui_kit::test]
    fn rescan_prunes_last_opened_and_saves_unless_a_repo_is_unreadable(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let mut settings = Settings::default();
        settings.record_open(Path::new("/gone"), 1_700_000_000);
        let file = settings_file(&tmp, settings);
        running(cx, &file);
        let code = TempDir::new().unwrap();
        let ok = std::process::Command::new("git")
            .args(["init", "-q", "--bare", "web"])
            .current_dir(code.path())
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .status()
            .unwrap()
            .success();
        assert!(ok);
        // Looks like a bare repo to discovery, but git can't list its worktrees.
        let broken = code.path().join("broken");
        for dir in ["objects", "refs"] {
            std::fs::create_dir_all(broken.join(dir)).unwrap();
        }
        std::fs::write(broken.join("HEAD"), "garbage").unwrap();
        let rescan = |cx: &mut TestAppContext| {
            cx.update(|cx| {
                cx.set_global(Live {
                    watch: None,
                    cache: SnapshotCache::beside(&file),
                    cache_saves: Saves::default(),
                    cache_pending: false,
                    scanning: false,
                    last_scan: None,
                });
                cx.global_mut::<Library>().store.replace(Snapshot {
                    code_dir: code.path().into(),
                    code_dir_exists: true,
                    repos: vec![],
                    last_fetch: None,
                });
                refresh_all(cx);
            });
            cx.run_until_parked();
            file.load().unwrap().last_opened
        };

        assert!(rescan(cx).contains_key(Path::new("/gone")), "broken repo");
        std::fs::remove_dir_all(&broken).unwrap();
        assert!(rescan(cx).is_empty());
    }

    /// Its scan dates from when the folder was picked, so it mustn't hold off a refresh.
    #[gpui_kit::test]
    fn settings_save_of_a_new_code_folder_lets_refresh_rescan(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        running(cx, &file);
        let code = TempDir::new().unwrap();
        cx.update(|cx| {
            cx.set_global(Live {
                watch: None,
                cache: SnapshotCache::beside(&file),
                cache_saves: Saves::default(),
                cache_pending: false,
                scanning: false,
                last_scan: None,
            });
            let snapshot = Snapshot {
                code_dir: code.path().into(),
                code_dir_exists: true,
                repos: vec![],
                last_fetch: None,
            };
            let prefs = library(cx).settings.preferences();
            save_preferences(prefs, Some(snapshot), cx);
            refresh_all(cx);
            assert!(cx.global::<Live>().scanning);
        });
        cx.run_until_parked();
    }
}
