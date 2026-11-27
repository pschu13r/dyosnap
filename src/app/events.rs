//! Menu-bar items, global shortcuts and the actions they start.

use super::*;

pub(super) fn action_from_id(id: &str, delay: u32) -> Option<Action> {
    if let Some(n) = crate::settings::folder_id(id) {
        return Some(Action::CaptureTo(n));
    }
    Some(match id {
        "area" => Action::Capture(CaptureKind::Area),
        "previous" => Action::Capture(CaptureKind::Previous),
        "window" => Action::Capture(CaptureKind::Window),
        "full" => Action::Capture(CaptureKind::FullScreen),
        "timed" => Action::Capture(CaptureKind::Timed(delay)),
        "record" => Action::Record,
        "show" => Action::Show,
        "open" => Action::OpenImage,
        "clipboard" => Action::OpenClipboard,
        "latest" => Action::OpenLatest,
        "folder" => Action::OpenFolder,
        "settings" => Action::Settings,
        "quit" => Action::Quit,
        "finish" => Action::FinishRec,
        "cancel" => Action::CancelRec,
        "pause" | "rec_pause" => Action::PauseRec,
        "rec_stop" => Action::FinishRec,
        _ => return None,
    })
}

/// What a capture or recording action does while a capture is being chosen.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Switch {
    /// Nothing is being chosen: start it.
    Start,
    /// The same action again: cancel, like Esc.
    Cancel,
    /// Another action: cancel what is chosen and start this.
    Replace,
}

pub(super) fn capture_switch(choosing: Option<Action>, new: Action) -> Switch {
    match choosing {
        None => Switch::Start,
        Some(c) if c == new => Switch::Cancel,
        Some(_) => Switch::Replace,
    }
}

/// The menu-bar menu, laid out like the original's but without the cloud items:
/// "Latest Uploads" becomes a list of recent local snaps, "Storage" opens the save folder.
pub(super) fn build_menu(recording: bool, st: &Settings) -> Vec<platform::MenuEntry> {
    use platform::MenuEntry;
    let item = |id: &str, text: &str| MenuEntry::Item { id: id.into(), text: text.into(), enabled: true, key: None };
    // Menu items show the shortcut chosen in Settings › Hotkeys.
    let keyed = |id: &str, key_id: &str, text: &str| MenuEntry::Item {
        id: id.into(),
        text: text.into(),
        enabled: true,
        key: if st.hotkeys { st.hotkey(key_id) } else { None },
    };
    if recording {
        return vec![
            keyed("finish", "rec_stop", "Finish Recording"),
            keyed("pause", "rec_pause", "Pause / Resume"),
            item("cancel", "Cancel Recording"),
        ];
    }
    let files = recent_files(&st.save_dir, 10);
    let mut recent: Vec<MenuEntry> = files
        .iter()
        .map(|p| item(&format!("recent:{}", p.display()), p.file_name().and_then(|n| n.to_str()).unwrap_or("")))
        .collect();
    if recent.is_empty() {
        recent.push(MenuEntry::Item { id: String::new(), text: "Nothing saved yet".into(), enabled: false, key: None });
    }
    vec![
        keyed("area", "area", "Capture Area"),
        keyed("previous", "previous", "Capture Previous Area"),
        keyed("full", "full", "Capture Fullscreen"),
        keyed("record", "record", "Record Video"),
        MenuEntry::Separator,
        item("open", "Open…"),
        keyed("clipboard", "clipboard", "Open from Clipboard"),
        keyed("latest", "latest", "Open Latest Image"),
        MenuEntry::Separator,
        MenuEntry::Submenu { text: "Latest Snaps".into(), items: recent },
        item("folder", "Saved Snaps Folder"),
        MenuEntry::Separator,
        item("settings", "Preferences…"),
        MenuEntry::Item {
            id: "quit".into(),
            text: "Quit".into(),
            enabled: true,
            key: Some(crate::settings::Hotkey::new(true, false, "KeyQ")),
        },
    ]
}

