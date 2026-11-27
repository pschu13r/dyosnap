//! The clipboard through plain Wayland (wl_data_device), for GNOME.
//!
//! arboard needs a data-control protocol to set the clipboard from the background. KDE,
//! Sway, Hyprland and COSMIC have one; GNOME has none, and there arboard falls back to X11
//! through XWayland. Without XWayland nothing is copied.
//!
//! Every Wayland app may set the clipboard while one of its windows has the keyboard
//! focus, with the serial of a recent input event. DyoSnap copies a screenshot right
//! when the mouse is let go on its area selection, which has the focus then. This module
//! listens to the seat on the main window's own Wayland connection (`attach`), so it
//! has that serial, and serves the copied data until something else is copied. Reading
//! the clipboard needs the focus too: the compositor tells only the focused app what is
//! on it.

use std::io::Write;
use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use wayland_backend::client::Backend;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{
    wl_data_device, wl_data_device_manager, wl_data_offer, wl_data_source, wl_keyboard, wl_pointer, wl_registry, wl_seat,
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum, event_created_child};

/// What to put on the clipboard: bytes per type.
pub struct Offer {
    pub items: Vec<(String, Arc<Vec<u8>>)>,
}

impl Offer {
    /// The same bytes under several types.
    pub fn same(mimes: &[&str], data: Vec<u8>) -> Self {
        let data = Arc::new(data);
        Offer { items: mimes.iter().map(|m| (m.to_string(), data.clone())).collect() }
    }
}

enum Request {
    Set(Offer, mpsc::SyncSender<bool>),
    /// Read the clipboard in the first of these types it has.
    Read(Vec<String>, mpsc::SyncSender<Option<Vec<u8>>>),
}

/// The types an offer from another app has, filled as the compositor lists them.
type OfferMimes = Arc<Mutex<Vec<String>>>;
/// One served format: its MIME type and the bytes.
type OfferItem = (String, Arc<Vec<u8>>);

struct Attached {
    display: usize,
    tx: mpsc::Sender<Request>,
    /// The compositor has a data-control protocol: arboard can copy without focus there.
    data_control: bool,
}

