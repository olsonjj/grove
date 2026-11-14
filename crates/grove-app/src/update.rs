//! Check for Updates…: the Grove-menu entry, the one shared check state, and the result
//! dialog for its three outcomes — up-to-date, a newer release with Download…, and a
//! failed check with Retry (`specs/updater.md`, "Check state single and global" and
//! "Result dialogs"). The launch auto-check shares all of this, except in a dev run
//! (`GROVE_DEV=1`, set by `just run` and `just run-release`): it skips the auto-check
//! entirely — no fetch, no dialog, nothing recorded — while the menu entry still checks.

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::base::Button;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyWindowHandle, App, AppContext, Bounds, ClickEvent, Context, Entity, FontWeight, Global,
    InteractiveElement, IntoElement, ParentElement, Render, StatefulInteractiveElement, Styled,
    Subscription, TestSupportExt, TitlebarOptions, Window, WindowBounds, WindowKind, WindowOptions,
    div, px, size,
};
use grove_core::{UpdateDecision, UpdateFetchError, auto_check_alerts, check_for_update};
use semver::Version;

use crate::{about, shell, theme, windows};

/// The running version as one semver string the check compares against, e.g.
/// `"0.1.0+17"`: the workspace version plus the build number, in one place. The dialog's
/// display words stay [`about::VERSION`], the readout About and Settings show.
pub const RUNNING_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "+", env!("GROVE_BUILD"));

/// The blocking HTTP getter the check's engine runs with: one GET of `url` accepting
/// only a 2xx response, GitHub's standard headers set. Every failure — transport,
/// non-2xx, unreadable body — surfaces as an [`UpdateFetchError`], which the engine maps
/// into its check-failed outcome.
pub type Fetch = Arc<dyn Fn(&str) -> Result<String, UpdateFetchError> + Send + Sync>;

/// Overrides the check's getter (tests), the pull writer's Global-override pattern.
/// Without it, Grove carries [`get()`], the real `ureq` fetch.
pub struct Fetcher {
    /// The getter each check runs with.
    pub fetch: Fetch,
}

impl Global for Fetcher {}

/// Marks a dev run: the launch auto-check ([`auto_check`]) is a silent no-op, while the
/// manual entry ([`check`]) always checks. That's `GROVE_DEV=1`, which `just run` and
/// `just run-release` set; tests override this Global instead of the process env, the
/// [`Fetcher`] override's pattern, so tests don't depend on the environment and parallel
/// tests stay isolated.
struct Dev(bool);

impl Global for Dev {}

/// Makes every update-dialog open fail, for tests: the [`Dev`] override's pattern,
/// standing in for the window-system failure [`open_dialog`] reports through.
#[cfg(test)]
struct UnavailableDialog;

#[cfg(test)]
impl Global for UnavailableDialog {}

/// Whether this launch is a dev run: the [`Dev`] Global's override when set, otherwise
/// whether `GROVE_DEV` is in the environment.
fn dev_run(cx: &App) -> bool {
    cx.try_global::<Dev>()
        .map(|dev| dev.0)
        .unwrap_or_else(|| std::env::var_os("GROVE_DEV").is_some())
}

/// The check's getter, overridden or not.
fn fetcher(cx: &App) -> Fetch {
    cx.try_global::<Fetcher>()
        .map(|f| f.fetch.clone())
        .unwrap_or_else(|| Arc::new(get))
}

/// The production getter (see [`Fetch`]; blocking, so the check that runs it stays off
/// the UI thread). Its one bounded budget covers the whole round trip — a wedged server
/// degrades to the engine's check-failed outcome instead of pinning a background worker
/// forever, so Retry stays meaningful.
fn get(url: &str) -> Result<String, UpdateFetchError> {
    let agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .timeout_global(Some(GET_TIMEOUT))
            .build(),
    );
    let detail = |e: ureq::Error| UpdateFetchError::new(e.to_string());
    let mut response = agent
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", "Grove")
        .call()
        .map_err(detail)?;
    response.body_mut().read_to_string().map_err(detail)
}

