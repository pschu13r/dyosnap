//! Linux implementation of `crate::platform`, for Wayland desktops (GNOME, KDE, Sway,
//! Hyprland, COSMIC).
//!
//! Wayland lets an app neither read the screen nor see the mouse or catch keys outside
//! its own windows. So screenshots, screen recording and global shortcuts go through the
//! xdg-desktop-portal, which asks the person once. The tray icon is a
//! StatusNotifierItem (GNOME shows it with the AppIndicator extension).

mod activate;
mod capture;
mod clipboard;
mod displays;
mod portal;
mod rec_frame;
mod recording;
mod system;
mod tray;
mod windows;
mod wl_clipboard;

pub use activate::activate;
pub use capture::*;
pub use clipboard::*;
pub use displays::*;
pub use rec_frame::*;
pub use recording::*;
pub use system::*;
pub use tray::*;
pub use windows::*;
pub use wl_clipboard::{attach as attach_clipboard, ready as clipboard_ready, ready_known as clipboard_ready_known};

use super::{APP_ID, OLD_APP_IDS};
