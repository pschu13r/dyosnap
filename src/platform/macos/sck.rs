//! Screen pictures taken inside the process with ScreenCaptureKit, to choose an area on.
//! That is faster than starting `screencapture` and reading a BMP of 24 MB from the disk.
//! DyoSnap's own windows are left out of the picture, so there is no need to wait for them
//! to vanish. Any error returns `Err`: the caller then falls back to `screencapture`.

use block2::RcBlock;
use egui::ColorImage;
use objc2::AnyThread;
use objc2::rc::Retained;
use objc2_core_foundation::CFRetained;
use objc2_core_graphics::{CGDataProvider, CGImage, CGImageAlphaInfo, CGImageByteOrderInfo};
use objc2_foundation::{NSArray, NSError};
use objc2_screen_capture_kit::{SCContentFilter, SCDisplay, SCRunningApplication, SCScreenshotManager, SCShareableContent, SCStreamConfiguration, SCWindow};
use std::ptr::NonNull;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// How long to wait for ScreenCaptureKit to answer, for the screens and for each picture.
const TIMEOUT: Duration = Duration::from_secs(2);

/// 'BGRA', the pixel format of the pictures.
const PIXEL_FORMAT_BGRA: u32 = 0x4247_5241;

/// Moves an Objective-C object to another thread. ScreenCaptureKit hands these over in
/// completion handlers on its own queues; they are not changed after that.
struct SendPtr<T>(T);
// SAFETY: see above; the wrapped objects (shareable content, CGImage) are immutable.
unsafe impl<T> Send for SendPtr<T> {}

/// What the last "shareable content" call returned: the screens and DyoSnap's own application.
struct Content {
    displays: Retained<NSArray<SCDisplay>>,
    own: Option<Retained<SCRunningApplication>>,
}

/// Kept between captures: asking for it costs time (see `content`). Never used for a screen it
/// does not list, or one whose size changed since.
static CACHE: Mutex<Option<SendPtr<Content>>> = Mutex::new(None);

fn cache() -> MutexGuard<'static, Option<SendPtr<Content>>> {
    CACHE.lock().unwrap_or_else(|e| e.into_inner())
}

fn error_text(e: *mut NSError) -> String {
    // SAFETY: ScreenCaptureKit passes a valid NSError or null.
    unsafe { e.as_ref() }.map_or_else(|| "no reason given".to_string(), |e| e.localizedDescription().to_string())
}

/// Asks ScreenCaptureKit for the screens and applications. Blocks, at most `TIMEOUT`.
fn fetch_content() -> Result<Content, String> {
    let (tx, rx) = channel();
    let block = RcBlock::new(move |c: *mut SCShareableContent, e: *mut NSError| {
        // SAFETY: `c` is valid inside the handler; it is retained to outlive it.
        let r = match unsafe { Retained::retain(c) } {
            Some(c) => Ok(SendPtr(c)),
            None => Err(error_text(e)),
        };
        let _ = tx.send(r);
    });
    // SAFETY: the block has the signature the method expects.
    unsafe { SCShareableContent::getShareableContentExcludingDesktopWindows_onScreenWindowsOnly_completionHandler(true, false, &block) };
    let content = rx.recv_timeout(TIMEOUT).map_err(|_| "ScreenCaptureKit did not answer".to_string())??.0;
    // SAFETY: plain getters of the object just received.
    unsafe {
        let apps = content.applications();
        let pid = std::process::id() as i32;
        let own = (0..apps.count())
            .map(|i| apps.objectAtIndex(i))
            .find(|a| a.processID() == pid);
        Ok(Content { displays: content.displays(), own })
    }
}

/// The ScreenCaptureKit screen for the CoreGraphics id `id`, and DyoSnap's own application.
/// From the cache if it still describes that screen, else asked for again.
fn find_display(id: u32, need_own: bool) -> Result<(Retained<SCDisplay>, Option<Retained<SCRunningApplication>>), String> {
    let (w, h) = super::display_point_size(id);
    let look = |c: &Content| {
        // SAFETY: plain getters.
        unsafe {
            (0..c.displays.count())
                .map(|i| c.displays.objectAtIndex(i))
                .find(|d| d.displayID() == id && (d.width() as f64 - w).abs() < 1.0 && (d.height() as f64 - h).abs() < 1.0)
        }
    };
    // A cache that does not know DyoSnap is no good: it may have windows now.
    if let Some(c) = cache().as_ref() && (c.0.own.is_some() || !need_own) && let Some(d) = look(&c.0) {
        return Ok((d, c.0.own.clone()));
    }
    let t0 = Instant::now();
    let fresh = fetch_content()?;
    log::info!(target: "dyosnap", "select: ScreenCaptureKit screens fetched in {} ms, own app listed: {}", t0.elapsed().as_millis(), fresh.own.is_some());
    let found = look(&fresh).map(|d| (d, fresh.own.clone()));
    let own_missing = need_own && fresh.own.is_none();
    *cache() = Some(SendPtr(fresh));
    if own_missing {
        return Err("DyoSnap is not in the list of applications".to_string());
    }
    found.ok_or_else(|| format!("screen {id} is not in the list"))
}

