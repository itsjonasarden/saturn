//! macOS notifications for agents that need you while Akyba is in the background. macOS only
//! delivers them to a signed app bundle, so a build run from source posts nothing.

use block2::RcBlock;
use objc2::runtime::Bool;
use objc2_foundation::{NSBundle, NSError, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotificationRequest, UNNotificationSound,
    UNUserNotificationCenter,
};

/// Whether Akyba runs as Akyba.app; outside a bundle the notification center refuses to start.
fn available() -> bool {
    NSBundle::mainBundle().bundleIdentifier().is_some()
}

/// Notes what macOS answered in `notifications.log` in Akyba's data folder: a refusal comes back
/// only here, never on screen.
fn note(line: &str) {
    use std::io::Write as _;
    let Some(dir) = crate::settings::data_dir() else { return };
    let _ = std::fs::create_dir_all(&dir);
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_secs());
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("notifications.log")) {
        let _ = writeln!(file, "{stamp} {line}");
    }
}

/// macOS's error, in its words.
#[expect(unsafe_code, reason = "the completion handler hands the error over as a raw pointer")]
fn describe(error: *mut NSError) -> String {
    // SAFETY: AppKit passes a valid NSError or null, alive for the handler's call.
    unsafe { error.as_ref() }.map_or_else(|| "no error".to_owned(), |error| error.localizedDescription().to_string())
}

/// Asks once for permission; macOS remembers the answer.
pub fn request_permission() {
    if !available() {
        note("not in Akyba.app: notifications are off");
        return;
    }
    let done = RcBlock::new(|granted: Bool, error: *mut NSError| {
        note(&format!("permission granted: {}, {}", granted.as_bool(), describe(error)));
    });
    UNUserNotificationCenter::currentNotificationCenter().requestAuthorizationWithOptions_completionHandler(
        UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
        &done,
    );
}

/// Shows a notification. One `id` (one tab) keeps one notification: a newer one replaces it.
pub fn post(id: &str, title: &str, body: &str, sound: bool) {
    if !available() {
        return;
    }
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(title));
    content.setBody(&NSString::from_str(body));
    content.setThreadIdentifier(&NSString::from_str("akyba-agents"));
    if sound {
        content.setSound(Some(&UNNotificationSound::defaultSound()));
    }
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&NSString::from_str(id), &content, None);
    let done = RcBlock::new(|error: *mut NSError| {
        if !error.is_null() {
            note(&format!("a notification was refused: {}", describe(error)));
        }
    });
    UNUserNotificationCenter::currentNotificationCenter()
        .addNotificationRequest_withCompletionHandler(&request, Some(&done));
}
