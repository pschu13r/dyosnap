//! DyoSnap: a light screenshot and screen-recording app for macOS.
//! No cloud, no network: everything is saved locally or copied to the clipboard.

#![allow(deprecated)]

mod about;
mod app;
mod doc;
mod editor;
mod icons;
mod platform;
mod prefs;
mod recbar;
mod record;
mod settings;
mod video;

use std::path::PathBuf;

fn main() -> eframe::Result<()> {
    // First of all: nothing else may run (folders, logs, a second instance).
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("DyoSnap {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    // First: it moves LightSnap's folders, which the logs and settings below would
    // otherwise create anew.
    platform::migrate_old_folders();
    install_crash_log();
    install_warning_log();
    platform::prepare();
    let args: Vec<String> = std::env::args().collect();
    let screenshot = args
        .iter()
        .position(|a| a == "--screenshot")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from);
    // `--no-focus` opens like a check run but stays open (for measuring idle use).
    let no_focus = args.iter().any(|a| a == "--no-focus");
    let quiet = screenshot.is_some() || no_focus;
    // `--background` (used by "Launch at login") starts in the menu bar with no window.
    let mut background = args.iter().any(|a| a == "--background") && screenshot.is_none();
    // A picture or video given on the command line opens right away.
    let open = args
        .iter()
        .enumerate()
        .skip(1)
        .filter(|(i, a)| !a.starts_with("--") && args.get(i - 1).is_none_or(|p| p != "--screenshot"))
        .map(|(_, a)| PathBuf::from(a))
        .find(|p| p.is_file())
        // Absolute, so ffmpeg never reads a name like "x:y.mov" as a protocol.
        .map(|p| std::fs::canonicalize(&p).unwrap_or(p));

    // Start quietly in the menu bar; only the very first start shows the window.
    if screenshot.is_none() && !no_focus && open.is_none() {
        let mut st = settings::Settings::load();
        if st.welcomed {
            background = true;
        } else {
            st.welcomed = true;
            st.save();
        }
    }

    let make_options = move || {
        let mut viewport = egui::ViewportBuilder::default()
            .with_title("DyoSnap")
            .with_app_id(platform::APP_ID)
            .with_inner_size([980.0, 620.0])
            .with_min_inner_size([900.0, 440.0]);
        if background {
            viewport = viewport.with_visible(false);
        }
        if quiet {
            // Open without taking focus so a check never gets in the way.
            viewport = viewport.with_active(false);
            #[cfg(target_os = "windows")]
            {
                viewport = viewport.with_window_level(egui::WindowLevel::AlwaysOnBottom);
            }
        }
        let mut options = eframe::NativeOptions {
            viewport,
            // Only repaint when something happens: near-zero CPU while idle.
            run_and_return: false,
            ..Default::default()
        };
        options.event_loop_builder = Some(platform::event_loop_hook(quiet));
        options
    };

    #[cfg(target_os = "linux")]
    return run_linux(make_options, screenshot, open, quiet, background);

    #[cfg(not(target_os = "linux"))]
    eframe::run_native(
        "DyoSnap",
        make_options(),
        Box::new(move |cc| Ok(Box::new(app::DyoSnap::new(&cc.egui_ctx, screenshot, open, quiet, background)))),
    )
}

/// Wayland can't hide a window, so on Linux "hiding" closes it. eframe then returns
/// here, DyoSnap keeps running (tray, shortcuts, recording) and the window opens again
/// when it is needed. The same egui context is used every time, so the app's state and
/// its helper threads stay valid.
#[cfg(target_os = "linux")]
fn run_linux(
    make_options: impl Fn() -> eframe::NativeOptions,
    screenshot: Option<PathBuf>,
    open: Option<PathBuf>,
    quiet: bool,
    background: bool,
) -> eframe::Result<()> {
    use std::cell::RefCell;
    use std::rc::Rc;
    // Set up like eframe sets up its own context: real windows for extra viewports,
    // and up to two passes per frame.
    let ctx = egui::Context::default();
    ctx.set_embed_viewports(false);
    ctx.options_mut(|o| o.max_passes = std::num::NonZeroUsize::new(2).unwrap_or(std::num::NonZeroUsize::MIN));
    let state: Rc<RefCell<app::DyoSnap>> =
        Rc::new(RefCell::new(app::DyoSnap::new(&ctx, screenshot, open, quiet, background)));
    let mut first = true;
    loop {
        if !(first && background && !state.borrow().no_tray()) {
            let mut options = make_options();
            options.run_and_return = true;
            options.viewport = options.viewport.with_visible(true).with_inner_size(state.borrow().open_size());
            if state.borrow().selecting() {
                // The record area is chosen in this window, over the whole screen.
                options.viewport = options.viewport.with_fullscreen(true).with_decorations(false);
            }
            if let Some((size, title)) = state.borrow_mut().window_opening() {
                // Only Preferences is wanted: a window like Preferences' own.
                options.viewport = options
                    .viewport
                    .with_title(title)
                    .with_inner_size(size)
                    .with_resizable(false)
                    .with_minimize_button(false)
                    .with_maximize_button(false);
                options.viewport.min_inner_size = None;
            }
            let st = state.clone();
            let reattach = !first;
            platform::main_window_open(true);
            if let Some(token) = platform::take_activation_token() {
                // egui-winit hands it to the new window and removes the variable again.
                // SAFETY: set right before the window is made; no other thread of
                // DyoSnap reads this variable.
                unsafe { std::env::set_var("XDG_ACTIVATION_TOKEN", token) };
            }
            let result = eframe::run_native_ext(
                "DyoSnap",
                options,
                Some(ctx.clone()),
                Box::new(move |cc| {
                    if reattach {
                        st.borrow_mut().reattach(&cc.egui_ctx);
                    }
                    Ok(Box::new(app::Shared(st)))
                }),
            );
            platform::main_window_open(false);
            result?;
        }
        first = false;
        // No window: keep the app going until it needs one again.
        loop {
            std::thread::sleep(std::time::Duration::from_millis(100));
            let mut s = state.borrow_mut();
            s.tick_hidden(&ctx);
            if s.wants_window() {
                break;
            }
        }
    }
}

