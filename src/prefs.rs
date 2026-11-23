//! The settings window: its own window with General, Screenshots, Video and About tabs.
//! Cloud-only options of the original (upload, links, account) are left out on purpose.

use crate::icons::{self, Icon};
use crate::settings::{
    AfterCapture, ClipFormat, HOTKEY_ACTIONS, Hotkey, ImageFormat, MAX_EXTRA_DIRS, Settings, VIDEO_HOTKEY_ACTIONS,
    VideoFormat, dir_name, folder_action, short_label,
};
use egui::{
    Align, Align2, Color32, FontId, Key, Layout, Rect, Sense, Stroke, Ui, Vec2, ViewportBuilder, ViewportCommand,
    ViewportId, pos2, vec2,
};

const BG: Color32 = Color32::from_rgb(34, 34, 36);
const HEADER: Color32 = Color32::from_rgb(42, 42, 44);
const TAB_SEL: Color32 = Color32::from_rgb(58, 58, 60);
const ACCENT: Color32 = Color32::from_rgb(70, 140, 255);
const TEXT: Color32 = Color32::from_rgb(232, 232, 236);
const DIM: Color32 = Color32::from_rgb(140, 140, 146);
const KEY_BTN: Color32 = Color32::from_rgb(88, 88, 92);
// Sizes measured from the original's settings screenshots (Retina pixels ÷ 2).
const WIN_W: f32 = 426.0;
const LABEL_W: f32 = 140.0;
const KEY_LABEL_W: f32 = 140.0;
const ROW_H: f32 = 24.0;
/// Every dropdown, slider and text field has this width, so their right edges line up.
const CTRL_W: f32 = 236.0;
const HEADER_H: f32 = 66.0;
const FONT: f32 = 13.0;
/// The tab body scrolls when it is taller than this, so the window stays at most about
/// 700 px high and fits a 1366×768 screen.
const MAX_BODY_H: f32 = 600.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    General,
    Screenshots,
    Video,
    About,
}

impl Tab {
    fn name(self) -> &'static str {
        match self {
            Tab::General => "General",
            Tab::Screenshots => "Screenshots",
            Tab::Video => "Video",
            Tab::About => "About",
        }
    }
    fn icon(self) -> Icon {
        match self {
            Tab::General => Icon::Gear,
            Tab::Screenshots => Icon::CameraPlus,
            Tab::Video => Icon::Video,
            Tab::About => Icon::Info,
        }
    }
    fn size(self) -> Vec2 {
        match self {
            Tab::General => vec2(WIN_W, 420.0),
            Tab::Screenshots => vec2(WIN_W, 640.0),
            // One more line when the encoder note shows (Linux without x264).
            Tab::Video => vec2(WIN_W, if crate::platform::encoder_note().is_some() { 424.0 } else { 400.0 }),
            Tab::About => vec2(WIN_W, 400.0),
        }
    }
}

pub struct Prefs {
    pub open: bool,
    pub tab: Tab,
    /// The shortcut being recorded right now.
    pub capturing: Option<String>,
    sized_for: Option<Tab>,
    /// Frames left to ask for the focus. The window is still hidden in its first frame,
    /// and another app may be in front, so it asks again until it has it.
    focus_tries: u8,
    /// Height of the current tab's contents, to fit the window to it.
    content_h: f32,
    /// The desktop can open its own page for changing the shortcuts (Linux).
    pub can_configure_keys: bool,
    /// Frames to wait before the window is made (Linux, see `show`).
    wait: u32,
    /// Check pictures: the height the frame inside the main window loses to its title bar.
    embed_extra: f32,
    /// Frames left to check that the window got the height it was sent.
    size_checks: u32,
    /// The app icon on the About tab, loaded when first shown.
    about_icon: Option<egui::TextureHandle>,
    /// macOS: the permissions as last read, and whether the window had the focus then.
    perms: Option<(bool, crate::platform::MicStatus)>,
    was_focused: bool,
}

#[derive(Default)]
pub struct PrefsOut {
    pub changed: bool,
    pub hotkeys_changed: bool,
    /// Global shortcuts must be paused while a new one is recorded.
    pub capture_started: bool,
    pub capture_ended: bool,
    pub save_dir_changed: bool,
    /// Ask the app to show a folder picker for the save folder.
    pub pick_save_dir: bool,
    /// Ask the app to show a folder picker for a new extra folder.
    pub pick_extra_dir: bool,
    /// Ask the app to show an app picker for the external editor.
    pub pick_editor_app: bool,
    /// Ask the desktop to show its page for changing the shortcuts (Linux).
    pub configure_keys: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            open: false,
            tab: Tab::General,
            capturing: None,
            sized_for: None,
            focus_tries: 0,
            content_h: 0.0,
            can_configure_keys: false,
            wait: 0,
            embed_extra: 0.0,
            size_checks: 0,
            about_icon: None,
            perms: None,
            was_focused: false,
        }
    }
}

