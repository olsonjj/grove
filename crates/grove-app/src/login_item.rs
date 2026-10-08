//! Launch at login via `SMAppService.mainAppService`. Only a bundled Grove.app has one; from
//! `cargo run` or tests nothing is touched. Tests swap in a fake service.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::{App, AppContext, Global, Task};
use grove_core::{LoginItemHost, LoginItemStatus, Settings, home_dir, is_installed_location};
use objc2_foundation::NSBundle;
use objc2_service_management::{SMAppService, SMAppServiceStatus};

use crate::main_window::tilde;
use crate::shell::{self, Library};

pub const LOGIN_ITEMS: &str = "System Settings → General → Login Items";

pub type Status = Arc<dyn Fn() -> LoginItemStatus + Send + Sync>;
/// Registers (`true`) or unregisters, returning the status after.
pub type Set = Arc<dyn Fn(bool) -> Result<LoginItemStatus, String> + Send + Sync>;

pub struct LoginItems {
    /// This copy's Grove.app, symlinks resolved; `None` when not running from one.
    pub bundle: Option<PathBuf>,
    /// `bundle` may register itself without a Save (see `is_installed_location`).
    pub installed: bool,
    pub status: Status,
    pub set: Set,
}

impl Global for LoginItems {}

impl Default for LoginItems {
    fn default() -> Self {
        let main = NSBundle::mainBundle();
        let bundle = main.bundleIdentifier().map(|_| {
            let path = PathBuf::from(main.bundlePath().to_string());
            fs::canonicalize(&path).unwrap_or(path)
        });
        let home = home_dir().map(|h| fs::canonicalize(&h).unwrap_or(h));
        let installed = bundle
            .as_deref()
            .is_some_and(|b| is_installed_location(b, home.as_deref()));
        Self {
            bundle,
            installed,
            status: Arc::new(status),
            set: Arc::new(set),
        }
    }
}

pub fn status() -> LoginItemStatus {
    // SAFETY: plain ServiceManagement calls; the service needs no main thread.
    let status = unsafe { SMAppService::mainAppService().status() };
    match status {
        SMAppServiceStatus::Enabled => LoginItemStatus::Enabled,
        SMAppServiceStatus::RequiresApproval => LoginItemStatus::RequiresApproval,
        // A main app that never registered reads as not found.
        SMAppServiceStatus::NotRegistered | SMAppServiceStatus::NotFound => {
            LoginItemStatus::NotRegistered
        }
        _ => LoginItemStatus::Unavailable,
    }
}

fn set(on: bool) -> Result<LoginItemStatus, String> {
    // SAFETY: as in `status`.
    let result = unsafe {
        let service = SMAppService::mainAppService();
        if on {
            service.registerAndReturnError()
        } else {
            service.unregisterAndReturnError()
        }
    };
    result.map_err(|e| e.localizedDescription().to_string())?;
    Ok(status())
}

/// At launch, once setup is done (see `Settings::sync_login_item`).
pub fn sync(cx: &mut App) {
    reconcile(Settings::sync_login_item, |_, _| {}, cx);
}

/// On settings Save: this copy takes over the login item and makes it match
/// `launch_at_login`. Posts what the user still has to do, if anything.
pub fn apply(cx: &mut App) {
    let Some(bundle) = cx.default_global::<LoginItems>().bundle.clone() else {
        return;
    };
    let previous = shell::library(cx).settings.login_item_path.clone();
    reconcile(
        |s, host, now| s.apply_login_item(host.bundle, now),
        move |result, cx| {
            let settings = &shell::library(cx).settings;
            let on = settings.launch_at_login;
            // Unavailable leaves the owner alone.
            let replaced = previous
                .filter(|p| settings.login_item_path.as_ref() == Some(&bundle) && *p != bundle);
            if let Some(text) = apply_notice(replaced.as_deref(), on, result) {
                crate::launcher::Notice { text, warn: true }.post(false, cx);
            }
        },
        cx,
    );
}

/// What to tell the user after a Save: a failure, approval still needed, or that the copy
/// it `replaced` as owner may stay a login item.
fn apply_notice(
    replaced: Option<&Path>,
    on: bool,
    result: Result<LoginItemStatus, String>,
) -> Option<String> {
    let mut lines = Vec::new();
    match result {
        Err(e) => lines.push(format!("Couldn't change launch at login: {e}.")),
        Ok(LoginItemStatus::RequiresApproval) if on => lines.push(format!(
            "To launch Grove at login, allow it in {LOGIN_ITEMS}."
        )),
        Ok(_) => {}
    }
    if let Some(other) = replaced {
        lines.push(format!(
            "Launch at login now uses this copy of Grove; {} may still open at login too \
             (remove it in {LOGIN_ITEMS}).",
            tilde(other)
        ));
    }
    (!lines.is_empty()).then(|| lines.join(" "))
}

/// macOS's current status for this copy; `None` when not running from Grove.app.
pub fn status_now(cx: &mut App) -> Task<Option<LoginItemStatus>> {
    let items = cx.default_global::<LoginItems>();
    let status = items.bundle.is_some().then(|| items.status.clone());
    cx.background_spawn(async move { status.map(|s| s()) })
}

