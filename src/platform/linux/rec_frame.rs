//! The red frame around the record area and the control bar below it, as on macOS.
//!
//! A normal Wayland window can neither place itself nor stay on top. The wlr-layer-shell
//! protocol can do both: KDE Plasma, Sway, Hyprland and COSMIC support it, GNOME does not.
//! The frame is four thin bars just outside the area, each a layer surface on the overlay
//! layer that takes no clicks and no keys. The recording crops to the area, so the bars
//! are not in the video. The control bar is one more layer surface that does take clicks:
//! timer, Pause, Stop and Cancel, drawn here with tiny-skia and ab_glyph. It can be moved
//! (drag the timer; the surface moves by its margins) and collapsed to a small pill (see
//! `crate::recbar`). A full-screen recording collapses it by itself after a few seconds.
//!
//! All surfaces live on their own Wayland connection in their own thread. Without layer
//! shell `RecFrame::show` returns `None`, and the recording runs without frame and bar.

use ab_glyph::{Font, FontRef, ScaleFont};
use crate::recbar;
use egui::{Pos2, Rect, pos2, vec2};
use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState, Region};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::pointer::cursor_shape::CursorShapeManager;
use smithay_client_toolkit::seat::pointer::{PointerEvent, PointerEventKind, PointerHandler};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure,
};
use smithay_client_toolkit::shm::slot::{Buffer, SlotPool};
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{
    delegate_compositor, delegate_layer, delegate_output, delegate_pointer, delegate_registry, delegate_seat, delegate_shm,
    registry_handlers,
};
use std::os::fd::AsRawFd;
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::{wl_output, wl_pointer, wl_seat, wl_shm, wl_surface};
use wayland_client::{Connection, QueueHandle};
use smithay_client_toolkit::reexports::protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::{
    Shape, WpCursorShapeDeviceV1,
};

/// Bar thickness and gap to the area, in points. The same as on macOS.
const THICK: f32 = 3.0;
const GAP: f32 = 2.0;
/// Red while recording, orange while paused (ARGB).
const RED: u32 = 0xFF_E2_26_26;
const ORANGE: u32 = 0xFF_FF_AA_00;
/// The control bar, in points, as on macOS (app/capture/rec_session.rs `recording_overlays`).
const BTN_H: f32 = 26.0;
const BTN_PAUSE_W: f32 = 36.0;
const BTN_STOP_W: f32 = 62.0;
const BTN_CANCEL_W: f32 = 58.0;

/// A click on the control bar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BarAction {
    Pause,
    Stop,
    Cancel,
}

enum Cmd {
    /// The timer text and whether the recording is paused.
    Status(String, bool),
    Hide,
}

/// Frame and control bar on screen. Dropping it removes them.
pub struct RecFrame {
    tx: mpsc::Sender<Cmd>,
    actions: mpsc::Receiver<BarAction>,
    thread: Option<JoinHandle<()>>,
}

impl RecFrame {
    /// Shows the frame around `area` (global points) and the control bar. `None` when the
    /// desktop has no layer shell.
    pub fn show(area: Rect, can_pause: bool) -> Option<RecFrame> {
        let (tx, rx) = mpsc::channel();
        let (act_tx, actions) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name("rec-frame".into())
            .spawn(move || {
                if let Err(e) = run(area, can_pause, rx, act_tx, &ready_tx) {
                    log::info!(target: "dyosnap", "no recording frame: {e}");
                    let _ = ready_tx.try_send(false);
                }
            })
            .ok()?;
        // Setting up takes a few round trips to the compositor.
        if ready_rx.recv_timeout(Duration::from_secs(2)).unwrap_or(false) {
            Some(RecFrame { tx, actions, thread: Some(thread) })
        } else {
            let _ = tx.send(Cmd::Hide);
            None
        }
    }

    /// The timer on the bar; frame and dot turn orange while paused.
    pub fn set_status(&self, elapsed: &str, paused: bool) {
        let _ = self.tx.send(Cmd::Status(elapsed.to_string(), paused));
    }

    /// A button clicked on the bar since the last call.
    pub fn take_action(&self) -> Option<BarAction> {
        self.actions.try_recv().ok()
    }
}