/// Marks a check between its trigger and its report: while it is set, a second trigger
/// is a no-op (spec: one app-level check at a time; the dialogs show shared state).
struct Running;

impl Global for Running {}

/// The auto-check already ran this launch — checked or not, it never runs again (spec:
/// never more than one per launch).
struct AutoRan;

impl Global for AutoRan {}

/// Which entry point started the check, deciding what its report shows.
#[derive(Clone, Copy)]
enum Kind {
    /// The Grove-menu entry (and Retry): shows progress, then every outcome honestly.
    Manual,
    /// The once-per-launch check after startup settles: no progress, and silent unless
    /// a newer, not-yet-dismissed release exists.
    Auto,
}

/// The open dialog's window, its view, and the window-closed observer freeing the slot;
/// [`window()`] also filters a closed window's stale handle against the windows that
/// still exist (the About window's slot pattern).
struct Open {
    window: AnyWindowHandle,
    dialog: Entity<Dialog>,
    /// Alive only while the dialog is; dropped with the global, so open/close cycles
    /// don't accumulate observers.
    _closed: Subscription,
}

impl Global for Open {}

/// The Grove-menu entry: starts the one check (or reruns it over a shown result) and
/// shows the progress dialog meanwhile. While a check runs, another trigger is a no-op.
/// Dispatch before setup finishes is a quiet no-op: nothing spawned, opened, or recorded
/// (see [`see_update`] for the Library the check's report reads).
pub fn check(cx: &mut App) {
    // The whole check's report reads the Library `shell::start` installs (see
    // [`see_update`]; spec: a half-configured Grove shows setup alone), so the Grove
    // menu's entry dispatching before setup is a no-op — before the [`Running`] guard so
    // a stray pre-start dispatch leaves no check state for the real launch to trip on.
    // Retry can't reach here pre-start: its button lives on a result dialog only a check
    // that already ran past `start` can have shown.
    if !shell::is_running(cx) {
        return;
    }
    if cx.has_global::<Running>() {
        return;
    }
    cx.activate(true);
    if window(cx).is_some() {
        // Re-checking over a shown result: the shared dialog returns to progress.
        let dialog = cx.global::<Open>().dialog.clone();
        dialog.update(cx, |dialog, cx| {
            dialog.stage = None;
            cx.notify();
        });
    } else {
        open_dialog(None, cx);
    }
    if window(cx).is_none() {
        eprintln!("[grove] update: dialog unavailable");
        return;
    }
    cx.set_global(Running);
    spawn_check(Kind::Manual, cx);
}

/// The one auto-check at the end of the launch (`shell::start`): once per launch, only
/// when the settings toggle is on, sharing the engine, state and dialogs with the menu
/// entry. Its report is fully silent unless a newer, not-yet-dismissed release shows.
/// A dev run ([`dev_run`]) skips the check entirely — no fetch, no dialog, nothing
/// recorded — while consuming the same once-per-launch slot, so a later call still runs
/// nothing; the manual entry is unaffected.
pub fn auto_check(cx: &mut App) {
    if cx.has_global::<AutoRan>() {
        return;
    }
    cx.set_global(AutoRan);
    if dev_run(cx) || cx.has_global::<Running>() || !shell::library(cx).settings.check_for_updates {
        return;
    }
    cx.set_global(Running);
    spawn_check(Kind::Auto, cx);
}

/// The open dialog's window, if it still exists.
pub fn window(cx: &App) -> Option<AnyWindowHandle> {
    cx.try_global::<Open>()
        .filter(|open| cx.windows().contains(&open.window))
        .map(|open| open.window)
}