impl DyoSnap {
    pub(super) fn setup_hotkeys(&mut self, ctx: &egui::Context, tx: Sender<AppEvent>) {
        let ctx = ctx.clone();
        self.hotkeys = platform::Hotkeys::new(std::sync::Arc::new(move |id: String| {
            let _ = tx.send(AppEvent::Hotkey(id));
            crate::wake(&ctx);
        }));
        self.register_hotkeys();
    }

    pub(super) fn unregister_hotkeys(&mut self) {
        if let Some(h) = &mut self.hotkeys {
            h.set(&[]);
        }
    }

    /// (Re)registers the global shortcuts from the settings.
    pub(super) fn register_hotkeys(&mut self) {
        // Linux announces every action, so the desktop can list those without a key too.
        let st = &self.settings;
        let keys: Vec<(String, String, Option<crate::settings::Hotkey>)> = if st.hotkeys {
            let folders = st.extra_dirs.iter().map(|d| crate::settings::folder_action(d.id));
            crate::settings::HOTKEY_ACTIONS
                .iter()
                .chain(crate::settings::VIDEO_HOTKEY_ACTIONS.iter())
                .map(|(id, _)| id.to_string())
                .chain(folders)
                .filter(|id| action_from_id(id, st.delay_secs).is_some())
                .map(|id| (st.action_description(&id), st.hotkey(&id), id))
                .filter(|(_, k, _)| cfg!(target_os = "linux") || k.is_some())
                .map(|(d, k, id)| (id, d, k))
                .collect()
        } else {
            Vec::new()
        };
        if let Some(h) = &mut self.hotkeys {
            h.set(&keys);
        }
    }

    /// Test hooks: LS_TEST_PREFS=1 opens Preferences once, as the menu-bar item does;
    /// LS_TEST_ABOUT=1 opens Preferences › About. On Linux they also run while no window
    /// is open (the app starts that way after the first start). LS_TEST_PREFS_AFTER=<s>
    /// waits that many seconds first (to put another app in front meanwhile).
    pub(super) fn test_open_prefs(&mut self, ctx: &egui::Context) {
        if self.test_prefs_done || self.prefs.open {
            return;
        }
        if let Some(secs) = std::env::var("LS_TEST_PREFS_AFTER").ok().and_then(|v| v.parse::<u64>().ok()) {
            static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
            let left = Duration::from_secs(secs).saturating_sub(START.get_or_init(Instant::now).elapsed());
            if cfg!(debug_assertions) && !left.is_zero() {
                ctx.send_viewport_cmd(ViewportCommand::RequestPaintWhileHidden);
                ctx.request_repaint_after(left.min(Duration::from_millis(200)));
                return;
            }
        }
        let about = test_env("LS_TEST_ABOUT");
        if about || test_env("LS_TEST_PREFS") {
            self.test_prefs_done = true;
            self.run_action(ctx, Action::Settings);
            if about {
                self.prefs.tab = crate::prefs::Tab::About;
            }
        }
    }

