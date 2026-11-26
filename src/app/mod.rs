//! The main window, the menu-bar icon, global shortcuts and the capture/record flow.
//!
//! This file holds the app's state and the per-frame loop. The work is split over:
//! - `events`: menu-bar items, global shortcuts and the actions they start
//! - `capture`: screenshots, choosing the record area, the recording overlays
//! - `files`: opening, saving and copying pictures and videos
//! - `views`: the bars, the start page, the video view and shared widgets

use crate::platform::{self, CaptureKind};
use crate::doc::Style;
use crate::editor::{Editor, ResizeDlg, Tool};
use crate::icons::{self, Icon};
use crate::platform::Devices;
use crate::record::{self, RecordOptions, Recorder};
use crate::settings::{AfterCapture, ImageFormat, Quality, Settings};
use crate::video::VideoView;
use ab_glyph::FontVec;
use egui::{
    Align, Align2, Color32, ColorImage, CornerRadius, FontId, Frame, Id, Key, Layout, Margin, Modifiers,
    Pos2, Rect, Sense, Stroke, StrokeKind, TextureHandle, TextureOptions, Ui, Vec2, ViewportBuilder,
    ViewportCommand, ViewportId, WindowLevel, pos2, vec2,
};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

mod capture;
mod events;
mod files;
mod views;

use capture::*;
use events::*;
use files::*;
use views::*;

// ---------------------------------------------------------------- colours

const BAR: Color32 = Color32::from_rgb(44, 44, 46);
const BTN: Color32 = Color32::from_rgb(58, 58, 60);
const BTN_HOVER: Color32 = Color32::from_rgb(72, 72, 76);
const EDGE: Color32 = Color32::from_rgb(24, 24, 26);
const BLUE: Color32 = Color32::from_rgb(40, 76, 186);
const BLUE_HOVER: Color32 = Color32::from_rgb(54, 92, 206);
const RED: Color32 = Color32::from_rgb(226, 38, 38);
const TEXT: Color32 = Color32::from_rgb(235, 235, 240);
const DIM_TEXT: Color32 = Color32::from_rgb(150, 150, 156);
/// Height of every control in the top bar (fields, buttons, button groups).
const BAR_ITEM_H: f32 = TOOL_H;
/// Height of every control in the bottom tool bar.
const TOOL_H: f32 = 26.0;
/// Width of every icon button, top and bottom.
const TOOL_W: f32 = 36.0;
/// Text size inside the top bar, kept small so the buttons look as tall as the tool buttons.
const BAR_FONT: f32 = 13.0;

const PALETTE: [Color32; 10] = [
    Color32::from_rgb(235, 51, 36),
    Color32::from_rgb(255, 149, 0),
    Color32::from_rgb(255, 214, 10),
    Color32::from_rgb(52, 199, 89),
    Color32::from_rgb(0, 122, 255),
    Color32::from_rgb(175, 82, 222),
    Color32::from_rgb(255, 55, 140),
    Color32::from_rgb(255, 255, 255),
    Color32::from_rgb(142, 142, 147),
    Color32::from_rgb(0, 0, 0),
];

// ---------------------------------------------------------------- actions

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Capture(CaptureKind),
    /// Capture an area into the extra folder with id N: no editor, saved and copied.
    CaptureTo(u32),
    Record,
    Show,
    OpenImage,
    OpenClipboard,
    OpenLatest,
    OpenFolder,
    Settings,
    Quit,
    FinishRec,
    CancelRec,
    PauseRec,
}

enum AppEvent {
    /// Whether the person allowed screenshots (see `ask_permission`).
    Permission(bool),
    Menu(String),
    Hotkey(String),
    Dialog(DialogKind, Option<PathBuf>),
}

/// File dialogs. They open as a sheet without blocking; a blocking dialog would make
/// macOS deliver events while one is still being handled, which stops the app.
#[derive(Clone, Copy, Debug)]
pub enum DialogKind {
    OpenImage,
    SaveAs,
    SaveDir,
    ExtraDir,
    EditorApp,
}

