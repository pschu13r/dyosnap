//! The connected screens and the mouse position, read from CoreGraphics. Coordinates are global
//! points with the origin at the top-left of the main screen, like window positions.

use crate::platform::Display;
use egui::{Pos2, Rect, pos2, vec2};
use std::ffi::c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGSize {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGGetActiveDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> i32;
    fn CGMainDisplayID() -> u32;
    fn CGDisplayBounds(display: u32) -> CGRect;
    fn CGDisplayCopyDisplayMode(display: u32) -> *mut c_void;
    fn CGDisplayModeGetPixelWidth(mode: *mut c_void) -> usize;
    fn CGDisplayModeGetPixelHeight(mode: *mut c_void) -> usize;
    fn CGDisplayModeRelease(mode: *mut c_void);
    fn CGEventCreate(source: *mut c_void) -> *mut c_void;
    fn CGEventGetLocation(event: *mut c_void) -> CGPoint;
    fn CGEventSourceButtonState(state: i32, button: u32) -> bool;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: *const c_void);
}

/// The CoreGraphics ids of the active screens, the main screen first.
pub fn display_ids() -> Vec<u32> {
    let mut ids = [0u32; 16];
    let mut count = 0u32;
    let ok = unsafe { CGGetActiveDisplayList(ids.len() as u32, ids.as_mut_ptr(), &mut count) } == 0;
    if !ok || count == 0 {
        return Vec::new();
    }
    let main = unsafe { CGMainDisplayID() };
    let mut ids: Vec<u32> = ids[..count as usize].to_vec();
    ids.sort_by_key(|id| *id != main);
    ids
}

/// The size of a screen in pixels (not points), as the picture of it has.
pub fn display_pixel_size(id: u32) -> Option<(usize, usize)> {
    unsafe {
        let mode = CGDisplayCopyDisplayMode(id);
        if mode.is_null() {
            return None;
        }
        let size = (CGDisplayModeGetPixelWidth(mode), CGDisplayModeGetPixelHeight(mode));
        CGDisplayModeRelease(mode);
        (size.0 > 0 && size.1 > 0).then_some(size)
    }
}

/// The size of a screen in points, for `display_ids`.
pub fn display_point_size(id: u32) -> (f64, f64) {
    let b = unsafe { CGDisplayBounds(id) };
    (b.size.width, b.size.height)
}

/// All active screens, the main screen first (the same order `screencapture` and
/// ffmpeg's "Capture screen N" use).
pub fn displays() -> Vec<Display> {
    display_ids()
        .into_iter()
        .map(|id| {
            let b = unsafe { CGDisplayBounds(id) };
            let bounds = Rect::from_min_size(
                pos2(b.origin.x as f32, b.origin.y as f32),
                vec2(b.size.width as f32, b.size.height as f32),
            );
            let mut scale = 1.0;
            unsafe {
                let mode = CGDisplayCopyDisplayMode(id);
                if !mode.is_null() {
                    let px = CGDisplayModeGetPixelWidth(mode) as f32;
                    if b.size.width > 0.0 && px > 0.0 {
                        scale = px / b.size.width as f32;
                    }
                    CGDisplayModeRelease(mode);
                }
            }
            Display { bounds, scale }
        })
        .collect()
}

/// Where the mouse pointer is, in global points.
pub fn mouse_position() -> Option<Pos2> {
    unsafe {
        let ev = CGEventCreate(std::ptr::null_mut());
        if ev.is_null() {
            return None;
        }
        let p = CGEventGetLocation(ev);
        CFRelease(ev as *const c_void);
        Some(pos2(p.x as f32, p.y as f32))
    }
}

/// The part of `screen` (global points, as in `displays()`) that the Dock and the menu bar
/// leave free: AppKit's `visibleFrame`. The whole screen if AppKit does not know it.
pub fn visible_frame(screen: Rect) -> Rect {
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2_foundation::NSRect;
    let Some(cls) = AnyClass::get(c"NSScreen") else { return screen };
    // SAFETY: +[NSScreen screens] returns an NSArray of NSScreen; frame and visibleFrame
    // return NSRects. AppKit counts y upwards from the bottom of the first (menu bar) screen.
    unsafe {
        let Some(list): Option<Retained<AnyObject>> = msg_send![cls, screens] else { return screen };
        let n: usize = msg_send![&*list, count];
        if n == 0 {
            return screen;
        }
        let Some(first): Option<Retained<AnyObject>> = msg_send![&*list, objectAtIndex: 0usize] else { return screen };
        let first: NSRect = msg_send![&*first, frame];
        let top = first.origin.y + first.size.height;
        let to_global = |r: NSRect| {
            Rect::from_min_size(
                pos2(r.origin.x as f32, (top - r.origin.y - r.size.height) as f32),
                vec2(r.size.width as f32, r.size.height as f32),
            )
        };
        for i in 0..n {
            let Some(s): Option<Retained<AnyObject>> = msg_send![&*list, objectAtIndex: i] else { continue };
            let frame: NSRect = msg_send![&*s, frame];
            let f = to_global(frame);
            if (f.min - screen.min).length() < 1.0 && (f.size() - screen.size()).length() < 1.0 {
                let visible: NSRect = msg_send![&*s, visibleFrame];
                return to_global(visible).intersect(screen);
            }
        }
    }
    screen
}

/// Whether the left mouse button is held down right now.
pub fn left_button_down() -> bool {
    // kCGEventSourceStateCombinedSessionState, kCGMouseButtonLeft
    unsafe { CGEventSourceButtonState(0, 0) }
}

#[cfg(test)]
mod tests {
    #[test]
    fn lists_screens() {
        let d = super::displays();
        println!("{d:?} mouse={:?}", super::mouse_position());
        assert!(!d.is_empty());
        assert!(d[0].bounds.min == egui::pos2(0.0, 0.0));
    }

    #[test]
    fn visible_frame_is_inside_its_screen() {
        for d in super::displays() {
            let v = super::visible_frame(d.bounds);
            println!("screen {:?} visible {v:?}", d.bounds);
            assert!(d.bounds.contains_rect(v));
            assert!(v.width() > d.bounds.width() / 2.0 && v.height() > d.bounds.height() / 2.0);
        }
    }
}
