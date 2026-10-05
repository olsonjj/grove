use std::sync::OnceLock;

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, Sel};
use objc2::sel;
use objc2_app_kit::{
    NSAppearance, NSAppearanceNameDarkAqua, NSApplication, NSApplicationActivationOptions,
    NSApplicationActivationPolicy, NSRunningApplication, NSScreen, NSWorkspace,
};
use objc2_foundation::MainThreadMarker;

fn mtm() -> MainThreadMarker {
    MainThreadMarker::new().expect("must run on the main thread")
}

type SetPolicyFn = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, isize) -> Bool;
static ORIGINAL_SET_POLICY: OnceLock<SetPolicyFn> = OnceLock::new();

unsafe extern "C-unwind" fn set_policy_accessory(this: *mut AnyObject, sel: Sel, _: isize) -> Bool {
    let original = ORIGINAL_SET_POLICY.get().expect("swizzle installed");
    unsafe { original(this, sel, NSApplicationActivationPolicy::Accessory.0) }
}

/// GPUI hard-codes `setActivationPolicy(Regular)` in applicationDidFinishLaunching, which
/// shows a Dock icon for ~125 ms before we can flip it back. Swizzle the setter so every
/// call becomes Accessory. Must run before `gpui_kit::application()`.
pub fn force_accessory_activation_policy() {
    let class = AnyClass::get(c"NSApplication").expect("NSApplication class");
    let method = class
        .instance_method(sel!(setActivationPolicy:))
        .expect("setActivationPolicy:");
    let replacement: SetPolicyFn = set_policy_accessory;
    // SAFETY: same signature as -[NSApplication setActivationPolicy:] (BOOL, NSInteger arg).
    let previous =
        unsafe { method.set_implementation(std::mem::transmute::<SetPolicyFn, Imp>(replacement)) };
    let previous = unsafe { std::mem::transmute::<Imp, SetPolicyFn>(previous) };
    let _ = ORIGINAL_SET_POLICY.set(previous);
}

pub fn set_accessory_activation_policy() {
    let app = NSApplication::sharedApplication(mtm());
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    eprintln!(
        "[grove] activation policy = {:?} (Accessory = {:?})",
        app.activationPolicy(),
        NSApplicationActivationPolicy::Accessory
    );
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
