//! Taking screenshots and recordings: the capture jobs, choosing the record area and
//! the overlays shown while recording.

use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

mod overlay;
mod rec_session;
mod select;

pub(super) use rec_session::*;
pub(super) use select::*;

/// Counts the capture jobs; see `start_capture_ex`.
static JOB_GEN: AtomicU64 = AtomicU64::new(0);

/// Set by a capture job when it ends, also a cancelled one: its `screencapture` is gone and
/// the arrow can come back (see `platform::reset_cursor`).
pub(super) static CURSOR_RESET: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// What a capture job sends back.
pub(super) enum JobOut {
    /// Image files: one picture, or none if it was cancelled or failed.
    Files(Vec<PathBuf>),
    /// The frozen pictures to choose an area on, one per screen, ready to show.
    Pictures(Vec<ColorImage>),
    /// macOS: ScreenCaptureKit failed. The overlay that waited for the picture goes, and the
    /// capture runs again with `screencapture`.
    Retry,
}

/// Loads the files of `screencapture` as pictures to choose an area on; they go away.
/// Runs in the job thread: decoding 24 MB takes a while.
pub(super) fn decode_screens(files: Vec<PathBuf>) -> Vec<ColorImage> {
    let t0 = Instant::now();
    let pictures: Vec<ColorImage> = files
        .iter()
        .filter_map(|p| {
            let rgba = image::open(p).ok()?.to_rgba8();
            Some(ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw()))
        })
        .collect();
    for p in &files {
        let _ = std::fs::remove_file(p);
    }
    log::info!(target: "dyosnap", "select: {} picture(s) decoded in {} ms", pictures.len(), t0.elapsed().as_millis());
    pictures
}

/// macOS: the pictures of the screens `indices` from ScreenCaptureKit, in this process.
#[cfg(target_os = "macos")]
fn sck_pictures(indices: &[usize]) -> Result<Vec<ColorImage>, String> {
    platform::capture_images(indices)
}

#[cfg(not(target_os = "macos"))]
fn sck_pictures(_indices: &[usize]) -> Result<Vec<ColorImage>, String> {
    Err("not available".to_string())
}

/// Pixels per point of the screen under the mouse, where a capture just ended.
/// Wayland gives no mouse position; there it is the first screen.
pub(super) fn capture_scale() -> Option<f32> {
    let displays = platform::displays();
    let i = platform::mouse_position().map_or(0, |m| platform::display_index_at(&displays, m));
    displays.get(i).map(|d| d.scale.max(1.0))
}

impl DyoSnap {
    pub(super) fn start_capture(&mut self, ctx: &egui::Context, kind: CaptureKind, for_record: bool) {
        let action = if for_record { Action::Record } else { Action::Capture(kind) };
        self.start_capture_ex(ctx, action, kind, for_record, None, true);
    }

