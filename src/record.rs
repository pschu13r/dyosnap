//! Screen video recording. Uses ffmpeg when it is installed (frame rate, quality,
//! pause and audio work then), otherwise the system's own recorder (see `platform`).
//! Recording runs in a separate process; the app only shows a timer.

use crate::platform::{self, Devices};
use crate::settings::Quality;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub fn find_ffmpeg() -> Option<PathBuf> {
    platform::ffmpeg_candidates().into_iter().find(|p| p.is_file())
}

#[derive(Clone, Debug)]
pub struct RecordOptions {
    /// Area in points (x, y, w, h), relative to the top-left of the screen being recorded.
    pub rect_pt: [f32; 4],
    /// The same area in global points (used by the macOS recorder fallback).
    pub rect_global: [f32; 4],
    /// Which screen (0 = main), in the order of `displays::list`.
    pub screen: usize,
    /// Screen pixels per point.
    pub scale: f32,
    pub fps: u32,
    pub quality: Quality,
    pub cursor: bool,
    pub clicks: bool,
    pub audio: Option<String>,
}

enum Backend {
    Ffmpeg { exe: PathBuf, screen_index: usize },
    ScreenCapture,
}

pub struct Recorder {
    opts: RecordOptions,
    backend: Backend,
    child: Option<Child>,
    segments: Vec<PathBuf>,
    started: Instant,
    paused_total: Duration,
    paused_at: Option<Instant>,
    pub error: Option<String>,
    log: PathBuf,
}

impl Recorder {
    pub fn start(opts: RecordOptions, ffmpeg: Option<PathBuf>, devices: &Devices) -> Result<Self, String> {
        let screen_index = devices
            .screens
            .iter()
            .find(|(n, _)| *n == opts.screen)
            .map(|(_, i)| *i)
            .or(if opts.screen == 0 { devices.screen_index } else { None });
        let backend = match (ffmpeg, screen_index) {
            (Some(exe), Some(screen_index)) => Backend::Ffmpeg { exe, screen_index },
            _ => Backend::ScreenCapture,
        };
        let mut rec = Recorder {
            opts,
            backend,
            child: None,
            segments: Vec::new(),
            started: Instant::now(),
            paused_total: Duration::ZERO,
            paused_at: None,
            error: None,
            log: platform::temp_file("rec", "log"),
        };
        rec.start_segment()?;
        Ok(rec)
    }

    pub fn can_pause(&self) -> bool {
        matches!(self.backend, Backend::Ffmpeg { .. })
    }

    pub fn is_paused(&self) -> bool {
        self.paused_at.is_some()
    }

    pub fn elapsed(&self) -> Duration {
        let now = self.paused_at.unwrap_or_else(Instant::now);
        now.saturating_duration_since(self.started).saturating_sub(self.paused_total)
    }

    fn log_sink(&self) -> Stdio {
        std::fs::File::create(&self.log).map(Stdio::from).unwrap_or_else(|_| Stdio::null())
    }

    fn start_segment(&mut self) -> Result<(), String> {
        let o = &self.opts;
        let [x, y, w, h] = o.rect_pt;
        let child = match &self.backend {
            Backend::Ffmpeg { exe, screen_index } => {
                let seg = platform::temp_file("rec", "mp4");
                let s = o.scale.max(1.0);
                let even = |v: f32| ((v * s).round() as i64 / 2 * 2).max(2);
                let (px, py, pw, ph) = (
                    ((x * s).round() as i64).max(0),
                    ((y * s).round() as i64).max(0),
                    even(w),
                    even(h),
                );
                let mut filter = format!("crop={pw}:{ph}:{px}:{py}");
                let (mut out_w, mut out_h) = (pw as u32, ph as u32);
                if o.quality == Quality::Low && pw >= 640 {
                    filter.push_str(",scale=trunc(iw/4)*2:trunc(ih/4)*2");
                    (out_w, out_h) = (out_w / 2, out_h / 2);
                }
                let mut cmd = Command::new(exe);
                cmd.args(["-hide_banner", "-loglevel", "error", "-y"])
                    .args(platform::ffmpeg_input_args(*screen_index, o.audio.as_deref(), o.fps, o.cursor, o.clicks))
                    .args(["-vf", &filter])
                    .args(["-r", &o.fps.to_string()])
                    .args(platform::video_encoder_args(o.quality.video_q(), out_w, out_h, o.fps));
                if o.audio.is_some() {
                    cmd.args(platform::audio_encoder_args());
                }
                cmd.arg(&seg);
                self.segments.push(seg);
                cmd.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(self.log_sink()).spawn()
            }
            Backend::ScreenCapture => {
                let seg = platform::temp_file("rec", platform::FALLBACK_EXT);
                let _ = (x, y, w, h);
                let mut cmd = platform::fallback_record_command(o.rect_global, o.cursor, o.clicks, o.audio.is_some(), &seg);
                self.segments.push(seg);
                cmd.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(self.log_sink()).spawn()
            }
        };
        match child {
            Ok(c) => {
                self.child = Some(c);
                Ok(())
            }
            Err(e) => Err(format!("Could not start recording: {e}")),
        }
    }

