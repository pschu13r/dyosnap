//! User settings, stored as a small JSON file. Nothing here touches the network.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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

/// How many extra save folders there can be.
pub const MAX_EXTRA_DIRS: usize = 9;

/// The shortcut action id of the extra folder with `id`: "folder1", "folder2", ...
pub fn folder_action(id: u32) -> String {
    format!("folder{id}")
}

/// The folder id inside a shortcut action id ("folder4" is 4); None for any other id.
pub fn folder_id(action: &str) -> Option<u32> {
    let id: u32 = action.strip_prefix("folder")?.parse().ok()?;
    (id >= 1).then_some(id)
}

/// An extra save folder. Its id never changes and is never used again, so its shortcut
/// (the action "folder<id>", also kept by the desktop on Linux) stays with it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtraDir {
    pub id: u32,
    pub path: PathBuf,
}

/// A folder's short name: the last path component ("Work" for "/home/me/Work").
pub fn dir_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
}

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
            AfterCapture::OpenEditor => "Open DyoSnap editor",
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
    /// More folders to save an area into by shortcut (up to `MAX_EXTRA_DIRS`), in the order
    /// shown. The shortcut of the folder with id N is the key "folderN" in `keys`.
    pub extra_dirs: Vec<ExtraDir>,
    /// The id the next added folder gets. Only goes up.
    pub next_folder_id: u32,
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
            save_dir: base.join("DyoSnap"),
            extra_dirs: Vec::new(),
            next_folder_id: 1,
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
    dirs::config_dir().map(|d| d.join("DyoSnap").join("settings.json"))
}

impl Settings {
    pub fn load() -> Self {
        let mut s: Self = settings_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        s.sanitize();
        s
    }

    /// Makes the extra folders of a file that was edited by hand usable: only absolute paths,
    /// no id 0, no repeated id or path (the first one stays), at most `MAX_EXTRA_DIRS`, and
    /// a counter above every id. Paths are not resolved: a folder may not exist yet.
    pub fn sanitize(&mut self) {
        let mut kept: Vec<ExtraDir> = Vec::new();
        for d in std::mem::take(&mut self.extra_dirs) {
            if kept.len() >= MAX_EXTRA_DIRS {
                break;
            }
            if d.id == 0 || !d.path.is_absolute() || kept.iter().any(|k| k.id == d.id || k.path == d.path) {
                continue;
            }
            kept.push(d);
        }
        self.extra_dirs = kept;
        // Shortcuts of folders that are gone would only clutter the file.
        let live: Vec<String> = self.extra_dirs.iter().map(|d| folder_action(d.id)).collect();
        self.keys.retain(|k, _| folder_id(k).is_none() || live.contains(k));
        let above = self.extra_dirs.iter().map(|d| d.id).max().map_or(Some(1), |m| m.checked_add(1));
        // With no id left above, the counter stays at the top: no folder can be added.
        self.next_folder_id = self.next_folder_id.max(above.unwrap_or(u32::MAX)).max(1);
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
        format!("Snap {}", jiff::ZonedDateTime::now().strftime("%Y-%m-%d %H-%M-%S"))
    }

