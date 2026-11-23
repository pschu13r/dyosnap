//! What the person allowed in System Settings › Privacy & Security. Reading it never
//! shows a prompt.

use crate::platform::{MicStatus, Pane};
use objc2::msg_send;
use objc2::runtime::AnyClass;
use objc2_foundation::NSString;
use std::process::Command;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    /// Silent: tells whether the app may record the screen, and never asks.
    fn CGPreflightScreenCaptureAccess() -> bool;
}

#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" {}

/// Whether LightSnap may read the screen.
pub fn screen_recording_allowed() -> bool {
    unsafe { CGPreflightScreenCaptureAccess() }
}

/// The microphone permission, without asking for it.
pub fn mic_status() -> MicStatus {
    let Some(class) = AnyClass::get(c"AVCaptureDevice") else {
        return MicStatus::NotAsked;
    };
    // AVMediaTypeAudio is the string "soun".
    let media = NSString::from_str("soun");
    // AVAuthorizationStatus: 0 not determined, 1 restricted, 2 denied, 3 authorized.
    let status: isize = unsafe { msg_send![class, authorizationStatusForMediaType: &*media] };
    match status {
        3 => MicStatus::Allowed,
        0 => MicStatus::NotAsked,
        _ => MicStatus::Denied,
    }
}

/// Opens a page of System Settings › Privacy & Security.
pub fn open_privacy(pane: Pane) {
    let anchor = match pane {
        Pane::ScreenRecording => "Privacy_ScreenCapture",
        Pane::Microphone => "Privacy_Microphone",
    };
    let url = format!("x-apple.systempreferences:com.apple.preference.security?{anchor}");
    let _ = Command::new("/usr/bin/open").arg(url).spawn();
}
