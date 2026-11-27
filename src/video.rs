//! After a recording: preview it, pick start and end on a timeline, then save.
//! Frames are decoded by an ffmpeg child process at reduced size, only while
//! playing or scrubbing, so the app stays idle otherwise.

use crate::settings::VideoFormat;
use egui::ColorImage;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

// Videos opened by the person may come from anywhere. `-protocol_whitelist file` stops
// ffmpeg from following links inside a file (playlists, network URLs) to other places.
// `-format_whitelist` stops ffmpeg from guessing another format from the content: a crafted
// ".mp4" could otherwise be read as an HLS playlist or ffconcat file and pull in local
// files. We only open mp4/mov/m4v, and ffmpeg's "mov" demuxer covers all of them. (GIF is
// only an export format here, never an input.)
const INPUT_FORMATS: &str = "mov";

pub struct Probe {
    pub duration: f32,
    pub size: [u32; 2],
    pub fps: Option<f32>,
    pub has_audio: bool,
}

/// Reads the frame rate from a stream line like `..., 29.97 fps, 29.97 tbr, ...`.
fn parse_fps(line: &str) -> Option<f32> {
    let before = &line[..line.find(" fps")?];
    before.rsplit([' ', ',']).next()?.parse().ok()
}

/// Reads duration and size from ffmpeg's description of the file.
pub fn probe(ffmpeg: &Path, file: &Path) -> Option<Probe> {
    let out = Command::new(ffmpeg)
        .args(["-hide_banner", "-protocol_whitelist", "file", "-format_whitelist", INPUT_FORMATS, "-i"])
        .arg(file)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stderr);
    let mut duration = None;
    let mut size = None;
    let mut fps = None;
    let mut has_audio = false;
    for line in text.lines() {
        has_audio |= line.contains("Stream #") && line.contains("Audio:");
        if let Some(i) = line.find("Duration: ") {
            let t = &line[i + 10..];
            let t = t.split(',').next().unwrap_or("");
            let parts: Vec<f32> = t.split(':').filter_map(|p| p.trim().parse().ok()).collect();
            if parts.len() == 3 {
                duration = Some(parts[0] * 3600.0 + parts[1] * 60.0 + parts[2]);
            }
        }
        if line.contains("Video:") && fps.is_none() {
            fps = parse_fps(line);
        }
        if line.contains("Video:") && size.is_none() {
            for tok in line.split([' ', ',']) {
                if let Some((a, b)) = tok.split_once('x')
                    && let (Ok(w), Ok(h)) = (a.parse::<u32>(), b.parse::<u32>())
                    && w > 1 && h > 1
                {
                    size = Some([w, h]);
                    break;
                }
            }
        }
    }
    Some(Probe { duration: duration?, size: size?, fps, has_audio })
}

pub struct VideoView {
    pub path: PathBuf,
    pub ffmpeg: Option<PathBuf>,
    pub duration: f32,
    pub size: [u32; 2],
    pub fps: Option<f32>,
    pub trim: (f32, f32),
    pub pos: f32,
    pub name: String,
    pub saved: Option<PathBuf>,
    preview_size: [u32; 2],
    frame: Arc<Mutex<Option<ColorImage>>>,
    pub tex: Option<egui::TextureHandle>,
    generation: Arc<AtomicU64>,
    play_started: Option<(Instant, f32)>,
    has_audio: bool,
    /// The ffmpeg that plays the sound while the picture plays.
    sound: Option<Child>,
}

impl VideoView {
    pub fn new(path: PathBuf, ffmpeg: Option<PathBuf>, name: String, fallback_duration: f32) -> Self {
        let pr = ffmpeg.as_deref().and_then(|f| probe(f, &path));
        let (duration, size, fps, has_audio) = match &pr {
            Some(p) => (p.duration, p.size, p.fps, p.has_audio),
            None => (fallback_duration, [0, 0], None, false),
        };
        let max_w = 1280u32;
        let preview_size = if size[0] > max_w {
            let h = (size[1] as f32 * max_w as f32 / size[0] as f32) as u32 / 2 * 2;
            [max_w, h.max(2)]
        } else {
            size
        };
        Self {
            path,
            ffmpeg: if pr.is_some() { ffmpeg } else { None },
            duration,
            size,
            fps,
            trim: (0.0, duration),
            pos: 0.0,
            name,
            saved: None,
            preview_size,
            frame: Arc::new(Mutex::new(None)),
            tex: None,
            generation: Arc::new(AtomicU64::new(0)),
            play_started: None,
            has_audio,
            sound: None,
        }
    }

