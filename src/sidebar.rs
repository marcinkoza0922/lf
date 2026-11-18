//! Collapsible left sidebar: documents opened this session plus recent files, and a
//! file explorer of readable documents under the launch directory. Collapsed, it is
//! a narrow rail of tab icons. When focused, it takes keyboard navigation.
use crate::quick_select::{Scan, scan};
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Tab {
    Opened,
    Files,
}
impl Tab {
    fn other(self) -> Self {
        match self {
            Self::Opened => Self::Files,
            Self::Files => Self::Opened,
        }
    }
    fn index(self) -> usize {
        self as usize
    }
    fn name(self) -> &'static str {
        match self {
            Self::Opened => "Opened",
            Self::Files => "Files",
        }
    }
    fn hint(self) -> &'static str {
        match self {
            Self::Opened => "Opened · ` · this session and recent documents",
            Self::Files => "Files · Ctrl+E · documents under the launch directory",
        }
    }
}

/// Persisted sidebar layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Layout {
    pub expanded: bool,
    pub tab: Tab,
}
impl Default for Layout {
    fn default() -> Self {
        Self {
            expanded: false,
            tab: Tab::Opened,
        }
    }
}

/// What the reader should do after the sidebar handled input.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    /// Open a document; `focus_document` moves keyboard focus to it.
    Open {
        path: PathBuf,
        focus_document: bool,
    },
    Expand(Tab),
    Collapse,
}

pub struct Lists<'a> {
    pub opened: &'a [PathBuf],
    pub recent: &'a [PathBuf],
    pub current: Option<&'a Path>,
    pub root: Result<&'a Path, &'a str>,
}

#[derive(Default)]
struct Dir {
    dirs: BTreeMap<OsString, Dir>,
    files: Vec<PathBuf>,
}
impl Dir {
    fn build(files: &[PathBuf]) -> Self {
        let mut root = Self::default();
        for file in files {
            let mut dir = &mut root;
            if let Some(parent) = file.parent() {
                for part in parent.iter() {
                    dir = dir.dirs.entry(part.to_os_string()).or_default();
                }
            }
            dir.files.push(file.clone());
        }
        root
    }
}

struct Explorer {
    root: PathBuf,
    receiver: mpsc::Receiver<Scan>,
    cancelled: Arc<AtomicBool>,
    tree: Option<(Dir, usize)>,
}
impl Explorer {
    fn new(root: PathBuf, ctx: &egui::Context) -> Self {
        let (sender, receiver) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let (worker_root, worker_cancelled, ctx) = (root.clone(), cancelled.clone(), ctx.clone());
        std::thread::spawn(move || {
            let result = scan(&worker_root, &worker_cancelled);
            if !worker_cancelled.load(Ordering::Relaxed) {
                let _ = sender.send(result);
                ctx.request_repaint();
            }
        });
        Self {
            root,
            receiver,
            cancelled,
            tree: None,
        }
    }
}
impl Drop for Explorer {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

/// One line of a sidebar list.
#[derive(Debug)]
enum Item {
    Header(&'static str),
    Note(String),
    Dir {
        path: PathBuf,
        name: String,
        depth: usize,
        open: bool,
    },
    File {
        path: PathBuf,
        name: String,
        folder: Option<String>,
        depth: usize,
        enabled: bool,
    },
}
impl Item {
    /// The navigable identity of this line, if the keyboard can select it.
    fn key(&self) -> Option<&Path> {
        match self {
            Self::Dir { path, .. }
            | Self::File {
                path,
                enabled: true,
                ..
            } => Some(path),
            _ => None,
        }
    }
}

#[derive(Default)]
pub struct Sidebar {
    /// Whether the sidebar has keyboard focus instead of the document.
    pub focused: bool,
    explorer: Option<Explorer>,
    /// Expanded explorer directories (absolute paths).
    expanded: HashSet<PathBuf>,
    /// The document the explorer last revealed.
    revealed: Option<PathBuf>,
    /// Keyboard selection per tab.
    cursor: [Option<PathBuf>; 2],
    /// Scroll the keyboard selection (or revealed document) into view next render.
    scroll_to: Option<PathBuf>,
}

impl Sidebar {
    /// Handle this tab's shortcut: expand and focus it, or collapse it when it is
    /// already focused.
    pub fn shortcut(&mut self, layout: &mut Layout, tab: Tab) {
        if layout.expanded && layout.tab == tab && self.focused {
            layout.expanded = false;
            self.focused = false;
        } else {
            layout.expanded = true;
            layout.tab = tab;
            self.focused = true;
        }
    }
    /// Stop any explorer scan; showing Files again rescans.
    pub fn hide(&mut self) {
        self.explorer = None;
    }

