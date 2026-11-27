//! Linux only: the window comes and goes (no tray host, Wayland), so the state outlives it.

use super::*;

impl DyoSnap {
    /// Linux: there is no tray host (see `no_tray`).
    #[cfg(target_os = "linux")]
    pub fn no_tray(&self) -> bool {
        self.no_tray
    }

    /// Linux: whether the window has to be open (it is shown, or the record area is being chosen).
    #[cfg(target_os = "linux")]
    pub fn wants_window(&self) -> bool {
        self.visible || self.select.is_some() || self.prefs.open
    }

    /// Linux: the window is wanted only for Preferences (opened from the tray, no picture,
    /// video or record area). Then the window itself shows Preferences, with no second one.
    #[cfg(target_os = "linux")]
    pub fn prefs_only(&self) -> bool {
        self.prefs.open && !self.visible && self.select.is_none()
    }

    /// Linux: called right before the window is made. In Preferences-only mode it returns
    /// the size and title the window gets.
    #[cfg(target_os = "linux")]
    pub fn window_opening(&mut self) -> Option<(Vec2, String)> {
        self.root_prefs = self.prefs_only();
        self.root_closing = false;
        if self.root_prefs {
            // There is no main window whose focus there is to wait for.
            self.prefs.skip_wait();
            Some((self.prefs.window_size(), self.prefs.window_title()))
        } else {
            None
        }
    }

    /// Linux: the frame of a window that shows Preferences only. Returns true when the
    /// frame is done (the caller draws nothing else).
    #[cfg(target_os = "linux")]
    pub(super) fn prefs_only_frame(&mut self, ui: &mut Ui) -> bool {
        if !self.root_prefs {
            return false;
        }
        let ctx = ui.ctx().clone();
        if !self.prefs_only() {
            // Something needs the real main window (a picture, a video, the record area).
            // Close this one; main.rs makes it again in the main size, and Preferences
            // then goes on as its own window.
            if !self.root_closing {
                self.root_closing = true;
                self.prefs.refit();
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
            return true;
        }
        self.prefs.skip_wait();
        self.prefs.can_configure_keys = self.hotkeys.as_ref().is_some_and(|h| h.can_configure());
        let mut out = crate::prefs::PrefsOut::default();
        let mut close = false;
        egui::CentralPanel::no_frame().show(ui, |ui| {
            close = self.prefs.draw(ui, &mut self.settings, &mut out);
        });
        if close {
            // Close the window too (for Ctrl+W; a close request is closing it already).
            self.prefs.close(&mut out);
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
        self.handle_prefs_out(&ctx, out);
        true
    }

    /// The size the window opens in: the one a just opened picture or video asks for, or the
    /// last one. On Linux the window is made anew each time it shows, and Wayland ignores a
    /// resize sent before the window is on screen, so the size has to be right from the start.
    #[cfg(target_os = "linux")]
    pub fn open_size(&self) -> Vec2 {
        self.pending_size.unwrap_or_else(|| self.window_size())
    }

    /// Linux: the record area is being chosen; the window opens full screen for it.
    #[cfg(target_os = "linux")]
    pub fn selecting(&self) -> bool {
        self.select.is_some()
    }

    /// Linux: the work of a frame while no window is open: tray and shortcut events,
    /// finished captures, the recording timer.
    #[cfg(target_os = "linux")]
    pub fn tick_hidden(&mut self, ctx: &egui::Context) {
        self.handle_events(ctx);
        self.test_open_prefs(ctx);
    }

    /// Linux: the window was opened again with a new graphics context, so every picture
    /// and letter has to be sent to the graphics card again.
    #[cfg(target_os = "linux")]
    pub fn reattach(&mut self, ctx: &egui::Context) {
        self.checker = None;
        if let Some(ed) = &mut self.ed {
            ed.forget_texture();
        }
        if let Some(v) = &mut self.video {
            v.tex = None;
            let pos = v.pos;
            v.seek(ctx, pos);
        }
        if let Some(sel) = &mut self.select {
            for scr in &mut sel.screens {
                scr.tex = None;
            }
        }
        // See resend_lost_uploads: this always rebuilds the font texture.
        ctx.set_glyph_rasterizers(Vec::new());
    }

    /// Linux: the area is chosen in the main window, full screen. The compositor opens it
    /// where the mouse or focus is; Wayland tells no app where the mouse is, but the window
    /// learns its screen. Show that screen's frozen picture there. If that screen was not
    /// shared, move the window to the first shared one. Once per selection.
    #[cfg(target_os = "linux")]
    pub(super) fn place_selection(&mut self, frame: &eframe::Frame) {
        let Some(sel) = &mut self.select else { return };
        if sel.placed {
            return;
        }
        let Some(win) = frame.winit_window() else { return };
        // Until the window is on a screen, the compositor has not chosen one yet.
        let Some(current) = win.current_monitor() else { return };
        sel.placed = true;
        let here = current.name().and_then(|n| platform::output_bounds(&n));
        if let Some(i) = here.and_then(|b| sel.screens.iter().position(|s| s.bounds.min == b.min)) {
            log::info!(target: "dyosnap", "selection on {:?} (screen {i})", current.name());
            sel.shown = i;
            if !sel.shot {
                // The record area starts on this screen too.
                let b = sel.screens[i].bounds;
                sel.rect = Some(Rect::from_center_size(b.center(), b.size() * 0.75));
            }
            return;
        }
        let Some(first) = sel.screens.first() else { return };
        let Some(target) = platform::output_name_at(first.bounds.min) else { return };
        if let Some(m) = win.available_monitors().find(|m| m.name().as_deref() == Some(target.as_str())) {
            log::info!(target: "dyosnap", "selection opened on {:?}, not shared; moving it to {target}", current.name());
            // KWin keeps a window that is already full screen where it is: leave full screen
            // first, then ask for it on the shared screen.
            win.set_fullscreen(None);
            win.set_fullscreen(Some(winit::window::Fullscreen::Borderless(Some(m))));
        }
    }
}

/// Linux: the app as eframe sees it while a window is open. The state outlives the
/// window (see `run_linux` in main.rs).
#[cfg(target_os = "linux")]
pub struct Shared(pub std::rc::Rc<std::cell::RefCell<DyoSnap>>);

#[cfg(target_os = "linux")]
impl eframe::App for Shared {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.0.borrow_mut().logic(ctx, frame);
    }
    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        self.0.borrow_mut().ui(ui, frame);
    }
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        self.0.borrow().clear_color(visuals)
    }
    // No on_exit: closing the window only hides DyoSnap. Quit saves and exits by itself.
}