/// How many frames (about 30 ms each) the Preferences window keeps itself in front.
const FOCUS_FRAMES: u8 = 20;

impl Prefs {
    pub fn open(&mut self) {
        // Wayland ignores "bring to front" for a window that is already open, so on
        // Linux it is made again (see `show`).
        if cfg!(target_os = "linux") {
            self.wait = if self.open { 1 } else { 30 };
        }
        self.open = true;
        self.focus_tries = FOCUS_FRAMES;
    }

    pub fn show(&mut self, ctx: &egui::Context, s: &mut Settings) -> PrefsOut {
        let mut out = PrefsOut::default();
        if !self.open {
            return out;
        }
        // Linux: GNOME puts a new window in front only while DyoSnap has the focus. So
        // the window waits until the main window has it (or a moment has passed).
        if self.wait > 0 {
            let main_focused = ctx.input(|i| i.viewport().focused) == Some(true);
            self.wait = if main_focused { self.wait.min(1) - 1 } else { self.wait - 1 };
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
            return out;
        }
        let size = self.tab.size();
        let mut builder = ViewportBuilder::default()
            .with_app_id(crate::platform::APP_ID)
            .with_title(format!("DyoSnap | {}", self.tab.name()))
            // The tab's last fitted height, or its usual one: the window then rarely has to
            // grow after it opens.
            .with_inner_size(vec2(size.x, if self.content_h > 0.0 { self.content_h } else { size.y }))
            .with_resizable(false)
            .with_minimize_button(false)
            .with_maximize_button(false);
        if ctx.embed_viewports() {
            // Check pictures draw the settings inside the main window. Give them the
            // size the real window gets.
            // The frame's own title bar takes `embed_extra` of that height.
            let h = if self.content_h > 0.0 { self.content_h } else { 420.0 };
            let fixed = vec2(size.x, h + self.embed_extra);
            builder = builder.with_min_inner_size(fixed).with_max_inner_size(fixed);
        }
        let mut close = false;
        crate::show_immediate(ctx, ViewportId::from_hash_of("dyosnap_prefs"), builder, |ui, _| {
            close = self.draw(ui, s, &mut out);
        });
        if close {
            self.close(&mut out);
        }
        out
    }

    /// Linux: the size and title of a window that only shows Preferences: the width of the
    /// tab and the last fitted height.
    #[cfg(target_os = "linux")]
    pub fn window_size(&self) -> Vec2 {
        vec2(self.tab.size().x, if self.content_h > 0.0 { self.content_h } else { 420.0 })
    }

    #[cfg(target_os = "linux")]
    pub fn window_title(&self) -> String {
        format!("DyoSnap | {}", self.tab.name())
    }

    /// Linux: no waiting for the main window's focus (see `show`), there is no such window.
    #[cfg(target_os = "linux")]
    pub fn skip_wait(&mut self) {
        self.wait = 0;
    }

    /// Linux: Preferences moves from the main window into its own window. Fit the new
    /// window's height again, as on a first open.
    #[cfg(target_os = "linux")]
    pub fn refit(&mut self) {
        self.sized_for = None;
    }

    /// Closes Preferences.
    pub fn close(&mut self, out: &mut PrefsOut) {
        self.open = false;
        self.sized_for = None;
        self.perms = None;
        self.cancel_capture(out);
        out.changed = true;
    }

