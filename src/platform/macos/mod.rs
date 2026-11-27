//! macOS implementation of `crate::platform`.

mod capture;
mod clipboard;
mod displays;
mod permissions;
mod recording;
mod sck;
mod system;
mod tray;

pub use capture::*;
pub use clipboard::*;
pub use displays::*;
pub use permissions::*;
pub use recording::*;
pub use sck::capture_images;
pub use system::*;
pub use tray::*;
