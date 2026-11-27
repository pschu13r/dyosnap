//! The main window, the menu-bar icon, global shortcuts and the capture/record flow.
//!
//! This file holds the app's state and its types. The work is split over:
//! - `frame`: the per-frame loop (`logic` and `ui`)
//! - `window`: showing and hiding the main window, its size, Preferences results
//! - `linux`: Linux only, the window that comes and goes while the state stays
//! - `theme`: colours, sizes and the egui style
//! - `events`: menu-bar items, global shortcuts and the actions they start
//! - `capture`: screenshots and recordings; `capture::select` (the area and its pixels),
//!   `capture::overlay` (the selection overlay, loupe, crosshair), `capture::rec_session`
//!   (a running recording and its bar)
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
mod frame;
#[cfg(target_os = "linux")]
mod linux;
mod theme;
mod views;
mod window;

use capture::*;
use events::*;
use files::*;
use theme::*;
use views::*;

#[cfg(target_os = "linux")]
pub use linux::Shared;

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
    /// When the capture was asked for, to log how long the overlay takes to show up.
    started: Instant,
    /// The screen (index in `displays()`) the first picture belongs to: macOS takes a picture of
    /// the screen under the mouse only, to choose a screenshot area on.
    first_screen: usize,
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
            // Later starts of DyoSnap reach this one like a menu click.
            #[cfg(target_os = "linux")]
            platform::serve_instance(sink.clone());
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
    pub(super) fn spawn_dialog(
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

    pub(super) fn style(&self) -> Style {
        let ppp = self.ed.as_ref().map(|e| e.ppp).unwrap_or(1.0);
        let [r, g, b, a] = self.settings.color;
        Style { color: Color32::from_rgba_unmultiplied(r, g, b, a), width: self.settings.width * ppp }
    }

    pub(super) fn toast(&mut self, text: impl Into<String>, path: Option<PathBuf>) {
        self.toast = Some(Toast { text: text.into(), until: Instant::now() + Duration::from_secs(4), path, action: None });
    }

    /// A toast with a button. It stays longer, so there is time to reach the button.
    pub(super) fn toast_action(&mut self, text: impl Into<String>, label: &'static str, run: fn()) {
        self.toast = Some(Toast {
            text: text.into(),
            until: Instant::now() + Duration::from_secs(8),
            path: None,
            action: Some(ToastAction { label, run }),
        });
    }

    /// The screen could not be read. On macOS the toast opens the Screen Recording settings.
    pub(super) fn toast_no_screen(&mut self) {
        let text = format!("Could not read the screen. {}", platform::SCREEN_PERMISSION_HINT);
        if cfg!(target_os = "macos") {
            self.toast_action(text, "Open System Settings", || platform::open_privacy(platform::Pane::ScreenRecording));
        } else {
            self.toast(text, None);
        }
    }

    pub(super) fn busy(&self) -> bool {
        self.job.is_some() || self.select.is_some() || self.recording.is_some() || self.finishing.is_some()
    }
}

// ---------------------------------------------------------------- helpers

/// Test hooks (LS_TEST_*) only work in debug builds; a release build ignores them.
fn test_env(name: &str) -> bool {
    cfg!(debug_assertions) && std::env::var_os(name).is_some()
}
