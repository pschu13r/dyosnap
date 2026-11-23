//! The connected screens, read from the Wayland compositor. Coordinates are logical
//! points, like window sizes. Wayland does not tell apps where the mouse is.

use crate::platform::Display;
use egui::{Pos2, Rect, pos2, vec2};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::{delegate_output, delegate_registry, registry_handlers};
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::wl_output;
use wayland_client::{Connection, QueueHandle};

struct State {
    registry: RegistryState,
    outputs: OutputState,
}

impl OutputHandler for State {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers!(OutputState);
}

delegate_output!(State);
delegate_registry!(State);

/// All screens, the one at the top-left corner of the desktop first.
pub fn displays() -> Vec<Display> {
    let mut list: Vec<Display> = outputs().into_iter().map(|(d, _)| d).collect();
    list.sort_by(|a, b| (a.bounds.min.x + a.bounds.min.y).total_cmp(&(b.bounds.min.x + b.bounds.min.y)));
    // While screens are shared, they come first: screen N is input N to the recorder, and
    // `capture_all_screens` has a picture of each.
    let shared = super::recording::shared_rects();
    if !shared.is_empty() {
        let is_shared = |d: &Display| shared.iter().any(|r| r.expand(1.0).contains_rect(d.bounds));
        let (mut first, rest): (Vec<Display>, Vec<Display>) = list.into_iter().partition(|d| is_shared(d));
        first.extend(rest);
        list = first;
    }
    list
}

/// The name of the screen at `pos` ("DP-1"), as the compositor calls it.
pub fn output_name_at(pos: Pos2) -> Option<String> {
    outputs().into_iter().find(|(d, _)| d.bounds.min == pos).and_then(|(_, n)| n)
}

/// Where the screen named `name` is on the desktop, in points.
pub fn output_bounds(name: &str) -> Option<Rect> {
    outputs().into_iter().find(|(_, n)| n.as_deref() == Some(name)).map(|(d, _)| d.bounds)
}

/// The screens with their names, in the compositor's order.
fn outputs() -> Vec<(Display, Option<String>)> {
    let Ok(conn) = Connection::connect_to_env() else { return Vec::new() };
    let Ok((globals, mut queue)) = registry_queue_init::<State>(&conn) else { return Vec::new() };
    let qh = queue.handle();
    let mut st = State { registry: RegistryState::new(&globals), outputs: OutputState::new(&globals, &qh) };
    // Two round trips: the outputs, then their details (xdg-output).
    for _ in 0..2 {
        if queue.roundtrip(&mut st).is_err() {
            return Vec::new();
        }
    }
    st.outputs
        .outputs()
        .filter_map(|o| st.outputs.info(&o))
        .map(|info| {
            let (x, y) = info.logical_position.unwrap_or(info.location);
            let mode = info.modes.iter().find(|m| m.current).map(|m| m.dimensions);
            let (lw, lh) = info.logical_size.or_else(|| {
                mode.map(|(w, h)| (w / info.scale_factor.max(1), h / info.scale_factor.max(1)))
            })
            .unwrap_or((0, 0));
            let scale = match mode {
                Some((pw, _)) if lw > 0 => pw as f32 / lw as f32,
                _ => info.scale_factor.max(1) as f32,
            };
            let d = Display {
                bounds: Rect::from_min_size(pos2(x as f32, y as f32), vec2(lw as f32, lh as f32)),
                scale,
            };
            (d, info.name.clone())
        })
        .filter(|(d, _)| d.bounds.width() > 0.0)
        .collect()
}

/// Wayland does not share the mouse position outside DyoSnap's own windows.
pub fn mouse_position() -> Option<Pos2> {
    None
}
