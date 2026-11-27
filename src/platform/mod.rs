//! Everything that talks to the operating system: screenshots, the clipboard, the
//! list of screens, opening files, notifications, launch at login, and the parts of
//! screen recording that depend on the system.
//!
//! The rest of the app only calls the functions re-exported here. Each system has its
//! own `platform/<os>/` with the same public functions and types: macOS, and Linux
//! (Wayland only).

use egui::{Pos2, Rect};
use std::path::PathBuf;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!("DyoSnap runs on macOS and Linux. A port adds src/platform/<os>/ (see src/platform/mod.rs).");

/// The app id. On Linux it names the .desktop file and is how the portals remember
/// what was allowed.
pub const APP_ID: &str = "io.github.pschu13r.DyoSnap";

/// The app ids of earlier versions: up to v0.4.2 (before the GitHub account was renamed),
/// and LightSnap (up to v1.0.0, before the app became DyoSnap). `prepare()` moves what
/// they left to the new id.
#[cfg(target_os = "linux")]
pub const OLD_APP_IDS: &[&str] = &["io.github.badger12345.LightSnap", "io.github.pschu13r.LightSnap"];

/// LightSnap (up to v1.0.0) kept its files in folders with that name. Moves each to the
/// DyoSnap folder if that does not exist yet: settings and screencast token (config),
/// icons and temp fallback (cache), the logs. Runs at the very start, before anything
/// writes to the new folders. The save folder is not touched: settings keep its path.
pub fn migrate_old_folders() {
    if let Some(d) = dirs::config_dir() {
        move_dir(&d.join("LightSnap"), &d.join("DyoSnap"));
    }
    if let Some(d) = dirs::cache_dir() {
        move_dir(&d.join("lightsnap"), &d.join("dyosnap"));
    }
    if let Some(new) = log_dir() {
        #[cfg(target_os = "linux")]
        let old = new.with_file_name("lightsnap");
        #[cfg(not(target_os = "linux"))]
        let old = new.with_file_name("LightSnap");
        move_dir(&old, &new);
    }
}

/// Renames the real folder `old` to `new`, unless `new` exists or `old` is missing or a link.
fn move_dir(old: &std::path::Path, new: &std::path::Path) {
    let is_dir = std::fs::symlink_metadata(old).is_ok_and(|m| m.file_type().is_dir());
    if is_dir && std::fs::symlink_metadata(new).is_err() {
        let _ = std::fs::rename(old, new);
    }
}

/// The microphone permission (macOS; Linux never reads it).
#[cfg_attr(target_os = "linux", allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MicStatus {
    Allowed,
    /// Denied, or turned off by the system's rules.
    Denied,
    NotAsked,
}

/// A page of the system's privacy settings (macOS).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    ScreenRecording,
    Microphone,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureKind {
    Area,
    /// The same area as the last "Capture Area" again, without choosing (macOS).
    /// Works like `Area` while no area is known yet.
    Previous,
    Window,
    FullScreen,
    Timed(u32),
}

