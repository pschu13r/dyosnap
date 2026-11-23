//! Opening, saving and copying pictures and videos.

use super::*;

/// The picture as the bytes of a file; the format follows the extension of `dest`.
pub(super) fn encode_image(img: &image::RgbaImage, dest: &Path, jpeg_quality: u8) -> Result<Vec<u8>, String> {
    let ext = dest.extension().and_then(|e| e.to_str()).unwrap_or("png").to_lowercase();
    let mut out = std::io::Cursor::new(Vec::new());
    if ext == "jpg" || ext == "jpeg" {
        let rgb = image::DynamicImage::ImageRgba8(img.clone()).to_rgb8();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, jpeg_quality.clamp(10, 100))
            .encode_image(&rgb)
            .map_err(|e| e.to_string())?;
    } else {
        let fmt = image::ImageFormat::from_extension(&ext).unwrap_or(image::ImageFormat::Png);
        img.write_to(&mut out, fmt).map_err(|e| e.to_string())?;
    }
    Ok(out.into_inner())
}

/// Writes `bytes` into a new file in `dir` named `stem.ext` (or the next free "(n)" name);
/// the file is made in one step that never overwrites or follows a link. A failed
/// write leaves no half file behind.
pub(super) fn write_new(dir: &Path, stem: &str, ext: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    use std::io::Write;
    let (path, mut f) = crate::settings::create_new_in(dir, stem, ext)?;
    if let Err(e) = f.write_all(bytes) {
        drop(f);
        let _ = std::fs::remove_file(&path);
        return Err(e);
    }
    Ok(path)
}

/// The file type a video is saved as: the chosen format, or without ffmpeg the type it
/// already has (then it can only be copied as it is).
pub(super) fn video_save_ext(v: &VideoView, fmt: crate::settings::VideoFormat) -> String {
    if v.ffmpeg.is_some() {
        fmt.ext().to_string()
    } else {
        v.path.extension().and_then(|e| e.to_str()).unwrap_or("mp4").to_lowercase()
    }
}

pub(super) fn short_path(p: &Path) -> String {
    let s = p.display().to_string();
    match dirs::home_dir() {
        Some(h) => {
            let h = h.display().to_string();
            if let Some(rest) = s.strip_prefix(&h) { format!("~{rest}") } else { s }
        }
        None => s,
    }
}

pub(super) fn recent_files(dir: &Path, n: usize) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut v: Vec<(std::time::SystemTime, PathBuf)> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let p = e.path();
            let ext = p.extension()?.to_str()?.to_lowercase();
            if !matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "mp4" | "mov") {
                return None;
            }
            Some((e.metadata().ok()?.modified().ok()?, p))
        })
        .collect();
    v.sort_by(|a, b| b.0.cmp(&a.0));
    v.into_iter().take(n).map(|(_, p)| p).collect()
}

/// Where the newest screenshot is kept (one file, replaced by each new capture).
pub(super) fn latest_capture_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("DyoSnap").join("latest-capture.png"))
}

pub(super) fn is_picture(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_lowercase().as_str(), "png" | "jpg" | "jpeg"))
}

impl DyoSnap {
    pub(super) fn open_file(&mut self, ctx: &egui::Context, p: &Path) {
        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
        if matches!(ext.as_str(), "mp4" | "mov" | "m4v") {
            if self.ffmpeg.is_some() {
                let name = p.file_stem().and_then(|s| s.to_str()).unwrap_or("Video").to_string();
                let mut v = VideoView::new(p.to_path_buf(), self.ffmpeg.clone(), name, 0.0);
                v.saved = Some(p.to_path_buf());
                v.seek(ctx, 0.0);
                self.fit_video_window(v.size);
                self.ed = None;
                self.video = Some(v);
            } else {
                platform::open_path(p);
            }
            return;
        }
        match image::open(p) {
            Ok(img) => {
                let name = if self.settings.apply_template_reopened {
                    self.settings.make_name("Screenshot")
                } else {
                    p.file_stem().and_then(|s| s.to_str()).unwrap_or("Picture").to_string()
                };
                let mut ed = Editor::new(img.to_rgba8(), name.clone(), self.native_ppp);
                if p.starts_with(&self.settings.save_dir) {
                    ed.saved = Some(p.to_path_buf());
                    ed.saved_name = name;
                }
                self.set_editor(ctx, ed);
            }
            Err(e) => self.toast(format!("Could not open the picture: {e}"), None),
        }
    }

    /// Sizes the window for a new picture or video: `want`, but at most 75% of the usable
    /// screen (below the menu bar / top bar), centred on it. Never more, even if that is
    /// smaller than `want`: the content is then shown smaller.
    pub(super) fn fit_window(&mut self, want: egui::Vec2) {
        // Linux: the window may be closed, so ask the system for the screen.
        self.refresh_screen();
        let (menu_bar, title_bar) = if cfg!(target_os = "linux") { (32.0, 36.0) } else { (25.0, 28.0) };
        let usable = self.monitor - vec2(0.0, menu_bar);
        let max = (usable * 0.75 - vec2(0.0, title_bar)).max(vec2(320.0, 240.0));
        let inner = want.min(max);
        let outer = inner + vec2(0.0, title_bar);
        self.pending_size = Some(inner);
        self.pending_pos = Some(pos2(
            ((self.monitor.x - outer.x) / 2.0).max(0.0),
            menu_bar + ((usable.y - outer.y) / 2.0).max(0.0),
        ));
    }