    pub fn can_preview(&self) -> bool {
        self.ffmpeg.is_some() && self.preview_size[0] > 0
    }

    pub fn is_playing(&self) -> bool {
        self.play_started.is_some()
    }

    pub fn is_trimmed(&self) -> bool {
        self.trim.0 > 0.05 || self.trim.1 < self.duration - 0.05
    }

    /// Takes a freshly decoded frame, if any.
    pub fn take_frame(&self) -> Option<ColorImage> {
        self.frame.lock().ok()?.take()
    }

    /// Advances the play position; returns true while playing.
    pub fn tick(&mut self) -> bool {
        if let Some((start, from)) = self.play_started {
            self.pos = from + start.elapsed().as_secs_f32();
            if self.pos >= self.trim.1 {
                self.pos = self.trim.1;
                // Let the sound play out; it ends by itself right after.
                self.generation.fetch_add(1, Ordering::SeqCst);
                self.play_started = None;
                return false;
            }
            return true;
        }
        false
    }

    pub fn stop(&mut self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.play_started = None;
        self.stop_sound();
    }

    fn stop_sound(&mut self) {
        if let Some(mut c) = self.sound.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    /// Plays the sound from `from` for `secs` seconds, next to the picture.
    fn play_sound(&mut self, from: f32, secs: f32) {
        self.stop_sound();
        let Some(ffmpeg) = &self.ffmpeg else { return };
        if !self.has_audio {
            return;
        }
        self.sound = Command::new(ffmpeg)
            // No -re here: the sound card sets the pace (see sound_output_args).
            .args(["-hide_banner", "-loglevel", "error", "-protocol_whitelist", "file"])
            .args(["-format_whitelist", INPUT_FORMATS])
            // Read only up to the end of the trim; the padding after it is silence.
            .args(["-ss", &format!("{from:.3}"), "-t", &format!("{:.3}", secs.max(0.05)), "-i"])
            .arg(&self.path)
            .args(["-t", &format!("{:.3}", secs.max(0.05) + 0.3), "-vn"])
            .args(crate::platform::sound_output_args())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok();
    }

    pub fn play(&mut self, ctx: &egui::Context) {
        if !self.can_preview() {
            return;
        }
        if self.pos >= self.trim.1 - 0.05 || self.pos < self.trim.0 {
            self.pos = self.trim.0;
        }
        self.play_started = Some((Instant::now(), self.pos));
        self.decode(ctx, self.pos, Some(self.trim.1 - self.pos));
        self.play_sound(self.pos, self.trim.1 - self.pos);
    }

    /// Shows the single frame at `t`.
    pub fn seek(&mut self, ctx: &egui::Context, t: f32) {
        self.stop();
        self.pos = t.clamp(0.0, self.duration);
        self.decode(ctx, self.pos, None);
    }

    fn decode(&self, ctx: &egui::Context, from: f32, play_for: Option<f32>) {
        let Some(ffmpeg) = self.ffmpeg.clone() else { return };
        let gen_id = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let generation = self.generation.clone();
        let slot = self.frame.clone();
        let path = self.path.clone();
        let [w, h] = self.preview_size;
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let mut cmd = Command::new(ffmpeg);
            cmd.args(["-hide_banner", "-loglevel", "error"]);
            if play_for.is_some() {
                cmd.arg("-re");
            }
            cmd.args(["-protocol_whitelist", "file", "-format_whitelist", INPUT_FORMATS, "-ss", &format!("{from:.3}"), "-i"]).arg(&path);
            match play_for {
                Some(d) => {
                    cmd.args(["-t", &format!("{:.3}", d.max(0.05))]);
                }
                None => {
                    cmd.args(["-frames:v", "1"]);
                }
            }
            cmd.args(["-an", "-vf", &format!("scale={w}:{h}"), "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null());
            let Ok(mut child) = cmd.spawn() else { return };
            let Some(mut out) = child.stdout.take() else { return };
            let mut buf = vec![0u8; (w * h * 4) as usize];
            loop {
                if generation.load(Ordering::SeqCst) != gen_id {
                    break;
                }
                if out.read_exact(&mut buf).is_err() {
                    break;
                }
                if generation.load(Ordering::SeqCst) != gen_id {
                    break;
                }
                let img = ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &buf);
                if let Ok(mut s) = slot.lock() {
                    *s = Some(img);
                }
                ctx.request_repaint();
            }
            let _ = child.kill();
            let _ = child.wait();
        });
    }

