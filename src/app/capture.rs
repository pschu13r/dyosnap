//! Taking screenshots and recordings: the capture jobs, choosing the record area and
//! the overlays shown while recording.

use super::*;
use crate::recbar;
use std::sync::atomic::{AtomicU64, Ordering};

/// Counts the capture jobs; see `start_capture_ex`.
static JOB_GEN: AtomicU64 = AtomicU64::new(0);

/// Set by a capture job when it ends, also a cancelled one: its `screencapture` is gone and
/// the arrow can come back (see `platform::reset_cursor`).
pub(super) static CURSOR_RESET: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Pixels per point of the screen under the mouse, where a capture just ended.
/// Wayland gives no mouse position; there it is the first screen.
pub(super) fn capture_scale() -> Option<f32> {
    let displays = platform::displays();
    let i = platform::mouse_position().map_or(0, |m| platform::display_index_at(&displays, m));
    displays.get(i).map(|d| d.scale.max(1.0))
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum SelDrag {
    New(Pos2),
    Move(Pos2, Rect),
    Resize(u8, Rect),
}

/// One screen while choosing the area to record: its frozen picture and where it is.
pub(super) struct SelScreen {
    pub(super) bounds: Rect,
    pub(super) scale: f32,
    /// Kept while choosing, so the picture can be sent to the graphics card again.
    pub(super) img: Option<Arc<ColorImage>>,
    pub(super) tex: Option<TextureHandle>,
}

impl SelScreen {
    /// Where the pixels of the picture are on the screen.
    pub(super) fn grid(&self) -> PixelGrid {
        let size = self.img.as_ref().map_or([0, 0], |i| i.size);
        // The picture decides how many pixels one point is: what is cut out must be what was shown.
        let scale = if size[0] > 0 && self.bounds.width() > 0.0 { size[0] as f32 / self.bounds.width() } else { self.scale };
        PixelGrid { bounds: self.bounds, scale, size }
    }
}

/// The pixels of one screen's picture, and where they are on the screen (global points).
/// A screenshot area is chosen in whole pixels, so what is shown, measured and saved is the same.
#[derive(Clone, Copy)]
pub(super) struct PixelGrid {
    pub(super) bounds: Rect,
    /// Pixels per point.
    pub(super) scale: f32,
    pub(super) size: [usize; 2],
}

impl PixelGrid {
    /// The pixel under the point `p`; the nearest one if `p` is outside the screen.
    pub(super) fn pixel_at(&self, p: Pos2) -> [u32; 2] {
        let q = |v: f32, len: usize| ((v * self.scale).floor().max(0.0) as u32).min(len.max(1) as u32 - 1);
        [q(p.x - self.bounds.min.x, self.size[0]), q(p.y - self.bounds.min.y, self.size[1])]
    }

    /// The area of a drag from `a` to `b`: from the pixel under the press to the pixel under
    /// the release, both included. In points, on the borders of those pixels.
    pub(super) fn drag_rect(&self, a: Pos2, b: Pos2) -> Rect {
        let (pa, pb) = (self.pixel_at(a), self.pixel_at(b));
        let at = |x: u32, y: u32| self.bounds.min + vec2(x as f32, y as f32) / self.scale;
        Rect::from_min_max(at(pa[0].min(pb[0]), pa[1].min(pb[1])), at(pa[0].max(pb[0]) + 1, pa[1].max(pb[1]) + 1))
    }

    /// The pixels `[x0, y0, x1, y1]` of an area (`x1` and `y1` are not included), cut to the
    /// picture. None if it has no pixels.
    pub(super) fn rect_pixels(&self, r: Rect) -> Option<[u32; 4]> {
        let px = |v: f32, len: usize| (v * self.scale).round().clamp(0.0, len as f32) as u32;
        let (x0, y0) = (px(r.min.x - self.bounds.min.x, self.size[0]), px(r.min.y - self.bounds.min.y, self.size[1]));
        let (x1, y1) = (px(r.max.x - self.bounds.min.x, self.size[0]), px(r.max.y - self.bounds.min.y, self.size[1]));
        (x1 > x0 && y1 > y0).then_some([x0, y0, x1, y1])
    }
}

/// Choosing the area to record, or on Linux the area of a screenshot. One overlay window
/// per screen; the area is in global points, so it can be dragged from one screen to another.
pub(super) struct Select {
    /// The action that started it (see `capture_switch`).
    pub(super) action: Action,
    pub(super) screens: Vec<SelScreen>,
    pub(super) rect: Option<Rect>,
    /// The area before "full screen" was clicked, to switch back.
    pub(super) before_full: Option<Rect>,
    pub(super) drag: Option<SelDrag>,
    pub(super) settings_open: bool,
    pub(super) focused: bool,
    /// How often lost pictures and letters were sent again (see show_immediate).
    pub(super) resent: u32,
    /// Linux: the area is chosen and shown for a few seconds before the recording starts
    /// (a plain Wayland window can't frame the area during the recording; GNOME shows no
    /// frame at all).
    pub(super) countdown: Option<Instant>,
    /// Linux: choosing the area of a screenshot, not of a recording. Holds whether the
    /// window was open before (it opens again afterwards). As with `screencapture -i` on
    /// macOS, the picture is taken when the mouse is let go: no button, no Enter.
    pub(super) shot: bool,
    /// The extra folder a screenshot goes into (Linux; see `Action::CaptureTo`).
    pub(super) target: Option<PathBuf>,
    /// Linux: the screen the full-screen window opened on was found (see `place_selection`).
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub(super) placed: bool,
    /// Linux: the screen whose picture the full-screen main window shows (index in `screens`).
    pub(super) shown: usize,
    /// Linux: take the picture without choosing, as soon as the window has the focus (it
    /// needs the focus to set the clipboard on GNOME). The area, or the whole screen the
    /// window is on. The window shows the frozen picture undimmed, so it is not seen.
    pub(super) instant: Option<(Instant, Option<Rect>)>,
    /// Linux "Capture Window" (KDE): the windows, top-most first. A click takes the one under
    /// the mouse, as `screencapture -w` on macOS.
    pub(super) windows: Vec<Rect>,
    /// When the capture was asked for, and whether the time until the first frame is logged.
    pub(super) started: Instant,
    pub(super) logged: bool,
}

impl Select {
    /// A fresh selection: nothing drawn or focused yet. Callers set what differs (`shot`,
    /// `target`, `instant`, `windows`), so the job and the mode switch build the same state.
    pub(super) fn new(action: Action, screens: Vec<SelScreen>, rect: Option<Rect>, started: Instant) -> Self {
        Select {
            action,
            screens,
            rect,
            before_full: None,
            drag: None,
            settings_open: false,
            focused: false,
            resent: 0,
            countdown: None,
            shot: false,
            target: None,
            placed: false,
            shown: 0,
            instant: None,
            windows: Vec::new(),
            started,
            logged: false,
        }
    }

    /// Choosing the area of a recording: all screens, starting with a frame of 75% of the
    /// screen under the mouse, centred.
    pub(super) fn for_record(action: Action, screens: Vec<SelScreen>, started: Instant) -> Self {
        let mouse = platform::mouse_position().unwrap_or(screens[0].bounds.center());
        let si = screens.iter().position(|s| s.bounds.contains(mouse)).unwrap_or(0);
        log::info!("record area: mouse at {mouse:?}, frame on screen {si}");
        let b = screens[si].bounds;
        let rect = Rect::from_center_size(b.center(), b.size() * 0.75);
        Select::new(action, screens, Some(rect), started)
    }
}

/// The title of the area overlay windows (macOS finds them by it, see `platform::raise_overlay`).
const OVERLAY_TITLE: &str = "Select area to record";

/// Linux: how long the chosen area is shown before the recording starts.
pub(super) const COUNTDOWN_SECS: u64 = 3;

#[derive(Default)]
pub(super) enum SelectOut {
    #[default]
    None,
    Cancel,
    Record(Rect),
    /// Take this area of the frozen picture as the screenshot.
    Shot(Rect),
}

/// The area `r` (global points) of the frozen pictures as a PNG in the temp folder.
pub(super) fn crop_screens(screens: &[SelScreen], r: Rect) -> Option<PathBuf> {
    let list: Vec<platform::Display> = screens.iter().map(|s| platform::Display { bounds: s.bounds, scale: s.scale }).collect();
    if list.is_empty() {
        return None;
    }
    let scr = &screens[platform::display_index_at(&list, r.center())];
    let img = scr.img.as_ref()?;
    let iw = img.size[0] as u32;
    let [x0, y0, x1, y1] = scr.grid().rect_pixels(r)?;
    let mut out = image::RgbaImage::new(x1 - x0, y1 - y0);
    for (x, y, p) in out.enumerate_pixels_mut() {
        let c = img.pixels[((y0 + y) * iw + x0 + x) as usize];
        *p = image::Rgba(c.to_srgba_unmultiplied());
    }
    let path = platform::temp_file("shot", "png");
    out.save(&path).ok()?;
    Some(path)
}

pub(super) struct RecState {
    pub(super) rec: Recorder,
    pub(super) rect: Rect,
    /// The screen being recorded (global points).
    pub(super) screen: Rect,
    pub(super) last_secs: u64,
    /// The control bar (macOS; see `recbar`): where it was placed (the builder's position
    /// changes only when the bar is collapsed or expanded, else it would fight a drag),
    /// where it is now, and whether it is collapsed.
    pub(super) bar_pos: Option<Pos2>,
    pub(super) bar_seen: Option<Rect>,
    pub(super) bar_collapsed: bool,
    /// A full-screen recording collapses its bar at this time, unless the mouse is on it.
    pub(super) bar_collapse_at: Option<Instant>,
    /// Linux: the red frame around the area (KDE, Sway, Hyprland, COSMIC; not GNOME).
    /// Dropped with the recording, which removes it.
    #[cfg(target_os = "linux")]
    pub(super) frame: Option<crate::platform::RecFrame>,
}

impl RecState {
    /// Collapses or expands the bar where it is now. The builder gets the new place, so the
    /// window follows.
    fn set_bar_collapsed(&mut self, collapsed: bool, can_pause: bool) {
        let size = recbar::size(collapsed, can_pause);
        let from = self.bar_seen.or(self.bar_pos.map(|p| Rect::from_min_size(p, recbar::size(self.bar_collapsed, can_pause))));
        if let Some(from) = from {
            let at = recbar::resized(from, size, self.screen);
            self.bar_pos = Some(at.min);
            self.bar_seen = Some(at);
        }
        self.bar_collapsed = collapsed;
    }
}

pub(super) fn edges_near(r: Rect, p: Pos2, tol: f32) -> u8 {
    let inside_y = p.y >= r.min.y - tol && p.y <= r.max.y + tol;
    let inside_x = p.x >= r.min.x - tol && p.x <= r.max.x + tol;
    let mut e = 0;
    if inside_y && (p.x - r.min.x).abs() <= tol {
        e |= 1;
    }
    if inside_y && (p.x - r.max.x).abs() <= tol {
        e |= 2;
    }
    if inside_x && (p.y - r.min.y).abs() <= tol {
        e |= 4;
    }
    if inside_x && (p.y - r.max.y).abs() <= tol {
        e |= 8;
    }
    e
}

/// Linux: the chosen area with a red frame and a countdown, before the recording starts.
/// Esc cancels; when the time is up the area is recorded.
pub(super) fn countdown_overlay(ui: &mut Ui, painter: &egui::Painter, full: Rect, lr: Rect, t0: Instant, r: Rect) -> SelectOut {
    let ctx = ui.ctx().clone();
    let dim = Color32::from_black_alpha(120);
    for d in [
        Rect::from_min_max(full.min, pos2(full.max.x, lr.min.y)),
        Rect::from_min_max(pos2(full.min.x, lr.max.y), full.max),
        Rect::from_min_max(pos2(full.min.x, lr.min.y), pos2(lr.min.x, lr.max.y)),
        Rect::from_min_max(pos2(lr.max.x, lr.min.y), pos2(full.max.x, lr.max.y)),
    ] {
        let d = d.intersect(full);
        if d.is_positive() {
            painter.rect_filled(d, 0.0, dim);
        }
    }
    painter.rect_stroke(lr, 0.0, Stroke::new(3.0, RED), StrokeKind::Outside);
    let elapsed = t0.elapsed();
    let left = COUNTDOWN_SECS.saturating_sub(elapsed.as_secs());
    if left == 0 {
        return SelectOut::Record(r);
    }
    let c = lr.center();
    painter.circle_filled(c, 56.0, Color32::from_black_alpha(170));
    painter.text(c, Align2::CENTER_CENTER, left.to_string(), FontId::proportional(64.0), Color32::WHITE);
    let galley = painter.layout_no_wrap("Recording starts   ·   Esc: cancel".into(), FontId::proportional(13.0), Color32::WHITE);
    let hr = Rect::from_center_size(pos2(c.x, c.y + 56.0 + 24.0), galley.size() + vec2(16.0, 8.0));
    painter.rect_filled(hr, 6.0, Color32::from_black_alpha(170));
    painter.galley(hr.min + vec2(8.0, 4.0), galley, Color32::WHITE);
    ctx.set_cursor_icon(egui::CursorIcon::Default);
    if ctx.input(|i| i.key_pressed(Key::Escape) || i.viewport().close_requested()) {
        return SelectOut::Cancel;
    }
    // Wake up when the number changes.
    let next = Duration::from_secs(elapsed.as_secs() + 1).saturating_sub(elapsed);
    ctx.request_repaint_after(next);
    SelectOut::None
}

/// How the frozen picture is sampled: smooth when it is shrunk, but crisp squares when it is
/// magnified, as in the loupe.
fn loupe_options() -> TextureOptions {
    TextureOptions { magnification: egui::TextureFilter::Nearest, ..TextureOptions::LINEAR }
}

/// The two lines of the crosshair, one physical pixel wide, through pixel `px` of a screen
/// whose top-left is at `origin` (in the window). Light with a dark edge: it shows on any picture.
fn draw_crosshair(painter: &egui::Painter, full: Rect, origin: Pos2, scale: f32, px: [u32; 2]) {
    let one = 1.0 / scale;
    let (x, y) = (origin.x + px[0] as f32 * one, origin.y + px[1] as f32 * one);
    let dark = Color32::from_black_alpha(170);
    let vertical = |from: f32, to: f32| Rect::from_min_max(pos2(x + from, full.min.y), pos2(x + to, full.max.y));
    let horizontal = |from: f32, to: f32| Rect::from_min_max(pos2(full.min.x, y + from), pos2(full.max.x, y + to));
    painter.rect_filled(vertical(-one, 2.0 * one), 0.0, dark);
    painter.rect_filled(horizontal(-one, 2.0 * one), 0.0, dark);
    painter.rect_filled(vertical(0.0, one), 0.0, Color32::WHITE);
    painter.rect_filled(horizontal(0.0, one), 0.0, Color32::WHITE);
}

/// Pixels across the loupe, and points per pixel in it.
const LOUPE_PIXELS: u32 = 15;
const LOUPE_CELL: f32 = 8.0;

/// The loupe at the bottom right of the mouse (left or top where that would leave the screen):
/// 15 x 15 pixels around pixel `px`, magnified, the middle one framed, and `info` under it.
#[allow(clippy::too_many_arguments)]
fn draw_loupe(painter: &egui::Painter, full: Rect, origin: Pos2, tex: &TextureHandle, img_size: [usize; 2], px: [u32; 2], scale: f32, info: &str) {
    let side = LOUPE_PIXELS as f32 * LOUPE_CELL;
    let galley = painter.layout_no_wrap(info.into(), FontId::proportional(13.0), Color32::WHITE);
    let info_size = galley.size() + vec2(16.0, 8.0);
    let total = vec2(side.max(info_size.x), side + 4.0 + info_size.y);
    let mouse = origin + vec2(px[0] as f32, px[1] as f32) / scale + vec2(0.5, 0.5) / scale;
    let mut pos = mouse + vec2(20.0, 20.0);
    if pos.x + total.x > full.max.x - 4.0 {
        pos.x = mouse.x - 20.0 - total.x;
    }
    if pos.y + total.y > full.max.y - 4.0 {
        pos.y = mouse.y - 20.0 - total.y;
    }
    let lrect = Rect::from_min_size(pos, vec2(side, side));
    // The pixels around the middle one. Past the edge of the picture the edge pixel repeats.
    let half = (LOUPE_PIXELS / 2) as f32;
    let (w, h) = (img_size[0].max(1) as f32, img_size[1].max(1) as f32);
    let uv = Rect::from_min_max(
        pos2((px[0] as f32 - half) / w, (px[1] as f32 - half) / h),
        pos2((px[0] as f32 + half + 1.0) / w, (px[1] as f32 + half + 1.0) / h),
    );
    painter.add(egui::epaint::RectShape::filled(lrect, 6.0, Color32::WHITE).with_texture(tex.id(), uv));
    // Faint crosshair through the middle pixel, as on the screen.
    let mid = lrect.min + vec2(half + 0.5, half + 0.5) * LOUPE_CELL;
    let cell = Rect::from_center_size(mid, vec2(LOUPE_CELL, LOUPE_CELL));
    for (a, b) in [
        (pos2(lrect.min.x, mid.y), pos2(cell.min.x, mid.y)),
        (pos2(cell.max.x, mid.y), pos2(lrect.max.x, mid.y)),
        (pos2(mid.x, lrect.min.y), pos2(mid.x, cell.min.y)),
        (pos2(mid.x, cell.max.y), pos2(mid.x, lrect.max.y)),
    ] {
        painter.line_segment([a, b], Stroke::new(3.0, Color32::from_black_alpha(50)));
        painter.line_segment([a, b], Stroke::new(1.0, Color32::from_white_alpha(130)));
    }
    painter.rect_stroke(cell, 0.0, Stroke::new(1.0, Color32::BLACK), StrokeKind::Outside);
    painter.rect_stroke(cell, 0.0, Stroke::new(1.0, Color32::WHITE), StrokeKind::Inside);
    painter.rect_stroke(lrect, 6.0, Stroke::new(2.0, Color32::from_black_alpha(220)), StrokeKind::Outside);
    painter.rect_stroke(lrect, 6.0, Stroke::new(1.0, Color32::WHITE), StrokeKind::Inside);
    let irect = Rect::from_min_size(pos2(lrect.min.x, lrect.max.y + 4.0), info_size);
    painter.rect_filled(irect, 5.0, Color32::from_black_alpha(200));
    painter.galley(irect.min + vec2(8.0, 4.0), galley, Color32::WHITE);
}

impl DyoSnap {
    pub(super) fn start_capture(&mut self, ctx: &egui::Context, kind: CaptureKind, for_record: bool) {
        let action = if for_record { Action::Record } else { Action::Capture(kind) };
        self.start_capture_ex(ctx, action, kind, for_record, None);
    }

    /// A screenshot that goes into the extra folder `n` (see `handle_capture`).
    pub(super) fn start_capture_to(&mut self, ctx: &egui::Context, n: u32, target: Option<PathBuf>) {
        self.start_capture_ex(ctx, Action::CaptureTo(n), CaptureKind::Area, false, target);
    }

    /// What the person is choosing right now: a capture that waits for the area or the
    /// window, or the picture it takes first. None while recording or saving a recording:
    /// then no shortcut cancels anything.
    pub(super) fn choosing(&self) -> Option<Action> {
        if self.recording.is_some() || self.finishing.is_some() {
            return None;
        }
        self.select.as_ref().map(|s| s.action).or(self.job.as_ref().map(|j| j.action))
    }

    /// Cancels what is being chosen, without a window or a toast. `restart`: a new capture
    /// starts right after, so the windows are left to it.
    pub(super) fn cancel_choosing(&mut self, ctx: &egui::Context, restart: bool) {
        let had_job = self.job.is_some();
        // The thread may still send something: nobody listens any more (see `start_capture_ex`).
        if let Some(job) = self.job.take() {
            job.cancelled.store(true, Ordering::SeqCst);
            platform::cancel_capture();
        }
        let had_select = self.select.take().is_some();
        if had_select {
            platform::end_screen_session();
        }
        // Linux: the selection is the main window itself, it closes. macOS: the overlays and
        // the crosshair go away on their own; the next picture waits for them, as it does for
        // windows (see `was_visible`).
        let open = if cfg!(target_os = "linux") { had_select } else { had_job || had_select };
        if open {
            self.visible = true;
            if !restart && cfg!(target_os = "linux") {
                self.hide_window(ctx);
            }
        }
    }

    /// Looks for the cameras and microphones in the background, for the record options.
    pub(super) fn list_devices(&self) {
        if let Some(ff) = self.ffmpeg.clone() {
            let slot = self.devices.clone();
            std::thread::spawn(move || {
                let d = platform::list_devices(&ff);
                if let Ok(mut s) = slot.lock() {
                    *s = Some(d);
                }
            });
        }
    }

    fn start_capture_ex(&mut self, ctx: &egui::Context, action: Action, kind: CaptureKind, for_record: bool, target: Option<PathBuf>) {
        if self.busy() {
            return;
        }
        let started = Instant::now();
        if let Some(v) = &mut self.video {
            v.stop();
        }
        // Every DyoSnap window goes away while capturing: the main window and Preferences.
        // None comes back unless the capture opens the editor (or fails). `was_visible` only
        // tells whether to wait for them to vanish before the picture is taken. Also when the
        // main window was shown without show_window, so the flag alone is not enough. macOS:
        // ask AppKit; egui reports a window that was never shown as "not covered".
        #[cfg(target_os = "macos")]
        let shown = platform::window_visible("DyoSnap");
        #[cfg(not(target_os = "macos"))]
        let shown = ctx.input(|i| i.viewport().occluded == Some(false));
        let was_visible = self.visible || self.prefs.open || shown;
        if self.prefs.open {
            let mut out = crate::prefs::PrefsOut::default();
            self.prefs.close(&mut out);
            self.handle_prefs_out(ctx, out);
        }
        self.hide_window(ctx);
        // Linux: an area is chosen in DyoSnap itself, on a frozen picture of the screen, as
        // for recordings. The desktop's own pickers ask for an extra click (GNOME) or offer
        // no area at all (KDE). Full-screen and timed captures go the same way, with a window
        // that takes the picture at once: on GNOME only a focused window can set the
        // clipboard without XWayland.
        #[cfg(target_os = "linux")]
        let lists_windows = platform::can_list_windows();
        #[cfg(not(target_os = "linux"))]
        let lists_windows = false;
        let linux_shot = cfg!(target_os = "linux")
            && !for_record
            && (matches!(kind, CaptureKind::Area | CaptureKind::Previous | CaptureKind::FullScreen | CaptureKind::Timed(_))
                || (kind == CaptureKind::Window && lists_windows));
        // macOS: an area is chosen in the same overlay, so the crosshair and the loupe are
        // the same on both systems and the area is exact to the pixel. Window, full-screen and
        // timed captures, and "Previous" once an area is known, stay with `screencapture`.
        // Only the screen under the mouse is taken: that is faster than all of them.
        let mac_shot = cfg!(target_os = "macos")
            && !for_record
            && (kind == CaptureKind::Area || (kind == CaptureKind::Previous && !platform::last_area_known()));
        let select_shot = linux_shot || mac_shot;
        let first_screen = if mac_shot {
            let displays = platform::displays();
            platform::mouse_position().map_or(0, |m| platform::display_index_at(&displays, m))
        } else {
            0
        };
        let windows = Arc::new(Mutex::new(Vec::new()));
        let windows2 = windows.clone();
        if for_record {
            self.list_devices();
        }
        let (tx, rx) = channel();
        let ctx2 = ctx.clone();
        let cursor = self.settings.capture_cursor;
        let sound = self.settings.sound && !for_record;
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = cancelled.clone();
        let generation = JOB_GEN.fetch_add(1, Ordering::SeqCst) + 1;
        std::thread::spawn(move || {
            // A cancelled job (see `cancel_choosing`) has no receiver. Its files go away, and
            // so does a screen share it may have started, unless a newer job has begun since:
            // that one ends the share itself.
            let send = move |r: Vec<PathBuf>| {
                if let Err(e) = tx.send(r) {
                    for p in e.0 {
                        let _ = std::fs::remove_file(p);
                    }
                    if JOB_GEN.load(Ordering::SeqCst) == generation {
                        platform::end_screen_session();
                    }
                }
                CURSOR_RESET.store(true, Ordering::SeqCst);
                ctx2.request_repaint();
            };
            if was_visible {
                std::thread::sleep(Duration::from_millis(350));
            }
            if let (true, CaptureKind::Timed(s)) = (select_shot, kind) {
                std::thread::sleep(Duration::from_secs(s as u64));
            }
            if stop.load(Ordering::SeqCst) {
                return send(Vec::new());
            }
            // Before the picture: DyoSnap's own window is not open yet.
            #[cfg(target_os = "linux")]
            if select_shot && kind == CaptureKind::Window {
                if let Ok(mut w) = windows2.lock() {
                    *w = platform::window_rects();
                }
            }
            let _ = &windows2;
            let r = if mac_shot {
                platform::capture_screen(first_screen)
            } else if for_record || select_shot {
                // A frozen picture of every screen to choose the area on. Area screenshots
                // leave the mouse pointer out, as on macOS; full-screen ones follow the setting.
                let choosing = select_shot && matches!(kind, CaptureKind::Area | CaptureKind::Previous | CaptureKind::Window);
                platform::capture_all_screens(platform::displays().len(), cursor && !choosing)
            } else {
                platform::run_capture(kind, sound).into_iter().collect()
            };
            send(r);
        });
        self.job = Some(Job { action, cancelled, rx, kind, for_record, select_shot, target, windows, started, first_screen });
    }

    /// A new picture. `target`: an extra folder to save it into (and copy it), whatever
    /// "After screenshot" says: no editor opens.
    pub(super) fn handle_capture(&mut self, ctx: &egui::Context, path: PathBuf, target: Option<PathBuf>) {
        let name = self.settings.make_name("Screenshot");
        // The scale of the screen the picture came from, not of the screen the window was
        // last on: a snap from a normal screen shows at its real size, one from Retina too.
        let mut ppp = capture_scale().unwrap_or(self.native_ppp);
        // "Shrink Retina snaps": store at normal (non-Retina) size.
        if self.settings.shrink_retina && ppp > 1.01 {
            if let Ok(img) = image::open(&path) {
                let w = (img.width() as f32 / ppp).round().max(1.0) as u32;
                let h = (img.height() as f32 / ppp).round().max(1.0) as u32;
                let small = image::imageops::resize(&img.to_rgba8(), w, h, image::imageops::FilterType::Triangle);
                let _ = small.save(&path);
                ppp = 1.0;
            }
        }
        // Keep a copy of the newest screenshot for "Open Latest Image".
        if let Some(latest) = latest_capture_path() {
            if let Some(d) = latest.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            if std::fs::copy(&path, &latest).is_ok() {
                let _ = std::fs::write(latest.with_extension("txt"), &name);
            }
        }
        // "Autosave screenshots" (and the save-and-copy mode) put the capture in the folder right away.
        let mut saved = None;
        let mut save_error = None;
        if let Some(dir) = &target {
            // The folder may be gone since it was added: make it again.
            match std::fs::create_dir_all(dir) {
                Ok(()) => match self.save_capture(&path, dir, &name) {
                    Some(dest) => saved = Some(dest),
                    None => save_error = Some(format!("Could not save to {}", crate::settings::dir_name(dir))),
                },
                Err(e) => save_error = Some(format!("Could not create {}: {e}", dir.display())),
            }
        } else if self.settings.autosave || self.settings.after_capture == AfterCapture::SaveAndCopy {
            let _ = std::fs::create_dir_all(&self.settings.save_dir);
            if let Some(dest) = self.save_capture(&path, &self.settings.save_dir, &name) {
                saved = Some(dest);
                self.recent_dirty = true;
                self.tray_menu_dirty = true;
            }
        }
        // Every capture goes straight to the clipboard, ready to paste. Also when saving
        // into an extra folder failed: then the picture is not lost.
        self.copy_file_to_clipboard(&path, saved.as_deref());
        if let Some(dir) = target {
            match (saved, save_error) {
                (Some(dest), _) => {
                    platform::notify("Saved", &dest.display().to_string());
                    self.toast(format!("Saved to {}", crate::settings::dir_name(&dir)), Some(dest));
                }
                (None, e) => self.toast(e.unwrap_or_default(), None),
            }
            let _ = std::fs::remove_file(&path);
            return;
        }
        match self.settings.after_capture {
            AfterCapture::OpenEditor => {
                match image::open(&path) {
                    Ok(img) => {
                        let mut ed = Editor::new(img.to_rgba8(), name.clone(), ppp);
                        if let Some(p) = &saved {
                            ed.saved = Some(p.clone());
                            ed.saved_name = name;
                        }
                        self.set_editor(ctx, ed);
                    }
                    Err(e) => self.toast(format!("Could not read the capture: {e}"), None),
                }
                self.show_window(ctx);
            }
            AfterCapture::SaveAndCopy => {
                if let Some(dest) = saved {
                    platform::notify("Saved", &dest.display().to_string());
                }
            }
            AfterCapture::CopyOnly => {}
        }
        let _ = std::fs::remove_file(&path);
    }

    pub(super) fn finish_recording(&mut self, ctx: &egui::Context) {
        let Some(r) = self.recording.take() else { return };
        self.set_tray_recording(false);
        let dest = platform::temp_file("screencast", "mp4");
        let (tx, rx) = channel();
        let ctx2 = ctx.clone();
        std::thread::spawn(move || {
            let res = r.rec.finish(dest);
            let _ = tx.send(res);
            ctx2.request_repaint();
        });
        self.finishing = Some(rx);
    }

    /// `root`: on Linux the record area is chosen in the main window itself (full screen):
    /// Wayland can neither place windows nor reliably put a new one in front.
    pub(super) fn select_overlay(&mut self, ctx: &egui::Context, mut root: Option<&mut Ui>) -> SelectOut {
        let Some(sel) = &mut self.select else { return SelectOut::None };
        let settings = &mut self.settings;
        let devices = &self.devices;
        let has_ff = self.ffmpeg.is_some();
        let Select { screens, rect, before_full, drag, settings_open, focused, countdown, shot, shown, instant, placed, windows, started, logged, .. } = sel;
        let (instant, placed) = (*instant, *placed);
        let windows: &[Rect] = windows;
        let shown = *shown;
        let shot_mode = *shot;
        let all = screens.iter().fold(Rect::NOTHING, |a, s| a.union(s.bounds));
        let bounds_list: Vec<Rect> = screens.iter().map(|s| s.bounds).collect();
        let grids: Vec<PixelGrid> = screens.iter().map(|s| s.grid()).collect();
        // The screen that holds `p`, for the pixels of a point.
        let grid_at = |p: Pos2| grids[bounds_list.iter().position(|b| b.contains(p)).unwrap_or(0)];
        let started = *started;
        // The screen that holds the middle of the area gets the Record button.
        let main_i = rect
            .map(|r| {
                let c = r.center();
                bounds_list.iter().position(|b| b.contains(c)).unwrap_or_else(|| {
                    (0..bounds_list.len())
                        .min_by(|a, b| {
                            bounds_list[*a].distance_sq_to_pos(c).total_cmp(&bounds_list[*b].distance_sq_to_pos(c))
                        })
                        .unwrap_or(0)
                })
            })
            .unwrap_or(0);
        let mouse_i = platform::mouse_position()
            .and_then(|m| bounds_list.iter().position(|b| b.contains(m)))
            .unwrap_or(shown);
        // macOS: while dragging, follow the mouse itself, not the events of the window where
        // the drag began. That window can stop getting them once the mouse is on another
        // screen, and the window on that screen has to be drawn again as well.
        #[cfg(target_os = "macos")]
        let (global, released) = match drag.is_some() {
            true => {
                ctx.request_repaint();
                (platform::mouse_position(), !platform::left_button_down())
            }
            false => (None, false),
        };
        #[cfg(not(target_os = "macos"))]
        let (global, released): (Option<Pos2>, bool) = (None, false);
        let mut out = SelectOut::None;
        let mut any_focus_sent = *focused;

        for (si, scr) in screens.iter_mut().enumerate() {
            // One window per physical screen, not per place in the list: after a switch between
            // screenshot and recording a screen keeps its window, and a window never jumps
            // to another screen.
            let vp_id = ViewportId::from_hash_of(("dyosnap_select", scr.bounds.min.x as i32, scr.bounds.min.y as i32));
            let builder = ViewportBuilder::default()
                .with_app_id(crate::platform::APP_ID)
                .with_title(OVERLAY_TITLE)
                .with_decorations(false)
                .with_position(scr.bounds.min)
                .with_inner_size(scr.bounds.size())
                .with_window_level(WindowLevel::AlwaysOnTop)
                .with_resizable(false)
                .with_taskbar(false)
                .with_has_shadow(false);
            let body = |ui: &mut Ui, _: egui::ViewportClass| {
                let ctx = ui.ctx().clone();
                // macOS: above the menu bar and the Dock, which stay on top of a floating window.
                platform::raise_overlay(OVERLAY_TITLE);
                if !*logged {
                    *logged = true;
                    log::info!(target: "dyosnap", "select: overlay shown after {} ms", started.elapsed().as_millis());
                }
                if !any_focus_sent && si == mouse_i {
                    any_focus_sent = true;
                    ctx.send_viewport_cmd(ViewportCommand::Focus);
                }
                if let (None, Some(img)) = (&scr.tex, &scr.img) {
                    scr.tex = Some(ctx.load_texture(format!("select_snapshot_{si}"), img.clone(), loupe_options()));
                }
                let offset = ctx
                    .input(|i| i.viewport().inner_rect.map(|r| r.min.to_vec2()))
                    .unwrap_or(scr.bounds.min.to_vec2());
                let full = ui.max_rect();
                let painter = ui.painter_at(full);
                let screen_rect = scr.bounds.translate(-offset);
                if let Some(t) = &scr.tex {
                    painter.image(t.id(), screen_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                } else {
                    painter.rect_filled(full, 0.0, Color32::from_gray(30));
                }
                if let Some((t0, area)) = instant {
                    // Wait until the window is on its screen and has the focus (or not much
                    // longer: a desktop may never give it).
                    #[cfg(target_os = "linux")]
                    let focused = platform::clipboard_ready();
                    #[cfg(not(target_os = "linux"))]
                    let focused = true;
                    if placed && (focused || t0.elapsed() > Duration::from_millis(800)) {
                        return SelectOut::Shot(area.unwrap_or(scr.bounds));
                    }
                    ctx.request_repaint_after(Duration::from_millis(20));
                    return SelectOut::None;
                }
                let to_local = |r: Rect| r.translate(-offset);
                if let (Some(t0), Some(r)) = (*countdown, *rect) {
                    return countdown_overlay(ui, &painter, full, to_local(r), t0, r);
                }
                let resp = ui.interact(full, Id::new(("sel_bg", si)), Sense::click_and_drag());
                let pointer = global.or_else(|| resp.interact_pointer_pos().map(|p| p + offset));

                if resp.drag_started() {
                    let p = ui.input(|i| i.pointer.press_origin()).unwrap_or_default() + offset;
                    *settings_open = false;
                    *drag = Some(match *rect {
                        // A screenshot area is drawn in one go.
                        _ if shot_mode => SelDrag::New(p),
                        Some(r) => {
                            let e = edges_near(r, p, 8.0);
                            if e != 0 {
                                SelDrag::Resize(e, r)
                            } else if r.contains(p) {
                                SelDrag::Move(p, r)
                            } else {
                                SelDrag::New(p)
                            }
                        }
                        None => SelDrag::New(p),
                    });
                    log::info!(target: "dyosnap", "select: drag started on screen {si} at {p:?}");
                }
                // On the release the mouse can have moved on since the last drag frame.
                if resp.dragged() || (shot_mode && resp.drag_stopped()) || (global.is_some() && drag.is_some()) {
                    if let (Some(d), Some(p)) = (*drag, pointer) {
                        *rect = Some(match d {
                            // A screenshot area is whole pixels of the screen where it began.
                            SelDrag::New(s) if shot_mode => grid_at(s).drag_rect(s, p),
                            SelDrag::New(s) => Rect::from_two_pos(s, p).intersect(all),
                            SelDrag::Move(s, r) => {
                                // The area can move across all screens.
                                let mut nr = r.translate(p - s);
                                let dx = (all.min.x - nr.min.x).max(0.0) + (all.max.x - nr.max.x).min(0.0);
                                let dy = (all.min.y - nr.min.y).max(0.0) + (all.max.y - nr.max.y).min(0.0);
                                nr = nr.translate(vec2(dx, dy));
                                nr
                            }
                            SelDrag::Resize(e, r) => {
                                let mut nr = r;
                                if e & 1 != 0 { nr.min.x = p.x.min(nr.max.x - 20.0); }
                                if e & 2 != 0 { nr.max.x = p.x.max(nr.min.x + 20.0); }
                                if e & 4 != 0 { nr.min.y = p.y.min(nr.max.y - 20.0); }
                                if e & 8 != 0 { nr.max.y = p.y.max(nr.min.y + 20.0); }
                                nr.intersect(all)
                            }
                        });
                        // The other screens show the area too.
                        ctx.request_repaint();
                    }
                }
                let mut shot_taken = None;
                // The pointer is on a button of the record area: no crosshair there.
                let mut over_ui = false;
                // "Capture Window": the window under the mouse, cut to this screen.
                let hovered_window = if shot_mode && drag.is_none() {
                    resp.hover_pos()
                        .map(|p| p + offset)
                        .and_then(|p| windows.iter().find(|w| w.contains(p)))
                        .map(|w| w.intersect(scr.bounds))
                        .filter(|w| w.is_positive())
                } else {
                    None
                };
                if let (true, Some(w)) = (resp.clicked(), hovered_window) {
                    shot_taken = Some(w);
                }
                if (resp.drag_stopped() || released) && drag.is_some() {
                    *drag = None;
                    log::info!(target: "dyosnap", "select: drag stopped on screen {si}, area {rect:?}");
                    // Too small: 3 pixels for a screenshot, 16 points for a recording.
                    let too_small = |r: Rect| match shot_mode {
                        true => grid_at(r.center()).rect_pixels(r).is_none_or(|p| p[2] - p[0] < 3 || p[3] - p[1] < 3),
                        false => r.width() < 16.0 || r.height() < 16.0,
                    };
                    if rect.is_some_and(too_small) {
                        *rect = None;
                    }
                    if shot_mode {
                        shot_taken = *rect;
                    }
                }
                if shot_mode {
                    ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
                } else if let (Some(r), Some(h)) = (*rect, resp.hover_pos().map(|p| p + offset)) {
                    let icon = match edges_near(r, h, 8.0) {
                        1 | 2 => egui::CursorIcon::ResizeHorizontal,
                        4 | 8 => egui::CursorIcon::ResizeVertical,
                        5 | 10 => egui::CursorIcon::ResizeNwSe,
                        6 | 9 => egui::CursorIcon::ResizeNeSw,
                        _ if r.contains(h) => egui::CursorIcon::Move,
                        _ => egui::CursorIcon::Crosshair,
                    };
                    ctx.set_cursor_icon(icon);
                } else {
                    ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
                }

                // Dim everything outside the area.
                let dim = Color32::from_black_alpha(120);
                let mut out = SelectOut::None;
                match hovered_window.or(*rect) {
                    Some(r) => {
                        let lr = to_local(r);
                        for d in [
                            Rect::from_min_max(full.min, pos2(full.max.x, lr.min.y)),
                            Rect::from_min_max(pos2(full.min.x, lr.max.y), full.max),
                            Rect::from_min_max(pos2(full.min.x, lr.min.y), pos2(lr.min.x, lr.max.y)),
                            Rect::from_min_max(pos2(lr.max.x, lr.min.y), pos2(full.max.x, lr.max.y)),
                        ] {
                            let d = d.intersect(full);
                            if d.is_positive() {
                                painter.rect_filled(d, 0.0, dim);
                            }
                        }
                        if shot_mode {
                            // One pixel wide, on the outer pixels of the area: the last row and
                            // column are the ones the crosshair runs through.
                            painter.rect_stroke(lr, 0.0, Stroke::new(1.0 / grids[si].scale, Color32::WHITE), StrokeKind::Inside);
                        } else {
                            painter.rect_stroke(lr, 0.0, Stroke::new(1.5, Color32::WHITE), StrokeKind::Outside);
                        }
                        let handles = if shot_mode { Vec::new() } else {
                            vec![lr.left_top(), lr.right_top(), lr.left_bottom(), lr.right_bottom(), lr.center_top(), lr.center_bottom(), lr.left_center(), lr.right_center()]
                        };
                        for c in handles {
                            painter.circle_filled(c, 5.0, Color32::WHITE);
                            painter.circle_stroke(c, 5.0, Stroke::new(1.0, Color32::from_gray(60)));
                        }
                        if si == main_i && drag.is_none() && !shot_mode {
                            // Top right: the size, and a button for a full-screen recording.
                            let fs = Rect::from_min_size(lr.right_top() + vec2(-8.0 - 26.0, 8.0), vec2(26.0, 26.0));
                            let fs_resp = ui.interact(fs, Id::new(("fullscreen_btn", si)), Sense::click());
                            over_ui |= fs_resp.hovered();
                            let is_full = (r.min - scr.bounds.min).length() < 1.0 && (r.max - scr.bounds.max).length() < 1.0;
                            painter.rect_filled(fs, 4.0, if fs_resp.hovered() || is_full { BLUE } else { Color32::from_black_alpha(190) });
                            icons::draw(&painter, fs.shrink(5.0), Icon::Resize, Color32::WHITE);
                            if fs_resp
                                .on_hover_text(if is_full { "Back to the chosen area" } else { "Record the whole screen" })
                                .clicked()
                            {
                                if is_full {
                                    *rect = Some(before_full.take().unwrap_or_else(|| {
                                        Rect::from_center_size(scr.bounds.center(), scr.bounds.size() * 0.75)
                                    }));
                                } else {
                                    *before_full = Some(r);
                                    *rect = Some(scr.bounds);
                                }
                                ctx.request_repaint();
                            }
                            let label = format!("{} x {}", (r.width() * scr.scale).round(), (r.height() * scr.scale).round());
                            let galley = painter.layout_no_wrap(label, FontId::proportional(13.0), Color32::WHITE);
                            let lrect = Rect::from_min_size(
                                pos2(fs.min.x - galley.size().x - 16.0 - 4.0, fs.min.y),
                                vec2(galley.size().x + 16.0, 26.0),
                            );
                            painter.rect_filled(lrect, 4.0, Color32::from_black_alpha(190));
                            painter.galley(pos2(lrect.min.x + 8.0, lrect.center().y - galley.size().y / 2.0), galley, Color32::WHITE);
                        }
                    }
                    None => {
                        painter.rect_filled(full, 0.0, dim);
                        if si == mouse_i {
                            let msg = if shot_mode && !windows.is_empty() {
                                "Click a window   ·   Drag for an area   ·   Esc: cancel"
                            } else if shot_mode {
                                "Drag to choose the area   ·   Enter: whole screen   ·   Esc: cancel"
                            } else {
                                "Drag to choose the area to record   ·   Enter: whole screen   ·   Esc: cancel"
                            };
                            let galley = painter.layout_no_wrap(msg.into(), FontId::proportional(13.0), Color32::WHITE);
                            let r = Rect::from_center_size(full.center(), galley.size() + vec2(32.0, 20.0));
                            painter.rect_filled(r, 10.0, Color32::from_black_alpha(200));
                            painter.galley(r.min + vec2(16.0, 10.0), galley, Color32::WHITE);
                        }
                    }
                }

                // Record button and options, on the screen that holds the area.
                if let (Some(r), None, true, false) = (*rect, *drag, si == main_i, shot_mode) {
                    let lr = to_local(r);
                    let c = lr.center();
                    let rec_rect = Rect::from_center_size(c - vec2(15.0, 0.0), vec2(84.0, 26.0));
                    let gear_rect = Rect::from_min_size(pos2(rec_rect.max.x + 4.0, rec_rect.min.y), vec2(26.0, 26.0));
                    let rec_resp = ui.interact(rec_rect, Id::new(("rec_btn", si)), Sense::click());
                    over_ui |= rec_resp.hovered();
                    let fill = if rec_resp.hovered() { Color32::from_rgb(240, 58, 58) } else { RED };
                    painter.rect_filled(rec_rect, 4.0, fill);
                    painter.text(rec_rect.center(), Align2::CENTER_CENTER, "Record", FontId::proportional(13.0), Color32::WHITE);
                    if rec_resp.clicked() {
                        out = SelectOut::Record(r);
                    }
                    let gear_resp = ui.interact(gear_rect, Id::new(("gear_btn", si)), Sense::click());
                    over_ui |= gear_resp.hovered();
                    painter.rect_filled(gear_rect, 4.0, if gear_resp.hovered() { Color32::from_gray(95) } else { Color32::from_gray(70) });
                    icons::draw(&painter, gear_rect.shrink(5.0), Icon::Gear, Color32::WHITE);
                    if gear_resp.clicked() {
                        *settings_open = !*settings_open;
                    }
                    // Linux: one screen per selection (see place_selection).
                    let hint = if cfg!(target_os = "linux") {
                        "Drag to move or resize"
                    } else {
                        "Drag to move or resize — also onto another screen"
                    };
                    let galley = painter.layout_no_wrap(hint.into(), FontId::proportional(13.0), Color32::WHITE);
                    let hr = Rect::from_center_size(pos2(c.x, rec_rect.min.y - 20.0), galley.size() + vec2(16.0, 8.0));
                    painter.rect_filled(hr, 6.0, Color32::from_black_alpha(170));
                    painter.galley(hr.min + vec2(8.0, 4.0), galley, Color32::WHITE);
                    if *settings_open {
                        let pos = pos2(gear_rect.center().x - 130.0, gear_rect.max.y + 8.0);
                        egui::Area::new(Id::new(("rec_opts", si)))
                            .fixed_pos(pos)
                            .order(egui::Order::Foreground)
                            .show(&ctx, |ui| {
                                Frame::popup(ui.style()).inner_margin(Margin::same(10)).show(ui, |ui| {
                                    ui.set_width(250.0);
                                    video_options(ui, settings, devices, has_ff);
                                });
                            });
                    }
                }
                // The crosshair and the loupe: wherever a drag would start a new area.
                let local_mouse = ctx.input(|i| i.pointer.latest_pos()).map(|p| p + offset);
                let mouse = match drag.is_some() {
                    true => global.or(local_mouse),
                    false => local_mouse,
                }
                .filter(|p| scr.bounds.contains(*p));
                let grid = grids[si];
                let cross = mouse.filter(|&m| match (shot_mode, *rect) {
                    (true, _) => windows.is_empty(),
                    (false, None) => true,
                    (false, Some(r)) => {
                        let panel = ctx.memory(|mem| mem.area_rect(Id::new(("rec_opts", si))));
                        matches!(drag, Some(SelDrag::New(_)))
                            || (drag.is_none()
                                && !over_ui
                                && edges_near(r, m, 8.0) == 0
                                && !r.contains(m)
                                && !(*settings_open && panel.is_some_and(|p| p.contains(m))))
                    }
                });
                if let (Some(m), Some(img)) = (cross, &scr.img) {
                    ctx.set_cursor_icon(egui::CursorIcon::None);
                    let px = grid.pixel_at(m);
                    let origin = screen_rect.min;
                    draw_crosshair(&painter, full, origin, grid.scale, px);
                    let [cx, cy] = [px[0] as usize, px[1] as usize];
                    let (w, h) = match (*drag, *rect) {
                        (Some(_), Some(r)) => grid.rect_pixels(r).map_or((0, 0), |p| (p[2] - p[0], p[3] - p[1])),
                        _ => (px[0], px[1]),
                    };
                    let c = img.pixels.get(cy * img.size[0] + cx).map_or([0; 4], |c| c.to_srgba_unmultiplied());
                    let info = format!("{w} x {h}   #{:02X}{:02X}{:02X}", c[0], c[1], c[2]);
                    if let Some(t) = &scr.tex {
                        draw_loupe(&painter, full, origin, t, img.size, px, grid.scale, &info);
                    }
                }
                // macOS: the other screens' lines go when the mouse leaves; nothing else
                // wakes a window the mouse is not on.
                #[cfg(target_os = "macos")]
                ctx.request_repaint_after(Duration::from_millis(16));
                let (enter, esc) = ctx.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
                if esc || ctx.input(|i| i.viewport().close_requested()) {
                    out = SelectOut::Cancel;
                } else if let Some(r) = shot_taken {
                    out = SelectOut::Shot(r);
                } else if enter && shot_mode {
                    out = SelectOut::Shot(scr.bounds);
                } else if enter {
                    out = SelectOut::Record(rect.unwrap_or(scr.bounds));
                }
                out
            };
            let res = match root.as_deref_mut() {
                Some(ui) if si == shown => {
                    let mut body = body;
                    body(ui, egui::ViewportClass::Root)
                }
                Some(_) => SelectOut::None,
                None => crate::show_immediate(ctx, vp_id, builder, body),
            };
            if !matches!(res, SelectOut::None) {
                out = res;
            }
        }
        *focused = any_focus_sent;
        out
    }

    pub(super) fn start_recording(&mut self, ctx: &egui::Context, rect: Rect, screen: usize, screen_bounds: Rect, scale: f32) {
        // A recording covers one screen: keep the area inside the screen that holds most of it.
        let mut rect = rect;
        let dx = (screen_bounds.min.x - rect.min.x).max(0.0) + (screen_bounds.max.x - rect.max.x).min(0.0);
        let dy = (screen_bounds.min.y - rect.min.y).max(0.0) + (screen_bounds.max.y - rect.max.y).min(0.0);
        rect = rect.translate(vec2(dx, dy)).intersect(screen_bounds);
        let devices = self.devices.lock().ok().and_then(|d| d.clone()).unwrap_or_default();
        let audio = self.settings.audio.clone().filter(|a| devices.audio_inputs.contains(a) || self.ffmpeg.is_none());
        let opts = RecordOptions {
            rect_pt: [rect.min.x - screen_bounds.min.x, rect.min.y - screen_bounds.min.y, rect.width(), rect.height()],
            rect_global: [rect.min.x, rect.min.y, rect.width(), rect.height()],
            screen,
            scale,
            fps: self.settings.fps,
            quality: self.settings.quality,
            cursor: self.settings.capture_cursor,
            clicks: self.settings.capture_clicks,
            audio,
        };
        #[cfg(target_os = "linux")]
        platform::keep_only_screen(screen_bounds);
        match Recorder::start(opts, self.ffmpeg.clone(), &devices) {
            Ok(rec) => {
                #[cfg(target_os = "linux")]
                let can_pause = rec.can_pause();
                self.rec_started_name = self.settings.make_name("Screencast");
                self.recording = Some(RecState {
                    rec,
                    rect,
                    screen: screen_bounds,
                    last_secs: u64::MAX,
                    bar_pos: None,
                    bar_seen: None,
                    bar_collapsed: false,
                    bar_collapse_at: recbar::is_full_screen(rect, screen_bounds)
                        .then(|| Instant::now() + Duration::from_secs(recbar::AUTO_COLLAPSE_SECS)),
                    #[cfg(target_os = "linux")]
                    frame: crate::platform::RecFrame::show(rect, can_pause),
                });
                self.set_tray_recording(true);
                ctx.send_viewport_cmd(ViewportCommand::RequestPaintWhileHidden);
            }
            Err(e) => {
                self.show_window(ctx);
                self.toast(e, None);
            }
        }
    }

    pub(super) fn recording_overlays(&mut self, ctx: &egui::Context) -> Option<Action> {
        let r = self.recording.as_ref()?;
        let rect = r.rect;
        let screen = r.screen;
        let paused = r.rec.is_paused();
        let can_pause = r.rec.can_pause();
        let elapsed = record::format_elapsed(r.rec.elapsed());
        let full_screen = recbar::is_full_screen(rect, screen);

        // Red frame around the area: four thin solid bars just outside it, so nothing covers
        // the area itself (a see-through window would be recorded as black).
        if !full_screen {
            let (t, g) = (3.0, 2.0);
            let o = rect.expand(g + t);
            let bars = [
                Rect::from_min_size(o.min, vec2(o.width(), t)),
                Rect::from_min_size(pos2(o.min.x, o.max.y - t), vec2(o.width(), t)),
                Rect::from_min_size(pos2(o.min.x, o.min.y + t), vec2(t, o.height() - 2.0 * t)),
                Rect::from_min_size(pos2(o.max.x - t, o.min.y + t), vec2(t, o.height() - 2.0 * t)),
            ];
            let color = if paused { Color32::from_rgb(255, 170, 0) } else { RED };
            for (bi, b) in bars.iter().enumerate() {
                let builder = ViewportBuilder::default()
                    .with_app_id(crate::platform::APP_ID)
                    .with_title("Recording area")
                    .with_decorations(false)
                    .with_mouse_passthrough(true)
                    .with_window_level(WindowLevel::AlwaysOnTop)
                    .with_position(b.min)
                    .with_inner_size(b.size())
                    .with_min_inner_size(vec2(1.0, 1.0))
                    .with_resizable(false)
                    .with_taskbar(false)
                    .with_has_shadow(false)
                    .with_active(false);
                crate::show_immediate(ctx, ViewportId::from_hash_of(("dyosnap_frame", bi)), builder, |ui, _| {
                    ui.painter().rect_filled(ui.max_rect().expand(2.0), 0.0, color);
                });
            }
        }

        // Small control bar below the area (or at the bottom right for a full-screen
        // recording). It moves by dragging its left part and collapses to a pill.
        let r = self.recording.as_mut()?;
        if r.bar_pos.is_none() {
            let at = recbar::initial_rect(rect, screen, can_pause);
            r.bar_pos = Some(at.min);
            r.bar_seen = Some(at);
        }
        // The auto-collapse of a full-screen recording, unless the mouse is on the bar.
        if let Some(t) = r.bar_collapse_at {
            let left = t.saturating_duration_since(Instant::now());
            if left.is_zero() {
                let on_bar = platform::mouse_position().is_some_and(|m| r.bar_seen.is_some_and(|b| b.contains(m)));
                if on_bar {
                    // Wait: it collapses once the mouse leaves.
                    r.bar_collapse_at = Some(Instant::now() + Duration::from_millis(300));
                    ctx.request_repaint_after(Duration::from_millis(300));
                } else {
                    r.bar_collapse_at = None;
                    r.set_bar_collapsed(true, can_pause);
                }
            } else {
                ctx.request_repaint_after(left);
            }
        }
        let collapsed = r.bar_collapsed;
        let size = recbar::size(collapsed, can_pause);
        let pos = r.bar_pos.unwrap_or(Pos2::ZERO);
        let builder = ViewportBuilder::default()
            .with_app_id(crate::platform::APP_ID)
            .with_title("Recording")
            .with_decorations(false)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_position(pos)
            .with_inner_size(size)
            .with_resizable(false)
            .with_taskbar(false)
            .with_active(false);
        let dot_color = if paused { Color32::from_rgb(255, 170, 0) } else { RED };
        let (act, seen, toggle) = crate::show_immediate(ctx, ViewportId::from_hash_of("dyosnap_recbar"), builder, |ui, _| {
            let ctx = ui.ctx().clone();
            let mut act = None;
            let mut toggle = false;
            // Dragging the window is left to the system; it starts on the first movement.
            let grab = |ui: &Ui, resp: &egui::Response| {
                if resp.drag_started() {
                    ctx.send_viewport_cmd(ViewportCommand::StartDrag);
                }
                if resp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                }
            };
            Frame::new().fill(Color32::from_rgb(236, 236, 238)).inner_margin(Margin::symmetric(8, 6)).show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                if collapsed {
                    // The whole pill: a click expands it, a drag moves it.
                    let resp = ui.interact(ui.max_rect().expand2(vec2(8.0, 6.0)), Id::new("recbar_pill"), Sense::click_and_drag());
                    ui.horizontal_centered(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let (dot, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
                        ui.painter().circle_filled(dot.center(), 5.0, dot_color);
                        ui.add(egui::Label::new(egui::RichText::new(elapsed.clone()).size(BAR_FONT).color(Color32::from_gray(30))).selectable(false));
                    });
                    grab(ui, &resp);
                    if resp.on_hover_text("Show the controls").clicked() {
                        toggle = true;
                    }
                    return;
                }
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let (dot, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
                    ui.painter().circle_filled(dot.center(), 5.0, dot_color);
                    let label = ui.add(egui::Label::new(egui::RichText::new(elapsed.clone()).size(BAR_FONT).color(Color32::from_gray(30))).selectable(false));
                    // Dot and timer: the grip.
                    let full = ui.max_rect().expand2(vec2(8.0, 6.0));
                    let grip = Rect::from_min_max(full.min, pos2(label.rect.max.x + 4.0, full.max.y));
                    let resp = ui.interact(grip, Id::new("recbar_grip"), Sense::click_and_drag());
                    grab(ui, &resp);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let cancel = ui.add(
                            egui::Button::new(egui::RichText::new("Cancel").color(Color32::from_gray(40)))
                                .fill(Color32::from_gray(214))
                                .min_size(vec2(58.0, TOOL_H)),
                        );
                        if cancel.clicked() {
                            act = Some(Action::CancelRec);
                        }
                        if can_pause {
                            let (r, pr) = ui.allocate_exact_size(vec2(TOOL_W, TOOL_H), Sense::click());
                            ui.painter().rect_filled(r, 4.0, if pr.hovered() { Color32::from_gray(200) } else { Color32::from_gray(214) });
                            icons::draw(ui.painter(), r.shrink(7.0), if paused { Icon::Play } else { Icon::Pause }, Color32::from_gray(40));
                            if pr.on_hover_text(if paused { "Resume" } else { "Pause" }).clicked() {
                                act = Some(Action::PauseRec);
                            }
                        }
                        let stop = ui.add(
                            egui::Button::new(egui::RichText::new("Stop").color(Color32::WHITE).size(BAR_FONT))
                                .fill(RED)
                                .min_size(vec2(62.0, TOOL_H)),
                        );
                        if stop.clicked() {
                            act = Some(Action::FinishRec);
                        }
                        // Collapse: a minus, left of Stop.
                        let (r, cr) = ui.allocate_exact_size(vec2(recbar::COLLAPSE_W, TOOL_H), Sense::click());
                        ui.painter().rect_filled(r, 4.0, if cr.hovered() { Color32::from_gray(200) } else { Color32::from_gray(214) });
                        ui.painter().line_segment([r.center() - vec2(5.0, 0.0), r.center() + vec2(5.0, 0.0)], Stroke::new(2.0, Color32::from_gray(40)));
                        if cr.on_hover_text("Collapse").clicked() {
                            toggle = true;
                        }
                    });
                });
            });
            let seen = ctx.input(|i| i.viewport().outer_rect);
            (act, seen, toggle)
        });
        // The window's real place, so a collapse starts from where it was dragged to.
        if let Some(r) = self.recording.as_mut() {
            if let Some(s) = seen {
                r.bar_seen = Some(s);
            }
            if toggle {
                r.set_bar_collapsed(!collapsed, can_pause);
                // A bar the person opened or closed by hand stays so.
                r.bar_collapse_at = None;
                ctx.request_repaint();
            }
        }
        act
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A screen of 100 x 50 points whose top-left is at `x` (a second screen to the right).
    fn grid(scale: f32, x: f32) -> PixelGrid {
        PixelGrid {
            bounds: Rect::from_min_size(pos2(x, 0.0), vec2(100.0, 50.0)),
            scale,
            size: [(100.0 * scale) as usize, (50.0 * scale) as usize],
        }
    }

    fn size_of(g: &PixelGrid, r: Rect) -> (u32, u32) {
        let p = g.rect_pixels(r).unwrap();
        (p[2] - p[0], p[3] - p[1])
    }

    #[test]
    fn drag_covers_both_end_pixels_at_scale_1() {
        let g = grid(1.0, 0.0);
        let r = g.drag_rect(pos2(10.0, 10.0), pos2(20.0, 30.0));
        assert_eq!(g.rect_pixels(r), Some([10, 10, 21, 31]));
        // One pixel: press and release on the same one.
        assert_eq!(g.rect_pixels(g.drag_rect(pos2(5.3, 5.9), pos2(5.7, 5.1))), Some([5, 5, 6, 6]));
    }

    #[test]
    fn drag_covers_both_end_pixels_at_scale_2() {
        let g = grid(2.0, 0.0);
        // Points 10.0 and 20.5 are pixels 20 and 41; 10.0 and 30.0 are 20 and 60.
        let r = g.drag_rect(pos2(10.0, 10.0), pos2(20.5, 30.0));
        assert_eq!(g.rect_pixels(r), Some([20, 20, 42, 61]));
        assert_eq!(r, Rect::from_min_max(pos2(10.0, 10.0), pos2(21.0, 30.5)));
        // The size shown is the size saved.
        assert_eq!(size_of(&g, r), ((r.width() * 2.0).round() as u32, (r.height() * 2.0).round() as u32));
    }

    #[test]
    fn drag_direction_does_not_matter() {
        let g = grid(2.0, 0.0);
        let a = g.drag_rect(pos2(10.0, 10.0), pos2(60.5, 40.0));
        assert_eq!(a, g.drag_rect(pos2(60.5, 40.0), pos2(10.0, 10.0)));
        assert_eq!(a, g.drag_rect(pos2(60.5, 10.0), pos2(10.0, 40.0)));
    }

    #[test]
    fn drag_on_a_second_screen_and_out_of_the_screen() {
        let g = grid(2.0, 100.0);
        let r = g.drag_rect(pos2(110.0, 5.0), pos2(500.0, 500.0));
        // Cut at the last pixel of the screen.
        assert_eq!(g.rect_pixels(r), Some([20, 10, 200, 100]));
        let r = g.drag_rect(pos2(90.0, -5.0), pos2(101.0, 2.0));
        assert_eq!(g.rect_pixels(r), Some([0, 0, 3, 5]));
    }

    #[test]
    fn whole_screen_is_all_pixels() {
        for scale in [1.0, 2.0] {
            let g = grid(scale, 0.0);
            assert_eq!(g.rect_pixels(g.bounds), Some([0, 0, g.size[0] as u32, g.size[1] as u32]));
        }
    }

    #[test]
    fn crop_has_the_size_that_was_shown() {
        let size = [200, 100];
        let mut img = ColorImage::filled(size, Color32::BLACK);
        img.pixels[(20 * 200 + 20) as usize] = Color32::from_rgb(10, 20, 30);
        let scr = SelScreen {
            bounds: Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 50.0)),
            scale: 2.0,
            img: Some(Arc::new(img)),
            tex: None,
        };
        let r = scr.grid().drag_rect(pos2(10.0, 10.0), pos2(20.5, 30.0));
        let path = crop_screens(&[scr], r).unwrap();
        let out = image::open(&path).unwrap().to_rgba8();
        let _ = std::fs::remove_file(&path);
        assert_eq!(out.dimensions(), (22, 41));
        assert_eq!(out.get_pixel(0, 0).0, [10, 20, 30, 255]);
    }
}
