//! The per-frame loop: `logic` and `ui` of the eframe app.

use super::*;

impl eframe::App for DyoSnap {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        #[cfg(target_os = "linux")]
        {
            self.place_selection(_frame);
            if let Some(w) = _frame.winit_window() {
                platform::attach_clipboard(w);
                let focused = ctx.input(|i| i.viewport().focused) == Some(true);
                if std::mem::take(&mut self.raise) && !focused {
                    platform::activate(w);
                }
            }
            if let Some(t) = self.clipboard_wait {
                if platform::clipboard_ready_known() || t.elapsed() > Duration::from_millis(1500) {
                    self.clipboard_wait = None;
                    self.open_clipboard(ctx);
                } else {
                    ctx.request_repaint_after(Duration::from_millis(20));
                }
            }
        }
        #[cfg(target_os = "macos")]
        if let Some(w) = _frame.winit_window() {
            crate::remember_root_window(w);
        }
        // Remember the screen size while the main window is around.
        if let Some(m) = ctx.input(|i| i.viewport().monitor_size) && m.x > 100.0 {
            self.monitor = m;
        }
        if let Some(p) = ctx.input(|i| i.viewport().native_pixels_per_point) {
            self.native_ppp = p;
        }

        // The window can get shown by macOS itself; keep the flag honest.
        // Not on Linux: there the window is closed to hide it, and it still reports focus
        // in its last frames, which would open it again (for example while recording).
        if cfg!(target_os = "macos") && ctx.input(|i| i.viewport().focused) == Some(true) {
            self.visible = true;
        }

        self.resend_lost_uploads(ctx);
        self.handle_events(ctx);
        self.reset_cursor_after_capture(ctx);
        if test_env("LS_TEST_AUTOREC") {
            if let Some(r) = &self.recording && r.rec.elapsed() > Duration::from_secs(3) {
                eprintln!("TEST rect={:?} screen={:?}", r.rect, r.screen);
                self.run_action(ctx, Action::FinishRec);
            }
            if let Some(v) = &self.video && !self.test_dialog_done {
                self.test_dialog_done = true;
                eprintln!("TEST video={}", v.path.display());
            }
        }
        if test_env("LS_TEST_RECORD") && !self.test_prefs_done {
            self.test_prefs_done = true;
            self.run_action(ctx, Action::Record);
        }

        if test_env("LS_TEST_EDIT") && !self.test_dialog_done && let Some(ed) = &mut self.ed {
            self.test_dialog_done = true;
            let font = self.font.clone();
            ed.apply_resize(&font, 300, 200);
        }
        self.auto_copy_edits(ctx);
        self.test_open_prefs(ctx);

        // Closing the window keeps DyoSnap in the menu bar. On Linux the window really
        // closes (Wayland can't hide it) and main.rs keeps the app running.
        // (A window that shows only Preferences handles its close request itself.)
        #[cfg(target_os = "linux")]
        let prefs_root = self.root_prefs;
        #[cfg(not(target_os = "linux"))]
        let prefs_root = false;
        // Without a tray host the window is put away too: the shortcuts still work.
        #[cfg(target_os = "linux")]
        let can_hide = self.tray.is_some() || self.no_tray;
        #[cfg(not(target_os = "linux"))]
        let can_hide = self.tray.is_some();
        if ctx.input(|i| i.viewport().close_requested()) && can_hide && !prefs_root {
            #[cfg(not(target_os = "linux"))]
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            if let Some(v) = &mut self.video {
                v.stop();
            }
            self.hide_window(ctx);
        }

        if let Some(t) = &self.toast {
            let now = Instant::now();
            if now >= t.until {
                self.toast = None;
            } else {
                ctx.request_repaint_after(t.until - now);
            }
        }

