//! Puts the open main window in front (xdg-activation-v1).
//!
//! Wayland lets no app raise its own window. The desktop hands out an activation token
//! with a tray click or a shortcut press, and an app may pass it on to a window. A new
//! window gets it from `XDG_ACTIVATION_TOKEN` (see `main.rs`); winit has no call for a
//! window that is already open, so this sends the token on winit's own connection.
//! KDE's shortcut portal sends no token; there a KWin script does it.

use wayland_backend::client::{Backend, ObjectId};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_registry, wl_surface::WlSurface};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::{self, XdgActivationV1};

/// Brings `window` to the front with the token of a tray click or shortcut from the last
/// seconds. Without one (shortcuts on KDE) KWin is asked through a script; elsewhere
/// nothing happens.
pub fn activate(window: &winit::window::Window) {
    use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
    let Some(token) = super::take_activation_token() else {
        super::kwin_activate_main();
        return;
    };
    let (Ok(d), Ok(w)) = (window.display_handle(), window.window_handle()) else { return };
    let (RawDisplayHandle::Wayland(d), RawWindowHandle::Wayland(w)) = (d.as_raw(), w.as_raw()) else { return };
    // SAFETY: display and surface belong to winit's window, which lives for this call.
    let conn = Connection::from_backend(unsafe { Backend::from_foreign_display(d.display.as_ptr().cast()) });
    let Ok(id) = (unsafe { ObjectId::from_ptr(WlSurface::interface(), w.surface.as_ptr().cast()) }) else { return };
    let Ok(surface) = WlSurface::from_id(&conn, id) else { return };
    let Ok((globals, queue)) = registry_queue_init::<Activate>(&conn) else { return };
    let Ok(activation) = globals.bind::<XdgActivationV1, _, _>(&queue.handle(), 1..=1, ()) else {
        log::info!(target: "dyosnap", "no xdg-activation on this desktop");
        return;
    };
    activation.activate(token, &surface);
    activation.destroy();
    let ok = conn.flush().is_ok();
    log::info!(target: "dyosnap", "main window activated with a token: {ok}");
}

struct Activate;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Activate {
    fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<XdgActivationV1, ()> for Activate {
    fn event(_: &mut Self, _: &XdgActivationV1, _: xdg_activation_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
