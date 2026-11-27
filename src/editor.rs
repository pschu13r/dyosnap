//! The picture editor canvas: draw, select, move, type text, crop.

use crate::doc::{self, Doc, Shape, Style};
use ab_glyph::FontVec;
use egui::{
    Color32, CursorIcon, FontFamily, FontId, Id, Key, Pos2, Rect, Sense, Stroke, StrokeKind,
    TextureHandle, TextureOptions, Ui, Vec2, pos2, vec2,
};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Arrow,
    Rect,
    Line,
    Ellipse,
    Pen,
    Highlight,
    Text,
    Callout,
    Step,
    Blur,
    Crop,
}

impl Tool {
    pub fn tip(self) -> &'static str {
        match self {
            Tool::Arrow => "Arrow (A)",
            Tool::Rect => "Rectangle (R)",
            Tool::Line => "Line (L)",
            Tool::Ellipse => "Oval (O)",
            Tool::Pen => "Pen (P)",
            Tool::Highlight => "Highlighter: Shift draws a straight line (H)",
            Tool::Text => "Text (T)",
            Tool::Callout => "Arrow with text: drag onto the object, then type (C)",
            Tool::Step => "Numbered marker: each click places the next number (N)",
            Tool::Blur => "Blur (B)",
            Tool::Crop => "Crop (X)",
        }
    }
}

pub fn annot_font(px: f32) -> FontId {
    FontId::new(px, FontFamily::Name("annot".into()))
}

enum Drag {
    New { start: Pos2, cur: Pos2, pts: Vec<Pos2> },
    Move { idx: usize, last: Pos2, before: Doc, moved: bool },
    Crop { start: Pos2 },
}

struct TextState {
    pos: Pos2,
    tip: Option<Pos2>,
    text: String,
    style: Style,
    replace: Option<usize>,
    focus: bool,
}

pub struct ResizeDlg {
    pub w: u32,
    pub h: u32,
    pub keep: bool,
}

pub struct Editor {
    pub doc: Doc,
    undo: Vec<Doc>,
    redo: Vec<Doc>,
    tex: Option<TextureHandle>,
    tex_dirty: bool,
    skip: Option<usize>,
    pub name: String,
    pub saved: Option<PathBuf>,
    pub saved_name: String,
    /// Image pixels per screen point (2 for Retina captures).
    pub ppp: f32,
    pub selected: Option<usize>,
    drag: Option<Drag>,
    text: Option<TextState>,
    pub crop: Option<Rect>,
    pub resize: Option<ResizeDlg>,
    pub zoom: f32,
    /// Goes up with every change to the picture (used to copy each edit to the clipboard).
    pub version: u64,
    /// Zoom chosen by the person, relative to "fit in window" (1.0 = fit).
    pub view_zoom: f32,
    /// How far the picture was moved while zoomed in, in screen points.
    pub pan: Vec2,
}

const MAX_UNDO: usize = 40;

