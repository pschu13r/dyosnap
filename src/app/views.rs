//! The parts of the main window: top and bottom bars, the start page, the video view,
//! the editor's shortcuts and the small widgets they share.

use super::*;

/// A row of controls of height `h`, centred vertically in the bar.
pub(super) fn centered_row<R>(ui: &mut Ui, h: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    let full = ui.max_rect();
    let r = Rect::from_center_size(full.center(), vec2(full.width(), h));
    ui.scope_builder(
        egui::UiBuilder::new().max_rect(r).layout(Layout::left_to_right(Align::Center)),
        add,
    )
    .inner
}

pub(super) fn video_options(ui: &mut Ui, s: &mut Settings, devices: &Arc<Mutex<Option<Devices>>>, has_ff: bool) {
    let mut changed = false;
    ui.add_enabled_ui(has_ff, |ui| {
        egui::ComboBox::from_id_salt("fps")
            .width(240.0)
            .selected_text(format!("{} frames per second", s.fps))
            .show_ui(ui, |ui| {
                for f in [10, 15, 24, 30, 60] {
                    changed |= ui.selectable_value(&mut s.fps, f, format!("{f} frames per second")).changed();
                }
            });
        egui::ComboBox::from_id_salt("quality")
            .width(240.0)
            .selected_text(s.quality.label())
            .show_ui(ui, |ui| {
                for q in [Quality::High, Quality::Medium, Quality::Low] {
                    changed |= ui.selectable_value(&mut s.quality, q, q.label()).changed();
                }
            });
    });
    changed |= ui.checkbox(&mut s.capture_cursor, "Capture mouse cursor").changed();
    if cfg!(target_os = "linux") {
        // Wayland tells no app about clicks in other apps' windows.
        let mut off = false;
        ui.add_enabled(false, egui::Checkbox::new(&mut off, "Capture mouse clicks"))
            .on_disabled_hover_text("Not on Linux: Wayland tells no app about clicks in other windows.");
    } else {
        changed |= ui.checkbox(&mut s.capture_clicks, "Capture mouse clicks").changed();
    }
    ui.label("Audio input:");
    let inputs = devices.lock().ok().and_then(|d| d.as_ref().map(|d| d.audio_inputs.clone())).unwrap_or_default();
    let current = s.audio.clone().unwrap_or_else(|| "Do Not Record Audio".into());
    egui::ComboBox::from_id_salt("audio").width(240.0).selected_text(current).show_ui(ui, |ui| {
        changed |= ui.selectable_value(&mut s.audio, None, "Do Not Record Audio").changed();
        if has_ff {
            for name in inputs {
                changed |= ui.selectable_value(&mut s.audio, Some(name.clone()), name).changed();
            }
        } else {
            changed |= ui
                .selectable_value(&mut s.audio, Some("Default".into()), "Default microphone")
                .changed();
        }
    });
    if !has_ff {
        ui.label(
            egui::RichText::new("Install ffmpeg for frame rate, quality, pause and trimming.")
                .small()
                .color(DIM_TEXT),
        );
    }
    if changed {
        s.save();
    }
}

pub(super) fn capture_menu(ui: &mut Ui, st: &Settings) -> Option<Action> {
    let mut a = None;
    let delay = st.delay_secs;
    let row = |ui: &mut Ui, label: &str, id: &str| {
        ui.add(egui::Button::new(label).shortcut_text(st.hotkey_short(id)).min_size(vec2(220.0, 0.0)))
            .clicked()
    };
    if row(ui, "Capture Area", "area") {
        a = Some(Action::Capture(CaptureKind::Area));
    }
    if row(ui, "Capture Previous Area", "previous") {
        a = Some(Action::Capture(CaptureKind::Previous));
    }
    if row(ui, "Capture Full Screen", "full") {
        a = Some(Action::Capture(CaptureKind::FullScreen));
    }
    if row(ui, "Capture Window", "window") {
        a = Some(Action::Capture(CaptureKind::Window));
    }
    if row(ui, &format!("Timed Capture ({delay} s)"), "timed") {
        a = Some(Action::Capture(CaptureKind::Timed(delay)));
    }
    if row(ui, "Record Video", "record") {
        a = Some(Action::Record);
    }
    ui.separator();
    if row(ui, "Open Picture…", "") {
        a = Some(Action::OpenImage);
    }
    if row(ui, "Open from Clipboard", "clipboard") {
        a = Some(Action::OpenClipboard);
    }
    if row(ui, "Open Latest Image", "latest") {
        a = Some(Action::OpenLatest);
    }
    a
}