/// If DyoSnap ever stops because of an error, write what happened to
/// crash.log in the system's log folder (see `platform::log_dir`) so it can be found and fixed.
fn install_crash_log() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if EXPECTED_PANIC.with(|c| c.get()) {
            // Handled by show_immediate; note it so a window that misbehaves can be traced.
            log::warn!(target: "dyosnap", "skipped a frame of an extra window: {info}");
            return;
        }
        if let Some(dir) = platform::log_dir() {
            let _ = std::fs::create_dir_all(&dir);
            let text = format!(
                "--- {} ---\n{info}\n{}\n",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
                std::backtrace::Backtrace::force_capture()
            );
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("crash.log")) {
                let _ = f.write_all(text.as_bytes());
            }
        }
        default(info);
    }));
}

/// Warnings and errors from DyoSnap and the GUI libraries (plus DyoSnap's own notes)
/// go to dyosnap.log in the
/// log folder (see `platform::log_dir`). The file starts over when it passes 1 MB.
struct WarningLog(std::sync::Mutex<Option<std::fs::File>>);

impl log::Log for WarningLog {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Warn || (m.level() <= log::Level::Info && m.target().starts_with("dyosnap"))
    }
    fn log(&self, r: &log::Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        if let Ok(mut f) = self.0.lock() {
            if let Some(f) = f.as_mut() {
                use std::io::Write;
                let _ = writeln!(
                    f,
                    "{} {} {}: {}",
                    chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
                    r.level(),
                    r.target(),
                    r.args()
                );
            }
        }
    }
    fn flush(&self) {}
}

fn install_warning_log() {
    let Some(dir) = platform::log_dir() else { return };
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("dyosnap.log");
    if std::fs::metadata(&path).map(|m| m.len() > 1_000_000).unwrap_or(false) {
        let _ = std::fs::remove_file(&path);
    }
    let file = std::fs::OpenOptions::new().create(true).append(true).open(&path).ok();
    if log::set_boxed_logger(Box::new(WarningLog(std::sync::Mutex::new(file)))).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
}

thread_local! {
    static EXPECTED_PANIC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

static UPLOADS_LOST: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// True once after an extra window lost a frame that had already been drawn (see
/// show_immediate). Pictures and letters sent to the graphics card in that frame are
/// gone and have to be sent again.
pub fn take_uploads_lost() -> bool {
    UPLOADS_LOST.swap(false, std::sync::atomic::Ordering::SeqCst)
}

/// Shows an extra window (immediate viewport). While the main window is hidden,
/// eframe sometimes can't create a brand-new window in that frame and egui then stops
/// the app. Here that frame is simply skipped: the window gets created right after it,
/// and the next frame draws it.
///
/// A fresh window can also fail a bit later, while drawing ("Unable to create Texture
/// object": OpenGL is not ready for it yet). Then the pictures and letters of that frame
/// never reach the graphics card; take_uploads_lost() reports it so they can be resent.
pub fn show_immediate<T: Default>(
    ctx: &egui::Context,
    id: egui::ViewportId,
    builder: egui::ViewportBuilder,
    mut cb: impl FnMut(&mut egui::Ui, egui::ViewportClass) -> T,
) -> T {
    let mut drawn = false;
    EXPECTED_PANIC.with(|c| c.set(true));
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.show_viewport_immediate(id, builder, |ui, class| {
            drawn = true;
            cb(ui, class)
        })
    }));
    EXPECTED_PANIC.with(|c| c.set(false));
    match r {
        Ok(v) => v,
        Err(_) => {
            if drawn {
                UPLOADS_LOST.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
            T::default()
        }
    }
}