impl Drop for RecFrame {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Hide);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Whether the desktop has layer shell, so `RecFrame::show` can draw the frame. GNOME has
/// none; there a countdown shows the area before the recording starts. Asked once.
pub fn frame_supported() -> bool {
    static SUPPORTED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SUPPORTED.get_or_init(|| {
        let Ok(conn) = Connection::connect_to_env() else { return false };
        let Ok((globals, _queue)) = registry_queue_init::<State>(&conn) else { return false };
        globals.contents().with_list(|list| list.iter().any(|g| g.interface == "zwlr_layer_shell_v1"))
    })
}

/// The four bars around `area`, in global points, cut to the screen `screen`. A bar that
/// would lie off screen (the area touches the screen edge) is left out.
fn bars(area: Rect, screen: Rect) -> Vec<Rect> {
    let o = area.expand(GAP + THICK);
    [
        Rect::from_min_size(o.min, vec2(o.width(), THICK)),
        Rect::from_min_size(pos2(o.min.x, o.max.y - THICK), vec2(o.width(), THICK)),
        Rect::from_min_size(pos2(o.min.x, o.min.y + THICK), vec2(THICK, o.height() - 2.0 * THICK)),
        Rect::from_min_size(pos2(o.max.x - THICK, o.min.y + THICK), vec2(THICK, o.height() - 2.0 * THICK)),
    ]
    .into_iter()
    .map(|b| b.intersect(screen))
    .filter(|b| b.width() >= 1.0 && b.height() >= 1.0)
    .collect()
}

/// The buttons of a bar `w` points wide, right to left as on macOS: Cancel, Pause, Stop.
fn buttons(w: f32, can_pause: bool) -> Vec<(Rect, BarAction)> {
    let y = (recbar::BAR_H - BTN_H) / 2.0;
    let mut x = w - 8.0;
    let mut out = Vec::new();
    let mut add = |width: f32, a: BarAction, out: &mut Vec<(Rect, BarAction)>| {
        x -= width;
        out.push((Rect::from_min_size(pos2(x, y), vec2(width, BTN_H)), a));
        x -= 6.0;
    };
    add(BTN_CANCEL_W, BarAction::Cancel, &mut out);
    if can_pause {
        add(BTN_PAUSE_W, BarAction::Pause, &mut out);
    }
    add(BTN_STOP_W, BarAction::Stop, &mut out);
    out
}

/// What a point on the control bar is.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Part {
    Button(BarAction),
    Collapse,
    /// Dot and timer: drag to move the bar.
    Grip,
    /// The whole collapsed bar: click to expand, drag to move.
    Pill,
}

/// The part of a bar `w` points wide at `p` (in the bar's own points).
fn part_at(collapsed: bool, w: f32, can_pause: bool, p: Pos2) -> Option<Part> {
    if collapsed {
        return Rect::from_min_size(Pos2::ZERO, recbar::PILL).contains(p).then_some(Part::Pill);
    }
    let list = buttons(w, can_pause);
    if let Some((_, a)) = list.iter().find(|(r, _)| r.contains(p)) {
        return Some(Part::Button(*a));
    }
    let first = list.iter().map(|(r, _)| r.min.x).fold(w, f32::min);
    let collapse = recbar::collapse_button(first);
    if collapse.contains(p) {
        return Some(Part::Collapse);
    }
    (p.x >= 0.0 && p.x < collapse.min.x && p.y >= 0.0 && p.y < recbar::BAR_H).then_some(Part::Grip)
}

/// A pressed left button on the bar.
struct Press {
    /// Where, in the bar's points. The bar moves under the pointer, so this stays the
    /// grip point while dragging.
    at: Pos2,
    part: Option<Part>,
    dragging: bool,
}

struct Surface {
    layer: LayerSurface,
    /// Size in points.
    size: (u32, u32),
    configured: bool,
    /// Kept until the next one is drawn, so the compositor can still read it.
    buffer: Option<Buffer>,
}

struct Bar {
    surface: Surface,
    /// Pixels per point of its screen (whole numbers: buffer scale).
    scale: i32,
    can_pause: bool,
    elapsed: String,
    hover: Option<Part>,
    /// Where the bar is (global points) and the screen it is on.
    rect: Rect,
    screen: Rect,
    collapsed: bool,
    press: Option<Press>,
    /// The pointer is on the bar.
    inside: bool,
    /// For the cursor shape: the serial of the last enter, and the shape now.
    serial: u32,
    shape: Option<Shape>,
    /// After a move, wait for the compositor to show it before the next one (else the
    /// pointer position it still reports would move the bar twice).
    waiting_frame: bool,
    last_move: Instant,
}