/// Starts the picture of screen `id`; the result arrives on the channel.
/// A picture on its way.
type Pending = Receiver<Result<SendPtr<CFRetained<CGImage>>, String>>;

fn start(id: u32, need_own: bool) -> Result<Pending, String> {
    let (display, own) = find_display(id, need_own)?;
    let (w, h) = super::display_pixel_size(id).ok_or("no size for the screen")?;
    let (tx, rx) = channel();
    let block = RcBlock::new(move |img: *mut CGImage, e: *mut NSError| {
        // SAFETY: `img` is valid inside the handler; it is retained to outlive it.
        let r = match NonNull::new(img) {
            Some(p) => Ok(SendPtr(unsafe { CFRetained::retain(p) })),
            None => Err(error_text(e)),
        };
        let _ = tx.send(r);
    });
    // SAFETY: the objects are valid and the block has the signature the method expects.
    unsafe {
        let own = NSArray::from_retained_slice(&own.into_iter().collect::<Vec<_>>());
        let none = NSArray::<SCWindow>::new();
        let filter = SCContentFilter::initWithDisplay_excludingApplications_exceptingWindows(SCContentFilter::alloc(), &display, &own, &none);
        let config = SCStreamConfiguration::new();
        config.setWidth(w);
        config.setHeight(h);
        config.setShowsCursor(false);
        config.setScalesToFit(false);
        config.setPixelFormat(PIXEL_FORMAT_BGRA);
        // sRGB, as the BMP of `screencapture` is: without it the pictures are in the screen's
        // own colour space and look different where they were cut out.
        config.setColorSpaceName(objc2_core_graphics::kCGColorSpaceSRGB);
        SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(&filter, &config, Some(&block));
    }
    Ok(rx)
}

/// The pixels of a picture as RGBA, opaque.
fn to_image(img: &CGImage) -> Result<ColorImage, String> {
    let (w, h) = (CGImage::width(Some(img)), CGImage::height(Some(img)));
    let stride = CGImage::bytes_per_row(Some(img));
    let info = CGImage::bitmap_info(Some(img));
    let order = CGImage::byte_order_info(Some(img));
    let alpha = CGImage::alpha_info(Some(img));
    // Blue, green, red, then alpha or nothing: what 'BGRA' gives.
    let bgra = CGImage::bits_per_pixel(Some(img)) == 32
        && CGImage::bits_per_component(Some(img)) == 8
        && order == CGImageByteOrderInfo::Order32Little
        && matches!(alpha, CGImageAlphaInfo::PremultipliedFirst | CGImageAlphaInfo::NoneSkipFirst | CGImageAlphaInfo::First);
    if !bgra || w == 0 || h == 0 || stride < w * 4 {
        return Err(format!("unexpected picture format: {w}x{h}, stride {stride}, bitmap info {:#x}", info.0));
    }
    let provider = CGImage::data_provider(Some(img)).ok_or("the picture has no data")?;
    let data = CGDataProvider::data(Some(&provider)).ok_or("the picture has no data")?;
    // SAFETY: `byte_ptr` points at `length` bytes that live as long as `data`.
    let bytes = unsafe { std::slice::from_raw_parts(data.byte_ptr(), data.length() as usize) };
    if bytes.len() < stride * (h - 1) + w * 4 {
        return Err("the picture data is too short".to_string());
    }
    let mut pixels = Vec::with_capacity(w * h);
    for row in bytes.chunks(stride).take(h) {
        pixels.extend(row[..w * 4].as_chunks::<4>().0.iter().map(|p| egui::Color32::from_rgb(p[2], p[1], p[0])));
    }
    Ok(ColorImage { size: [w, h], source_size: egui::vec2(w as f32, h as f32), pixels })
}

/// Takes a silent picture of each of the screens `indices` (as in `displays()`), without the
/// mouse pointer and without DyoSnap's own windows. All pictures are taken at the same time.
pub fn capture_images(indices: &[usize]) -> Result<Vec<ColorImage>, String> {
    capture(indices, true)
}

