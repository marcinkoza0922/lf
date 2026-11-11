use eframe::egui;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

#[derive(Default)]
struct Scan {
    files: Vec<PathBuf>,
    skipped: usize,
}

fn scan(root: &Path, cancelled: &AtomicBool) -> Scan {
    let mut result = Scan::default();
    let mut directories = vec![root.to_path_buf()];
    while let Some(directory) = directories.pop() {
        if cancelled.load(Ordering::Relaxed) {
            break;
        }
        let entries = match fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(_) => {
                result.skipped += 1;
                continue;
            }
        };
        for entry in entries {
            if cancelled.load(Ordering::Relaxed) {
                return result;
            }
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    result.skipped += 1;
                    continue;
                }
            };
            let kind = match entry.file_type() {
                Ok(kind) => kind,
                Err(_) => {
                    result.skipped += 1;
                    continue;
                }
            };
            let path = entry.path();
            if kind.is_dir() {
                directories.push(path);
            } else if (kind.is_file() || (kind.is_symlink() && path.is_file()))
                && path.extension().is_some_and(|ext| {
                    ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("txt")
                })
            {
                result
                    .files
                    .push(path.strip_prefix(root).unwrap_or(&path).to_path_buf());
            }
        }
    }
    result.files.sort();
    result
}

pub struct QuickSelect {
    root: PathBuf,
    result: Option<Scan>,
    receiver: mpsc::Receiver<Scan>,
    cancelled: Arc<AtomicBool>,
    selected: usize,
    scroll_to_selection: bool,
    query: String,
    filtered_query: Option<String>,
    matches: Vec<usize>,
    focus_search: bool,
}

pub enum Action {
    Stay,
    Close,
    Open(PathBuf),
}

impl QuickSelect {
    pub fn new(root: PathBuf, ctx: &egui::Context) -> Self {
        let (sender, receiver) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = cancelled.clone();
        let worker_root = root.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = scan(&worker_root, &worker_cancelled);
            if !worker_cancelled.load(Ordering::Relaxed) {
                let _ = sender.send(result);
                ctx.request_repaint();
            }
        });
        Self {
            root,
            result: None,
            receiver,
            cancelled,
            selected: 0,
            scroll_to_selection: true,
            query: String::new(),
            filtered_query: None,
            matches: Vec::new(),
            focus_search: true,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Action {
        if self.result.is_none() {
            match self.receiver.try_recv() {
                Ok(result) => self.result = Some(result),
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.result = Some(Scan {
                        skipped: 1,
                        ..Default::default()
                    });
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        let mut action = Action::Stay;
        egui::Modal::new(egui::Id::new("quick-select")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 64.0).clamp(240.0, 720.0));
            ui.heading("Quick select");
            ui.label(self.root.display().to_string());
            ui.small("Type to find · Tab / Shift+Tab to select · Enter to open · Esc to close");
            ui.separator();
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                action = Action::Close;
            }
            let previous = ctx.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, egui::Key::Tab));
            let next = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Tab));
            let open = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
            let search_id = ui.make_persistent_id("quick-select-query");
            if !self.focus_search {
                ui.memory_mut(|memory| memory.request_focus(search_id));
            }
            let search = ui.add(
                egui::TextEdit::singleline(&mut self.query)
                    .id(search_id)
                    .lock_focus(true)
                    .hint_text("Find a Markdown file…")
                    .desired_width(f32::INFINITY),
            );
            if self.focus_search || previous || next {
                search.request_focus();
                self.focus_search = false;
            }
            let Some(result) = &self.result else {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Finding Markdown files…");
                });
                if ui.button("Cancel").clicked() {
                    action = Action::Close;
                }
                return;
            };
            if self.filtered_query.as_deref() != Some(&self.query) {
                self.matches = fuzzy_matches(&result.files, &self.query);
                self.filtered_query = Some(self.query.clone());
                self.selected = 0;
                self.scroll_to_selection = true;
            }
            if previous || next {
                self.selected = move_selection(self.selected, self.matches.len(), next);
                self.scroll_to_selection = true;
            }
            if open && !self.matches.is_empty() && matches!(action, Action::Stay) {
                action = Action::Open(self.root.join(&result.files[self.matches[self.selected]]));
            }
            if result.skipped > 0 {
                ui.label(format!(
                    "Could not read {} entries or directories.",
                    result.skipped
                ));
            }
            if result.files.is_empty() {
                ui.label("No .md or .txt files found.");
            } else if self.matches.is_empty() {
                ui.label("No matching files.");
            } else {
                ui.small(format!(
                    "{} of {} Markdown files",
                    self.matches.len(),
                    result.files.len()
                ));
                let row_height = ui
                    .text_style_height(&egui::TextStyle::Body)
                    .max(ui.spacing().interact_size.y);
                let row_stride = row_height + ui.spacing().item_spacing.y;
                let max_height = (ctx.content_rect().height() * 0.55).max(100.0);
                let mut scroll = egui::ScrollArea::vertical()
                    .id_salt("quick-select-files")
                    .max_height(max_height);
                if self.scroll_to_selection {
                    scroll = scroll.vertical_scroll_offset(
                        (self.selected as f32 * row_stride - max_height / 2.0).max(0.0),
                    );
                    self.scroll_to_selection = false;
                }
                scroll.show_rows(ui, row_height, self.matches.len(), |ui, rows| {
                    for index in rows {
                        let path = &result.files[self.matches[index]];
                        if ui
                            .add_sized(
                                [ui.available_width(), row_height],
                                egui::Button::selectable(
                                    self.selected == index,
                                    path.to_string_lossy(),
                                )
                                .truncate(),
                            )
                            .on_hover_text(path.display().to_string())
                            .clicked()
                        {
                            action = Action::Open(self.root.join(path));
                        }
                    }
                });
            }
            if ui.button("Close").clicked() {
                action = Action::Close;
            }
        });
        action
    }
}