struct State {
    registry: RegistryState,
    outputs: OutputState,
    seat: SeatState,
    shm: Shm,
    pool: Option<SlotPool>,
    frame: Vec<Surface>,
    bar: Option<Bar>,
    paused: bool,
    closed: bool,
    /// A full-screen recording collapses its bar then, unless the pointer is on it.
    auto_collapse: Option<Instant>,
    pointer: Option<wl_pointer::WlPointer>,
    cursor_shapes: Option<CursorShapeManager>,
    cursor_device: Option<WpCursorShapeDeviceV1>,
    actions: mpsc::Sender<BarAction>,
    font: FontRef<'static>,
}

impl State {
    fn draw_frame(&mut self, i: usize) {
        let color = if self.paused { ORANGE } else { RED };
        let Some(pool) = &mut self.pool else { return };
        let s = &mut self.frame[i];
        if !s.configured {
            return;
        }
        let (w, h) = (s.size.0 as i32, s.size.1 as i32);
        let Ok((buffer, canvas)) = pool.create_buffer(w, h, w * 4, wl_shm::Format::Argb8888) else { return };
        for px in canvas.chunks_exact_mut(4) {
            px.copy_from_slice(&color.to_le_bytes());
        }
        let surface = s.layer.wl_surface();
        surface.damage_buffer(0, 0, w, h);
        if buffer.attach_to(surface).is_ok() {
            s.layer.commit();
            s.buffer = Some(buffer);
        }
    }

    fn draw_bar(&mut self) {
        let paused = self.paused;
        let font = self.font.clone();
        let Some(pool) = &mut self.pool else { return };
        let Some(bar) = &mut self.bar else { return };
        if !bar.surface.configured {
            return;
        }
        let sc = bar.scale.max(1);
        let (w, h) = (bar.surface.size.0 as i32 * sc, bar.surface.size.1 as i32 * sc);
        let Some(pm) = paint_bar(&font, bar, paused) else { return };
        let Ok((buffer, canvas)) = pool.create_buffer(w, h, w * 4, wl_shm::Format::Argb8888) else { return };
        // tiny-skia: premultiplied RGBA. wl_shm ARGB8888 (little endian): B, G, R, A.
        for (dst, src) in canvas.chunks_exact_mut(4).zip(pm.data().chunks_exact(4)) {
            dst.copy_from_slice(&[src[2], src[1], src[0], src[3]]);
        }
        let surface = bar.surface.layer.wl_surface();
        surface.set_buffer_scale(sc);
        surface.damage_buffer(0, 0, w, h);
        if buffer.attach_to(surface).is_ok() {
            bar.surface.layer.commit();
            bar.surface.buffer = Some(buffer);
        }
    }

    /// Moves and resizes the bar's surface to `bar.rect` and draws it again.
    fn relayout(&mut self, qh: &QueueHandle<Self>) {
        let Some(bar) = &mut self.bar else { return };
        let local = bar.rect.min - bar.screen.min;
        let size = (bar.rect.width().round() as u32, bar.rect.height().round() as u32);
        bar.surface.layer.set_margin(local.y.round() as i32, 0, 0, local.x.round() as i32);
        bar.surface.layer.set_size(size.0, size.1);
        bar.surface.size = size;
        bar.waiting_frame = true;
        bar.last_move = Instant::now();
        let surface = bar.surface.layer.wl_surface();
        surface.frame(qh, surface.clone());
        self.draw_bar();
    }

    /// Collapses or expands the bar; the edge nearest to the screen edge stays.
    fn set_collapsed(&mut self, collapsed: bool, qh: &QueueHandle<Self>) {
        let Some(bar) = &mut self.bar else { return };
        bar.collapsed = collapsed;
        bar.rect = recbar::resized(bar.rect, recbar::size(collapsed, bar.can_pause), bar.screen);
        bar.hover = None;
        self.auto_collapse = None;
        self.relayout(qh);
    }

