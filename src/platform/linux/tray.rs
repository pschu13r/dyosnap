//! The tray icon (StatusNotifierItem over D-Bus, through ksni) and the global shortcuts
//! (GlobalShortcuts portal: the desktop asks the person once to confirm the keys).

use crate::platform::{MenuEntry, Sink, TRAY_PX, tray_pixels};
use crate::settings::Hotkey;
use ksni::blocking::{Handle, TrayMethods};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

struct Sni {
    menu: Vec<MenuEntry>,
    recording: bool,
    /// While recording: the ring's step (see `ring_pixels`) and whether paused.
    progress: (u32, bool),
    title: Option<String>,
    sink: Sink,
    /// Folder with the two icons as files (see `write_icons`), if it could be made.
    icon_dir: Option<std::path::PathBuf>,
}

const ICON_IDLE: &str = "dyosnap-tray";
const ICON_REC: &str = "dyosnap-tray-recording";
/// Steps of the ring around the recording dot: it fills once a minute, a step every 5 s.
const RING_STEPS: u32 = 12;

fn rec_icon_name(step: u32, paused: bool) -> String {
    format!("{ICON_REC}-{step:02}{}", if paused { "-paused" } else { "" })
}

/// The tray icon while recording, as RGBA (`TRAY_PX` square): a dot, and a ring around it
/// that fills clockwise from the top in `RING_STEPS` steps per minute. Shows that the
/// recording runs and roughly how far into the minute it is; the bar on KDE and the tray
/// menu have the exact time. Red, orange while paused.
fn ring_pixels(step: u32, paused: bool) -> Vec<u8> {
    use tiny_skia::{FillRule, LineCap, Paint, PathBuilder, Pixmap, Stroke, Transform};
    let n = TRAY_PX;
    let Some(mut pm) = Pixmap::new(n, n) else { return vec![0; (n * n * 4) as usize] };
    let (r, g, b) = if paused { (255, 170, 0) } else { (230, 40, 40) };
    let c = n as f32 / 2.0;
    let mut paint = Paint { anti_alias: true, ..Default::default() };
    paint.set_color_rgba8(r, g, b, 255);
    if let Some(dot) = PathBuilder::from_circle(c, c, 9.0) {
        pm.fill_path(&dot, &paint, FillRule::Winding, Transform::identity(), None);
    }
    let stroke = Stroke { width: 4.0, line_cap: LineCap::Butt, ..Default::default() };
    // The track, faint, then the part of the minute that has passed.
    let mut track = Paint { anti_alias: true, ..Default::default() };
    track.set_color_rgba8(r, g, b, 70);
    if let Some(ring) = PathBuilder::from_circle(c, c, 17.0) {
        pm.stroke_path(&ring, &track, &stroke, Transform::identity(), None);
    }
    let frac = (step.min(RING_STEPS) as f32) / RING_STEPS as f32;
    if frac > 0.0 {
        let mut pb = PathBuilder::new();
        let segs = (frac * 48.0).ceil() as i32;
        for i in 0..=segs {
            let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * frac * i as f32 / segs as f32;
            let (x, y) = (c + 17.0 * a.cos(), c + 17.0 * a.sin());
            if i == 0 { pb.move_to(x, y) } else { pb.line_to(x, y) }
        }
        if let Some(arc) = pb.finish() {
            pm.stroke_path(&arc, &paint, &stroke, Transform::identity(), None);
        }
    }
    // tiny-skia is premultiplied; the image crate and the tray want straight alpha.
    let mut px = pm.take();
    for p in px.as_chunks_mut::<4>().0 {
        let a = p[3] as u32;
        if a > 0 && a < 255 {
            for v in &mut p[..3] {
                *v = ((*v as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    px
}

/// Writes both tray icons as files. Tray hosts reliably follow a change of icon *name*; a
/// changed picture alone is not always picked up (GNOME's AppIndicator extension keeps
/// the first one).
///
/// The camera is an SVG in KDE's text color class: Plasma paints it in the panel's text
/// color, dark on a light panel and light on a dark one. Other hosts draw it in the white
/// given in the file (GNOME's top bar is dark). The red dot is a PNG, it shows on any panel.
fn write_icons() -> Option<std::path::PathBuf> {
    let dir = dirs::cache_dir()?.join("dyosnap").join("icons");
    std::fs::create_dir_all(&dir).ok()?;
    // Up to v0.4.2 the camera was a PNG; with both files the host may pick the old one.
    let _ = std::fs::remove_file(dir.join(format!("{ICON_IDLE}.png")));
    std::fs::write(dir.join(format!("{ICON_IDLE}.svg")), CAMERA_SVG).ok()?;
    let _ = std::fs::remove_file(dir.join(format!("{ICON_REC}.png")));
    for paused in [false, true] {
        for step in 0..=RING_STEPS {
            let img = image::RgbaImage::from_raw(TRAY_PX, TRAY_PX, ring_pixels(step, paused))?;
            img.save(dir.join(format!("{}.png", rec_icon_name(step, paused)))).ok()?;
        }
    }
    Some(dir)
}

/// The camera of `tray_pixels` as SVG (44 × 44): body, top bump, lens ring and lens.
const CAMERA_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="44" height="44" viewBox="0 0 44 44">
<style id="current-color-scheme" type="text/css">.ColorScheme-Text { color: #ffffff; }</style>
<path class="ColorScheme-Text" fill="currentColor" fill-rule="evenodd"
 d="M5 13h34v23H5z M15 8h14v5H15z M14.5 24.5a7.5 7.5 0 1 0 15 0a7.5 7.5 0 1 0 -15 0z M18 24.5a4 4 0 1 0 8 0a4 4 0 1 0 -8 0z"/>
</svg>
"#;

impl ksni::Tray for Sni {
    fn id(&self) -> String {
        super::APP_ID.into()
    }
    fn title(&self) -> String {
        match &self.title {
            Some(t) => format!("DyoSnap {t}"),
            None => "DyoSnap".into(),
        }
    }
    fn icon_theme_path(&self) -> String {
        self.icon_dir.as_ref().map(|d| d.display().to_string()).unwrap_or_default()
    }
    fn icon_name(&self) -> String {
        if self.icon_dir.is_none() {
            return String::new();
        }
        if self.recording { rec_icon_name(self.progress.0, self.progress.1) } else { ICON_IDLE.into() }
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        // RGBA → ARGB, as the spec wants.
        let mut data = if self.recording { ring_pixels(self.progress.0, self.progress.1) } else { tray_pixels(false, true) };
        for px in data.as_chunks_mut::<4>().0 {
            px.rotate_right(1);
        }
        vec![ksni::Icon { width: TRAY_PX as i32, height: TRAY_PX as i32, data }]
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip { title: self.title(), ..Default::default() }
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        build(&self.menu)
    }
}

fn build(entries: &[MenuEntry]) -> Vec<ksni::MenuItem<Sni>> {
    use ksni::menu::{StandardItem, SubMenu};
    entries
        .iter()
        .map(|e| match e {
            MenuEntry::Separator => ksni::MenuItem::Separator,
            MenuEntry::Submenu { text, items } => {
                SubMenu { label: text.clone(), submenu: build(items), ..Default::default() }.into()
            }
            MenuEntry::Item { id, text, enabled, key } => {
                let id = id.clone();
                StandardItem {
                    label: text.replace('_', "__"),
                    enabled: *enabled,
                    shortcut: key.as_ref().map(|k| vec![shortcut_parts(k)]).unwrap_or_default(),
                    activate: Box::new(move |t: &mut Sni| {
                        // The tray host may hand over the right to bring a window to the front.
                        let tok = ksni::current_xdg_activation_token();
                        log::debug!(target: "dyosnap", "tray item {id}, activation token: {}", tok.is_some());
                        if let (Some(tok), Ok(mut slot)) = (tok, ACTIVATION.lock()) {
                            *slot = Some((tok, std::time::Instant::now()));
                        }
                        if !id.is_empty() {
                            (t.sink)(id.clone());
                        }
                    }),
                    ..Default::default()
                }
                .into()
            }
        })
        .collect()
}

/// Shortcut as the dbusmenu spec writes it: ["Control", "Alt", "5"].
fn shortcut_parts(k: &Hotkey) -> Vec<String> {
    let mut v = Vec::new();
    if k.cmd || k.ctrl {
        v.push("Control".into());
    }
    if k.alt {
        v.push("Alt".into());
    }
    if k.shift {
        v.push("Shift".into());
    }
    v.push(keysym(&k.code));
    v
}

/// The activation token of the newest shortcut press, if the desktop sent one.
static ACTIVATION: Mutex<Option<(String, std::time::Instant)>> = Mutex::new(None);

/// The token of a shortcut pressed in the last seconds, once. A window opened with it gets
/// the focus; without one GNOME opens new windows behind or without the keyboard.
pub fn take_activation_token() -> Option<String> {
    let (t, at) = ACTIVATION.lock().ok()?.take()?;
    (at.elapsed() < std::time::Duration::from_secs(10)).then_some(t)
}

/// Keeps a token from another start of DyoSnap (see `instance.rs`) for the next window.
pub fn remember_activation_token(token: String) {
    if let Ok(mut slot) = ACTIVATION.lock() {
        *slot = Some((token, std::time::Instant::now()));
    }
}

pub struct Tray(Handle<Sni>);

impl Tray {
    pub fn new(menu: Vec<MenuEntry>, sink: Sink) -> Option<Self> {
        let sni = Sni { menu, recording: false, progress: (0, false), title: None, sink, icon_dir: write_icons() };
        match sni.spawn() {
            Ok(h) => Some(Self(h)),
            Err(e) => {
                log::warn!(target: "dyosnap", "tray: {e}");
                None
            }
        }
    }

    pub fn set_menu(&self, menu: Vec<MenuEntry>) {
        self.0.update(|t| t.menu = menu);
    }

    pub fn set_recording(&self, on: bool) {
        self.0.update(|t| {
            t.recording = on;
            t.progress = (0, false);
        });
    }

    /// The ring around the recording dot after `secs` seconds; orange while paused.
    pub fn set_progress(&self, secs: u64, paused: bool) {
        let p = (((secs % 60) / (60 / RING_STEPS as u64)) as u32, paused);
        self.0.update(|t| {
            if t.progress != p {
                t.progress = p;
            }
        });
    }

    pub fn set_title(&self, title: Option<&str>) {
        let title = title.map(str::to_string);
        self.0.update(|t| t.title = title);
    }
}

// ---------------------------------------------------------------- global shortcuts

/// Global shortcuts through the portal. Binding runs on a helper thread because the
/// desktop may show a dialog first.
pub struct Hotkeys {
    tx: Sender<Cmd>,
    muted: Arc<AtomicBool>,
    last: Vec<(String, String, Option<Hotkey>)>,
    live: Arc<Mutex<Option<HashMap<String, String>>>>,
    version: Arc<std::sync::atomic::AtomicU32>,
}

enum Cmd {
    Bind(Vec<(String, String, Option<Hotkey>)>),
    Configure,
}

/// Sent to the app's sink when the desktop's shortcuts changed (see `live_labels`).
pub const KEYS_CHANGED: &str = "keys-changed";

impl Hotkeys {
    pub fn new(sink: Sink) -> Option<Self> {
        super::portal::register();
        let (tx, rx) = channel::<Cmd>();
        let muted = Arc::new(AtomicBool::new(false));
        let live: Arc<Mutex<Option<HashMap<String, String>>>> = Default::default();
        let version = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let shared = Shared { sink, muted: muted.clone(), live: live.clone(), version: version.clone() };
        std::thread::spawn(move || pollster::block_on(run(rx, shared)));
        Some(Self { tx, muted, last: Vec::new(), live, version })
    }

    /// The shortcuts as the desktop has them (it may have kept or changed the keys
    /// DyoSnap asked for), by action id. None until they are bound.
    pub fn live_labels(&self) -> Option<HashMap<String, String>> {
        self.live.lock().ok()?.clone()
    }

    /// Whether the desktop can open its own page for changing the shortcuts.
    pub fn can_configure(&self) -> bool {
        self.version.load(Ordering::SeqCst) >= 2
    }

    /// Opens the desktop's page for changing DyoSnap's shortcuts.
    pub fn configure(&self) {
        let _ = self.tx.send(Cmd::Configure);
    }

    /// Replaces all shortcuts. Each item is (action id, description, key). Actions without a key are still
    /// announced, so the desktop lists them and the person can assign one. An empty list only mutes
    /// them (while a new key is being chosen in the settings): binding again would make
    /// the desktop ask again.
    pub fn set(&mut self, keys: &[(String, String, Option<Hotkey>)]) {
        if keys.is_empty() {
            self.muted.store(true, Ordering::SeqCst);
            return;
        }
        self.muted.store(false, Ordering::SeqCst);
        if keys != self.last.as_slice() {
            self.last = keys.to_vec();
            let _ = self.tx.send(Cmd::Bind(keys.to_vec()));
        }
    }
}

struct Shared {
    sink: Sink,
    muted: Arc<AtomicBool>,
    live: Arc<Mutex<Option<HashMap<String, String>>>>,
    version: Arc<std::sync::atomic::AtomicU32>,
}

impl Shared {
    fn set_live(&self, shortcuts: &[ashpd::desktop::global_shortcuts::Shortcut]) {
        let map: HashMap<String, String> = shortcuts
            .iter()
            .map(|s| (s.id().to_string(), tidy_trigger(s.trigger_description())))
            .collect();
        log::info!(target: "dyosnap", "global shortcuts: {map:?}");
        if let Ok(mut l) = self.live.lock() {
            *l = Some(map);
        }
        (self.sink)(KEYS_CHANGED.into());
    }
}

/// GNOME describes keys like "Press <Control><Alt>4"; show "Ctrl+Alt+4" like the rest of the app.
fn tidy_trigger(s: &str) -> String {
    let s = s.trim().strip_prefix("Press ").unwrap_or(s.trim());
    if !s.contains('<') {
        return s.to_string();
    }
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find('<') {
        let Some(end) = rest[start..].find('>') else { break };
        let m = &rest[start + 1..start + end];
        out.push_str(match m {
            "Control" | "Primary" | "Ctrl" => "Ctrl+",
            "Alt" => "Alt+",
            "Shift" => "Shift+",
            "Super" | "Meta" => "Super+",
            other => other,
        });
        rest = &rest[start + end + 1..];
    }
    let key = rest.trim();
    out + &if key.chars().count() == 1 { key.to_uppercase() } else { key.to_string() }
}

async fn run(rx: std::sync::mpsc::Receiver<Cmd>, shared: Shared) {
    use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
    use futures_util::StreamExt;
    let portal = match GlobalShortcuts::new().await {
        Ok(p) => p,
        Err(e) => {
            log::warn!(target: "dyosnap", "global shortcuts portal not available: {e}");
            return;
        }
    };
    shared.version.store(portal.version(), Ordering::SeqCst);
    let shared = Arc::new(shared);
    // Key presses and changes made in the desktop's settings arrive on their own threads.
    {
        let sh = shared.clone();
        std::thread::spawn(move || {
            pollster::block_on(async move {
                let Ok(p) = GlobalShortcuts::new().await else { return };
                let Ok(mut stream) = p.receive_activated().await else { return };
                while let Some(ev) = stream.next().await {
                    let id = ev.shortcut_id().to_string();
                    // The desktop may hand over the right to bring a window to the front.
                    let token = ev.options().get("activation_token").and_then(|v| <&str>::try_from(&**v).ok().map(str::to_string));
                    log::debug!(target: "dyosnap", "shortcut {id}, activation token: {}", token.is_some());
                    if let (Some(t), Ok(mut slot)) = (token, ACTIVATION.lock()) {
                        *slot = Some((t, std::time::Instant::now()));
                    }
                    let known = sh.live.lock().map(|l| l.as_ref().is_some_and(|m| m.contains_key(&id))).unwrap_or(false);
                    if known && !sh.muted.load(Ordering::SeqCst) {
                        (sh.sink)(id);
                    }
                }
            })
        });
        let sh = shared.clone();
        std::thread::spawn(move || {
            pollster::block_on(async move {
                let Ok(p) = GlobalShortcuts::new().await else { return };
                let Ok(mut stream) = p.receive_shortcuts_changed().await else { return };
                while let Some(ev) = stream.next().await {
                    sh.set_live(ev.shortcuts());
                }
            })
        });
    }
    let mut session: Option<ashpd::desktop::Session<GlobalShortcuts>> = None;
    while let Ok(cmd) = rx.recv() {
        let mut keys = match cmd {
            Cmd::Configure => {
                if let Some(s) = &session
                    && let Err(e) = portal.configure_shortcuts(s, None, Default::default()).await
                {
                    log::warn!(target: "dyosnap", "configure shortcuts: {e}");
                }
                continue;
            }
            Cmd::Bind(k) => k,
        };
        // Only the newest list counts.
        while let Ok(Cmd::Bind(k)) = rx.try_recv() {
            keys = k;
        }
        if let Some(s) = session.take() {
            let _ = s.close().await;
        }
        let Ok(s) = portal.create_session(Default::default()).await else { continue };
        let shortcuts: Vec<NewShortcut> = keys
            .iter()
            .map(|(id, description, k)| {
                let s = NewShortcut::new(id.as_str(), description.as_str());
                match k {
                    Some(k) => s.preferred_trigger(Some(trigger(k).as_str())),
                    None => s,
                }
            })
            .collect();
        match portal.bind_shortcuts(&s, &shortcuts, None, Default::default()).await.and_then(|r| r.response()) {
            Ok(r) => shared.set_live(r.shortcuts()),
            Err(e) => log::warn!(target: "dyosnap", "global shortcuts: {e}"),
        }
        session = Some(s);
    }
}

/// The key in the XDG shortcut format: "CTRL+ALT+5".
fn trigger(k: &Hotkey) -> String {
    let mut s = String::new();
    if k.cmd || k.ctrl {
        s.push_str("CTRL+");
    }
    if k.alt {
        s.push_str("ALT+");
    }
    if k.shift {
        s.push_str("SHIFT+");
    }
    s + &keysym(&k.code)
}

/// XKB key name for a W3C key code: "KeyA" → "a", "Digit5" → "5".
fn keysym(code: &str) -> String {
    if let Some(k) = code.strip_prefix("Key") {
        return k.to_lowercase();
    }
    if let Some(d) = code.strip_prefix("Digit") {
        return d.to_string();
    }
    match code {
        "Space" => "space",
        "Minus" => "minus",
        "Equal" => "equal",
        "Comma" => "comma",
        "Period" => "period",
        "Slash" => "slash",
        "Backslash" => "backslash",
        "Semicolon" => "semicolon",
        "Quote" => "apostrophe",
        "BracketLeft" => "bracketleft",
        "BracketRight" => "bracketright",
        "Backquote" => "grave",
        "ArrowUp" => "Up",
        "ArrowDown" => "Down",
        "ArrowLeft" => "Left",
        "ArrowRight" => "Right",
        "Enter" => "Return",
        "Escape" => "Escape",
        "Backspace" => "BackSpace",
        "Tab" => "Tab",
        "PrintScreen" => "Print",
        other => other, // F1..F24 and the rest keep their name
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    #[test]
    fn tidies_gnome_triggers() {
        assert_eq!(super::tidy_trigger("Press <Control><Alt>4"), "Ctrl+Alt+4");
        assert_eq!(super::tidy_trigger("<Shift><Super>s"), "Shift+Super+S");
        assert_eq!(super::tidy_trigger("Ctrl+Alt+6"), "Ctrl+Alt+6");
        assert_eq!(super::tidy_trigger(""), "");
    }
}
