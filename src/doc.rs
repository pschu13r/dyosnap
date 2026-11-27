//! The picture being edited: a base image plus a list of shapes drawn on top.
//! Shapes stay editable; the final picture is drawn with tiny-skia only when it changes.

use ab_glyph::{Font, FontVec, PxScale, ScaleFont};
use egui::{Color32, Pos2, Rect, Vec2, pos2, vec2};
use image::RgbaImage;
use std::sync::Arc;
use tiny_skia::{BlendMode, FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    pub color: Color32,
    /// Line width in image pixels.
    pub width: f32,
}

impl Style {
    pub fn font_px(&self) -> f32 {
        10.0 + self.width * 3.0
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    Arrow { a: Pos2, b: Pos2, style: Style },
    Line { a: Pos2, b: Pos2, style: Style },
    Rect { a: Pos2, b: Pos2, style: Style },
    Ellipse { a: Pos2, b: Pos2, style: Style },
    Pen { pts: Vec<Pos2>, style: Style },
    /// `pos` is the top-left corner of the text.
    Text { pos: Pos2, text: String, style: Style },
    /// An arrow from `pos` to `tip` (the object it points at), with text at `pos`, the arrow's tail.
    Callout { tip: Pos2, pos: Pos2, text: String, style: Style },
    Blur { a: Pos2, b: Pos2 },
    /// A numbered circle for steps (1, 2, 3 …). `pos` is its centre.
    Step { pos: Pos2, n: u32, style: Style },
    /// A see-through stroke that marks text, like a highlighter pen on paper.
    Highlight { pts: Vec<Pos2>, style: Style },
}

pub fn rect_of(a: Pos2, b: Pos2) -> Rect {
    Rect::from_two_pos(a, b)
}

fn seg_dist(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let len2 = ab.length_sq();
    if len2 < 1e-6 {
        return p.distance(a);
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

impl Shape {
    pub fn style(&self) -> Option<Style> {
        match self {
            Shape::Arrow { style, .. }
            | Shape::Line { style, .. }
            | Shape::Rect { style, .. }
            | Shape::Ellipse { style, .. }
            | Shape::Pen { style, .. }
            | Shape::Text { style, .. }
            | Shape::Callout { style, .. }
            | Shape::Step { style, .. }
            | Shape::Highlight { style, .. } => Some(*style),
            Shape::Blur { .. } => None,
        }
    }

    pub fn set_style(&mut self, s: Style) {
        match self {
            Shape::Arrow { style, .. }
            | Shape::Line { style, .. }
            | Shape::Rect { style, .. }
            | Shape::Ellipse { style, .. }
            | Shape::Pen { style, .. }
            | Shape::Text { style, .. }
            | Shape::Callout { style, .. }
            | Shape::Step { style, .. }
            | Shape::Highlight { style, .. } => *style = s,
            Shape::Blur { .. } => {}
        }
    }

    pub fn translate(&mut self, d: Vec2) {
        match self {
            Shape::Arrow { a, b, .. }
            | Shape::Line { a, b, .. }
            | Shape::Rect { a, b, .. }
            | Shape::Ellipse { a, b, .. }
            | Shape::Blur { a, b } => {
                *a += d;
                *b += d;
            }
            Shape::Pen { pts, .. } | Shape::Highlight { pts, .. } => pts.iter_mut().for_each(|p| *p += d),
            Shape::Text { pos, .. } | Shape::Step { pos, .. } => *pos += d,
            Shape::Callout { tip, pos, .. } => {
                *tip += d;
                *pos += d;
            }
        }
    }

    /// Whether the point (in image pixels) touches this shape.
    pub fn hit(&self, p: Pos2, tol: f32, font: &FontVec) -> bool {
        match self {
            Shape::Arrow { a, b, style } | Shape::Line { a, b, style } => {
                seg_dist(p, *a, *b) <= tol + style.width
            }
            Shape::Rect { a, b, style } => {
                let r = rect_of(*a, *b);
                let outer = r.expand(tol + style.width);
                let inner = r.shrink(tol + style.width);
                outer.contains(p) && !(inner.is_positive() && inner.contains(p))
            }
            Shape::Ellipse { a, b, style } => {
                let r = rect_of(*a, *b);
                let c = r.center();
                let (rx, ry) = (r.width() / 2.0, r.height() / 2.0);
                if rx < 1.0 || ry < 1.0 {
                    return r.expand(tol).contains(p);
                }
                let d = ((p.x - c.x) / rx).powi(2) + ((p.y - c.y) / ry).powi(2);
                let band = (tol + style.width) / rx.min(ry);
                (d.sqrt() - 1.0).abs() <= band
            }
            Shape::Pen { pts, style } => pts
                .windows(2)
                .any(|w| seg_dist(p, w[0], w[1]) <= tol + style.width),
            Shape::Callout { tip, pos, text, style } => {
                callout_text_rect(font, *pos, *tip, text, style).expand(tol).contains(p)
                    || seg_dist(p, *pos, *tip) <= tol + style.width
            }
            Shape::Highlight { pts, style } => {
                let r = tol + highlight_width(style) / 2.0;
                if pts.len() == 1 {
                    return p.distance(pts[0]) <= r;
                }
                pts.windows(2).any(|w| seg_dist(p, w[0], w[1]) <= r)
            }
            Shape::Step { pos, style, .. } => p.distance(*pos) <= step_radius(style) + tol,
            Shape::Text { .. } | Shape::Blur { .. } => self.bounds(font).expand(tol).contains(p),
        }
    }

    pub fn bounds(&self, font: &FontVec) -> Rect {
        match self {
            Shape::Arrow { a, b, style }
            | Shape::Line { a, b, style }
            | Shape::Rect { a, b, style }
            | Shape::Ellipse { a, b, style } => rect_of(*a, *b).expand(style.width),
            Shape::Blur { a, b } => rect_of(*a, *b),
            Shape::Pen { pts, style } => {
                let mut r = Rect::NOTHING;
                for p in pts {
                    r.extend_with(*p);
                }
                r.expand(style.width)
            }
            Shape::Highlight { pts, style } => {
                let mut r = Rect::NOTHING;
                for p in pts {
                    r.extend_with(*p);
                }
                r.expand(highlight_width(style) / 2.0)
            }
            Shape::Step { pos, style, .. } => Rect::from_center_size(*pos, Vec2::splat(step_radius(style) * 2.0)),
            Shape::Text { pos, text, style } => {
                let size = measure_text(font, style.font_px(), text);
                Rect::from_min_size(*pos, size)
            }
            Shape::Callout { tip, pos, text, style } => {
                callout_text_rect(font, *pos, *tip, text, style).union(rect_of(*pos, *tip).expand(style.width * 3.0))
            }
        }
    }
}

/// The radius of a numbered marker: it grows with the thickness slider, like text does.
pub fn step_radius(style: &Style) -> f32 {
    style.font_px() * 0.75
}

/// The size of the number inside a marker.
pub fn step_font_px(style: &Style) -> f32 {
    step_radius(style) * 1.15
}

/// White numbers on dark colours, black ones on light colours (yellow, white).
pub fn step_text_color(c: Color32) -> Color32 {
    let lum = 0.2126 * c.r() as f32 + 0.7152 * c.g() as f32 + 0.0722 * c.b() as f32;
    if lum > 160.0 { Color32::BLACK } else { Color32::WHITE }
}

/// The next free marker number: one more than the highest one in the picture.
pub fn next_step(shapes: &[Shape]) -> u32 {
    shapes
        .iter()
        .filter_map(|s| if let Shape::Step { n, .. } = s { Some(*n) } else { None })
        .max()
        .unwrap_or(0)
        + 1
}

/// Where to put a number's top-left corner so its digits sit in the middle of the
/// marker. Uses the glyph outlines, because digits have no descender.
pub fn step_text_origin(font: &FontVec, px: f32, center: Pos2, text: &str) -> Pos2 {
    let sf = font.as_scaled(PxScale::from(px));
    let mut x = 0.0;
    let mut prev = None;
    let mut ink = Rect::NOTHING;
    for c in text.chars() {
        let id = sf.glyph_id(c);
        if let Some(p) = prev {
            x += sf.kern(p, id);
        }
        let glyph = id.with_scale_and_position(PxScale::from(px), ab_glyph::point(x, sf.ascent()));
        if let Some(o) = font.outline_glyph(glyph) {
            let b = o.px_bounds();
            ink = ink.union(Rect::from_min_max(pos2(b.min.x, b.min.y), pos2(b.max.x, b.max.y)));
        }
        x += sf.h_advance(id);
        prev = Some(id);
    }
    if !ink.is_positive() {
        return center;
    }
    center - ink.center().to_vec2()
}

/// How wide the highlighter is: wide enough to cover a line of text.
pub fn highlight_width(style: &Style) -> f32 {
    style.width * 4.0 + 10.0
}

/// How strong the highlighter colour is (it is multiplied with the picture below).
pub const HIGHLIGHT_ALPHA: u8 = 130;

/// Where the text of an arrow-with-text sits: just behind the arrow's tail,
/// on the side facing away from the object the arrow points at.
pub fn callout_text_rect(font: &FontVec, tail: Pos2, tip: Pos2, text: &str, style: &Style) -> Rect {
    let size = measure_text(font, style.font_px(), if text.is_empty() { "M" } else { text });
    let d = tip - tail;
    let gap = style.font_px() * 0.3 + style.width;
    let min = if d.x.abs() >= d.y.abs() {
        if d.x >= 0.0 {
            pos2(tail.x - size.x - gap, tail.y - size.y / 2.0)
        } else {
            pos2(tail.x + gap, tail.y - size.y / 2.0)
        }
    } else if d.y >= 0.0 {
        pos2(tail.x - size.x / 2.0, tail.y - size.y - gap)
    } else {
        pos2(tail.x - size.x / 2.0, tail.y + gap)
    };
    Rect::from_min_size(min, size)
}

#[derive(Clone)]
pub struct Doc {
    pub base: Arc<RgbaImage>,
    pub shapes: Vec<Shape>,
}

impl Doc {
    pub fn size(&self) -> Vec2 {
        vec2(self.base.width() as f32, self.base.height() as f32)
    }
}

// ---------------------------------------------------------------- text

pub fn measure_text(font: &FontVec, px: f32, text: &str) -> Vec2 {
    let sf = font.as_scaled(PxScale::from(px));
    let line_h = sf.height() + sf.line_gap();
    let mut w: f32 = 0.0;
    let lines: Vec<&str> = text.split('\n').collect();
    for line in &lines {
        let mut x = 0.0;
        let mut prev = None;
        for c in line.chars() {
            let id = sf.glyph_id(c);
            if let Some(p) = prev {
                x += sf.kern(p, id);
            }
            x += sf.h_advance(id);
            prev = Some(id);
        }
        w = w.max(x);
    }
    vec2(w.ceil().max(px * 0.3), (line_h * lines.len() as f32).ceil())
}

fn draw_text(pm: &mut Pixmap, font: &FontVec, px: f32, origin: Pos2, text: &str, color: Color32) {
    let sf = font.as_scaled(PxScale::from(px));
    let line_h = sf.height() + sf.line_gap();
    let (w, h) = (pm.width() as i32, pm.height() as i32);
    let data = pm.data_mut();
    for (li, line) in text.split('\n').enumerate() {
        let mut x = origin.x;
        let y = origin.y + sf.ascent() + li as f32 * line_h;
        let mut prev = None;
        for c in line.chars() {
            let id = sf.glyph_id(c);
            if let Some(p) = prev {
                x += sf.kern(p, id);
            }
            let glyph = id.with_scale_and_position(PxScale::from(px), ab_glyph::point(x, y));
            x += sf.h_advance(id);
            prev = Some(id);
            let Some(outlined) = font.outline_glyph(glyph) else { continue };
            let bb = outlined.px_bounds();
            outlined.draw(|gx, gy, cov| {
                let ix = bb.min.x as i32 + gx as i32;
                let iy = bb.min.y as i32 + gy as i32;
                if ix < 0 || iy < 0 || ix >= w || iy >= h {
                    return;
                }
                let a = (cov.clamp(0.0, 1.0) * color.a() as f32 / 255.0).clamp(0.0, 1.0);
                let i = ((iy * w + ix) * 4) as usize;
                let src = [color.r() as f32, color.g() as f32, color.b() as f32, 255.0];
                for k in 0..4 {
                    let d = data[i + k] as f32;
                    data[i + k] = (src[k] * a + d * (1.0 - a)).round() as u8;
                }
            });
        }
    }
}

// ---------------------------------------------------------------- render

fn paint(c: Color32) -> Paint<'static> {
    let mut p = Paint::default();
    // Color32 keeps its colour multiplied by alpha; tiny-skia wants it without.
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    p.set_color_rgba8(r, g, b, a);
    p.anti_alias = true;
    p
}

fn stroke(width: f32) -> Stroke {
    Stroke {
        width: width.max(0.5),
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Stroke::default()
    }
}

/// Arrow sizes: the shaft is exactly as thick as every other stroke of the
/// same slider setting, the head grows with it.
pub fn arrow_dims(style: &Style) -> (f32, f32) {
    (style.width, style.width * 2.5 + 8.0)
}

/// Arrow geometry shared by the final picture and the live preview:
/// returns the end of the shaft and the three head points.
pub fn arrow_geometry(a: Pos2, b: Pos2, style: &Style) -> (Pos2, [Pos2; 3]) {
    let (line_w, head) = arrow_dims(style);
    let d = b - a;
    let len = d.length().max(0.001);
    let dir = d / len;
    let perp = vec2(-dir.y, dir.x);
    let head_len = head.min(len * 0.6).max(line_w * 1.5);
    let head_w = head_len * 0.5;
    let base = b - dir * head_len;
    let shaft_end = b - dir * head_len * 0.7;
    (shaft_end, [b, base + perp * head_w, base - perp * head_w])
}

fn box_blur_region(pm: &mut Pixmap, r: Rect) {
    let (w, h) = (pm.width() as i32, pm.height() as i32);
    let x0 = (r.min.x.floor() as i32).clamp(0, w);
    let y0 = (r.min.y.floor() as i32).clamp(0, h);
    let x1 = (r.max.x.ceil() as i32).clamp(0, w);
    let y1 = (r.max.y.ceil() as i32).clamp(0, h);
    let (rw, rh) = ((x1 - x0) as usize, (y1 - y0) as usize);
    if rw < 2 || rh < 2 {
        return;
    }
    let radius = ((rw.min(rh) as f32 / 6.0).clamp(4.0, 14.0) * (w as f32 / 1600.0).max(1.0)) as usize;
    let data = pm.data_mut();
    let mut buf: Vec<[u32; 4]> = Vec::with_capacity(rw * rh);
    for y in 0..rh {
        for x in 0..rw {
            let i = (((y0 as usize + y) * w as usize) + x0 as usize + x) * 4;
            buf.push([data[i] as u32, data[i + 1] as u32, data[i + 2] as u32, data[i + 3] as u32]);
        }
    }
    let mut tmp = buf.clone();
    for _ in 0..3 {
        blur_pass(&buf, &mut tmp, rw, rh, radius, true);
        blur_pass(&tmp, &mut buf, rw, rh, radius, false);
    }
    for y in 0..rh {
        for x in 0..rw {
            let i = (((y0 as usize + y) * w as usize) + x0 as usize + x) * 4;
            let p = buf[y * rw + x];
            data[i] = p[0] as u8;
            data[i + 1] = p[1] as u8;
            data[i + 2] = p[2] as u8;
            data[i + 3] = p[3] as u8;
        }
    }
}

fn blur_pass(src: &[[u32; 4]], dst: &mut [[u32; 4]], w: usize, h: usize, r: usize, horiz: bool) {
    let (outer, inner) = if horiz { (h, w) } else { (w, h) };
    let idx = |o: usize, i: usize| if horiz { o * w + i } else { i * w + o };
    for o in 0..outer {
        let mut sum = [0u32; 4];
        let get = |i: isize| src[idx(o, i.clamp(0, inner as isize - 1) as usize)];
        let r = r as isize;
        for i in -r..=r {
            let p = get(i);
            for k in 0..4 {
                sum[k] += p[k];
            }
        }
        let n = (2 * r + 1) as u32;
        for i in 0..inner as isize {
            let out = &mut dst[idx(o, i as usize)];
            for k in 0..4 {
                out[k] = sum[k] / n;
            }
            let add = get(i + r + 1);
            let sub = get(i - r);
            for k in 0..4 {
                sum[k] = sum[k] + add[k] - sub[k];
            }
        }
    }
}



fn draw_arrow(pm: &mut Pixmap, a: Pos2, b: Pos2, style: &Style) {
    let t = Transform::identity();
    let (end, head) = arrow_geometry(a, b, style);
    let line_w = arrow_dims(style).0;
    let mut pb = PathBuilder::new();
    pb.move_to(a.x, a.y);
    pb.line_to(end.x, end.y);
    if let Some(path) = pb.finish() {
        pm.stroke_path(&path, &paint(style.color), &stroke(line_w), t, None);
    }
    let mut pb = PathBuilder::new();
    pb.move_to(head[0].x, head[0].y);
    pb.line_to(head[1].x, head[1].y);
    pb.line_to(head[2].x, head[2].y);
    pb.close();
    if let Some(path) = pb.finish() {
        pm.fill_path(&path, &paint(style.color), FillRule::Winding, t, None);
    }
}

fn draw_shape(pm: &mut Pixmap, s: &Shape, font: &FontVec) {
    let t = Transform::identity();
    match s {
        Shape::Arrow { a, b, style } => draw_arrow(pm, *a, *b, style),
        Shape::Callout { tip, pos, text, style } => {
            draw_arrow(pm, *pos, *tip, style);
            let r = callout_text_rect(font, *pos, *tip, text, style);
            draw_text(pm, font, style.font_px(), r.min, text, style.color);
        }
        Shape::Line { a, b, style } => {
            let mut pb = PathBuilder::new();
            pb.move_to(a.x, a.y);
            pb.line_to(b.x, b.y);
            if let Some(path) = pb.finish() {
                pm.stroke_path(&path, &paint(style.color), &stroke(style.width), t, None);
            }
        }
        Shape::Rect { a, b, style } => {
            let r = rect_of(*a, *b);
            if let Some(rr) = tiny_skia::Rect::from_ltrb(r.min.x, r.min.y, r.max.x, r.max.y) {
                let path = PathBuilder::from_rect(rr);
                let mut st = stroke(style.width);
                st.line_join = LineJoin::Miter;
                pm.stroke_path(&path, &paint(style.color), &st, t, None);
            }
        }
        Shape::Ellipse { a, b, style } => {
            let r = rect_of(*a, *b);
            if let Some(rr) = tiny_skia::Rect::from_ltrb(r.min.x, r.min.y, r.max.x, r.max.y)
                && let Some(path) = PathBuilder::from_oval(rr)
            {
                pm.stroke_path(&path, &paint(style.color), &stroke(style.width), t, None);
            }
        }
        Shape::Pen { pts, style } => {
            if pts.len() == 1 {
                let p = pts[0];
                if let Some(path) = PathBuilder::from_circle(p.x, p.y, style.width / 2.0) {
                    pm.fill_path(&path, &paint(style.color), FillRule::Winding, t, None);
                }
                return;
            }
            let mut pb = PathBuilder::new();
            if let Some(f) = pts.first() {
                pb.move_to(f.x, f.y);
            }
            for p in pts.iter().skip(1) {
                pb.line_to(p.x, p.y);
            }
            if let Some(path) = pb.finish() {
                pm.stroke_path(&path, &paint(style.color), &stroke(style.width), t, None);
            }
        }
        Shape::Text { pos, text, style } => {
            // Flat text: no shadow or outline.
            draw_text(pm, font, style.font_px(), *pos, text, style.color);
        }
        Shape::Blur { a, b } => box_blur_region(pm, rect_of(*a, *b)),
        Shape::Step { pos, n, style } => {
            let r = step_radius(style);
            if let Some(path) = PathBuilder::from_circle(pos.x, pos.y, r) {
                pm.fill_path(&path, &paint(style.color), FillRule::Winding, t, None);
                // A thin white ring keeps the marker visible on dark and busy pictures.
                pm.stroke_path(&path, &paint(Color32::WHITE), &stroke((r * 0.1).max(1.0)), t, None);
            }
            let px = step_font_px(style);
            let text = n.to_string();
            let origin = step_text_origin(font, px, *pos, &text);
            draw_text(pm, font, px, origin, &text, step_text_color(style.color));
        }
        Shape::Highlight { pts, style } => {
            let c = style.color;
            let mut p = paint(Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), HIGHLIGHT_ALPHA));
            // Multiply: dark text below stays dark, white paper takes the colour.
            p.blend_mode = BlendMode::Multiply;
            let w = highlight_width(style);
            let mut pb = PathBuilder::new();
            match pts.as_slice() {
                [] => return,
                [only] => {
                    pb.move_to(only.x, only.y);
                    pb.line_to(only.x + 0.01, only.y);
                }
                [first, rest @ ..] => {
                    pb.move_to(first.x, first.y);
                    for q in rest {
                        pb.line_to(q.x, q.y);
                    }
                }
            }
            if let Some(path) = pb.finish() {
                let mut st = stroke(w);
                st.line_cap = LineCap::Square;
                pm.stroke_path(&path, &p, &st, t, None);
            }
        }
    }
}


