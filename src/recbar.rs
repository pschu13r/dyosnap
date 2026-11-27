//! Where the recording control bar (timer, Pause, Stop, Cancel) goes, and how it moves and
//! collapses. Pure geometry in global points, shared by the egui bar on macOS
//! (app/capture/rec_session.rs) and the layer surface on Linux (platform/linux/rec_frame.rs).
//!
//! The bar is recorded in a full-screen recording, so it can be moved, and it collapses to
//! a small pill (red dot and time) that expands again with a click.

// Dragging by hand is for Linux only; on macOS the system drags the window.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use egui::{Pos2, Rect, Vec2, pos2, vec2};

pub const BAR_H: f32 = 40.0;
/// The collapsed bar.
pub const PILL: Vec2 = vec2(80.0, 28.0);
/// The collapse button, and the gap before it.
pub const COLLAPSE_W: f32 = 22.0;
pub const COLLAPSE_GAP: f32 = 6.0;
/// A full-screen recording collapses its bar after this many seconds.
pub const AUTO_COLLAPSE_SECS: u64 = 3;
/// Distance to the screen edge the bar keeps.
const EDGE: f32 = 4.0;
/// Where the bar of a full-screen recording sits: this far from the right and bottom edges
/// of the usable part of the screen (without the Dock, the menu bar or a panel).
const FULL_INSET: f32 = 16.0;
/// Where the system does not say how much a panel takes: room left for it at the bottom.
const PANEL_GUESS: f32 = 54.0;
/// A press that moves more than this many points is a drag, not a click.
const DRAG_MIN: f32 = 3.0;

/// The expanded bar: dot and timer, then the collapse button, Stop, Pause and Cancel.
pub fn bar_size(can_pause: bool) -> Vec2 {
    vec2(if can_pause { 256.0 } else { 214.0 } + COLLAPSE_W + COLLAPSE_GAP, BAR_H)
}

pub fn size(collapsed: bool, can_pause: bool) -> Vec2 {
    if collapsed { PILL } else { bar_size(can_pause) }
}

/// Whether the recorded area is the whole screen.
pub fn is_full_screen(area: Rect, screen: Rect) -> bool {
    area.width() >= screen.width() - 2.0 && area.height() >= screen.height() - 2.0
}

/// Keeps `r` on the screen.
pub fn clamp_to_screen(r: Rect, screen: Rect) -> Rect {
    let min_x = screen.min.x + EDGE;
    let min_y = screen.min.y + EDGE;
    let x = r.min.x.clamp(min_x, (screen.max.x - r.width() - EDGE).max(min_x));
    let y = r.min.y.clamp(min_y, (screen.max.y - r.height() - EDGE).max(min_y));
    Rect::from_min_size(pos2(x, y), r.size())
}

/// The usable part of `screen` where the system does not tell (Linux): all but a strip at
/// the bottom for a panel or dock.
pub fn usable_guess(screen: Rect) -> Rect {
    Rect::from_min_max(screen.min, pos2(screen.max.x, screen.max.y - PANEL_GUESS))
}

/// Where the bar starts: at the bottom right for a full-screen recording; else below the
/// area, or above it, or at the bottom of the screen. `usable`: the part of the screen
/// that no Dock, menu bar or panel covers.
pub fn initial_rect(area: Rect, screen: Rect, usable: Rect, can_pause: bool) -> Rect {
    let size = bar_size(can_pause);
    let corner = pos2(usable.max.x - FULL_INSET - size.x, usable.max.y - FULL_INSET - size.y);
    if is_full_screen(area, screen) {
        return clamp_to_screen(Rect::from_min_size(corner, size), usable);
    }
    let mut pos = pos2(area.center().x - size.x / 2.0, area.max.y + 12.0);
    if pos.y + size.y > usable.max.y - EDGE {
        pos.y = area.min.y - size.y - 12.0;
        if pos.y < usable.min.y + 30.0 {
            pos.y = corner.y;
        }
    }
    clamp_to_screen(Rect::from_min_size(pos, size), usable)
}

/// The bar `cur` with another size (collapsing or expanding). The edge nearest to the
/// screen edge stays where it is, and so does the vertical middle: the pill does not jump.
pub fn resized(cur: Rect, new_size: Vec2, screen: Rect) -> Rect {
    let x = if cur.center().x > screen.center().x { cur.max.x - new_size.x } else { cur.min.x };
    let y = cur.center().y - new_size.y / 2.0;
    clamp_to_screen(Rect::from_min_size(pos2(x, y), new_size), screen)
}