    pub(super) fn run_action(&mut self, ctx: &egui::Context, a: Action) {
        match a {
            Action::Capture(kind) => {
                if self.switch_choosing(ctx, a) {
                    self.start_capture(ctx, kind, false)
                }
            }
            Action::CaptureTo(n) => {
                // A shortcut of a folder that was removed does nothing.
                if let Some(dir) = self.settings.extra_dirs.iter().find(|d| d.id == n).map(|d| d.path.clone())
                    && self.switch_choosing(ctx, a)
                {
                    self.start_capture_to(ctx, n, Some(dir));
                }
            }
            Action::Record => {
                if self.recording.is_none() && self.switch_choosing(ctx, a) {
                    self.start_capture(ctx, CaptureKind::FullScreen, true)
                }
            }
            Action::Show => self.show_window(ctx),
            Action::OpenClipboard => {
                // Linux: GNOME tells only the focused app what is on the clipboard. Open the
                // window first and read once it has the focus (see `logic`).
                #[cfg(target_os = "linux")]
                if !platform::clipboard_ready_known() {
                    self.clipboard_wait = Some(Instant::now());
                    self.show_window(ctx);
                    return;
                }
                self.open_clipboard(ctx);
            }
            Action::OpenLatest => {
                // The last screenshot taken, even if it was never saved.
                let latest = latest_capture_path().filter(|p| p.is_file()).and_then(|p| image::open(&p).ok());
                if let Some(img) = latest {
                    let name = latest_capture_path()
                        .map(|p| p.with_extension("txt"))
                        .and_then(|p| std::fs::read_to_string(p).ok())
                        .map(|n| n.trim().to_string())
                        .filter(|n| !n.is_empty())
                        .unwrap_or_else(|| self.settings.make_name("Screenshot"));
                    let ed = Editor::new(img.to_rgba8(), name, self.native_ppp);
                    self.set_editor(ctx, ed);
                } else if let Some(p) = recent_files(&self.settings.save_dir, 20).into_iter().find(|p| is_picture(p)) {
                    self.open_file(ctx, &p);
                } else {
                    self.toast("No screenshot taken yet", None);
                }
                self.show_window(ctx);
            }
            Action::OpenImage => {
                self.show_window(ctx);
                let d = rfd::AsyncFileDialog::new().add_filter("Pictures", &["png", "jpg", "jpeg", "mp4", "mov"]);
                self.spawn_dialog(ctx, DialogKind::OpenImage, d.pick_file());
            }
            Action::OpenFolder => {
                let _ = std::fs::create_dir_all(&self.settings.save_dir);
                platform::open_path(&self.settings.save_dir);
            }
            Action::Settings => {
                self.prefs.open();
                // Also when the main window is covered by another app's window: egui runs
                // no frames for a covered window, and the Preferences window is drawn
                // in those frames.
                ctx.send_viewport_cmd(ViewportCommand::RequestPaintWhileHidden);
            }
            Action::Quit => {
                if let Some(r) = self.recording.take() {
                    r.rec.cancel();
                }
                self.settings.save();
                std::process::exit(0);
            }
            Action::FinishRec => self.finish_recording(ctx),
            Action::CancelRec => {
                if let Some(r) = self.recording.take() {
                    r.rec.cancel();
                }
                self.set_tray_recording(false);
                self.toast("Recording cancelled", None);
            }
            Action::PauseRec => {
                if let Some(r) = &mut self.recording {
                    r.rec.toggle_pause();
                    r.last_secs = u64::MAX;
                }
            }
        }
    }

    /// A capture or recording action while a capture is still being chosen: the same action
    /// cancels it, another one cancels it and starts. Returns whether the action goes on
    /// to start something.
    fn switch_choosing(&mut self, ctx: &egui::Context, a: Action) -> bool {
        match capture_switch(self.choosing(), a) {
            Switch::Start => true,
            Switch::Cancel => {
                self.cancel_choosing(ctx, false);
                false
            }
            Switch::Replace => {
                if self.reuse_picture(ctx, a) {
                    return false;
                }
                self.cancel_choosing(ctx, true);
                true
            }
        }
    }

