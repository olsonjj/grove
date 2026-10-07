use objc2::rc::Retained;
use objc2_app_kit::{
    NSAppearance, NSAppearanceNameDarkAqua, NSApplication, NSApplicationActivationOptions, NSEvent,
    NSRunningApplication, NSScreen, NSWorkspace,
};
use objc2_foundation::{MainThreadMarker, NSNumber, NSPoint, NSRect, ns_string};

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

/// The display the mouse is on, as its CGDirectDisplayID (GPUI's `DisplayId`). `None` off
/// the main thread.
pub fn display_under_mouse() -> Option<u64> {
    let screens = NSScreen::screens(MainThreadMarker::new()?);
    let frames: Vec<NSRect> = screens.iter().map(|s| s.frame()).collect();
    let screen = screens.objectAtIndex(screen_at(&frames, NSEvent::mouseLocation())?);
    let number = screen
        .deviceDescription()
        .objectForKey(ns_string!("NSScreenNumber"))?;
    Some(number.downcast::<NSNumber>().ok()?.unsignedIntegerValue() as u64)
}

/// The first of `frames` containing `point`, both in AppKit's global coordinates.
fn screen_at(frames: &[NSRect], point: NSPoint) -> Option<usize> {
    frames.iter().position(|f| {
        (f.origin.x..=f.origin.x + f.size.width).contains(&point.x)
            && (f.origin.y..=f.origin.y + f.size.height).contains(&point.y)
    })
}

#[cfg(test)]
mod tests {
    use objc2_foundation::NSSize;

    use super::*;

    #[test]
    fn screen_at_picks_the_frame_under_the_point() {
        let rect = |x, y, w, h| NSRect::new(NSPoint::new(x, y), NSSize::new(w, h));
        // A 1512×982 primary, a 2560×1440 external to its right, one below it.
        let frames = [
            rect(0., 0., 1512., 982.),
            rect(1512., -200., 2560., 1440.),
            rect(0., -1080., 1920., 1080.),
        ];
        let at = |x, y| screen_at(&frames, NSPoint::new(x, y));
        assert_eq!(at(700., 500.), Some(0));
        assert_eq!(at(2000., 1100.), Some(1));
        assert_eq!(at(300., -500.), Some(2));
        assert_eq!(at(1000., 1200.), None);
        // On a shared edge, the first frame wins.
        assert_eq!(at(1512., 500.), Some(0));
        assert_eq!(at(700., 0.), Some(0));
    }
}