    fn set_shape(&mut self, pointer: &wl_pointer::WlPointer, qh: &QueueHandle<Self>, shape: Shape) {
        let Some(bar) = &mut self.bar else { return };
        if bar.shape == Some(shape) {
            return;
        }
        bar.shape = Some(shape);
        if let Some(m) = &self.cursor_shapes {
            let dev = self.cursor_device.get_or_insert_with(|| m.get_shape_device(pointer, qh));
            dev.set_shape(bar.serial, shape);
        }
    }

    fn draw_all(&mut self) {
        for i in 0..self.frame.len() {
            self.draw_frame(i);
        }
        self.draw_bar();
    }
}

/// The control bar as pixels, at the bar's scale.
fn paint_bar(font: &FontRef<'static>, bar: &Bar, paused: bool) -> Option<tiny_skia::Pixmap> {
    use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Transform};
    let sc = bar.scale.max(1) as f32;
    let (w, h) = (bar.surface.size.0 as f32, bar.surface.size.1 as f32);
    let mut pm = Pixmap::new((w * sc) as u32, (h * sc) as u32)?;
    let t = Transform::from_scale(sc, sc);
    let fill = |pm: &mut Pixmap, r: Rect, radius: f32, rgb: (u8, u8, u8)| {
        let mut p = Paint::default();
        p.anti_alias = true;
        p.set_color_rgba8(rgb.0, rgb.1, rgb.2, 255);
        if let Some(path) = rounded(r, radius) {
            pm.fill_path(&path, &p, FillRule::Winding, t, None);
        }
    };
    pm.fill(Color::from_rgba8(236, 236, 238, 255));
    // Dot and timer.
    let (dr, dg, db) = if paused { (255, 170, 0) } else { (226, 38, 38) };
    let mut p = Paint::default();
    p.anti_alias = true;
    p.set_color_rgba8(dr, dg, db, 255);
    if let Some(c) = PathBuilder::from_circle(8.0 + 6.0, h / 2.0, 5.0) {
        pm.fill_path(&c, &p, FillRule::Winding, t, None);
    }
    text(&mut pm, font, &bar.elapsed, (8.0 + 12.0 + 6.0) * sc, h / 2.0 * sc, 13.0 * sc, (30, 30, 30), false);
    if bar.collapsed {
        return Some(pm);
    }
    // Collapse: a minus, left of the buttons.
    let list = buttons(w, bar.can_pause);
    let first = list.iter().map(|(r, _)| r.min.x).fold(w, f32::min);
    let cr = recbar::collapse_button(first);
    fill(&mut pm, cr, 4.0, if bar.hover == Some(Part::Collapse) { (200, 200, 200) } else { (214, 214, 214) });
    let mut ip = Paint::default();
    ip.set_color_rgba8(40, 40, 40, 255);
    if let Some(m) = tiny_skia::Rect::from_xywh(cr.center().x - 5.0, cr.center().y - 1.0, 10.0, 2.0) {
        pm.fill_rect(m, &ip, t, None);
    }
    for (r, a) in list {
        let hover = bar.hover == Some(Part::Button(a));
        match a {
            BarAction::Stop => {
                fill(&mut pm, r, 4.0, if hover { (240, 58, 58) } else { (226, 38, 38) });
                text(&mut pm, font, "Stop", r.center().x * sc, r.center().y * sc, 13.0 * sc, (255, 255, 255), true);
            }
            BarAction::Cancel => {
                fill(&mut pm, r, 4.0, if hover { (200, 200, 200) } else { (214, 214, 214) });
                text(&mut pm, font, "Cancel", r.center().x * sc, r.center().y * sc, 13.0 * sc, (40, 40, 40), true);
            }
            BarAction::Pause => {
                fill(&mut pm, r, 4.0, if hover { (200, 200, 200) } else { (214, 214, 214) });
                let c = r.center();
                let mut ip = Paint::default();
                ip.anti_alias = true;
                ip.set_color_rgba8(40, 40, 40, 255);
                let mut pb = PathBuilder::new();
                if paused {
                    // Play: a triangle.
                    pb.move_to(c.x - 4.0, c.y - 6.0);
                    pb.line_to(c.x + 6.0, c.y);
                    pb.line_to(c.x - 4.0, c.y + 6.0);
                    pb.close();
                } else {
                    // Pause: two bars.
                    for x in [c.x - 5.0, c.x + 2.0] {
                        if let Some(r) = tiny_skia::Rect::from_xywh(x, c.y - 6.0, 3.0, 12.0) {
                            pb.push_rect(r);
                        }
                    }
                }
                if let Some(path) = pb.finish() {
                    pm.fill_path(&path, &ip, FillRule::Winding, t, None);
                }
            }
        }
    }
    Some(pm)
}

