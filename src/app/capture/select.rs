//! The record-area selection: its state, the pixel grid and cropping the frozen pictures.

use super::*;

#[derive(Clone, Copy, PartialEq)]
pub(in crate::app) enum SelDrag {
    New(Pos2),
    Move(Pos2, Rect),
    Resize(u8, Rect),
}

/// One screen while choosing the area to record: its frozen picture and where it is.
pub(in crate::app) struct SelScreen {
    pub(in crate::app) bounds: Rect,
    pub(in crate::app) scale: f32,
    /// Kept while choosing, so the picture can be sent to the graphics card again.
    pub(in crate::app) img: Option<Arc<ColorImage>>,
    pub(in crate::app) tex: Option<TextureHandle>,
}

impl SelScreen {
    /// Where the pixels of the picture are on the screen.
    pub(in crate::app) fn grid(&self) -> PixelGrid {
        let size = self.img.as_ref().map_or([0, 0], |i| i.size);
        // The picture decides how many pixels one point is: what is cut out must be what was shown.
        let scale = if size[0] > 0 && self.bounds.width() > 0.0 { size[0] as f32 / self.bounds.width() } else { self.scale };
        PixelGrid { bounds: self.bounds, scale, size }
    }
}

/// The pixels of one screen's picture, and where they are on the screen (global points).
/// A screenshot area is chosen in whole pixels, so what is shown, measured and saved is the same.
#[derive(Clone, Copy)]
pub(in crate::app) struct PixelGrid {
    pub(in crate::app) bounds: Rect,
    /// Pixels per point.
    pub(in crate::app) scale: f32,
    pub(in crate::app) size: [usize; 2],
}

impl PixelGrid {
    /// The pixel under the point `p`; the nearest one if `p` is outside the screen.
    pub(in crate::app) fn pixel_at(&self, p: Pos2) -> [u32; 2] {
        let q = |v: f32, len: usize| ((v * self.scale).floor().max(0.0) as u32).min(len.max(1) as u32 - 1);
        [q(p.x - self.bounds.min.x, self.size[0]), q(p.y - self.bounds.min.y, self.size[1])]
    }

    /// The area of a drag from `a` to `b`: from the pixel under the press to the pixel under
    /// the release, both included. In points, on the borders of those pixels.
    pub(in crate::app) fn drag_rect(&self, a: Pos2, b: Pos2) -> Rect {
        let (pa, pb) = (self.pixel_at(a), self.pixel_at(b));
        let at = |x: u32, y: u32| self.bounds.min + vec2(x as f32, y as f32) / self.scale;
        Rect::from_min_max(at(pa[0].min(pb[0]), pa[1].min(pb[1])), at(pa[0].max(pb[0]) + 1, pa[1].max(pb[1]) + 1))
    }

    /// The pixels `[x0, y0, x1, y1]` of an area (`x1` and `y1` are not included), cut to the
    /// picture. None if it has no pixels.
    pub(in crate::app) fn rect_pixels(&self, r: Rect) -> Option<[u32; 4]> {
        let px = |v: f32, len: usize| (v * self.scale).round().clamp(0.0, len as f32) as u32;
        let (x0, y0) = (px(r.min.x - self.bounds.min.x, self.size[0]), px(r.min.y - self.bounds.min.y, self.size[1]));
        let (x1, y1) = (px(r.max.x - self.bounds.min.x, self.size[0]), px(r.max.y - self.bounds.min.y, self.size[1]));
        (x1 > x0 && y1 > y0).then_some([x0, y0, x1, y1])
    }
}

/// Choosing the area to record, or on Linux the area of a screenshot. One overlay window
/// per screen; the area is in global points, so it can be dragged from one screen to another.
pub(in crate::app) struct Select {
    /// The action that started it (see `capture_switch`).
    pub(in crate::app) action: Action,
    pub(in crate::app) screens: Vec<SelScreen>,
    pub(in crate::app) rect: Option<Rect>,
    /// The area before "full screen" was clicked, to switch back.
    pub(in crate::app) before_full: Option<Rect>,
    pub(in crate::app) drag: Option<SelDrag>,
    pub(in crate::app) settings_open: bool,
    pub(in crate::app) focused: bool,
    /// How often lost pictures and letters were sent again (see show_immediate).
    pub(in crate::app) resent: u32,
    /// Linux: the area is chosen and shown for a few seconds before the recording starts
    /// (a plain Wayland window can't frame the area during the recording; GNOME shows no
    /// frame at all).
    pub(in crate::app) countdown: Option<Instant>,
    /// Linux: choosing the area of a screenshot, not of a recording. Holds whether the
    /// window was open before (it opens again afterwards). As with `screencapture -i` on
    /// macOS, the picture is taken when the mouse is let go: no button, no Enter.
    pub(in crate::app) shot: bool,
    /// The extra folder a screenshot goes into (Linux; see `Action::CaptureTo`).
    pub(in crate::app) target: Option<PathBuf>,
    /// Linux: the screen the full-screen window opened on was found (see `place_selection`).
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub(in crate::app) placed: bool,
    /// Linux: the screen whose picture the full-screen main window shows (index in `screens`).
    pub(in crate::app) shown: usize,
    /// Linux: take the picture without choosing, as soon as the window has the focus (it
    /// needs the focus to set the clipboard on GNOME). The area, or the whole screen the
    /// window is on. The window shows the frozen picture undimmed, so it is not seen.
    pub(in crate::app) instant: Option<(Instant, Option<Rect>)>,
    /// Linux "Capture Window" (KDE): the windows, top-most first. A click takes the one under
    /// the mouse, as `screencapture -w` on macOS.
    pub(in crate::app) windows: Vec<Rect>,
    /// When the capture was asked for, and whether the time until the first frame is logged.
    pub(in crate::app) started: Instant,
    pub(in crate::app) logged: bool,
}

impl Select {
    /// A fresh selection: nothing drawn or focused yet. Callers set what differs (`shot`,
    /// `target`, `instant`, `windows`), so the job and the mode switch build the same state.
    pub(in crate::app) fn new(action: Action, screens: Vec<SelScreen>, rect: Option<Rect>, started: Instant) -> Self {
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
    pub(in crate::app) fn for_record(action: Action, screens: Vec<SelScreen>, started: Instant) -> Self {
        let mouse = platform::mouse_position().unwrap_or(screens[0].bounds.center());
        let si = screens.iter().position(|s| s.bounds.contains(mouse)).unwrap_or(0);
        log::info!("record area: mouse at {mouse:?}, frame on screen {si}");
        let b = screens[si].bounds;
        let rect = Rect::from_center_size(b.center(), b.size() * 0.75);
        Select::new(action, screens, Some(rect), started)
    }
}

/// Linux: how long the chosen area is shown before the recording starts.
pub(in crate::app) const COUNTDOWN_SECS: u64 = 3;

#[derive(Default)]
pub(in crate::app) enum SelectOut {
    #[default]
    None,
    Cancel,
    Record(Rect),
    /// Take this area of the frozen picture as the screenshot.
    Shot(Rect),
}

/// The area `r` (global points) of the frozen pictures as a PNG in the temp folder.
pub(in crate::app) fn crop_screens(screens: &[SelScreen], r: Rect) -> Option<PathBuf> {
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

pub(in crate::app) fn edges_near(r: Rect, p: Pos2, tol: f32) -> u8 {
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
