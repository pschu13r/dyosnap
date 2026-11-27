//! Screen recording on Wayland. An app may not read the screen itself, so:
//!
//! 1. The ScreenCast portal asks the person which screens to share (once: the answer is
//!    kept as a restore token) and hands out a PipeWire stream per screen. KDE can also
//!    share the whole desktop as one stream.
//! 2. A PipeWire thread per stream keeps its newest frame.
//! 3. The first frames are the still pictures the area is chosen on, one per screen.
//! 4. While recording, a writer thread cuts the recorded screen out of its stream and
//!    feeds it at a steady rate into a named pipe that ffmpeg reads as raw video; ffmpeg
//!    crops and encodes as on macOS.
//!
//! The session ends with `end_screen_session`, or when DyoSnap quits.

use crate::platform::{Devices, temp_file};
use ashpd::desktop::PersistMode;
use ashpd::desktop::screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType};
use pipewire as pw;
use std::io::Write;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------- frames

/// One picture of the shared screen, rows packed without gaps.
#[derive(Clone)]
struct Frame {
    width: u32,
    height: u32,
    /// ffmpeg's name for the pixel layout ("bgr0", "rgba", ...).
    pix_fmt: &'static str,
    data: Arc<Vec<u8>>,
    /// Counts up with every frame any stream sends (see `SEQ`).
    seq: u64,
}

/// One shared stream: a screen, or the whole desktop.
struct Source {
    frame: Arc<Mutex<Option<Frame>>>,
    stop: pw::channel::Sender<()>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// What the stream shows, in points on the desktop: x, y, width, height.
    rect: Option<(i32, i32, i32, i32)>,
}

struct Cast {
    session: ashpd::desktop::Session<Screencast>,
    sources: Vec<Source>,
}

static CAST: Mutex<Option<Cast>> = Mutex::new(None);
/// Numbers the frames of all streams in the order they come.
static SEQ: AtomicU64 = AtomicU64::new(0);
/// Whether DyoSnap's main window is on screen, and the number of the newest frame when
/// it last closed. Frames up to that one may still show it (see `feed`).
static WINDOW_OPEN: AtomicBool = AtomicBool::new(false);
static CLOSED_AT_SEQ: AtomicU64 = AtomicU64::new(0);
/// Description → PulseAudio source name, filled by `list_devices`.
static SOURCES: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

/// Where the restore token lives, so the person picks the screens only once. Up to v0.4.2
/// one screen was shared (`screencast-token`); the new name makes the portal ask once more,
/// now for all screens.
fn token_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("DyoSnap").join("screencast-token-screens"))
}

