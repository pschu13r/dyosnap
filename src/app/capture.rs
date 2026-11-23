//! Taking screenshots and recordings: the capture jobs, choosing the record area and
//! the overlays shown while recording.

use super::*;

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

/// Choosing the area to record, or on Linux the area of a screenshot. One overlay window
/// per screen; the area is in global points, so it can be dragged from one screen to another.
pub(super) struct Select {
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
    pub(super) shot: Option<bool>,
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
}

/// Linux: how long the chosen area is shown before the recording starts.
pub(super) const COUNTDOWN_SECS: u64 = 3;

#[derive(Default)]
pub(super) enum SelectOut {
    #[default]
    None,
    Cancel,
    Record(Rect),
    /// Linux: take this area of the frozen picture as the screenshot.
    Shot(Rect),
}

/// Linux: the area `r` (global points) of the frozen pictures as a PNG in the temp folder.
pub(super) fn crop_screens(screens: &[SelScreen], r: Rect) -> Option<PathBuf> {
    let list: Vec<platform::Display> = screens.iter().map(|s| platform::Display { bounds: s.bounds, scale: s.scale }).collect();
    if list.is_empty() {
        return None;
    }
    let scr = &screens[platform::display_index_at(&list, r.center())];
    let img = scr.img.as_ref()?;
    let r = r.intersect(scr.bounds);
    let (iw, ih) = (img.size[0] as u32, img.size[1] as u32);
    let px = |v: f32, max: u32| (v * scr.scale).round().clamp(0.0, max as f32) as u32;
    let (x0, y0) = (px(r.min.x - scr.bounds.min.x, iw), px(r.min.y - scr.bounds.min.y, ih));
    let (x1, y1) = (px(r.max.x - scr.bounds.min.x, iw), px(r.max.y - scr.bounds.min.y, ih));
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
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
    /// Linux: the red frame around the area (KDE, Sway, Hyprland, COSMIC; not GNOME).
    /// Dropped with the recording, which removes it.
    #[cfg(target_os = "linux")]
    pub(super) frame: Option<crate::platform::RecFrame>,
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

impl LightSnap {
    pub(super) fn start_capture(&mut self, ctx: &egui::Context, kind: CaptureKind, for_record: bool) {
        self.start_capture_ex(ctx, kind, for_record, None);
    }

    /// A screenshot that goes into an extra folder (see `handle_capture`).
    pub(super) fn start_capture_to(&mut self, ctx: &egui::Context, kind: CaptureKind, target: Option<PathBuf>) {
        self.start_capture_ex(ctx, kind, false, target);
    }

    fn start_capture_ex(&mut self, ctx: &egui::Context, kind: CaptureKind, for_record: bool, target: Option<PathBuf>) {
        if self.busy() {
            return;
        }
        if let Some(v) = &mut self.video {
            v.stop();
        }
        // Get the window out of the way. Also when it was shown without show_window, so the
        // flag alone is not enough. macOS: ask AppKit; egui reports a window that was never
        // shown as "not covered", which brought the home window up after the first capture.
        #[cfg(target_os = "macos")]
        let shown = platform::window_visible("LightSnap");
        #[cfg(not(target_os = "macos"))]
        let shown = ctx.input(|i| i.viewport().occluded == Some(false));
        let was_visible = self.visible || shown;
        self.hide_window(ctx);
        // Linux: an area is chosen in LightSnap itself, on a frozen picture of the screen, as
        // for recordings. The desktop's own pickers ask for an extra click (GNOME) or offer
        // no area at all (KDE). Full-screen and timed captures go the same way, with a window
        // that takes the picture at once: on GNOME only a focused window can set the
        // clipboard without XWayland.
        #[cfg(target_os = "linux")]
        let lists_windows = platform::can_list_windows();
        #[cfg(not(target_os = "linux"))]
        let lists_windows = false;
        let select_shot = cfg!(target_os = "linux")
            && !for_record
            && (matches!(kind, CaptureKind::Area | CaptureKind::Previous | CaptureKind::FullScreen | CaptureKind::Timed(_))
                || (kind == CaptureKind::Window && lists_windows));
        let windows = Arc::new(Mutex::new(Vec::new()));
        let windows2 = windows.clone();
        if for_record {
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
        let (tx, rx) = channel();
        let ctx2 = ctx.clone();
        let cursor = self.settings.capture_cursor;
        let sound = self.settings.sound && !for_record;
        std::thread::spawn(move || {
            if was_visible {
                std::thread::sleep(Duration::from_millis(350));
            }
            if let (true, CaptureKind::Timed(s)) = (select_shot, kind) {
                std::thread::sleep(Duration::from_secs(s as u64));
            }
            // Before the picture: LightSnap's own window is not open yet.
            #[cfg(target_os = "linux")]
            if select_shot && kind == CaptureKind::Window {
                if let Ok(mut w) = windows2.lock() {
                    *w = platform::window_rects();
                }
            }
            let _ = &windows2;
            let r = if for_record || select_shot {
                // A frozen picture of every screen to choose the area on. Area screenshots
                // leave the mouse pointer out, as on macOS; full-screen ones follow the setting.
                let choosing = select_shot && matches!(kind, CaptureKind::Area | CaptureKind::Previous | CaptureKind::Window);
                platform::capture_all_screens(platform::displays().len(), cursor && !choosing)
            } else {
                platform::run_capture(kind, sound).into_iter().collect()
            };
            let _ = tx.send(r);
            ctx2.request_repaint();
        });
        self.job = Some(Job { rx, kind, for_record, select_shot, show_after: was_visible, target, windows });
    }

    /// A new picture. `target`: an extra folder to save it into (and copy it), whatever
    /// "After screenshot" says: no editor opens.
    pub(super) fn handle_capture(&mut self, ctx: &egui::Context, path: PathBuf, show_after: bool, target: Option<PathBuf>) {
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
            if show_after {
                self.show_window(ctx);
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
                if show_after {
                    self.show_window(ctx);
                }
            }
            AfterCapture::CopyOnly => {
                if show_after {
                    self.show_window(ctx);
                }
            }
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
        let Select { screens, rect, before_full, drag, settings_open, focused, countdown, shot, shown, instant, placed, windows, .. } = sel;
        let (instant, placed) = (*instant, *placed);
        let windows: &[Rect] = windows;
        let shown = *shown;
        let shot_mode = shot.is_some();
        let all = screens.iter().fold(Rect::NOTHING, |a, s| a.union(s.bounds));
        let bounds_list: Vec<Rect> = screens.iter().map(|s| s.bounds).collect();
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
            let builder = ViewportBuilder::default()
                .with_app_id(crate::platform::APP_ID)
                .with_title("Select area to record")
                .with_decorations(false)
                .with_position(scr.bounds.min)
                .with_inner_size(scr.bounds.size())
                .with_window_level(WindowLevel::AlwaysOnTop)
                .with_resizable(false)
                .with_taskbar(false)
                .with_has_shadow(false);
            let body = |ui: &mut Ui, _: egui::ViewportClass| {
                let ctx = ui.ctx().clone();
                if !any_focus_sent && si == mouse_i {
                    any_focus_sent = true;
                    ctx.send_viewport_cmd(ViewportCommand::Focus);
                }
                if let (None, Some(img)) = (&scr.tex, &scr.img) {
                    scr.tex = Some(ctx.load_texture(format!("select_snapshot_{si}"), img.clone(), TextureOptions::LINEAR));
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
                    log::info!(target: "lightsnap", "select: drag started on screen {si} at {p:?}");
                }
                if resp.dragged() || (global.is_some() && drag.is_some()) {
                    if let (Some(d), Some(p)) = (*drag, pointer) {
                        *rect = Some(match d {
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
                    log::info!(target: "lightsnap", "select: drag stopped on screen {si}, area {rect:?}");
                    let min = if shot_mode { 3.0 } else { 16.0 };
                    if rect.is_some_and(|r| r.width() < min || r.height() < min) {
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
                        painter.rect_stroke(lr, 0.0, Stroke::new(1.5, Color32::WHITE), StrokeKind::Outside);
                        if shot_mode {
                            // Only the size, next to the corner being dragged.
                            let label = format!("{} x {}", (r.width() * scr.scale).round(), (r.height() * scr.scale).round());
                            let galley = painter.layout_no_wrap(label, FontId::proportional(13.0), Color32::WHITE);
                            let lrect = Rect::from_min_size(lr.right_bottom() + vec2(8.0, 8.0), galley.size() + vec2(16.0, 8.0));
                            let lrect = lrect.translate(vec2(
                                (full.max.x - 4.0 - lrect.max.x).min(0.0),
                                (full.max.y - 4.0 - lrect.max.y).min(0.0),
                            ));
                            painter.rect_filled(lrect, 4.0, Color32::from_black_alpha(190));
                            painter.galley(lrect.min + vec2(8.0, 4.0), galley, Color32::WHITE);
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
                    let fill = if rec_resp.hovered() { Color32::from_rgb(240, 58, 58) } else { RED };
                    painter.rect_filled(rec_rect, 4.0, fill);
                    painter.text(rec_rect.center(), Align2::CENTER_CENTER, "Record", FontId::proportional(13.0), Color32::WHITE);
                    if rec_resp.clicked() {
                        out = SelectOut::Record(r);
                    }
                    let gear_resp = ui.interact(gear_rect, Id::new(("gear_btn", si)), Sense::click());
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
                None => crate::show_immediate(ctx, ViewportId::from_hash_of(("lightsnap_select", si)), builder, body),
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
        let monitor = screen.size();
        let paused = r.rec.is_paused();
        let can_pause = r.rec.can_pause();
        let elapsed = record::format_elapsed(r.rec.elapsed());
        let full_screen = rect.width() >= monitor.x - 2.0 && rect.height() >= monitor.y - 2.0;
        let _ = monitor;

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
                crate::show_immediate(ctx, ViewportId::from_hash_of(("lightsnap_frame", bi)), builder, |ui, _| {
                    ui.painter().rect_filled(ui.max_rect().expand(2.0), 0.0, color);
                });
            }
        }

        // Small control bar below the area.
        let bar_size = vec2(if can_pause { 256.0 } else { 214.0 }, 40.0);
        // The control bar goes below the area, or above it, on the same screen.
        let mut pos = pos2(rect.center().x - bar_size.x / 2.0, rect.max.y + 12.0);
        if pos.y + bar_size.y > screen.max.y - 4.0 {
            pos.y = rect.min.y - bar_size.y - 12.0;
            if pos.y < screen.min.y + 30.0 {
                pos.y = screen.max.y - bar_size.y - 70.0;
            }
        }
        pos.x = pos.x.clamp(screen.min.x + 4.0, (screen.max.x - bar_size.x - 4.0).max(screen.min.x + 4.0));
        let builder = ViewportBuilder::default()
            .with_app_id(crate::platform::APP_ID)
            .with_title("Recording")
            .with_decorations(false)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_position(pos)
            .with_inner_size(bar_size)
            .with_resizable(false)
            .with_taskbar(false)
            .with_active(false);
        crate::show_immediate(ctx, ViewportId::from_hash_of("lightsnap_recbar"), builder, |ui, _| {
            let mut act = None;
            Frame::new().fill(Color32::from_rgb(236, 236, 238)).inner_margin(Margin::symmetric(8, 6)).show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let (dot, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
                    ui.painter().circle_filled(dot.center(), 5.0, if paused { Color32::from_rgb(255, 170, 0) } else { RED });
                    ui.label(egui::RichText::new(elapsed.clone()).size(BAR_FONT).color(Color32::from_gray(30)));
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
                    });
                });
            });
            act
        })
    }
}