fn rounded(r: Rect, radius: f32) -> Option<tiny_skia::Path> {
    let (x0, y0, x1, y1) = (r.min.x, r.min.y, r.max.x, r.max.y);
    let k = radius.min(r.width() / 2.0).min(r.height() / 2.0);
    let mut pb = tiny_skia::PathBuilder::new();
    pb.move_to(x0 + k, y0);
    pb.line_to(x1 - k, y0);
    pb.quad_to(x1, y0, x1, y0 + k);
    pb.line_to(x1, y1 - k);
    pb.quad_to(x1, y1, x1 - k, y1);
    pb.line_to(x0 + k, y1);
    pb.quad_to(x0, y1, x0, y1 - k);
    pb.line_to(x0, y0 + k);
    pb.quad_to(x0, y0, x0 + k, y0);
    pb.close();
    pb.finish()
}

/// Draws `s` in pixels: starting at `x` (or centered on it), centered on `cy`.
#[allow(clippy::too_many_arguments)]
fn text(pm: &mut tiny_skia::Pixmap, font: &FontRef<'static>, s: &str, x: f32, cy: f32, size: f32, rgb: (u8, u8, u8), center: bool) {
    let sf = font.as_scaled(size);
    let mut width = 0.0;
    let mut prev = None;
    for ch in s.chars() {
        let id = sf.glyph_id(ch);
        if let Some(p) = prev {
            width += sf.kern(p, id);
        }
        width += sf.h_advance(id);
        prev = Some(id);
    }
    let mut pen = if center { x - width / 2.0 } else { x };
    let baseline = cy + (sf.ascent() + sf.descent()) / 2.0;
    let (pw, ph) = (pm.width() as i32, pm.height() as i32);
    let data = pm.data_mut();
    let mut prev = None;
    for ch in s.chars() {
        let id = sf.glyph_id(ch);
        if let Some(p) = prev {
            pen += sf.kern(p, id);
        }
        let glyph = id.with_scale_and_position(size, ab_glyph::point(pen, baseline));
        if let Some(outline) = font.outline_glyph(glyph) {
            let b = outline.px_bounds();
            outline.draw(|gx, gy, cov| {
                let (px, py) = (b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32);
                if px < 0 || py < 0 || px >= pw || py >= ph {
                    return;
                }
                let i = (py * pw + px) as usize * 4;
                let a = cov.clamp(0.0, 1.0);
                // Over a solid background: blend straight into the premultiplied pixel.
                for (k, c) in [rgb.0, rgb.1, rgb.2].into_iter().enumerate() {
                    data[i + k] = (c as f32 * a + data[i + k] as f32 * (1.0 - a)).round() as u8;
                }
                data[i + 3] = 255;
            });
        }
        pen += sf.h_advance(id);
        prev = Some(id);
    }
}