/// Starts sharing the screens: asks the portal (a dialog the first time), then connects
/// to the PipeWire streams. Blocks until every stream sent its first frame.
fn start_cast(cursor: bool) -> Result<(), String> {
    end_screen_session();
    super::portal::register();
    let token = token_path().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| t.trim().to_string());
    let res: Result<_, ashpd::Error> = pollster::block_on(async {
        let proxy = Screencast::new().await?;
        let session = proxy.create_session(Default::default()).await?;
        let modes = proxy.available_cursor_modes().await.unwrap_or(CursorMode::HIDDEN);
        let cursor_mode = if cursor && modes.contains(CursorMode::EMBEDDED) { CursorMode::EMBEDDED } else { CursorMode::HIDDEN };
        proxy
            .select_sources(
                &session,
                SelectSourcesOptions::default()
                    .set_cursor_mode(cursor_mode)
                    .set_sources(SourceType::MONITOR)
                    .set_multiple(true)
                    .set_persist_mode(PersistMode::ExplicitlyRevoked)
                    .set_restore_token(token.as_deref().filter(|t| !t.is_empty())),
            )
            .await?
            .response()?;
        let streams = proxy.start(&session, None, Default::default()).await?.response()?;
        // One PipeWire connection per stream: each runs in its own thread.
        let mut fds = Vec::new();
        for _ in streams.streams() {
            fds.push(proxy.open_pipe_wire_remote(&session, Default::default()).await?);
        }
        Ok((session, streams, fds))
    });
    let (session, streams, fds) = match res {
        Ok(v) => v,
        Err(e) => return Err(format!("{e}")),
    };
    if let (Some(t), Some(p)) = (streams.restore_token(), token_path()) {
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(p, t);
    }
    if streams.streams().is_empty() {
        let _ = pollster::block_on(session.close());
        return Err("no screen was shared".into());
    }
    let mut sources = Vec::new();
    for (stream, fd) in streams.streams().iter().zip(fds) {
        let node = stream.pipe_wire_node_id();
        let rect = match (stream.position(), stream.size()) {
            (Some((x, y)), Some((w, h))) => Some((x, y, w, h)),
            _ => None,
        };
        log::info!(target: "dyosnap", "shared stream {node}: {rect:?}");
        let frame: Arc<Mutex<Option<Frame>>> = Arc::new(Mutex::new(None));
        let (stop, stop_rx) = pw::channel::channel::<()>();
        let f = frame.clone();
        let thread = std::thread::Builder::new()
            .name("pipewire".into())
            .spawn(move || {
                if let Err(e) = run_stream(fd, node, f, stop_rx) {
                    log::warn!(target: "dyosnap", "pipewire: {e}");
                }
            })
            .map_err(|e| e.to_string())?;
        sources.push(Source { frame, stop, thread: Some(thread), rect });
    }
    let frames: Vec<_> = sources.iter().map(|s| s.frame.clone()).collect();
    CLOSED_AT_SEQ.store(0, Ordering::Relaxed);
    *CAST.lock().map_err(|e| e.to_string())? = Some(Cast { session, sources });
    // The compositor sends a frame right away.
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if frames.iter().all(|f| f.lock().map(|f| f.is_some()).unwrap_or(false)) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    end_screen_session();
    Err("a shared screen sent no picture".into())
}

/// Stops sharing the screen. Safe to call when nothing is shared.
pub fn end_screen_session() {
    let cast = CAST.lock().ok().and_then(|mut c| c.take());
    if let Some(mut c) = cast {
        for s in &mut c.sources {
            let _ = s.stop.send(());
            if let Some(t) = s.thread.take() {
                let _ = t.join();
            }
        }
        let _ = pollster::block_on(c.session.close());
    }
}

/// Called by main.rs when the main window opens and after it closed. A recording starts
/// with the first frame after the window is gone, so the video never shows the area
/// selection.
pub fn main_window_open(open: bool) {
    if !open {
        CLOSED_AT_SEQ.store(SEQ.load(Ordering::Relaxed), Ordering::Relaxed);
    }
    WINDOW_OPEN.store(open, Ordering::Relaxed);
}

/// Stops the streams that don't show the screen `bounds`: a recording needs only one, and
/// every running stream costs the compositor and DyoSnap a copy of each frame. The
/// screens stay listed (`shared_rects`) so the recording still finds its screen.
pub fn keep_only_screen(bounds: egui::Rect) {
    let Some(cut) = locate(bounds, &super::displays()) else { return };
    let mut stopped = Vec::new();
    if let Ok(mut cast) = CAST.lock() {
        let Some(c) = cast.as_mut() else { return };
        for (i, s) in c.sources.iter_mut().enumerate() {
            if i == cut.source {
                continue;
            }
            let _ = s.stop.send(());
            if let Some(t) = s.thread.take() {
                stopped.push((t, s.frame.clone()));
            }
        }
    }
    let n = stopped.len();
    for (t, frame) in stopped {
        let _ = t.join();
        // Free the last picture; the stream is gone, so none comes after it.
        if let Ok(mut f) = frame.lock() {
            *f = None;
        }
    }
    if n > 0 {
        log::info!(target: "dyosnap", "recording screen at {:?}: stopped {n} other stream(s)", bounds.min);
    }
}

