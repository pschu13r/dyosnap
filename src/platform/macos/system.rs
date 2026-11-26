//! Opening files and apps, notifications, launch at login, and where logs go.

use block2::{Block, RcBlock};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, NSObject};
use objc2::{ClassType, define_class, msg_send};
use objc2_foundation::NSString;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Opens a file or folder with its default app.
pub fn open_path(path: &Path) {
    let _ = Command::new("/usr/bin/open").arg(path).spawn();
}

/// Shows a file in the Finder.
pub fn reveal_path(path: &Path) {
    let _ = Command::new("/usr/bin/open").arg("-R").arg(path).spawn();
}

/// Opens a file in a given app (an app name like "Preview" or the path of an .app).
pub fn open_with(app: &str, path: &Path) {
    let _ = Command::new("/usr/bin/open").arg("-a").arg(app).arg(path).spawn();
}

/// Shows a system notification. Inside DyoSnap.app it comes from DyoSnap itself (its
/// name and icon; macOS asks once whether DyoSnap may send them). A bare binary has no
/// bundle for that and goes through `osascript`, which shows it as Script Editor's.
pub fn notify(title: &str, body: &str) {
    if in_app_bundle() && notify_from_app(title, body) {
        return;
    }
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!("display notification \"{}\" with title \"DyoSnap\" subtitle \"{}\"", esc(body), esc(title));
    let _ = Command::new("/usr/bin/osascript").args(["-e", &script]).spawn();
}

#[link(name = "UserNotifications", kind = "framework")]
unsafe extern "C" {}

define_class!(
    // Without a delegate macOS shows no banner while DyoSnap is the active app, which
    // it is right after saving in the editor.
    #[unsafe(super(NSObject))]
    #[name = "DyoSnapNotificationDelegate"]
    struct NotifyDelegate;

    impl NotifyDelegate {
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(&self, _center: &AnyObject, _note: &AnyObject, handler: &Block<dyn Fn(usize)>) {
            // UNNotificationPresentationOptionList | UNNotificationPresentationOptionBanner
            handler.call(((1 << 3) | (1 << 4),));
        }
    }
);

/// Sends the notification through UNUserNotificationCenter. False if that is not there.
fn notify_from_app(title: &str, body: &str) -> bool {
    static DELEGATE: std::sync::Once = std::sync::Once::new();
    let Some(cls) = AnyClass::get(c"UNUserNotificationCenter") else { return false };
    let Some(center): Option<Retained<AnyObject>> = (unsafe { msg_send![cls, currentNotificationCenter] }) else {
        return false;
    };
    DELEGATE.call_once(|| {
        let delegate: Retained<NotifyDelegate> = unsafe { msg_send![NotifyDelegate::class(), new] };
        let _: () = unsafe { msg_send![&*center, setDelegate: &*delegate] };
        // The center does not keep the delegate alive.
        std::mem::forget(delegate);
    });
    let (title, body) = (title.to_string(), body.to_string());
    // Ask first (macOS asks the person only once, later this answers at once), then send:
    // so the first notification is not lost while the question is open.
    let send = RcBlock::new(move |granted: Bool, _error: *mut AnyObject| {
        if !granted.as_bool() {
            log::info!(target: "dyosnap", "notifications are not allowed");
            return;
        }
        let (Some(content_cls), Some(request_cls), Some(center_cls)) = (
            AnyClass::get(c"UNMutableNotificationContent"),
            AnyClass::get(c"UNNotificationRequest"),
            AnyClass::get(c"UNUserNotificationCenter"),
        ) else {
            return;
        };
        unsafe {
            let content: Retained<AnyObject> = msg_send![content_cls, new];
            let _: () = msg_send![&*content, setTitle: &*NSString::from_str("DyoSnap")];
            let _: () = msg_send![&*content, setSubtitle: &*NSString::from_str(&title)];
            let _: () = msg_send![&*content, setBody: &*NSString::from_str(&body)];
            let id = NSString::from_str(&format!("{:?}", std::time::SystemTime::now()));
            let none: Option<&AnyObject> = None;
            let Some(request): Option<Retained<AnyObject>> =
                msg_send![request_cls, requestWithIdentifier: &*id, content: &*content, trigger: none]
            else {
                return;
            };
            let Some(center): Option<Retained<AnyObject>> = msg_send![center_cls, currentNotificationCenter] else {
                return;
            };
            let done: Option<&Block<dyn Fn(*mut AnyObject)>> = None;
            let _: () = msg_send![&*center, addNotificationRequest: &*request, withCompletionHandler: done];
        }
    });
    // UNAuthorizationOptionAlert
    let _: () = unsafe { msg_send![&*center, requestAuthorizationWithOptions: 1usize << 2, completionHandler: &*send] };
    true
}