    /// Draws Preferences into `ui`: the child window's contents, or on Linux the root
    /// window's (Preferences-only mode). Returns true when it should close.
    pub fn draw(&mut self, ui: &mut Ui, s: &mut Settings, out: &mut PrefsOut) -> bool {
        let mut close = false;
        let size = self.tab.size();
        let ctx = ui.ctx().clone();
        // Keep the window in front for a moment: when it was opened from the menu-bar
        // menu, macOS hands the focus back to the app that was in front when the menu
        // closes, which can be a few frames later.
        if self.focus_tries > 0 {
            self.focus_tries -= 1;
            let focused = ctx.input(|i| i.viewport().focused) == Some(true);
            if !focused {
                ctx.send_viewport_cmd(ViewportCommand::Focus);
                // From the second frame on the window exists and can be put in front
                // of other apps' windows too.
                if self.focus_tries < FOCUS_FRAMES - 1 {
                    crate::platform::raise_window("DyoSnap | ");
                }
                if self.focus_tries == 0 {
                    log::info!("preferences: could not take the focus");
                }
            }
            // The frames must go on while the main window is covered by another app.
            ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::RequestPaintWhileHidden);
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
        // Back from System Settings: read the permissions again.
        let focused = ctx.input(|i| i.viewport().focused) == Some(true);
        if focused && !self.was_focused {
            self.perms = None;
        }
        self.was_focused = focused;
        if self.sized_for != Some(self.tab) {
            self.sized_for = Some(self.tab);
            self.content_h = 0.0;
            ctx.send_viewport_cmd(ViewportCommand::Title(format!("DyoSnap | {}", self.tab.name())));
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            close = true;
        }
        if self.capturing.is_none() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::W)) {
            close = true;
        }
        let full = ui.max_rect();
        if ctx.embed_viewports() {
            let h = if self.content_h > 0.0 { self.content_h } else { 420.0 };
            let extra = h + self.embed_extra - full.height();
            if (extra - self.embed_extra).abs() > 0.5 {
                self.embed_extra = extra;
                ctx.request_repaint();
            }
        }
        ui.painter().rect_filled(full, 0.0, BG);

        // Tab bar
        let header = Rect::from_min_size(full.min, vec2(full.width(), HEADER_H));
        ui.painter().rect_filled(header, 0.0, HEADER);
        ui.painter().line_segment(
            [header.left_bottom(), header.right_bottom()],
            Stroke::new(1.0, Color32::from_gray(24)),
        );
        let tabs = [Tab::General, Tab::Screenshots, Tab::Video, Tab::About];
        let tw = 66.0;
        let x0 = header.center().x - tw * tabs.len() as f32 / 2.0;
        for (i, t) in tabs.iter().enumerate() {
            let r = Rect::from_min_size(pos2(x0 + i as f32 * tw + 3.0, header.min.y + 6.0), vec2(tw - 6.0, 54.0));
            let resp = ui.interact(r, egui::Id::new(("tab", i)), Sense::click());
            let sel = *t == self.tab;
            if sel {
                ui.painter().rect_filled(r, 6.0, TAB_SEL);
            }
            let col = if sel { ACCENT } else if resp.hovered() { TEXT } else { DIM };
            icons::draw(
                ui.painter(),
                Rect::from_center_size(pos2(r.center().x, r.min.y + 20.0), vec2(27.0, 27.0)),
                t.icon(),
                if sel { Color32::from_gray(200) } else { DIM },
            );
            ui.painter().text(
                pos2(r.center().x, r.max.y - 9.0),
                Align2::CENTER_CENTER,
                t.name(),
                FontId::proportional(12.0),
                col,
            );
            if resp.clicked() && !sel {
                self.tab = *t;
                self.cancel_capture(out);
            }
        }

        let body = Rect::from_min_max(pos2(full.min.x + 12.0, header.max.y + 12.0), full.max - vec2(12.0, 10.0));
        let used = ui.scope_builder(egui::UiBuilder::new().max_rect(body).layout(Layout::top_down(Align::Min)), |ui| {
            {
                let st = ui.style_mut();
                st.spacing.item_spacing = vec2(6.0, 5.0);
                st.spacing.interact_size.y = 22.0;
                st.spacing.button_padding = vec2(8.0, 2.0);
                st.spacing.icon_width = 15.0;
                st.spacing.icon_width_inner = 9.0;
                for (ts, f) in st.text_styles.iter_mut() {
                    f.size = match ts {
                        egui::TextStyle::Small => 11.0,
                        egui::TextStyle::Heading => 15.0,
                        _ => FONT,
                    };
                }
            }
            // The scroll area can only be as tall as the window is now, so its own height says
            // nothing about how tall the tab wants to be. Its contents do: fit to them at once.
            let mut scroll_h = None;
            match self.tab {
                Tab::General => self.general(ui, s, out),
                Tab::Screenshots => {
                    // Taller than the screen allows once there are folders: scroll. The bar is
                    // always shown, so it is clear that there is more.
                    ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
                    let mut area = egui::ScrollArea::vertical().max_height(MAX_BODY_H).auto_shrink([false, true]);
                    // Check pictures: LS_PREFS_SCROLL=<points> scrolls the body down.
                    if let (true, Some(off)) = (cfg!(debug_assertions), std::env::var("LS_PREFS_SCROLL").ok()) {
                        area = area.vertical_scroll_offset(off.parse().unwrap_or(0.0));
                    }
                    let shown = area.show(ui, |ui| self.screenshots(ui, s, out));
                    // Rows scrolled under an edge fade out, instead of showing half a row.
                    let view = shown.inner_rect;
                    if shown.state.offset().y > 0.0 {
                        fade_edge(ui.painter(), view, true);
                    }
                    if shown.state.offset().y + view.height() < shown.content_size.y - 1.0 {
                        fade_edge(ui.painter(), view, false);
                    }
                    scroll_h = Some(shown.content_size.y.min(MAX_BODY_H));
                }
                Tab::Video => self.video(ui, s, out),
                Tab::About => crate::about::draw(ui, &mut self.about_icon),
            }
            scroll_h.unwrap_or_else(|| ui.min_rect().height())
        });
        // Fit the window height to the tab's contents.
        let want = HEADER_H + 12.0 + used.inner + 16.0;
        if (want - self.content_h).abs() > 1.0 {
            self.content_h = want;
            if ctx.embed_viewports() {
                // Check pictures: the command would resize the main window.
                ctx.request_repaint();
            } else {
                self.size_checks = 20;
                ctx.send_viewport_cmd(ViewportCommand::InnerSize(vec2(size.x, want)));
            }
        }
        // A new window may not exist yet when the size is sent (Wayland), and then the
        // request is lost. So send it again until the window has the height. Wayland
        // never reports the size, so there it is sent for the first 20 frames.
        if self.size_checks > 0 && !ctx.embed_viewports() {
            match ctx.input(|i| i.viewport().inner_rect) {
                Some(r) if (r.height() - want).abs() <= 1.0 => self.size_checks = 0,
                _ => {
                    self.size_checks -= 1;
                    ctx.send_viewport_cmd(ViewportCommand::InnerSize(vec2(size.x, want)));
                    ctx.request_repaint_after(std::time::Duration::from_millis(50));
                }
            }
        }
        close
    }

    fn cancel_capture(&mut self, out: &mut PrefsOut) {
        if self.capturing.take().is_some() {
            out.capture_ended = true;
        }
    }

    // ------------------------------------------------------------ General

    fn general(&mut self, ui: &mut Ui, s: &mut Settings, out: &mut PrefsOut) {
        row(ui, "", |ui| {
            let mut on = crate::platform::login_item_enabled();
            if ui.checkbox(&mut on, "Launch at login").changed() {
                crate::platform::set_login_item(on);
            }
        });
        row(ui, "", |ui| {
            out.changed |= ui.checkbox(&mut s.sound, "Sound effects").changed();
        });
        separator(ui);
        row(ui, "Filename template:", |ui| {
            out.changed |= ui
                .add(egui::TextEdit::singleline(&mut s.name_template).desired_width(CTRL_W))
                .changed();
        });
        row(ui, "", |ui| {
            out.changed |= ui.checkbox(&mut s.apply_template_reopened, "Apply to reopened files").changed();
        });
        ui.vertical_centered(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            for line in [
                "%Y — 4-digit year, %y — 2-digit year,",
                "%m — month, %d — day, %H — hour,",
                "%M — minutes, %S — seconds,",
                "%R — 5 random symbols, %T — file type",
            ] {
                ui.label(egui::RichText::new(line).color(DIM).size(12.0));
            }
        });
        separator(ui);
        if cfg!(target_os = "macos") {
            self.permissions(ui);
            separator(ui);
        }
        ui.vertical_centered(|ui| {
            if ui.add(egui::Button::new("Reset all settings").min_size(vec2(190.0, 24.0))).clicked() {
                *s = Settings::default();
                out.changed = true;
                out.hotkeys_changed = true;
                out.save_dir_changed = true;
            }
            ui.label(egui::RichText::new("ALL settings in the application will be reset").color(DIM));
        });
    }

    /// macOS: what DyoSnap may do, read from the system. The status is read again when
    /// the window gets the focus back (the person comes from System Settings), not every frame.
    fn permissions(&mut self, ui: &mut Ui) {
        use crate::platform::{MicStatus, Pane};
        let (screen, mic) = *self
            .perms
            .get_or_insert_with(|| (crate::platform::screen_recording_allowed(), crate::platform::mic_status()));
        ui.vertical_centered(|ui| ui.label(egui::RichText::new("Permissions").color(TEXT)));
        let ok = Color32::from_rgb(90, 200, 120);
        let bad = Color32::from_rgb(235, 170, 80);
        let rows = [
            ("Screen Recording:", Pane::ScreenRecording, screen, if screen { "Allowed" } else { "Not allowed" }),
            (
                "Microphone:",
                Pane::Microphone,
                mic == MicStatus::Allowed,
                match mic {
                    MicStatus::Allowed => "Allowed",
                    MicStatus::Denied => "Not allowed",
                    MicStatus::NotAsked => "Not asked yet",
                },
            ),
        ];
        for (label, pane, allowed, text) in rows {
            row(ui, label, |ui| {
                ui.allocate_ui_with_layout(vec2(CTRL_W, ROW_H), Layout::right_to_left(Align::Center), |ui| {
                    ui.set_width(CTRL_W);
                    if ui.button("Open System Settings").clicked() {
                        crate::platform::open_privacy(pane);
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.label(egui::RichText::new(text).color(if allowed { ok } else { bad }));
                    });
                });
            });
        }
    }

    // ------------------------------------------------------------ Screenshots

    fn screenshots(&mut self, ui: &mut Ui, s: &mut Settings, out: &mut PrefsOut) {
        row(ui, "After screenshot:", |ui| {
            egui::ComboBox::from_id_salt("after").width(CTRL_W).selected_text(s.after_capture.label()).show_ui(ui, |ui| {
                for a in [AfterCapture::OpenEditor, AfterCapture::SaveAndCopy, AfterCapture::CopyOnly] {
                    out.changed |= ui.selectable_value(&mut s.after_capture, a, a.label()).changed();
                }
            });
        });
        row(ui, "Timed capture after:", |ui| {
            ui.spacing_mut().slider_width = CTRL_W - 46.0;
            out.changed |= ui.add(egui::Slider::new(&mut s.delay_secs, 1..=30).suffix(" s")).changed();
        });
        separator(ui);
        row(ui, "Image File Format:", |ui| {
            let label = match s.format {
                ImageFormat::Png => "PNG",
                ImageFormat::Jpg => "JPEG",
            };
            egui::ComboBox::from_id_salt("img_fmt").width(CTRL_W).selected_text(label).show_ui(ui, |ui| {
                out.changed |= ui.selectable_value(&mut s.format, ImageFormat::Png, "PNG").changed();
                out.changed |= ui.selectable_value(&mut s.format, ImageFormat::Jpg, "JPEG").changed();
            });
        });
        row(ui, &format!("JPEG Quality: {}%", s.jpeg_quality), |ui| {
            ui.spacing_mut().slider_width = CTRL_W;
            out.changed |= ui.add(egui::Slider::new(&mut s.jpeg_quality, 10..=100).show_value(false)).changed();
        });
        row(ui, "Save folder:", |ui| {
            // Folder and "Choose…" share the control column; the button ends where the dropdowns end.
            ui.allocate_ui_with_layout(vec2(CTRL_W, ROW_H), Layout::right_to_left(Align::Center), |ui| {
                ui.set_width(CTRL_W);
                if ui.button("Choose…").clicked() {
                    out.pick_save_dir = true;
                }
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                    icons::draw(ui.painter(), r, Icon::Folder, Color32::from_rgb(110, 170, 240));
                    ui.add(egui::Label::new(egui::RichText::new(short_path(&s.save_dir)).color(TEXT)).truncate())
                        .on_hover_text(s.save_dir.display().to_string());
                });
            });
        });
        self.extra_dirs(ui, s, out);
        row(ui, "", |ui| {
            out.changed |= ui.checkbox(&mut s.autosave, "Autosave screenshots").changed();
        });
        row(ui, "", |ui| {
            out.changed |= ui.checkbox(&mut s.shrink_retina, "Shrink Retina snaps").changed();
        });
        separator(ui);
        row(ui, "Copy to clipboard:", |ui| {
            out.changed |= ui.checkbox(&mut s.copy_path, "Path").changed();
            ui.add_space(16.0);
            out.changed |= ui.checkbox(&mut s.copy_image, "Image").changed();
            // Linux copies PNG only: TIFF is a macOS clipboard format.
            if cfg!(target_os = "linux") {
                return;
            }
            ui.add_space(16.0);
            let label = match s.clip_format {
                ClipFormat::Png => "PNG",
                ClipFormat::Tiff => "TIFF",
            };
            egui::ComboBox::from_id_salt("clip_fmt").width(70.0).selected_text(label).show_ui(ui, |ui| {
                out.changed |= ui.selectable_value(&mut s.clip_format, ClipFormat::Png, "PNG").changed();
                out.changed |= ui.selectable_value(&mut s.clip_format, ClipFormat::Tiff, "TIFF").changed();
            });
        });
        row(ui, "External Image Editor:", |ui| {
            let name = crate::platform::editor_label(&s.external_editor);
            egui::ComboBox::from_id_salt("ext_editor").width(CTRL_W).selected_text(name).show_ui(ui, |ui| {
                for (value, label) in crate::platform::EDITOR_APPS {
                    out.changed |= ui.selectable_value(&mut s.external_editor, value.to_string(), *label).changed();
                }
                if !crate::platform::EDITOR_APPS.iter().any(|(v, _)| *v == s.external_editor) {
                    let cur = s.external_editor.clone();
                    let _ = ui.selectable_label(true, crate::platform::editor_label(&cur));
                }
                if ui.selectable_label(false, "Choose…").clicked() {
                    out.pick_editor_app = true;
                }
            });
        });
        separator(ui);
        let actions = screenshot_actions(s);
        self.shortcuts(ui, s, &actions, out);
    }

    /// The extra save folders under the "Save folder" row, and the button to add one.
    fn extra_dirs(&mut self, ui: &mut Ui, s: &mut Settings, out: &mut PrefsOut) {
        let mut remove = None;
        for dir in s.extra_dirs.iter() {
            row(ui, "", |ui| {
                ui.allocate_ui_with_layout(vec2(CTRL_W, ROW_H), Layout::right_to_left(Align::Center), |ui| {
                    ui.set_width(CTRL_W);
                    if x_button(ui, "Remove this folder") {
                        remove = Some(dir.id);
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                        icons::draw(ui.painter(), r, Icon::Folder, Color32::from_rgb(110, 170, 240));
                        ui.add(egui::Label::new(egui::RichText::new(short_path(&dir.path)).color(TEXT)).truncate())
                            .on_hover_text(dir.path.display().to_string());
                    });
                });
            });
        }
        if let Some(id) = remove {
            if self.capturing.as_deref() == Some(folder_action(id).as_str()) {
                self.cancel_capture(out);
            }
            s.remove_extra_dir(id);
            out.changed = true;
            out.hotkeys_changed = true;
        }
        row(ui, "", |ui| {
            if ui.add_enabled(s.extra_dirs.len() < MAX_EXTRA_DIRS, egui::Button::new("Add folder")).clicked() {
                out.pick_extra_dir = true;
            }
            ui.label(egui::RichText::new("Optional, up to 9").color(DIM));
        });
    }

    // ------------------------------------------------------------ Video

    fn video(&mut self, ui: &mut Ui, s: &mut Settings, out: &mut PrefsOut) {
        row(ui, "Video File Format:", |ui| {
            egui::ComboBox::from_id_salt("vid_fmt").width(CTRL_W).selected_text(s.video_format.label()).show_ui(ui, |ui| {
                for f in [VideoFormat::Mp4, VideoFormat::Mov, VideoFormat::Gif] {
                    out.changed |= ui.selectable_value(&mut s.video_format, f, f.label()).changed();
                }
            });
        });
        if let Some(note) = crate::platform::encoder_note() {
            row(ui, "", |ui| {
                ui.label(egui::RichText::new(note).small().color(DIM));
            });
        }
        row(ui, &format!("GIF FPS: {}", s.gif_fps), |ui| {
            ui.spacing_mut().slider_width = CTRL_W;
            out.changed |= ui.add(egui::Slider::new(&mut s.gif_fps, 1..=30).show_value(false)).changed();
        });
        row(ui, &format!("GIF Quality: {}%", s.gif_quality), |ui| {
            ui.spacing_mut().slider_width = CTRL_W;
            out.changed |= ui.add(egui::Slider::new(&mut s.gif_quality, 10..=100).show_value(false)).changed();
        });
        row(ui, "", |ui| {
            out.changed |= ui.checkbox(&mut s.autosave_movies, "Autosave to \"Movies\" folder").changed();
        });
        separator(ui);
        self.shortcuts(ui, s, &video_actions(), out);
    }

    // ------------------------------------------------------------ Shortcuts

    /// The "Shortcuts" block of a tab: a key row for each of its actions.
    fn shortcuts(&mut self, ui: &mut Ui, s: &mut Settings, actions: &[(String, String)], out: &mut PrefsOut) {
        ui.vertical_centered(|ui| ui.label(egui::RichText::new("Shortcuts").color(TEXT)));
        // Linux: the desktop owns global shortcuts. Show what it has; changes happen there.
        if let Some(live) = s.live_keys.clone() {
            for (id, label) in actions {
                let label = if id == "timed" { format!("Fullscreen after {}s:", s.delay_secs) } else { format!("{label}:") };
                row_full(ui, &label, folder_path(s, id).as_deref(), KEY_LABEL_W, 10.0, |ui| {
                    let text = live.get(id).filter(|t| !t.is_empty()).cloned().unwrap_or_else(|| "Not set".into());
                    let (r, _) = ui.allocate_exact_size(vec2(CTRL_W - 40.0, 27.0), Sense::hover());
                    ui.painter().rect_filled(r, 3.0, KEY_BTN);
                    ui.painter().text(r.center(), Align2::CENTER_CENTER, text, FontId::proportional(FONT), TEXT);
                });
            }
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(
                    "Your desktop manages global shortcuts. Change them in GNOME Settings › Apps › DyoSnap or KDE System Settings › Shortcuts.",
                )
                .color(DIM),
            );
            if self.can_configure_keys && ui.button("Change Shortcuts…").clicked() {
                out.configure_keys = true;
            }
            return;
        }
        self.record_key(ui, s, out);
        for (id, label) in actions {
            let label = if id == "timed" { format!("Fullscreen after {}s:", s.delay_secs) } else { format!("{label}:") };
            let hover = folder_path(s, id);
            self.key_row(ui, s, id, &label, hover.as_deref(), out);
        }
        ui.add_space(4.0);
        ui.label(egui::RichText::new("Click a field, then press the new shortcut. Esc cancels.").color(DIM));
    }

    /// Records a new shortcut from the next key press.
    fn record_key(&mut self, ui: &mut Ui, s: &mut Settings, out: &mut PrefsOut) {
        let Some(id) = self.capturing.clone() else { return };
        let pressed = ui.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Key { key, pressed: true, modifiers, .. } => Some((*key, *modifiers)),
                _ => None,
            })
        });
        if let Some((key, m)) = pressed {
            if key == Key::Escape && !m.any() {
                self.cancel_capture(out);
            } else if let Some(code) = key_to_code(key) {
                let is_fkey = code.starts_with('F') && code.len() > 1 && code[1..].chars().all(|c| c.is_ascii_digit());
                if m.mac_cmd || m.command || m.alt || m.ctrl || is_fkey {
                    // On Linux `command` is Ctrl; `cmd` stands for it, so `ctrl` stays off.
                    let ctrl = m.ctrl && cfg!(target_os = "macos");
                    let hk = Hotkey { cmd: m.mac_cmd || m.command, alt: m.alt, ctrl, shift: m.shift, code };
                    // One shortcut, one action: across the actions of every tab.
                    let others: Vec<String> = screenshot_actions(s)
                        .into_iter()
                        .chain(video_actions())
                        .map(|(i, _)| i)
                        .filter(|i| *i != id && s.hotkey(i).as_ref() == Some(&hk))
                        .collect();
                    for o in others {
                        s.keys.insert(o, None);
                    }
                    s.keys.insert(id.clone(), Some(hk));
                    out.changed = true;
                    out.hotkeys_changed = true;
                    self.cancel_capture(out);
                }
            }
        }
        ui.ctx().request_repaint();
    }

    fn key_row(&mut self, ui: &mut Ui, s: &mut Settings, id: &str, label: &str, hover: Option<&str>, out: &mut PrefsOut) {
        row_full(ui, label, hover, KEY_LABEL_W, 10.0, |ui| {
            let capturing = self.capturing.as_deref() == Some(id);
            let text = if capturing {
                "Press keys…".to_string()
            } else {
                s.hotkey(id).map(|k| k.label()).unwrap_or_else(|| "Not set".into())
            };
            let (r, resp) = ui.allocate_exact_size(vec2(CTRL_W - 40.0, 27.0), Sense::click());
            let fill = if capturing {
                Color32::from_rgb(40, 76, 186)
            } else if resp.hovered() {
                Color32::from_gray(100)
            } else {
                KEY_BTN
            };
            ui.painter().rect_filled(r, 3.0, fill);
            ui.painter().rect_stroke(r, 3.0, Stroke::new(1.0, Color32::from_gray(30)), egui::StrokeKind::Inside);
            ui.painter().text(r.center(), Align2::CENTER_CENTER, text, FontId::proportional(FONT), TEXT);
            if resp.clicked() {
                if !capturing {
                    let was = self.capturing.is_some();
                    self.capturing = Some(id.to_string());
                    if !was {
                        out.capture_started = true;
                    }
                } else {
                    self.cancel_capture(out);
                }
            }
            ui.add_space(14.0);
            if x_button(ui, "Clear this shortcut") {
                s.keys.insert(id.to_string(), None);
                out.changed = true;
                out.hotkeys_changed = true;
                if capturing {
                    self.cancel_capture(out);
                }
            }
        });
    }
}

