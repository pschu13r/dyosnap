//! User settings, stored as a small JSON file. Nothing here touches the network.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VideoFormat {
    Mp4,
    Mov,
    Gif,
}

impl VideoFormat {
    pub fn label(self) -> &'static str {
        match self {
            VideoFormat::Mp4 => "MPEG-4 (.mp4)",
            VideoFormat::Mov => "QuickTime (.mov)",
            VideoFormat::Gif => "Animated GIF (.gif)",
        }
    }
    pub fn ext(self) -> &'static str {
        match self {
            VideoFormat::Mp4 => "mp4",
            VideoFormat::Mov => "mov",
            VideoFormat::Gif => "gif",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipFormat {
    Png,
    Tiff,
}

/// A keyboard shortcut: modifier keys plus one key (a W3C key code such as "Digit4").
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hotkey {
    pub cmd: bool,
    pub alt: bool,
    pub ctrl: bool,
    pub shift: bool,
    pub code: String,
}

impl Hotkey {
    pub fn new(cmd: bool, alt: bool, code: &str) -> Self {
        Self { cmd, alt, ctrl: false, shift: false, code: code.into() }
    }

    /// Shown like the original: "⌘+⌥+6". On Linux: "Ctrl+Alt+6".
    pub fn label(&self) -> String {
        if cfg!(target_os = "linux") {
            return self.linux_text();
        }
        let mut parts: Vec<String> = Vec::new();
        if self.ctrl {
            parts.push("⌃".into());
        }
        if self.cmd {
            parts.push("⌘".into());
        }
        if self.alt {
            parts.push("⌥".into());
        }
        if self.shift {
            parts.push("⇧".into());
        }
        parts.push(key_symbol(&self.code));
        parts.join("+")
    }

    /// Linux style: "Ctrl+Alt+Shift+6" (`cmd` is Ctrl there).
    fn linux_text(&self) -> String {
        let mut s = String::new();
        if self.cmd || self.ctrl {
            s.push_str("Ctrl+");
        }
        if self.alt {
            s.push_str("Alt+");
        }
        if self.shift {
            s.push_str("Shift+");
        }
        s + &key_symbol(&self.code)
    }

    /// Compact form for menus: "⌥⌘6". On Linux the same as `label`.
    pub fn short(&self) -> String {
        if cfg!(target_os = "linux") {
            return self.linux_text();
        }
        let mut s = String::new();
        if self.ctrl {
            s.push('⌃');
        }
        if self.alt {
            s.push('⌥');
        }
        if self.shift {
            s.push('⇧');
        }
        if self.cmd {
            s.push('⌘');
        }
        s + &key_symbol(&self.code)
    }
}

pub fn key_symbol(code: &str) -> String {
    if let Some(k) = code.strip_prefix("Key") {
        return k.to_string();
    }
    if let Some(d) = code.strip_prefix("Digit") {
        return d.to_string();
    }
    match code {
        "Space" => "Space",
        "Minus" => "-",
        "Equal" => "=",
        "Comma" => ",",
        "Period" => ".",
        "Slash" => "/",
        "Backslash" => "\\",
        "Semicolon" => ";",
        "Quote" => "'",
        "BracketLeft" => "[",
        "BracketRight" => "]",
        "Backquote" => "`",
        "ArrowUp" => "↑",
        "ArrowDown" => "↓",
        "ArrowLeft" => "←",
        "ArrowRight" => "→",
        "Enter" => "↩",
        "Tab" => "⇥",
        "Backspace" => "⌫",
        "Delete" => "⌦",
        "Escape" => "⎋",
        other => other,
    }
    .to_string()
}

/// The shortcut actions, in the order the Hotkeys tab shows them.
pub const HOTKEY_ACTIONS: [(&str, &str); 8] = [
    ("area", "Capture area"),
    ("previous", "Capture previous area"),
    ("full", "Capture fullscreen"),
    ("timed", "Fullscreen after delay"),
    ("window", "Capture window"),
    ("latest", "Open Latest Image"),
    ("record", "Record video"),
    ("clipboard", "Open from Clipboard"),
];

pub const VIDEO_HOTKEY_ACTIONS: [(&str, &str); 2] = [("rec_pause", "Record/Pause"), ("rec_stop", "Stop")];

/// The keys of a shortcut inside the app, for tooltips: "⇧⌘Z", on Linux "Ctrl+Shift+Z".
pub fn keys_text(shift: bool, key: &str) -> String {
    if cfg!(target_os = "linux") {
        format!("Ctrl+{}{key}", if shift { "Shift+" } else { "" })
    } else {
        format!("{}⌘{key}", if shift { "⇧" } else { "" })
    }
}

pub fn default_hotkey(id: &str) -> Option<Hotkey> {
    // On Linux Ctrl+digit belongs to other apps (browser tabs), so all defaults add Alt.
    let linux = cfg!(target_os = "linux");
    match id {
        "area" => Some(Hotkey::new(true, linux, "Digit4")),
        // Linux: these are only suggestions. The desktop's settings list every action and
        // the person can assign or change them there.
        "previous" if linux => Some(Hotkey { shift: true, ..Hotkey::new(true, true, "Digit4") }),
        "full" => Some(Hotkey::new(true, true, "Digit6")),
        "timed" => Some(Hotkey::new(true, linux, "Digit2")),
        "window" if linux => Some(Hotkey::new(true, true, "Digit5")),
        "latest" => Some(Hotkey::new(true, true, "Digit0")),
        "record" => Some(Hotkey::new(true, linux, "Digit3")),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageFormat {
    Png,
    Jpg,
}

impl ImageFormat {
    pub fn ext(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpg => "jpg",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AfterCapture {
    OpenEditor,
    SaveAndCopy,
    CopyOnly,
}

impl AfterCapture {
    pub fn label(self) -> &'static str {
        match self {
            AfterCapture::OpenEditor => "Open LightSnap editor",
            AfterCapture::SaveAndCopy => "Save to folder and copy",
            AfterCapture::CopyOnly => "Copy to clipboard only",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Quality {
    Low,
    Medium,
    High,
}

impl Quality {
    pub fn label(self) -> &'static str {
        match self {
            Quality::Low => "Low recording quality",
            Quality::Medium => "Medium recording quality",
            Quality::High => "High recording quality",
        }
    }
    /// Video quality from 1 (smallest file) to 100 (best picture).
    pub fn video_q(self) -> u32 {
        match self {
            Quality::Low => 35,
            Quality::Medium => 50,
            Quality::High => 60,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub save_dir: PathBuf,
    pub format: ImageFormat,
    pub after_capture: AfterCapture,
    pub delay_secs: u32,
    pub fps: u32,
    pub quality: Quality,
    pub capture_cursor: bool,
    pub capture_clicks: bool,
    /// Name of the audio input, or None for no audio.
    pub audio: Option<String>,
    pub hotkeys: bool,
    pub color: [u8; 4],
    pub width: f32,
    // General
    pub sound: bool,
    pub external_editor: String,
    // Advanced
    pub jpeg_quality: u8,
    pub autosave: bool,
    pub shrink_retina: bool,
    pub video_format: VideoFormat,
    pub gif_fps: u32,
    pub gif_quality: u32,
    pub autosave_movies: bool,
    pub copy_path: bool,
    pub copy_image: bool,
    pub clip_format: ClipFormat,
    pub name_template: String,
    pub apply_template_reopened: bool,
    /// Shortcuts the person changed; `None` means "Not set". Missing ones use the defaults.
    pub keys: BTreeMap<String, Option<Hotkey>>,
    /// The window was shown once (first start); later starts stay quietly in the menu bar.
    pub welcomed: bool,
    /// Linux: the size of the main window when it was last closed (Wayland closes the
    /// window instead of hiding it, so the size has to be kept here).
    pub window_size: Option<[f32; 2]>,
    /// The shortcuts as the desktop has them, by action id (Linux: the desktop owns
    /// global shortcuts and the person may have changed them there). Not saved.
    #[serde(skip)]
    pub live_keys: Option<std::collections::HashMap<String, String>>,
}

impl Default for Settings {
    fn default() -> Self {
        let base = dirs::picture_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(std::env::temp_dir);
        Self {
            save_dir: base.join("LightSnap"),
            format: ImageFormat::Png,
            after_capture: AfterCapture::OpenEditor,
            delay_secs: 5,
            fps: 15,
            quality: Quality::High,
            capture_cursor: true,
            capture_clicks: true,
            audio: None,
            hotkeys: true,
            color: [235, 51, 36, 255],
            width: 4.0,
            sound: false,
            external_editor: crate::platform::DEFAULT_EDITOR.into(),
            jpeg_quality: 90,
            autosave: false,
            shrink_retina: false,
            video_format: VideoFormat::Mp4,
            gif_fps: 10,
            gif_quality: 100,
            autosave_movies: false,
            copy_path: false,
            copy_image: true,
            clip_format: ClipFormat::Png,
            name_template: "Snap %Y-%m-%d %H-%M-%S".into(),
            apply_template_reopened: false,
            keys: BTreeMap::new(),
            welcomed: false,
            window_size: None,
            live_keys: None,
        }
    }
}

fn settings_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("LightSnap").join("settings.json"))
}

impl Settings {
    pub fn load() -> Self {
        settings_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Some(p) = settings_path() {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(s) = serde_json::to_string_pretty(self) {
                let _ = std::fs::write(p, s);
            }
        }
    }

    /// A fresh file name like "Snap 2026-11-16 11-41-59".
    pub fn new_name() -> String {
        format!("Snap {}", chrono::Local::now().format("%Y-%m-%d %H-%M-%S"))
    }

    /// A file name from the name template. `kind` fills %T ("Screenshot" or "Screencast").
    pub fn make_name(&self, kind: &str) -> String {
        let now = chrono::Local::now();
        let mut out = String::new();
        let mut chars = self.name_template.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '%' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('Y') => out += &now.format("%Y").to_string(),
                Some('y') => out += &now.format("%y").to_string(),
                Some('m') => out += &now.format("%m").to_string(),
                Some('d') => out += &now.format("%d").to_string(),
                Some('H') => out += &now.format("%H").to_string(),
                Some('M') => out += &now.format("%M").to_string(),
                Some('S') => out += &now.format("%S").to_string(),
                Some('T') => out += kind,
                Some('R') => {
                    let mut seed = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_nanos() as u64)
                        .unwrap_or(7);
                    for _ in 0..5 {
                        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                        let n = (seed >> 33) % 36;
                        out.push(char::from_digit(n as u32, 36).unwrap_or('x'));
                    }
                }
                Some('%') => out.push('%'),
                Some(o) => {
                    out.push('%');
                    out.push(o);
                }
                None => out.push('%'),
            }
        }
        let out = sanitize(&out);
        if out.is_empty() { Self::new_name() } else { out }
    }

    /// The shortcut for an action, or None when not set.
    pub fn hotkey(&self, id: &str) -> Option<Hotkey> {
        match self.keys.get(id) {
            Some(k) => k.clone(),
            None => default_hotkey(id),
        }
    }

    pub fn hotkey_short(&self, id: &str) -> String {
        if !self.hotkeys {
            return String::new();
        }
        if let Some(live) = &self.live_keys {
            return live.get(id).cloned().unwrap_or_default();
        }
        self.hotkey(id).map(|k| k.short()).unwrap_or_default()
    }

    /// A free path in the save folder for `stem.ext`.
    pub fn free_path(&self, stem: &str, ext: &str) -> PathBuf {
        let _ = std::fs::create_dir_all(&self.save_dir);
        let stem = sanitize(stem);
        let mut p = self.save_dir.join(format!("{stem}.{ext}"));
        let mut n = 2;
        while p.exists() {
            p = self.save_dir.join(format!("{stem} ({n}).{ext}"));
            n += 1;
        }
        p
    }
}

pub fn sanitize(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| if matches!(c, '/' | ':' | '\\' | '\0') { '-' } else { c })
        .collect();
    let t = t.trim().to_string();
    if t.is_empty() { Settings::new_name() } else { t }
}