fn run(
    area: Rect,
    can_pause: bool,
    rx: mpsc::Receiver<Cmd>,
    actions: mpsc::Sender<BarAction>,
    ready: &mpsc::SyncSender<bool>,
) -> Result<(), String> {
    let conn = Connection::connect_to_env().map_err(|e| e.to_string())?;
    let (globals, mut queue) = registry_queue_init::<State>(&conn).map_err(|e| e.to_string())?;
    let qh = queue.handle();
    let compositor = CompositorState::bind(&globals, &qh).map_err(|e| e.to_string())?;
    let layer_shell = LayerShell::bind(&globals, &qh).map_err(|_| "the desktop has no layer shell".to_string())?;
    let shm = Shm::bind(&globals, &qh).map_err(|e| e.to_string())?;
    let font = FontRef::try_from_slice(epaint_default_fonts::UBUNTU_LIGHT).map_err(|e| e.to_string())?;
    let mut st = State {
        registry: RegistryState::new(&globals),
        outputs: OutputState::new(&globals, &qh),
        seat: SeatState::new(&globals, &qh),
        shm,
        pool: None,
        frame: Vec::new(),
        bar: None,
        paused: false,
        closed: false,
        auto_collapse: None,
        pointer: None,
        cursor_shapes: CursorShapeManager::bind(&globals, &qh).ok(),
        cursor_device: None,
        actions,
        font,
    };
    // Two round trips: the outputs, then their details (xdg-output).
    for _ in 0..2 {
        queue.roundtrip(&mut st).map_err(|e| e.to_string())?;
    }

    // The screen that holds the area, in the same global points as `displays()`.
    let screens: Vec<(wl_output::WlOutput, Rect, i32)> = st
        .outputs
        .outputs()
        .filter_map(|o| {
            let info = st.outputs.info(&o)?;
            let (x, y) = info.logical_position.unwrap_or(info.location);
            let (w, h) = info.logical_size?;
            Some((o, Rect::from_min_size(pos2(x as f32, y as f32), vec2(w as f32, h as f32)), info.scale_factor))
        })
        .collect();
    let (output, screen, scale) = screens
        .iter()
        .find(|(_, r, _)| r.contains(area.center()))
        .or(screens.first())
        .ok_or("no screen found")?;

    // A layer surface at `r` (global points) on that screen.
    let place = |r: Rect, input: bool| -> Surface {
        let (w, h) = (r.width().round().max(1.0) as u32, r.height().round().max(1.0) as u32);
        let surface = compositor.create_surface(&qh);
        if !input {
            // Clicks go through, to whatever is below.
            if let Ok(region) = Region::new(&compositor) {
                surface.set_input_region(Some(region.wl_region()));
            }
        }
        let layer = layer_shell.create_layer_surface(&qh, surface, Layer::Overlay, Some("dyosnap-frame"), Some(output));
        layer.set_anchor(Anchor::TOP | Anchor::LEFT);
        // Place it from the screen's corner, not moved aside by panels.
        layer.set_exclusive_zone(-1);
        let local = r.min - screen.min;
        layer.set_margin(local.y.round() as i32, 0, 0, local.x.round() as i32);
        layer.set_size(w, h);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.commit();
        Surface { layer, size: (w, h), configured: false, buffer: None }
    };

    let mut bytes = 0;
    for b in bars(area, *screen) {
        let s = place(b, false);
        bytes += (s.size.0 * s.size.1 * 4) as usize;
        st.frame.push(s);
    }
    let br = recbar::initial_rect(area, *screen, can_pause);
    let bar_surface = place(br, true);
    bytes += (bar_surface.size.0 * bar_surface.size.1 * 4) as usize * (*scale as usize).pow(2);
    st.bar = Some(Bar {
        surface: bar_surface,
        scale: *scale,
        can_pause,
        elapsed: "00:00".into(),
        hover: None,
        rect: br,
        screen: *screen,
        collapsed: false,
        press: None,
        inside: false,
        serial: 0,
        shape: None,
        waiting_frame: false,
        last_move: Instant::now(),
    });
    // A full-screen recording would show the bar: it collapses after a few seconds.
    st.auto_collapse = recbar::is_full_screen(area, *screen).then(|| Instant::now() + Duration::from_secs(recbar::AUTO_COLLAPSE_SECS));
    // Room for two buffers per surface: one shown, one being drawn.
    st.pool = Some(SlotPool::new(bytes * 2, &st.shm).map_err(|e| e.to_string())?);
    queue.roundtrip(&mut st).map_err(|e| e.to_string())?;
    let _ = ready.try_send(true);
    log::info!(target: "dyosnap", "recording frame shown: {} bars and the control bar", st.frame.len());

    // Wait for commands, and keep answering the compositor (it may close the surfaces).
    loop {
        loop {
            match rx.try_recv() {
                Ok(Cmd::Status(elapsed, paused)) => {
                    let color_changed = paused != st.paused;
                    st.paused = paused;
                    let text_changed = st.bar.as_ref().is_some_and(|b| b.elapsed != elapsed);
                    if let Some(b) = &mut st.bar {
                        b.elapsed = elapsed;
                    }
                    if color_changed {
                        st.draw_all();
                    } else if text_changed {
                        st.draw_bar();
                    }
                }
                Ok(Cmd::Hide) | Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }
        if st.closed {
            return Ok(());
        }
        // While the pointer is on the bar, wait: it collapses once the pointer leaves.
        if st.auto_collapse.is_some_and(|t| Instant::now() >= t) && !st.bar.as_ref().is_some_and(|b| b.inside) {
            st.auto_collapse = None;
            if st.bar.as_ref().is_some_and(|b| !b.collapsed) {
                st.set_collapsed(true, &qh);
            }
        }
        queue.dispatch_pending(&mut st).map_err(|e| e.to_string())?;
        conn.flush().map_err(|e| e.to_string())?;
        if let Some(guard) = queue.prepare_read() {
            let mut fd = libc::pollfd { fd: guard.connection_fd().as_raw_fd(), events: libc::POLLIN, revents: 0 };
            // SAFETY: one valid pollfd; the fd stays open while the guard lives.
            let n = unsafe { libc::poll(&mut fd, 1, 100) };
            if n > 0 {
                guard.read().map_err(|e| e.to_string())?;
            }
        }
        queue.dispatch_pending(&mut st).map_err(|e| e.to_string())?;
    }
}

impl LayerShellHandler for State {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.closed = true;
    }

    fn configure(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface, c: LayerSurfaceConfigure, _: u32) {
        if let Some(i) = self.frame.iter().position(|s| &s.layer == layer) {
            let s = &mut self.frame[i];
            if c.new_size.0 > 0 && c.new_size.1 > 0 {
                s.size = c.new_size;
            }
            s.configured = true;
            self.draw_frame(i);
        } else if let Some(b) = self.bar.as_mut().filter(|b| &b.surface.layer == layer) {
            if c.new_size.0 > 0 && c.new_size.1 > 0 {
                b.surface.size = c.new_size;
            }
            b.surface.configured = true;
            self.draw_bar();
        }
    }
}