/// The actions of the Screenshots tab's shortcuts, as (id, label): the fixed ones, then
/// one "To <folder>" for each extra folder (short, so the folder name fits; the desktop's
/// own list says "Capture area to <folder>").
fn screenshot_actions(s: &Settings) -> Vec<(String, String)> {
    let folders = s.extra_dirs.iter().map(|d| (folder_action(d.id), format!("To {}", short_label(&dir_name(&d.path)))));
    HOTKEY_ACTIONS
        .iter()
        .filter(|(id, _)| *id != "record")
        .map(|(i, l)| (i.to_string(), l.to_string()))
        .chain(folders)
        .collect()
}

/// The full path of the extra folder behind a shortcut id, for the row's hover text.
fn folder_path(s: &Settings, id: &str) -> Option<String> {
    s.extra_dir(id).map(|d| d.path.display().to_string())
}

/// A small × button. Returns true when clicked.
fn x_button(ui: &mut Ui, tip: &str) -> bool {
    let (r, resp) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
    let col = if resp.hovered() { TEXT } else { DIM };
    let c = r.center();
    let d = 5.0;
    ui.painter().line_segment([c + vec2(-d, -d), c + vec2(d, d)], Stroke::new(2.0, col));
    ui.painter().line_segment([c + vec2(-d, d), c + vec2(d, -d)], Stroke::new(2.0, col));
    resp.on_hover_text(tip).clicked()
}