/// A joined row of `n` buttons, each `w` wide; the group is exactly `h` tall.
pub(super) fn group<R>(ui: &mut Ui, n: usize, w: f32, h: f32, add: impl FnOnce(&mut Ui) -> R) -> egui::InnerResponse<R> {
    let size = vec2(n as f32 * w + (n as f32 - 1.0) + 2.0, h);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, 5.0, EDGE);
    let inner = ui
        .scope_builder(
            egui::UiBuilder::new().max_rect(rect.shrink(1.0)).layout(Layout::left_to_right(Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = 1.0;
                add(ui)
            },
        )
        .inner;
    egui::InnerResponse::new(inner, resp)
}

pub(super) fn icon_button(ui: &mut Ui, icon: Icon, selected: bool, enabled: bool, size: Vec2, tip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let bg = if selected {
        BLUE
    } else if enabled && resp.hovered() {
        BTN_HOVER
    } else {
        BTN
    };
    ui.painter().rect_filled(rect, 3.0, bg);
    let fg = if enabled { Color32::WHITE } else { Color32::from_gray(105) };
    let s = 22.0f32.min(size.y - 8.0);
    icons::draw(ui.painter(), Rect::from_center_size(rect.center(), vec2(s, s)), icon, fg);
    resp.on_hover_text(tip)
}

pub(super) fn colored_icon_button(ui: &mut Ui, icon: Icon, fill: Color32, size: Vec2, tip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let bg = if resp.hovered() { BLUE_HOVER } else { fill };
    ui.painter().rect_filled(rect, CornerRadius { nw: 0, sw: 0, ne: 5, se: 5 }, bg);
    icons::draw(ui.painter(), Rect::from_center_size(rect.center(), vec2(16.0, 16.0)), icon, Color32::WHITE);
    resp.on_hover_text(tip)
}

pub(super) fn text_button(ui: &mut Ui, text: &str, fill: Color32, hover: Color32, enabled: bool, size: Vec2) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let bg = if enabled && resp.hovered() { hover } else { fill };
    let radius = if fill == BLUE { CornerRadius { nw: 5, sw: 5, ne: 0, se: 0 } } else { CornerRadius::same(5) };
    ui.painter().rect_filled(rect, radius, if enabled { bg } else { bg.gamma_multiply(0.6) });
    ui.painter().rect_stroke(rect, radius, Stroke::new(1.0, EDGE), StrokeKind::Inside);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(BAR_FONT),
        if enabled { Color32::WHITE } else { Color32::from_gray(130) },
    );
    resp
}

pub(super) fn big_button(ui: &mut Ui, label: &str, key: &str, w: f32, enabled: bool, red: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 34.0), if enabled { Sense::click() } else { Sense::hover() });
    let bg = if enabled && resp.hovered() { BTN_HOVER } else { BTN };
    let p = ui.painter();
    p.rect_filled(rect, 6.0, bg);
    p.rect_stroke(rect, 6.0, Stroke::new(1.0, EDGE), StrokeKind::Inside);
    if red {
        p.circle_filled(pos2(rect.min.x + 20.0, rect.center().y), 6.0, RED);
    }
    let x = rect.min.x + if red { 36.0 } else { 16.0 };
    p.text(pos2(x, rect.center().y), Align2::LEFT_CENTER, label, FontId::proportional(BAR_FONT), if enabled { TEXT } else { DIM_TEXT });
    if !key.is_empty() {
        p.text(pos2(rect.max.x - 14.0, rect.center().y), Align2::RIGHT_CENTER, key, FontId::proportional(12.0), DIM_TEXT);
    }
    resp
}

