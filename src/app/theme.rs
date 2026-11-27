//! Colours, sizes and the egui style.

use super::*;

// ---------------------------------------------------------------- colours

pub(super) const BAR: Color32 = Color32::from_rgb(44, 44, 46);
pub(super) const BTN: Color32 = Color32::from_rgb(58, 58, 60);
pub(super) const BTN_HOVER: Color32 = Color32::from_rgb(72, 72, 76);
pub(super) const EDGE: Color32 = Color32::from_rgb(24, 24, 26);
pub(super) const BLUE: Color32 = Color32::from_rgb(40, 76, 186);
pub(super) const BLUE_HOVER: Color32 = Color32::from_rgb(54, 92, 206);
pub(super) const RED: Color32 = Color32::from_rgb(226, 38, 38);
pub(super) const TEXT: Color32 = Color32::from_rgb(235, 235, 240);
pub(super) const DIM_TEXT: Color32 = Color32::from_rgb(150, 150, 156);
/// Height of every control in the top bar (fields, buttons, button groups).
pub(super) const BAR_ITEM_H: f32 = TOOL_H;
/// Height of every control in the bottom tool bar.
pub(super) const TOOL_H: f32 = 26.0;
/// Width of every icon button, top and bottom.
pub(super) const TOOL_W: f32 = 36.0;
/// Text size inside the top bar, kept small so the buttons look as tall as the tool buttons.
pub(super) const BAR_FONT: f32 = 13.0;

pub(super) const PALETTE: [Color32; 10] = [
    Color32::from_rgb(235, 51, 36),
    Color32::from_rgb(255, 149, 0),
    Color32::from_rgb(255, 214, 10),
    Color32::from_rgb(52, 199, 89),
    Color32::from_rgb(0, 122, 255),
    Color32::from_rgb(175, 82, 222),
    Color32::from_rgb(255, 55, 140),
    Color32::from_rgb(255, 255, 255),
    Color32::from_rgb(142, 142, 147),
    Color32::from_rgb(0, 0, 0),
];

pub(super) fn setup_style(ctx: &egui::Context, font_bytes: Vec<u8>) {
    // ⌘+ / ⌘- / ⌘0 zoom the picture, not the whole interface.
    ctx.options_mut(|o| o.zoom_with_keyboard = false);
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert("annot".into(), Arc::new(egui::FontData::from_owned(font_bytes)));
    fonts.families.insert(egui::FontFamily::Name("annot".into()), vec!["annot".into()]);
    // Small system font with the ⌥ ⌘ ⇧ key symbols, used as a fallback.
    if let Ok(sym) = std::fs::read(platform::SYMBOL_FONT) {
        fonts.font_data.insert("symbols".into(), Arc::new(egui::FontData::from_owned(sym)));
        for fam in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts.families.entry(fam).or_default().push("symbols".into());
        }
    }
    ctx.set_fonts(fonts);

    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|style| {
        style.visuals = egui::Visuals::dark();
        let v = &mut style.visuals;
        v.panel_fill = BAR;
        v.window_fill = Color32::from_rgb(38, 38, 40);
        v.extreme_bg_color = Color32::from_rgb(30, 30, 32);
        v.selection.bg_fill = BLUE;
        v.widgets.inactive.weak_bg_fill = BTN;
        v.widgets.inactive.bg_fill = BTN;
        v.widgets.hovered.weak_bg_fill = BTN_HOVER;
        v.widgets.hovered.bg_fill = BTN_HOVER;
        v.widgets.noninteractive.fg_stroke.color = TEXT;
        v.widgets.inactive.fg_stroke.color = TEXT;
        style.spacing.button_padding = vec2(8.0, 3.0);
        style.spacing.interact_size.y = 22.0;
        for (ts, f) in style.text_styles.iter_mut() {
            // Same text sizes as the settings window.
            f.size = match ts {
                egui::TextStyle::Small => 11.0,
                egui::TextStyle::Heading => 17.0,
                _ => 13.0,
            };
        }
    });
}