    /// macOS: switches between the screenshot and the record overlay on the picture that is
    /// frozen already: no wait for the old overlay to vanish, no new capture, no decode.
    /// Only for actions that would open an overlay on a fresh picture right away; anything
    /// else (and a failure) returns false and takes the normal way, cancel and start.
    /// The main window is hidden since the first start and `visible` stays false, so there
    /// is nothing to restore. The job is None (the overlay is open), so no flag is left.
    fn reuse_picture(&mut self, ctx: &egui::Context, a: Action) -> bool {
        // Not while the picture is still being taken: the overlay has nothing to reuse.
        if !cfg!(target_os = "macos") || self.job.is_some() || self.select.as_ref().is_none_or(|s| s.screens.iter().any(|x| x.img.is_none())) {
            return false;
        }
        // The extra folder of a "capture to" shortcut, if it still exists.
        let target = match a {
            Action::CaptureTo(n) => self.settings.extra_dirs.iter().find(|d| d.id == n).map(|d| d.path.clone()),
            _ => None,
        };
        let record = match a {
            Action::Record => true,
            Action::Capture(CaptureKind::Area) | Action::CaptureTo(_) => false,
            Action::Capture(CaptureKind::Previous) if !platform::last_area_known() => false,
            _ => return false,
        };
        let started = Instant::now();
        let displays = platform::displays();
        let Some(old) = self.select.as_mut() else { return false };
        let same = |s: &SelScreen, d: &platform::Display| (s.bounds.min - d.bounds.min).abs().max_elem() < 1.0;
        let screens = if record {
            // All screens, in the order of `displays()`: the ones we have, and a silent
            // picture of each other one (no overlay is on them, so no wait).
            if !displays.iter().any(|d| old.screens.iter().any(|s| same(s, d))) {
                return false;
            }
            let mut have: Vec<Option<SelScreen>> = old.screens.drain(..).map(Some).collect();
            let mut all = Vec::new();
            for (i, d) in displays.iter().enumerate() {
                match have.iter_mut().find(|s| s.as_ref().is_some_and(|s| same(s, d))).and_then(|s| s.take()) {
                    Some(s) => all.push(s),
                    None => all.extend(self.sel_screens(decode_screens(platform::capture_screen(i)), i)),
                }
            }
            all
        } else {
            // Only the screen under the mouse, as a fresh area screenshot does.
            let i = platform::mouse_position().map_or(0, |m| platform::display_index_at(&displays, m));
            let Some(pos) = displays.get(i).and_then(|d| old.screens.iter().position(|s| same(s, d))) else {
                return false;
            };
            vec![old.screens.swap_remove(pos)]
        };
        if screens.is_empty() {
            return false;
        }
        let sel = if record {
            // Record options need the devices list, as in `start_capture_ex`.
            self.list_devices();
            Select::for_record(a, screens, started)
        } else {
            let mut sel = Select::new(a, screens, None, started);
            sel.shot = true;
            sel.target = target;
            sel
        };
        self.select = Some(sel);
        ctx.send_viewport_cmd(ViewportCommand::RequestPaintWhileHidden);
        true
    }

    /// Opens the picture on the clipboard in the editor.
    pub(super) fn open_clipboard(&mut self, ctx: &egui::Context) {
        let tmp = platform::temp_file("clip", "png");
        if platform::clipboard_png_to_file(&tmp) {
            match image::open(&tmp) {
                Ok(img) => {
                    let ed = Editor::new(img.to_rgba8(), self.settings.make_name("Screenshot"), self.native_ppp);
                    self.set_editor(ctx, ed);
                }
                Err(_) => self.toast("There is no picture on the clipboard", None),
            }
        } else {
            self.toast("There is no picture on the clipboard", None);
        }
        let _ = std::fs::remove_file(&tmp);
        self.show_window(ctx);
    }

    /// The frozen pictures of a capture job as screens to choose an area on.
    /// Picture `i` belongs to screen `first + i` of `displays()`.
    fn sel_screens(&self, res: Vec<ColorImage>, first: usize) -> Vec<SelScreen> {
        let displays = platform::displays();
        let mut screens = Vec::new();
        for (i, img) in res.into_iter().enumerate() {
            let d = displays.get(first + i).copied().unwrap_or(platform::Display {
                bounds: Rect::from_min_size(Pos2::ZERO, self.monitor),
                scale: img.size[0] as f32 / self.monitor.x.max(1.0),
            });
            log::info!("select area: screen {i} at {:?} scale {} picture {}x{}", d.bounds, d.scale, img.size[0], img.size[1]);
            screens.push(SelScreen { bounds: d.bounds, scale: d.scale, img: Some(Arc::new(img)), tex: None });
        }
        screens
    }

