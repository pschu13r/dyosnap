//! The system-specific parts of screen recording: where ffmpeg lives, which screens
//! and microphones it can record (AVFoundation), and macOS's own `screencapture -v`
//! as the recorder when ffmpeg is missing.

use crate::platform::Devices;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// Ends the screen sharing that `capture_all_screens` started on systems that need one
/// (Linux). Nothing to do on macOS.
pub fn end_screen_session() {}

/// Places to look for ffmpeg, in order.
pub fn ffmpeg_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    // The one shipped next to the app's own binary (DyoSnap.app/Contents/MacOS/ffmpeg).
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.to_path_buf())) {
        candidates.push(dir.join("ffmpeg"));
    }
    // Run from source: the one scripts/build-ffmpeg.sh made, else one from a package manager.
    if cfg!(debug_assertions) {
        candidates.push(concat!(env!("CARGO_MANIFEST_DIR"), "/build/ffmpeg/bin/ffmpeg").into());
    }
    candidates.extend::<[PathBuf; 3]>([
        "/opt/homebrew/bin/ffmpeg".into(),
        "/usr/local/bin/ffmpeg".into(),
        "/opt/local/bin/ffmpeg".into(),
    ]);
    if let Some(path) = std::env::var_os("PATH") {
        // Only absolute entries: "" or "." would run an ffmpeg from the current folder.
        candidates.extend(std::env::split_paths(&path).filter(|d| d.is_absolute()).map(|d| d.join("ffmpeg")));
    }
    candidates
}

/// Asks ffmpeg which screens and microphones exist. Takes well under a second.
pub fn list_devices(ffmpeg: &Path) -> Devices {
    let out = Command::new(ffmpeg)
        .args(["-hide_banner", "-f", "avfoundation", "-list_devices", "true", "-i", ""])
        .stdin(Stdio::null())
        .output();
    let mut devices = Devices::default();
    let Ok(out) = out else { return devices };
    let text = String::from_utf8_lossy(&out.stderr);
    let mut in_audio = false;
    for line in text.lines() {
        if line.contains("video devices:") {
            in_audio = false;
            continue;
        }
        if line.contains("audio devices:") {
            in_audio = true;
            continue;
        }
        // Lines look like: "[AVFoundation indev @ 0x..] [1] Capture screen 0"
        let Some(pos) = line.rfind("] [") else { continue };
        let rest = &line[pos + 3..];
        let Some(close) = rest.find(']') else { continue };
        let Ok(idx) = rest[..close].parse::<usize>() else { continue };
        let name = rest[close + 1..].trim().to_string();
        if in_audio {
            devices.audio_inputs.push(name);
        } else if let Some(n) = name.strip_prefix("Capture screen ").and_then(|n| n.trim().parse::<usize>().ok()) {
            devices.screens.push((n, idx));
            if n == 0 {
                devices.screen_index = Some(idx);
            }
        }
    }
    devices
}

/// ffmpeg arguments that select the screen (and microphone) to record, up to and including `-i`.
/// Encode video with Apple's hardware H.264 encoder. `quality` goes from 1 to 100.
pub fn video_encoder_args(quality: u32) -> Vec<String> {
    let q = quality.clamp(1, 100).to_string();
    ["-c:v", "h264_videotoolbox", "-q:v", &q, "-pix_fmt", "yuv420p"].iter().map(|s| s.to_string()).collect()
}

/// Encode sound with Apple's AAC encoder.
pub fn audio_encoder_args() -> Vec<String> {
    ["-c:a", "aac_at", "-b:a", "128k"].iter().map(|s| s.to_string()).collect()
}

/// Output options that make ffmpeg play sound on the default speakers.
///
/// ffmpeg's audiotoolbox output holds just two buffers of one packet each. With the
/// usual ~20 ms packets they run dry and the sound crackles, so the sound goes out in
/// 100 ms blocks. Writing blocks until a buffer is free, so the sound card sets the pace.
/// On exit it drops what is still queued, so a bit of silence goes after the sound.
pub fn sound_output_args() -> Vec<String> {
    ["-af", "aresample=48000,apad=pad_dur=0.3,asetnsamples=n=4800:p=0", "-f", "audiotoolbox", "-"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

pub fn ffmpeg_input_args(screen_index: usize, audio: Option<&str>, fps: u32, cursor: bool, clicks: bool) -> Vec<String> {
    let input = match audio {
        Some(a) => format!("{screen_index}:{a}"),
        None => format!("{screen_index}:none"),
    };
    [
        "-f",
        "avfoundation",
        "-capture_cursor",
        if cursor { "1" } else { "0" },
        "-capture_mouse_clicks",
        if clicks { "1" } else { "0" },
        "-framerate",
        &fps.to_string(),
        "-pixel_format",
        "nv12",
        "-i",
        &input,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// The recorder used without ffmpeg: `screencapture -v` for a rectangle in global points.
pub fn fallback_record_command(rect_global: [f32; 4], cursor: bool, clicks: bool, audio: bool, out: &Path) -> Command {
    let mut cmd = Command::new("/usr/sbin/screencapture");
    let [gx, gy, gw, gh] = rect_global;
    cmd.args(["-v", "-x", "-R"]).arg(format!(
        "{},{},{},{}",
        gx.round() as i64,
        gy.round() as i64,
        gw.round() as i64,
        gh.round() as i64
    ));
    if cursor {
        cmd.arg("-C");
    }
    if clicks {
        cmd.arg("-k");
    }
    if audio {
        cmd.arg("-g");
    }
    cmd.arg(out);
    cmd
}

/// Video file type the fallback recorder writes.
pub const FALLBACK_EXT: &str = "mov";

/// Asks the fallback recorder to stop and write its file.
pub fn stop_fallback(child: &mut Child) {
    // screencapture stops on a key press or Ctrl-C.
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(b"\n");
    }
    let _ = Command::new("/bin/kill").args(["-INT", &child.id().to_string()]).status();
}
