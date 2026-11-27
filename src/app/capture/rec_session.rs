//! The recording session: its state, starting it and the overlays shown while it runs.

use super::*;
use crate::recbar;

pub(in crate::app) struct RecState {
    pub(in crate::app) rec: Recorder,
    pub(in crate::app) rect: Rect,
    /// The screen being recorded (global points).
    pub(in crate::app) screen: Rect,
    pub(in crate::app) last_secs: u64,
    /// The control bar (macOS; see `recbar`): where it was placed (the builder's position
    /// changes only when the bar is collapsed or expanded, else it would fight a drag),
    /// where it is now, and whether it is collapsed.
    pub(in crate::app) bar_pos: Option<Pos2>,
    pub(in crate::app) bar_seen: Option<Rect>,
    pub(in crate::app) bar_collapsed: bool,
    /// A full-screen recording collapses its bar at this time, unless the mouse is on it.
    pub(in crate::app) bar_collapse_at: Option<Instant>,
    /// Linux: the red frame around the area (KDE, Sway, Hyprland, COSMIC; not GNOME).
    /// Dropped with the recording, which removes it.
    #[cfg(target_os = "linux")]
    pub(in crate::app) frame: Option<crate::platform::RecFrame>,
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

impl DyoSnap {
    pub(in crate::app) fn finish_recording(&mut self, ctx: &egui::Context) {
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

    pub(in crate::app) fn start_recording(&mut self, ctx: &egui::Context, rect: Rect, screen: usize, screen_bounds: Rect, scale: f32) {
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

    pub(in crate::app) fn recording_overlays(&mut self, ctx: &egui::Context) -> Option<Action> {
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