/// Files the Finder asked DyoSnap to open ("Open With", a drop on the app icon), and
/// where they go. Until the menu-bar icon is set up they wait here.
struct Opened {
    waiting: Vec<PathBuf>,
    sink: Option<crate::platform::Sink>,
}

static OPENED: std::sync::Mutex<Opened> = std::sync::Mutex::new(Opened { waiting: Vec::new(), sink: None });

/// From now on, files opened from the Finder go to `sink` like a file chosen under
/// "Recent files"; the ones that came before go there now.
pub fn send_opened_files_to(sink: crate::platform::Sink) {
    let waiting = {
        let Ok(mut o) = OPENED.lock() else { return };
        o.sink = Some(sink.clone());
        std::mem::take(&mut o.waiting)
    };
    for p in waiting {
        sink(format!("recent:{}", p.display()));
    }
}

fn file_opened(path: PathBuf) {
    log::info!(target: "dyosnap", "opened from the Finder: {}", path.display());
    let sink = {
        let Ok(mut o) = OPENED.lock() else { return };
        match o.sink.clone() {
            Some(s) => s,
            None => {
                o.waiting.push(path);
                return;
            }
        }
    };
    sink(format!("recent:{}", path.display()));
}

// Apple Event codes: kCoreEventClass, kAEOpenDocuments, keyDirectObject.
const CORE_EVENT_CLASS: u32 = u32::from_be_bytes(*b"aevt");
const OPEN_DOCUMENTS: u32 = u32::from_be_bytes(*b"odoc");
const DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");

define_class!(
    // The Finder opens a file by sending an "open documents" Apple Event. winit leaves it
    // to AppKit, which hands it to NSDocumentController; that knows no document types
    // and answers "DyoSnap cannot open files in the … format". This takes the event over.
    #[unsafe(super(NSObject))]
    #[name = "DyoSnapOpenHandler"]
    struct OpenHandler;

    impl OpenHandler {
        /// AppKit puts its own handlers in place before this; the files the app was
        /// started with arrive right after it.
        #[unsafe(method(willFinishLaunching:))]
        fn will_finish_launching(&self, _note: &AnyObject) {
            let Some(cls) = AnyClass::get(c"NSAppleEventManager") else { return };
            let manager: Retained<AnyObject> = unsafe { msg_send![cls, sharedAppleEventManager] };
            let _: () = unsafe {
                msg_send![&*manager,
                    setEventHandler: self,
                    andSelector: objc2::sel!(openDocuments:withReplyEvent:),
                    forEventClass: CORE_EVENT_CLASS,
                    andEventID: OPEN_DOCUMENTS]
            };
        }

        #[unsafe(method(openDocuments:withReplyEvent:))]
        fn open_documents(&self, event: &AnyObject, _reply: &AnyObject) {
            let Some(list): Option<Retained<AnyObject>> =
                (unsafe { msg_send![event, paramDescriptorForKeyword: DIRECT_OBJECT] })
            else {
                return;
            };
            // DyoSnap shows one file at a time: the first one is opened.
            let count: isize = unsafe { msg_send![&*list, numberOfItems] };
            let item: Option<Retained<AnyObject>> =
                if count > 0 { unsafe { msg_send![&*list, descriptorAtIndex: 1isize] } } else { Some(list) };
            let Some(item) = item else { return };
            let Some(url): Option<Retained<AnyObject>> = (unsafe { msg_send![&*item, fileURLValue] }) else { return };
            let Some(path): Option<Retained<NSString>> = (unsafe { msg_send![&*url, path] }) else { return };
            file_opened(PathBuf::from(path.to_string()));
        }
    }
);

