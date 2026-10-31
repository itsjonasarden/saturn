//! The number on Akyba's Dock icon: agents waiting for you that you haven't looked at yet.

use objc2::MainThreadMarker;
use objc2_app_kit::NSApplication;
use objc2_foundation::NSString;

pub fn set_badge(count: usize) {
    let Some(main_thread) = MainThreadMarker::new() else { return };
    let label = (count > 0).then(|| NSString::from_str(&count.to_string()));
    NSApplication::sharedApplication(main_thread).dockTile().setBadgeLabel(label.as_deref());
}