/// The bar `start` after the pointer moved by `delta`, kept on the screen.
pub fn dragged(start: Rect, delta: Vec2, screen: Rect) -> Rect {
    clamp_to_screen(start.translate(delta), screen)
}

/// Whether a press at `press` that is now at `now` is a drag.
pub fn is_drag(press: Pos2, now: Pos2) -> bool {
    (now - press).length() > DRAG_MIN
}

/// The collapse button of a bar `w` wide, left of the buttons that sit at `first_x`
/// (the left edge of the leftmost one).
pub fn collapse_button(first_x: f32) -> Rect {
    let h = 26.0;
    Rect::from_min_size(pos2(first_x - COLLAPSE_GAP - COLLAPSE_W, (BAR_H - h) / 2.0), vec2(COLLAPSE_W, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen() -> Rect {
        Rect::from_min_size(pos2(0.0, 0.0), vec2(1920.0, 1080.0))
    }

    #[test]
    fn bar_goes_below_above_or_at_the_bottom() {
        let mid = Rect::from_min_size(pos2(500.0, 200.0), vec2(400.0, 300.0));
        let b = initial_rect(mid, screen(), usable_guess(screen()), true);
        assert_eq!(b.min.y, 512.0);
        assert!(!b.intersects(mid));
        // No room below: above.
        let low = Rect::from_min_size(pos2(500.0, 700.0), vec2(400.0, 360.0));
        assert!(initial_rect(low, screen(), usable_guess(screen()), true).max.y <= low.min.y);
        // Nearly the whole screen: at the bottom, on screen.
        let tall = Rect::from_min_size(pos2(500.0, 10.0), vec2(400.0, 1060.0));
        assert!(screen().contains_rect(initial_rect(tall, screen(), usable_guess(screen()), false)));
    }

    #[test]
    fn full_screen_bar_sits_at_the_bottom_right() {
        let b = initial_rect(screen(), screen(), usable_guess(screen()), true);
        assert_eq!(b.max.x, 1920.0 - 16.0);
        assert_eq!(b.max.y, 1080.0 - 70.0);
        assert!(is_full_screen(screen(), screen()));
        assert!(!is_full_screen(Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 1080.0)), screen()));
    }

    #[test]
    fn full_screen_bar_stays_clear_of_a_dock_on_the_right() {
        // Menu bar at the top, Dock of 70 points on the right.
        let usable = Rect::from_min_max(pos2(0.0, 25.0), pos2(1850.0, 1080.0));
        let b = initial_rect(screen(), screen(), usable, true);
        assert_eq!(b.max.x, 1850.0 - 16.0);
        assert_eq!(b.max.y, 1080.0 - 16.0);
    }

    #[test]
    fn pill_keeps_the_edge_nearest_to_the_screen_edge() {
        let right = initial_rect(screen(), screen(), usable_guess(screen()), true);
        let pill = resized(right, PILL, screen());
        assert_eq!(pill.max.x, right.max.x);
        assert_eq!(pill.center().y, right.center().y);
        assert_eq!(resized(pill, bar_size(true), screen()), right);
        let left = Rect::from_min_size(pos2(100.0, 500.0), bar_size(true));
        assert_eq!(resized(left, PILL, screen()).min.x, 100.0);
    }

    #[test]
    fn dragging_stays_on_the_screen() {
        let start = Rect::from_min_size(pos2(100.0, 100.0), bar_size(true));
        let r = dragged(start, vec2(-500.0, -500.0), screen());
        assert_eq!(r.min, pos2(4.0, 4.0));
        let r = dragged(start, vec2(5000.0, 5000.0), screen());
        assert_eq!(r.max, pos2(1916.0, 1076.0));
        assert_eq!(dragged(start, vec2(10.0, 20.0), screen()).min, pos2(110.0, 120.0));
    }

    #[test]
    fn a_small_movement_is_a_click() {
        assert!(!is_drag(pos2(10.0, 10.0), pos2(12.0, 11.0)));
        assert!(is_drag(pos2(10.0, 10.0), pos2(14.0, 10.0)));
    }

    #[test]
    fn collapse_button_fits_in_the_bar() {
        // Left of Stop, Pause and Cancel (8 + 58 + 6 + 36 + 6 + 62 from the right).
        let first = bar_size(true).x - 8.0 - 58.0 - 6.0 - 36.0 - 6.0 - 62.0;
        let b = collapse_button(first);
        assert!(b.min.x > 70.0, "{b:?} covers the timer");
    }
}