/// What the shared streams show, in points on the desktop. Empty while nothing is shared,
/// and when the portal did not say (then the one stream is the first screen).
pub(super) fn shared_rects() -> Vec<egui::Rect> {
    let Ok(cast) = CAST.lock() else { return Vec::new() };
    let Some(c) = cast.as_ref() else { return Vec::new() };
    c.sources
        .iter()
        .filter_map(|s| s.rect)
        .map(|(x, y, w, h)| egui::Rect::from_min_size(egui::pos2(x as f32, y as f32), egui::vec2(w as f32, h as f32)))
        .collect()
}

fn latest_frame(source: usize) -> Option<Frame> {
    let cast = CAST.lock().ok()?;
    cast.as_ref()?.sources.get(source)?.frame.lock().ok()?.clone()
}

/// The pixels of a screen inside one stream's frames.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Cut {
    source: usize,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

/// Where the screen `bounds` (points) is in the shared streams, if one shows it.
/// `screens` are all screens, as `displays()` lists them.
fn locate(bounds: egui::Rect, screens: &[crate::platform::Display]) -> Option<Cut> {
    let desktop = screens.iter().fold(egui::Rect::NOTHING, |a, d| a.union(d.bounds));
    let rects: Vec<Option<egui::Rect>> = {
        let cast = CAST.lock().ok()?;
        let c = cast.as_ref()?;
        c.sources
            .iter()
            .map(|s| s.rect.map(|(x, y, w, h)| egui::Rect::from_min_size(egui::pos2(x as f32, y as f32), egui::vec2(w as f32, h as f32))))
            .collect()
    };
    for (i, r) in rects.iter().enumerate() {
        // A stopped stream (see `keep_only_screen`) has no frame.
        let Some(f) = latest_frame(i) else { continue };
        let r = match r {
            Some(r) => *r,
            // Without a position from the portal: a stream shaped like the whole desktop is
            // the whole desktop (KDE's "full workspace"); else the only stream is the first screen.
            None => {
                let ratio = f.width as f32 / f.height.max(1) as f32;
                let fits = |r: egui::Rect| r.height() > 0.0 && (r.width() / r.height() - ratio).abs() < 0.01;
                if fits(desktop) {
                    desktop
                } else if rects.len() == 1 && screens.first().is_some_and(|d| d.bounds == bounds) {
                    bounds
                } else {
                    continue;
                }
            }
        };
        if let Some(cut) = cut_in(bounds, r, f.width, f.height) {
            return Some(Cut { source: i, ..cut });
        }
    }
    None
}

/// The pixels of `bounds` in a stream that shows `stream` (points) as `fw` × `fh` pixels.
fn cut_in(bounds: egui::Rect, stream: egui::Rect, fw: u32, fh: u32) -> Option<Cut> {
    if !stream.expand(1.0).contains_rect(bounds) || stream.width() < 1.0 {
        return None;
    }
    let s = fw as f32 / stream.width();
    let px = |v: f32, max: u32| (v * s).round().clamp(0.0, max as f32) as u32;
    let (x, y) = (px(bounds.min.x - stream.min.x, fw), px(bounds.min.y - stream.min.y, fh));
    let (w, h) = (px(bounds.width(), fw - x), px(bounds.height(), fh - y));
    (w > 0 && h > 0).then_some(Cut { source: 0, x, y, w, h })
}

/// The cut's rows out of a frame, packed.
fn cut_rows(f: &Frame, c: Cut) -> Option<Vec<u8>> {
    if c.x + c.w > f.width || c.y + c.h > f.height {
        return None;
    }
    if c.x == 0 && c.y == 0 && c.w == f.width && c.h == f.height {
        return Some((*f.data).clone());
    }
    let row = f.width as usize * 4;
    let mut out = Vec::with_capacity(c.w as usize * c.h as usize * 4);
    for y in c.y..c.y + c.h {
        let start = y as usize * row + c.x as usize * 4;
        out.extend_from_slice(&f.data[start..start + c.w as usize * 4]);
    }
    Some(out)
}