impl CaptureKind {
    #[allow(dead_code)]
    pub fn label(self) -> String {
        match self {
            CaptureKind::Area => "Capture Area".into(),
            CaptureKind::Previous => "Capture Previous Area".into(),
            CaptureKind::Window => "Capture Window".into(),
            CaptureKind::FullScreen => "Capture Full Screen".into(),
            CaptureKind::Timed(s) => format!("Timed Capture ({s} s)"),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Display {
    /// Global position and size in points (origin at the top-left of the main screen).
    pub bounds: Rect,
    /// Pixels per point (2 on Retina screens).
    pub scale: f32,
}

/// The index of the screen that holds `p` (or the closest one).
pub fn display_index_at(displays: &[Display], p: Pos2) -> usize {
    if let Some(i) = displays.iter().position(|d| d.bounds.contains(p)) {
        return i;
    }
    displays
        .iter()
        .enumerate()
        .min_by(|a, b| {
            let da = a.1.bounds.distance_sq_to_pos(p);
            let db = b.1.bounds.distance_sq_to_pos(p);
            da.total_cmp(&db)
        })
        .map(|(i, _)| i)
        .unwrap_or(0)
}

/// A fresh file in the app's own temp folder, like `shot-1731840000000-0.png`.
pub fn temp_file(prefix: &str, ext: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNT: AtomicU64 = AtomicU64::new(0);
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let c = COUNT.fetch_add(1, Ordering::Relaxed);
    temp_dir().join(format!("{prefix}-{n}-{c}.{ext}"))
}

/// The app's temp folder. Screenshots and recordings pass through it, so only this user
/// may see into it (0700). On Linux /tmp is shared by all users: the folder goes into
/// $XDG_RUNTIME_DIR, else into /tmp with the user id in its name. A folder that another
/// user made, or a symlink, is not used.
fn temp_dir() -> PathBuf {
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        // SAFETY: getuid has no preconditions and cannot fail.
        let uid = unsafe { libc::getuid() };
        #[cfg(target_os = "linux")]
        let first = dirs::runtime_dir().map(|d| d.join("dyosnap")).unwrap_or_else(|| std::env::temp_dir().join(format!("dyosnap-{uid}")));
        // macOS: $TMPDIR is already per user.
        #[cfg(not(target_os = "linux"))]
        let first = std::env::temp_dir().join("dyosnap");
        let fallback = dirs::cache_dir().map(|d| d.join("dyosnap").join("tmp"));
        for dir in std::iter::once(first).chain(fallback) {
            if private_dir(&dir, uid) {
                return dir;
            }
            log::warn!(target: "dyosnap", "temp folder {} is not private, not using it", dir.display());
        }
        // Last resort: a new folder with a name no one can guess ahead.
        let dir = std::env::temp_dir().join(format!("dyosnap-{uid}-{}-{}", std::process::id(), random_u64()));
        let _ = private_dir(&dir, uid);
        dir
    })
    .clone()
}

/// Makes `dir` (mode 0700) if needed and checks that it is a real folder of this user
/// that no one else can open.
fn private_dir(dir: &std::path::Path, uid: u32) -> bool {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    if let Some(parent) = dir.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::DirBuilder::new().mode(0o700).create(dir);
    let Ok(meta) = std::fs::symlink_metadata(dir) else { return false };
    if !meta.file_type().is_dir() || meta.uid() != uid {
        return false;
    }
    if meta.mode() & 0o077 != 0 {
        // Ours, but made open by an older DyoSnap: close it.
        return std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).is_ok();
    }
    true
}

fn random_u64() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    h.finish()
}

/// Microphones and screens that the screen recorder can use.
#[derive(Clone, Debug, Default)]
pub struct Devices {
    /// Recorder input index of the main screen.
    pub screen_index: Option<usize>,
    /// Recorder input index for every screen N (in the order of `displays()`).
    pub screens: Vec<(usize, usize)>,
    pub audio_inputs: Vec<String>,
}

/// One entry of the tray menu. The app builds the menu from these, each system shows it
/// its own way.
#[derive(Clone, Debug)]
pub enum MenuEntry {
    Item {
        /// Sent back to the app when the item is chosen. Empty for an item that does nothing.
        id: String,
        text: String,
        enabled: bool,
        /// Shortcut shown next to the item.
        key: Option<crate::settings::Hotkey>,
    },
    Separator,
    Submenu { text: String, items: Vec<MenuEntry> },
}

/// Called with the id of a chosen menu item or a pressed global shortcut. It runs on a
/// system thread, so it only hands the id on.
pub type Sink = std::sync::Arc<dyn Fn(String) + Send + Sync>;