/// Takes over the Finder's "open documents" event (see `OpenHandler`).
fn listen_for_opened_files() {
    let Some(cls) = AnyClass::get(c"NSNotificationCenter") else { return };
    let handler: Retained<OpenHandler> = unsafe { msg_send![OpenHandler::class(), new] };
    let center: Retained<AnyObject> = unsafe { msg_send![cls, defaultCenter] };
    let name = NSString::from_str("NSApplicationWillFinishLaunchingNotification");
    let none: Option<&AnyObject> = None;
    let _: () = unsafe {
        msg_send![&*center,
            addObserver: &*handler,
            selector: objc2::sel!(willFinishLaunching:),
            name: &*name,
            object: none]
    };
    // Neither the center nor the event manager keeps the handler alive.
    std::mem::forget(handler);
}

// Words for the interface.
pub const THIS_COMPUTER: &str = "this Mac";
pub const FILE_MANAGER: &str = "Finder";
pub const VIDEO_PLAYER: &str = "QuickTime Player";
pub const SCREEN_PERMISSION_HINT: &str = "Allow screen recording in System Settings › Privacy & Security.";
pub const FFMPEG_HINT: &str = "brew install ffmpeg";
/// Image editors offered in the settings: (value stored in the settings, label).
pub const EDITOR_APPS: &[(&str, &str)] = &[("Preview", "Preview"), ("Photos", "Photos")];
pub const DEFAULT_EDITOR: &str = "Preview";
/// Where "Choose…" starts, and the file type it accepts.
pub const EDITOR_DIR: &str = "/Applications";
pub const EDITOR_EXT: &str = "app";

/// The name of an image editor as stored in the settings ("Preview" or a path to an .app).
pub fn editor_label(app: &str) -> String {
    Path::new(app).file_stem().and_then(|s| s.to_str()).unwrap_or(app).to_string()
}

/// Runs once at start. The app bundle tells the system about DyoSnap. Only a LaunchAgent
/// from an earlier version (v0.4.2 and older, or LightSnap run as a bare binary) is moved
/// to the new label. A login item set up through SMAppService can't be carried over: it
/// belongs to the old bundle id (delete LightSnap.app, its Login Items entry goes with it),
/// so "Launch at login" has to be switched on again once.
pub fn prepare() {
    listen_for_opened_files();
    let Some(agents) = dirs::home_dir().map(|h| h.join("Library/LaunchAgents")) else {
        return;
    };
    for label in OLD_AGENT_LABELS {
        let old = agents.join(format!("{label}.plist"));
        if old.is_file() {
            let _ = std::fs::remove_file(&old);
            set_login_item(true);
        }
    }
}

/// Sets up the app's event loop: a menu-bar app without a Dock icon. With `quiet`
/// (check pictures) the app also starts without taking focus.
pub fn event_loop_hook(quiet: bool) -> eframe::EventLoopBuilderHook {
    Box::new(move |b| {
        use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
        // winit would otherwise switch to a regular app with a Dock icon, even with
        // LSUIElement in Info.plist.
        b.with_activation_policy(ActivationPolicy::Accessory);
        if quiet {
            b.with_activate_ignoring_other_apps(false);
        }
    })
}

/// Bold fonts for text annotations, first one found wins.
pub const ANNOTATION_FONTS: &[&str] = &[
    "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
    "/Library/Fonts/Arial Bold.ttf",
    "/System/Library/Fonts/Supplemental/Verdana Bold.ttf",
    "/System/Library/Fonts/Supplemental/Tahoma Bold.ttf",
];

/// Small font with the ⌥ ⌘ ⇧ key symbols, used as a fallback in the interface.
pub const SYMBOL_FONT: &str = "/System/Library/Fonts/Apple Symbols.ttf";

/// Where the crash log goes: ~/Library/Logs/DyoSnap.
pub fn log_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join("Library/Logs/DyoSnap"))
}

// ---------------------------------------------------------------- launch at login
// Inside DyoSnap.app this uses SMAppService, so the app shows up in System Settings ›
// General › Login Items. A bare binary (`cargo run`) can't use SMAppService and falls
// back to a LaunchAgent file. Started at login, the app opens in the menu bar only
// (see `main.rs`: after the first start it always starts quietly).

#[link(name = "ServiceManagement", kind = "framework")]
unsafe extern "C" {}

fn in_app_bundle() -> bool {
    std::env::current_exe()
        .ok()
        .is_some_and(|p| p.to_string_lossy().contains(".app/Contents/MacOS/"))
}