/// The actions of the Video tab's shortcuts, as (id, label).
fn video_actions() -> Vec<(String, String)> {
    HOTKEY_ACTIONS
        .iter()
        .filter(|(id, _)| *id == "record")
        .chain(VIDEO_HOTKEY_ACTIONS.iter())
        .map(|(i, l)| (i.to_string(), l.to_string()))
        .collect()
}

/// A settings row: a right-aligned label column, then the controls.
fn row(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    row_w(ui, label, LABEL_W, 6.0, add);
}

fn row_w(ui: &mut Ui, label: &str, label_w: f32, gap: f32, add: impl FnOnce(&mut Ui)) {
    row_full(ui, label, None, label_w, gap, add);
}

/// A row whose label is cut short when it is too wide. `hover` shows on the label then.
fn row_full(ui: &mut Ui, label: &str, hover: Option<&str>, label_w: f32, gap: f32, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(vec2(label_w, ROW_H), Layout::right_to_left(Align::Center), |ui| {
            ui.set_min_size(vec2(label_w, ROW_H));
            if !label.is_empty() {
                let resp = ui.add(egui::Label::new(egui::RichText::new(label).color(TEXT)).truncate());
                if let Some(h) = hover {
                    resp.on_hover_text(h);
                }
            }
        });
        ui.add_space(gap);
        add(ui);
    });
}

