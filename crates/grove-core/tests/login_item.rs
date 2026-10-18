//! Login-item detection through a stub SMAppService host.

use std::path::{Path, PathBuf};

use grove_core::LoginItemStatus::{Enabled, NotRegistered, RequiresApproval, Unavailable};
use grove_core::{LoginItemHost, Settings, is_installed_location};

const APP: &str = "/Applications/Grove.app";
const OTHER: &str = "/Users/me/Applications/Grove.app";
const DOWNLOADS: &str = "/Users/me/Downloads/Grove.app";

fn app() -> &'static Path {
    Path::new(APP)
}

/// Installed at `APP`; the recorded owner, if another copy, still exists.
fn host() -> LoginItemHost<'static> {
    LoginItemHost {
        bundle: app(),
        installed: true,
        owner_exists: true,
    }
}

fn settings(launch_at_login: bool, owner: Option<&str>) -> Settings {
    Settings {
        launch_at_login,
        login_item_path: owner.map(PathBuf::from),
        ..Default::default()
    }
}

#[test]
fn installed_means_under_an_applications_folder() {
    let home = Some(Path::new("/Users/me"));
    let installed = |p: &str| is_installed_location(Path::new(p), home);
    assert!(installed(APP));
    assert!(installed(OTHER));
    assert!(!installed(DOWNLOADS));
    assert!(installed("/Applications/Dev/Grove.app"));
    assert!(installed("/Users/me/Applications/Utilities/Grove.app"));
    assert!(!installed("/Applications Old/Grove.app"));
    assert!(!installed("/Users/me/Applications2/Grove.app"));
    assert!(!installed(
        "/Users/me/grove/target/release/bundle/osx/Grove.app"
    ));
    assert!(!installed(
        "/private/var/folders/xy/T/AppTranslocation/1234/d/Grove.app"
    ));
}

#[test]
fn first_launch_registers_by_default() {
    let mut s = Settings::default();
    assert_eq!(s.sync_login_item(host(), NotRegistered), Some(true));
    assert_eq!(s.login_item_path, None, "owned only once registered");
}

#[test]
fn successful_registration_takes_ownership() {
    let mut s = Settings::default();
    s.login_item_registered(app(), Enabled);
    assert_eq!(s.login_item_path.as_deref(), Some(app()));
}

#[test]
fn failed_registration_takes_no_ownership() {
    let mut s = Settings::default();
    s.login_item_registered(app(), NotRegistered);
    assert_eq!(s.login_item_path, None);
}

#[test]
fn first_launch_adopts_an_existing_registration() {
    for status in [Enabled, RequiresApproval] {
        let mut s = settings(false, None);
        assert_eq!(s.sync_login_item(host(), status), None);
        assert_eq!(s.login_item_path.as_deref(), Some(app()));
        assert!(s.launch_at_login);
    }
}

#[test]
fn setting_off_before_ownership_registers_nothing() {
    let mut s = settings(false, None);
    assert_eq!(s.sync_login_item(host(), NotRegistered), None);
}

#[test]
fn removal_in_system_settings_turns_the_setting_off() {
    let mut s = settings(true, Some(APP));
    assert_eq!(s.sync_login_item(host(), NotRegistered), None);
    assert!(!s.launch_at_login);
}

#[test]
fn readding_in_system_settings_turns_the_setting_on() {
    let mut s = settings(false, Some(APP));
    assert_eq!(s.sync_login_item(host(), Enabled), None);
    assert!(s.launch_at_login);
}

#[test]
fn awaiting_approval_counts_as_on() {
    let mut s = settings(false, Some(APP));
    s.sync_login_item(host(), RequiresApproval);
    assert!(s.launch_at_login);
}

#[test]
fn another_existing_copy_keeps_the_login_item() {
    let mut s = settings(true, Some(OTHER));
    assert_eq!(s.sync_login_item(host(), NotRegistered), None);
    assert!(s.launch_at_login);
    assert_eq!(s.login_item_path.as_deref(), Some(Path::new(OTHER)));
}

#[test]
fn owner_moved_away_is_replaced_by_an_installed_copy() {
    let gone = LoginItemHost {
        owner_exists: false,
        ..host()
    };
    let mut s = settings(true, Some(DOWNLOADS));
    assert_eq!(s.sync_login_item(gone, NotRegistered), Some(true));
    assert_eq!(s.login_item_path, None);
    let mut s = settings(true, Some(DOWNLOADS));
    assert_eq!(s.sync_login_item(gone, Enabled), None);
    assert_eq!(s.login_item_path.as_deref(), Some(app()));
}

#[test]
fn copy_outside_applications_neither_registers_nor_adopts() {
    let elsewhere = LoginItemHost {
        bundle: Path::new(DOWNLOADS),
        installed: false,
        owner_exists: true,
    };
    for owner in [None, Some(APP)] {
        let mut s = settings(true, owner);
        let before = s.clone();
        assert_eq!(s.sync_login_item(elsewhere, NotRegistered), None);
        assert_eq!(s.sync_login_item(elsewhere, Enabled), None);
        assert_eq!(s, before);
    }
}

#[test]
fn unavailable_changes_nothing() {
    for owner in [None, Some(APP)] {
        let mut s = settings(true, owner);
        let before = s.clone();
        assert_eq!(s.sync_login_item(host(), Unavailable), None);
        assert_eq!(s.apply_login_item(app(), Unavailable), None);
        assert_eq!(s, before);
    }
}

#[test]
fn apply_matches_the_setting() {
    assert_eq!(
        settings(true, None).apply_login_item(app(), NotRegistered),
        Some(true)
    );
    assert_eq!(
        settings(false, None).apply_login_item(app(), Enabled),
        Some(false)
    );
    assert_eq!(
        settings(false, None).apply_login_item(app(), RequiresApproval),
        Some(false)
    );
    assert_eq!(settings(true, None).apply_login_item(app(), Enabled), None);
}

#[test]
fn apply_takes_ownership_from_anywhere() {
    let mut s = settings(true, Some(APP));
    s.apply_login_item(Path::new(DOWNLOADS), NotRegistered);
    assert_eq!(s.login_item_path.as_deref(), Some(Path::new(DOWNLOADS)));
}