    /// A file name from the name template. `kind` fills %T ("Screenshot" or "Screencast").
    pub fn make_name(&self, kind: &str) -> String {
        let now = jiff::ZonedDateTime::now();
        let mut out = String::new();
        let mut chars = self.name_template.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '%' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('Y') => out += &now.strftime("%Y").to_string(),
                Some('y') => out += &now.strftime("%y").to_string(),
                Some('m') => out += &now.strftime("%m").to_string(),
                Some('d') => out += &now.strftime("%d").to_string(),
                Some('H') => out += &now.strftime("%H").to_string(),
                Some('M') => out += &now.strftime("%M").to_string(),
                Some('S') => out += &now.strftime("%S").to_string(),
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

    /// What a shortcut does, in the desktop's own list of shortcuts (Linux).
    pub fn action_description(&self, id: &str) -> String {
        if let Some(d) = self.extra_dir(id) {
            return format!("Capture area to {}", short_label(&dir_name(&d.path)));
        }
        HOTKEY_ACTIONS
            .iter()
            .chain(VIDEO_HOTKEY_ACTIONS.iter())
            .find(|(i, _)| *i == id)
            .map(|(_, d)| d.to_string())
            .unwrap_or_else(|| id.to_string())
    }

    /// The extra folder a shortcut action id ("folder4") belongs to, if it still exists.
    pub fn extra_dir(&self, action: &str) -> Option<&ExtraDir> {
        let id = folder_id(action)?;
        self.extra_dirs.iter().find(|d| d.id == id)
    }

    /// Adds an extra folder with a new id. Returns false (and changes nothing) when it is
    /// the save folder, is listed already, or there are `MAX_EXTRA_DIRS`.
    pub fn add_extra_dir(&mut self, path: PathBuf) -> bool {
        if self.extra_dirs.len() >= MAX_EXTRA_DIRS || path == self.save_dir || self.extra_dirs.iter().any(|d| d.path == path) {
            return false;
        }
        // Also above every id in the list, in case the file was edited by hand. With no id
        // left, nothing is added (like at the limit).
        let mut above = 1;
        for d in &self.extra_dirs {
            let Some(next) = d.id.checked_add(1) else { return false };
            above = above.max(next);
        }
        let id = self.next_folder_id.max(above);
        let Some(next) = id.checked_add(1) else { return false };
        self.next_folder_id = next;
        self.extra_dirs.push(ExtraDir { id, path });
        true
    }

    /// Removes the extra folder with `id` and its shortcut. The other folders keep their ids
    /// and keys; the id is not used again.
    pub fn remove_extra_dir(&mut self, id: u32) {
        self.extra_dirs.retain(|d| d.id != id);
        self.keys.remove(&folder_action(id));
    }
}

/// How many names `create_new_in` tries.
const NAME_TRIES: u32 = 1000;

/// Makes a new file in `dir` named `stem.ext`, then `stem (2).ext`, and so on. The file is
/// created in one step that fails if the name exists already, also as a dangling symbolic
/// link, so nothing is overwritten or followed to another place.
pub fn create_new_in(dir: &Path, stem: &str, ext: &str) -> std::io::Result<(PathBuf, std::fs::File)> {
    let stem = sanitize(stem);
    for n in 1..=NAME_TRIES {
        let name = if n == 1 { format!("{stem}.{ext}") } else { format!("{stem} ({n}).{ext}") };
        let p = dir.join(name);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&p) {
            Ok(f) => return Ok((p, f)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "no free file name"))
}

/// Longest folder name shown in a shortcut's description.
const LABEL_MAX: usize = 60;

/// A name for a label: control characters (also NUL) removed, and cut to `LABEL_MAX`
/// characters with "…".
pub fn short_label(name: &str) -> String {
    let clean: String = name.chars().filter(|c| !c.is_control()).collect();
    if clean.chars().count() <= LABEL_MAX {
        return clean;
    }
    clean.chars().take(LABEL_MAX - 1).collect::<String>() + "…"
}

pub fn sanitize(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| if matches!(c, '/' | ':' | '\\' | '\0') { '-' } else { c })
        .collect();
    let t = t.trim().to_string();
    if t.is_empty() { Settings::new_name() } else { t }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hk(code: &str) -> Option<Hotkey> {
        Some(Hotkey::new(true, true, code))
    }

    fn ids(s: &Settings) -> Vec<u32> {
        s.extra_dirs.iter().map(|d| d.id).collect()
    }