    /// Keyboard navigation while focused. Call before document shortcuts.
    pub fn keys(
        &mut self,
        ctx: &egui::Context,
        layout: &mut Layout,
        lists: &Lists,
    ) -> Option<Action> {
        if !self.focused || !layout.expanded {
            self.focused = false;
            return None;
        }
        use egui::{Key, Modifiers};
        let pressed = |key| ctx.input_mut(|i| i.consume_key(Modifiers::NONE, key));
        if pressed(Key::Escape) {
            self.focused = false;
            return None;
        }
        if ctx.input_mut(|i| {
            i.consume_key(Modifiers::SHIFT, Key::Tab) | i.consume_key(Modifiers::NONE, Key::Tab)
        }) {
            layout.tab = layout.tab.other();
            return None;
        }
        let items = self.items(ctx, layout.tab, lists);
        let keys: Vec<&Item> = items.iter().filter(|i| i.key().is_some()).collect();
        if keys.is_empty() {
            return None;
        }
        let cursor = &mut self.cursor[layout.tab.index()];
        let selected = cursor
            .as_deref()
            .or(lists.current)
            .and_then(|c| keys.iter().position(|i| i.key() == Some(c)))
            .unwrap_or(0);
        let mut target = selected;
        let mut action = None;
        let item = keys[selected];
        if pressed(Key::J) | pressed(Key::ArrowDown) {
            target = (selected + 1).min(keys.len() - 1);
        } else if pressed(Key::K) | pressed(Key::ArrowUp) {
            target = selected.saturating_sub(1);
        } else if pressed(Key::Home) {
            target = 0;
        } else if pressed(Key::End) {
            target = keys.len() - 1;
        } else if pressed(Key::Enter) {
            match item {
                Item::Dir { path, .. } => toggle(&mut self.expanded, path),
                Item::File { path, .. } => {
                    self.focused = false;
                    action = Some(Action::Open {
                        path: path.clone(),
                        focus_document: true,
                    });
                }
                _ => {}
            }
        } else if pressed(Key::L) | pressed(Key::ArrowRight) {
            match item {
                Item::Dir {
                    path, open: false, ..
                } => {
                    self.expanded.insert(path.clone());
                }
                Item::Dir { open: true, .. } => target = (selected + 1).min(keys.len() - 1),
                Item::File { path, .. } => {
                    action = Some(Action::Open {
                        path: path.clone(),
                        focus_document: false,
                    })
                }
                _ => {}
            }
        } else if pressed(Key::H) | pressed(Key::ArrowLeft) {
            match item {
                Item::Dir {
                    path, open: true, ..
                } => {
                    self.expanded.remove(path);
                }
                Item::Dir { path, .. } | Item::File { path, .. } => {
                    if let Some(parent) = path
                        .parent()
                        .and_then(|p| keys.iter().position(|i| i.key() == Some(p)))
                    {
                        target = parent;
                    }
                }
                _ => {}
            }
        }
        let key = keys[target].key().map(Path::to_path_buf);
        if target != selected || cursor.is_none() {
            self.scroll_to.clone_from(&key);
        }
        *cursor = key;
        action
    }