/// `decide` picks the change; `done` gets the status after it (or the one found, if none).
fn reconcile(
    decide: fn(&mut Settings, LoginItemHost<'_>, LoginItemStatus) -> Option<bool>,
    done: impl FnOnce(Result<LoginItemStatus, String>, &mut App) + 'static,
    cx: &mut App,
) {
    let items = cx.default_global::<LoginItems>();
    let Some(bundle) = items.bundle.clone() else {
        return;
    };
    let (installed, status, set) = (items.installed, items.status.clone(), items.set.clone());
    let owner = shell::library(cx).settings.login_item_path.clone();
    cx.spawn(async move |cx| {
        let (now, owner_exists) = cx
            .background_spawn(async move { (status(), owner.is_some_and(|o| o.exists())) })
            .await;
        let host = LoginItemHost {
            bundle: &bundle,
            installed,
            owner_exists,
        };
        let change = cx.update(|cx| {
            update_settings(
                cx,
                |s| decide(s, host, now),
                |s| {
                    eprintln!(
                        "[grove] login item {now:?}: launch at login {}",
                        s.launch_at_login
                    )
                },
            )
        });
        let Some(on) = change else {
            return cx.update(|cx| done(Ok(now), cx));
        };
        let result = cx.background_spawn(async move { set(on) }).await;
        cx.update(|cx| {
            match &result {
                Ok(after) => {
                    eprintln!("[grove] login item {after:?}");
                    update_settings(cx, |s| s.login_item_registered(&bundle, *after), |_| {});
                }
                Err(e) => eprintln!("[grove] login item: {e}"),
            }
            done(result, cx);
        });
    })
    .detach();
}

/// Runs `f` on the settings, saving them if it changed anything; `on_toggle` when it
/// changed `launch_at_login`.
fn update_settings<R>(
    cx: &mut App,
    f: impl FnOnce(&mut Settings) -> R,
    on_toggle: impl FnOnce(&Settings),
) -> R {
    let settings = &mut cx.global_mut::<Library>().settings;
    let before = settings.clone();
    let result = f(settings);
    if settings.launch_at_login != before.launch_at_login {
        on_toggle(settings);
    }
    if *settings != before {
        shell::save_settings(cx);
    }
    result
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use gpui_kit::TestAppContext;
    use grove_core::LoginItemStatus::{Enabled, NotRegistered, RequiresApproval};
    use grove_core::SettingsFile;
    use tempfile::TempDir;

    use super::*;
    use crate::test_support::{set_library, settings_file};

    const APP: &str = "/Applications/Grove.app";

    /// macOS's status, and each register/unregister call.
    #[derive(Default)]
    struct Fake {
        status: Mutex<Option<LoginItemStatus>>,
        calls: Mutex<Vec<bool>>,
    }

    /// Fake service at `APP`, starting at `status`. `register_to` is the status a
    /// successful register leaves behind.
    fn fake(
        cx: &mut TestAppContext,
        status: LoginItemStatus,
        register_to: LoginItemStatus,
    ) -> Arc<Fake> {
        let fake = Arc::new(Fake::default());
        *fake.status.lock().unwrap() = Some(status);
        let (read, write) = (fake.clone(), fake.clone());
        cx.update(|cx| {
            cx.set_global(LoginItems {
                bundle: Some(APP.into()),
                installed: true,
                status: Arc::new(move || read.status.lock().unwrap().unwrap()),
                set: Arc::new(move |on| {
                    write.calls.lock().unwrap().push(on);
                    let after = if on { register_to } else { NotRegistered };
                    *write.status.lock().unwrap() = Some(after);
                    Ok(after)
                }),
            })
        });
        fake
    }

    fn run(
        cx: &mut TestAppContext,
        settings: Settings,
        step: fn(&mut App),
    ) -> (TempDir, SettingsFile) {
        let tmp = TempDir::new().unwrap();
        let file = settings_file(&tmp, settings);
        cx.update(|cx| {
            set_library(&file, vec![], cx);
            step(cx);
        });
        cx.run_until_parked();
        (tmp, file)
    }

    fn owner(file: &SettingsFile) -> Option<PathBuf> {
        file.load().unwrap().login_item_path
    }

    #[gpui_kit::test]
    fn first_launch_registers_and_saves_the_owner(cx: &mut TestAppContext) {
        let fake = fake(cx, NotRegistered, Enabled);
        let (_tmp, file) = run(cx, Settings::default(), sync);
        assert_eq!(*fake.calls.lock().unwrap(), [true]);
        assert_eq!(owner(&file), Some(APP.into()));
    }

    #[gpui_kit::test]
    fn registration_that_doesnt_take_saves_no_owner(cx: &mut TestAppContext) {
        fake(cx, NotRegistered, NotRegistered);
        let (_tmp, file) = run(cx, Settings::default(), sync);
        assert_eq!(owner(&file), None);
    }

    /// Quit after registering but before saving: the next launch finds it registered.
    #[gpui_kit::test]
    fn launch_adopts_a_registration_it_didnt_record(cx: &mut TestAppContext) {
        let fake = fake(cx, Enabled, Enabled);
        let (_tmp, file) = run(cx, Settings::default(), sync);
        assert!(fake.calls.lock().unwrap().is_empty());
        assert_eq!(owner(&file), Some(APP.into()));
    }

    #[gpui_kit::test]
    fn removal_in_system_settings_is_saved(cx: &mut TestAppContext) {
        let fake = fake(cx, NotRegistered, Enabled);
        let settings = Settings {
            login_item_path: Some(APP.into()),
            ..Default::default()
        };
        let (_tmp, file) = run(cx, settings, sync);
        assert!(fake.calls.lock().unwrap().is_empty());
        assert!(!file.load().unwrap().launch_at_login);
    }

    #[gpui_kit::test]
    fn apply_unregisters_and_takes_ownership(cx: &mut TestAppContext) {
        let fake = fake(cx, Enabled, Enabled);
        let settings = Settings {
            launch_at_login: false,
            login_item_path: Some("/Users/me/Downloads/Grove.app".into()),
            ..Default::default()
        };
        let (_tmp, file) = run(cx, settings, apply);
        assert_eq!(*fake.calls.lock().unwrap(), [false]);
        assert_eq!(owner(&file), Some(APP.into()));
    }

    fn notice(cx: &mut TestAppContext) -> Option<String> {
        cx.update(|cx| {
            cx.try_global::<shell::Notices>()
                .and_then(|n| n.last.clone())
                .map(|s| s.text)
        })
    }

    #[gpui_kit::test]
    fn apply_from_another_copy_says_the_old_one_may_stay(cx: &mut TestAppContext) {
        fake(cx, Enabled, Enabled);
        let settings = Settings {
            login_item_path: Some("/Users/me/Downloads/Grove.app".into()),
            ..Default::default()
        };
        let (_tmp, _file) = run(cx, settings, apply);
        assert_eq!(
            notice(cx).unwrap(),
            "Launch at login now uses this copy of Grove; /Users/me/Downloads/Grove.app may \
             still open at login too (remove it in System Settings → General → Login Items)."
        );
    }

    #[gpui_kit::test]
    fn apply_needing_approval_says_where_to_allow_it(cx: &mut TestAppContext) {
        fake(cx, NotRegistered, RequiresApproval);
        let (_tmp, _file) = run(cx, Settings::default(), apply);
        assert_eq!(
            notice(cx).unwrap(),
            "To launch Grove at login, allow it in System Settings → General → Login Items."
        );
    }

    #[gpui_kit::test]
    fn apply_by_the_owner_says_nothing(cx: &mut TestAppContext) {
        fake(cx, Enabled, Enabled);
        let settings = Settings {
            login_item_path: Some(APP.into()),
            ..Default::default()
        };
        let (_tmp, _file) = run(cx, settings, apply);
        assert_eq!(notice(cx), None);
    }

    #[gpui_kit::test]
    fn launch_leaves_another_existing_copys_login_item(cx: &mut TestAppContext) {
        let fake = fake(cx, NotRegistered, Enabled);
        let other = TempDir::new().unwrap();
        let settings = Settings {
            login_item_path: Some(other.path().into()),
            ..Default::default()
        };
        let (_tmp, file) = run(cx, settings, sync);
        assert!(fake.calls.lock().unwrap().is_empty());
        assert_eq!(owner(&file), Some(other.path().into()));
    }

    #[gpui_kit::test]
    fn launch_replaces_an_owner_moved_away(cx: &mut TestAppContext) {
        let fake = fake(cx, NotRegistered, Enabled);
        let settings = Settings {
            login_item_path: Some("/nonexistent/Downloads/Grove.app".into()),
            ..Default::default()
        };
        let (_tmp, file) = run(cx, settings, sync);
        assert_eq!(*fake.calls.lock().unwrap(), [true]);
        assert_eq!(owner(&file), Some(APP.into()));
    }

    #[gpui_kit::test]
    fn copy_outside_applications_registers_only_on_save(cx: &mut TestAppContext) {
        let fake = fake(cx, NotRegistered, Enabled);
        cx.update(|cx| cx.global_mut::<LoginItems>().installed = false);
        let (_tmp, file) = run(cx, Settings::default(), sync);
        assert!(fake.calls.lock().unwrap().is_empty());
        cx.update(|cx| {
            set_library(&file, vec![], cx);
            apply(cx);
        });
        cx.run_until_parked();
        assert_eq!(*fake.calls.lock().unwrap(), [true]);
        assert_eq!(owner(&file), Some(APP.into()));
    }

    #[gpui_kit::test]
    fn unbundled_touches_nothing(cx: &mut TestAppContext) {
        let fake = fake(cx, NotRegistered, Enabled);
        cx.update(|cx| cx.global_mut::<LoginItems>().bundle = None);
        let (_tmp, file) = run(cx, Settings::default(), sync);
        assert!(fake.calls.lock().unwrap().is_empty());
        assert_eq!(file.load().unwrap(), Settings::default());
    }
}