    /// Sizes the window for a video of `size` pixels (after `seek` knows it).
    pub(super) fn fit_video_window(&mut self, size: [u32; 2]) {
        if size[0] == 0 {
            return;
        }
        // The scale comes from the system on Linux, so refresh before using it.
        self.refresh_screen();
        let sz = vec2(size[0] as f32, size[1] as f32) / self.native_ppp;
        self.fit_window(vec2((sz.x + 60.0).max(900.0), (sz.y + 52.0 + 64.0 + 40.0).max(480.0)));
    }

    pub(super) fn set_editor(&mut self, ctx: &egui::Context, ed: Editor) {
        // A freshly opened picture is not an edit; don't copy it again.
        self.clip_version = ed.version;
        self.clip_due = None;
        let size = ed.doc.size() / ed.ppp;
        self.fit_window(vec2((size.x + 60.0).max(980.0), (size.y + 52.0 + 52.0 + 60.0).max(480.0)));
        self.video = None;
        self.ed = Some(ed);
        let _ = ctx;
    }

    /// After each edit, put the finished picture on the clipboard (quietly, in the background).
    pub(super) fn auto_copy_edits(&mut self, ctx: &egui::Context) {
        let Some(ed) = &self.ed else {
            self.clip_due = None;
            return;
        };
        if ed.version == self.clip_version || ed.is_busy() {
            return;
        }
        let now = Instant::now();
        let due = *self.clip_due.get_or_insert(now + Duration::from_millis(350));
        if now < due {
            ctx.request_repaint_after(due - now);
            return;
        }
        self.clip_due = None;
        self.clip_version = ed.version;
        let doc = ed.doc.clone();
        let saved = if self.settings.copy_path { ed.saved.clone() } else { None };
        let image = self.settings.copy_image;
        let tiff = self.settings.clip_format == crate::settings::ClipFormat::Tiff;
        let font = self.font.clone();
        let gen_id = self.clip_gen.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        let gen_now = self.clip_gen.clone();
        std::thread::spawn(move || {
            let img = crate::doc::render(&doc, &font, None);
            let tmp = platform::temp_file("autocopy", "png");
            // A newer edit wins: skip this one if another copy was started meanwhile.
            if img.save(&tmp).is_ok() && gen_now.load(std::sync::atomic::Ordering::SeqCst) == gen_id {
                platform::set_clipboard(if image { Some(&tmp) } else { None }, tiff, saved.as_deref());
            }
            let _ = std::fs::remove_file(tmp);
        });
    }

    /// Puts a PNG on the clipboard the way the settings say (image as PNG/TIFF, file path).
    pub(super) fn copy_file_to_clipboard(&self, png: &Path, saved: Option<&Path>) -> bool {
        let image = if self.settings.copy_image { Some(png) } else { None };
        let path = if self.settings.copy_path { saved } else { None };
        let tiff = self.settings.clip_format == crate::settings::ClipFormat::Tiff;
        platform::set_clipboard(image, tiff, path)
    }

    /// Writes the captured picture `src` (a PNG) into a new file `name` in `dir`, in the
    /// chosen image format. Returns the path of the file.
    pub(super) fn save_capture(&self, src: &Path, dir: &Path, name: &str) -> Option<PathBuf> {
        let ext = self.settings.format.ext();
        let bytes = match self.settings.format {
            crate::settings::ImageFormat::Png => std::fs::read(src).ok()?,
            crate::settings::ImageFormat::Jpg => {
                let img = image::open(src).ok()?;
                encode_image(&img.to_rgba8(), Path::new(&format!("x.{ext}")), self.settings.jpeg_quality).ok()?
            }
        };
        write_new(dir, name, ext, &bytes).ok()
    }

    pub(super) fn save_picture(&mut self, save_as: bool) -> Option<PathBuf> {
        self.save_picture_ex(save_as, true)
    }

    pub(super) fn save_picture_ex(&mut self, save_as: bool, close: bool) -> Option<PathBuf> {
        if save_as {
            let fmt = self.settings.format;
            let name = self.ed.as_ref()?.name.clone();
            let d = rfd::AsyncFileDialog::new()
                .set_directory(&self.settings.save_dir)
                .set_file_name(format!("{}.{}", crate::settings::sanitize(&name), fmt.ext()))
                .add_filter("Picture", &["png", "jpg", "jpeg"]);
            let ctx = self.egui_ctx.clone();
            self.spawn_dialog(&ctx, DialogKind::SaveAs, d.save_file());
            return None;
        }
        self.save_picture_at(None, close)
    }