    /// A screenshot that goes into the extra folder `n` (see `handle_capture`).
    pub(super) fn start_capture_to(&mut self, ctx: &egui::Context, n: u32, target: Option<PathBuf>) {
        self.start_capture_ex(ctx, Action::CaptureTo(n), CaptureKind::Area, false, target, true);
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

    /// `use_sck`: macOS may take the picture in this process (see `Job::pending`); false after
    /// that failed.
    pub(super) fn start_capture_ex(&mut self, ctx: &egui::Context, action: Action, kind: CaptureKind, for_record: bool, target: Option<PathBuf>, use_sck: bool) {
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
        let displays = platform::displays();
        let first_screen = if mac_shot {
            platform::mouse_position().map_or(0, |m| platform::display_index_at(&displays, m))
        } else {
            0
        };
        // macOS: the picture comes from ScreenCaptureKit, which leaves DyoSnap's own windows out
        // of it: no wait for them to vanish, and the overlay can open while the picture is taken.
        let sck = cfg!(target_os = "macos") && use_sck && (mac_shot || for_record) && !displays.is_empty();
        let indices: Vec<usize> = if mac_shot { vec![first_screen] } else { (0..displays.len()).collect() };
        let screens: Vec<SelScreen> = match sck {
            true => indices
                .iter()
                .filter_map(|&i| displays.get(i))
                .map(|d| SelScreen { bounds: d.bounds, scale: d.scale, img: None, tex: None })
                .collect(),
            false => Vec::new(),
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
            let send = move |r: JobOut| {
                if let Err(e) = tx.send(r) {
                    if let JobOut::Files(files) = e.0 {
                        for p in files {
                            let _ = std::fs::remove_file(p);
                        }
                    }
                    if JOB_GEN.load(Ordering::SeqCst) == generation {
                        platform::end_screen_session();
                    }
                }
                CURSOR_RESET.store(true, Ordering::SeqCst);
                crate::wake(&ctx2);
            };
            // ScreenCaptureKit leaves our windows out of the picture: nothing to wait for.
            if was_visible && !sck {
                std::thread::sleep(Duration::from_millis(350));
            }
            if let (true, CaptureKind::Timed(s)) = (select_shot, kind) {
                std::thread::sleep(Duration::from_secs(s as u64));
            }
            if stop.load(Ordering::SeqCst) {
                return send(JobOut::Files(Vec::new()));
            }
            if sck {
                let t0 = Instant::now();
                return match sck_pictures(&indices) {
                    Ok(pictures) => {
                        log::info!(target: "dyosnap", "select: ScreenCaptureKit picture(s) ready in {} ms", t0.elapsed().as_millis());
                        send(JobOut::Pictures(pictures));
                    }
                    Err(e) => {
                        log::warn!(target: "dyosnap", "select: ScreenCaptureKit failed ({e}); using screencapture");
                        send(JobOut::Retry);
                    }
                };
            }
            // Before the picture: DyoSnap's own window is not open yet.
            #[cfg(target_os = "linux")]
            if select_shot && kind == CaptureKind::Window && let Ok(mut w) = windows2.lock() {
                *w = platform::window_rects();
            }
            let _ = &windows2;
            let t0 = Instant::now();
            let r = if mac_shot {
                let files = platform::capture_screen(first_screen);
                log::info!(target: "dyosnap", "select: screencapture took {} ms", t0.elapsed().as_millis());
                JobOut::Pictures(decode_screens(files))
            } else if for_record || select_shot {
                // A frozen picture of every screen to choose the area on. Area screenshots
                // leave the mouse pointer out, as on macOS; full-screen ones follow the setting.
                let choosing = select_shot && matches!(kind, CaptureKind::Area | CaptureKind::Previous | CaptureKind::Window);
                let files = platform::capture_all_screens(indices.len(), cursor && !choosing);
                log::info!(target: "dyosnap", "select: screencapture took {} ms", t0.elapsed().as_millis());
                JobOut::Pictures(decode_screens(files))
            } else {
                JobOut::Files(platform::run_capture(kind, sound).into_iter().collect())
            };
            send(r);
        });
        // macOS: the overlay opens now, on the live screen, while the picture is taken: the
        // windows and their graphics are made meanwhile. It shows nothing until the picture is
        // there (see `select_overlay`).
        if sck {
            let sel = if for_record {
                Select::for_record(action, screens, started)
            } else {
                let mut sel = Select::new(action, screens, None, started);
                sel.shot = true;
                sel.target = target.clone();
                sel
            };
            self.select = Some(sel);
            ctx.send_viewport_cmd(ViewportCommand::RequestPaintWhileHidden);
            crate::wake(ctx);
        }
        self.job = Some(Job { action, cancelled, rx, kind, for_record, select_shot, target, windows, started, first_screen, pending: sck });
    }

    /// A new picture. `target`: an extra folder to save it into (and copy it), whatever
    /// "After screenshot" says: no editor opens.
    pub(super) fn handle_capture(&mut self, ctx: &egui::Context, path: PathBuf, target: Option<PathBuf>) {
        let name = self.settings.make_name("Screenshot");
        // The scale of the screen the picture came from, not of the screen the window was
        // last on: a snap from a normal screen shows at its real size, one from Retina too.
        let mut ppp = capture_scale().unwrap_or(self.native_ppp);
        // "Shrink Retina snaps": store at normal (non-Retina) size.
        if self.settings.shrink_retina && ppp > 1.01 && let Ok(img) = image::open(&path) {
            let w = (img.width() as f32 / ppp).round().max(1.0) as u32;
            let h = (img.height() as f32 / ppp).round().max(1.0) as u32;
            let small = image::imageops::resize(&img.to_rgba8(), w, h, image::imageops::FilterType::Triangle);
            let _ = small.save(&path);
            ppp = 1.0;
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
}
