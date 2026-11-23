//! Toolbar icons drawn with simple shapes (no image files needed).

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Undo,
    Redo,
    CameraPlus,
    Menu,
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
    Resize,
    Play,
    Pause,
    Folder,
    Gear,
    Video,
    Info,
}

fn arc(c: Pos2, r: f32, a0: f32, a1: f32, n: usize) -> Vec<Pos2> {
    (0..=n)
        .map(|i| {
            let t = a0 + (a1 - a0) * i as f32 / n as f32;
            pos2(c.x + r * t.cos(), c.y + r * t.sin())
        })
        .collect()
}

pub fn draw(p: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = rect.width().min(rect.height());
    let c = rect.center();
    let u = s / 20.0; // one unit on a 20x20 grid
    let at = |x: f32, y: f32| pos2(c.x + (x - 10.0) * u, c.y + (y - 10.0) * u);
    let st = Stroke::new(2.0 * u.max(0.8), color);
    let thin = Stroke::new(1.6 * u.max(0.8), color);
    match icon {
        Icon::Undo | Icon::Redo => {
            let flip = if icon == Icon::Redo { -1.0 } else { 1.0 };
            let pi = std::f32::consts::PI;
            let pts: Vec<Pos2> = arc(pos2(0.0, 0.0), 6.0, pi, 2.0 * pi + 0.9, 18)
                .into_iter()
                .map(|q| at(10.0 + (q.x + 1.0) * flip, 11.5 + q.y))
                .collect();
            let start = pts[0];
            p.add(Shape::line(pts, st));
            p.add(Shape::convex_polygon(
                vec![
                    start + vec2(-3.6 * u, -0.5 * u),
                    start + vec2(3.6 * u, -0.5 * u),
                    start + vec2(0.0, 4.5 * u),
                ],
                color,
                Stroke::NONE,
            ));
        }
        Icon::CameraPlus => {
            let body = Rect::from_min_max(at(1.5, 6.0), at(14.5, 16.5));
            p.rect_filled(body, 2.0 * u, color);
            p.rect_filled(Rect::from_min_max(at(5.0, 4.0), at(10.5, 6.5)), 1.0 * u, color);
            p.circle_filled(at(8.0, 11.2), 3.6 * u, Color32::from_gray(40));
            p.circle_filled(at(8.0, 11.2), 2.0 * u, color);
            p.line_segment([at(15.5, 11.0), at(19.5, 11.0)], st);
            p.line_segment([at(17.5, 9.0), at(17.5, 13.0)], st);
        }
        Icon::Menu => {
            for y in [5.5, 10.0, 14.5] {
                p.line_segment([at(3.5, y), at(16.5, y)], Stroke::new(2.2 * u, color));
            }
        }
        Icon::Arrow => {
            let a = at(4.0, 16.0);
            let b = at(16.0, 4.0);
            p.line_segment([a, at(13.0, 7.0)], st);
            p.add(Shape::convex_polygon(
                vec![b, at(9.5, 6.0), at(14.0, 10.5)],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Rect => {
            p.rect_stroke(
                Rect::from_min_max(at(3.0, 5.0), at(17.0, 15.0)),
                0.0,
                st,
                egui::StrokeKind::Middle,
            );
        }
        Icon::Line => {
            p.line_segment([at(4.0, 4.0), at(16.0, 16.0)], st);
        }
        Icon::Ellipse => {
            let pts = arc(c, 1.0, 0.0, std::f32::consts::TAU, 40)
                .into_iter()
                .map(|q| pos2(c.x + (q.x - c.x) * 7.5 * u, c.y + (q.y - c.y) * 5.5 * u))
                .collect::<Vec<_>>();
            p.add(Shape::line(pts, st));
        }
        Icon::Pen => {
            p.add(Shape::convex_polygon(
                vec![at(14.0, 3.0), at(17.0, 6.0), at(7.0, 16.0), at(4.0, 13.0)],
                color,
                Stroke::NONE,
            ));
            p.add(Shape::convex_polygon(
                vec![at(4.0, 13.0), at(7.0, 16.0), at(2.8, 17.2)],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Highlight => {
            // A chisel-tip marker above the band it leaves.
            p.add(Shape::convex_polygon(
                vec![at(12.5, 2.5), at(16.5, 6.5), at(10.0, 13.0), at(6.0, 9.0)],
                color,
                Stroke::NONE,
            ));
            p.add(Shape::convex_polygon(
                vec![at(6.0, 9.0), at(10.0, 13.0), at(7.5, 14.5), at(4.5, 11.5)],
                color,
                Stroke::NONE,
            ));
            let band = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 110);
            p.rect_filled(Rect::from_min_max(at(2.0, 15.5), at(18.0, 18.5)), 0.0, band);
        }
        Icon::Step => {
            p.circle_filled(c, 7.5 * u, color);
            let font = egui::FontId::proportional(11.0 * u);
            p.text(at(10.0, 10.3), egui::Align2::CENTER_CENTER, "1", font, Color32::from_gray(40));
        }
        Icon::Text | Icon::Callout => {
            let font = egui::FontId::proportional(11.0 * u);
            let pos = if icon == Icon::Text { at(10.0, 11.0) } else { at(8.5, 13.0) };
            p.text(pos, egui::Align2::CENTER_CENTER, "Aa", font, color);
            if icon == Icon::Callout {
                p.line_segment([at(12.5, 6.5), at(17.0, 2.5)], thin);
                p.add(Shape::convex_polygon(
                    vec![at(18.0, 1.5), at(14.5, 2.5), at(17.0, 5.0)],
                    color,
                    Stroke::NONE,
                ));
            }
        }
        Icon::Blur => {
            let mut pts = vec![at(10.0, 3.0)];
            pts.extend(arc(at(10.0, 12.5), 4.6 * u, -0.55, std::f32::consts::PI + 0.55, 20));
            p.add(Shape::convex_polygon(pts, color, Stroke::NONE));
        }
        Icon::Crop => {
            p.add(Shape::line(vec![at(6.0, 2.5), at(6.0, 14.0), at(17.5, 14.0)], st));
            p.add(Shape::line(vec![at(2.5, 6.0), at(14.0, 6.0), at(14.0, 17.5)], st));
            p.line_segment([at(4.0, 16.0), at(16.0, 4.0)], thin);
        }
        Icon::Resize => {
            p.line_segment([at(5.0, 15.0), at(15.0, 5.0)], st);
            p.add(Shape::convex_polygon(
                vec![at(17.0, 3.0), at(10.5, 4.0), at(16.0, 9.5)],
                color,
                Stroke::NONE,
            ));
            p.add(Shape::convex_polygon(
                vec![at(3.0, 17.0), at(4.0, 10.5), at(9.5, 16.0)],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Play => {
            p.add(Shape::convex_polygon(
                vec![at(5.0, 2.5), at(17.0, 10.0), at(5.0, 17.5)],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Pause => {
            p.rect_filled(Rect::from_min_max(at(5.0, 3.5), at(8.5, 16.5)), 1.0, color);
            p.rect_filled(Rect::from_min_max(at(11.5, 3.5), at(15.0, 16.5)), 1.0, color);
        }
        Icon::Folder => {
            p.rect_filled(Rect::from_min_max(at(2.5, 5.0), at(9.0, 8.0)), 1.5 * u, color);
            p.rect_filled(Rect::from_min_max(at(2.5, 6.5), at(17.5, 16.0)), 1.5 * u, color);
        }
        Icon::Video => {
            p.rect_filled(Rect::from_min_max(at(1.5, 5.0), at(13.0, 15.0)), 2.0 * u, color);
            p.add(Shape::convex_polygon(vec![at(14.5, 9.0), at(18.5, 6.0), at(18.5, 14.0), at(14.5, 11.0)], color, Stroke::NONE));
        }
        Icon::Info => {
            p.circle_stroke(c, 8.5 * u, Stroke::new(1.6 * u.max(0.8), color));
            p.circle_filled(at(10.0, 5.8), 1.3 * u, color);
            p.line_segment([at(10.0, 8.6), at(10.0, 14.6)], Stroke::new(2.0 * u.max(0.8), color));
        }
        Icon::Gear => {
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::TAU / 8.0;
                let d = vec2(a.cos(), a.sin());
                p.line_segment([c + d * 4.0 * u, c + d * 8.0 * u], Stroke::new(3.2 * u, color));
            }
            p.circle_filled(c, 6.0 * u, color);
            p.circle_filled(c, 2.6 * u, Color32::from_gray(45));
        }
    }
}