/// The PipeWire side: connect to the portal's remote and keep the newest frame.
fn run_stream(fd: OwnedFd, node: u32, frame: Arc<Mutex<Option<Frame>>>, stop: pw::channel::Receiver<()>) -> Result<(), String> {
    use pw::spa;
    use pw::spa::param::video::VideoFormat;
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None).map_err(|e| e.to_string())?;
    let context = pw::context::ContextRc::new(&mainloop, None).map_err(|e| e.to_string())?;
    let core = context.connect_fd_rc(fd, None).map_err(|e| e.to_string())?;
    let ml = mainloop.clone();
    let _stop = stop.attach(mainloop.loop_(), move |_| ml.quit());

    struct Format {
        info: spa::param::video::VideoInfoRaw,
    }
    let stream = pw::stream::StreamBox::new(
        &core,
        "dyosnap-screen",
        pw::properties::properties! {
            *pw::keys::MEDIA_TYPE => "Video",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Screen",
        },
    )
    .map_err(|e| e.to_string())?;
    let _listener = stream
        .add_local_listener_with_user_data(Format { info: Default::default() })
        .param_changed(|_, fmt, id, param| {
            let Some(param) = param else { return };
            if id != pw::spa::param::ParamType::Format.as_raw() {
                return;
            }
            let _ = fmt.info.parse(param);
        })
        .process(move |stream, fmt| {
            let seq = SEQ.fetch_add(1, Ordering::Relaxed) + 1;
            let Some(mut buffer) = stream.dequeue_buffer() else { return };
            let datas = buffer.datas_mut();
            let Some(d) = datas.first_mut() else { return };
            let (w, h) = (fmt.info.size().width, fmt.info.size().height);
            let pix_fmt = match fmt.info.format() {
                VideoFormat::BGRx => "bgr0",
                VideoFormat::BGRA => "bgra",
                VideoFormat::RGBx => "rgb0",
                VideoFormat::RGBA => "rgba",
                _ => return,
            };
            let stride = d.chunk().stride().max(0) as usize;
            let offset = d.chunk().offset() as usize;
            let size = d.chunk().size() as usize;
            let Some(bytes) = d.data() else { return };
            let row = w as usize * 4;
            let stride = if stride == 0 { row } else { stride };
            if w == 0 || h == 0 || size == 0 || bytes.len() < offset + stride * (h as usize - 1) + row {
                return; // an empty or partial buffer (for example only the cursor moved)
            }
            let mut packed = Vec::with_capacity(row * h as usize);
            for y in 0..h as usize {
                let start = offset + y * stride;
                packed.extend_from_slice(&bytes[start..start + row]);
            }
            if let Ok(mut f) = frame.lock() {
                *f = Some(Frame { width: w, height: h, pix_fmt, data: Arc::new(packed), seq });
            }
        })
        .register()
        .map_err(|e| e.to_string())?;

    let obj = pw::spa::pod::object!(
        pw::spa::utils::SpaTypes::ObjectParamFormat,
        pw::spa::param::ParamType::EnumFormat,
        pw::spa::pod::property!(pw::spa::param::format::FormatProperties::MediaType, Id, pw::spa::param::format::MediaType::Video),
        pw::spa::pod::property!(pw::spa::param::format::FormatProperties::MediaSubtype, Id, pw::spa::param::format::MediaSubtype::Raw),
        pw::spa::pod::property!(
            pw::spa::param::format::FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            VideoFormat::BGRx,
            VideoFormat::BGRx,
            VideoFormat::BGRA,
            VideoFormat::RGBx,
            VideoFormat::RGBA
        ),
        pw::spa::pod::property!(
            pw::spa::param::format::FormatProperties::VideoSize,
            Choice,
            Range,
            Rectangle,
            pw::spa::utils::Rectangle { width: 1920, height: 1080 },
            pw::spa::utils::Rectangle { width: 1, height: 1 },
            pw::spa::utils::Rectangle { width: 16384, height: 16384 }
        ),
        pw::spa::pod::property!(
            pw::spa::param::format::FormatProperties::VideoFramerate,
            Choice,
            Range,
            Fraction,
            pw::spa::utils::Fraction { num: 60, denom: 1 },
            pw::spa::utils::Fraction { num: 0, denom: 1 },
            pw::spa::utils::Fraction { num: 240, denom: 1 }
        ),
    );
    let values: Vec<u8> = pw::spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &pw::spa::pod::Value::Object(obj),
    )
    .map_err(|e| format!("{e:?}"))?
    .0
    .into_inner();
    let mut params = [spa::pod::Pod::from_bytes(&values).ok_or("bad format")?];
    stream
        .connect(
            spa::utils::Direction::Input,
            Some(node),
            pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
            &mut params,
        )
        .map_err(|e| e.to_string())?;
    mainloop.run();
    Ok(())
}