impl PointerHandler for State {
    fn pointer_frame(&mut self, _: &Connection, qh: &QueueHandle<Self>, pointer: &wl_pointer::WlPointer, events: &[PointerEvent]) {
        let Some(bar_surface) = self.bar.as_ref().map(|b| b.surface.layer.wl_surface().clone()) else { return };
        let mut redraw = false;
        for ev in events.iter().filter(|e| e.surface == bar_surface) {
            let p = pos2(ev.position.0 as f32, ev.position.1 as f32);
            let Some(bar) = self.bar.as_mut() else { return };
            let (w, can_pause, collapsed) = (bar.surface.size.0 as f32, bar.can_pause, bar.collapsed);
            let part = part_at(collapsed, w, can_pause, p);
            let hover = |part: Option<Part>| part.filter(|p| matches!(p, Part::Button(_) | Part::Collapse));
            let (mut move_by, mut toggle, mut action) = (None, false, None);
            let was_hover = bar.hover;
            match ev.kind {
                PointerEventKind::Enter { serial } => {
                    bar.serial = serial;
                    bar.shape = None;
                    bar.inside = true;
                    bar.hover = hover(part);
                }
                PointerEventKind::Leave { .. } => {
                    bar.inside = false;
                    bar.hover = None;
                }
                PointerEventKind::Motion { .. } => {
                    bar.hover = hover(part);
                    if let Some(press) = &mut bar.press {
                        let grip = matches!(press.part, Some(Part::Grip | Part::Pill));
                        if grip && !press.dragging && recbar::is_drag(press.at, p) {
                            press.dragging = true;
                        }
                        // The surface moves under the pointer: each step is the way from
                        // the press point, once the last move is on screen.
                        if press.dragging && (!bar.waiting_frame || bar.last_move.elapsed() > Duration::from_millis(100)) {
                            move_by = Some(p - press.at);
                        }
                    }
                }
                // Left button.
                PointerEventKind::Press { button: 0x110, .. } => bar.press = Some(Press { at: p, part, dragging: false }),
                PointerEventKind::Release { button: 0x110, .. } => {
                    // A click: pressed and let go on the same part, without a drag.
                    if bar.press.take().is_some_and(|pr| !pr.dragging && pr.part == part) {
                        match part {
                            Some(Part::Button(a)) => action = Some(a),
                            Some(Part::Collapse | Part::Pill) => toggle = true,
                            _ => {}
                        }
                    }
                    bar.hover = hover(part);
                }
                _ => {}
            }
            let shape = match (bar.press.as_ref().is_some_and(|p| p.dragging), part) {
                (true, _) => Shape::Grabbing,
                (_, Some(Part::Grip | Part::Pill)) => Shape::Grab,
                _ => Shape::Default,
            };
            redraw |= bar.hover != was_hover;
            if let Some(a) = action {
                let _ = self.actions.send(a);
            }
            if let Some(d) = move_by {
                if let Some(bar) = &mut self.bar {
                    bar.rect = recbar::dragged(bar.rect, d, bar.screen);
                }
                self.relayout(qh);
            }
            if toggle {
                self.set_collapsed(!collapsed, qh);
            }
            self.set_shape(pointer, qh, shape);
        }
        if redraw {
            self.draw_bar();
        }
    }
}