struct Job {
    /// The action that started it, to tell a repeat from a switch (see `capture_switch`).
    action: Action,
    /// Set when the job was cancelled: its thread takes no picture any more.
    cancelled: Arc<AtomicBool>,
    kind: CaptureKind,
    /// The captured picture(s): one per screen when preparing a recording; empty if cancelled.
    rx: Receiver<Vec<PathBuf>>,
    for_record: bool,
    /// Linux: the picture is for choosing a screenshot area in DyoSnap (see `start_capture`).
    select_shot: bool,
    /// An extra folder to save the picture into (see `Action::CaptureTo`).
    target: Option<PathBuf>,
    /// Linux "Capture Window": the windows, listed before the picture was taken.
    windows: Arc<Mutex<Vec<Rect>>>,
}

struct Toast {
    text: String,
    until: Instant,
    path: Option<PathBuf>,
    /// A button in the toast.
    action: Option<ToastAction>,
}

struct ToastAction {
    label: &'static str,
    run: fn(),
}

struct Screenshot {
    path: PathBuf,
    frames: u32,
    sent: bool,
}

pub struct DyoSnap {
    settings: Settings,
    font: Arc<FontVec>,
    ed: Option<Editor>,
    video: Option<VideoView>,
    tool: Tool,
    checker: Option<TextureHandle>,
    job: Option<Job>,
    select: Option<Select>,
    recording: Option<RecState>,
    finishing: Option<Receiver<Result<PathBuf, String>>>,
    rec_started_name: String,
    video_saving: Option<Receiver<Result<PathBuf, String>>>,
    copy_after_save: bool,
    ffmpeg: Option<PathBuf>,
    devices: Arc<Mutex<Option<Devices>>>,
    tray: Option<platform::Tray>,
    tray_recording: bool,
    events: Receiver<AppEvent>,
    _tx: Sender<AppEvent>,
    hotkeys: Option<platform::Hotkeys>,
    toast: Option<Toast>,
    prefs: crate::prefs::Prefs,
    background: bool,
    /// Linux: the desktop has no tray host, so the main window stays and shows a hint.
    #[cfg(target_os = "linux")]
    no_tray: bool,
    test_prefs_done: bool,
    /// Last editor version put on the clipboard, and when the next copy is due.
    clip_version: u64,
    clip_due: Option<Instant>,
    clip_gen: Arc<std::sync::atomic::AtomicU64>,
    test_dialog_done: bool,
    test_select_since: Option<Instant>,
    egui_ctx: egui::Context,
    shot: Option<Screenshot>,
    visible: bool,
    /// Linux: the open main window was made only to show Preferences (see `prefs_only`).
    #[cfg(target_os = "linux")]
    root_prefs: bool,
    /// Linux: that window was asked to close, so the real main window can take its place.
    #[cfg(target_os = "linux")]
    root_closing: bool,
    monitor: Vec2,
    native_ppp: f32,
    recent: Vec<PathBuf>,
    recent_dirty: bool,
    tray_menu_dirty: bool,
    close_after_save: bool,
    pending_size: Option<Vec2>,
    pending_pos: Option<Pos2>,
    /// How often lost uploads were resent outside of choosing a record area.
    resent: u32,
    /// Show the note that asks to allow screenshots (Linux, see `permission_prompt`).
    perm_prompt: bool,
    perm_asking: bool,
    /// Linux: the area of the last area screenshot, for "Capture Previous Area" (global points).
    /// macOS keeps it in `platform`.
    last_area: Option<Rect>,
    /// Linux: "Open from Clipboard" waits for the window's focus since then (see `logic`).
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    clipboard_wait: Option<Instant>,
    /// Linux: put the open main window in front in the next frame (see `logic`).
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    raise: bool,
    /// macOS: when to show the arrow again after a crosshair (see `platform::reset_cursor`),
    /// and whether the selection overlay was open in the last frame.
    cursor_reset_at: Option<Instant>,
    select_was_open: bool,
}