/// Opens the shared dialog, at its progress stage unless it opens showing a finished
/// outcome (the auto-check never shows a progress stage). Returns whether it opened.
fn open_dialog(stage: Option<UpdateDecision>, cx: &mut App) -> bool {
    #[cfg(test)]
    if cx.has_global::<UnavailableDialog>() {
        // The injected failure stands in for the `Err` the platform's window factory
        // can raise below; report through the same channel to keep the seam's shape.
        eprintln!("[grove] update: dialog unavailable");
        return false;
    }
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(SIZE.0), px(SIZE.1)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some("Check for Updates".into()),
            ..Default::default()
        }),
        focus: true,
        show: true,
        kind: WindowKind::Normal,
        is_resizable: false,
        is_minimizable: false,
        ..Default::default()
    };
    match windows::open_window(options, cx, |_, cx| cx.new(|_| Dialog { stage })) {
        // When the dialog closes by any path the slot frees, so the next check builds
        // a new dialog instead of reaching for the gone window.
        Ok((window, dialog)) => {
            let closed = cx.on_window_closed(|cx, window_id| {
                let gone = cx
                    .try_global::<Open>()
                    .is_some_and(|open| open.window.window_id() == window_id);
                if gone {
                    cx.remove_global::<Open>();
                }
            });
            cx.set_global(Open {
                window,
                dialog: dialog.clone(),
                _closed: closed,
            });
            true
        }
        // The caller reads the return as whether anything was shown (see `report`).
        Err(e) => {
            eprintln!("[grove] update: {e:#}");
            false
        }
    }
}

const SIZE: (f32, f32) = (320., 190.);

/// The whole check's one bound, agent-level, covering transport and the body read;
/// anything slower degrades to the engine's check-failed outcome.
const GET_TIMEOUT: Duration = Duration::from_secs(30);

/// Runs one check on a background thread (the blocking getter never runs on the UI
/// thread) and reports the outcome back over the async channel Grove's background work
/// already reports through. A slow, dropped, or failing fetch can never block the UI:
/// only the finished decision crosses back.
fn spawn_check(kind: Kind, cx: &mut App) {
    let fetch = fetcher(cx);
    let (tx, rx) = async_channel::unbounded::<UpdateDecision>();
    cx.background_spawn(async move {
        let decision = check_for_update(RUNNING_VERSION, |url| fetch(url));
        let _ = tx.send_blocking(decision);
    })
    .detach();
    cx.spawn(async move |cx| {
        while let Ok(decision) = rx.recv().await {
            cx.update(|cx| report(decision, kind, cx));
        }
    })
    .detach();
}

/// Reports one finished check: the shared state frees, the dialog shows the outcome.
/// A manual check keeps its behavior on every outcome and on a dialog closed while its
/// check ran (the user closed it; the next trigger runs a check of its own). The
/// auto-check opens the shared dialog only for a release newer than both the running
/// version and the persisted seen one, and records the release only if that dialog
/// opened; otherwise it shows nothing and records nothing. A shown update-available
/// counts as seen, from either entry point.
fn report(decision: UpdateDecision, kind: Kind, cx: &mut App) {
    cx.remove_global::<Running>();
    if let Kind::Auto = kind {
        let seen = shell::library(cx).settings.dismissed_version.clone();
        if !auto_check_alerts(&decision, seen.as_deref()) {
            return;
        }
        // No progress stage: the auto-check opens the shared dialog straight at its
        // result, the same dialog the menu entry shows.
        if !open_dialog(Some(decision.clone()), cx) {
            // A dialog that couldn't open showed nothing, so the release stays unseen:
            // recording it would silence every later launch's alert for it.
            return;
        }
    } else {
        let Some(open) = cx.try_global::<Open>().map(|open| open.dialog.clone()) else {
            return;
        };
        open.update(cx, |dialog, cx| {
            dialog.stage = Some(decision.clone());
            cx.notify();
        });
    }
    if let UpdateDecision::Available { version, .. } = &decision {
        see_update(version, cx);
    }
}

/// Records a shown update as seen: the newest version seen persists and the settings
/// file follows through the normal save path. The dismissed version lives only in the
/// auto-check's gating; manual checks ignore it and always report the truth.
fn see_update(version: &Version, cx: &mut App) {
    let seen = cx
        .global_mut::<shell::Library>()
        .settings
        .set_dismissed_version(version);
    if seen {
        shell::save_settings(cx);
    }
}