    /// Saves the picture to `dest`, or to its usual place in the save folder.
    pub(super) fn save_picture_at(&mut self, dest_override: Option<PathBuf>, close: bool) -> Option<PathBuf> {
        let font = self.font.clone();
        let fmt = self.settings.format;
        let q = self.settings.jpeg_quality;
        let ed = self.ed.as_mut()?;
        ed.commit_text();
        // A chosen file (Save As) or the picture's own file is written again; a new picture
        // gets a new file in the save folder.
        let target = if let Some(d) = dest_override {
            Some(d)
        } else {
            match &ed.saved {
                Some(p) if ed.saved_name == ed.name && p.extension().is_some_and(|e| e == fmt.ext()) => Some(p.clone()),
                _ => None,
            }
        };
        let img = ed.flatten(&font);
        let probe = target.clone().unwrap_or_else(|| PathBuf::from(format!("x.{}", fmt.ext())));
        let written = encode_image(&img, &probe, q).and_then(|bytes| match &target {
            Some(d) => std::fs::write(d, bytes).map(|_| d.clone()).map_err(|e| e.to_string()),
            None => {
                let _ = std::fs::create_dir_all(&self.settings.save_dir);
                write_new(&self.settings.save_dir, &ed.name, fmt.ext(), &bytes).map_err(|e| e.to_string())
            }
        });
        match written {
            Ok(dest) => {
                ed.saved = Some(dest.clone());
                ed.saved_name = ed.name.clone();
                self.recent_dirty = true;
                self.tray_menu_dirty = true;
                self.toast(format!("Saved to {}", short_path(&dest)), Some(dest.clone()));
                // Like the original: saving finishes the picture and closes the editor.
                if close {
                    self.ed = None;
                    self.close_after_save = true;
                    platform::notify("Saved", &short_path(&dest));
                }
                Some(dest)
            }
            Err(e) => {
                self.toast(format!("Could not save: {e}"), None);
                None
            }
        }
    }

    pub(super) fn copy_picture(&mut self) {
        let font = self.font.clone();
        let Some(ed) = self.ed.as_mut() else { return };
        ed.commit_text();
        let img = ed.flatten(&font);
        // The path needs a saved file; save first (the editor stays open).
        let saved = if self.settings.copy_path { self.save_picture_ex(false, false) } else { None };
        let tmp = platform::temp_file("copy", "png");
        if img.save(&tmp).is_ok() && self.copy_file_to_clipboard(&tmp, saved.as_deref()) {
            self.toast("Copied to clipboard", saved);
        } else {
            self.toast("Could not copy the picture", None);
        }
        let _ = std::fs::remove_file(tmp);
    }

    pub(super) fn open_in_external_editor(&mut self) {
        let font = self.font.clone();
        let Some(ed) = self.ed.as_mut() else { return };
        ed.commit_text();
        let img = ed.flatten(&font);
        let tmp = platform::temp_file(&crate::settings::sanitize(&ed.name), "png");
        if img.save(&tmp).is_ok() {
            platform::open_with(&self.settings.external_editor, &tmp);
        }
    }

    pub(super) fn save_video(&mut self, then_copy: bool) {
        let dir = self.settings.save_dir.clone();
        self.save_video_to(then_copy, dir);
    }

    pub(super) fn save_video_to(&mut self, then_copy: bool, dir: PathBuf) {
        let Some(v) = &mut self.video else { return };
        if self.video_saving.is_some() {
            return;
        }
        v.stop();
        let fmt = self.settings.video_format;
        let src_ext = v.path.extension().and_then(|e| e.to_str()).unwrap_or("mp4").to_lowercase();
        let ext = video_save_ext(v, fmt);
        if !v.is_trimmed() && dir == self.settings.save_dir {
            if let Some(saved) = v.saved.clone().filter(|p| p.extension().is_some_and(|e| e == ext.as_str())) {
                if then_copy {
                    platform::copy_file_ref(&saved);
                    self.toast("Copied the video file", Some(saved));
                }
                return;
            }
        }
        let _ = std::fs::create_dir_all(&dir);
        // Reserve the name by making the file; the export then fills it. (ffmpeg opens the
        // path itself, so that last step can't take the open file.)
        let dest = match crate::settings::create_new_in(&dir, &v.name, &ext) {
            Ok((p, _)) => p,
            Err(e) => {
                self.toast(format!("Could not save the video: {e}"), None);
                return;
            }
        };
        let opts = crate::video::ExportOptions {
            trim: if v.is_trimmed() { Some(v.trim) } else { None },
            format: if v.ffmpeg.is_some() { fmt } else { crate::settings::VideoFormat::Mp4 },
            gif_fps: self.settings.gif_fps,
            gif_scale: self.settings.gif_quality as f32 / 100.0,
            same_format: ext == src_ext,
        };
        let (path, ff) = (v.path.clone(), v.ffmpeg.clone());
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let res = VideoView::export(path, ff, opts, dest.clone());
            if res.is_err() {
                // No empty file for a failed export.
                let _ = std::fs::remove_file(&dest);
            }
            let _ = tx.send(res);
        });
        self.video_saving = Some(rx);
        self.copy_after_save = then_copy;
    }
}
