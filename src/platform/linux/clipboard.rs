//! The clipboard, through arboard (Wayland data-control, else XWayland). On GNOME, which
//! has no data-control, copying goes through plain Wayland while DyoSnap has the focus
//! (see `wl_clipboard`), and through arboard only otherwise. The clipboard objects stay
//! alive: on Linux the copied data is served by this app until something else is copied.

use std::path::Path;
use std::sync::Mutex;

static CLIPBOARD: Mutex<Option<arboard::Clipboard>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut arboard::Clipboard) -> Option<R>) -> Option<R> {
    let mut g = CLIPBOARD.lock().ok()?;
    if g.is_none() {
        *g = arboard::Clipboard::new().map_err(|e| log::warn!(target: "dyosnap", "clipboard: {e}")).ok();
    }
    f(g.as_mut()?)
}

const TEXT_MIMES: [&str; 5] = ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain", "TEXT", "STRING"];

fn wayland_text(text: &str) -> bool {
    super::wl_clipboard::set(super::wl_clipboard::Offer::same(&TEXT_MIMES, text.as_bytes().to_vec()))
}

/// "file:///home/a/Bild%201.png": the path with everything but plain characters escaped.
fn file_uri(p: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut s = String::from("file://");
    for &b in p.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    s
}

/// Copies plain text to the clipboard.
pub fn copy_text(text: &str) {
    if !wayland_text(text) {
        with(|c| c.set_text(text).ok());
    }
}

/// Writes the picture on the clipboard to a PNG file. Returns false if there is none.
pub fn clipboard_png_to_file(dest: &Path) -> bool {
    if let Some(bytes) = super::wl_clipboard::read(&["image/png"]) {
        return std::fs::write(dest, bytes).is_ok();
    }
    let Some(img) = with(|c| c.get_image().ok()) else { return false };
    let Some(buf) = image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.into_owned()) else {
        return false;
    };
    buf.save_with_format(dest, image::ImageFormat::Png).is_ok()
}

/// Puts an image or, without one, a file path on the clipboard. Linux apps take one or
/// the other, so the picture wins. `tiff` is a macOS option and ignored here.
pub fn set_clipboard(png: Option<&Path>, _tiff: bool, path: Option<&Path>) -> bool {
    if let Some(png) = png {
        if let Ok(bytes) = std::fs::read(png) {
            let offer = super::wl_clipboard::Offer::same(&["image/png"], bytes);
            if super::wl_clipboard::set(offer) {
                return true;
            }
        }
        if let Ok(img) = image::open(png) {
            let img = img.to_rgba8();
            let data = arboard::ImageData {
                width: img.width() as usize,
                height: img.height() as usize,
                bytes: img.into_raw().into(),
            };
            return with(|c| c.set_image(data).ok()).is_some();
        }
    }
    match path {
        Some(p) => wayland_text(&p.display().to_string()) || with(|c| c.set_text(p.display().to_string()).ok()).is_some(),
        None => false,
    }
}

/// Puts a file reference on the clipboard (paste in the file manager copies the file).
pub fn copy_file_ref(p: &Path) -> bool {
    let uri = file_uri(p);
    let offer = super::wl_clipboard::Offer {
        items: vec![
            ("text/uri-list".into(), std::sync::Arc::new(format!("{uri}\r\n").into_bytes())),
            // GNOME Files pastes this as "copy the file".
            ("x-special/gnome-copied-files".into(), std::sync::Arc::new(format!("copy\n{uri}").into_bytes())),
            ("text/plain;charset=utf-8".into(), std::sync::Arc::new(p.display().to_string().into_bytes())),
        ],
    };
    super::wl_clipboard::set(offer) || with(|c| c.set().file_list(&[p]).ok()).is_some()
}
