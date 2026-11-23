//! The menu-bar icon and the global shortcuts (tray-icon and global-hotkey crates).

use crate::platform::{MenuEntry, Sink, TRAY_PX, tray_pixels};
use crate::settings::Hotkey;
use global_hotkey::hotkey::HotKey;
use tray_icon::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};

pub struct Tray(tray_icon::TrayIcon);

impl Tray {
    pub fn new(menu: Vec<MenuEntry>, sink: Sink) -> Option<Self> {
        tray_icon::menu::MenuEvent::set_event_handler(Some(move |e: tray_icon::menu::MenuEvent| sink(e.id.0.clone())));
        let mut b = tray_icon::TrayIconBuilder::new()
            .with_menu(Box::new(build(&menu)))
            .with_tooltip("DyoSnap")
            .with_icon_as_template(true);
        if let Some(i) = icon(false) {
            b = b.with_icon(i);
        }
        b.build().ok().map(Self)
    }

    pub fn set_menu(&self, menu: Vec<MenuEntry>) {
        self.0.set_menu(Some(Box::new(build(&menu))));
    }

    /// Switches between the camera and the red recording dot.
    pub fn set_recording(&self, on: bool) {
        let _ = self.0.set_icon_with_as_template(icon(on), !on);
    }

    /// Text next to the icon (the recording timer).
    pub fn set_title(&self, title: Option<&str>) {
        self.0.set_title(title);
    }
}

fn icon(recording: bool) -> Option<tray_icon::Icon> {
    tray_icon::Icon::from_rgba(tray_pixels(recording, false), TRAY_PX, TRAY_PX).ok()
}

fn build(entries: &[MenuEntry]) -> Menu {
    let menu = Menu::new();
    for e in entries {
        match e {
            MenuEntry::Separator => {
                let _ = menu.append(&PredefinedMenuItem::separator());
            }
            MenuEntry::Item { .. } => {
                let _ = menu.append(&item(e));
            }
            MenuEntry::Submenu { text, items } => {
                let _ = menu.append(&submenu(text, items));
            }
        }
    }
    menu
}

fn submenu(text: &str, entries: &[MenuEntry]) -> Submenu {
    let sub = Submenu::new(text, true);
    for e in entries {
        match e {
            MenuEntry::Separator => {
                let _ = sub.append(&PredefinedMenuItem::separator());
            }
            MenuEntry::Item { .. } => {
                let _ = sub.append(&item(e));
            }
            MenuEntry::Submenu { text, items } => {
                let _ = sub.append(&submenu(text, items));
            }
        }
    }
    sub
}

fn item(e: &MenuEntry) -> MenuItem {
    let MenuEntry::Item { id, text, enabled, key } = e else { unreachable!() };
    let acc = key.as_ref().and_then(to_accelerator);
    if id.is_empty() { MenuItem::new(text, *enabled, acc) } else { MenuItem::with_id(id.as_str(), text, *enabled, acc) }
}

fn to_accelerator(k: &Hotkey) -> Option<tray_icon::menu::accelerator::Accelerator> {
    use tray_icon::menu::accelerator::{Accelerator, Code, Modifiers as M};
    let code: Code = k.code.parse().ok()?;
    let mut m = M::empty();
    m.set(M::META, k.cmd);
    m.set(M::ALT, k.alt);
    m.set(M::CONTROL, k.ctrl);
    m.set(M::SHIFT, k.shift);
    Some(Accelerator::new(m, code))
}

/// Global shortcuts that work while another app is in front.
pub struct Hotkeys {
    mgr: global_hotkey::GlobalHotKeyManager,
    registered: std::sync::Arc<std::sync::Mutex<Vec<(u32, String)>>>,
    keys: Vec<HotKey>,
}

impl Hotkeys {
    pub fn new(sink: Sink) -> Option<Self> {
        let mgr = global_hotkey::GlobalHotKeyManager::new().ok()?;
        let registered: std::sync::Arc<std::sync::Mutex<Vec<(u32, String)>>> = Default::default();
        let reg = registered.clone();
        global_hotkey::GlobalHotKeyEvent::set_event_handler(Some(move |e: global_hotkey::GlobalHotKeyEvent| {
            if e.state != global_hotkey::HotKeyState::Pressed {
                return;
            }
            let id = reg.lock().ok().and_then(|r| r.iter().find(|(i, _)| *i == e.id).map(|(_, a)| a.clone()));
            if let Some(id) = id {
                sink(id);
            }
        }));
        Some(Self { mgr, registered, keys: Vec::new() })
    }

    /// The shortcuts as the system has them. macOS uses DyoSnap's own settings.
    pub fn live_labels(&self) -> Option<std::collections::HashMap<String, String>> {
        None
    }

    /// Whether the system can open its own page for changing the shortcuts.
    pub fn can_configure(&self) -> bool {
        false
    }

    pub fn configure(&self) {}

    /// Replaces all shortcuts. Each item is (action id, description, key). Actions without a
    /// key are skipped. The description is for desktops that list the shortcuts (Linux).
    pub fn set(&mut self, keys: &[(String, String, Option<Hotkey>)]) {
        let _ = self.mgr.unregister_all(&self.keys);
        self.keys.clear();
        let mut reg = Vec::new();
        for (id, _, k) in keys {
            let Some(hk) = k.as_ref().and_then(to_global_hotkey) else { continue };
            if self.mgr.register(hk).is_ok() {
                self.keys.push(hk);
                reg.push((hk.id(), id.clone()));
            }
        }
        if let Ok(mut r) = self.registered.lock() {
            *r = reg;
        }
    }
}

fn to_global_hotkey(k: &Hotkey) -> Option<HotKey> {
    use global_hotkey::hotkey::{Code, Modifiers as M};
    let code: Code = k.code.parse().ok()?;
    let mut m = M::empty();
    m.set(M::META, k.cmd);
    m.set(M::ALT, k.alt);
    m.set(M::CONTROL, k.ctrl);
    m.set(M::SHIFT, k.shift);
    Some(HotKey::new(if m.is_empty() { None } else { Some(m) }, code))
}