/// Height of the fade at the edge of a scroll area.
const FADE_H: f32 = 12.0;

/// A short gradient over the top or bottom edge of `view`: the background color at the edge,
/// clear towards the inside. Painted on top, so it changes no layout.
fn fade_edge(painter: &egui::Painter, view: Rect, top: bool) {
    let (outer, inner) = if top {
        (view.min.y, view.min.y + FADE_H)
    } else {
        (view.max.y, view.max.y - FADE_H)
    };
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(pos2(view.min.x, outer), BG);
    mesh.colored_vertex(pos2(view.max.x, outer), BG);
    mesh.colored_vertex(pos2(view.max.x, inner), Color32::TRANSPARENT);
    mesh.colored_vertex(pos2(view.min.x, inner), Color32::TRANSPARENT);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}

fn separator(ui: &mut Ui) {
    let r = ui.available_rect_before_wrap();
    let y = r.min.y + 1.0;
    ui.painter().line_segment([pos2(r.min.x, y), pos2(r.max.x, y)], Stroke::new(1.0, Color32::from_gray(120)));
    ui.add_space(4.0);
}

fn short_path(p: &std::path::Path) -> String {
    let s = p.display().to_string();
    match dirs::home_dir() {
        Some(h) => {
            let h = h.display().to_string();
            if let Some(rest) = s.strip_prefix(&h) { format!("~{rest}") } else { s }
        }
        None => s,
    }
}