    /// Show the sidebar content: the rail when collapsed, the lists when expanded.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        layout: Layout,
        expanded: bool,
        lists: &Lists,
    ) -> Option<Action> {
        if !expanded {
            self.hide();
            return rail(ui, layout.tab);
        }
        let mut action = None;
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            for tab in [Tab::Opened, Tab::Files] {
                if ui
                    .selectable_label(layout.tab == tab, tab.name())
                    .on_hover_text(tab.hint())
                    .clicked()
                {
                    action = Some(Action::Expand(tab));
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if chevron_button(ui, true)
                    .on_hover_text("Collapse sidebar")
                    .clicked()
                {
                    action = Some(Action::Collapse);
                }
            });
        });
        if self.focused {
            // A thin accent under the tabs shows that the sidebar has keyboard focus.
            let y = ui.cursor().top() + 1.0;
            let x = ui.max_rect().x_range();
            ui.painter()
                .hline(x, y, egui::Stroke::new(2.0, ui.visuals().hyperlink_color));
        }
        ui.separator();
        if layout.tab == Tab::Files
            && let Ok(root) = lists.root
        {
            ui.horizontal(|ui| {
                let name = root.file_name().map_or_else(
                    || root.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                );
                ui.strong(name).on_hover_text(root.display().to_string());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .small_button("Refresh")
                        .on_hover_text("Scan the launch directory again")
                        .clicked()
                    {
                        self.explorer = None;
                    }
                });
            });
        }
        let items = self.items(ui.ctx(), layout.tab, lists);
        let cursor = self.cursor[layout.tab.index()].clone();
        let scroll_to = self.scroll_to.take();
        egui::ScrollArea::vertical()
            .id_salt(("sidebar", layout.tab))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for item in &items {
                    let response = match item {
                        Item::Header(text) => {
                            ui.add_space(4.0);
                            ui.weak(*text);
                            continue;
                        }
                        Item::Note(text) => {
                            ui.weak(text);
                            continue;
                        }
                        _ => row(
                            ui,
                            item,
                            lists.current,
                            self.focused && cursor.as_deref() == item.key(),
                        ),
                    };
                    if scroll_to.is_some() && scroll_to.as_deref() == item.key() {
                        response.scroll_to_me(Some(egui::Align::Center));
                    }
                    if response.clicked() {
                        self.cursor[layout.tab.index()] = item.key().map(Path::to_path_buf);
                        match item {
                            Item::Dir { path, .. } => toggle(&mut self.expanded, path),
                            Item::File {
                                path,
                                enabled: true,
                                ..
                            } => {
                                action = Some(Action::Open {
                                    path: path.clone(),
                                    focus_document: true,
                                });
                            }
                            _ => {}
                        }
                    }
                }
            });
        action
    }

    /// Flatten the tab's content into lines. Reveals the current document in the
    /// explorer (expanding its folders) whenever it changes.
    fn items(&mut self, ctx: &egui::Context, tab: Tab, lists: &Lists) -> Vec<Item> {
        let mut items = Vec::new();
        match tab {
            Tab::Opened => {
                items.push(Item::Header("This session"));
                if lists.opened.is_empty() {
                    items.push(Item::Note("Nothing opened yet.".into()));
                }
                for path in lists.opened {
                    items.push(file_item(path, lists.root.ok(), 0, true));
                }
                let recent: Vec<_> = lists
                    .recent
                    .iter()
                    .filter(|path| !lists.opened.contains(path))
                    .collect();
                if !recent.is_empty() {
                    items.push(Item::Header("Recent"));
                    for path in recent {
                        items.push(file_item(path, lists.root.ok(), 0, path.is_file()));
                    }
                }
            }
            Tab::Files => {
                let root = match lists.root {
                    Ok(root) => root,
                    Err(error) => {
                        items.push(Item::Note(format!(
                            "Could not find launch directory: {error}"
                        )));
                        return items;
                    }
                };
                let explorer = match &mut self.explorer {
                    Some(explorer) if explorer.root == root => explorer,
                    _ => {
                        self.revealed = None;
                        self.explorer.insert(Explorer::new(root.to_path_buf(), ctx))
                    }
                };
                if explorer.tree.is_none()
                    && let Ok(scan) = explorer.receiver.try_recv()
                {
                    explorer.tree = Some((Dir::build(&scan.files), scan.skipped));
                }
                let Some((tree, skipped)) = &explorer.tree else {
                    items.push(Item::Note("Scanning…".into()));
                    return items;
                };
                if let Some(current) = lists.current
                    && self.revealed.as_deref() != Some(current)
                    && current.starts_with(root)
                {
                    for ancestor in current.ancestors().skip(1) {
                        if ancestor == root {
                            break;
                        }
                        self.expanded.insert(ancestor.to_path_buf());
                    }
                    self.revealed = Some(current.to_path_buf());
                    self.cursor[Tab::Files.index()] = Some(current.to_path_buf());
                    self.scroll_to = Some(current.to_path_buf());
                }
                if *skipped > 0 {
                    items.push(Item::Note(format!("{skipped} unreadable entries skipped")));
                }
                if tree.dirs.is_empty() && tree.files.is_empty() {
                    items.push(Item::Note("No Markdown or text files here.".into()));
                }
                flatten(tree, root, root, 0, &self.expanded, &mut items);
            }
        }
        items
    }
}

