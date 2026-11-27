//! Screenshots through the Screenshot portal. Area and window use the desktop's own
//! picker (interactive); full screen and timed captures take the picture right away.

use crate::platform::{CaptureKind, temp_file};
use ashpd::desktop::screenshot::Screenshot;
use std::path::{Path, PathBuf};

/// Runs the capture and blocks until it is done. Returns the image file,
/// or None if the person cancelled or capture failed.
pub fn run_capture(kind: CaptureKind, _sound: bool) -> Option<PathBuf> {
    let interactive = match kind {
        // The portal does not say which area was chosen, so there is none to repeat.
        CaptureKind::Area | CaptureKind::Previous | CaptureKind::Window => true,
        CaptureKind::FullScreen => false,
        CaptureKind::Timed(s) => {
            std::thread::sleep(std::time::Duration::from_secs(s as u64));
            false
        }
    };
    let src = portal_screenshot(interactive)?;
    take_file(&src, "shot")
}

/// Nothing to stop: the portal's own picker (Window on GNOME) can't be closed from here,
/// and the job thread of the other captures checks a flag itself. The result of a
/// cancelled job is dropped by the app.
pub fn cancel_capture() {}

/// macOS only: the desktop resets the pointer itself here.
pub fn reset_cursor() {}

/// macOS only: Wayland puts a full-screen window above the panels itself.
pub fn raise_overlay(_title: &str) {}

/// macOS only: the overlay is never open without its picture on Linux.
pub fn set_overlay_visible(_title: &str, _visible: bool) {}

/// macOS only; on Linux the recording bar is a layer surface (see rec_frame.rs).
pub fn raise_recbar(_title: &str) {}

/// macOS only: the part of the screen the Dock and the menu bar leave free.
pub fn visible_frame(screen: egui::Rect) -> egui::Rect {
    crate::recbar::usable_guess(screen)
}

/// macOS only: Linux always takes a picture of every screen (see `capture_all_screens`).
pub fn capture_screen(_index: usize) -> Vec<PathBuf> {
    super::capture_all_screens(super::displays().len(), false)
}

/// macOS only: the app keeps the previous area itself here (`last_area`).
pub fn set_last_area(_r: egui::Rect) {}

/// macOS only: Linux always chooses the area in DyoSnap, so none is ever "unknown".
pub fn last_area_known() -> bool {
    false
}

fn portal_screenshot(interactive: bool) -> Option<PathBuf> {
    super::portal::register();
    ensure_pictures_dir();
    let res = pollster::block_on(async {
        Screenshot::request().interactive(interactive).modal(false).send().await?.response()
    });
    match res {
        Ok(r) => file_uri_to_path(r.uri().as_str()),
        Err(e) => {
            // Cancelled is normal; anything else goes to the log.
            if !matches!(e, ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled)) {
                log::warn!(target: "dyosnap", "screenshot portal: {e}");
            }
            None
        }
    }
}

/// KDE's portal saves the picture in the Pictures folder and fails without a reason when
/// that folder is missing (minimal installs, or deleted). Make it first; DyoSnap moves
/// the picture out again, so the folder stays empty.
fn ensure_pictures_dir() {
    let Some(dir) = dirs::picture_dir().or_else(|| dirs::home_dir().map(|h| h.join("Pictures"))) else { return };
    if !dir.exists() && std::fs::create_dir_all(&dir).is_ok() {
        log::info!(target: "dyosnap", "made the missing Pictures folder {} for the screenshot portal", dir.display());
    }
}

/// Moves the portal's file into DyoSnap's temp folder. The desktop may have saved it
/// in the person's Pictures folder; that copy was made for us and goes away.
fn take_file(src: &Path, prefix: &str) -> Option<PathBuf> {
    let out = temp_file(prefix, "png");
    if std::fs::rename(src, &out).is_err() {
        std::fs::copy(src, &out).ok()?;
        let _ = std::fs::remove_file(src);
    }
    Some(out)
}

/// "file:///home/a/Bild%201.png" → "/home/a/Bild 1.png".
fn file_uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    let mut out = Vec::with_capacity(rest.len());
    let b = rest.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).ok()?, 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    use std::os::unix::ffi::OsStringExt;
    Some(PathBuf::from(std::ffi::OsString::from_vec(out)))
}

/// Whether captures that don't ask first (full screen, timed, record) may run: the
/// desktop remembers in its permission store that the person allowed it.
pub fn screenshot_allowed() -> bool {
    let r: zbus::Result<bool> = pollster::block_on(async {
        let c = zbus::Connection::session().await?;
        let reply = c
            .call_method(
                Some("org.freedesktop.impl.portal.PermissionStore"),
                "/org/freedesktop/impl/portal/PermissionStore",
                Some("org.freedesktop.impl.portal.PermissionStore"),
                "Lookup",
                &("screenshot", "screenshot"),
            )
            .await?;
        let (perms, _data): (std::collections::HashMap<String, Vec<String>>, zbus::zvariant::OwnedValue) =
            reply.body().deserialize()?;
        Ok(perms.get(super::APP_ID).is_some_and(|v| v.iter().any(|p| p == "yes")))
    });
    r.unwrap_or(false)
}

/// Asks the desktop for that permission by taking one picture and throwing it away.
/// GNOME only asks while one of DyoSnap's windows is in front.
pub fn ask_screenshot_permission() -> bool {
    match portal_screenshot(false) {
        Some(p) => {
            let _ = std::fs::remove_file(p);
            true
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn decodes_file_uris() {
        assert_eq!(super::file_uri_to_path("file:///home/a/Bild%201.png").unwrap(), std::path::Path::new("/home/a/Bild 1.png"));
        assert_eq!(super::file_uri_to_path("file:///tmp/x%").unwrap(), std::path::Path::new("/tmp/x%"));
        assert!(super::file_uri_to_path("https://x").is_none());
    }
}