/// The dialog's stage: progress until the check reports, then its one outcome.
struct Dialog {
    stage: Option<UpdateDecision>,
}

impl Render for Dialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = theme::palette(cx);
        let (heading, detail): (String, Option<&'static str>) = match &self.stage {
            None => ("Checking for updates…".into(), None),
            // Up-to-date and failed appear from manual checks only, always carrying the
            // version readout the About window and Settings show.
            Some(UpdateDecision::UpToDate) => ("You're up to date.".into(), Some(about::VERSION)),
            Some(UpdateDecision::Available { version, .. }) => {
                (format!("Grove {version} is available."), None)
            }
            Some(UpdateDecision::CheckFailed) => (
                "Couldn't check for updates.".into(),
                Some("Grove didn't reach the release page."),
            ),
        };
        let button: Option<Button> = match &self.stage {
            // Download… opens the release page exactly as the response carried it (Grove
            // constructs no URL), through the NSWorkspace opening Grove's links use.
            Some(UpdateDecision::Available { page_url, .. }) => {
                let page_url = page_url.clone();
                Some(
                    theme::button(
                        cx,
                        "update-download",
                        "Download…",
                        theme::ButtonKind::Primary,
                        Some(move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                            cx.open_url(&page_url);
                        }),
                    )
                    .accessibility_label("Download, opens the release page in your browser"),
                )
            }
            Some(UpdateDecision::CheckFailed) => Some(
                theme::button(
                    cx,
                    "update-retry",
                    "Retry",
                    theme::ButtonKind::Secondary,
                    Some(|_: &ClickEvent, _: &mut Window, cx: &mut App| {
                        check(cx);
                    }),
                )
                .accessibility_label("Retry, checks for updates again"),
            ),
            _ => None,
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(12.))
            .px(px(24.))
            .bg(palette.panel())
            .text_color(palette.text())
            .child(
                div()
                    .id("update-heading")
                    .test_support()
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .aria_label(heading.clone())
                    .child(heading),
            )
            .when_some(detail, |d, detail| {
                d.child(
                    div()
                        .id("update-detail")
                        .test_support()
                        .text_size(px(12.))
                        .text_color(palette.muted())
                        .aria_label(detail)
                        .child(detail),
                )
            })
            .when_some(button, |d, button| d.child(button))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use gpui_kit::TestAppContext;
    use gpui_kit::test::TestWindowExt;
    use grove_core::{Outcome, RELEASES_ENDPOINT, Settings, SettingsFile};
    use tempfile::TempDir;

    use crate::about::VERSION;
    use crate::shell;
    use crate::test_support::{canned, fake_launcher, menu_action, set_library, settings_file};
    use crate::{theme, update};

    use super::*;

    /// What one injected check hands the engine: either a canned releases body or a
    /// fetch failure (offline, non-2xx), which maps into the check-failed outcome.
    enum Reply {
        /// A releases response body the engine reads.
        Body(&'static str),
        /// A fetch failure with its detail.
        Failed(&'static str),
    }

    /// A releases response whose latest entry is `tag`, its page link alongside.
    fn body_for(tag: &'static str) -> &'static str {
        match tag {
            "v0.2.0" => {
                r#"[{"tag_name":"v0.2.0","html_url":"https://github.com/olsonjj/grove/releases/tag/v0.2.0"}]"#
            }
            "0.1.0" => {
                r#"[{"tag_name":"0.1.0","html_url":"https://github.com/olsonjj/grove/releases/tag/0.1.0"}]"#
            }
            other => panic!("no body fixture for {other}"),
        }
    }

    /// Grove past setup, with an active window every dispatch runs inside (the
    /// app-menu tests' shape).
    fn running(cx: &mut TestAppContext, file: &SettingsFile) -> AnyWindowHandle {
        fake_launcher(cx, Ok(Outcome::Opened));
        let main = cx.update(|cx| {
            gpui_kit::init(cx);
            theme::init(cx);
            shell::init(cx);
            set_library(file, vec![canned()], cx);
            shell::start_bare(cx);
            theme::apply_choice_from(file, cx);
            shell::show_main(cx);
            cx.windows()[0]
        });
        cx.update_window(main, |_, window, _| window.activate_window())
            .unwrap();
        main
    }

    /// Injects `replies` as the check's canned fetch, recording every endpoint it gets.
    fn inject(cx: &mut TestAppContext, replies: Vec<Reply>) -> Arc<Mutex<Vec<String>>> {
        let queue: Arc<Mutex<VecDeque<Reply>>> = Arc::new(Mutex::new(replies.into()));
        let hits: Arc<Mutex<Vec<String>>> = Default::default();
        let record = hits.clone();
        cx.update(|cx| {
            cx.set_global(Fetcher {
                fetch: Arc::new(move |url: &str| {
                    record.lock().unwrap().push(url.to_string());
                    match queue.lock().unwrap().pop_front() {
                        Some(Reply::Body(body)) => Ok(body.to_string()),
                        Some(Reply::Failed(detail)) => Err(UpdateFetchError::new(detail)),
                        // Only a misfired trigger reaches an empty queue.
                        None => Err(UpdateFetchError::new("no canned response left")),
                    }
                }),
            });
        });
        hits
    }

    /// Dispatches `item` the way its menu selection does, without parking the executor,
    /// so a second trigger can land while the first check is still only queued.
    fn menu_dispatch(cx: &mut TestAppContext, item: &str) {
        let action = menu_action(cx, "Grove", item);
        cx.update(|cx| cx.dispatch_action(action.as_ref()));
    }

    /// The dialog's heading and detail words, rendering before reading.
    fn dialog_labels(cx: &mut TestAppContext) -> (Option<String>, Option<String>) {
        let Some(w) = cx.update(|cx| update::window(cx)) else {
            return (None, None);
        };
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            (
                window
                    .try_find("update-heading")
                    .and_then(|v| v.label().map(str::to_string)),
                window
                    .try_find("update-detail")
                    .and_then(|v| v.label().map(str::to_string)),
            )
        })
        .unwrap()
    }

    #[gpui_kit::test]
    fn the_menu_entry_checks_and_names_the_new_version(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        let main = running(cx, &file);
        let hits = inject(cx, vec![Reply::Body(body_for("v0.2.0"))]);
        menu_dispatch(cx, "Check for Updates…");
        cx.run_until_parked();

        assert_eq!(
            hits.lock().unwrap().clone(),
            [RELEASES_ENDPOINT.to_string()],
            "the engine fetched the one releases endpoint"
        );
        assert_ne!(cx.update(|cx| update::window(cx)), Some(main));
        assert_eq!(
            dialog_labels(cx).0.as_deref(),
            Some("Grove 0.2.0 is available."),
        );
        assert_eq!(cx.opened_url(), None);
    }

    #[gpui_kit::test]
    fn up_to_date_names_the_running_version(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        running(cx, &file);
        let hits = inject(cx, vec![Reply::Body(body_for("0.1.0"))]);
        menu_dispatch(cx, "Check for Updates…");
        cx.run_until_parked();

        assert_eq!(hits.lock().unwrap().len(), 1);
        assert_eq!(
            dialog_labels(cx),
            (Some("You're up to date.".into()), Some(VERSION.into())),
            "the words the About window and Settings show"
        );
    }

    #[gpui_kit::test]
    fn a_failed_check_offers_retry_which_reruns_it(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        running(cx, &file);
        let hits = inject(
            cx,
            vec![
                Reply::Failed("connection refused"),
                Reply::Body(body_for("v0.2.0")),
            ],
        );
        menu_dispatch(cx, "Check for Updates…");
        cx.run_until_parked();

        let w = cx.update(|cx| update::window(cx)).unwrap();
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            window.click("update-retry", cx);
        })
        .unwrap();
        cx.run_until_parked();

        assert_eq!(hits.lock().unwrap().len(), 2, "the retry ran its own check");
        assert_eq!(
            dialog_labels(cx).0.as_deref(),
            Some("Grove 0.2.0 is available."),
        );
    }

    #[gpui_kit::test]
    fn a_download_click_opens_the_release_page(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        running(cx, &file);
        let _hits = inject(cx, vec![Reply::Body(body_for("v0.2.0"))]);
        menu_dispatch(cx, "Check for Updates…");
        cx.run_until_parked();

        let w = cx.update(|cx| update::window(cx)).unwrap();
        cx.update_window(w, |_, window, cx| {
            window.render_frame(cx);
            window.click("update-download", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(
            cx.opened_url().as_deref(),
            Some("https://github.com/olsonjj/grove/releases/tag/v0.2.0"),
        );
    }

    #[gpui_kit::test]
    fn a_second_trigger_while_one_check_runs_is_ignored(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        running(cx, &file);
        let hits = inject(cx, vec![Reply::Body(body_for("v0.2.0"))]);
        menu_dispatch(cx, "Check for Updates…");
        let dialog = cx.update(|cx| update::window(cx)).unwrap();
        // Nothing was parked, so the check is still only queued.
        assert_eq!(
            dialog_labels(cx).0.as_deref(),
            Some("Checking for updates…"),
            "progress while the check runs"
        );
        // Still only queued (nothing was parked), so this trigger lands mid-check.
        menu_dispatch(cx, "Check for Updates…");
        assert_eq!(
            dialog_labels(cx).0.as_deref(),
            Some("Checking for updates…"),
            "the no-op trigger left the progress as it found it"
        );
        cx.run_until_parked();

        assert_eq!(
            hits.lock().unwrap().len(),
            1,
            "the second trigger started no check of its own"
        );
        assert_eq!(
            cx.update(|cx| update::window(cx)),
            Some(dialog),
            "still the one shared dialog"
        );
        assert_eq!(
            dialog_labels(cx).0.as_deref(),
            Some("Grove 0.2.0 is available."),
        );
    }

    /// Grove past setup on `settings`, with the file and its backing temp dir — the
    /// caller keeps them alive, so a seen-version record can reach the disk.
    fn launched(cx: &mut TestAppContext, settings: Settings) -> (TempDir, SettingsFile) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, settings);
        running(cx, &file);
        (tmp, file)
    }

    #[gpui_kit::test]
    fn the_auto_check_runs_once_per_launch_and_shows_a_newer_release(cx: &mut TestAppContext) {
        let (_tmp, _file) = launched(cx, Settings::default());
        let hits = inject(cx, vec![Reply::Body(body_for("v0.2.0"))]);
        cx.update(update::auto_check);
        cx.run_until_parked();

        assert_eq!(
            hits.lock().unwrap().clone(),
            [RELEASES_ENDPOINT.to_string()],
            "the one anonymous check ran"
        );
        assert_eq!(
            dialog_labels(cx).0.as_deref(),
            Some("Grove 0.2.0 is available."),
            "a genuinely new release reaches the user"
        );
        // Never more than one per launch: the second call starts no check of its own.
        cx.update(update::auto_check);
        cx.run_until_parked();
        assert_eq!(hits.lock().unwrap().len(), 1);
    }

    #[gpui_kit::test]
    fn the_toggle_off_means_the_auto_check_never_runs(cx: &mut TestAppContext) {
        let (_tmp, _file) = launched(
            cx,
            Settings {
                check_for_updates: false,
                ..Default::default()
            },
        );
        let hits = inject(cx, vec![]);
        cx.update(update::auto_check);
        cx.run_until_parked();

        assert!(hits.lock().unwrap().is_empty(), "no fetch at all");
        assert!(cx.update(|cx| update::window(cx)).is_none());
    }

    #[gpui_kit::test]
    fn the_auto_check_stays_silent_when_the_running_version_is_current(cx: &mut TestAppContext) {
        let (_tmp, _file) = launched(cx, Settings::default());
        let hits = inject(cx, vec![Reply::Body(body_for("0.1.0"))]);
        cx.update(update::auto_check);
        cx.run_until_parked();

        assert_eq!(hits.lock().unwrap().len(), 1, "the check ran, quietly");
        assert!(cx.update(|cx| update::window(cx)).is_none());
    }

    #[gpui_kit::test]
    fn the_auto_check_stays_silent_when_the_check_fails(cx: &mut TestAppContext) {
        let (_tmp, _file) = launched(cx, Settings::default());
        let hits = inject(cx, vec![Reply::Failed("403")]);
        cx.update(update::auto_check);
        cx.run_until_parked();

        assert_eq!(hits.lock().unwrap().len(), 1, "the check ran, quietly");
        assert!(cx.update(|cx| update::window(cx)).is_none());
    }

    #[gpui_kit::test]
    fn the_auto_check_stays_silent_for_version_already_seen(cx: &mut TestAppContext) {
        let (_tmp, _file) = launched(
            cx,
            Settings {
                dismissed_version: Some("0.2.0".into()),
                ..Default::default()
            },
        );
        let hits = inject(cx, vec![Reply::Body(body_for("v0.2.0"))]);
        cx.update(update::auto_check);
        cx.run_until_parked();

        assert_eq!(hits.lock().unwrap().len(), 1, "the check ran, quietly");
        assert!(cx.update(|cx| update::window(cx)).is_none());
        assert_eq!(
            cx.update(|cx| shell::library(cx).settings.dismissed_version.clone()),
            Some("0.2.0".into()),
            "silence records nothing new"
        );
    }

    #[gpui_kit::test]
    fn the_auto_check_alerts_when_a_release_is_newer_than_the_seen_one(cx: &mut TestAppContext) {
        let (_tmp, _file) = launched(
            cx,
            Settings {
                dismissed_version: Some("0.1.0".into()),
                ..Default::default()
            },
        );
        let _hits = inject(cx, vec![Reply::Body(body_for("v0.2.0"))]);
        cx.update(update::auto_check);
        cx.run_until_parked();

        // Straight at the result: the auto-check never shows a progress stage.
        assert_eq!(
            dialog_labels(cx).0.as_deref(),
            Some("Grove 0.2.0 is available."),
        );
    }

    /// A shown release persists through the settings file and, loaded back, gates the
    /// auto-check that would otherwise alert for it.
    #[gpui_kit::test]
    fn an_auto_shown_release_is_recorded_as_seen_for_the_next_launch(cx: &mut TestAppContext) {
        let (_tmp, file) = launched(cx, Settings::default());
        let _hits = inject(cx, vec![Reply::Body(body_for("v0.2.0"))]);
        cx.update(update::auto_check);
        cx.run_until_parked();

        let saved = file.load().unwrap();
        assert_eq!(saved.dismissed_version.as_deref(), Some("0.2.0"));
        let release = UpdateDecision::Available {
            version: semver::Version::parse("0.2.0").unwrap(),
            page_url: "https://github.com/olsonjj/grove/releases/tag/v0.2.0".into(),
        };
        assert!(
            !auto_check_alerts(&release, saved.dismissed_version.as_deref()),
            "the persisted version gates the auto-check on the next launch"
        );
    }

    #[gpui_kit::test]
    fn an_auto_check_dialog_that_cannot_open_records_nothing(cx: &mut TestAppContext) {
        let (_tmp, file) = launched(cx, Settings::default());
        let hits = inject(cx, vec![Reply::Body(body_for("v0.2.0"))]);
        cx.update(|cx| {
            cx.set_global(UnavailableDialog);
            update::auto_check(cx);
        });
        cx.run_until_parked();

        assert_eq!(hits.lock().unwrap().len(), 1, "the check ran");
        assert!(
            cx.update(|cx| update::window(cx)).is_none(),
            "no dialog opened"
        );
        assert_eq!(
            file.load().unwrap().dismissed_version,
            None,
            "nothing was shown, so nothing is recorded"
        );
    }

    #[gpui_kit::test]
    fn a_manual_check_reports_a_release_already_seen(cx: &mut TestAppContext) {
        let (_tmp, _file) = launched(
            cx,
            Settings {
                dismissed_version: Some("0.2.0".into()),
                ..Default::default()
            },
        );
        let hits = inject(cx, vec![Reply::Body(body_for("v0.2.0"))]);
        menu_dispatch(cx, "Check for Updates…");
        cx.run_until_parked();

        assert_eq!(hits.lock().unwrap().len(), 1);
        assert_eq!(
            dialog_labels(cx).0.as_deref(),
            Some("Grove 0.2.0 is available."),
            "manual checks ignore the seen version and report the truth"
        );
    }

    #[gpui_kit::test]
    fn a_dev_run_never_auto_checks(cx: &mut TestAppContext) {
        let (_tmp, _file) = launched(cx, Settings::default());
        cx.update(|cx| cx.set_global(Dev(true)));
        let hits = inject(cx, vec![]);
        cx.update(update::auto_check);
        cx.run_until_parked();

        assert!(hits.lock().unwrap().is_empty(), "no fetch at all");
        assert!(cx.update(|cx| update::window(cx)).is_none(), "no dialog");
        assert_eq!(
            cx.update(|cx| shell::library(cx).settings.dismissed_version.clone()),
            None,
            "silence records nothing"
        );
        // The dev run consumed the once-per-launch slot, so a later call also runs
        // nothing.
        cx.update(update::auto_check);
        cx.run_until_parked();
        assert!(hits.lock().unwrap().is_empty());
    }

    #[gpui_kit::test]
    fn the_dev_override_off_leaves_the_auto_check_on_the_normal_path(cx: &mut TestAppContext) {
        let (_tmp, _file) = launched(cx, Settings::default());
        cx.update(|cx| cx.set_global(Dev(false)));
        let hits = inject(cx, vec![Reply::Body(body_for("v0.2.0"))]);
        cx.update(update::auto_check);
        cx.run_until_parked();

        assert_eq!(
            hits.lock().unwrap().clone(),
            [RELEASES_ENDPOINT.to_string()],
            "the override off checks as ever"
        );
    }

    /// Check for Updates… before `shell::start`: the Grove menu exists from launch
    /// ([`shell::init`]), but the check's report reads the Library `start` installs, so
    /// the dispatch stays a quiet no-op in the half-configured state only setup is open
    /// in — no fetch opens no dialog, and no check state leaks out of the dispatch.
    #[gpui_kit::test]
    fn a_check_for_updates_dispatch_before_setup_finishes_stays_quiet(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            shell::init(cx);
        });
        let hits = inject(cx, vec![Reply::Body(body_for("v0.2.0"))]);
        menu_dispatch(cx, "Check for Updates…");
        cx.run_until_parked();

        assert!(
            hits.lock().unwrap().is_empty(),
            "the pre-start dispatch fetched nothing"
        );
        assert!(
            cx.update(|cx| update::window(cx)).is_none(),
            "the pre-start dispatch opened no dialog"
        );
        cx.update(|cx| {
            assert!(
                !cx.has_global::<Running>(),
                "no in-flight check was left behind"
            );
            assert!(
                cx.try_global::<Open>().is_none(),
                "no result dialog was left behind"
            );
            assert!(
                !cx.has_global::<AutoRan>(),
                "the manual dispatch consumed no once-per-launch auto-check slot"
            );
        });
    }

    #[gpui_kit::test]
    fn the_manual_entry_still_checks_in_a_dev_run(cx: &mut TestAppContext) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, Settings::default());
        running(cx, &file);
        cx.update(|cx| cx.set_global(Dev(true)));
        let hits = inject(cx, vec![Reply::Body(body_for("v0.2.0"))]);
        menu_dispatch(cx, "Check for Updates…");
        cx.run_until_parked();

        assert_eq!(
            hits.lock().unwrap().clone(),
            [RELEASES_ENDPOINT.to_string()],
            "the dev marker never blocks the menu entry"
        );
        assert_eq!(
            dialog_labels(cx).0.as_deref(),
            Some("Grove 0.2.0 is available."),
        );
    }
}