fn toggle(set: &mut HashSet<PathBuf>, path: &Path) {
    if !set.remove(path) {
        set.insert(path.to_path_buf());
    }
}

fn file_item(path: &Path, root: Option<&Path>, depth: usize, enabled: bool) -> Item {
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let folder = path.parent().map(|parent| match root {
        Some(root) if parent.starts_with(root) => {
            let relative = parent.strip_prefix(root).unwrap_or(parent);
            if relative.as_os_str().is_empty() {
                ".".to_string()
            } else {
                relative.display().to_string()
            }
        }
        _ => parent.display().to_string(),
    });
    Item::File {
        path: path.to_path_buf(),
        name,
        folder,
        depth,
        enabled,
    }
}

fn flatten(
    dir: &Dir,
    root: &Path,
    path: &Path,
    depth: usize,
    expanded: &HashSet<PathBuf>,
    items: &mut Vec<Item>,
) {
    for (name, child) in &dir.dirs {
        let path = path.join(name);
        let open = expanded.contains(&path);
        items.push(Item::Dir {
            path: path.clone(),
            name: name.to_string_lossy().into_owned(),
            depth,
            open,
        });
        if open {
            flatten(child, root, &path, depth + 1, expanded, items);
        }
    }
    for file in &dir.files {
        let mut item = file_item(&root.join(file), None, depth, true);
        if let Item::File { folder, .. } = &mut item {
            // The tree already shows the folder.
            *folder = None;
        }
        items.push(item);
    }
}

/// A list line: indentation, a disclosure triangle for folders, the name, and for
/// files outside the tree their folder in weak text.
fn row(ui: &mut egui::Ui, item: &Item, current: Option<&Path>, cursor: bool) -> egui::Response {
    let height = ui.spacing().interact_size.y;
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::click(),
    );
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let visuals = ui.visuals().clone();
    let painter = ui.painter();
    let (depth, enabled) = match item {
        Item::Dir { depth, .. } => (*depth, true),
        Item::File { depth, enabled, .. } => (*depth, *enabled),
        _ => (0, false),
    };
    let is_current = matches!(item, Item::File { path, .. } if Some(path.as_path()) == current);
    if is_current {
        painter.rect_filled(rect, 3.0, visuals.selection.bg_fill);
    } else if response.hovered() && enabled {
        painter.rect_filled(rect, 3.0, visuals.widgets.hovered.weak_bg_fill);
    }
    if cursor {
        painter.rect_stroke(
            rect.shrink(1.0),
            3.0,
            egui::Stroke::new(1.5, visuals.hyperlink_color),
            egui::StrokeKind::Inside,
        );
    }
    let indent = height * 0.7;
    let left = rect.left() + 4.0 + depth as f32 * indent;
    let text_color = if enabled {
        visuals.text_color()
    } else {
        visuals.weak_text_color()
    };
    let triangle = egui::Rect::from_center_size(
        egui::pos2(left + indent * 0.4, rect.center().y),
        egui::vec2(1.0, 1.0) * height * 0.3,
    );
    let mut job = egui::text::LayoutJob::default();
    let body = egui::TextStyle::Body.resolve(ui.style());
    match item {
        Item::Dir { name, open, .. } => {
            paint_triangle(painter, triangle, *open, visuals.weak_text_color());
            job.append(name, 0.0, egui::TextFormat::simple(body, text_color));
        }
        Item::File { name, folder, .. } => {
            job.append(name, 0.0, egui::TextFormat::simple(body, text_color));
            if let Some(folder) = folder {
                let small = egui::TextStyle::Small.resolve(ui.style());
                let format = egui::TextFormat::simple(small, visuals.weak_text_color());
                job.append(folder, 8.0, format);
            }
        }
        _ => {}
    }
    let text_left = left + indent;
    job.wrap.max_width = (rect.right() - text_left - 4.0).max(1.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let pos = egui::pos2(text_left, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(pos, galley, text_color);
    let hover = match item {
        Item::File { path, enabled, .. } if !enabled => format!("{} (missing)", path.display()),
        Item::File { path, .. } | Item::Dir { path, .. } => path.display().to_string(),
        _ => String::new(),
    };
    response.on_hover_text(hover)
}

/// The collapsed sidebar: an expand chevron and one icon per tab.
fn rail(ui: &mut egui::Ui, selected: Tab) -> Option<Action> {
    let mut action = None;
    ui.add_space(4.0);
    ui.vertical_centered(|ui| {
        if chevron_button(ui, false)
            .on_hover_text("Expand sidebar")
            .clicked()
        {
            action = Some(Action::Expand(selected));
        }
        ui.add_space(6.0);
        for tab in [Tab::Opened, Tab::Files] {
            if icon_button(ui, tab, tab == selected)
                .on_hover_text(tab.hint())
                .clicked()
            {
                action = Some(Action::Expand(tab));
            }
        }
    });
    action
}

fn square_button(ui: &mut egui::Ui, selected: bool) -> (egui::Response, egui::Color32) {
    let size = ui.spacing().interact_size.y * 1.2;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::click());
    let visuals = ui.style().interact_selectable(&response, selected);
    if selected || response.hovered() {
        ui.painter().rect_filled(rect, 4.0, visuals.weak_bg_fill);
    }
    (response, visuals.fg_stroke.color)
}