// ---------------------------------------------------------------- the still picture

/// Shares the screens and saves a picture of each, for choosing the area. The pictures
/// are in the order of `displays()`, which lists the shared screens first. The screens
/// stay shared for the recording (see `end_screen_session`).
pub fn capture_all_screens(_count: usize, cursor: bool) -> Vec<PathBuf> {
    if let Err(e) = start_cast(cursor) {
        log::warn!(target: "dyosnap", "screen cast: {e}");
        return Vec::new();
    }
    let mut out = Vec::new();
    let screens = super::displays();
    for (i, d) in screens.iter().enumerate() {
        let Some(cut) = locate(d.bounds, &screens) else { break };
        let Some(f) = latest_frame(cut.source) else { break };
        log::info!(target: "dyosnap", "screen {i} at {:?}: {cut:?} of a {}x{} frame", d.bounds, f.width, f.height);
        let Some(mut rgba) = cut_rows(&f, cut) else { break };
        if f.pix_fmt.starts_with("bgr") {
            for px in rgba.as_chunks_mut::<4>().0 {
                px.swap(0, 2);
            }
        }
        if f.pix_fmt.ends_with('0') {
            for px in rgba.as_chunks_mut::<4>().0 {
                px[3] = 255;
            }
        }
        let Some(img) = image::RgbaImage::from_raw(cut.w, cut.h, rgba) else { break };
        let path = temp_file(&format!("screen{i}"), "png");
        if img.save(&path).is_err() {
            break;
        }
        out.push(path);
    }
    out
}

// ---------------------------------------------------------------- ffmpeg

/// Places to look for ffmpeg, in order: the system's. Only absolute PATH entries count:
/// an empty or relative one ("", ".") would run an ffmpeg from the current folder.
pub fn ffmpeg_candidates() -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = vec!["/usr/bin/ffmpeg".into()];
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).filter(|d| d.is_absolute()).map(|d| d.join("ffmpeg")));
    }
    candidates
}

/// Screen N of `displays()` is input N (see `ffmpeg_input_args`); microphones come from
/// PulseAudio (or PipeWire's Pulse server).
pub fn list_devices(ffmpeg: &Path) -> Devices {
    let mut devices = Devices { screen_index: Some(0), screens: (0..16).map(|i| (i, i)).collect(), audio_inputs: Vec::new() };
    let Ok(out) = Command::new(ffmpeg).args(["-hide_banner", "-sources", "pulse"]).output() else { return devices };
    // Lines look like: "* alsa_input.pci-0000_00_1f.3.analog-stereo [Built-in Audio Analog Stereo] (audio)"
    let mut map = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let line = line.trim_start_matches(['*', ' ']);
        let Some(open) = line.find(" [") else { continue };
        let name = line[..open].trim().to_string();
        if name.ends_with(".monitor") {
            continue; // the speakers' loopback, not a microphone
        }
        let desc = line[open + 2..].split(']').next().unwrap_or(&name).to_string();
        map.push((desc.clone(), name));
        devices.audio_inputs.push(desc);
    }
    if let Ok(mut s) = SOURCES.lock() {
        *s = map;
    }
    devices
}