    #[test]
    fn removing_a_folder_keeps_the_ids_and_keys_of_the_others() {
        let mut s = Settings::default();
        for d in ["/a", "/b", "/c"] {
            assert!(s.add_extra_dir(d.into()));
        }
        assert_eq!(ids(&s), [1, 2, 3]);
        s.keys.insert("folder1".into(), hk("Digit1"));
        s.keys.insert("folder2".into(), hk("Digit2"));
        s.keys.insert("folder3".into(), hk("Digit3"));
        s.keys.insert("area".into(), hk("Digit9"));
        s.remove_extra_dir(1);
        assert_eq!(ids(&s), [2, 3]);
        assert_eq!(s.extra_dirs[0].path, PathBuf::from("/b"));
        assert!(!s.keys.contains_key("folder1"));
        assert_eq!(s.keys.get("folder2"), Some(&hk("Digit2")));
        assert_eq!(s.keys.get("folder3"), Some(&hk("Digit3")));
        assert_eq!(s.keys.get("area"), Some(&hk("Digit9")));
        // The id of a removed folder finds nothing.
        assert!(s.extra_dir("folder1").is_none());
        assert_eq!(s.extra_dir("folder3").map(|d| d.path.clone()), Some(PathBuf::from("/c")));
        s.remove_extra_dir(42); // unknown: nothing happens
        assert_eq!(s.extra_dirs.len(), 2);
    }

    #[test]
    fn a_new_folder_after_a_removal_gets_a_new_id() {
        let mut s = Settings::default();
        s.add_extra_dir("/a".into());
        s.add_extra_dir("/b".into());
        s.remove_extra_dir(2);
        s.add_extra_dir("/c".into());
        assert_eq!(ids(&s), [1, 3]);
        assert_eq!(s.next_folder_id, 4);
        // A hand-edited file with a too low counter still gets a free id.
        s.next_folder_id = 1;
        s.add_extra_dir("/d".into());
        assert_eq!(ids(&s), [1, 3, 4]);
    }

    #[test]
    fn adds_only_new_folders_up_to_nine() {
        let mut s = Settings { save_dir: "/save".into(), ..Settings::default() };
        assert!(!s.add_extra_dir("/save".into()));
        assert!(s.add_extra_dir("/a".into()));
        assert!(!s.add_extra_dir("/a".into()));
        for i in 2..=MAX_EXTRA_DIRS {
            assert!(s.add_extra_dir(format!("/f{i}").into()));
        }
        assert_eq!(s.extra_dirs.len(), MAX_EXTRA_DIRS);
        assert!(!s.add_extra_dir("/one-too-many".into()));
    }

    #[test]
    fn folder_ids_and_names() {
        assert_eq!(folder_action(3), "folder3");
        assert_eq!(folder_id("folder3"), Some(3));
        assert_eq!(folder_id("folder12"), Some(12));
        assert_eq!(folder_id("folder0"), None);
        assert_eq!(folder_id("folder"), None);
        assert_eq!(folder_id("area"), None);
        assert_eq!(dir_name(Path::new("/home/me/Work")), "Work");
        assert_eq!(dir_name(Path::new("/home/me/Work/")), "Work");
        assert_eq!(dir_name(Path::new("/")), "/");
        let mut s = Settings::default();
        s.add_extra_dir("/x/Clients".into());
        assert_eq!(s.action_description("folder1"), "Capture area to Clients");
        assert_eq!(s.action_description("area"), "Capture area");
    }