/// Puts DyoSnap's window whose title starts with `title` in front of all windows, also
/// those of other apps, and makes it the key window. Since macOS 14 a menu-bar app can't
/// take the front with winit's "focus" (the old activation call is ignored).
pub fn raise_window(title: &str) {
    let Some(cls) = AnyClass::get(c"NSApplication") else { return };
    unsafe {
        let Some(app): Option<Retained<AnyObject>> = msg_send![cls, sharedApplication] else { return };
        let _: () = msg_send![&*app, activate];
        let Some(windows): Option<Retained<AnyObject>> = msg_send![&*app, windows] else { return };
        let n: usize = msg_send![&*windows, count];
        for i in 0..n {
            let Some(w): Option<Retained<AnyObject>> = msg_send![&*windows, objectAtIndex: i] else { continue };
            let Some(t): Option<Retained<AnyObject>> = msg_send![&*w, title] else { continue };
            let s: *const std::ffi::c_char = msg_send![&*t, UTF8String];
            if s.is_null() || !std::ffi::CStr::from_ptr(s).to_string_lossy().starts_with(title) {
                continue;
            }
            let _: () = msg_send![&*w, orderFrontRegardless];
            let _: () = msg_send![&*w, makeKeyWindow];
        }
    }
}

/// Whether a window with exactly this title is on screen. The main window can be shown
/// without the app knowing (macOS restores it); this asks AppKit itself.
pub fn window_visible(title: &str) -> bool {
    let Some(cls) = AnyClass::get(c"NSApplication") else { return false };
    unsafe {
        let Some(app): Option<Retained<AnyObject>> = msg_send![cls, sharedApplication] else { return false };
        let Some(windows): Option<Retained<AnyObject>> = msg_send![&*app, windows] else { return false };
        let n: usize = msg_send![&*windows, count];
        for i in 0..n {
            let Some(w): Option<Retained<AnyObject>> = msg_send![&*windows, objectAtIndex: i] else { continue };
            let Some(t): Option<Retained<AnyObject>> = msg_send![&*w, title] else { continue };
            let s: *const std::ffi::c_char = msg_send![&*t, UTF8String];
            if s.is_null() || std::ffi::CStr::from_ptr(s).to_string_lossy() != title {
                continue;
            }
            let visible: Bool = msg_send![&*w, isVisible];
            if visible.as_bool() {
                return true;
            }
        }
    }
    false
}

fn main_app_service() -> Option<Retained<AnyObject>> {
    let cls = AnyClass::get(c"SMAppService")?;
    unsafe { msg_send![cls, mainAppService] }
}

pub fn login_item_enabled() -> bool {
    if in_app_bundle() {
        let Some(svc) = main_app_service() else { return false };
        let status: isize = unsafe { msg_send![&*svc, status] };
        // 1 = enabled, 2 = waiting for approval in System Settings.
        return matches!(status, 1 | 2);
    }
    agent_path().is_some_and(|p| p.is_file())
}

/// Starts DyoSnap (in the menu bar, window hidden) when the person logs in.
pub fn set_login_item(on: bool) {
    if in_app_bundle() {
        // Builds before the app bundle wrote a LaunchAgent file; SMAppService replaces it.
        if let Some(p) = agent_path() {
            let _ = std::fs::remove_file(p);
        }
        let Some(svc) = main_app_service() else { return };
        let no_error: *mut *mut AnyObject = std::ptr::null_mut();
        let _ok: bool = unsafe {
            if on {
                msg_send![&*svc, registerAndReturnError: no_error]
            } else {
                msg_send![&*svc, unregisterAndReturnError: no_error]
            }
        };
        return;
    }
    set_launch_agent(on);
}

const AGENT_LABEL: &str = "io.github.pschu13r.dyosnap.login";
const OLD_AGENT_LABELS: &[&str] = &["io.github.badger12345.lightsnap.login", "io.github.pschu13r.lightsnap.login"];

fn agent_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join("Library/LaunchAgents").join(format!("{AGENT_LABEL}.plist")))
}

fn set_launch_agent(on: bool) {
    let Some(path) = agent_path() else { return };
    if !on {
        let _ = std::fs::remove_file(&path);
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    let exe = exe.display().to_string().replace('&', "&amp;").replace('<', "&lt;");
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{AGENT_LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe}</string>
        <string>--background</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>ProcessType</key>
    <string>Interactive</string>
</dict>
</plist>
"#
    );
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(&path, plist);
}