static ATTACHED: Mutex<Option<Attached>> = Mutex::new(None);
/// A surface of DyoSnap has the keyboard focus (as the compositor last said).
static FOCUSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether copying can work now: the desktop needs no focus for it (data-control), or
/// DyoSnap has the focus.
pub fn ready() -> bool {
    let needs_focus = ATTACHED.lock().ok().is_some_and(|g| g.as_ref().is_some_and(|a| !a.data_control));
    !needs_focus || FOCUSED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Like `ready`, but false until the main window's connection is known: before that it is
/// not known yet whether the desktop needs the focus.
pub fn ready_known() -> bool {
    ATTACHED.lock().ok().is_some_and(|g| g.is_some()) && ready()
}

/// Starts listening on the main window's Wayland connection. Called every frame; does
/// work only the first time (eframe keeps one connection for the whole run).
pub fn attach(window: &winit::window::Window) {
    use winit::raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
    let Ok(handle) = window.display_handle() else { return };
    let RawDisplayHandle::Wayland(h) = handle.as_raw() else { return };
    let display = h.display.as_ptr() as usize;
    let Ok(mut g) = ATTACHED.lock() else { return };
    if g.as_ref().is_some_and(|a| a.display == display) {
        return;
    }
    // SAFETY: the display belongs to winit's event loop, which eframe keeps for the whole
    // run of the app (a thread-local in eframe's `with_event_loop`).
    let backend = unsafe { Backend::from_foreign_display(display as *mut _) };
    let conn = Connection::from_backend(backend);
    let (tx, rx) = mpsc::channel();
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let spawned = std::thread::Builder::new().name("wl-clipboard".into()).spawn(move || {
        if let Err(e) = run(conn, rx, &ready_tx) {
            log::warn!(target: "dyosnap", "wayland clipboard: {e}");
            let _ = ready_tx.try_send(None);
        }
    });
    if spawned.is_err() {
        return;
    }
    if let Ok(Some(data_control)) = ready_rx.recv_timeout(Duration::from_secs(1)) {
        log::info!(target: "dyosnap", "wayland clipboard attached (data-control: {data_control})");
        *g = Some(Attached { display, tx, data_control });
    }
}

/// Sets the clipboard through Wayland, if that is the way on this desktop (no data-control)
/// and DyoSnap has the focus. False means: use arboard.
pub fn set(offer: Offer) -> bool {
    let tx = {
        let Ok(g) = ATTACHED.lock() else { return false };
        match g.as_ref() {
            Some(a) if !a.data_control => a.tx.clone(),
            _ => return false,
        }
    };
    let (done_tx, done_rx) = mpsc::sync_channel(1);
    if tx.send(Request::Set(offer, done_tx)).is_err() {
        return false;
    }
    done_rx.recv_timeout(Duration::from_millis(500)).unwrap_or(false)
}

/// Reads the clipboard through Wayland, in the first of `mimes` it has, where that is the
/// way (no data-control) and DyoSnap has the focus. None means: nothing there, or use arboard.
pub fn read(mimes: &[&str]) -> Option<Vec<u8>> {
    let tx = {
        let g = ATTACHED.lock().ok()?;
        match g.as_ref() {
            Some(a) if !a.data_control => a.tx.clone(),
            _ => return None,
        }
    };
    let (done_tx, done_rx) = mpsc::sync_channel(1);
    tx.send(Request::Read(mimes.iter().map(|m| m.to_string()).collect(), done_tx)).ok()?;
    done_rx.recv_timeout(Duration::from_secs(3)).ok().flatten()
}

struct State {
    seat: Option<wl_seat::WlSeat>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    pointer: Option<wl_pointer::WlPointer>,
    device: Option<wl_data_device::WlDataDevice>,
    /// A surface of this app has the keyboard focus.
    focused: bool,
    /// The serial of the newest input event on this app's surfaces.
    serial: Option<u32>,
    /// The data being served, with its source.
    current: Option<(wl_data_source::WlDataSource, Vec<OfferItem>)>,
    /// What is on the clipboard now (told only while focused).
    selection: Option<wl_data_offer::WlDataOffer>,
}

fn run(conn: Connection, rx: mpsc::Receiver<Request>, ready: &mpsc::SyncSender<Option<bool>>) -> Result<(), String> {
    let (globals, mut queue) = registry_queue_init::<State>(&conn).map_err(|e| e.to_string())?;
    let qh = queue.handle();
    let data_control = globals.contents().with_list(|list| {
        list.iter().any(|g| g.interface == "ext_data_control_manager_v1" || g.interface == "zwlr_data_control_manager_v1")
    });
    let manager: wl_data_device_manager::WlDataDeviceManager = globals.bind(&qh, 1..=3, ()).map_err(|e| e.to_string())?;
    let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=5, ()).map_err(|e| e.to_string())?;
    let mut st = State {
        device: Some(manager.get_data_device(&seat, &qh, ())),
        seat: Some(seat),
        keyboard: None,
        pointer: None,
        focused: false,
        serial: None,
        current: None,
        selection: None,
    };
    queue.roundtrip(&mut st).map_err(|e| e.to_string())?;
    let _ = ready.try_send(Some(data_control));
    loop {
        while let Ok(req) = rx.try_recv() {
            // Take in events that winit read already, so serial and selection are current.
            let rt = queue.roundtrip(&mut st);
            log::debug!(target: "dyosnap", "clipboard request: focused {} serial {:?} roundtrip {:?}", st.focused, st.serial, rt.is_ok());
            match req {
                Request::Set(offer, done) => {
                    let ok = match (st.focused, st.serial, &st.device) {
                        (true, Some(serial), Some(device)) => {
                            let source = manager.create_data_source(&qh, ());
                            for (m, _) in &offer.items {
                                source.offer(m.clone());
                            }
                            device.set_selection(Some(&source), serial);
                            if let Some((old, _)) = st.current.replace((source, offer.items)) {
                                old.destroy();
                            }
                            conn.flush().is_ok()
                        }
                        _ => false,
                    };
                    let _ = done.send(ok);
                }
                Request::Read(wanted, done) => {
                    let offer = st.selection.as_ref().filter(|_| st.focused);
                    let have = offer.and_then(|o| o.data::<OfferMimes>()).and_then(|m| m.lock().ok().map(|m| m.clone()));
                    let mime = have.and_then(|have| wanted.into_iter().find(|w| have.contains(w)));
                    match (offer, mime) {
                        (Some(offer), Some(mime)) => match receive(offer, &mime, &conn) {
                            // Read in a thread: when DyoSnap itself owns the clipboard,
                            // this loop has to answer the request meanwhile.
                            Some(file) => {
                                std::thread::spawn(move || {
                                    let _ = done.send(read_all(file));
                                });
                            }
                            None => {
                                let _ = done.send(None);
                            }
                        },
                        _ => {
                            let _ = done.send(None);
                        }
                    }
                }
            }
        }
        queue.dispatch_pending(&mut st).map_err(|e| e.to_string())?;
        conn.flush().map_err(|e| e.to_string())?;
        // Wait for events, but not long: winit may read them for us on its thread.
        if let Some(guard) = queue.prepare_read() {
            let mut fd = libc::pollfd { fd: guard.connection_fd().as_raw_fd(), events: libc::POLLIN, revents: 0 };
            // SAFETY: one valid pollfd; the fd stays open while the guard lives.
            if unsafe { libc::poll(&mut fd, 1, 50) } > 0 {
                let _ = guard.read();
            }
        }
    }
}