/// `capture_images`; `need_own`: fail if DyoSnap is not among the applications, as its windows
/// could not be left out then (a test has no windows).
fn capture(indices: &[usize], need_own: bool) -> Result<Vec<ColorImage>, String> {
    let ids = super::display_ids();
    let jobs = indices
        .iter()
        .map(|&i| ids.get(i).copied().ok_or_else(|| format!("no screen {i}")).and_then(|id| start(id, need_own)))
        .collect::<Result<Vec<_>, _>>()?;
    jobs.into_iter()
        .map(|rx| {
            let img = rx.recv_timeout(TIMEOUT).map_err(|_| "ScreenCaptureKit gave no picture".to_string())??.0;
            to_image(&img)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs a screen and the permission to capture it: run with `--ignored`. Compares the
    /// picture of ScreenCaptureKit with the BMP of `screencapture` for the same screen: the
    /// size and the colours must be the same (two captures differ a little: menu bar blur,
    /// clock), and prints how long each step takes.
    #[test]
    #[ignore]
    fn sck_matches_bmp() {
        let t0 = Instant::now();
        let content = fetch_content().unwrap();
        println!("screens fetched first time: {} ms (own app listed: {})", t0.elapsed().as_millis(), content.own.is_some());
        let t0 = Instant::now();
        let first = capture(&[0], false);
        println!("first picture (with screens): {} ms", t0.elapsed().as_millis());
        let img = first.expect("ScreenCaptureKit failed: no Screen Recording permission for the terminal?");
        for _ in 0..3 {
            let t = Instant::now();
            let again = capture(&[0], false).unwrap();
            println!("next picture (cached screens): {} ms", t.elapsed().as_millis());
            assert_eq!(again[0].size, img[0].size);
        }
        let t = Instant::now();
        let content = fetch_content().unwrap();
        println!("screens fetched again: {} ms ({} screens)", t.elapsed().as_millis(), content.displays.count());
        let bmp = |name: &str| {
            let file = std::env::temp_dir().join(name);
            let t = Instant::now();
            let ok = std::process::Command::new("/usr/sbin/screencapture").args(["-x", "-t", "bmp", "-D", "1"]).arg(&file).status().unwrap().success();
            println!("screencapture + file: {} ms", t.elapsed().as_millis());
            assert!(ok);
            let img = image::open(&file).unwrap().to_rgba8();
            let _ = std::fs::remove_file(&file);
            img
        };
        // The picture of ScreenCaptureKit is taken between the two BMPs.
        let reference = bmp("dyosnap-test-sck1.bmp");
        let img = capture(&[0], false).unwrap();
        let second = bmp("dyosnap-test-sck2.bmp");
        let sck = &img[0];
        assert_eq!([reference.width() as usize, reference.height() as usize], sck.size, "different size");
        let [w, h] = sck.size;
        // Mean and median difference per colour value, below the menu bar (it is blurred, so it
        // changes with the wallpaper behind it). The BMP against a second BMP is the noise.
        let s = |x: usize, y: usize| { let p = sck.pixels[y * w + x].to_srgba_unmultiplied(); [p[0], p[1], p[2]] };
        let r = |x: usize, y: usize| { let p = reference.get_pixel(x as u32, y as u32).0; [p[0], p[1], p[2]] };
        let n = |x: usize, y: usize| { let p = second.get_pixel(x as u32, y as u32).0; [p[0], p[1], p[2]] };
        let stats = |a: &dyn Fn(usize, usize) -> [u8; 3], b: &dyn Fn(usize, usize) -> [u8; 3]| {
            let mut diffs = Vec::new();
            for y in (120..h).step_by(5) {
                for x in (0..w).step_by(5) {
                    // Only where the two BMPs agree: the rest is moving content.
                    if r(x, y) != n(x, y) {
                        continue;
                    }
                    let (p, q) = (a(x, y), b(x, y));
                    diffs.extend((0..3).map(|c| p[c].abs_diff(q[c])));
                }
            }
            diffs.sort_unstable();
            (diffs.iter().map(|d| *d as f64).sum::<f64>() / diffs.len() as f64, diffs[diffs.len() / 2], diffs[diffs.len() * 99 / 100])
        };
        let (noise, sck_diff) = (stats(&r, &n), stats(&s, &r));
        println!("bmp vs bmp: mean {:.2} median {} p99 {}", noise.0, noise.1, noise.2);
        println!("sck vs bmp: mean {:.2} median {} p99 {}", sck_diff.0, sck_diff.1, sck_diff.2);
        for (x, y) in [(w - 1, h - 1), (w / 2, h / 2), (w / 4, h / 3), (w / 3, h * 3 / 4)] {
            println!("{x},{y}: sck {:?} bmp {:?}", s(x, y), r(x, y));
        }
        assert!(sck_diff.0 < noise.0 + 2.0 && sck_diff.1 <= noise.1 + 2, "colours differ");
    }
}