/// The H.264 encoder the system's ffmpeg offers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum H264Encoder {
    X264,
    OpenH264,
}

/// Asks ffmpeg once which H.264 encoder it has: x264 if possible, else OpenH264 (Fedora's
/// `ffmpeg-free`). If neither shows up, x264 stays the choice and ffmpeg reports the error.
/// `prepare` asks on a thread at start, so opening the settings never waits for ffmpeg.
pub(super) fn h264_encoder() -> H264Encoder {
    static ENCODER: OnceLock<H264Encoder> = OnceLock::new();
    *ENCODER.get_or_init(|| {
        let list = crate::record::find_ffmpeg()
            .and_then(|ff| {
                Command::new(ff)
                    .args(["-hide_banner", "-encoders"])
                    .stdin(std::process::Stdio::null())
                    .output()
                    .ok()
            })
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
            .unwrap_or_default();
        let enc = pick_encoder(&list);
        log::info!(target: "dyosnap", "video encoder: {}", if enc == H264Encoder::X264 { "libx264" } else { "libopenh264" });
        enc
    })
}

/// Picks the encoder from the output of `ffmpeg -encoders`.
fn pick_encoder(list: &str) -> H264Encoder {
    let has = |name: &str| list.lines().any(|l| l.split_whitespace().nth(1) == Some(name));
    if has("libx264") {
        H264Encoder::X264
    } else if has("libopenh264") {
        H264Encoder::OpenH264
    } else {
        H264Encoder::X264
    }
}

/// A note for the recording settings when the encoder makes bigger files.
pub fn encoder_note() -> Option<&'static str> {
    (h264_encoder() == H264Encoder::OpenH264).then_some("Larger files: this ffmpeg has no x264 (see README).")
}

/// Target bit rate for OpenH264, in bits per second. OpenH264 has no constant-quality
/// mode. For screen text it stays far below this target (measured 0.7 to 1.25 Mbit/s at
/// 1080p), so the target mainly caps busy content. It grows with the square root of
/// `fps`, because more frames of a screen repeat each other.
fn openh264_bitrate(quality: u32, width: u32, height: u32, fps: u32) -> u32 {
    let bpp = if quality <= 35 {
        0.06
    } else if quality <= 50 {
        0.10
    } else {
        0.15
    };
    let rate = width as f64 * height as f64 * 15.0 * bpp * (fps as f64 / 15.0).sqrt();
    rate.round().clamp(1_000_000.0, 30_000_000.0) as u32
}

/// Encode with x264 if the system's ffmpeg has it, else with OpenH264. `quality` goes from
/// 1 to 100. `width`, `height` and `fps` describe the output video.
pub fn video_encoder_args(quality: u32, width: u32, height: u32, fps: u32) -> Vec<String> {
    match h264_encoder() {
        H264Encoder::X264 => {
            let crf = 40 - (quality.clamp(1, 100) * 22 / 100);
            ["-c:v", "libx264", "-preset", "veryfast", "-crf", &crf.to_string(), "-pix_fmt", "yuv420p"]
                .iter()
                .map(|s| s.to_string())
                .collect()
        }
        H264Encoder::OpenH264 => {
            let rate = openh264_bitrate(quality, width, height, fps).to_string();
            ["-c:v", "libopenh264", "-rc_mode", "bitrate", "-b:v", &rate, "-allow_skip_frames", "0", "-pix_fmt", "yuv420p"]
                .iter()
                .map(|s| s.to_string())
                .collect()
        }
    }
}

pub fn audio_encoder_args() -> Vec<String> {
    ["-c:a", "aac", "-b:a", "128k"].iter().map(|s| s.to_string()).collect()
}