    #[test]
    fn old_settings_load_without_extra_dirs() {
        let s: Settings = serde_json::from_str(r#"{"sound": true}"#).unwrap();
        assert!(s.extra_dirs.is_empty());
        assert_eq!(s.next_folder_id, 1);
        assert!(s.sound);
    }

    fn dirs_of(list: &[(u32, &str)]) -> Settings {
        Settings {
            extra_dirs: list.iter().map(|(id, p)| ExtraDir { id: *id, path: PathBuf::from(p) }).collect(),
            ..Settings::default()
        }
    }

    #[test]
    fn sanitize_drops_bad_extra_dirs() {
        let mut s = dirs_of(&[(0, "/zero"), (1, "relative/x"), (2, "/a"), (2, "/same-id"), (3, "/a"), (4, "/b")]);
        s.keys.insert("folder3".into(), hk("Digit3"));
        s.keys.insert("area".into(), hk("Digit4"));
        s.sanitize();
        assert_eq!(ids(&s), [2, 4]);
        assert!(s.next_folder_id > 4);
        assert!(!s.keys.contains_key("folder3"));
        assert!(s.keys.contains_key("area"));
    }

    #[test]
    fn sanitize_truncates_and_keeps_the_counter_above() {
        let list: Vec<(u32, String)> = (1..=50).map(|i| (i, format!("/f{i}"))).collect();
        let mut s = Settings {
            extra_dirs: list.iter().map(|(id, p)| ExtraDir { id: *id, path: p.into() }).collect(),
            next_folder_id: 3,
            ..Settings::default()
        };
        s.sanitize();
        assert_eq!(s.extra_dirs.len(), MAX_EXTRA_DIRS);
        assert_eq!(s.next_folder_id, MAX_EXTRA_DIRS as u32 + 1);
    }

    #[test]
    fn no_id_left_adds_nothing() {
        let mut s = dirs_of(&[(u32::MAX, "/a")]);
        s.sanitize();
        assert_eq!(ids(&s), [u32::MAX]);
        assert!(!s.add_extra_dir("/b".into()));
        let mut s = Settings { next_folder_id: u32::MAX, ..Settings::default() };
        assert!(!s.add_extra_dir("/b".into()));
        assert!(s.extra_dirs.is_empty());
    }

    #[test]
    fn descriptions_have_no_control_characters_and_a_length_limit() {
        assert_eq!(short_label("a\0b\nc\u{7}"), "abc");
        let long = "x".repeat(200);
        let l = short_label(&long);
        assert_eq!(l.chars().count(), 60);
        assert!(l.ends_with('…'));
        let mut s = Settings::default();
        s.add_extra_dir(format!("/x/{long}\u{1}").into());
        let d = s.action_description("folder1");
        assert!(d.starts_with("Capture area to xxx"));
        assert!(!d.chars().any(|c| c.is_control()));
        assert!(d.chars().count() < 90);
    }

    #[test]
    fn name_template_fills_every_placeholder() {
        let s = Settings { name_template: "%Y %y %m %d %H %M %S %T %R".into(), ..Settings::default() };
        let name = s.make_name("Screenshot");
        let parts: Vec<&str> = name.split(' ').collect();
        let lens: Vec<usize> = parts.iter().map(|p| p.len()).collect();
        assert_eq!(lens, [4, 2, 2, 2, 2, 2, 2, 10, 5], "{name}");
        assert!(parts[..7].iter().all(|p| p.chars().all(|c| c.is_ascii_digit())), "{name}");
        assert_eq!(&parts[0][2..], parts[1]);
        assert_eq!(parts[7], "Screenshot");
        assert!(parts[8].chars().all(|c| c.is_ascii_alphanumeric()), "{name}");
        assert!(Settings::new_name().starts_with("Snap 20"));
    }

    #[test]
    fn create_new_takes_the_next_name() {
        let dir = std::env::temp_dir().join(format!("dyosnap-test-{}-a", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (a, _) = create_new_in(&dir, "Shot", "png").unwrap();
        let (b, _) = create_new_in(&dir, "Shot", "png").unwrap();
        assert_eq!(a.file_name().unwrap(), "Shot.png");
        assert_eq!(b.file_name().unwrap(), "Shot (2).png");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn create_new_does_not_follow_a_dangling_symlink() {
        let dir = std::env::temp_dir().join(format!("dyosnap-test-{}-b", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("elsewhere.txt");
        std::os::unix::fs::symlink(&target, dir.join("Shot.png")).unwrap();
        let (p, _) = create_new_in(&dir, "Shot", "png").unwrap();
        assert_eq!(p.file_name().unwrap(), "Shot (2).png");
        assert!(!target.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
