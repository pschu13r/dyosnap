//! Shared helpers for talking to the xdg-desktop-portal.

use std::sync::Once;

/// Tells the portal which app is asking, so it can remember what was allowed. Only
/// needed outside Flatpak; harmless when it fails (older portals).
pub fn register() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        if let Ok(id) = ashpd::AppID::try_from(super::APP_ID) {
            if let Err(e) = pollster::block_on(ashpd::register_host_app(id)) {
                log::info!(target: "dyosnap", "portal: could not register app id: {e}");
            }
        }
    });
}