impl Drop for QuickSelect {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

fn move_selection(selected: usize, count: usize, forward: bool) -> usize {
    if count == 0 {
        return 0;
    }
    if forward {
        selected.saturating_add(1).min(count - 1)
    } else {
        selected.saturating_sub(1)
    }
}

fn fuzzy_matches(files: &[PathBuf], query: &str) -> Vec<usize> {
    let query: Vec<char> = query.trim().to_lowercase().chars().collect();
    if query.is_empty() {
        return (0..files.len()).collect();
    }
    let mut matches: Vec<_> = files
        .iter()
        .enumerate()
        .filter_map(|(index, path)| {
            fuzzy_score(&path.to_string_lossy(), &query).map(|score| (index, score))
        })
        .collect();
    matches.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| files[a.0].cmp(&files[b.0])));
    matches.into_iter().map(|(index, _)| index).collect()
}

fn fuzzy_score(path: &str, query: &[char]) -> Option<i64> {
    let text: Vec<char> = path.to_lowercase().chars().collect();
    // Dynamic programming keeps the best alignment, rather than greedily
    // matching an earlier directory name instead of a contiguous filename.
    let mut previous = vec![None; text.len()];
    for (q, needle) in query.iter().enumerate() {
        let mut current = vec![None; text.len()];
        let mut best_prefix: Option<i64> = None;
        for (i, ch) in text.iter().enumerate() {
            if i > 0
                && let Some(score) = previous[i - 1]
            {
                best_prefix = Some(best_prefix.map_or(score, |best| best.max(score)));
            }
            if ch != needle {
                continue;
            }
            let boundary = i == 0 || matches!(text[i - 1], '/' | '-' | '_' | ' ' | '.');
            let bonus = 10 + if boundary { 18 } else { 0 };
            if q == 0 {
                current[i] = Some(bonus - i as i64);
            } else if let Some(best) = best_prefix {
                let contiguous = if i > 0 {
                    previous[i - 1].map(|score| score + 15)
                } else {
                    None
                };
                current[i] = Some(contiguous.unwrap_or(best).max(best) + bonus);
            }
        }
        previous = current;
    }
    previous
        .into_iter()
        .flatten()
        .max()
        .map(|score| score - text.len() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recursively_lists_markdown_and_avoids_symlink_cycles() {
        let root = std::env::temp_dir().join(format!("md-reader-select-{}", std::process::id()));
        fs::create_dir_all(root.join("nested/.hidden")).unwrap();
        for file in [
            "first.md",
            "nested/first.md",
            "nested/.hidden/UPPER.MD",
            "plain.TXT",
            "almost.md.bak",
        ] {
            fs::write(root.join(file), "# test").unwrap();
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&root, root.join("nested/cycle")).unwrap();
            std::os::unix::fs::symlink(root.join("first.md"), root.join("linked.md")).unwrap();
        }
        let result = scan(&root, &AtomicBool::new(false));
        let mut expected: Vec<PathBuf> = [
            "first.md",
            "nested/.hidden/UPPER.MD",
            "nested/first.md",
            "plain.TXT",
        ]
        .into_iter()
        .map(PathBuf::from)
        .collect();
        #[cfg(unix)]
        expected.push(PathBuf::from("linked.md"));
        expected.sort();
        assert_eq!(result.files, expected);
        assert_eq!(result.skipped, 0);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn selection_stops_at_boundaries_and_handles_empty_lists() {
        assert_eq!(move_selection(0, 0, true), 0);
        assert_eq!(move_selection(0, 4, false), 0);
        assert_eq!(move_selection(0, 4, true), 1);
        assert_eq!(move_selection(2, 4, false), 1);
        assert_eq!(move_selection(3, 4, true), 3);
    }
    #[test]
    fn keyboard_navigation_opens_selected_file_and_escape_closes() {
        let ctx = egui::Context::default();
        let (_sender, receiver) = mpsc::channel();
        let mut chooser = QuickSelect {
            root: PathBuf::from("/documents"),
            result: Some(Scan {
                files: vec!["a.md".into(), "nested/b.md".into(), "c.md".into()],
                skipped: 0,
            }),
            receiver,
            cancelled: Arc::new(AtomicBool::new(false)),
            selected: 0,
            scroll_to_selection: true,
            query: String::new(),
            filtered_query: None,
            matches: Vec::new(),
            focus_search: true,
        };
        let mut send = |events| {
            let mut action = Action::Stay;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    action = chooser.show(ui.ctx());
                },
            );
            output.textures_delta.clear();
            action
        };
        let key = |key, modifiers| {
            vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }]
        };
        send(Vec::new()); // initial search focus
        assert!(matches!(
            send(key(egui::Key::Tab, egui::Modifiers::NONE)),
            Action::Stay
        ));
        assert!(matches!(
            send(key(egui::Key::Tab, egui::Modifiers::NONE)),
            Action::Stay
        ));
        assert!(matches!(
            send(key(egui::Key::Tab, egui::Modifiers::SHIFT)),
            Action::Stay
        ));
        assert!(
            matches!(send(key(egui::Key::Enter, egui::Modifiers::NONE)), Action::Open(path) if path == PathBuf::from("/documents/nested/b.md"))
        );
        send(vec![egui::Event::Text("nb".into())]);
        assert!(
            matches!(send(key(egui::Key::Enter, egui::Modifiers::NONE)), Action::Open(path) if path == PathBuf::from("/documents/nested/b.md"))
        );
        send(vec![egui::Event::Text("xyz".into())]);
        assert!(matches!(
            send(key(egui::Key::Enter, egui::Modifiers::NONE)),
            Action::Stay
        ));
        assert!(matches!(
            send(key(egui::Key::Escape, egui::Modifiers::NONE)),
            Action::Close
        ));
    }
    #[test]
    fn fuzzy_matching_is_ordered_case_insensitive_and_ranked() {
        let files: Vec<PathBuf> = [
            "a-long-road.md",
            "archive/readme.md",
            "readme.md",
            "日本語.md",
        ]
        .into_iter()
        .map(PathBuf::from)
        .collect();
        assert_eq!(fuzzy_matches(&files, ""), vec![0, 1, 2, 3]);
        assert_eq!(fuzzy_matches(&files, "READ"), vec![2, 1]);
        assert_eq!(fuzzy_matches(&files, "rme"), vec![2, 1]);
        assert!(fuzzy_matches(&files, "zz").is_empty());
        assert_eq!(fuzzy_matches(&files, "日語"), vec![3]);
        assert!(fuzzy_matches(&files, "語日").is_empty());
    }
    #[test]
    fn missing_root_is_reported() {
        let result = scan(
            Path::new("/this-directory-does-not-exist-md-reader"),
            &AtomicBool::new(false),
        );
        assert!(result.files.is_empty());
        assert_eq!(result.skipped, 1);
    }
}
