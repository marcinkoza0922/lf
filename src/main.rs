mod attachments;
mod document;
mod grep_search;
mod history;
mod navigation;
mod playback;
#[path = "../tools/profile.rs"]
mod profile;
mod quick_select;
mod reading_cursor;
#[path = "../tools/screenshot.rs"]
mod screenshot;
mod sidebar;
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
            "lf [FILE]\n\nRead Markdown and UTF-8 text on Linux and Windows. Open: Ctrl+Shift+O, reload: R or Ctrl+R, quick select: Space.\nBack/forward: Ctrl+O / Ctrl+I, Alt+Left / Alt+Right, or mouse back/forward buttons.\nSidebar: ` (opened files), Ctrl+E (file explorer).\nFind text: / or Ctrl+F, search all files: S, headings: [ and ].\nDrop a file onto the window to open it. Preferences are saved automatically."
        );
        return Ok(());
    }
    let path = first.map(PathBuf::from);
    eframe::run_native(
        "lf",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1000.0, 800.0])
                .with_min_inner_size([580.0, 360.0]),
            ..Default::default()
        },
        Box::new(move |cc| {
            let mut reader = Reader::new(cc, path);
            if std::env::var_os("LF_PROFILE_STARTUP").is_some() {
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
    screenshot: Option<screenshot::Screenshot>,
    history: history::History,
    /// Documents opened this session, most recent first.
    opened: Vec<PathBuf>,
    sidebar: sidebar::Sidebar,
    /// The document scroll offset of the last frame.
    scroll: egui::Vec2,
    /// A scroll offset being restored by back/forward, with frames attempted.
    restore: Option<(egui::Vec2, u8)>,
    /// A heading anchor to scroll to once the document is laid out.
    anchor: Option<String>,
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
            screenshot: screenshot::Screenshot::from_env(),
            history: Default::default(),
            opened: Vec::new(),
            sidebar: Default::default(),
            scroll: egui::Vec2::ZERO,
            restore: None,
            anchor: None,
        };
        if reader.screenshot.is_some()
            && let Some(tab) = screenshot::Screenshot::sidebar()
        {
            reader.preferences.sidebar_layout = sidebar::Layout {
                expanded: true,
                tab,
            };
        }
        if let Some(path) = path {
            reader.open(path, &cc.egui_ctx);
        }
        reader
    }
    /// Open a document as a new destination, recording the current place in history.
    fn open(&mut self, path: PathBuf, ctx: &egui::Context) -> bool {
        let from = self.location();
        if !self.load(path, ctx) {
            return false;
        }
        if let Some(from) = from
            && self.document.as_ref().is_some_and(|d| d.path != from.path)
        {
            self.history.visit(from);
        }
        true
    }
    fn location(&self) -> Option<history::Location> {
        self.document.as_ref().map(|document| history::Location {
            path: document.path.clone(),
            scroll: self.scroll,
            cursor: self.cursor.clone(),
        })
    }
    /// Go back (or forward), restoring the destination's scroll position and cursor.
    fn step(&mut self, back: bool, ctx: &egui::Context) {
        let Some(current) = self.location() else {
            return;
        };
        let Some(target) = self.history.step(back, current) else {
            return;
        };
        let same = self
            .document
            .as_ref()
            .is_some_and(|d| d.path == target.path);
        if !same && !self.load(target.path.clone(), ctx) {
            self.history.cancel(back);
            return;
        }
        self.cursor = target.cursor;
        self.restore = Some((target.scroll, 0));
        self.anchor = None;
        self.search.manually_scrolled();
    }
    /// Follow a document or heading link.
    fn follow(&mut self, jump: attachments::Jump, ctx: &egui::Context) {
        let current = self.document.as_ref().map(|d| d.path.clone());
        let elsewhere = jump
            .path
            .as_ref()
            .is_some_and(|path| path.canonicalize().ok().as_ref() != current.as_ref());
        if elsewhere {
            if !self.open(jump.path.unwrap(), ctx) {
                return;
            }
        } else if jump.fragment.is_some()
            && let Some(from) = self.location()
        {
            self.history.visit(from);
        }
        self.anchor = jump.fragment;
    }
    fn sidebar_action(&mut self, action: Option<sidebar::Action>, ctx: &egui::Context) {
        let layout = &mut self.preferences.sidebar_layout;
        match action {
            Some(sidebar::Action::Open {
                path,
                focus_document,
            }) => {
                if focus_document {
                    self.sidebar.focused = false;
                }
                self.open(path, ctx);
            }
            Some(sidebar::Action::Expand(tab)) => {
                layout.expanded = true;
                layout.tab = tab;
                self.sidebar.focused = true;
            }
            Some(sidebar::Action::Collapse) => {
                layout.expanded = false;
                self.sidebar.focused = false;
            }
            None => {}
        }
    }
    /// Remember a document in this session's list and the persisted recent list.
    fn remember(&mut self, path: &std::path::Path) {
        for list in [&mut self.opened, &mut self.preferences.recent] {
            list.retain(|p| p != path);
            list.insert(0, path.to_path_buf());
        }
        self.preferences.recent.truncate(theme::RECENT_LIMIT);
    }
    /// Load a document into the window without touching history.
    fn load(&mut self, path: PathBuf, ctx: &egui::Context) -> bool {
        match Document::open(path) {
            Ok(document) => {
                let name = document
                    .path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy();
                ctx.send_viewport_cmd(egui::ViewportCommand::Title(format!("{name} — lf")));
                self.navigator.reset();
                self.search.clear(ctx);
                self.cursor = reading_cursor::ReadingCursor::default();
                self.remember(&document.path);
                self.restore = None;
                self.anchor = None;
                self.scroll = egui::Vec2::ZERO;
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
/// A toolbar button with a painted chevron, independent of the font's glyphs.
fn arrow_button(ui: &mut egui::Ui, back: bool, enabled: bool) -> egui::Response {
    let height = ui.spacing().interact_size.y;
    let response = ui.add_enabled(
        enabled,
        egui::Button::new("").min_size(egui::vec2(height * 1.2, height)),
    );
    let rect =
        egui::Rect::from_center_size(response.rect.center(), egui::vec2(1.0, 1.0) * height * 0.4);
    let (tip, tail) = if back {
        (rect.left(), rect.right())
    } else {
        (rect.right(), rect.left())
    };
    let stroke = ui.style().interact(&response).fg_stroke;
    let color = if enabled {
        stroke.color
    } else {
        ui.visuals().weak_text_color()
    };
    ui.painter().add(egui::Shape::line(
        vec![
            egui::pos2(tail, rect.top()),
            egui::pos2(tip, rect.center().y),
            egui::pos2(tail, rect.bottom()),
        ],
        egui::Stroke::new(stroke.width.max(1.5), color),
    ));
    response
}
fn sidebar_lists<'a>(
    opened: &'a [PathBuf],
    recent: &'a [PathBuf],
    document: &'a Option<Document>,
    root: &'a Result<PathBuf, String>,
) -> sidebar::Lists<'a> {
    sidebar::Lists {
        opened,
        recent,
        current: document.as_ref().map(|d| d.path.as_path()),
        root: root.as_deref().map_err(String::as_str),
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
        if self.screenshot.is_some() {
            return;
        }
        eframe::set_value(storage, "preferences", &self.preferences);
    }
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        ctx.request_repaint_after(Duration::from_millis(500));
        if let Some(screenshot) = &mut self.screenshot {
            screenshot.update(&ctx);
        }
        // Links followed during the previous frame open in this window.
        let jump = self.attachments.borrow_mut().take_jump();
        if let Some(jump) = jump {
            self.follow(jump, &ctx);
        }
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
            && self.picker.is_none()
            && !ctx.text_edit_focused()
        {
            let lists = sidebar_lists(
                &self.opened,
                &self.preferences.recent,
                &self.document,
                &self.launch_directory,
            );
            let action = self
                .sidebar
                .keys(&ctx, &mut self.preferences.sidebar_layout, &lists);
            self.sidebar_action(action, &ctx);
        }
        self.cursor.input(
            &ctx,
            !self.sidebar.focused
                && self.document.is_some()
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
            && ctx.input_mut(|i| {
                i.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::O)
            })
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
        if no_modal {
            // Consumed before arrow-key scrolling, which ignores extra modifiers.
            let (back, forward) = ctx.input_mut(|i| {
                let back = i.consume_key(egui::Modifiers::CTRL, egui::Key::O)
                    | i.consume_key(egui::Modifiers::ALT, egui::Key::ArrowLeft)
                    | i.pointer.button_pressed(egui::PointerButton::Extra1);
                let forward = i.consume_key(egui::Modifiers::CTRL, egui::Key::I)
                    | i.consume_key(egui::Modifiers::ALT, egui::Key::ArrowRight)
                    | i.pointer.button_pressed(egui::PointerButton::Extra2);
                (back, forward)
            });
            if back || forward {
                self.step(back, &ctx);
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::E)) {
                self.sidebar
                    .shortcut(&mut self.preferences.sidebar_layout, sidebar::Tab::Files);
            }
            if !typing
                && ctx.input_mut(|i| {
                    let key = i.consume_key(egui::Modifiers::NONE, egui::Key::Backtick);
                    // Layouts that produce ` through a dead key or modifier send text only.
                    let text = !i.modifiers.ctrl
                        && !i.modifiers.alt
                        && i.events
                            .iter()
                            .any(|e| matches!(e, egui::Event::Text(t) if t == "`"));
                    i.events
                        .retain(|e| !matches!(e, egui::Event::Text(t) if t == "`"));
                    key || text
                })
            {
                self.sidebar
                    .shortcut(&mut self.preferences.sidebar_layout, sidebar::Tab::Opened);
            }
        }
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
                for (back, hint) in [
                    (true, "Back · Ctrl+O, Alt+Left, mouse back button"),
                    (false, "Forward · Ctrl+I, Alt+Right, mouse forward button"),
                ] {
                    if arrow_button(ui, back, self.history.can_step(back))
                        .on_hover_text(hint)
                        .on_disabled_hover_text(hint)
                        .clicked()
                    {
                        self.step(back, &ctx);
                    }
                }
                ui.separator();
                if ui
                    .add_enabled(
                        self.picker.is_none(),
                        egui::Button::new(if self.picker.is_some() {
                            "Choosing…"
                        } else {
                            "Open…"
                        }),
                    )
                    .on_hover_text("Ctrl+Shift+O · system file chooser")
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
                ui.label(RichText::new("lf").strong());
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
        let layout = self.preferences.sidebar_layout;
        let mut expanded = layout.expanded;
        let lists = sidebar_lists(
            &self.opened,
            &self.preferences.recent,
            &self.document,
            &self.launch_directory,
        );
        let rail_width = ui.spacing().interact_size.y * 1.2 + 12.0;
        let panel = egui::Panel::show_switched(
            ui,
            &mut expanded,
            egui::Panel::left("sidebar-rail")
                .exact_size(rail_width)
                .resizable(false),
            egui::Panel::left("sidebar")
                .resizable(true)
                .default_size(260.0)
                .min_size(160.0),
            |ui, shown| self.sidebar.show(ui, layout, shown, &lists),
        );
        if !expanded {
            // Collapsed by dragging the edge.
            self.preferences.sidebar_layout.expanded = false;
        }
        // Keyboard focus follows the pointer: into the sidebar, or back to the document.
        if let Some(pos) = ctx.input(|i| {
            i.pointer
                .primary_pressed()
                .then(|| i.pointer.interact_pos())
                .flatten()
        }) {
            self.sidebar.focused =
                self.preferences.sidebar_layout.expanded && panel.response.rect.contains(pos);
        }
        self.sidebar_action(panel.inner, &ctx);
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
                        && !self.sidebar.focused
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
                                self.attachments.borrow_mut().line_height =
                                    Some(self.preferences.size * self.preferences.line_height);
                                let attachments = self.attachments.clone();
                                let attachment_base = url::Url::parse(&base).unwrap();
                                let source = doc.text.as_str();
                                let render_attachment =
                                    move |ui: &mut egui::Ui,
                                          attachment: &egui_commonmark::Attachment<'_>| {
                                        attachments.borrow_mut().render(
                                            ui,
                                            &attachment_base,
                                            source,
                                            attachment,
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
                        self.scroll = viewport.min.to_vec2();
                        if let Some(fragment) = self.anchor.take() {
                            match history::find_heading(&self.text_index, &fragment) {
                                Some((id, rect)) => {
                                    ui.scroll_to_rect_animation(
                                        rect,
                                        Some(egui::Align::TOP),
                                        egui::style::ScrollAnimation::none(),
                                    );
                                    self.search.heading = Some(id);
                                }
                                None => {
                                    self.error =
                                        Some(format!("No heading matches #{fragment} here."))
                                }
                            }
                        }
                        if let Some((target, frames)) = &mut self.restore {
                            // The new document's height is only known after layout, so
                            // keep scrolling until the offset is reached or unreachable.
                            let delta = *target - self.scroll;
                            if delta.length() < 0.5 || *frames >= 30 {
                                self.restore = None;
                            } else {
                                *frames += 1;
                                ui.scroll_with_delta_animation(
                                    -delta,
                                    egui::style::ScrollAnimation::none(),
                                );
                                ui.ctx().request_repaint();
                            }
                        }
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
                    ui.weak("Ctrl+Shift+O to open · ` recent files · Ctrl+E browse");
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

    #[test]
    fn table_cells_wrap_and_rows_grow_to_contain_text() {
        let text = "| Topic | Decision |\n|---|---|\n| Game data | `QUAKE_BASEDIR` env var (default: the `vkQuake directory with maps` symlink). Data-dependent tests skip when absent. |\n| A much longer topic | Short decision |\n";
        for width in [360.0, 500.0, 700.0] {
            for size in [14.0, 24.0, 32.0] {
                let ctx = egui::Context::default();
                let mut cache = CommonMarkCache::default();
                for _ in 0..3 {
                    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                        ui.set_width(width);
                        ui.style_mut().override_font_id = Some(egui::FontId::proportional(size));
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                        let (response, report) =
                            CommonMarkViewer::new().show_with_report(ui, &mut cache, text);
                        assert!(response.response.rect.width() <= width + 1.0);
                        let decision = report
                            .runs
                            .iter()
                            .find(|r| r.galley.text().starts_with("QUAKE_BASEDIR"))
                            .unwrap();
                        assert!(decision.galley.rows.len() > 1);
                        assert!(decision.galley.text().ends_with("skip when absent."));
                        let following = report
                            .runs
                            .iter()
                            .find(|r| r.galley.text() == "Short decision")
                            .unwrap();
                        assert!(following.pos.y >= decision.pos.y + decision.galley.size().y);
                        let topic = report
                            .runs
                            .iter()
                            .find(|r| r.galley.text() == "Topic")
                            .unwrap();
                        let header = report
                            .runs
                            .iter()
                            .find(|r| r.galley.text() == "Decision")
                            .unwrap();
                        for run in &report.runs {
                            let second = run.pos.x >= header.pos.x - 1.0;
                            let limit = if second {
                                header.pos.x + header.pos.x - topic.pos.x
                            } else {
                                header.pos.x
                            };
                            for row in &run.galley.rows {
                                for glyph in &row.glyphs {
                                    if !glyph.chr.is_whitespace() {
                                        assert!(
                                            run.pos.x + row.pos.x + glyph.max_x() <= limit + 1.0
                                        );
                                    }
                                }
                            }
                        }
                    });
                    output.textures_delta.clear();
                }
            }
        }
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
    fn blocks_inside_list_items_start_on_their_own_lines() {
        let text = "- Intro\n  continues:\n\n  | a | b |\n  |---|---|\n  | 1 | 2 |\n\n  Second paragraph.\n\n  ```\n  code\n  ```\n\n  Third.\n- Next item\n";
        for gap in [None, Some(10.0)] {
            let ctx = egui::Context::default();
            let mut cache = CommonMarkCache::default();
            for pass in 0..2 {
                let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        let mut viewer = CommonMarkViewer::new().line_height(20.0);
                        if let Some(gap) = gap {
                            viewer = viewer.paragraph_spacing(gap);
                        }
                        let (_, report) = viewer.show_with_report(ui, &mut cache, text);
                        let find =
                            |t: &str| report.runs.iter().find(|r| r.galley.text() == t).unwrap();
                        let below = |upper: &str, lower: &str| {
                            let (upper, lower) = (find(upper), find(lower));
                            assert!(
                                lower.pos.y >= upper.pos.y + upper.galley.size().y,
                                "{:?} should be below {:?}",
                                lower.galley.text(),
                                upper.galley.text()
                            );
                        };
                        if pass == 1 {
                            below("continues:", "a");
                            below("1", "Second paragraph.");
                            below("Second paragraph.", "code");
                            below("code", "Third.");
                            below("Third.", "Next item");
                            // The table starts at the item's indent, not after the text.
                            assert!(find("a").pos.x < find("Intro").pos.x + 20.0);
                        }
                    });
                });
                output.textures_delta.clear();
            }
        }
    }
    fn frames(reader: &mut Reader, ctx: &egui::Context, events: Vec<egui::Event>, count: usize) {
        let mut frame = eframe::Frame::_new_kittest();
        let mut events = Some(events);
        for _ in 0..count {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 700.0),
                )),
                events: events.take().unwrap_or_default(),
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| eframe::App::ui(reader, ui, &mut frame));
            output.textures_delta.clear();
        }
    }
    #[test]
    fn links_back_and_forward_restore_documents_and_positions() {
        let dir = std::env::temp_dir().join(format!("lf-history-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let filler = "Filler paragraph.\n\n".repeat(80);
        std::fs::write(dir.join("a.md"), format!("# A\n\n{filler}")).unwrap();
        std::fs::write(
            dir.join("b.md"),
            format!("# B\n\n{filler}## Second part\n\nEnd.\n"),
        )
        .unwrap();
        let (a, b) = (
            dir.join("a.md").canonicalize().unwrap(),
            dir.join("b.md").canonicalize().unwrap(),
        );
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut reader = Reader::new(&cc, Some(a.clone()));
        frames(&mut reader, &ctx, Vec::new(), 3);
        reader.restore = Some((egui::vec2(0.0, 400.0), 0));
        frames(&mut reader, &ctx, Vec::new(), 4);
        assert!((reader.scroll.y - 400.0).abs() < 1.0, "{:?}", reader.scroll);

        reader.follow(
            attachments::Jump {
                path: Some(dir.join("b.md")),
                fragment: Some("second-part".into()),
            },
            &ctx,
        );
        frames(&mut reader, &ctx, Vec::new(), 4);
        let heading = reader.scroll.y;
        assert_eq!(reader.document.as_ref().unwrap().path, b);
        assert!(heading > 1000.0, "{heading}");
        assert!(reader.search.heading.is_some());
        assert_eq!(reader.opened, [b.clone(), a.clone()]);

        let key = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        frames(
            &mut reader,
            &ctx,
            vec![key(egui::Key::O, egui::Modifiers::CTRL)],
            5,
        );
        assert_eq!(reader.document.as_ref().unwrap().path, a);
        assert!((reader.scroll.y - 400.0).abs() < 1.0, "{:?}", reader.scroll);

        frames(
            &mut reader,
            &ctx,
            vec![key(egui::Key::I, egui::Modifiers::CTRL)],
            5,
        );
        assert_eq!(reader.document.as_ref().unwrap().path, b);
        assert!(
            (reader.scroll.y - heading).abs() < 1.0,
            "{:?}",
            reader.scroll
        );

        // An in-document heading jump is a step of its own.
        reader.follow(
            attachments::Jump {
                path: None,
                fragment: Some("B".into()),
            },
            &ctx,
        );
        frames(&mut reader, &ctx, Vec::new(), 4);
        assert!(
            reader.scroll.y < 60.0 && reader.error.is_none(),
            "{:?}",
            reader.scroll
        );
        frames(
            &mut reader,
            &ctx,
            vec![key(egui::Key::ArrowLeft, egui::Modifiers::ALT)],
            5,
        );
        assert_eq!(reader.document.as_ref().unwrap().path, b);
        assert!(
            (reader.scroll.y - heading).abs() < 1.0,
            "{:?}",
            reader.scroll
        );

        // Sidebar shortcuts expand and focus tabs without touching the document.
        let back_len = reader.opened.len();
        frames(
            &mut reader,
            &ctx,
            vec![key(egui::Key::E, egui::Modifiers::CTRL)],
            1,
        );
        let layout = reader.preferences.sidebar_layout;
        assert!(layout.expanded && layout.tab == sidebar::Tab::Files && reader.sidebar.focused);
        frames(&mut reader, &ctx, vec![egui::Event::Text("`".into())], 1);
        assert_eq!(reader.preferences.sidebar_layout.tab, sidebar::Tab::Opened);
        // While focused, j moves the sidebar selection, not the reading cursor or scroll.
        let scroll = reader.scroll;
        frames(
            &mut reader,
            &ctx,
            vec![key(egui::Key::J, egui::Modifiers::NONE)],
            2,
        );
        assert_eq!(reader.scroll, scroll);
        // Enter opens the selected document (a.md, below the current b.md) and
        // returns focus to the document.
        frames(
            &mut reader,
            &ctx,
            vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
            3,
        );
        assert_eq!(reader.document.as_ref().unwrap().path, a);
        assert!(!reader.sidebar.focused);
        assert_eq!(reader.opened.len(), back_len);
        // Escape on a focused sidebar returns focus; pressing ` while focused collapses.
        frames(&mut reader, &ctx, vec![egui::Event::Text("`".into())], 1);
        frames(
            &mut reader,
            &ctx,
            vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
            1,
        );
        assert!(!reader.sidebar.focused && reader.preferences.sidebar_layout.expanded);
        frames(&mut reader, &ctx, vec![egui::Event::Text("`".into())], 1);
        frames(&mut reader, &ctx, vec![egui::Event::Text("`".into())], 1);
        assert!(!reader.preferences.sidebar_layout.expanded);
        std::fs::remove_dir_all(&dir).unwrap();
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
