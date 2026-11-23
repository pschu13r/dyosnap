//! Screenshots through macOS's own `screencapture` tool.
//! It runs as a short-lived child process, so the app itself stays small and idle.

use crate::platform::{CaptureKind, Display, temp_file};
use egui::{Pos2, Rect, vec2};
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::Mutex;
use std::time::Duration;

/// The area of the last "Capture Area", in global points, for "Capture Previous Area".
static LAST_AREA: Mutex<Option<Rect>> = Mutex::new(None);

/// Runs the capture and blocks until it is done. Returns the image file,
/// or None if the person cancelled (pressed Esc) or capture failed.
pub fn run_capture(kind: CaptureKind, sound: bool) -> Option<PathBuf> {
    let out = temp_file("shot", "png");
    let mut cmd = Command::new("/usr/sbin/screencapture");
    if !sound {
        cmd.arg("-x"); // no shutter sound
    }
    let previous = match kind {
        CaptureKind::Previous => LAST_AREA.lock().ok().and_then(|a| *a),
        _ => None,
    };
    let choose_area = matches!(kind, CaptureKind::Area) || (kind == CaptureKind::Previous && previous.is_none());
    match kind {
        CaptureKind::Area | CaptureKind::Previous => match previous {
            Some(r) => {
                let (x, y) = (r.min.x.round() as i64, r.min.y.round() as i64);
                let (w, h) = (r.width().round() as i64, r.height().round() as i64);
                cmd.args(["-R", &format!("{x},{y},{w},{h}")]);
            }
            None => {
                cmd.arg("-i");
            }
        },
        CaptureKind::Window => {
            cmd.args(["-i", "-w"]);
        }
        CaptureKind::FullScreen => {
            cmd.args(["-D", &mouse_screen().to_string()]);
        }
        CaptureKind::Timed(s) => {
            cmd.args(["-D", &mouse_screen().to_string(), "-T", &s.to_string()]);
        }
    }
    cmd.arg(&out);
    let ok = if choose_area {
        let Ok(child) = cmd.spawn() else { return None };
        let (ok, drag) = wait_watching_mouse(child);
        if ok && out.exists() {
            remember_area(drag, &out);
        }
        ok
    } else {
        cmd.status().map(|s| s.success()).unwrap_or(false)
    };
    if ok && out.exists() { Some(out) } else { None }
}

/// The screen under the mouse as `screencapture -D` counts it: 1 is the main screen, then
/// the others in the order CoreGraphics lists them (the order of `displays()`).
fn mouse_screen() -> usize {
    let displays = super::displays();
    super::mouse_position().map_or(0, |m| crate::platform::display_index_at(&displays, m)) + 1
}

/// Waits for `screencapture -i` and follows the mouse meanwhile: `screencapture` does not
/// say which area was chosen, but the drag does. Returns where the drag started and ended.
fn wait_watching_mouse(mut child: Child) -> (bool, Option<(Pos2, Pos2)>) {
    let (mut start, mut end, mut was_down) = (None, None, false);
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) => {}
            Err(_) => break None,
        }
        let down = super::left_button_down();
        let p = super::mouse_position();
        if down && !was_down {
            start = p;
            end = None;
        } else if !down && was_down {
            end = p;
        }
        was_down = down;
        std::thread::sleep(Duration::from_millis(5));
    };
    let ok = status.is_some_and(|s| s.success());
    // The capture can end before the release was seen: then the mouse is where it ended.
    let end = end.or_else(super::mouse_position);
    (ok, start.zip(end))
}

fn remember_area(drag: Option<(Pos2, Pos2)>, file: &std::path::Path) {
    let Some((start, end)) = drag else {
        log::info!("area capture: no drag seen");
        return;
    };
    let Ok((w, h)) = image::image_dimensions(file) else { return };
    let displays = super::displays();
    let area = area_from_drag(start, end, (w, h), &displays);
    log::info!("area capture: drag {start:?} to {end:?}, picture {w}x{h}, area {area:?}");
    if let (Some(a), Ok(mut last)) = (area, LAST_AREA.lock()) {
        *last = Some(a);
    }
}

/// The chosen area in global points. The picture gives the exact size; the corner where
/// the drag ended gives the place (with Space the area can be moved while choosing, so
/// the start is only used for the direction). A click without a drag picked a window:
/// that is no area to repeat.
fn area_from_drag(start: Pos2, end: Pos2, px: (u32, u32), displays: &[Display]) -> Option<Rect> {
    let d = end - start;
    if d.x.abs() < 3.0 || d.y.abs() < 3.0 {
        return None;
    }
    let scale = displays
        .get(crate::platform::display_index_at(displays, end))
        .map(|s| s.scale.max(1.0))
        .unwrap_or(1.0);
    let size = vec2(px.0 as f32 / scale, px.1 as f32 / scale);
    let other = end - vec2(size.x * d.x.signum(), size.y * d.y.signum());
    let r = Rect::from_two_pos(other, end);
    (r.width() >= 1.0 && r.height() >= 1.0).then_some(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::pos2;

    fn retina() -> Vec<Display> {
        vec![Display { bounds: Rect::from_min_size(pos2(0.0, 0.0), vec2(1512.0, 982.0)), scale: 2.0 }]
    }

    #[test]
    fn area_from_a_drag_in_any_direction() {
        let d = retina();
        let want = Rect::from_min_max(pos2(100.0, 200.0), pos2(400.0, 350.0));
        // Down-right, up-left, and one where Space moved the area on the way.
        assert_eq!(area_from_drag(pos2(100.0, 200.0), pos2(400.0, 350.0), (600, 300), &d), Some(want));
        assert_eq!(area_from_drag(pos2(400.0, 350.0), pos2(100.0, 200.0), (600, 300), &d), Some(want));
        assert_eq!(area_from_drag(pos2(50.0, 50.0), pos2(400.0, 350.0), (600, 300), &d), Some(want));
    }

    #[test]
    fn a_click_is_no_area() {
        assert_eq!(area_from_drag(pos2(100.0, 200.0), pos2(101.0, 200.0), (800, 600), &retina()), None);
    }
}

/// Takes one silent picture of every screen (main screen first), to choose the record
/// area on. `cursor` is for systems that share the screen first (Linux).
pub fn capture_all_screens(count: usize, _cursor: bool) -> Vec<PathBuf> {
    let files: Vec<PathBuf> = (0..count.max(1)).map(|i| temp_file(&format!("screen{i}"), "png")).collect();
    let ok = Command::new("/usr/sbin/screencapture")
        .arg("-x")
        .args(&files)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        return Vec::new();
    }
    files.into_iter().filter(|f| f.exists()).collect()
}

/// Whether captures that don't ask first (full screen, timed) may run. macOS asks by
/// itself when needed, so DyoSnap does not have to.
pub fn screenshot_allowed() -> bool {
    true
}

/// Asks for that permission. Nothing to do on macOS.
pub fn ask_screenshot_permission() -> bool {
    true
}