        if let Some(s) = &mut self.shot {
            s.frames += 1;
            if s.frames >= 120 && !s.sent {
                s.sent = true;
                let path = s.path.clone();
                ctx.send_viewport_cmd(ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |img| {
                    let [w, h] = img.size;
                    let mut raw = Vec::with_capacity(w * h * 4);
                    for p in &img.pixels {
                        raw.extend_from_slice(&p.to_srgba_unmultiplied());
                    }
                    if let Some(buf) = image::RgbaImage::from_raw(w as u32, h as u32, raw) {
                        if let Some(d) = path.parent() {
                            let _ = std::fs::create_dir_all(d);
                        }
                        let _ = buf.save(&path);
                    }
                    std::process::exit(0);
                })));
            }
            ctx.request_repaint();
        }
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        #[cfg(target_os = "linux")]
        if self.prefs_only_frame(ui) {
            return;
        }
        if self.checker.is_none() {
            let a = Color32::from_gray(44);
            let b = Color32::from_gray(52);
            let img = ColorImage::new([2, 2], vec![a, b, b, a]);
            self.checker = Some(ctx.load_texture("checker", img, TextureOptions::NEAREST_REPEAT));
        }
        if let Some(sz) = self.pending_size.take() {
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(sz));
        }
        // Linux: note the size of the normal window, so it opens again in that size.
        if cfg!(target_os = "linux") && self.select.is_none() {
            // Wayland gives no window position, so `inner_rect` is empty there; the size of
            // the window's area is known all the same.
            let (r, full, min) = ctx.input(|i| (i.viewport_rect(), i.viewport().fullscreen, i.viewport().minimized));
            if full != Some(true) && min != Some(true) {
                let sz = [r.width().round(), r.height().round()];
                if r.width() >= 100.0 && self.settings.window_size != Some(sz) {
                    self.settings.window_size = Some(sz);
                }
            }
        }
        if let Some(p) = self.pending_pos.take() {
            ctx.send_viewport_cmd(ViewportCommand::OuterPosition(p));
        }

        // Pictures dropped onto the window open in the editor.
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
        if let Some(p) = dropped.first() {
            self.open_file(&ctx, p);
        }

        let mut action = None;

        // Area selection before a recording.
        let selecting_in_root = cfg!(target_os = "linux") && self.select.is_some();
        let mut sel_out = self.select_overlay(&ctx, if selecting_in_root { Some(&mut *ui) } else { None });
        if test_env("LS_TEST_AUTOREC") && self.select.as_ref().is_some_and(|s| s.countdown.is_none()) {
            let t = *self.test_select_since.get_or_insert_with(Instant::now);
            if t.elapsed() > Duration::from_millis(1500) {
                if let Some(r) = self.select.as_ref().and_then(|s| s.rect) {
                    sel_out = SelectOut::Record(r);
                }
            } else {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
        // Linux: without a red frame during the recording (GNOME), a countdown shows the area first.
        #[cfg(target_os = "linux")]
        let framed = platform::frame_supported();
        #[cfg(not(target_os = "linux"))]
        let framed = true;
        match sel_out {
            SelectOut::None => {}
            SelectOut::Shot(r) => {
                let sel = self.select.take();
                platform::end_screen_session();
                let target = sel.as_ref().and_then(|s| s.target.clone());
                // A full-screen capture is not an area to repeat with "Capture Previous Area".
                let chosen = !sel.as_ref().is_some_and(|s| matches!(s.instant, Some((_, None))));
                let path = sel.as_ref().and_then(|s| capture::crop_screens(&s.screens, r));
                // `screencapture` plays its own sound for the captures it takes itself.
                if path.is_some() && self.settings.sound {
                    platform::play_capture_sound();
                }
                self.visible = false;
                match path {
                    Some(p) => {
                        if chosen {
                            self.last_area = Some(r);
                            platform::set_last_area(r);
                        }
                        self.handle_capture(&ctx, p, target);
                    }
                    None => {
                        self.show_window(&ctx);
                        self.toast("Could not read the screen.", None);
                    }
                }
                if selecting_in_root {
                    if self.visible {
                        // The editor (or the window that was open) takes over the main window.
                        ctx.send_viewport_cmd(ViewportCommand::Fullscreen(false));
                        ctx.send_viewport_cmd(ViewportCommand::Decorations(true));
                        #[cfg(target_os = "linux")]
                        let size = self.open_size();
                        #[cfg(not(target_os = "linux"))]
                        let size = self.window_size();
                        ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
                    } else {
                        // Copy only: the selection window just goes away.
                        self.visible = true;
                        self.hide_window(&ctx);
                    }
                }
            }
            SelectOut::Cancel => {
                self.select = None;
                // macOS: Esc before the picture was there; it is not wanted any more.
                if let Some(job) = self.job.take() {
                    job.cancelled.store(true, std::sync::atomic::Ordering::SeqCst);
                }
                platform::end_screen_session();
                if selecting_in_root {
                    // A cancelled capture leaves no window: the selection window just goes away.
                    self.visible = true;
                    self.hide_window(&ctx);
                }
            }
            SelectOut::Record(r) if selecting_in_root && !framed && self.select.as_ref().is_some_and(|s| s.countdown.is_none()) => {
                // Show the area a moment with a countdown, then record.
                if let Some(s) = &mut self.select {
                    s.rect = Some(r);
                    s.drag = None;
                    s.settings_open = false;
                    s.countdown = Some(Instant::now());
                }
                ctx.request_repaint();
            }
            SelectOut::Record(r) => {
                let (screen, bounds, scale) = self
                    .select
                    .as_ref()
                    .and_then(|s| {
                        let list: Vec<platform::Display> =
                            s.screens.iter().map(|x| platform::Display { bounds: x.bounds, scale: x.scale }).collect();
                        if list.is_empty() {
                            return None;
                        }
                        let i = platform::display_index_at(&list, r.center());
                        Some((i, list[i].bounds, list[i].scale))
                    })
                    .unwrap_or((0, Rect::from_min_size(Pos2::ZERO, self.monitor), self.native_ppp));
                self.select = None;
                self.start_recording(&ctx, r, screen, bounds, scale);
                if selecting_in_root {
                    if self.recording.is_some() {
                        // The main window was the selection; it goes away for the recording.
                        self.visible = true;
                        self.hide_window(&ctx);
                    } else {
                        ctx.send_viewport_cmd(ViewportCommand::Fullscreen(false));
                        ctx.send_viewport_cmd(ViewportCommand::Decorations(true));
                        ctx.send_viewport_cmd(ViewportCommand::InnerSize(self.window_size()));
                    }
                }
            }
        }
        if selecting_in_root {
            // The main window shows only the selection this frame.
            return;
        }
        // Linux: frame and control bar are layer surfaces (see platform::RecFrame); GNOME
        // has neither, there the recording is controlled from the tray and the shortcuts.
        if !cfg!(target_os = "linux") && let Some(a) = self.recording_overlays(&ctx) {
            action = Some(a);
        }

        // Settings, in a window of their own.
        self.prefs.can_configure_keys = self.hotkeys.as_ref().is_some_and(|h| h.can_configure());
        let out = self.prefs.show(&ctx, &mut self.settings);
        self.handle_prefs_out(&ctx, out);

        egui::Panel::top("top")
            .exact_size(40.0)
            .frame(Frame::new().fill(BAR).stroke(Stroke::new(1.0, EDGE)))
            .show(ui, |ui| {
                if let Some(a) = self.top_bar(ui) {
                    action = Some(a);
                }
            });
        egui::Panel::bottom("bottom")
            .exact_size(40.0)
            .frame(Frame::new().fill(BAR).stroke(Stroke::new(1.0, EDGE)))
            .show(ui, |ui| {
                if self.video.is_some() {
                    self.video_bar(ui);
                } else {
                    self.bottom_bar(ui);
                }
            });
        egui::CentralPanel::no_frame().show(ui, |ui| {
            let font = self.font.clone();
            let style = self.style();
            let tool = self.tool;
            if self.video.is_some() {
                self.video_view(ui);
            } else if let (Some(ed), Some(ch)) = (&mut self.ed, &self.checker) {
                ed.canvas(ui, tool, style, &font, ch);
            } else if let Some(a) = self.home(ui) {
                action = Some(a);
            }

            // Toast
            if let Some(t) = &self.toast {
                let rect = ui.max_rect();
                let galley = ui.painter().layout_no_wrap(t.text.clone(), FontId::proportional(14.0), Color32::WHITE);
                let mut size = galley.size() + vec2(28.0, 16.0);
                let btn_w = t.action.as_ref().map_or(0.0, |a| {
                    let w = ui.painter().layout_no_wrap(a.label.into(), FontId::proportional(13.0), Color32::WHITE).size().x;
                    w + 20.0
                });
                // The button sits under the text.
                if btn_w > 0.0 {
                    size.y += 24.0 + 8.0;
                }
                let r = Rect::from_center_size(pos2(rect.center().x, rect.max.y - 34.0 - (size.y - 36.0).max(0.0) / 2.0), size);
                ui.painter().rect_filled(r, 8.0, Color32::from_black_alpha(215));
                ui.painter().galley(r.min + vec2(14.0, 8.0), galley, Color32::WHITE);
                if let Some(p) = t.path.clone()
                    && ui.interact(r, Id::new("toast"), Sense::click()).on_hover_text(format!("Show in {}", platform::FILE_MANAGER)).clicked()
                {
                    platform::reveal_path(&p);
                }
                if let Some(a) = &t.action {
                    let b = Rect::from_center_size(pos2(r.center().x, r.max.y - 8.0 - 12.0), vec2(btn_w, 24.0));
                    if ui.put(b, egui::Button::new(a.label)).clicked() {
                        (a.run)();
                    }
                }
            }
        });

        self.permission_prompt(&ctx);

        // ⌘W closes the picture editor (or video) and puts the window away, like ⌘W in other Mac apps.
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::W)) {
            if let Some(v) = &mut self.video {
                v.stop();
            }
            if let Some(e) = &mut self.ed {
                e.commit_text();
            }
            self.ed = None;
            self.video = None;
            self.recent_dirty = true;
            self.close_after_save = true;
        }
        self.editor_shortcuts(&ctx);
        if self.close_after_save {
            self.close_after_save = false;
            #[cfg(target_os = "linux")]
            let can_hide = self.tray.is_some() || self.no_tray;
            #[cfg(not(target_os = "linux"))]
            let can_hide = self.tray.is_some();
            if can_hide {
                self.hide_window(&ctx);
            }
        }
        self.resize_window(&ctx);

        if let Some(a) = action {
            self.run_action(&ctx, a);
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(r) = self.recording.take() {
            r.rec.cancel();
        }
        self.settings.save();
    }
}