    /// Checks whether the recorder process died on its own (e.g. no permission).
    pub fn poll_failure(&mut self) -> Option<String> {
        let child = self.child.as_mut()?;
        match child.try_wait() {
            Ok(Some(status)) => {
                let msg: String = std::fs::read_to_string(&self.log)
                    .unwrap_or_default()
                    .lines()
                    .filter(|l| !l.starts_with("objc[") && !l.contains("pixel format"))
                    .collect::<Vec<_>>()
                    .join(" ");
                self.child = None;
                let msg = if msg.trim().is_empty() {
                    format!("Recording stopped unexpectedly ({status}). {}", platform::SCREEN_PERMISSION_HINT)
                } else {
                    format!("Recording stopped: {}", msg.trim())
                };
                Some(msg)
            }
            _ => None,
        }
    }

    fn stop_segment(&mut self) {
        let Some(mut child) = self.child.take() else { return };
        match self.backend {
            Backend::Ffmpeg { .. } => {
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(b"q");
                    let _ = stdin.flush();
                }
            }
            Backend::ScreenCapture => platform::stop_fallback(&mut child),
        }
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(30))
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                }
            }
        }
    }

    pub fn toggle_pause(&mut self) {
        if !self.can_pause() {
            return;
        }
        if let Some(at) = self.paused_at.take() {
            self.paused_total += at.elapsed();
            if let Err(e) = self.start_segment() {
                self.error = Some(e);
            }
        } else {
            self.stop_segment();
            self.paused_at = Some(Instant::now());
        }
    }

    /// Stops and throws the recording away.
    pub fn cancel(mut self) {
        self.stop_segment();
        platform::end_screen_session();
        for s in &self.segments {
            let _ = std::fs::remove_file(s);
        }
    }

    /// Stops and writes the video into `dest_dir`. Blocks for a moment.
    pub fn finish(mut self, dest_stem_path: PathBuf) -> Result<PathBuf, String> {
        self.stop_segment();
        platform::end_screen_session();
        let segments: Vec<PathBuf> = self.segments.iter().filter(|p| p.exists()).cloned().collect();
        if segments.is_empty() {
            return Err(format!("No video was recorded. {}", platform::SCREEN_PERMISSION_HINT));
        }
        let ext = segments[0].extension().and_then(|e| e.to_str()).unwrap_or("mp4").to_string();
        let dest = dest_stem_path.with_extension(&ext);
        if let Some(d) = dest.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if segments.len() == 1 {
            move_file(&segments[0], &dest)?;
            return Ok(dest);
        }
        // Several pieces (after pause): join them without re-encoding.
        let Backend::Ffmpeg { exe, .. } = &self.backend else {
            move_file(&segments[0], &dest)?;
            return Ok(dest);
        };
        let list = platform::temp_file("list", "txt");
        let body: String = segments
            .iter()
            .map(|p| format!("file '{}'\n", p.display().to_string().replace('\'', "'\\''")))
            .collect();
        std::fs::write(&list, body).map_err(|e| e.to_string())?;
        // `-safe 0` stays: the segment paths are absolute, which safe mode rejects. The list is
        // ours, so this is fine; `-protocol_whitelist file` keeps it to local files.
        let ok = Command::new(exe)
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "concat", "-safe", "0"])
            .args(["-protocol_whitelist", "file", "-i"])
            .arg(&list)
            .args(["-c", "copy"])
            .arg(&dest)
            .stdin(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        let _ = std::fs::remove_file(&list);
        if ok {
            for s in &segments {
                let _ = std::fs::remove_file(s);
            }
            Ok(dest)
        } else {
            Err("Could not join the recorded pieces.".into())
        }
    }
}

fn move_file(from: &Path, to: &Path) -> Result<(), String> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(from);
    Ok(())
}

pub fn format_elapsed(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{:02}:{:02}", s / 60, s % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs screen-recording permission; run with `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn record_pause_finish() {
        let ff = find_ffmpeg().expect("ffmpeg");
        // Linux records from the shared screen, which the still picture starts sharing.
        #[cfg(target_os = "linux")]
        assert!(!platform::capture_all_screens(1, false).is_empty(), "no screen shared");
        let devices = platform::list_devices(&ff);
        assert!(devices.screen_index.is_some());
        let opts = RecordOptions {
            rect_pt: [50.0, 50.0, 300.0, 200.0],
            rect_global: [50.0, 50.0, 300.0, 200.0],
            screen: 0,
            scale: 2.0,
            fps: 15,
            quality: Quality::High,
            cursor: true,
            clicks: true,
            audio: None,
        };
        let mut r = Recorder::start(opts, Some(ff), &devices).unwrap();
        std::thread::sleep(Duration::from_millis(1500));
        r.toggle_pause();
        std::thread::sleep(Duration::from_millis(500));
        r.toggle_pause();
        std::thread::sleep(Duration::from_millis(1500));
        assert!(r.poll_failure().is_none());
        let out = r.finish(std::env::temp_dir().join("dyosnap-test-rec")).unwrap();
        let meta = std::fs::metadata(&out).unwrap();
        assert!(meta.len() > 1000, "file too small");
        println!("recorded {} ({} bytes)", out.display(), meta.len());
    }
}