    /// Writes the chosen part of the video to `dest` in the chosen format. Blocks; run it on a thread.
    pub fn export(path: PathBuf, ffmpeg: Option<PathBuf>, o: ExportOptions, dest: PathBuf) -> Result<PathBuf, String> {
        if let Some(d) = dest.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let Some(ff) = ffmpeg else {
            return std::fs::copy(&path, &dest).map(|_| dest).map_err(|e| format!("Could not save the video: {e}"));
        };
        if o.trim.is_none() && o.same_format {
            return std::fs::copy(&path, &dest).map(|_| dest).map_err(|e| format!("Could not save the video: {e}"));
        }
        let mut cmd = Command::new(ff);
        cmd.args(["-hide_banner", "-loglevel", "error", "-y", "-protocol_whitelist", "file"]);
        cmd.args(["-format_whitelist", INPUT_FORMATS]);
        if let Some((s, _)) = o.trim {
            cmd.args(["-ss", &format!("{s:.3}")]);
        }
        cmd.arg("-i").arg(&path);
        if let Some((s, e)) = o.trim {
            cmd.args(["-t", &format!("{:.3}", (e - s).max(0.1))]);
        }
        match o.format {
            VideoFormat::Gif => {
                let scale = o.gif_scale.clamp(0.1, 1.0);
                let filter = format!(
                    "fps={},scale=trunc(iw*{scale:.3}/2)*2:-2:flags=lanczos,split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=sierra2_4a",
                    o.gif_fps.clamp(1, 30)
                );
                cmd.args(["-an", "-filter_complex", &filter, "-loop", "0"]);
            }
            VideoFormat::Mp4 | VideoFormat::Mov => {
                if o.trim.is_none() {
                    // Only the container changes: no re-encoding needed.
                    cmd.args(["-c", "copy"]);
                } else {
                    cmd.args(crate::platform::video_encoder_args(
                        crate::settings::Quality::High.video_q(),
                        o.size[0],
                        o.size[1],
                        o.fps,
                    ))
                        .args(crate::platform::audio_encoder_args());
                }
                if o.format == VideoFormat::Mp4 {
                    cmd.args(["-movflags", "+faststart"]);
                }
            }
        }
        let ok = cmd
            .arg(&dest)
            .stdin(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok { Ok(dest) } else { Err("Could not save the video.".into()) }
    }
}

/// How a video is saved.
pub struct ExportOptions {
    pub trim: Option<(f32, f32)>,
    pub format: VideoFormat,
    pub gif_fps: u32,
    /// Size of the GIF relative to the recording (0.1–1.0).
    pub gif_scale: f32,
    /// The recording is already in the wanted format.
    pub same_format: bool,
    /// Size of the video in pixels, for the encoder's bit rate.
    pub size: [u32; 2],
    /// Frame rate of the video.
    pub fps: u32,
}

impl Drop for VideoView {
    fn drop(&mut self) {
        // Ends the picture thread and the sound.
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fps_is_read_from_the_stream_line() {
        let l = "  Stream #0:0[0x1](und): Video: h264 (High) (avc1 / 0x31637661), yuv420p(progressive), 1920x1080, 1200 kb/s, 29.97 fps, 29.97 tbr, 15360 tbn (default)";
        assert_eq!(parse_fps(l), Some(29.97));
        assert_eq!(parse_fps("Video: h264, 1920x1080, 30 fps, 30 tbr"), Some(30.0));
        assert_eq!(parse_fps("Video: h264, 1920x1080, 30 tbr"), None);
    }
}
