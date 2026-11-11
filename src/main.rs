mod attachments;
mod document;
mod grep_search;
mod navigation;
mod playback;
#[path = "../tools/profile.rs"]
mod profile;
mod quick_select;
mod reading_cursor;
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
    let startup = Instant::now();
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new("--profile")) {
        profile::run();
        return Ok(());
    }
    let mut args = std::env::args_os().skip(1);
    let first = args.next();
    if first.as_deref().is_some_and(|a| a == "--help" || a == "-h") {
        println!(
            "md-reader [FILE]\n\nRead Markdown on Wayland. Open: Ctrl+O, reload: R or Ctrl+R, quick select: Space.\nFind text: / or Ctrl+F, search all files: S, headings: [ and ].\nDrop a file onto the window to open it. Preferences are saved automatically."
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
        Box::new(move |cc| {
            let mut reader = Reader::new(cc, path);
            if std::env::var_os("MD_READER_PROFILE_STARTUP").is_some() {
                reader.startup = Some(startup);
                eprintln!(
                    "startup_setup_ms={:.3}",
                    startup.elapsed().as_secs_f64() * 1000.0
                );
            }
            Ok(Box::new(reader))
        }),
    )
}

struct Reader {
    startup: Option<Instant>,
    preferences: Preferences,
    document: Option<Document>,
    cache: CommonMarkCache,
    attachments: std::rc::Rc<std::cell::RefCell<attachments::Attachments>>,
    text_index: text_search::TextIndex,
    fonts: Option<fontdb::Database>,
    families: Vec<String>,
    applied_font: String,
    applied_theme: Option<Theme>,
    last_poll: Instant,
    picker: Option<mpsc::Receiver<Option<PathBuf>>>,
    error: Option<String>,
    settings: bool,
    generation: u64,
    navigator: navigation::Navigator,
    cursor: reading_cursor::ReadingCursor,
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
        let mut reader = Self {
            startup: None,
            preferences,
            document: None,
            cache: CommonMarkCache::default(),
            attachments: Default::default(),
            text_index: text_search::TextIndex::default(),
            fonts: None,
            families: Vec::new(),
            applied_font: String::new(),
            applied_theme: None,
            last_poll: Instant::now(),
            picker: None,
            error: None,
            settings: false,
            generation: 0,
            navigator: navigation::Navigator::default(),
            cursor: reading_cursor::ReadingCursor::default(),
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
                self.cursor = reading_cursor::ReadingCursor::default();
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
        self.attachments.borrow_mut().clear();
        self.cache = CommonMarkCache::default();
        self.text_index = text_search::TextIndex::default();
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
                .add_filter("Markdown", &["md", "markdown", "mdown", "mkd"])
                .add_filter("Plain text", &["txt"])
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
        if self.fonts.is_none() && (self.settings || self.preferences.font != "Built-in") {
            let mut fonts = fontdb::Database::new();
            fonts.load_system_fonts();
            self.families = fonts
                .faces()
                .flat_map(|f| f.families.iter().map(|(name, _)| name.clone()))
                .collect();
            self.families.sort();
            self.families.dedup();
            self.fonts = Some(fonts);
        }
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
                let fonts = self.fonts.as_ref().unwrap();
                if let Some(id) = fonts.query(&query) {
                    if let Some(data) = fonts.with_face_data(id, |bytes, index| {
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
        ui.checkbox(
            &mut self.preferences.auto_reload,
            "Reload changed documents automatically",
        );
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
        self.cursor.input(
            &ctx,
            self.document.is_some()
                && self.quick_select.is_none()
                && self.grep.is_none()
                && self.picker.is_none()
                && !ctx.text_edit_focused()
                && !(self.settings && ctx.egui_wants_keyboard_input()),
            self.navigator.word_end_reserved(&ctx),
        );
        if self.quick_select.is_none()
            && self.grep.is_none()
            && !ctx.text_edit_focused()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Space))
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
        if no_modal {
            self.search.option_shortcuts(&ctx);
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
        let force_reload = no_modal
            && (ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::R))
                || (!typing
                    && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::R))));
        if force_reload {
            self.reload(&ctx);
        }
        if self.last_poll.elapsed() >= Duration::from_millis(500) {
            if self.preferences.auto_reload {
                self.reload(&ctx);
            } else if let Some(document) = &mut self.document {
                document.check_for_changes();
            }
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
                    .on_hover_text("R / Ctrl+R · reload from disk")
                    .clicked()
                {
                    self.reload(&ctx);
                }
                if ui.button("Quick select (Space)").clicked() {
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
        if self
            .document
            .as_ref()
            .is_some_and(|document| document.changed)
        {
            egui::Panel::top("document-changed").show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label("This document has changed on disk. Reload to see the changes?");
                    if ui.button("Reload (R)").clicked() {
                        self.reload(&ctx);
                    }
                });
            });
        }
        if self.search.open && self.document.is_some() {
            egui::Window::new("Find in document")
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .collapsible(false)
                .resizable(false)
                .show(&ctx, |ui| self.search.bar(ui));
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
                        "{} · {}",
                        self.preferences.theme(chrono::Local::now().hour()).name(),
                        if self.preferences.auto_reload {
                            "Auto-reload on"
                        } else {
                            "Auto-reload off"
                        }
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
                        let width = (viewport.width() - 24.0)
                            .max(1.0)
                            .min(self.preferences.width);
                        let inset = ((viewport.width() - width) / 2.0).max(0.0);
                        let mut report = egui_commonmark::RenderReport::default();
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            ui.add_space(inset);
                            ui.vertical(|ui| {
                                ui.set_width(width);
                                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                                ui.add_space(20.0);
                                let base = url::Url::from_directory_path(
                                    doc.path.parent().unwrap_or(std::path::Path::new("/")),
                                )
                                .map(|u| u.to_string())
                                .unwrap_or_else(|_| "file:///".into());
                                let attachments = self.attachments.clone();
                                let attachment_base = url::Url::parse(&base).unwrap();
                                let render_attachment =
                                    move |ui: &mut egui::Ui, destination: &str, title: &str| {
                                        attachments.borrow_mut().render(
                                            ui,
                                            &attachment_base,
                                            destination,
                                            title,
                                        )
                                    };
                                let viewer = CommonMarkViewer::new()
                                    .render_attachment_fn(Some(&render_attachment))
                                    .line_height(
                                        self.preferences.size * self.preferences.line_height,
                                    )
                                    .paragraph_spacing(self.preferences.paragraph_gap)
                                    .default_implicit_uri_scheme(base)
                                    .max_image_width(Some(width as usize));
                                report = if doc
                                    .path
                                    .extension()
                                    .is_some_and(|ext| ext.eq_ignore_ascii_case("txt"))
                                {
                                    egui_commonmark::show_plain_text(
                                        ui,
                                        &doc.text,
                                        self.preferences.size * self.preferences.line_height,
                                    )
                                } else {
                                    viewer.show_with_report(ui, &mut self.cache, &doc.text).1
                                };
                                ui.add_space(40.0);
                            });
                        });
                        self.text_index.update(&report);
                        let index = &self.text_index;
                        self.search.render(ui, index, ui.clip_rect().top());
                        self.cursor.navigate(&commands);
                        self.cursor.render(ui, index);
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
                    ui.label("Open a Markdown or plain-text document, or drop one here.");
                    ui.add_space(20.0);
                    if ui.button("Open a document…").clicked() {
                        self.choose(&ctx);
                    }
                    ui.add_space(12.0);
                    ui.weak("Ctrl+O to open · R to reload");
                });
            }
        });
        if let Some(startup) = self.startup.take() {
            eprintln!(
                "startup_first_ui_ms={:.3}",
                startup.elapsed().as_secs_f64() * 1000.0
            );
        }
    }
}

#[cfg(test)]
mod rendering_tests {
    use super::*;
    #[test]
    fn plain_text_preserves_markup_whitespace_and_source_locations() {
        let ctx = egui::Context::default();
        let text = "# Heading\n\n  **literal** [link](file.md)\n\tindented 日本語\n";
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                let report = egui_commonmark::show_plain_text(ui, text, 24.0);
                assert_eq!(report.runs.len(), 1);
                let run = &report.runs[0];
                assert_eq!(run.galley.text(), text);
                assert_eq!(run.source, 0..text.len());
                assert_eq!(run.heading, None);
                assert!(run.galley.rows.len() >= 5);
            });
        });
        output.textures_delta.clear();
    }

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
