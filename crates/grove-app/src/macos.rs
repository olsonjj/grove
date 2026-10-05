use objc2::rc::Retained;
use objc2_app_kit::{
    NSAppearance, NSAppearanceNameDarkAqua, NSApplication, NSApplicationActivationOptions,
    NSRunningApplication, NSScreen, NSWorkspace,
};
use objc2_foundation::MainThreadMarker;

fn mtm() -> MainThreadMarker {
    MainThreadMarker::new().expect("must run on the main thread")
}

/// Dark theme only: also darkens titlebars and system panels (folder picker).
pub fn use_dark_appearance() {
    // SAFETY: AppKit-provided constant.
    let name = unsafe { NSAppearanceNameDarkAqua };
    let appearance = NSAppearance::appearanceNamed(name);
    NSApplication::sharedApplication(mtm()).setAppearance(appearance.as_deref());
}

pub fn is_app_active() -> bool {
    NSApplication::sharedApplication(mtm()).isActive()
}

/// Frontmost app, unless it's Grove itself.
pub fn frontmost_other_app() -> Option<Retained<NSRunningApplication>> {
    let front = NSWorkspace::sharedWorkspace().frontmostApplication()?;
    let me = NSRunningApplication::currentApplication();
    (front.processIdentifier() != me.processIdentifier()).then_some(front)
}

pub fn app_name(app: &NSRunningApplication) -> String {
    app.localizedName()
        .map(|n| n.to_string())
        .unwrap_or_default()
}

pub fn activate(app: &NSRunningApplication) {
    app.activateWithOptions(NSApplicationActivationOptions::empty());
}

/// Backing scale of the menu-bar screen (status item coordinates are physical pixels on it).
pub fn primary_screen_scale() -> f64 {
    NSScreen::screens(mtm())
        .firstObject()
        .map(|s| s.backingScaleFactor())
        .unwrap_or(2.0)
}