/// Maps a pressed key to a W3C key code ("KeyA", "Digit4", "F5", ...).
pub fn key_to_code(k: Key) -> Option<String> {
    let n = k.name();
    if n.len() == 1 {
        let c = n.chars().next()?;
        if c.is_ascii_alphabetic() {
            return Some(format!("Key{}", c.to_ascii_uppercase()));
        }
        if c.is_ascii_digit() {
            return Some(format!("Digit{c}"));
        }
    }
    if n.starts_with('F') && n.len() > 1 && n[1..].chars().all(|c| c.is_ascii_digit()) {
        return Some(n.to_string());
    }
    Some(
        match k {
            Key::Space => "Space",
            Key::Minus => "Minus",
            Key::Equals | Key::Plus => "Equal",
            Key::Comma => "Comma",
            Key::Period => "Period",
            Key::Slash => "Slash",
            Key::Backslash => "Backslash",
            Key::Semicolon => "Semicolon",
            Key::Quote => "Quote",
            Key::OpenBracket => "BracketLeft",
            Key::CloseBracket => "BracketRight",
            Key::Backtick => "Backquote",
            Key::ArrowUp => "ArrowUp",
            Key::ArrowDown => "ArrowDown",
            Key::ArrowLeft => "ArrowLeft",
            Key::ArrowRight => "ArrowRight",
            Key::Enter => "Enter",
            Key::Tab => "Tab",
            Key::Backspace => "Backspace",
            Key::Delete => "Delete",
            _ => return None,
        }
        .to_string(),
    )
}