impl Editor {
    /// Drops the picture on the graphics card; the next frame sends it again.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub fn forget_texture(&mut self) {
        self.tex = None;
        self.tex_dirty = true;
    }

    pub fn new(img: image::RgbaImage, name: String, ppp: f32) -> Self {
        Self {
            doc: Doc { base: Arc::new(img), shapes: Vec::new() },
            undo: Vec::new(),
            redo: Vec::new(),
            tex: None,
            tex_dirty: true,
            skip: None,
            name,
            saved: None,
            saved_name: String::new(),
            ppp: ppp.max(1.0),
            selected: None,
            drag: None,
            text: None,
            crop: None,
            resize: None,
            zoom: 1.0,
            version: 0,
            view_zoom: 1.0,
            pan: Vec2::ZERO,
        }
    }

    /// Back to "fit in window".
    pub fn reset_view(&mut self) {
        self.view_zoom = 1.0;
        self.pan = Vec2::ZERO;
    }

    pub fn zoom_by(&mut self, f: f32) {
        self.view_zoom = (self.view_zoom * f).clamp(0.2, 16.0);
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    fn push_undo(&mut self, before: Doc) {
        self.undo.push(before);
        if self.undo.len() > MAX_UNDO {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    fn changed(&mut self) {
        self.tex_dirty = true;
        self.version += 1;
    }

    /// Something is still being drawn, moved or typed.
    pub fn is_busy(&self) -> bool {
        self.drag.is_some() || self.text.is_some()
    }

    pub fn undo(&mut self) {
        self.commit_text();
        if let Some(d) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.doc, d));
            self.selected = None;
            self.skip = None;
            self.changed();
        }
    }

    pub fn redo(&mut self) {
        self.commit_text();
        if let Some(d) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.doc, d));
            self.selected = None;
            self.skip = None;
            self.changed();
        }
    }

    pub fn is_editing_text(&self) -> bool {
        self.text.is_some()
    }

    pub fn delete_selected(&mut self) {
        if let Some(i) = self.selected.take() && i < self.doc.shapes.len() {
            let before = self.doc.clone();
            self.doc.shapes.remove(i);
            self.push_undo(before);
            self.changed();
        }
    }

    /// Adds a new shape as one undo step and selects it.
    fn add_shape(&mut self, s: Shape) {
        let before = self.doc.clone();
        self.doc.shapes.push(s);
        self.push_undo(before);
        self.selected = Some(self.doc.shapes.len() - 1);
        self.changed();
    }

    fn new_step(&self, pos: Pos2, style: Style) -> Shape {
        Shape::Step { pos, n: doc::next_step(&self.doc.shapes), style }
    }

    pub fn restyle_selected(&mut self, style: Style) {
        if let Some(i) = self.selected
            && let Some(s) = self.doc.shapes.get(i)
            && s.style().is_some() && s.style() != Some(style)
        {
            let before = self.doc.clone();
            self.doc.shapes[i].set_style(style);
            self.push_undo(before);
            self.changed();
        }
        if let Some(t) = &mut self.text {
            t.style = style;
        }
    }

    pub fn commit_text(&mut self) {
        let Some(t) = self.text.take() else { return };
        self.skip = None;
        let text = t.text.trim_end().to_string();
        let before = self.doc.clone();
        let new_shape = if text.trim().is_empty() {
            None
        } else if let Some(tip) = t.tip {
            Some(Shape::Callout { tip, pos: t.pos, text, style: t.style })
        } else {
            Some(Shape::Text { pos: t.pos, text, style: t.style })
        };
        match (t.replace, new_shape) {
            (Some(i), Some(s)) if i < self.doc.shapes.len() => {
                if self.doc.shapes[i] != s {
                    self.doc.shapes[i] = s;
                    self.push_undo(before);
                }
                self.selected = Some(i);
            }
            (Some(i), None) if i < self.doc.shapes.len() => {
                self.doc.shapes.remove(i);
                self.push_undo(before);
                self.selected = None;
            }
            (None, Some(s)) => {
                self.doc.shapes.push(s);
                self.push_undo(before);
                self.selected = Some(self.doc.shapes.len() - 1);
            }
            _ => {}
        }
        self.changed();
    }

    pub fn apply_crop(&mut self) {
        self.reset_view();
        if let Some(r) = self.crop.take() && let Some(d) = doc::crop(&self.doc, r) {
            let before = std::mem::replace(&mut self.doc, d);
            self.push_undo(before);
            self.selected = None;
            self.changed();
        }
    }

    pub fn apply_resize(&mut self, font: &FontVec, w: u32, h: u32) {
        self.reset_view();
        let d = doc::resize(&self.doc, font, w, h);
        let before = std::mem::replace(&mut self.doc, d);
        self.push_undo(before);
        self.selected = None;
        self.changed();
    }

    pub fn flatten(&self, font: &FontVec) -> image::RgbaImage {
        doc::render(&self.doc, font, None)
    }

    /// Draws the canvas and handles drawing with the current tool.
    pub fn canvas(&mut self, ui: &mut Ui, tool: Tool, style: Style, font: &FontVec, checker: &TextureHandle) {
        let rect = ui.max_rect();
        let painter = ui.painter_at(rect);
        painter.image(
            checker.id(),
            rect,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(rect.width() / 16.0, rect.height() / 16.0)),
            Color32::WHITE,
        );

        if self.tex_dirty {
            self.tex_dirty = false;
            let img = doc::render(&self.doc, font, self.skip);
            let ci = egui::ColorImage::from_rgba_unmultiplied(
                [img.width() as usize, img.height() as usize],
                img.as_raw(),
            );
            drop(img);
            match &mut self.tex {
                Some(t) => t.set(ci, TextureOptions::LINEAR),
                None => self.tex = Some(ui.ctx().load_texture("doc", ci, TextureOptions::LINEAR)),
            }
        }

        let size = self.doc.size();
        let margin = 24.0;
        let avail = (rect.size() - vec2(margin, margin) * 2.0).max(vec2(50.0, 50.0));
        // Big pictures shrink to fit the window; small ones show at their real size.
        let fit = (avail.x / size.x).min(avail.y / size.y).min(1.0 / self.ppp);

        // Pinch to zoom (or ⌘ + scroll) around the pointer; two-finger scroll moves the picture.
        if let Some(hover) = ui.input(|i| i.pointer.hover_pos()).filter(|p| rect.contains(*p)) {
            let (zd, scroll) = ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta));
            if zd != 1.0 {
                let old = fit * self.view_zoom;
                let max_zoom = 8.0 / self.ppp;
                self.view_zoom = (self.view_zoom * zd).clamp(0.2, (max_zoom / fit).max(1.0));
                let new = fit * self.view_zoom;
                let old_origin = rect.center() - size * old / 2.0 + self.pan;
                let under = (hover - old_origin) / old;
                let new_origin = hover - under * new;
                self.pan = new_origin - (rect.center() - size * new / 2.0);
            }
            if scroll != Vec2::ZERO {
                self.pan += scroll;
            }
        }
        let zoom = fit * self.view_zoom;
        // Keep the picture centred when it fits, and never let it slide out of view.
        let shown = size * zoom;
        let limit = ((shown - avail) / 2.0 + vec2(margin, margin)).max(Vec2::ZERO);
        self.pan = self.pan.clamp(-limit, limit);
        self.zoom = zoom;
        let origin = rect.center() - size * zoom / 2.0 + self.pan;
        let to_screen = |p: Pos2| origin + p.to_vec2() * zoom;
        let to_img = |s: Pos2| pos2((s.x - origin.x) / zoom, (s.y - origin.y) / zoom);
        let img_rect = Rect::from_min_size(origin, size * zoom);

        // Drop shadow and picture.
        painter.rect_filled(img_rect.translate(vec2(0.0, 3.0)).expand(2.0), 2.0, Color32::from_black_alpha(70));
        if let Some(t) = &self.tex {
            painter.image(t.id(), img_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }

        let resp = ui.interact(rect, Id::new("canvas"), Sense::click_and_drag());
        let hover_img = resp.hover_pos().map(to_img);
        let tol = 6.0 / zoom;
        let hit = |p: Pos2, doc: &Doc| -> Option<usize> {
            (0..doc.shapes.len()).rev().find(|&i| doc.shapes[i].hit(p, tol, font))
        };

        // Cursor
        if let Some(h) = hover_img {
            if self.drag.is_none() && tool != Tool::Crop && hit(h, &self.doc).is_some() {
                ui.ctx().set_cursor_icon(CursorIcon::Grab);
            } else if matches!(self.drag, Some(Drag::Move { .. })) {
                ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
            } else if tool == Tool::Text || tool == Tool::Callout {
                ui.ctx().set_cursor_icon(CursorIcon::Text);
            } else {
                ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
            }
        }

        // ---- interaction
        if resp.drag_started() {
            let start = ui.input(|i| i.pointer.press_origin()).map(to_img).unwrap_or_default();
            self.commit_text();
            if tool == Tool::Crop {
                self.crop = None;
                self.drag = Some(Drag::Crop { start });
            } else if let Some(i) = hit(start, &self.doc) {
                self.selected = Some(i);
                self.skip = Some(i);
                self.tex_dirty = true;
                self.drag = Some(Drag::Move { idx: i, last: start, before: self.doc.clone(), moved: false });
            } else {
                self.selected = None;
                self.drag = Some(Drag::New { start, cur: start, pts: vec![start] });
            }
        }
        if let Some(p) = resp.interact_pointer_pos().map(to_img) && resp.dragged() {
            let shift = ui.input(|i| i.modifiers.shift);
            match &mut self.drag {
                Some(Drag::New { start, cur, pts }) => {
                    let mut p = p;
                    if shift && matches!(tool, Tool::Rect | Tool::Ellipse | Tool::Blur) {
                        let d = p - *start;
                        let m = d.x.abs().max(d.y.abs());
                        p = *start + vec2(m * d.x.signum(), m * d.y.signum());
                    }
                    *cur = p;
                    if pts.last().is_none_or(|l| l.distance(p) > 1.0 / zoom) {
                        pts.push(p);
                    }
                }
                Some(Drag::Move { idx, last, moved, .. }) => {
                    let d = p - *last;
                    if d.length_sq() > 0.0 {
                        if let Some(s) = self.doc.shapes.get_mut(*idx) {
                            s.translate(d);
                        }
                        *last = p;
                        *moved = true;
                    }
                }
                Some(Drag::Crop { start }) => {
                    let r = Rect::from_two_pos(*start, p)
                        .intersect(Rect::from_min_size(Pos2::ZERO, size));
                    self.crop = Some(r);
                }
                None => {}
            }
        }
        if resp.drag_stopped() {
            match self.drag.take() {
                Some(Drag::New { start, cur, pts }) => {
                    let min_len = 3.0 / zoom;
                    let big = start.distance(cur) > min_len;
                    let straight = ui.input(|i| i.modifiers.shift);
                    let shape = match tool {
                        Tool::Arrow if big => Some(Shape::Arrow { a: start, b: cur, style }),
                        Tool::Line if big => Some(Shape::Line { a: start, b: cur, style }),
                        Tool::Rect if big => Some(Shape::Rect { a: start, b: cur, style }),
                        Tool::Ellipse if big => Some(Shape::Ellipse { a: start, b: cur, style }),
                        Tool::Blur if big => Some(Shape::Blur { a: start, b: cur }),
                        Tool::Pen => Some(Shape::Pen { pts, style }),
                        Tool::Highlight if straight => Some(Shape::Highlight { pts: vec![start, cur], style }),
                        Tool::Highlight => Some(Shape::Highlight { pts, style }),
                        Tool::Step => Some(self.new_step(cur, style)),
                        Tool::Text => {
                            self.start_text(cur, None, style, None);
                            None
                        }
                        Tool::Callout => {
                            // Dragged from the text spot onto the object; a plain click points at the click.
                            if big {
                                self.start_text(start, Some(cur), style, None);
                            } else {
                                self.start_text(start - vec2(80.0, 60.0) * self.ppp, Some(start), style, None);
                            }
                            None
                        }
                        _ => None,
                    };
                    if let Some(s) = shape {
                        self.add_shape(s);
                    }
                }
                Some(Drag::Move { before, moved, .. }) => {
                    if moved {
                        self.push_undo(before);
                    }
                    self.skip = None;
                    self.changed();
                }
                // Folding this `if` into a match guard would make the arm non-exhaustive.
                #[allow(clippy::collapsible_match)]
                Some(Drag::Crop { .. }) => {
                    if self.crop.is_some_and(|r| r.width() < 4.0 || r.height() < 4.0) {
                        self.crop = None;
                    }
                }
                None => {}
            }
        }
        if resp.clicked() && let Some(p) = resp.interact_pointer_pos().map(to_img) {
            let was_editing = self.text.is_some();
            self.commit_text();
            match hit(p, &self.doc) {
                Some(i) if tool != Tool::Crop => self.selected = Some(i),
                _ => {
                    self.selected = None;
                    if !was_editing {
                        match tool {
                            Tool::Text => self.start_text(p, None, style, None),
                            Tool::Callout => {
                                self.start_text(p - vec2(80.0, 60.0) * self.ppp, Some(p), style, None)
                            }
                            Tool::Step => {
                                let s = self.new_step(p, style);
                                self.add_shape(s);
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        if resp.double_clicked()
            && let Some(p) = resp.interact_pointer_pos().map(to_img)
            && let Some(i) = hit(p, &self.doc)
        {
            match self.doc.shapes[i].clone() {
                Shape::Text { pos, text, style } => {
                    self.start_text(pos, None, style, Some(i));
                    self.text.as_mut().unwrap().text = text;
                }
                Shape::Callout { tip, pos, text, style } => {
                    self.start_text(pos, Some(tip), style, Some(i));
                    self.text.as_mut().unwrap().text = text;
                }
                _ => {}
            }
        }

        // ---- live previews
        if let Some(Drag::New { start, cur, pts }) = &self.drag {
            let s = match tool {
                Tool::Arrow => Some(Shape::Arrow { a: *start, b: *cur, style }),
                Tool::Line => Some(Shape::Line { a: *start, b: *cur, style }),
                Tool::Rect => Some(Shape::Rect { a: *start, b: *cur, style }),
                Tool::Ellipse => Some(Shape::Ellipse { a: *start, b: *cur, style }),
                Tool::Blur => Some(Shape::Blur { a: *start, b: *cur }),
                Tool::Pen => Some(Shape::Pen { pts: pts.clone(), style }),
                Tool::Highlight if ui.input(|i| i.modifiers.shift) => {
                    Some(Shape::Highlight { pts: vec![*start, *cur], style })
                }
                Tool::Highlight => Some(Shape::Highlight { pts: pts.clone(), style }),
                Tool::Step => Some(self.new_step(*cur, style)),
                Tool::Callout => Some(Shape::Arrow { a: *start, b: *cur, style }),
                _ => None,
            };
            if let Some(s) = s {
                paint_preview(&painter, &s, &to_screen, zoom, font);
            }
        }
        if let Some(Drag::Move { idx, .. }) = &self.drag && let Some(s) = self.doc.shapes.get(*idx) {
            paint_preview(&painter, s, &to_screen, zoom, font);
        }

        // Selection outline
        if let Some(i) = self.selected && self.text.is_none() && let Some(s) = self.doc.shapes.get(i) {
            let b = s.bounds(font);
            let r = Rect::from_min_max(to_screen(b.min), to_screen(b.max)).expand(4.0);
            dashed_rect(&painter, r, Color32::from_rgb(90, 140, 255));
        }

        // Crop overlay
        if let Some(c) = self.crop {
            let cr = Rect::from_min_max(to_screen(c.min), to_screen(c.max));
            let dim = Color32::from_black_alpha(140);
            for r in [
                Rect::from_min_max(img_rect.min, pos2(img_rect.max.x, cr.min.y)),
                Rect::from_min_max(pos2(img_rect.min.x, cr.max.y), img_rect.max),
                Rect::from_min_max(pos2(img_rect.min.x, cr.min.y), pos2(cr.min.x, cr.max.y)),
                Rect::from_min_max(pos2(cr.max.x, cr.min.y), pos2(img_rect.max.x, cr.max.y)),
            ] {
                if r.is_positive() {
                    painter.rect_filled(r, 0.0, dim);
                }
            }
            painter.rect_stroke(cr, 0.0, Stroke::new(1.5, Color32::WHITE), StrokeKind::Outside);
            painter.text(
                cr.left_top() + vec2(4.0, -4.0),
                egui::Align2::LEFT_BOTTOM,
                format!("{} × {}", c.width().round(), c.height().round()),
                FontId::proportional(12.0),
                Color32::WHITE,
            );
            if self.drag.is_none() {
                let bar = Rect::from_center_size(
                    pos2(cr.center().x, (cr.max.y + 22.0).min(rect.max.y - 18.0)),
                    vec2(170.0, 30.0),
                );
                let mut apply = false;
                let mut cancel = false;
                ui.scope_builder(egui::UiBuilder::new().max_rect(bar), |ui| {
                    ui.horizontal_centered(|ui| {
                        apply = ui.button("Crop (Enter)").clicked();
                        cancel = ui.button("Cancel").clicked();
                    });
                });
                if apply || ui.input(|i| i.key_pressed(Key::Enter)) {
                    self.apply_crop();
                } else if cancel || ui.input(|i| i.key_pressed(Key::Escape)) {
                    self.crop = None;
                }
            }
        }

        // Text being typed
        self.text_ui(ui, &to_screen, zoom, font);
    }

    fn start_text(&mut self, pos: Pos2, tip: Option<Pos2>, style: Style, replace: Option<usize>) {
        self.text = Some(TextState { pos, tip, text: String::new(), style, replace, focus: true });
        self.selected = None;
        if replace.is_some() {
            self.skip = replace;
            self.tex_dirty = true;
        }
    }

    fn text_ui(&mut self, ui: &mut Ui, to_screen: &dyn Fn(Pos2) -> Pos2, zoom: f32, font: &FontVec) {
        let Some(t) = &mut self.text else { return };
        let px = t.style.font_px();
        let measure = doc::measure_text(font, px, if t.text.is_empty() { "M" } else { &t.text });
        let painter = ui.painter().clone();
        let mut text_pos = t.pos;
        if let Some(tip) = t.tip {
            paint_arrow(&painter, t.pos, tip, &t.style, to_screen, zoom);
            text_pos = doc::callout_text_rect(font, t.pos, tip, &t.text, &t.style).min;
        }
        let text_color = t.style.color;
        let tl = to_screen(text_pos);
        let size = measure * zoom + vec2(px * zoom * 1.5, 4.0);
        let edit_rect = Rect::from_min_size(tl, size);
        dashed_rect(&painter, edit_rect.expand(3.0), Color32::from_rgb(90, 140, 255));

        let enter = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter));
        let esc = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape));
        let edit = egui::TextEdit::multiline(&mut t.text)
            .font(annot_font(px * zoom))
            .text_color(text_color)
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .desired_rows(1)
            .desired_width(size.x)
            .id(Id::new("annot_text"));
        let r = ui.put(edit_rect, edit);
        if t.focus {
            r.request_focus();
            t.focus = false;
        }
        if enter || esc {
            self.commit_text();
        }
    }
}

pub fn dashed_rect(p: &egui::Painter, r: Rect, color: Color32) {
    let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
    p.extend(egui::Shape::dashed_line(&pts, Stroke::new(1.2, color), 5.0, 4.0));
}

fn paint_arrow(p: &egui::Painter, a: Pos2, b: Pos2, style: &Style, to_screen: &dyn Fn(Pos2) -> Pos2, zoom: f32) {
    let (end, head) = doc::arrow_geometry(a, b, style);
    let line_w = doc::arrow_dims(style).0;
    p.line_segment([to_screen(a), to_screen(end)], Stroke::new((line_w * zoom).max(1.0), style.color));
    p.add(egui::Shape::convex_polygon(head.iter().map(|q| to_screen(*q)).collect(), style.color, Stroke::NONE));
}

/// Draws a shape with egui while it is being drawn or moved.
fn paint_preview(p: &egui::Painter, s: &Shape, to_screen: &dyn Fn(Pos2) -> Pos2, zoom: f32, font: &FontVec) {
    let stroke = |st: &Style| Stroke::new((st.width * zoom).max(1.0), st.color);
    match s {
        Shape::Arrow { a, b, style } => paint_arrow(p, *a, *b, style, to_screen, zoom),
        Shape::Line { a, b, style } => {
            p.line_segment([to_screen(*a), to_screen(*b)], stroke(style));
        }
        Shape::Rect { a, b, style } => {
            p.rect_stroke(Rect::from_two_pos(to_screen(*a), to_screen(*b)), 0.0, stroke(style), StrokeKind::Middle);
        }
        Shape::Ellipse { a, b, style } => {
            let r = Rect::from_two_pos(to_screen(*a), to_screen(*b));
            p.add(egui::Shape::ellipse_stroke(r.center(), r.size() / 2.0, stroke(style)));
        }
        Shape::Pen { pts, style } => {
            p.add(egui::Shape::line(pts.iter().map(|q| to_screen(*q)).collect(), stroke(style)));
        }
        Shape::Text { pos, text, style } => {
            p.text(to_screen(*pos), egui::Align2::LEFT_TOP, text, annot_font(style.font_px() * zoom), style.color);
        }
        Shape::Callout { tip, pos, text, style } => {
            paint_arrow(p, *pos, *tip, style, to_screen, zoom);
            let r = doc::callout_text_rect(font, *pos, *tip, text, style);
            p.text(to_screen(r.min), egui::Align2::LEFT_TOP, text, annot_font(style.font_px() * zoom), style.color);
        }
        Shape::Blur { a, b } => {
            let r = Rect::from_two_pos(to_screen(*a), to_screen(*b));
            p.rect_filled(r, 0.0, Color32::from_white_alpha(40));
            dashed_rect(p, r, Color32::WHITE);
        }
        Shape::Step { pos, n, style } => {
            let r = doc::step_radius(style) * zoom;
            let c = to_screen(*pos);
            p.circle(c, r, style.color, Stroke::new((r * 0.1).max(1.0), Color32::WHITE));
            let text = n.to_string();
            let px = doc::step_font_px(style);
            let origin = doc::step_text_origin(font, px, *pos, &text);
            p.text(to_screen(origin), egui::Align2::LEFT_TOP, text, annot_font(px * zoom), doc::step_text_color(style.color));
        }
        Shape::Highlight { pts, style } => {
            // egui cannot multiply colours. Plain see-through colour comes close while
            // drawing, and on white paper it looks exactly like the final picture.
            let c = style.color;
            let [r, g, b, _] = c.to_srgba_unmultiplied();
            let st = Stroke::new(doc::highlight_width(style) * zoom, Color32::from_rgba_unmultiplied(r, g, b, doc::HIGHLIGHT_ALPHA));
            let pts: Vec<Pos2> = pts.iter().map(|q| to_screen(*q)).collect();
            if pts.len() == 1 {
                p.rect_filled(Rect::from_center_size(pts[0], Vec2::splat(st.width)), 0.0, st.color);
            } else {
                p.add(egui::Shape::line(pts, st));
            }
        }
    }
}
