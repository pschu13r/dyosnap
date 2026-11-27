//! Draws the app icon in every size an .iconset needs.
//! Used by scripts/bundle.sh: `cargo run --release --example app_icon -- <dir>.iconset`

use tiny_skia::{Color, FillRule, GradientStop, LinearGradient, Paint, PathBuilder, Pixmap, Point, Rect, SpreadMode, Transform};

fn rounded_rect(r: Rect, radius: f32) -> Option<tiny_skia::Path> {
    let (x, y, w, h, k) = (r.x(), r.y(), r.width(), r.height(), radius * 0.5523);
    let mut pb = PathBuilder::new();
    pb.move_to(x + radius, y);
    pb.line_to(x + w - radius, y);
    pb.cubic_to(x + w - radius + k, y, x + w, y + radius - k, x + w, y + radius);
    pb.line_to(x + w, y + h - radius);
    pb.cubic_to(x + w, y + h - radius + k, x + w - radius + k, y + h, x + w - radius, y + h);
    pb.line_to(x + radius, y + h);
    pb.cubic_to(x + radius - k, y + h, x, y + h - radius + k, x, y + h - radius);
    pb.line_to(x, y + radius);
    pb.cubic_to(x, y + radius - k, x + radius - k, y, x + radius, y);
    pb.close();
    pb.finish()
}

/// The icon on a 1024 grid, scaled to `size` pixels.
// The Linux SVG `linux/io.github.pschu13r.DyoSnap.svg` draws the same shapes and must be kept in sync.
fn draw(size: u32) -> Pixmap {
    let mut pm = Pixmap::new(size, size).expect("size");
    let t = Transform::from_scale(size as f32 / 1024.0, size as f32 / 1024.0);
    let mut paint = Paint { anti_alias: true, ..Default::default() };

    // Body: Apple's grid puts an 824 px square with ~185 px corners in the 1024 canvas.
    let body = Rect::from_xywh(100.0, 100.0, 824.0, 824.0).unwrap();
    paint.shader = LinearGradient::new(
        Point::from_xy(512.0, 100.0),
        Point::from_xy(512.0, 924.0),
        vec![
            GradientStop::new(0.0, Color::from_rgba8(64, 110, 230, 255)),
            GradientStop::new(1.0, Color::from_rgba8(30, 58, 150, 255)),
        ],
        SpreadMode::Pad,
        Transform::identity(),
    )
    .unwrap();
    pm.fill_path(&rounded_rect(body, 185.0).unwrap(), &paint, FillRule::Winding, t, None);

    // Camera, the same shape as the menu-bar icon.
    paint.shader = tiny_skia::Shader::SolidColor(Color::WHITE);
    pm.fill_path(
        &rounded_rect(Rect::from_xywh(232.0, 352.0, 560.0, 380.0).unwrap(), 56.0).unwrap(),
        &paint,
        FillRule::Winding,
        t,
        None,
    );
    pm.fill_path(
        &rounded_rect(Rect::from_xywh(396.0, 292.0, 232.0, 100.0).unwrap(), 28.0).unwrap(),
        &paint,
        FillRule::Winding,
        t,
        None,
    );
    // Lens: a blue ring with a white centre.
    paint.shader = tiny_skia::Shader::SolidColor(Color::from_rgba8(42, 76, 180, 255));
    pm.fill_path(&PathBuilder::from_circle(512.0, 542.0, 132.0).unwrap(), &paint, FillRule::Winding, t, None);
    paint.shader = tiny_skia::Shader::SolidColor(Color::WHITE);
    pm.fill_path(&PathBuilder::from_circle(512.0, 542.0, 76.0).unwrap(), &paint, FillRule::Winding, t, None);
    pm
}

fn main() {
    let dir = std::env::args().nth(1).expect("usage: app_icon <dir>.iconset");
    std::fs::create_dir_all(&dir).expect("create iconset dir");
    for base in [16u32, 32, 128, 256, 512] {
        for scale in [1u32, 2] {
            let px = base * scale;
            let name = if scale == 1 { format!("icon_{base}x{base}.png") } else { format!("icon_{base}x{base}@2x.png") };
            let pm = draw(px);
            let rgba: Vec<u8> = pm
                .pixels()
                .iter()
                .flat_map(|c| {
                    let c = c.demultiply();
                    [c.red(), c.green(), c.blue(), c.alpha()]
                })
                .collect();
            image::RgbaImage::from_raw(px, px, rgba)
                .expect("buffer")
                .save(format!("{dir}/{name}"))
                .expect("write png");
        }
    }
}
