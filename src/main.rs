mod document;
mod grep_search;
mod navigation;
mod quick_select;
mod text_search;
mod theme;

use chrono::Timelike;
use document::Document;
use eframe::egui::{self, FontData, FontDefinitions, FontFamily, FontId, RichText};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use std::{
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant},
};
use theme::{Preferences, Theme};

fn main() -> eframe::Result {
    let mut args = std::env::args_os().skip(1);
    let first = args.next();
    if first.as_deref().is_some_and(|a| a == "--help" || a == "-h") {
        println!(
            "md-reader [FILE]\n\nRead Markdown on Wayland. Open: Ctrl+O, reload: Ctrl+R, quick select: F.\nFind text: / or Ctrl+F, search all files: S, headings: [ and ].\nDrop a file onto the window to open it. Preferences are saved automatically."
        );
        return Ok(());
    }
    let path = first.map(PathBuf::from);
    eframe::run_native(
        "Markdown Reader",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1000.0, 800.0])
                .with_min_inner_size([580.0, 360.0]),
            ..Default::default()
        },
        Box::new(move |cc| Ok(Box::new(Reader::new(cc, path)))),
    )
}

struct Reader {
    preferences: Preferences,
    document: Option<Document>,
    cache: CommonMarkCache,
    fonts: fontdb::Database,
    families: Vec<String>,
    applied_font: String,
    applied_theme: Option<Theme>,
    last_poll: Instant,
    picker: Option<mpsc::Receiver<Option<PathBuf>>>,
    error: Option<String>,
    settings: bool,
    generation: u64,
    navigator: navigation::Navigator,
    search: text_search::Search,
    grep: Option<grep_search::GrepSearch>,
    quick_select: Option<quick_select::QuickSelect>,
    launch_directory: Result<PathBuf, String>,
}
impl Reader {
    fn new(cc: &eframe::CreationContext<'_>, path: Option<PathBuf>) -> Self {
        let launch_directory = std::env::current_dir().map_err(|error| error.to_string());
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let mut preferences: Preferences = cc
            .storage
            .and_then(|s| eframe::get_value(s, "preferences"))
            .unwrap_or_default();
        preferences.sanitize();
        let mut fonts = fontdb::Database::new();
        fonts.load_system_fonts();
        let mut families: Vec<_> = fonts
            .faces()
            .flat_map(|f| f.families.iter().map(|(name, _)| name.clone()))
            .collect();
        families.sort();
        families.dedup();
        let mut reader = Self {
            preferences,
            document: None,
            cache: CommonMarkCache::default(),
            fonts,
            families,
            applied_font: String::new(),
            applied_theme: None,
            last_poll: Instant::now(),
            picker: None,
            error: None,
            settings: false,
            generation: 0,
            navigator: navigation::Navigator::default(),
            search: text_search::Search::default(),
            grep: None,
            quick_select: None,
            launch_directory,
        };
        if let Some(path) = path {
            reader.open(path, &cc.egui_ctx);
        }
        reader
    }
    fn open(&mut self, path: PathBuf, ctx: &egui::Context) -> bool {
        match Document::open(path) {
            Ok(document) => {
                let name = document
                    .path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy();
                ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!(
                    "{name} — Markdown Reader"
                )));
                self.navigator.reset();
                self.search.clear(ctx);
                self.document = Some(document);
                self.error = None;
                self.generation = self.generation.wrapping_add(1);
                self.clear_cache(ctx);
                true
            }
            Err(e) => {
                self.error = Some(format!("Could not open document: {e}"));
                false
            }
        }
    }
    fn clear_cache(&mut self, ctx: &egui::Context) {
        self.cache = CommonMarkCache::default();
        ctx.forget_all_images();
    }
    fn reload(&mut self, ctx: &egui::Context) {
        if self.document.as_mut().is_some_and(Document::reload) {
            self.clear_cache(ctx);
        }
    }
    fn choose(&mut self, ctx: &egui::Context) {
        if self.picker.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.picker = Some(rx);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let path = rfd::FileDialog::new()
                .add_filter("Markdown", &["md", "markdown", "mdown", "mkd", "txt"])
                .add_filter("All files", &["*"])
                .pick_file();
            let _ = tx.send(path);
            ctx.request_repaint();
        });
    }
    fn quick_select(&mut self, ctx: &egui::Context) {
        match &self.launch_directory {
            Ok(root) => self.quick_select = Some(quick_select::QuickSelect::new(root.clone(), ctx)),
            Err(error) => self.error = Some(format!("Could not find launch directory: {error}")),
        }
    }

    fn appearance(&mut self, ctx: &egui::Context) {
        let theme = self.preferences.theme(chrono::Local::now().hour());
        if self.applied_theme != Some(theme) {
            ctx.set_visuals(theme.visuals());
            self.applied_theme = Some(theme);
        }
        ctx.global_style_mut(|style| {
            for (kind, factor) in [
                (egui::TextStyle::Body, 1.0),
                (egui::TextStyle::Button, 0.85),
                (egui::TextStyle::Small, 0.75),
                (egui::TextStyle::Heading, 1.6),
                (egui::TextStyle::Monospace, 0.9),
            ] {
                let family = if kind == egui::TextStyle::Monospace {
                    FontFamily::Monospace
                } else {
                    FontFamily::Proportional
                };
                style
                    .text_styles
                    .insert(kind, FontId::new(self.preferences.size * factor, family));
            }
        });
        if self.applied_font != self.preferences.font {
            let mut definitions = FontDefinitions::default();
            if self.preferences.font != "Built-in" {
                let query = fontdb::Query {
                    families: &[fontdb::Family::Name(&self.preferences.font)],
                    ..Default::default()
                };
                if let Some(id) = self.fonts.query(&query) {
                    if let Some(data) = self.fonts.with_face_data(id, |bytes, index| {
                        let mut data = FontData::from_owned(bytes.to_vec());
                        data.index = index;
                        data
                    }) {
                        definitions.font_data.insert("selected".into(), data.into());
                        definitions
                            .families
                            .entry(FontFamily::Proportional)
                            .or_default()
                            .insert(0, "selected".into());
                    }
                } else {
                    self.error = Some(format!(
                        "Font '{}' is unavailable; using the built-in font.",
                        self.preferences.font
                    ));
                }
            }
            ctx.set_fonts(definitions);
            self.applied_font = self.preferences.font.clone();
        }
    }
    fn settings_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Reading preferences");
        ui.add_space(12.0);
        ui.label("Font family");
        egui::ComboBox::from_id_salt("font")
            .selected_text(&self.preferences.font)
            .width(220.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.preferences.font, "Built-in".into(), "Built-in");
                for family in &self.families {
                    ui.selectable_value(&mut self.preferences.font, family.clone(), family);
                }
            });
        ui.add(egui::Slider::new(&mut self.preferences.size, 10.0..=36.0).text("Text size"));
        ui.separator();
        ui.label("Visible spacing");
        ui.add(egui::Slider::new(&mut self.preferences.line_height, 1.0..=2.4).text("Line height"));
        ui.add(
            egui::Slider::new(&mut self.preferences.paragraph_gap, 0.0..=40.0)
                .text("Paragraph gap"),
        );
        ui.add(
            egui::Slider::new(&mut self.preferences.width, 360.0..=1400.0).text("Reading width"),
        );
        ui.small("Display only; your document is never modified.");
        ui.separator();
        ui.checkbox(&mut self.preferences.automatic, "Follow time of day");
        if self.preferences.automatic {
            ui.label("Day theme");
            theme_picker(ui, "day", &mut self.preferences.day);
            ui.label("Night theme");
            theme_picker(ui, "night", &mut self.preferences.night);
            ui.horizontal(|ui| {
                ui.label("Day begins");
                ui.add(
                    egui::DragValue::new(&mut self.preferences.day_start)
                        .range(0..=23)
                        .suffix(":00"),
                );
            });
            ui.horizontal(|ui| {
                ui.label("Night begins");
                ui.add(
                    egui::DragValue::new(&mut self.preferences.night_start)
                        .range(0..=23)
                        .suffix(":00"),
                );
            });
            ui.small("Uses your local clock (24-hour time).");
            if self.preferences.day_start == self.preferences.night_start {
                ui.small("Equal hours use the night theme all day.");
            }
        } else {
            theme_picker(ui, "manual", &mut self.preferences.manual);
        }
        ui.add_space(12.0);
        ui.small("Preferences are saved automatically.");
    }
}
fn theme_picker(ui: &mut egui::Ui, id: &str, selected: &mut Theme) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(selected.name())
        .show_ui(ui, |ui| {
            for theme in Theme::ALL {
                ui.selectable_value(selected, theme, theme.name());
            }
        });
}
impl eframe::App for Reader {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, "preferences", &self.preferences);
    }
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        ctx.request_repaint_after(Duration::from_millis(500));
        if let Some(rx) = &self.picker {
            match rx.try_recv() {
                Ok(path) => {
                    self.picker = None;
                    if let Some(path) = path {
                        self.open(path, &ctx);
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.picker = None;
                    self.error = Some("The file chooser closed unexpectedly.".into());
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if let Some(path) = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .next()
        }) {
            self.open(path, &ctx);
        }
        if self.quick_select.is_none()
            && self.grep.is_none()
            && !ctx.text_edit_focused()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F))
        {
            self.quick_select(&ctx);
        }
        if self.quick_select.is_none()
            && self.grep.is_none()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::O))
        {
            self.choose(&ctx);
        }
        let no_modal = self.quick_select.is_none() && self.grep.is_none() && self.picker.is_none();
        if no_modal && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.search.clear(&ctx);
        }
        let typing = ctx.text_edit_focused();
        if no_modal && self.document.is_some() && self.search.open_shortcut(&ctx) {
            self.navigator.reset();
        }
        if no_modal
            && !typing
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::S))
        {
            match &self.launch_directory {
                Ok(root) => self.grep = Some(grep_search::GrepSearch::new(root.clone())),
                Err(error) => {
                    self.error = Some(format!("Could not find launch directory: {error}"))
                }
            }
        }
        if no_modal && !typing {
            self.search.shortcuts(&ctx);
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::OpenBracket)) {
                self.navigator.reset();
                self.search.heading_step = Some(false);
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::CloseBracket)) {
                self.navigator.reset();
                self.search.heading_step = Some(true);
            }
        }
        let force_reload = ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::R));
        if force_reload || self.last_poll.elapsed() >= Duration::from_millis(500) {
            self.reload(&ctx);
            self.last_poll = Instant::now();
        }
        self.appearance(&ctx);
        let document_shortcuts =
            self.quick_select.is_none() && self.grep.is_none() && self.picker.is_none();
        if !document_shortcuts {
            self.navigator.reset();
        }
        if let Some(chooser) = &mut self.quick_select {
            match chooser.show(&ctx) {
                quick_select::Action::Stay => {}
                quick_select::Action::Close => self.quick_select = None,
                quick_select::Action::Open(path) => {
                    self.quick_select = None;
                    self.open(path, &ctx);
                }
            }
        }
        if let Some(grep) = &mut self.grep {
            match grep.show(&ctx) {
                grep_search::Action::Stay => {}
                grep_search::Action::Close => self.grep = None,
                grep_search::Action::Open { path, line, query } => {
                    self.grep = None;
                    if self.open(path, &ctx)
                        && let Some(doc) = &self.document
                    {
                        let offset = doc
                            .text
                            .split_inclusive('\n')
                            .take(line.saturating_sub(1))
                            .map(str::len)
                            .sum();
                        self.search.target(query, offset);
                    }
                }
            }
        }
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        self.picker.is_none(),
                        egui::Button::new(if self.picker.is_some() {
                            "Choosing…"
                        } else {
                            "Open…"
                        }),
                    )
                    .on_hover_text("Ctrl+O · system file chooser")
                    .clicked()
                {
                    self.choose(&ctx);
                }
                if ui
                    .add_enabled(self.document.is_some(), egui::Button::new("Reload"))
                    .on_hover_text("Ctrl+R · also reloads automatically")
                    .clicked()
                {
                    self.reload(&ctx);
                }
                if ui.button("Quick select (F)").clicked() {
                    self.quick_select(&ctx);
                }
                ui.separator();
                ui.label(RichText::new("Markdown Reader").strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.selectable_label(self.settings, "Appearance").clicked() {
                        self.settings = !self.settings;
                    }
                });
            });
            ui.add_space(6.0);
        });
        if self.search.open && self.document.is_some() {
            egui::Panel::top("document-search").show(ui, |ui| self.search.bar(ui));
        }
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.small(
                    self.document
                        .as_ref()
                        .map(|d| d.path.display().to_string())
                        .unwrap_or_else(|| "Open a Markdown file to start reading".into()),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.small(format!(
                        "{} · Autoreload",
                        self.preferences.theme(chrono::Local::now().hour()).name()
                    ));
                });
            });
        });
        if self.settings {
            egui::Panel::right("appearance")
                .default_size(280.0)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| self.settings_ui(ui));
                });
        }
        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(error) = &self.error {
                let error = error.clone();
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(egui::Color32::from_rgb(220, 100, 90), error);
                    if ui.small_button("Dismiss").clicked() {
                        self.error = None;
                    }
                });
                ui.separator();
            }
            if let Some(doc) = &self.document {
                if let Some(error) = &doc.error {
                    ui.colored_label(
                        egui::Color32::from_rgb(220, 100, 90),
                        format!("Reload failed: {error}. Showing last readable version; retrying…"),
                    );
                }
                let commands = self.navigator.commands(
                    &ctx,
                    document_shortcuts
                        && !ctx.text_edit_focused()
                        && !(self.settings && ctx.egui_wants_keyboard_input()),
                );
                if !commands.is_empty() || ctx.input(|i| i.smooth_scroll_delta != egui::Vec2::ZERO)
                {
                    self.search.manually_scrolled();
                }
                egui::ScrollArea::both()
                    .animated(false)
                    .id_salt(("document", self.generation))
                    .auto_shrink([false, false])
                    .show_viewport(ui, |ui, viewport| {
                        let width = ui.available_width().min(self.preferences.width);
                        let inset = ((ui.available_width() - width) / 2.0).max(0.0);
                        let mut report = egui_commonmark::RenderReport::default();
                        ui.horizontal(|ui| {
                            ui.add_space(inset);
                            ui.vertical(|ui| {
                                ui.set_width(width);
                                ui.add_space(20.0);
                                let base = url::Url::from_directory_path(
                                    doc.path.parent().unwrap_or(std::path::Path::new("/")),
                                )
                                .map(|u| u.to_string())
                                .unwrap_or_else(|_| "file:///".into());
                                let viewer = CommonMarkViewer::new()
                                    .line_height(
                                        self.preferences.size * self.preferences.line_height,
                                    )
                                    .paragraph_spacing(self.preferences.paragraph_gap)
                                    .default_implicit_uri_scheme(base)
                                    .max_image_width(Some(width as usize));
                                if self.search.needs_report() {
                                    report =
                                        viewer.show_with_report(ui, &mut self.cache, &doc.text).1;
                                } else {
                                    viewer.show(ui, &mut self.cache, &doc.text);
                                }
                                ui.add_space(40.0);
                            });
                        });
                        let index = text_search::TextIndex::new(&report);
                        self.search.render(ui, &index, ui.clip_rect().top());
                        navigation::apply(
                            ui,
                            viewport,
                            &commands,
                            (self.preferences.size * self.preferences.line_height).max(24.0),
                        );
                    });
            } else {
                ui.vertical_centered(|ui| {
                    ui.add_space((ui.available_height() * 0.25).max(20.0));
                    ui.heading("A quiet place to read.");
                    ui.add_space(12.0);
                    ui.label("Open a Markdown document, or drop one here.");
                    ui.add_space(20.0);
                    if ui.button("Open a document…").clicked() {
                        self.choose(&ctx);
                    }
                    ui.add_space(12.0);
                    ui.weak("Ctrl+O to open · Changes reload automatically");
                });
            }
        });
    }
}