/// Output options that make ffmpeg play sound on the default speakers.
pub fn sound_output_args() -> Vec<String> {
    ["-af", "aresample=48000,apad=pad_dur=0.3", "-f", "pulse", "-name", "DyoSnap", "DyoSnap"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// ffmpeg arguments for screen `screen_index` of `displays()` (and the microphone), up to
/// and including `-i`. Starts a writer that feeds the screen into a named pipe for this
/// ffmpeg run.
pub fn ffmpeg_input_args(screen_index: usize, audio: Option<&str>, fps: u32, _cursor: bool, _clicks: bool) -> Vec<String> {
    let screens = super::displays();
    let cut = screens.get(screen_index).and_then(|d| locate(d.bounds, &screens));
    let (Some(cut), Some(first)) = (cut, cut.and_then(|c| latest_frame(c.source))) else {
        // Nothing shared: let ffmpeg fail with a readable message.
        return vec!["-f".into(), "lavfi".into(), "-i".into(), "nullsrc=d=0:s=2x2".into(), "-frames:v".into(), "0".into()];
    };
    let fifo = temp_file("frames", "raw");
    let c = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap_or_default();
    if unsafe { libc::mkfifo(c.as_ptr(), 0o600) } != 0 {
        log::warn!(target: "dyosnap", "mkfifo failed: {}", std::io::Error::last_os_error());
    }
    let fps = fps.clamp(1, 60);
    let f2 = fifo.clone();
    let (fw, fh) = (first.width, first.height);
    let (w, h) = (cut.w, cut.h);
    std::thread::spawn(move || feed(&f2, cut, fw, fh, fps));
    let mut args: Vec<String> = [
        "-f",
        "rawvideo",
        "-pixel_format",
        first.pix_fmt,
        "-video_size",
        &format!("{w}x{h}"),
        "-framerate",
        &fps.to_string(),
        "-thread_queue_size",
        "64",
        "-i",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    args.push(fifo.display().to_string());
    if let Some(a) = audio {
        let name = SOURCES
            .lock()
            .ok()
            .and_then(|s| s.iter().find(|(d, _)| d == a).map(|(_, n)| n.clone()))
            .unwrap_or_else(|| "default".into());
        args.extend(["-f", "pulse", "-thread_queue_size", "1024", "-i", &name].iter().map(|s| s.to_string()));
    }
    args
}

/// Writes the screen `cut` out of its stream's newest frame into the pipe `fps` times a
/// second until ffmpeg stops reading. Frames of another size than `fw` × `fh` (the screen
/// changed) are skipped.
fn feed(fifo: &Path, cut: Cut, fw: u32, fh: u32, fps: u32) {
    use std::os::unix::fs::OpenOptionsExt;
    // Opening a pipe for writing waits for the reader; poll so a failed ffmpeg can't hang us.
    let deadline = Instant::now() + Duration::from_secs(10);
    let file = loop {
        match std::fs::OpenOptions::new().write(true).custom_flags(libc::O_NONBLOCK).open(fifo) {
            Ok(f) => break Some(f),
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => break None,
        }
    };
    let Some(mut file) = file else {
        let _ = std::fs::remove_file(fifo);
        return;
    };
    // From now on, writes wait until ffmpeg has room.
    unsafe {
        use std::os::fd::AsRawFd;
        let fd = file.as_raw_fd();
        let flags = libc::fcntl(fd, libc::F_GETFL);
        libc::fcntl(fd, libc::F_SETFL, flags & !libc::O_NONBLOCK);
    }
    // The compositor sends frames only when the screen changes. The newest ones may still
    // show DyoSnap's area selection, which closes just now (the red frame appearing
    // around the area makes a new one while it is open). Wait until the window is gone
    // and a frame came after that.
    let wait_until = Instant::now() + Duration::from_secs(2);
    let window_gone = || {
        !WINDOW_OPEN.load(Ordering::Relaxed)
            && latest_frame(cut.source).is_some_and(|f| f.seq > CLOSED_AT_SEQ.load(Ordering::Relaxed))
    };
    while Instant::now() < wait_until && !window_gone() {
        std::thread::sleep(Duration::from_millis(10));
    }
    let period = Duration::from_secs_f64(1.0 / fps as f64);
    let mut next = Instant::now();
    let mut last: Option<Arc<Vec<u8>>> = None;
    let mut last_seq = 0;
    loop {
        if let Some(f) = latest_frame(cut.source) && f.width == fw && f.height == fh && f.seq != last_seq {
            last_seq = f.seq;
            last = cut_rows(&f, cut).map(Arc::new);
        }
        let Some(data) = &last else { break };
        if file.write_all(data).is_err() {
            break; // ffmpeg stopped reading
        }
        next += period;
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else if now - next > Duration::from_secs(1) {
            next = now; // fell far behind (machine busy): don't try to catch up
        }
    }
    let _ = std::fs::remove_file(fifo);
}

/// Without ffmpeg there is no recorder on Linux; this command explains that.
pub fn fallback_record_command(_rect_global: [f32; 4], _cursor: bool, _clicks: bool, _audio: bool, _out: &Path) -> Command {
    let mut c = Command::new("sh");
    c.args(["-c", "echo 'Recording needs ffmpeg. Install it with your package manager.' >&2; exit 1"]);
    c
}

pub const FALLBACK_EXT: &str = "mp4";

pub fn stop_fallback(child: &mut Child) {
    let _ = child.kill();
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Rect, pos2, vec2};

    #[test]
    fn openh264_bitrate_follows_size_quality_and_fps() {
        let near = |a: u32, b: f64| (a as f64 - b).abs() < 20_000.0;
        assert!(near(openh264_bitrate(60, 1920, 1080, 15), 4.67e6));
        assert!(near(openh264_bitrate(35, 1920, 1080, 15), 1.87e6));
        assert!(near(openh264_bitrate(60, 1920, 1080, 60), 9.33e6));
        assert_eq!(openh264_bitrate(35, 100, 100, 10), 1_000_000);
        assert_eq!(openh264_bitrate(60, 3840, 2160, 60), 30_000_000);
    }

    #[test]
    fn encoder_is_picked_from_the_list() {
        let x = " V....D libx264              libx264 H.264\n V....D libopenh264          OpenH264\n";
        let o = " V....D libopenh264          OpenH264 H.264\n";
        assert_eq!(pick_encoder(x), H264Encoder::X264);
        assert_eq!(pick_encoder(o), H264Encoder::OpenH264);
        assert_eq!(pick_encoder(""), H264Encoder::X264);
    }

    fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(pos2(x, y), vec2(w, h))
    }

    #[test]
    fn screens_are_found_in_their_stream() {
        // One stream per screen: the whole frame.
        let c = cut_in(r(1920.0, 0.0, 1920.0, 1080.0), r(1920.0, 0.0, 1920.0, 1080.0), 1920, 1080).unwrap();
        assert_eq!((c.x, c.y, c.w, c.h), (0, 0, 1920, 1080));
        // KDE's whole desktop as one stream, at 2x: the right screen is the right half.
        let c = cut_in(r(1920.0, 0.0, 1920.0, 1080.0), r(0.0, 0.0, 3840.0, 1080.0), 7680, 2160).unwrap();
        assert_eq!((c.x, c.y, c.w, c.h), (3840, 0, 3840, 2160));
        // A stream of another screen does not show it.
        assert!(cut_in(r(1920.0, 0.0, 1920.0, 1080.0), r(0.0, 0.0, 1920.0, 1080.0), 1920, 1080).is_none());
    }

    #[test]
    fn cut_rows_takes_the_screen_out_of_the_frame() {
        // 4 x 2 frame, each pixel's first byte is its x; cut the right half.
        let data: Vec<u8> = (0..2).flat_map(|_| (0..4u8).flat_map(|x| [x, 0, 0, 255])).collect();
        let f = Frame { width: 4, height: 2, pix_fmt: "rgba", data: Arc::new(data), seq: 1 };
        let rows = cut_rows(&f, Cut { source: 0, x: 2, y: 0, w: 2, h: 2 }).unwrap();
        assert_eq!(rows.chunks(4).map(|p| p[0]).collect::<Vec<_>>(), vec![2, 3, 2, 3]);
    }
}
