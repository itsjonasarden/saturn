//! Updates, through Sparkle: Akyba.app carries `Sparkle.framework` (see `scripts/bundle.sh`),
//! loaded here at launch rather than linked, so a build without it (`cargo run`) simply has no
//! updates. Sparkle checks the newest GitHub release's feed once a day, shows its own window when
//! a version is waiting (Remind me later puts it off until the next check), and installs it; the
//! feed, the check interval and the public key that updates must be signed with are in the app's
//! Info.plist.

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool};
use objc2::{class, msg_send};
use objc2_foundation::NSString;

thread_local! {
    /// Sparkle's controller, alive as long as Akyba: it runs the daily checks.
    static CONTROLLER: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
}

/// Loads Sparkle from the app bundle and starts its updater. Does nothing outside Akyba.app.
#[expect(unsafe_code, reason = "Sparkle is an Objective-C framework, loaded and called at run time")]
pub fn start() {
    let controller = unsafe {
        let bundle: *mut AnyObject = msg_send![class!(NSBundle), mainBundle];
        let frameworks: *mut NSString = msg_send![bundle, privateFrameworksPath];
        let Some(frameworks) = frameworks.as_ref() else { return };
        let path = NSString::from_str(&format!("{frameworks}/Sparkle.framework"));
        let sparkle: *mut AnyObject = msg_send![class!(NSBundle), bundleWithPath: &*path];
        if sparkle.is_null() {
            return;
        }
        let loaded: Bool = msg_send![sparkle, load];
        if !loaded.as_bool() {
            return;
        }
        let Some(controller_class) = AnyClass::get(c"SPUStandardUpdaterController") else { return };
        let allocated: *mut AnyObject = msg_send![controller_class, alloc];
        let nothing: *mut AnyObject = std::ptr::null_mut();
        let controller: *mut AnyObject = msg_send![
            allocated,
            initWithStartingUpdater: Bool::YES,
            updaterDelegate: nothing,
            userDriverDelegate: nothing
        ];
        Retained::from_raw(controller)
    };
    if std::env::var_os("AKYBA_UPDATES_DEBUG").is_some() {
        eprintln!("updates: Sparkle {}", if controller.is_some() { "running" } else { "not found" });
    }
    CONTROLLER.with_borrow_mut(|slot| *slot = controller);
}

/// Checks for a new version now, showing Sparkle's window with the answer. Nothing happens in a
/// development build, which has no Sparkle.
#[expect(unsafe_code, reason = "calling Sparkle's controller")]
pub fn check() {
    CONTROLLER.with_borrow(|controller| {
        if let Some(controller) = controller {
            let nothing: *mut AnyObject = std::ptr::null_mut();
            let () = unsafe { msg_send![&**controller, checkForUpdates: nothing] };
        }
    });
}