/// Draws the base picture with all shapes (optionally leaving one out).
pub fn render(doc: &Doc, font: &FontVec, skip: Option<usize>) -> RgbaImage {
    let (w, h) = (doc.base.width(), doc.base.height());
    let Some(mut pm) = Pixmap::new(w.max(1), h.max(1)) else {
        return (*doc.base).clone();
    };
    // Premultiply alpha for tiny-skia.
    for (dst, src) in pm.data_mut().as_chunks_mut::<4>().0.iter_mut().zip(doc.base.as_raw().as_chunks::<4>().0) {
        let a = src[3] as u32;
        if a == 255 {
            dst.copy_from_slice(src);
        } else {
            dst[0] = (src[0] as u32 * a / 255) as u8;
            dst[1] = (src[1] as u32 * a / 255) as u8;
            dst[2] = (src[2] as u32 * a / 255) as u8;
            dst[3] = a as u8;
        }
    }
    for (i, s) in doc.shapes.iter().enumerate() {
        if Some(i) != skip {
            draw_shape(&mut pm, s, font);
        }
    }
    let mut data = pm.take();
    for px in data.as_chunks_mut::<4>().0 {
        let a = px[3] as u32;
        if a != 255 && a != 0 {
            px[0] = ((px[0] as u32 * 255 + a / 2) / a).min(255) as u8;
            px[1] = ((px[1] as u32 * 255 + a / 2) / a).min(255) as u8;
            px[2] = ((px[2] as u32 * 255 + a / 2) / a).min(255) as u8;
        }
    }
    RgbaImage::from_raw(w, h, data).unwrap_or_else(|| (*doc.base).clone())
}