impl DyoSnap {
    pub fn new(ctx: &egui::Context, screenshot: Option<PathBuf>, open: Option<PathBuf>, quiet: bool, background: bool) -> Self {
        let ctx = ctx.clone();
        let (font, font_bytes) = crate::doc::load_font();
        setup_style(&ctx, font_bytes);
        let settings = Settings::load();
        let (tx, rx) = channel();

        let mut app = Self {
            tool: Tool::Arrow,
            settings,
            font: Arc::new(font),
            ed: None,
            video: None,
            checker: None,
            job: None,
            select: None,
            recording: None,
            finishing: None,
            rec_started_name: String::new(),
            video_saving: None,
            copy_after_save: false,
            ffmpeg: record::find_ffmpeg(),
            devices: Arc::new(Mutex::new(None)),
            tray: None,
            tray_recording: false,
            events: rx,
            _tx: tx.clone(),
            hotkeys: None,
            toast: None,
            prefs: Default::default(),
            background,
            #[cfg(target_os = "linux")]
            no_tray: false,
            test_prefs_done: false,
            clip_version: 0,
            clip_due: None,
            clip_gen: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            test_dialog_done: false,
            test_select_since: None,
            egui_ctx: ctx.clone(),
            shot: screenshot.map(|path| Screenshot { path, frames: 0, sent: false }),
            visible: !background,
            #[cfg(target_os = "linux")]
            root_prefs: false,
            #[cfg(target_os = "linux")]
            root_closing: false,
            monitor: vec2(1440.0, 900.0),
            native_ppp: 2.0,
            recent: Vec::new(),
            recent_dirty: true,
            tray_menu_dirty: false,
            close_after_save: false,
            pending_size: None,
            pending_pos: None,
            resent: 0,
            perm_prompt: false,
            last_area: None,
            clipboard_wait: None,
            raise: false,
            perm_asking: false,
            cursor_reset_at: None,
            select_was_open: false,
        };
        app.refresh_screen();
        if !quiet && !platform::screenshot_allowed() {
            app.perm_prompt = true;
        }

        // The menu-bar icon and global shortcuts are skipped for a quick check picture.
        if !quiet || test_env("LS_TEST_TRAY") {
            let sink = {
                let tx = tx.clone();
                let ctx = ctx.clone();
                std::sync::Arc::new(move |id: String| {
                    let _ = tx.send(AppEvent::Menu(id));
                    ctx.request_repaint();
                })
            };
            app.tray = platform::Tray::new(build_menu(false, &app.settings), sink);
            // No tray host (GNOME without the AppIndicator extension): the app would be
            // out of reach when it starts hidden, so keep the window and hint there.
            #[cfg(target_os = "linux")]
            if app.tray.is_none() {
                app.no_tray = true;
                app.visible = true;
            }
            if app.settings.hotkeys {
                app.setup_hotkeys(&ctx, tx);
            }
        }
        if let Some(p) = open {
            app.open_file(&ctx, &p);
        }
        // For check pictures: LS_PREFS=general|screenshots|video|about draws the settings inside the main window.
        if let (true, Ok(tab)) = (app.shot.is_some() && cfg!(debug_assertions), std::env::var("LS_PREFS")) {
            ctx.set_embed_viewports(true);
            app.pending_size = Some(vec2(900.0, 840.0));
            app.prefs.open();
            app.prefs.tab = match tab.as_str() {
                "screenshots" => crate::prefs::Tab::Screenshots,
                "video" => crate::prefs::Tab::Video,
                "about" => crate::prefs::Tab::About,
                _ => crate::prefs::Tab::General,
            };
        }
        let _ = app.background;
        app
    }