    pub(super) fn set_tray_recording(&mut self, on: bool) {
        if self.tray_recording == on {
            return;
        }
        self.tray_recording = on;
        if let Some(t) = &self.tray {
            t.set_recording(on);
            t.set_menu(build_menu(on, &self.settings));
            t.set_title(if on { Some("00:00") } else { None });
        }
    }

    pub(super) fn handle_events(&mut self, ctx: &egui::Context) {
        while let Ok(ev) = self.events.try_recv() {
            if let AppEvent::Dialog(kind, path) = &ev {
                let Some(p) = path.clone() else { continue };
                match kind {
                    DialogKind::OpenImage => {
                        self.open_file(ctx, &p);
                        self.show_window(ctx);
                    }
                    DialogKind::SaveAs => {
                        self.save_picture_at(Some(p), true);
                    }
                    DialogKind::SaveDir => {
                        self.settings.save_dir = p;
                        self.settings.save();
                        self.recent_dirty = true;
                        self.tray_menu_dirty = true;
                    }
                    DialogKind::ExtraDir => {
                        if self.settings.add_extra_dir(p) {
                            self.settings.save();
                            // A new shortcut id: the desktop binds the list again (Linux).
                            if self.prefs.capturing.is_none() {
                                self.register_hotkeys();
                            }
                        }
                    }
                    DialogKind::EditorApp => {
                        self.settings.external_editor = p.display().to_string();
                        self.settings.save();
                    }
                }
                continue;
            }
            if let AppEvent::Permission(ok) = ev {
                self.perm_asking = false;
                self.perm_prompt = !ok;
                if ok {
                    self.toast("Screenshots allowed", None);
                }
                continue;
            }
            if let AppEvent::Menu(id) = &ev && let Some(path) = id.strip_prefix("recent:") {
                let p = PathBuf::from(path);
                self.open_file(ctx, &p);
                self.show_window(ctx);
                continue;
            }
            let action = match ev {
                AppEvent::Menu(id) => action_from_id(&id, self.settings.delay_secs),
                // The desktop's shortcuts changed (Linux): show the keys it has now.
                AppEvent::Hotkey(id) if id == "keys-changed" => {
                    self.settings.live_keys = self.hotkeys.as_ref().and_then(|h| h.live_labels());
                    self.tray_menu_dirty = true;
                    None
                }
                AppEvent::Hotkey(id) => action_from_id(&id, self.settings.delay_secs),
                AppEvent::Dialog(..) | AppEvent::Permission(_) => None,
            };
            if let Some(a) = action {
                // During a recording the record shortcut finishes it.
                let a = if a == Action::Record && self.recording.is_some() { Action::FinishRec } else { a };
                self.run_action(ctx, a);
            }
        }

        if let Some(job) = &self.job && let Ok(res) = job.rx.try_recv() {
            let (kind, for_record, job_action) = (job.kind, job.for_record, job.action);
            let job_target = job.target.clone();
            let select_shot = job.select_shot;
            let (started, first_screen, pending) = (job.started, job.first_screen, job.pending);
            let job_windows = job.windows.lock().map(|w| w.clone()).unwrap_or_default();
            self.job = None;
            if for_record || select_shot {
                log::info!(target: "dyosnap", "select: job result received after {} ms", started.elapsed().as_millis());
            }
            let mut retried = false;
            let (file, pictures) = match res {
                JobOut::Retry => {
                    retried = true;
                    // The overlay goes away and the next picture waits for it to vanish (see
                    // `was_visible`), so it can not end up in the picture.
                    self.cancel_choosing(ctx, true);
                    self.start_capture_ex(ctx, job_action, kind, for_record, job_target.clone(), false);
                    (None, Vec::new())
                }
                JobOut::Files(files) => (files.first().cloned(), Vec::new()),
                JobOut::Pictures(pictures) => (None, pictures),
            };
            match (file, !pictures.is_empty(), for_record || select_shot) {
                _ if retried => {}
                (Some(p), _, false) => self.handle_capture(ctx, p, job_target),
                (_, true, true) if pending => {
                    // The overlay is open: it gets the picture.
                    let fits = self.select.as_ref().is_some_and(|s| s.screens.len() == pictures.len());
                    if fits && let Some(sel) = self.select.as_mut() {
                        for (scr, img) in sel.screens.iter_mut().zip(pictures) {
                            scr.img = Some(Arc::new(img));
                        }
                        ctx.send_viewport_cmd(ViewportCommand::RequestPaintWhileHidden);
                        ctx.request_repaint();
                    } else {
                        self.cancel_choosing(ctx, false);
                        self.show_window(ctx);
                        self.toast_no_screen();
                    }
                }
                (_, true, true) if select_shot => {
                    let screens = self.sel_screens(pictures, first_screen);
                    let previous = if kind == CaptureKind::Previous { self.last_area } else { None };
                    // Without choosing: the previous area again, or the whole screen.
                    let instant = match kind {
                        CaptureKind::FullScreen | CaptureKind::Timed(_) => Some((Instant::now(), None)),
                        _ => previous.map(|r| (Instant::now(), Some(r))),
                    };
                    if screens.is_empty() {
                        platform::end_screen_session();
                        self.show_window(ctx);
                        self.toast_no_screen();
                    } else {
                        let mut sel = Select::new(job_action, screens, None, started);
                        sel.shot = true;
                        sel.target = job_target;
                        sel.instant = instant;
                        sel.windows = job_windows;
                        self.select = Some(sel);
                        ctx.send_viewport_cmd(ViewportCommand::RequestPaintWhileHidden);
                    }
                }
                (_, true, true) => {
                    let screens = self.sel_screens(pictures, 0);
                    if screens.is_empty() {
                        self.show_window(ctx);
                        self.perm_prompt = !platform::screenshot_allowed();
                        self.toast_no_screen();
                    } else {
                        self.select = Some(Select::for_record(job_action, screens, started));
                        ctx.send_viewport_cmd(ViewportCommand::RequestPaintWhileHidden);
                    }
                }
                (_, false, true) => {
                    // No picture: also close an overlay that waited for it.
                    if pending {
                        self.cancel_choosing(ctx, false);
                    }
                    self.show_window(ctx);
                    self.perm_prompt = !platform::screenshot_allowed();
                    self.toast_no_screen();
                }
                (_, _, false) => {
                    let asks = matches!(kind, CaptureKind::Area | CaptureKind::Previous | CaptureKind::Window);
                    if !asks && !platform::screenshot_allowed() {
                        self.perm_prompt = true;
                        self.show_window(ctx);
                    }
                }
            }
        }

        if let Some(rx) = &self.finishing && let Ok(res) = rx.try_recv() {
            self.finishing = None;
            match res {
                Ok(path) => {
                    let mut v = VideoView::new(path, self.ffmpeg.clone(), self.rec_started_name.clone(), 0.0);
                    v.seek(ctx, 0.0);
                    self.fit_video_window(v.size);
                    self.ed = None;
                    self.video = Some(v);
                    if self.settings.autosave_movies {
                        let movies = dirs::video_dir()
                            .or_else(|| dirs::home_dir().map(|h| h.join("Movies")))
                            .unwrap_or_else(|| self.settings.save_dir.clone());
                        self.save_video_to(false, movies, false);
                    }
                }
                Err(e) => self.toast(e, None),
            }
            self.show_window(ctx);
        }

        if let Some(rx) = &self.video_saving && let Ok(res) = rx.try_recv() {
            self.video_saving = None;
            let close_src = self.close_video_after_save.take();
            match res {
                Ok(p) => {
                    if let Some(v) = &mut self.video {
                        v.saved = Some(p.clone());
                    }
                    // Save or Copy was clicked and the same video is still open: it is done.
                    let close = self.ed.is_none() && close_src.is_some_and(|src| self.video.as_ref().is_some_and(|v| v.path == src));
                    if self.copy_after_save {
                        platform::copy_file_ref(&p);
                        self.toast("Saved and copied the video file", Some(p));
                    } else {
                        if close {
                            platform::notify("Saved", &short_path(&p));
                        }
                        self.toast(format!("Saved to {}", short_path(&p)), Some(p));
                    }
                    self.recent_dirty = true;
                    self.tray_menu_dirty = true;
                    if close {
                        self.close_video();
                    }
                }
                // The editor stays open, so nothing is lost.
                Err(e) => self.toast(e, None),
            }
        }

        // Keep the menu-bar list of recent snaps current.
        if self.tray_menu_dirty && self.recording.is_none() {
            self.tray_menu_dirty = false;
            if let Some(t) = &self.tray {
                t.set_menu(build_menu(false, &self.settings));
            }
        }

        // Recording timer in the menu bar, once per second.
        if let Some(r) = &mut self.recording {
            if let Some(err) = r.rec.poll_failure() {
                let r = self.recording.take().unwrap();
                r.rec.cancel();
                self.set_tray_recording(false);
                self.show_window(ctx);
                self.toast(err, None);
            } else {
                let secs = r.rec.elapsed().as_secs();
                if secs != r.last_secs {
                    r.last_secs = secs;
                    let paused = r.rec.is_paused();
                    #[cfg(target_os = "linux")]
                    if let Some(f) = &r.frame {
                        f.set_status(&record::format_elapsed(r.rec.elapsed()), paused);
                    }
                    let title = format!("{}{}", if paused { "❚❚ " } else { "" }, record::format_elapsed(r.rec.elapsed()));
                    if let Some(t) = &self.tray {
                        t.set_title(Some(&title));
                        // Linux: the tray shows no text; a ring around the dot fills each minute.
                        #[cfg(target_os = "linux")]
                        t.set_progress(r.rec.elapsed().as_secs(), paused);
                    }
                }
                ctx.request_repaint_after(Duration::from_millis(500));
            }
        }

        // Linux (KDE): a click on the control bar below the recorded area.
        #[cfg(target_os = "linux")]
        if let Some(a) = self.recording.as_ref().and_then(|r| r.frame.as_ref()).and_then(|f| f.take_action()) {
            let action = match a {
                platform::BarAction::Pause => Action::PauseRec,
                platform::BarAction::Stop => Action::FinishRec,
                platform::BarAction::Cancel => Action::CancelRec,
            };
            self.run_action(ctx, action);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_ids_start_a_capture_into_that_folder() {
        assert_eq!(action_from_id("folder3", 5), Some(Action::CaptureTo(3)));
        assert_eq!(action_from_id("folder1", 5), Some(Action::CaptureTo(1)));
        assert_eq!(action_from_id("folder12", 5), Some(Action::CaptureTo(12)));
        assert_eq!(action_from_id("folder0", 5), None);
        // The tray's "folder" item still opens the save folder.
        assert_eq!(action_from_id("folder", 5), Some(Action::OpenFolder));
        assert_eq!(action_from_id("area", 5), Some(Action::Capture(CaptureKind::Area)));
    }

    #[test]
    fn a_new_action_replaces_what_is_chosen_and_the_same_one_cancels_it() {
        let area = Action::Capture(CaptureKind::Area);
        assert_eq!(capture_switch(None, area), Switch::Start);
        assert_eq!(capture_switch(Some(area), area), Switch::Cancel);
        assert_eq!(capture_switch(Some(area), Action::Record), Switch::Replace);
        assert_eq!(capture_switch(Some(Action::Record), area), Switch::Replace);
        assert_eq!(capture_switch(Some(Action::CaptureTo(1)), Action::CaptureTo(2)), Switch::Replace);
        assert_eq!(capture_switch(Some(Action::Capture(CaptureKind::Timed(3))), Action::Capture(CaptureKind::Timed(5))), Switch::Replace);
    }
}