/// Loads a font for text annotations: a bold system font if there is one.
pub fn load_font() -> (FontVec, Vec<u8>) {
    for c in crate::platform::ANNOTATION_FONTS {
        if let Ok(bytes) = std::fs::read(c) && let Ok(f) = FontVec::try_from_vec(bytes.clone()) {
            return (f, bytes);
        }
    }
    let bytes = epaint_default_fonts::UBUNTU_LIGHT.to_vec();
    (FontVec::try_from_vec(bytes.clone()).expect("built-in font"), bytes)
}

pub fn crop(doc: &Doc, r: Rect) -> Option<Doc> {
    let (w, h) = (doc.base.width() as f32, doc.base.height() as f32);
    let r = r.intersect(Rect::from_min_max(pos2(0.0, 0.0), pos2(w, h)));
    let (x, y) = (r.min.x.round() as u32, r.min.y.round() as u32);
    let (cw, ch) = (r.width().round() as u32, r.height().round() as u32);
    if cw < 2 || ch < 2 {
        return None;
    }
    let base = image::imageops::crop_imm(&*doc.base, x, y, cw, ch).to_image();
    let d = vec2(-(x as f32), -(y as f32));
    let shapes = doc
        .shapes
        .iter()
        .cloned()
        .map(|mut s| {
            s.translate(d);
            s
        })
        .collect();
    Some(Doc { base: Arc::new(base), shapes })
}

