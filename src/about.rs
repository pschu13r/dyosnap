//! The About tab of the settings window: version, what DyoSnap is, links to the project and the licenses.
//! DyoSnap makes no network connections itself; every link opens in the browser.

use egui::{Align, Color32, FontFamily, FontId, Layout, RichText, TextureHandle, vec2};
use std::path::{Path, PathBuf};

const TEXT: Color32 = Color32::from_rgb(232, 232, 236);
const DIM: Color32 = Color32::from_rgb(140, 140, 146);

const REPO: &str = "https://github.com/pschu13r/dyosnap";
const ICON_PNG: &[u8] = include_bytes!("../linux/io.github.pschu13r.DyoSnap.png");

/// Draws the About tab of the settings window. `icon` holds the app icon once loaded.
pub fn draw(ui: &mut egui::Ui, icon: &mut Option<TextureHandle>) {
    if icon.is_none() {
        *icon = image::load_from_memory(ICON_PNG).ok().map(|img| {
            let img = img.to_rgba8();
            let size = [img.width() as usize, img.height() as usize];
            let ci = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
            ui.ctx().load_texture("about_icon", ci, egui::TextureOptions::LINEAR)
        });
    }
    ui.with_layout(Layout::top_down(Align::Center), |ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 4.0);
        ui.add_space(6.0);
        if let Some(t) = icon.as_ref() {
            ui.add(egui::Image::new(t).fit_to_exact_size(vec2(80.0, 80.0)));
        }
        ui.add_space(6.0);
        ui.label(RichText::new("DyoSnap").font(FontId::new(22.0, FontFamily::Name("annot".into()))).color(TEXT));
        ui.label(RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION"))).color(DIM));
        ui.add_space(10.0);
        ui.label(RichText::new("A lightweight, free and open-source app for screenshots and screen recordings.").color(TEXT));
        ui.add_space(2.0);
        ui.label(RichText::new("Nothing is ever uploaded.").color(DIM));
        ui.add_space(14.0);
        centered_row(ui, |ui| {
            if ui.button("Website").clicked() {
                open_url(REPO);
            }
            if ui.button("Check for Updates").clicked() {
                open_url(&format!("{REPO}/releases"));
            }
            if ui.button("Report a Problem").clicked() {
                open_url(&format!("{REPO}/issues"));
            }
        });
        ui.add_space(16.0);
        ui.label(RichText::new("© 2026 Philipp Schuler · MIT License").size(11.0).color(DIM));
        ui.add_space(2.0);
        if ui.button(RichText::new("Third-Party Licenses").size(11.0)).clicked() {
            match notices_file() {
                Some(p) => crate::platform::open_path(&p),
                None => open_url(&format!("{REPO}/blob/main/THIRD_PARTY_NOTICES.txt")),
            }
        }
        ui.add_space(4.0);
        ui.label(RichText::new("Provided as is, without warranty.").size(11.0).color(DIM));
    });
}

/// Lays out a row of buttons centred in the available width.
fn centered_row(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    // Measure the row once, invisibly, then draw it with the space split on both sides.
    let id = ui.id().with("row_w");
    let w: f32 = ui.ctx().data(|d| d.get_temp(id)).unwrap_or(0.0);
    ui.horizontal(|ui| {
        ui.add_space(((ui.available_width() - w) / 2.0).max(0.0));
        let start = ui.cursor().min.x;
        add(ui);
        let new_w = ui.cursor().min.x - start - ui.spacing().item_spacing.x;
        if (new_w - w).abs() > 0.5 {
            ui.ctx().data_mut(|d| d.insert_temp(id, new_w));
            ui.ctx().request_repaint();
        }
    });
}

fn open_url(url: &str) {
    crate::platform::open_path(Path::new(url));
}

/// THIRD_PARTY_NOTICES.txt of this copy: in the app bundle (macOS), next to the program
/// (Linux package), or at the top of the source folder (run from source).
fn notices_file() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [dir.join("../Resources"), dir.to_path_buf(), dir.join("../..")]
        .into_iter()
        .map(|d| d.join("THIRD_PARTY_NOTICES.txt"))
        .find(|p| p.is_file())
}