fn chevron_button(ui: &mut egui::Ui, collapse: bool) -> egui::Response {
    let (response, color) = square_button(ui, false);
    let r = egui::Rect::from_center_size(
        response.rect.center(),
        egui::vec2(0.5, 1.0) * response.rect.height() * 0.4,
    );
    let (tip, tail) = if collapse {
        (r.left(), r.right())
    } else {
        (r.right(), r.left())
    };
    ui.painter().add(egui::Shape::line(
        vec![
            egui::pos2(tail, r.top()),
            egui::pos2(tip, r.center().y),
            egui::pos2(tail, r.bottom()),
        ],
        egui::Stroke::new(1.5, color),
    ));
    response
}

/// Tab icons painted with shapes: a clock for Opened, a folder for Files.
fn icon_button(ui: &mut egui::Ui, tab: Tab, selected: bool) -> egui::Response {
    let (response, color) = square_button(ui, selected);
    let r = response.rect.shrink(response.rect.width() * 0.28);
    let stroke = egui::Stroke::new(1.5, color);
    let painter = ui.painter();
    match tab {
        Tab::Opened => {
            painter.circle_stroke(r.center(), r.width() / 2.0, stroke);
            let c = r.center();
            painter.line_segment([c, c - egui::vec2(0.0, r.height() * 0.32)], stroke);
            painter.line_segment([c, c + egui::vec2(r.width() * 0.24, 0.0)], stroke);
        }
        Tab::Files => {
            let at =
                |x: f32, y: f32| egui::pos2(r.left() + r.width() * x, r.top() + r.height() * y);
            painter.add(egui::Shape::closed_line(
                vec![
                    at(0.0, 0.12),
                    at(0.38, 0.12),
                    at(0.5, 0.27),
                    at(1.0, 0.27),
                    at(1.0, 0.9),
                    at(0.0, 0.9),
                ],
                stroke,
            ));
        }
    }
    response
}