/// Asks the owner of the clipboard to write it into a new pipe; returns the reading end.
fn receive(offer: &wl_data_offer::WlDataOffer, mime: &str, conn: &Connection) -> Option<std::fs::File> {
    use std::os::fd::{AsFd, FromRawFd};
    let mut fds = [0; 2];
    // SAFETY: pipe2 fills two new fds, which are owned right after.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return None;
    }
    // SAFETY: both fds are new and owned only here.
    let (r, w) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    offer.receive(mime.to_string(), w.as_fd());
    conn.flush().ok()?;
    // Close our writing end, so the read ends when the owner closes theirs.
    drop(w);
    Some(std::fs::File::from(r))
}

/// Reads a pipe to its end, at most 10 s and 100 MB.
fn read_all(mut f: std::fs::File) -> Option<Vec<u8>> {
    use std::io::Read;
    let start = std::time::Instant::now();
    let mut out = Vec::new();
    let mut buf = [0u8; 65536];
    loop {
        let mut p = libc::pollfd { fd: f.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        // SAFETY: one valid pollfd.
        if unsafe { libc::poll(&mut p, 1, 1000) } <= 0 {
            if start.elapsed() > Duration::from_secs(10) {
                return None;
            }
            continue;
        }
        match f.read(&mut buf) {
            Ok(0) => return (!out.is_empty()).then_some(out),
            Ok(n) => {
                out.extend_from_slice(&buf[..n]);
                if out.len() > 100 << 20 {
                    return None;
                }
            }
            Err(_) => return None,
        }
    }
}

/// Writes the copied bytes into the pipe of the app that pastes. In a thread: the other
/// app may read slowly, and DyoSnap must keep answering.
fn serve(fd: OwnedFd, data: Arc<Vec<u8>>) {
    std::thread::spawn(move || {
        // Mutter hands out a non-blocking pipe: a full pipe would end the write after the
        // first 64 KB. Wait for the reader instead.
        // SAFETY: fcntl on an fd we own.
        unsafe {
            let flags = libc::fcntl(fd.as_raw_fd(), libc::F_GETFL);
            if flags >= 0 {
                libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags & !libc::O_NONBLOCK);
            }
        }
        let mut f = std::fs::File::from(fd);
        let _ = f.write_all(&data);
    });
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(st: &mut Self, seat: &wl_seat::WlSeat, ev: wl_seat::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        // Keyboards and mice come and go (unplugged, a remote session ends). A gone device's
        // object gets no events any more: drop it, and make a new one when one is back.
        if let wl_seat::Event::Capabilities { capabilities: WEnum::Value(caps) } = ev {
            match (caps.contains(wl_seat::Capability::Keyboard), st.keyboard.is_some()) {
                (true, false) => st.keyboard = Some(seat.get_keyboard(qh, ())),
                (false, true) => {
                    if let Some(k) = st.keyboard.take() && k.version() >= 3 {
                        k.release();
                    }
                    st.focused = false;
                    FOCUSED.store(false, std::sync::atomic::Ordering::Relaxed);
                }
                _ => {}
            }
            match (caps.contains(wl_seat::Capability::Pointer), st.pointer.is_some()) {
                (true, false) => st.pointer = Some(seat.get_pointer(qh, ())),
                (false, true) => {
                    if let Some(p) = st.pointer.take() && p.version() >= 3 {
                        p.release();
                    }
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for State {
    fn event(st: &mut Self, _: &wl_keyboard::WlKeyboard, ev: wl_keyboard::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match ev {
            wl_keyboard::Event::Enter { serial, .. } => {
                log::debug!(target: "dyosnap", "clipboard: keyboard enter {serial}");
                st.focused = true;
                FOCUSED.store(true, std::sync::atomic::Ordering::Relaxed);
                st.serial = Some(serial);
            }
            wl_keyboard::Event::Leave { .. } => {
                log::debug!(target: "dyosnap", "clipboard: keyboard leave");
                st.focused = false;
                FOCUSED.store(false, std::sync::atomic::Ordering::Relaxed);
            }
            wl_keyboard::Event::Key { serial, .. } => st.serial = Some(serial),
            // The keymap's fd is closed when the event is dropped.
            _ => {}
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for State {
    fn event(st: &mut Self, _: &wl_pointer::WlPointer, ev: wl_pointer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match ev {
            wl_pointer::Event::Enter { serial, .. } | wl_pointer::Event::Button { serial, .. } => st.serial = Some(serial),
            _ => {}
        }
    }
}

impl Dispatch<wl_data_device_manager::WlDataDeviceManager, ()> for State {
    fn event(_: &mut Self, _: &wl_data_device_manager::WlDataDeviceManager, _: wl_data_device_manager::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<wl_data_device::WlDataDevice, ()> for State {
    fn event(st: &mut Self, _: &wl_data_device::WlDataDevice, ev: wl_data_device::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match ev {
            // What is on the clipboard now; the old offer is no longer valid.
            wl_data_device::Event::Selection { id } => {
                if let Some(old) = std::mem::replace(&mut st.selection, id) {
                    old.destroy();
                }
            }
            // Drag and drop is not used: let go of its offers.
            wl_data_device::Event::Enter { id: Some(offer), .. } => offer.destroy(),
            _ => {}
        }
    }

    event_created_child!(State, wl_data_device::WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (wl_data_offer::WlDataOffer, OfferMimes::default()),
    ]);
}

impl Dispatch<wl_data_offer::WlDataOffer, OfferMimes> for State {
    fn event(_: &mut Self, _: &wl_data_offer::WlDataOffer, ev: wl_data_offer::Event, mimes: &OfferMimes, _: &Connection, _: &QueueHandle<Self>) {
        if let wl_data_offer::Event::Offer { mime_type } = ev && let Ok(mut m) = mimes.lock() {
            m.push(mime_type);
        }
    }
}

impl Dispatch<wl_data_source::WlDataSource, ()> for State {
    fn event(st: &mut Self, source: &wl_data_source::WlDataSource, ev: wl_data_source::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match ev {
            wl_data_source::Event::Send { fd, mime_type } => {
                if let Some((s, items)) = &st.current
                    && s == source
                    && let Some((_, data)) = items.iter().find(|(m, _)| *m == mime_type)
                {
                    serve(fd, data.clone());
                }
            }
            wl_data_source::Event::Cancelled => {
                // Something else was copied.
                if st.current.as_ref().is_some_and(|(s, _)| s == source) {
                    st.current = None;
                }
                source.destroy();
            }
            _ => {}
        }
    }
}

impl Drop for State {
    fn drop(&mut self) {
        if let Some(d) = self.device.take() && d.version() >= 2 {
            d.release();
        }
        let _ = self.seat.take();
    }
}