impl DyoSnap {
    pub(super) fn top_bar(&mut self, ui: &mut Ui) -> Option<Action> {
        let mut action = None;
        let ctx = ui.ctx().clone();
        ui.spacing_mut().item_spacing.x = 10.0;
        centered_row(ui, BAR_ITEM_H, |ui| {
            ui.add_space(6.0);
            let has_ed = self.ed.is_some();
            if self.video.is_none() {
                group(ui, 2, TOOL_W, BAR_ITEM_H, |ui| {
                    let (cu, cr) = self.ed.as_ref().map(|e| (e.can_undo(), e.can_redo())).unwrap_or((false, false));
                    if icon_button(ui, Icon::Undo, false, cu, vec2(TOOL_W, BAR_ITEM_H - 2.0), &format!("Undo ({})", crate::settings::keys_text(false, "Z"))).clicked() {
                        if let Some(e) = &mut self.ed {
                            e.undo();
                        }
                    }
                    if icon_button(ui, Icon::Redo, false, cr, vec2(TOOL_W, BAR_ITEM_H - 2.0), &format!("Redo ({})", crate::settings::keys_text(true, "Z"))).clicked() {
                        if let Some(e) = &mut self.ed {
                            e.redo();
                        }
                    }
                });
            }
            let cam = group(ui, 1, TOOL_W, BAR_ITEM_H, |ui| icon_button(ui, Icon::CameraPlus, false, !self.busy(), vec2(TOOL_W, BAR_ITEM_H - 2.0), "New capture"))
                .inner;
            egui::Popup::menu(&cam).show(|ui| {
                ui.set_min_width(230.0);
                if let Some(a) = capture_menu(ui, &self.settings) {
                    action = Some(a);
                }
            });

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(6.0);
                // Copy + menu, joined like one control.
                ui.spacing_mut().item_spacing.x = 1.0;
                let menu_btn = colored_icon_button(ui, Icon::Menu, BLUE, vec2(32.0, BAR_ITEM_H), "More");
                egui::Popup::menu(&menu_btn).show(|ui| {
                    ui.set_min_width(200.0);
                    if let Some(a) = self.more_menu(ui) {
                        action = Some(a);
                    }
                });
                let can_copy = has_ed || self.video.is_some();
                let copy = text_button(ui, "Copy", BLUE, BLUE_HOVER, can_copy, vec2(62.0, BAR_ITEM_H));
                if copy.clicked() {
                    if has_ed {
                        self.copy_picture();
                    } else {
                        self.save_video(true);
                    }
                }
                copy.on_hover_text(format!("Copy to the clipboard ({}). Nothing is uploaded.", crate::settings::keys_text(false, "C")));
                ui.spacing_mut().item_spacing.x = 10.0;
                let saving = self.video_saving.is_some();
                // Videos say what they become ("Save as GIF"): the format is set in Preferences
                // and changes what the export does.
                let video_ext = self.video.as_ref().map(|v| video_save_ext(v, self.settings.video_format));
                let label = match (&video_ext, saving) {
                    (_, true) => "Saving…".to_string(),
                    (Some(ext), false) => format!("Save as {}", ext.to_uppercase()),
                    (None, false) => "Save".to_string(),
                };
                let save_w = if video_ext.is_some() { 98.0 } else { 58.0 };
                let save = text_button(ui, &label, BTN, BTN_HOVER, can_copy && !saving, vec2(save_w, BAR_ITEM_H));
                if save.clicked() {
                    if has_ed {
                        self.save_picture(false);
                    } else {
                        self.save_video(false);
                    }
                }
                save.on_hover_text(format!("Save to {} ({})", short_path(&self.settings.save_dir), crate::settings::keys_text(false, "S")));

                // File name with its extension.
                let ext = video_ext.clone().unwrap_or_else(|| self.settings.format.ext().to_string());
                ui.spacing_mut().item_spacing.x = 0.0;
                let field_bg = Color32::from_rgb(36, 36, 38);
                let ext_text = format!(".{ext}");
                let ext_w = 12.0 + 7.5 * ext_text.len() as f32;
                let (ext_rect, ext_resp) = ui.allocate_exact_size(vec2(ext_w, BAR_ITEM_H), Sense::click());
                let er = CornerRadius { nw: 0, sw: 0, ne: 5, se: 5 };
                ui.painter().rect_filled(ext_rect, er, field_bg);
                ui.painter().rect_stroke(ext_rect, er, Stroke::new(1.0, EDGE), StrokeKind::Inside);
                ui.painter().text(ext_rect.center(), Align2::CENTER_CENTER, ext_text, FontId::proportional(BAR_FONT), DIM_TEXT);
                if self.ed.is_some() {
                    let ext_resp = ext_resp.on_hover_text("Click to switch between PNG and JPG");
                    if ext_resp.clicked() {
                        self.settings.format = match self.settings.format {
                            ImageFormat::Png => ImageFormat::Jpg,
                            ImageFormat::Jpg => ImageFormat::Png,
                        };
                        self.settings.save();
                    }
                }
                let w = (ui.available_width() - 6.0).max(80.0);
                let (name_rect, _) = ui.allocate_exact_size(vec2(w, BAR_ITEM_H), Sense::hover());
                let nr = CornerRadius { nw: 5, sw: 5, ne: 0, se: 0 };
                ui.painter().rect_filled(name_rect, nr, field_bg);
                ui.painter().rect_stroke(name_rect, nr, Stroke::new(1.0, EDGE), StrokeKind::Inside);
                let name: Option<&mut String> = if let Some(e) = &mut self.ed {
                    Some(&mut e.name)
                } else if let Some(v) = &mut self.video {
                    Some(&mut v.name)
                } else {
                    None
                };
                match name {
                    Some(n) => {
                        ui.put(
                            Rect::from_center_size(name_rect.center() + vec2(0.0, 1.5), vec2(name_rect.width() - 20.0, 18.0)),
                            egui::TextEdit::singleline(n)
                                .frame(Frame::NONE)
                                .margin(Margin::ZERO)
                                .horizontal_align(Align::Center)
                                .font(FontId::proportional(BAR_FONT))
                                .text_color(Color32::from_rgb(200, 200, 205))
                                .desired_width(f32::INFINITY),
                        )
                        .on_hover_text("File name");
                    }
                    None => {
                        ui.painter().text(name_rect.center(), Align2::CENTER_CENTER, "DyoSnap", FontId::proportional(BAR_FONT), DIM_TEXT);
                    }
                }
                let _ = &ctx;
            });
        });
        action
    }

    pub(super) fn more_menu(&mut self, ui: &mut Ui) -> Option<Action> {
        let mut action = None;
        if self.ed.is_some() && ui.button("Save As…").clicked() {
            self.save_picture(true);
        }
        if self.ed.is_some() {
            let app = platform::editor_label(&self.settings.external_editor);
            if ui.button(format!("Open in {app}")).clicked() {
                self.open_in_external_editor();
            }
        }
        let saved = self.ed.as_ref().and_then(|e| e.saved.clone()).or_else(|| self.video.as_ref().and_then(|v| v.saved.clone()));
        if let Some(p) = saved {
            if ui.button(format!("Show in {}", platform::FILE_MANAGER)).clicked() {
                platform::reveal_path(&p);
            }
            if ui.button("Copy file path").clicked() {
                platform::copy_text(&p.display().to_string());
                self.toast("Copied the file path", None);
            }
        }
        if self.video.is_some() {
            if let Some(v) = &self.video {
                if ui.button(format!("Open in {}", platform::VIDEO_PLAYER)).clicked() {
                    platform::open_path(v.saved.as_ref().unwrap_or(&v.path));
                }
            }
        }
        if self.ed.is_some() || self.video.is_some() {
            if ui.add(egui::Button::new("Close").shortcut_text(crate::settings::keys_text(false, "W"))).clicked() {
                self.ed = None;
                self.video = None;
                self.recent_dirty = true;
                self.tray_menu_dirty = true;
            }
            ui.separator();
        }
        if ui.button("Open Picture…").clicked() {
            action = Some(Action::OpenImage);
        }
        if ui.button("Open Saved Snaps Folder").clicked() {
            action = Some(Action::OpenFolder);
        }
        if ui.button("Settings…").clicked() {
            action = Some(Action::Settings);
        }
        ui.separator();
        if ui.button("Quit DyoSnap").clicked() {
            action = Some(Action::Quit);
        }
        action
    }

    pub(super) fn bottom_bar(&mut self, ui: &mut Ui) {
        let enabled = self.ed.is_some();
        centered_row(ui, TOOL_H, |ui| {
            ui.add_space(14.0);
            ui.spacing_mut().item_spacing.x = 10.0;
            // Colour
            let [r, g, b, a] = self.settings.color;
            let mut color = Color32::from_rgba_unmultiplied(r, g, b, a);
            let (rect, swatch) = ui.allocate_exact_size(vec2(44.0, TOOL_H), if enabled { Sense::click() } else { Sense::hover() });
            ui.painter().rect_filled(rect, 5.0, BTN);
            ui.painter().rect_stroke(rect, 5.0, Stroke::new(1.0, EDGE), StrokeKind::Inside);
            ui.painter().rect_filled(rect.shrink(4.0), 3.0, if enabled { color } else { color.gamma_multiply(0.4) });
            let swatch = swatch.on_hover_text("Colour");
            let mut changed = false;
            egui::Popup::menu(&swatch).show(|ui| {
                ui.horizontal(|ui| {
                    for (i, c) in PALETTE.iter().enumerate() {
                        if i == 5 {
                            ui.end_row();
                        }
                        let (r, resp) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
                        ui.painter().rect_filled(r, 4.0, *c);
                        if *c == color {
                            ui.painter().rect_stroke(r.expand(2.0), 5.0, Stroke::new(2.0, Color32::WHITE), StrokeKind::Inside);
                        }
                        if resp.clicked() {
                            color = *c;
                            changed = true;
                            ui.close();
                        }
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Custom:");
                    if ui.color_edit_button_srgba(&mut color).changed() {
                        changed = true;
                    }
                });
            });

            // Thickness
            let mut width = self.settings.width;
            ui.add_enabled_ui(enabled, |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let (r, _) = ui.allocate_exact_size(vec2(10.0, TOOL_H), Sense::hover());
                ui.painter().circle_filled(r.center(), 3.0, DIM_TEXT);
                ui.spacing_mut().slider_width = 100.0;
                if ui.add(egui::Slider::new(&mut width, 1.0..=20.0).show_value(false)).on_hover_text("Thickness and text size").changed() {
                    changed = true;
                }
                let (r, _) = ui.allocate_exact_size(vec2(18.0, TOOL_H), Sense::hover());
                ui.painter().circle_filled(r.center(), 7.5, DIM_TEXT);
            });
            if changed {
                self.settings.color = color.to_srgba_unmultiplied();
                self.settings.width = width;
                let style = self.style();
                if let Some(e) = &mut self.ed {
                    e.restyle_selected(style);
                }
            }

            let sz = vec2(TOOL_W, TOOL_H - 2.0);
            let tool_btn = |ui: &mut Ui, icon: Icon, tool: Tool, this: &mut Self| {
                if icon_button(ui, icon, this.tool == tool && enabled, enabled, sz, tool.tip()).clicked() {
                    this.tool = tool;
                    if let Some(e) = &mut this.ed {
                        e.commit_text();
                        if tool != Tool::Crop {
                            e.crop = None;
                        }
                    }
                }
            };
            group(ui, 6, sz.x, TOOL_H, |ui| {
                tool_btn(ui, Icon::Arrow, Tool::Arrow, self);
                tool_btn(ui, Icon::Rect, Tool::Rect, self);
                tool_btn(ui, Icon::Line, Tool::Line, self);
                tool_btn(ui, Icon::Ellipse, Tool::Ellipse, self);
                tool_btn(ui, Icon::Pen, Tool::Pen, self);
                tool_btn(ui, Icon::Highlight, Tool::Highlight, self);
            });
            group(ui, 3, sz.x, TOOL_H, |ui| {
                tool_btn(ui, Icon::Text, Tool::Text, self);
                tool_btn(ui, Icon::Callout, Tool::Callout, self);
                tool_btn(ui, Icon::Step, Tool::Step, self);
            });
            group(ui, 3, sz.x, TOOL_H, |ui| {
                tool_btn(ui, Icon::Blur, Tool::Blur, self);
                tool_btn(ui, Icon::Crop, Tool::Crop, self);
                if icon_button(ui, Icon::Resize, false, enabled, sz, "Resize picture").clicked() {
                    if let Some(e) = &mut self.ed {
                        e.resize = Some(ResizeDlg { w: e.doc.base.width(), h: e.doc.base.height(), keep: true });
                    }
                }
            });

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(14.0);
                let wide = ui.available_width() > 130.0;
                if let Some(e) = self.ed.as_mut().filter(|_| wide) {
                    let r = ui.add(
                        egui::Label::new(
                            egui::RichText::new(format!(
                                "{} × {}   {}%",
                                e.doc.base.width(),
                                e.doc.base.height(),
                                (e.zoom * e.ppp * 100.0).round()
                            ))
                            .color(DIM_TEXT),
                        )
                        .sense(Sense::click()),
                    );
                    if r.on_hover_text(format!("Pinch to zoom, two fingers to move. Click or {} to fit the window.", crate::settings::keys_text(false, "0"))).clicked() {
                        e.reset_view();
                    }
                }
            });
        });
    }

    pub(super) fn home(&mut self, ui: &mut Ui) -> Option<Action> {
        let mut action = None;
        let rect = ui.max_rect();
        if let Some(ch) = &self.checker {
            ui.painter().image(
                ch.id(),
                rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(rect.width() / 16.0, rect.height() / 16.0)),
                Color32::WHITE,
            );
        }
        if self.recent_dirty {
            self.recent_dirty = false;
            self.recent = recent_files(&self.settings.save_dir, 3);
        }
        let card_w = 560.0f32.min(rect.width() - 40.0);
        let card = Rect::from_center_size(rect.center(), vec2(card_w, (rect.height() - 40.0).min(430.0)));
        ui.scope_builder(egui::UiBuilder::new().max_rect(card), |ui| {
            Frame::new()
                .fill(Color32::from_rgba_unmultiplied(30, 30, 32, 235))
                .stroke(Stroke::new(1.0, Color32::from_gray(64)))
                .corner_radius(10)
                .inner_margin(Margin::same(22))
                .show(ui, |ui| {
                    ui.set_width(card_w - 44.0);
                    ui.label(egui::RichText::new("Capture something").size(18.0).color(TEXT).strong());
                    ui.label(
                        egui::RichText::new(format!(
                            "Screenshots and screen recordings stay on {}. Nothing is uploaded.",
                            platform::THIS_COMPUTER
                        ))
                            .color(DIM_TEXT),
                    );
                    ui.add_space(14.0);
                    let items = [
                        ("Capture Area", self.settings.hotkey_short("area"), Action::Capture(CaptureKind::Area)),
                        ("Capture Full Screen", self.settings.hotkey_short("full"), Action::Capture(CaptureKind::FullScreen)),
                        ("Capture Window", self.settings.hotkey_short("window"), Action::Capture(CaptureKind::Window)),
                        ("Record Video", self.settings.hotkey_short("record"), Action::Record),
                    ];
                    let bw = (card_w - 44.0 - 10.0) / 2.0;
                    egui::Grid::new("home_grid").spacing(vec2(10.0, 10.0)).show(ui, |ui| {
                        for (i, (label, key, a)) in items.iter().enumerate() {
                            if big_button(ui, label, key, bw, !self.busy(), *a == Action::Record).clicked() {
                                action = Some(*a);
                            }
                            if i % 2 == 1 {
                                ui.end_row();
                            }
                        }
                        let timed = format!("Timed Capture ({} s)", self.settings.delay_secs);
                        if big_button(ui, &timed, &self.settings.hotkey_short("timed"), bw, !self.busy(), false).clicked() {
                            action = Some(Action::Capture(CaptureKind::Timed(self.settings.delay_secs)));
                        }
                        if big_button(ui, "Open Picture…", "", bw, true, false).clicked() {
                            action = Some(Action::OpenImage);
                        }
                        ui.end_row();
                    });
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Recent").color(DIM_TEXT).strong());
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.link("Open folder").clicked() {
                                action = Some(Action::OpenFolder);
                            }
                        });
                    });
                    if self.recent.is_empty() {
                        ui.label(egui::RichText::new("Nothing saved yet.").color(DIM_TEXT).italics());
                    }
                    let mut open = None;
                    for p in &self.recent {
                        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
                        if ui.add(egui::Button::new(egui::RichText::new(name).color(TEXT)).frame(false)).clicked() {
                            open = Some(p.clone());
                        }
                    }
                    if let Some(p) = open {
                        let ctx = ui.ctx().clone();
                        self.open_file(&ctx, &p);
                    }
                });
        });
        action
    }

    pub(super) fn video_view(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let Some(v) = &mut self.video else { return };
        let rect = ui.max_rect();
        if let Some(ch) = &self.checker {
            ui.painter().image(
                ch.id(),
                rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(rect.width() / 16.0, rect.height() / 16.0)),
                Color32::WHITE,
            );
        }
        if let Some(f) = v.take_frame() {
            match &mut v.tex {
                Some(t) => t.set(f, TextureOptions::LINEAR),
                None => v.tex = Some(ctx.load_texture("video", f, TextureOptions::LINEAR)),
            }
        }
        if v.tick() {
            ctx.request_repaint_after(Duration::from_millis(33));
        }
        if let Some(t) = &v.tex {
            let size = t.size_vec2();
            let avail = rect.size() - vec2(24.0, 24.0);
            let z = (avail.x / size.x).min(avail.y / size.y).min(1.0 / self.native_ppp.max(1.0) * (v.size[0] as f32 / size.x).max(1.0));
            let r = Rect::from_center_size(rect.center(), size * z);
            ui.painter().image(t.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        } else {
            let msg = if v.can_preview() {
                "Loading preview…"
            } else {
                &format!(
                    "Preview and trimming need ffmpeg ({}). You can still save the video or open it in {}.",
                    platform::FFMPEG_HINT,
                    platform::VIDEO_PLAYER
                )
            };
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, msg, FontId::proportional(15.0), DIM_TEXT);
        }
    }

    pub(super) fn video_bar(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let Some(v) = &mut self.video else { return };
        let can = v.can_preview();
        // Same sizes as the picture editor's tool bar: 26-point controls on one centre line.
        centered_row(ui, TOOL_H, |ui| {
            ui.add_space(14.0);
            let icon = if v.is_playing() { Icon::Pause } else { Icon::Play };
            let tip = if v.is_playing() { "Pause (Space)" } else { "Play (Space)" };
            let resp = group(ui, 1, TOOL_W, TOOL_H, |ui| icon_button(ui, icon, false, can, vec2(TOOL_W, TOOL_H - 2.0), tip)).inner;
            if resp.clicked() || (can && ui.input(|i| i.key_pressed(Key::Space)) && !ctx.egui_wants_keyboard_input()) {
                if v.is_playing() {
                    v.stop();
                } else {
                    v.play(&ctx);
                }
            }
            ui.add_space(12.0);
            let time_w = 110.0;
            let tl_w = (ui.available_width() - time_w - 24.0).max(100.0);
            let (tl, tl_resp) = ui.allocate_exact_size(vec2(tl_w, TOOL_H), Sense::click_and_drag());
            let p = ui.painter();
            let x0 = tl.min.x + 8.0;
            let x1 = tl.max.x - 8.0;
            let dur = v.duration.max(0.001);
            let to_x = |t: f32| x0 + (x1 - x0) * (t / dur).clamp(0.0, 1.0);
            let to_t = |x: f32| ((x - x0) / (x1 - x0)).clamp(0.0, 1.0) * dur;
            let cy = tl.center().y;
            p.rect_filled(Rect::from_min_max(pos2(x0, cy - 2.0), pos2(x1, cy + 2.0)), 2.0, Color32::from_gray(80));
            p.rect_filled(
                Rect::from_min_max(pos2(to_x(v.trim.0), cy - 2.0), pos2(to_x(v.trim.1), cy + 2.0)),
                2.0,
                Color32::from_gray(185),
            );
            // Trim handles
            let handle = |t: f32| Rect::from_center_size(pos2(to_x(t), cy), vec2(7.0, TOOL_H - 4.0));
            for t in [v.trim.0, v.trim.1] {
                let h = handle(t);
                p.rect_filled(h, 2.0, Color32::from_gray(205));
                p.rect_stroke(h, 2.0, Stroke::new(1.0, Color32::from_gray(90)), StrokeKind::Inside);
                p.line_segment([h.center_top() + vec2(0.0, 6.0), h.center_bottom() - vec2(0.0, 6.0)], Stroke::new(1.0, Color32::from_gray(110)));
            }
            // Playhead
            let ph = pos2(to_x(v.pos), cy);
            p.circle_filled(ph, 8.0, Color32::WHITE);
            p.circle_filled(ph, 4.5, RED);

            let drag_id = Id::new("tl_drag");
            if can {
                if tl_resp.drag_started() {
                    let px = ui.input(|i| i.pointer.press_origin()).map(|q| q.x).unwrap_or(0.0);
                    let which = if (px - to_x(v.trim.0)).abs() < 10.0 {
                        1u8
                    } else if (px - to_x(v.trim.1)).abs() < 10.0 {
                        2
                    } else {
                        0
                    };
                    ui.data_mut(|d| d.insert_temp(drag_id, which));
                }
                if tl_resp.dragged() || tl_resp.clicked() {
                    if let Some(q) = tl_resp.interact_pointer_pos() {
                        let which: u8 = if tl_resp.clicked() { 0 } else { ui.data(|d| d.get_temp(drag_id).unwrap_or(0)) };
                        let t = to_t(q.x);
                        match which {
                            1 => {
                                v.trim.0 = t.min(v.trim.1 - 0.2).max(0.0);
                                v.seek(&ctx, v.trim.0);
                            }
                            2 => {
                                v.trim.1 = t.max(v.trim.0 + 0.2).min(dur);
                                v.seek(&ctx, v.trim.1);
                            }
                            _ => {
                                let t = t.clamp(v.trim.0, v.trim.1);
                                v.seek(&ctx, t);
                            }
                        }
                    }
                }
                tl_resp.on_hover_text("Drag the grey handles to choose where the video starts and ends");
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(20.0);
                let label = if v.is_trimmed() {
                    format!(
                        "{} / {}",
                        record::format_elapsed(Duration::from_secs_f32((v.pos - v.trim.0).max(0.0))),
                        record::format_elapsed(Duration::from_secs_f32(v.trim.1 - v.trim.0))
                    )
                } else {
                    format!(
                        "{} / {}",
                        record::format_elapsed(Duration::from_secs_f32(v.pos)),
                        record::format_elapsed(Duration::from_secs_f32(v.duration))
                    )
                };
                ui.label(egui::RichText::new(label).size(BAR_FONT).color(TEXT));
            });
        });
    }

    pub(super) fn resize_window(&mut self, ctx: &egui::Context) {
        let font = self.font.clone();
        let Some(ed) = &mut self.ed else { return };
        let Some(dlg) = &mut ed.resize else { return };
        let (ow, oh) = (ed.doc.base.width() as f32, ed.doc.base.height() as f32);
        let mut apply = None;
        let mut close = false;
        let resp = egui::Modal::new(Id::new("resize")).show(ctx, |ui| {
            ui.heading("Resize picture");
            ui.add_space(6.0);
            egui::Grid::new("rs").num_columns(2).show(ui, |ui| {
                ui.label("Width");
                if ui.add(egui::DragValue::new(&mut dlg.w).range(1..=20000).suffix(" px")).changed() && dlg.keep {
                    dlg.h = ((dlg.w as f32) * oh / ow).round().max(1.0) as u32;
                }
                ui.end_row();
                ui.label("Height");
                if ui.add(egui::DragValue::new(&mut dlg.h).range(1..=20000).suffix(" px")).changed() && dlg.keep {
                    dlg.w = ((dlg.h as f32) * ow / oh).round().max(1.0) as u32;
                }
                ui.end_row();
            });
            ui.checkbox(&mut dlg.keep, "Keep proportions");
            ui.horizontal(|ui| {
                for pct in [25u32, 50, 75] {
                    if ui.button(format!("{pct}%")).clicked() {
                        dlg.w = (ow * pct as f32 / 100.0).round() as u32;
                        dlg.h = (oh * pct as f32 / 100.0).round() as u32;
                    }
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Resize").clicked() {
                    apply = Some((dlg.w, dlg.h));
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });
        if let Some((w, h)) = apply {
            ed.resize = None;
            ed.apply_resize(&font, w, h);
        } else if close || resp.should_close() {
            ed.resize = None;
        }
    }

    pub(super) fn editor_shortcuts(&mut self, ctx: &egui::Context) {
        if self.ed.is_none() {
            return;
        }
        let editing_text = self.ed.as_ref().is_some_and(|e| e.is_editing_text());
        let typing = ctx.egui_wants_keyboard_input() || editing_text;
        let (undo, redo, save, copy, del, esc) = ctx.input_mut(|i| {
            let redo = i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
            let undo = i.consume_key(Modifiers::COMMAND, Key::Z);
            let save = i.consume_key(Modifiers::COMMAND, Key::S);
            let copy = !typing && i.events.iter().any(|e| matches!(e, egui::Event::Copy));
            let del = !typing && (i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace));
            let esc = !typing && i.key_pressed(Key::Escape);
            (undo, redo, save, copy, del, esc)
        });
        if !typing {
            let keys = [
                (Key::A, Tool::Arrow),
                (Key::R, Tool::Rect),
                (Key::L, Tool::Line),
                (Key::O, Tool::Ellipse),
                (Key::P, Tool::Pen),
                (Key::H, Tool::Highlight),
                (Key::T, Tool::Text),
                (Key::C, Tool::Callout),
                (Key::N, Tool::Step),
                (Key::B, Tool::Blur),
                (Key::X, Tool::Crop),
            ];
            for (k, t) in keys {
                if ctx.input(|i| i.key_pressed(k) && i.modifiers.is_none()) {
                    self.tool = t;
                }
            }
        }
        let (fit, zin, zout) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::COMMAND, Key::Num0),
                i.consume_key(Modifiers::COMMAND, Key::Plus) || i.consume_key(Modifiers::COMMAND, Key::Equals),
                i.consume_key(Modifiers::COMMAND, Key::Minus),
            )
        });
        if let Some(e) = &mut self.ed {
            if fit {
                e.reset_view();
            }
            if zin {
                e.zoom_by(1.25);
            }
            if zout {
                e.zoom_by(0.8);
            }
        }
        if save {
            self.save_picture(false);
        }
        if copy {
            self.copy_picture();
        }
        let Some(ed) = &mut self.ed else { return };
        if undo && !typing {
            ed.undo();
        }
        if redo && !typing {
            ed.redo();
        }
        if del {
            ed.delete_selected();
        }
        if esc && ed.crop.is_none() {
            ed.selected = None;
        }
    }
}