    /// Waits for a sheet dialog on a helper thread and reports the chosen path.
    fn spawn_dialog(
        &mut self,
        ctx: &egui::Context,
        kind: DialogKind,
        fut: impl std::future::Future<Output = Option<rfd::FileHandle>> + Send + 'static,
    ) {
        let tx = self._tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let path = pollster::block_on(fut).map(|h| h.path().to_path_buf());
            let _ = tx.send(AppEvent::Dialog(kind, path));
            ctx.request_repaint();
        });
    }

    fn style(&self) -> Style {
        let ppp = self.ed.as_ref().map(|e| e.ppp).unwrap_or(1.0);
        let [r, g, b, a] = self.settings.color;
        Style { color: Color32::from_rgba_unmultiplied(r, g, b, a), width: self.settings.width * ppp }
    }

    fn toast(&mut self, text: impl Into<String>, path: Option<PathBuf>) {
        self.toast = Some(Toast { text: text.into(), until: Instant::now() + Duration::from_secs(4), path, action: None });
    }

    /// A toast with a button. It stays longer, so there is time to reach the button.
    fn toast_action(&mut self, text: impl Into<String>, label: &'static str, run: fn()) {
        self.toast = Some(Toast {
            text: text.into(),
            until: Instant::now() + Duration::from_secs(8),
            path: None,
            action: Some(ToastAction { label, run }),
        });
    }

    /// The screen could not be read. On macOS the toast opens the Screen Recording settings.
    fn toast_no_screen(&mut self) {
        let text = format!("Could not read the screen. {}", platform::SCREEN_PERMISSION_HINT);
        if cfg!(target_os = "macos") {
            self.toast_action(text, "Open System Settings", || platform::open_privacy(platform::Pane::ScreenRecording));
        } else {
            self.toast(text, None);
        }
    }

    fn show_window(&mut self, ctx: &egui::Context) {
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
    fn resend_lost_uploads(&mut self, ctx: &egui::Context) {
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
    fn permission_prompt(&mut self, ctx: &egui::Context) {
        if !self.perm_prompt || self.select.is_some() {
            return;
        }
        let mut later = false;
        egui::Modal::new(Id::new("perm_prompt")).show(ctx, |ui| {
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

    fn hide_window(&mut self, ctx: &egui::Context) {
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
    fn prefs_only_frame(&mut self, ui: &mut Ui) -> bool {
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

    /// What the Preferences window asked for.
    fn handle_prefs_out(&mut self, ctx: &egui::Context, out: crate::prefs::PrefsOut) {
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
    pub fn window_size(&self) -> Vec2 {
        self.settings.window_size.map(|[w, h]| vec2(w, h)).unwrap_or(vec2(980.0, 620.0))
    }

    /// The size the window opens in: the one a just opened picture or video asks for, or the
    /// last one. On Linux the window is made anew each time it shows, and Wayland ignores a
    /// resize sent before the window is on screen, so the size has to be right from the start.
    #[cfg(target_os = "linux")]
    pub fn open_size(&self) -> Vec2 {
        self.pending_size.unwrap_or_else(|| self.window_size())
    }

    /// Linux: take the screen size and scale from the system. While the window is closed,
    /// egui knows neither, and the defaults are the Mac's (a Retina screen).
    fn refresh_screen(&mut self) {
        if cfg!(target_os = "linux") {
            if let Some(d) = platform::displays().first() {
                if d.bounds.width() > 100.0 {
                    self.monitor = d.bounds.size();
                    self.native_ppp = d.scale.max(1.0);
                }
            }
        }
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
    fn place_selection(&mut self, frame: &eframe::Frame) {
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

    /// A `screencapture` ended or the selection overlay closed: the arrow comes back at once
    /// and again a moment later, once the overlay window is gone. Not while a new selection
    /// is open: it sets its own cursor.
    fn reset_cursor_after_capture(&mut self, ctx: &egui::Context) {
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

    fn busy(&self) -> bool {
        self.job.is_some() || self.select.is_some() || self.recording.is_some() || self.finishing.is_some()
    }
}

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
        // Remember the screen size while the main window is around.
        if let Some(m) = ctx.input(|i| i.viewport().monitor_size) {
            if m.x > 100.0 {
                self.monitor = m;
            }
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
            if let Some(r) = &self.recording {
                if r.rec.elapsed() > Duration::from_secs(3) {
                    eprintln!("TEST rect={:?} screen={:?}", r.rect, r.screen);
                    self.run_action(ctx, Action::FinishRec);
                }
            }
            if let Some(v) = &self.video {
                if !self.test_dialog_done {
                    self.test_dialog_done = true;
                    eprintln!("TEST video={}", v.path.display());
                }
            }
        }
        if test_env("LS_TEST_RECORD") && !self.test_prefs_done {
            self.test_prefs_done = true;
            self.run_action(ctx, Action::Record);
        }
        if test_env("LS_TEST_EDIT") && !self.test_dialog_done {
            if let Some(ed) = &mut self.ed {
                self.test_dialog_done = true;
                let font = self.font.clone();
                ed.apply_resize(&font, 300, 200);
            }
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
                // macOS: `screencapture` plays the shutter itself.
                #[cfg(target_os = "linux")]
                if path.is_some() && self.settings.sound {
                    platform::play_capture_sound();
                }
                self.visible = false;
                match path {
                    Some(p) => {
                        if chosen {
                            self.last_area = Some(r);
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
        if !cfg!(target_os = "linux") {
            if let Some(a) = self.recording_overlays(&ctx) {
                action = Some(a);
            }
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
                if let Some(p) = t.path.clone() {
                    if ui.interact(r, Id::new("toast"), Sense::click()).on_hover_text(format!("Show in {}", platform::FILE_MANAGER)).clicked() {
                        platform::reveal_path(&p);
                    }
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

// ---------------------------------------------------------------- helpers

/// Test hooks (LS_TEST_*) only work in debug builds; a release build ignores them.
fn test_env(name: &str) -> bool {
    cfg!(debug_assertions) && std::env::var_os(name).is_some()
}

fn setup_style(ctx: &egui::Context, font_bytes: Vec<u8>) {
    // ⌘+ / ⌘- / ⌘0 zoom the picture, not the whole interface.
    ctx.options_mut(|o| o.zoom_with_keyboard = false);
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert("annot".into(), Arc::new(egui::FontData::from_owned(font_bytes)));
    fonts.families.insert(egui::FontFamily::Name("annot".into()), vec!["annot".into()]);
    // Small system font with the ⌥ ⌘ ⇧ key symbols, used as a fallback.
    if let Ok(sym) = std::fs::read(platform::SYMBOL_FONT) {
        fonts.font_data.insert("symbols".into(), Arc::new(egui::FontData::from_owned(sym)));
        for fam in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts.families.entry(fam).or_default().push("symbols".into());
        }
    }
    ctx.set_fonts(fonts);

    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|style| {
        style.visuals = egui::Visuals::dark();
        let v = &mut style.visuals;
        v.panel_fill = BAR;
        v.window_fill = Color32::from_rgb(38, 38, 40);
        v.extreme_bg_color = Color32::from_rgb(30, 30, 32);
        v.selection.bg_fill = BLUE;
        v.widgets.inactive.weak_bg_fill = BTN;
        v.widgets.inactive.bg_fill = BTN;
        v.widgets.hovered.weak_bg_fill = BTN_HOVER;
        v.widgets.hovered.bg_fill = BTN_HOVER;
        v.widgets.noninteractive.fg_stroke.color = TEXT;
        v.widgets.inactive.fg_stroke.color = TEXT;
        style.spacing.button_padding = vec2(8.0, 3.0);
        style.spacing.interact_size.y = 22.0;
        for (ts, f) in style.text_styles.iter_mut() {
            // Same text sizes as the settings window.
            f.size = match ts {
                egui::TextStyle::Small => 11.0,
                egui::TextStyle::Heading => 17.0,
                _ => 13.0,
            };
        }
    });
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
