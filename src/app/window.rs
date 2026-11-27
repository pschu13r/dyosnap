//! Showing and hiding the main window, its size, and what Preferences asked for.

use super::*;

impl DyoSnap {
    pub(super) fn show_window(&mut self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(ViewportCommand::Focus);
        self.visible = true;
        // Wayland ignores `Focus`; there the window needs the desktop's token.
        self.raise = true;
    }

    /// An extra window lost a frame after drawing it, so pictures and letters sent in that
    /// frame never reached the graphics card (a black record area with missing letters).
    /// Send them again: the screen pictures, and all letters by rebuilding the fonts.
    /// A few times at most, so a window that keeps failing can't keep the app busy.
    pub(super) fn resend_lost_uploads(&mut self, ctx: &egui::Context) {
        if !crate::take_uploads_lost() {
            return;
        }
        let tries = match &mut self.select {
            Some(sel) => {
                sel.resent += 1;
                sel.resent
            }
            None => {
                self.resent += 1;
                self.resent
            }
        };
        log::warn!(target: "dyosnap", "an extra window lost a frame; resending (try {tries})");
        if (self.select.is_some() && tries > 3) || tries > 10 {
            return;
        }
        if let Some(sel) = &mut self.select {
            for scr in &mut sel.screens {
                scr.tex = None;
            }
        }
        // Rebuild all fonts, so every letter is sent again. set_fonts would skip it (the
        // fonts did not change); setting the glyph rasterizers always rebuilds. The list
        // stays as it was: empty, eframe adds none on desktop.
        ctx.set_glyph_rasterizers(Vec::new());
        // The main window is hidden; let it paint so the new letters reach the card.
        ctx.send_viewport_cmd(ViewportCommand::RequestPaintWhileHidden);
    }

    /// Linux: GNOME asks once whether DyoSnap may take screenshots, and only while
    /// DyoSnap's window is in front. This note lets the person do that.
    pub(super) fn permission_prompt(&mut self, ctx: &egui::Context) {
        if !self.perm_prompt || self.select.is_some() {
            return;
        }
        let mut later = false;
        // At the top, so it stays off the home card's no-tray hint further down.
        let id = Id::new("perm_prompt");
        let area = egui::Modal::default_area(id).anchor(Align2::CENTER_TOP, vec2(0.0, 12.0));
        egui::Modal::new(id).area(area).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(egui::RichText::new("Allow screenshots").size(16.0).strong().color(TEXT));
            ui.add_space(6.0);
            ui.label(
                "Full-screen and timed captures and screen recordings need your permission once. \
                 Your desktop asks for it only while this window is in front.",
            );
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let ask = ui.add_enabled(
                    !self.perm_asking,
                    egui::Button::new(egui::RichText::new("Allow…").color(Color32::WHITE)).fill(BLUE),
                );
                if ask.clicked() {
                    self.perm_asking = true;
                    let tx = self._tx.clone();
                    let ctx = ctx.clone();
                    std::thread::spawn(move || {
                        let ok = platform::ask_screenshot_permission() && platform::screenshot_allowed();
                        let _ = tx.send(AppEvent::Permission(ok));
                        ctx.request_repaint();
                    });
                }
                if ui.button("Later").clicked() {
                    later = true;
                }
            });
        });
        if later {
            self.perm_prompt = false;
        }
    }

    pub(super) fn hide_window(&mut self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        // Wayland can't hide a window: close it. main.rs keeps the app running and opens
        // the window again when it is needed (see `wants_window`), in the size it had.
        #[cfg(target_os = "linux")]
        if self.visible {
            ctx.send_viewport_cmd(ViewportCommand::Close);
            self.settings.save();
        }
        self.visible = false;
    }

    /// What the Preferences window asked for.
    pub(super) fn handle_prefs_out(&mut self, ctx: &egui::Context, out: crate::prefs::PrefsOut) {
        if out.configure_keys {
            if let Some(h) = &self.hotkeys {
                h.configure();
            }
        }
        if out.capture_started {
            // Let the next key press reach the settings window instead of starting a capture.
            self.unregister_hotkeys();
        }
        if out.changed {
            self.settings.save();
            self.tray_menu_dirty = true;
        }
        if out.save_dir_changed {
            self.recent_dirty = true;
        }
        if test_env("LS_TEST_DIALOG") && self.prefs.open && !self.test_dialog_done {
            self.test_dialog_done = true;
            let d = rfd::AsyncFileDialog::new().set_directory(&self.settings.save_dir);
            self.spawn_dialog(ctx, DialogKind::SaveDir, d.pick_folder());
        }
        if out.pick_save_dir {
            let d = rfd::AsyncFileDialog::new().set_directory(&self.settings.save_dir);
            self.spawn_dialog(ctx, DialogKind::SaveDir, d.pick_folder());
        }
        if out.pick_extra_dir {
            let d = rfd::AsyncFileDialog::new().set_directory(&self.settings.save_dir);
            self.spawn_dialog(ctx, DialogKind::ExtraDir, d.pick_folder());
        }
        if out.pick_editor_app {
            let d = rfd::AsyncFileDialog::new()
                .set_directory(platform::EDITOR_DIR)
                .add_filter("Application", &[platform::EDITOR_EXT]);
            self.spawn_dialog(ctx, DialogKind::EditorApp, d.pick_file());
        }
        if (out.hotkeys_changed || out.capture_ended) && self.prefs.capturing.is_none() {
            self.register_hotkeys();
        }
    }

    /// The size of the main window as a normal window: the last one it had, or the default.
    /// Never below the window's minimum (900x440): Wayland does not enforce it on a new
    /// window, and a smaller size may have been saved from the Preferences window.
    pub fn window_size(&self) -> Vec2 {
        self.settings.window_size.map(|[w, h]| vec2(w.max(900.0), h.max(440.0))).unwrap_or(vec2(980.0, 620.0))
    }

    /// Linux: take the screen size and scale from the system. While the window is closed,
    /// egui knows neither, and the defaults are the Mac's (a Retina screen).
    pub(super) fn refresh_screen(&mut self) {
        if cfg!(target_os = "linux") {
            if let Some(d) = platform::displays().first() {
                if d.bounds.width() > 100.0 {
                    self.monitor = d.bounds.size();
                    self.native_ppp = d.scale.max(1.0);
                }
            }
        }
    }

    /// A `screencapture` ended or the selection overlay closed: the arrow comes back at once
    /// and again a moment later, once the overlay window is gone. Not while a new selection
    /// is open: it sets its own cursor.
    pub(super) fn reset_cursor_after_capture(&mut self, ctx: &egui::Context) {
        let open = self.select.is_some();
        let closed = std::mem::replace(&mut self.select_was_open, open) && !open;
        if capture::CURSOR_RESET.swap(false, std::sync::atomic::Ordering::SeqCst) || closed {
            // At once, so no crosshair flashes, and once more when the overlay is surely gone.
            if !open {
                platform::reset_cursor();
            }
            self.cursor_reset_at = Some(Instant::now() + Duration::from_millis(150));
        }
        if let Some(t) = self.cursor_reset_at {
            let left = t.saturating_duration_since(Instant::now());
            if !left.is_zero() {
                ctx.request_repaint_after(left);
            } else {
                self.cursor_reset_at = None;
                if !open {
                    platform::reset_cursor();
                }
            }
        }
    }
}