fn paint_triangle(painter: &egui::Painter, r: egui::Rect, open: bool, color: egui::Color32) {
    let points = if open {
        vec![r.left_top(), r.right_top(), r.center_bottom()]
    } else {
        vec![r.left_top(), r.right_center(), r.left_bottom()]
    };
    painter.add(egui::Shape::convex_polygon(
        points,
        color,
        egui::Stroke::NONE,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shortcuts_expand_focus_switch_and_collapse() {
        let (mut sidebar, mut layout) = (Sidebar::default(), Layout::default());
        sidebar.shortcut(&mut layout, Tab::Files);
        assert!(layout.expanded && sidebar.focused && layout.tab == Tab::Files);
        sidebar.shortcut(&mut layout, Tab::Opened);
        assert!(layout.expanded && sidebar.focused && layout.tab == Tab::Opened);
        sidebar.focused = false;
        sidebar.shortcut(&mut layout, Tab::Opened);
        assert!(
            layout.expanded && sidebar.focused,
            "an unfocused tab is focused first"
        );
        sidebar.shortcut(&mut layout, Tab::Opened);
        assert!(!layout.expanded && !sidebar.focused);
    }
    #[test]
    fn explorer_tree_groups_files_by_directory() {
        let files: Vec<PathBuf> = ["a.md", "docs/b.md", "docs/perf/c.md", "docs/z.txt"]
            .into_iter()
            .map(PathBuf::from)
            .collect();
        let tree = Dir::build(&files);
        assert_eq!(tree.files, [PathBuf::from("a.md")]);
        let docs = &tree.dirs[&OsString::from("docs")];
        assert_eq!(
            docs.files,
            [PathBuf::from("docs/b.md"), PathBuf::from("docs/z.txt")]
        );
        assert_eq!(
            docs.dirs[&OsString::from("perf")].files,
            [PathBuf::from("docs/perf/c.md")]
        );
    }
    fn press(
        ctx: &egui::Context,
        sidebar: &mut Sidebar,
        layout: &mut Layout,
        lists: &Lists,
        key: egui::Key,
    ) -> Option<Action> {
        let mut action = None;
        let input = egui::RawInput {
            events: vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |_| action = sidebar.keys(ctx, layout, lists));
        output.textures_delta.clear();
        action
    }
    #[test]
    fn keyboard_navigates_lists_and_the_explorer_tree() {
        let root = std::env::temp_dir().join(format!("lf-sidebar-{}", std::process::id()));
        std::fs::create_dir_all(root.join("docs/perf")).unwrap();
        for file in ["a.md", "docs/b.md", "docs/perf/c.md"] {
            std::fs::write(root.join(file), "x").unwrap();
        }
        let (a, b) = (root.join("a.md"), root.join("docs/b.md"));
        let opened = [b.clone(), a.clone()];
        let lists = Lists {
            opened: &opened,
            recent: &[],
            current: Some(&b),
            root: Ok(&root),
        };
        let ctx = egui::Context::default();
        let (mut sidebar, mut layout) = (Sidebar::default(), Layout::default());
        sidebar.shortcut(&mut layout, Tab::Opened);
        use egui::Key;
        // The selection starts at the current document.
        assert_eq!(press(&ctx, &mut sidebar, &mut layout, &lists, Key::J), None);
        assert_eq!(sidebar.cursor[0].as_deref(), Some(a.as_path()));
        assert_eq!(
            press(&ctx, &mut sidebar, &mut layout, &lists, Key::L),
            Some(Action::Open {
                path: a.clone(),
                focus_document: false
            })
        );
        assert!(sidebar.focused);
        press(&ctx, &mut sidebar, &mut layout, &lists, Key::Tab);
        assert_eq!(layout.tab, Tab::Files);
        // Wait for the scan; the current document's folder is revealed.
        for _ in 0..500 {
            press(&ctx, &mut sidebar, &mut layout, &lists, Key::F1);
            if sidebar.explorer.as_ref().is_some_and(|e| e.tree.is_some()) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        press(&ctx, &mut sidebar, &mut layout, &lists, Key::F1);
        assert!(sidebar.expanded.contains(&root.join("docs")));
        assert_eq!(sidebar.cursor[1].as_deref(), Some(b.as_path()));
        // Rows: a.md is after the docs folder; docs/ → perf/ → b.md.
        press(&ctx, &mut sidebar, &mut layout, &lists, Key::K);
        assert_eq!(
            sidebar.cursor[1].as_deref(),
            Some(root.join("docs/perf").as_path())
        );
        press(&ctx, &mut sidebar, &mut layout, &lists, Key::L);
        assert!(sidebar.expanded.contains(&root.join("docs/perf")));
        press(&ctx, &mut sidebar, &mut layout, &lists, Key::H);
        assert!(!sidebar.expanded.contains(&root.join("docs/perf")));
        press(&ctx, &mut sidebar, &mut layout, &lists, Key::H);
        assert_eq!(
            sidebar.cursor[1].as_deref(),
            Some(root.join("docs").as_path())
        );
        press(&ctx, &mut sidebar, &mut layout, &lists, Key::End);
        assert_eq!(
            press(&ctx, &mut sidebar, &mut layout, &lists, Key::Enter),
            Some(Action::Open {
                path: a.clone(),
                focus_document: true
            })
        );
        assert!(!sidebar.focused);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