#[cfg(test)]
mod rendering_tests {
    use super::*;
    fn height(text: &str, line: f32, gap: f32) -> f32 {
        let ctx = egui::Context::default();
        let mut cache = CommonMarkCache::default();
        let mut height = 0.0;
        for _ in 0..2 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(500.0, 1000.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        height = CommonMarkViewer::new()
                            .line_height(line)
                            .paragraph_spacing(gap)
                            .show(ui, &mut cache, text)
                            .response
                            .rect
                            .height();
                    });
                },
            );
            output.textures_delta.clear();
        }
        height
    }
    #[test]
    fn visible_spacing_changes_rendered_height() {
        let paragraph = "A line with a hard break.  \nAnother line.\n\nA second paragraph.";
        let normal = height(paragraph, 20.0, 5.0);
        assert!(height(paragraph, 36.0, 5.0) > normal + 10.0);
        assert!(height(paragraph, 20.0, 35.0) > normal + 10.0);
    }
    #[test]
    fn basic_markdown_renders_without_mutating_source() {
        let text = "# Heading\n\n**Bold** and *italic* with [a link](https://example.com).\n\n- [ ] Read only\n- Item\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\n> Quote\n\n```rust\nfn main() {}\n```\n\n---\n";
        let original = text.to_owned();
        assert!(height(text, 24.0, 12.0) > 100.0);
        assert_eq!(text, original);
    }
}