/// The tray icon as RGBA pixels, `TRAY_PX` × `TRAY_PX`: a camera, or a red dot while recording.
/// `light` draws the camera white for dark panels.
pub fn tray_pixels(recording: bool, light: bool) -> Vec<u8> {
    use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Transform};
    let n = TRAY_PX;
    let Some(mut pm) = Pixmap::new(n, n) else { return vec![0; (n * n * 4) as usize] };
    let mut paint = Paint { anti_alias: true, ..Default::default() };
    if recording {
        paint.set_color_rgba8(230, 40, 40, 255);
        if let Some(c) = PathBuilder::from_circle(22.0, 22.0, 13.0) {
            pm.fill_path(&c, &paint, FillRule::Winding, Transform::identity(), None);
        }
    } else {
        let v = if light { 255 } else { 0 };
        paint.set_color_rgba8(v, v, v, 255);
        if let Some(r) = tiny_skia::Rect::from_xywh(5.0, 13.0, 34.0, 23.0) {
            pm.fill_rect(r, &paint, Transform::identity(), None);
        }
        if let Some(r) = tiny_skia::Rect::from_xywh(15.0, 8.0, 14.0, 6.0) {
            pm.fill_rect(r, &paint, Transform::identity(), None);
        }
        paint.blend_mode = tiny_skia::BlendMode::Clear;
        if let Some(c) = PathBuilder::from_circle(22.0, 24.5, 7.5) {
            pm.fill_path(&c, &paint, FillRule::Winding, Transform::identity(), None);
        }
        paint.blend_mode = tiny_skia::BlendMode::SourceOver;
        if let Some(c) = PathBuilder::from_circle(22.0, 24.5, 4.0) {
            pm.fill_path(&c, &paint, FillRule::Winding, Transform::identity(), None);
        }
    }
    pm.take()
}

pub const TRAY_PX: u32 = 44;

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn old_folder_moves_to_new_name() {
        let base = std::env::temp_dir().join(format!("dyosnap-movetest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let (old, new) = (base.join("LightSnap"), base.join("DyoSnap"));
        // Nothing to move: no error, no new folder.
        super::move_dir(&old, &new);
        assert!(!new.exists());
        // Moves with its content.
        std::fs::create_dir(&old).unwrap();
        std::fs::write(old.join("settings.json"), "{}").unwrap();
        super::move_dir(&old, &new);
        assert!(!old.exists());
        assert_eq!(std::fs::read_to_string(new.join("settings.json")).unwrap(), "{}");
        // An existing new folder wins: the old one stays as it is.
        std::fs::create_dir(&old).unwrap();
        std::fs::write(old.join("settings.json"), "old").unwrap();
        super::move_dir(&old, &new);
        assert!(old.join("settings.json").is_file());
        assert_eq!(std::fs::read_to_string(new.join("settings.json")).unwrap(), "{}");
        // A link called LightSnap is not followed.
        let (link, target, fresh) = (base.join("link-old"), base.join("elsewhere"), base.join("link-new"));
        std::fs::create_dir(&target).unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        super::move_dir(&link, &fresh);
        assert!(!fresh.exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn temp_folder_is_private() {
        let uid = unsafe { libc::getuid() };
        let base = std::env::temp_dir().join(format!("dyosnap-privtest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        // New folder: made with 0700.
        let fresh = base.join("fresh");
        assert!(super::private_dir(&fresh, uid));
        assert_eq!(std::fs::metadata(&fresh).unwrap().permissions().mode() & 0o777, 0o700);
        // Our own open folder (older DyoSnap): closed to 0700.
        let open = base.join("open");
        std::fs::create_dir(&open).unwrap();
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o775)).unwrap();
        assert!(super::private_dir(&open, uid));
        assert_eq!(std::fs::metadata(&open).unwrap().permissions().mode() & 0o777, 0o700);
        // A symlink to a folder is refused, even to our own.
        let link = base.join("link");
        std::os::unix::fs::symlink(&fresh, &link).unwrap();
        assert!(!super::private_dir(&link, uid));
        // A folder of another user is refused.
        assert!(!super::private_dir(&fresh, uid + 1));
        // temp_file lands in a private folder and never repeats a name.
        let (a, b) = (super::temp_file("t", "png"), super::temp_file("t", "png"));
        assert_ne!(a, b);
        let dir = a.parent().unwrap();
        assert_eq!(std::fs::metadata(dir).unwrap().permissions().mode() & 0o077, 0);
        std::fs::remove_dir_all(&base).unwrap();
    }
}