pub fn resize(doc: &Doc, font: &FontVec, w: u32, h: u32) -> Doc {
    let flat = render(doc, font, None);
    let out = image::imageops::resize(&flat, w.max(1), h.max(1), image::imageops::FilterType::Triangle);
    Doc { base: Arc::new(out), shapes: Vec::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_all_shapes() {
        let (font, _) = load_font();
        let mut base = RgbaImage::new(600, 400);
        for (x, y, p) in base.enumerate_pixels_mut() {
            *p = image::Rgba([(x % 255) as u8, (y % 255) as u8, 180, 255]);
        }
        let st = Style { color: Color32::from_rgb(235, 51, 36), width: 4.0 };
        let doc = Doc {
            base: Arc::new(base),
            shapes: vec![
                Shape::Blur { a: pos2(380.0, 20.0), b: pos2(580.0, 140.0) },
                Shape::Arrow { a: pos2(40.0, 40.0), b: pos2(200.0, 120.0), style: st },
                Shape::Rect { a: pos2(220.0, 30.0), b: pos2(360.0, 120.0), style: st },
                Shape::Line { a: pos2(40.0, 180.0), b: pos2(200.0, 180.0), style: st },
                Shape::Ellipse { a: pos2(220.0, 150.0), b: pos2(360.0, 230.0), style: st },
                Shape::Pen { pts: vec![pos2(400.0, 200.0), pos2(430.0, 170.0), pos2(470.0, 230.0), pos2(520.0, 180.0)], style: st },
                Shape::Text { pos: pos2(40.0, 260.0), text: "Hello there".into(), style: st },
                Shape::Callout { tip: pos2(300.0, 300.0), pos: pos2(400.0, 340.0), text: "Look here\nsecond line".into(), style: st },
                Shape::Highlight { pts: vec![pos2(30.0, 275.0), pos2(200.0, 275.0)], style: Style { color: Color32::from_rgb(255, 214, 0), width: 4.0 } },
                Shape::Step { pos: pos2(560.0, 360.0), n: 1, style: st },
                Shape::Step { pos: pos2(500.0, 360.0), n: 12, style: Style { color: Color32::from_rgb(255, 214, 0), width: 4.0 } },
            ],
        };
        let out = render(&doc, &font, None);
        assert_eq!(out.dimensions(), (600, 400));
        let _ = std::fs::create_dir_all("shots");
        out.save("shots/render-test.png").unwrap();
        let c = crop(&doc, Rect::from_min_max(pos2(10.0, 10.0), pos2(300.0, 200.0))).unwrap();
        assert_eq!(c.base.dimensions(), (290, 190));
    }

    /// Arrow, callout, line, rect, ellipse and pen all use the slider width.
    /// A highlighter on white paper shows its own colour, lightened, not a greyed one.
    #[test]
    fn highlighter_keeps_its_colour() {
        let (font, _) = load_font();
        let yellow = Style { color: Color32::from_rgb(255, 214, 0), width: 4.0 };
        let doc = Doc {
            base: Arc::new(RgbaImage::from_pixel(100, 60, image::Rgba([255, 255, 255, 255]))),
            shapes: vec![Shape::Highlight { pts: vec![pos2(10.0, 30.0), pos2(90.0, 30.0)], style: yellow }],
        };
        let px = render(&doc, &font, None).get_pixel(50, 30).0;
        let a = HIGHLIGHT_ALPHA as f32 / 255.0;
        assert_eq!(px[0], 255, "{px:?}");
        assert!((px[2] as f32 - 255.0 * (1.0 - a)).abs() <= 2.0, "{px:?}");
    }

    #[test]
    fn same_stroke_width_for_all_shapes() {
        let (font, _) = load_font();
        for width in [1.0, 4.0, 12.0, 40.0] {
            let st = Style { color: Color32::BLACK, width };
            let doc = Doc {
                base: Arc::new(RgbaImage::from_pixel(400, 1000, image::Rgba([255, 255, 255, 255]))),
                shapes: vec![
                    Shape::Arrow { a: pos2(20.0, 100.0), b: pos2(380.0, 100.0), style: st },
                    Shape::Callout { tip: pos2(380.0, 250.0), pos: pos2(20.0, 250.0), text: String::new(), style: st },
                    Shape::Line { a: pos2(20.0, 400.0), b: pos2(380.0, 400.0), style: st },
                    Shape::Rect { a: pos2(20.0, 500.0), b: pos2(380.0, 600.0), style: st },
                    Shape::Ellipse { a: pos2(20.0, 700.0), b: pos2(380.0, 900.0), style: st },
                    Shape::Pen { pts: vec![pos2(20.0, 950.0), pos2(200.0, 950.0), pos2(380.0, 950.0)], style: st },
                ],
            };
            let out = render(&doc, &font, None);
            // Ink across one column, in pixels, around a given y.
            let ink = |y: f32| -> f32 {
                let (a, b) = ((y - 30.0) as u32, (y + 30.0) as u32);
                (a..b).map(|y| (255 - out.get_pixel(100, y)[0]) as f32 / 255.0).sum()
            };
            let ellipse_top = ink(700.0);
            for (name, got) in [
                ("arrow", ink(100.0)),
                ("callout", ink(250.0)),
                ("line", ink(400.0)),
                ("rect", ink(500.0)),
                ("pen", ink(950.0)),
            ] {
                assert!((got - width).abs() < 0.3, "{name}: {got} px at width {width}");
            }
            // The ellipse top is slightly curved at x=100, so allow a bit more.
            assert!((ellipse_top - width).abs() < width * 0.15 + 0.3, "ellipse: {ellipse_top} px at width {width}");
        }
    }
}