impl SeatHandler for State {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(&mut self, _: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Pointer && self.pointer.is_none() {
            self.pointer = self.seat.get_pointer(qh, &seat).ok();
        }
    }
    fn remove_capability(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat, capability: Capability) {
        if capability == Capability::Pointer {
            if let Some(p) = self.pointer.take() {
                p.release();
            }
            if let Some(d) = self.cursor_device.take() {
                d.destroy();
            }
        }
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl CompositorHandler for State {
    fn scale_factor_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: i32) {}
    fn transform_changed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: wl_output::Transform) {}
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, surface: &wl_surface::WlSurface, _: u32) {
        // The last move of the bar is on screen.
        if let Some(b) = self.bar.as_mut().filter(|b| b.surface.layer.wl_surface() == surface) {
            b.waiting_frame = false;
        }
    }
    fn surface_enter(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: &wl_output::WlOutput) {}
}

impl OutputHandler for State {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ShmHandler for State {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers!(OutputState, SeatState);
}

delegate_compositor!(State);
delegate_output!(State);
delegate_shm!(State);
delegate_layer!(State);
delegate_seat!(State);
delegate_pointer!(State);
delegate_registry!(State);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bars_sit_outside_the_area() {
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(1920.0, 1080.0));
        let area = Rect::from_min_size(pos2(100.0, 100.0), vec2(400.0, 300.0));
        let b = bars(area, screen);
        assert_eq!(b.len(), 4);
        for bar in &b {
            assert!(!bar.intersects(area), "{bar:?} covers the area");
        }
        // Top bar: 5 points above the area, as wide as the area plus both side bars.
        assert_eq!(b[0], Rect::from_min_size(pos2(95.0, 95.0), vec2(410.0, 3.0)));
    }

    #[test]
    fn bars_off_screen_are_left_out() {
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(1920.0, 1080.0));
        // Area at the top-left corner: no top and no left bar.
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0));
        assert_eq!(bars(area, screen).len(), 2);
        // Whole screen: nothing to draw.
        assert!(bars(screen, screen).is_empty());
    }

    #[test]
    fn parts_of_the_bar() {
        let w = recbar::bar_size(true).x;
        // Dot and timer are the grip, the buttons are buttons, the minus collapses.
        assert_eq!(part_at(false, w, true, pos2(20.0, 20.0)), Some(Part::Grip));
        let (stop, a) = buttons(w, true)[2];
        assert_eq!(a, BarAction::Stop);
        assert_eq!(part_at(false, w, true, stop.center()), Some(Part::Button(BarAction::Stop)));
        let c = recbar::collapse_button(stop.min.x);
        assert_eq!(part_at(false, w, true, c.center()), Some(Part::Collapse));
        assert!(c.max.x < stop.min.x);
        // Buttons fit inside the bar.
        for (r, _) in buttons(w, true) {
            assert!(r.min.x > 0.0 && r.max.x < w);
        }
        // The pill is one part.
        assert_eq!(part_at(true, recbar::PILL.x, true, pos2(40.0, 14.0)), Some(Part::Pill));
        assert_eq!(part_at(true, recbar::PILL.x, true, pos2(90.0, 14.0)), None);
    }

    #[test]
    fn bar_paints_without_panicking() {
        let font = FontRef::try_from_slice(epaint_default_fonts::UBUNTU_LIGHT).unwrap();
        // A bar needs a surface only for its size; paint_bar reads nothing else from it.
        let pm = {
            let w = 256.0;
            let mut pm = tiny_skia::Pixmap::new((w * 2.0) as u32, (recbar::BAR_H * 2.0) as u32).unwrap();
            text(&mut pm, &font, "01:23", 52.0, 40.0, 26.0, (30, 30, 30), false);
            pm
        };
        // Some pixels of the text are dark.
        assert!(pm.data().chunks_exact(4).any(|p| p[3] == 255 && p[0] < 100));
    }
}
