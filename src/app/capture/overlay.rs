//! The record-area overlay: the selection windows, the countdown, the loupe and the crosshair.

use super::*;

/// The title of the area overlay windows (macOS finds them by it, see `platform::raise_overlay`).
const OVERLAY_TITLE: &str = "Select area to record";

/// Linux: the chosen area with a red frame and a countdown, before the recording starts.
/// Esc cancels; when the time is up the area is recorded.
fn countdown_overlay(ui: &mut Ui, painter: &egui::Painter, full: Rect, lr: Rect, t0: Instant, r: Rect) -> SelectOut {
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
    /// `root`: on Linux the record area is chosen in the main window itself (full screen):
    /// Wayland can neither place windows nor reliably put a new one in front.
    pub(in crate::app) fn select_overlay(&mut self, ctx: &egui::Context, mut root: Option<&mut Ui>) -> SelectOut {
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
                if (resp.dragged() || (shot_mode && resp.drag_stopped()) || (global.is_some() && drag.is_some()))
                    && let (Some(d), Some(p)) = (*drag, pointer)
                {
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
}
