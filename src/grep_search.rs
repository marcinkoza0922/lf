use eframe::egui;
use std::{
    io::{self, BufRead, BufReader},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

#[derive(Clone, Debug)]
pub struct Hit {
    pub path: PathBuf,
    pub line: usize,
    pub text: String,
}
#[derive(Default)]
struct Results {
    hits: Vec<Hit>,
    error: Option<String>,
    truncated: bool,
}
#[derive(Default)]
struct Process {
    child: Mutex<Option<Child>>,
    cancelled: AtomicBool,
}
impl Process {
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
        if let Some(child) = self.child.lock().unwrap().as_mut() {
            let _ = child.kill();
        }
    }
}
struct Task {
    process: Arc<Process>,
    receiver: mpsc::Receiver<Results>,
}
impl Drop for Task {
    fn drop(&mut self) {
        self.process.cancel();
    }
}

fn command(tool: &str, root: &std::path::Path, query: &str) -> Command {
    let mut command = Command::new(tool);
    command
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if tool == "rg" {
        command.args([
            "--no-config",
            "--null",
            "--line-number",
            "--with-filename",
            "--no-heading",
            "--color",
            "never",
            "--fixed-strings",
            "--ignore-case",
            "--hidden",
            "--no-ignore",
            "--glob",
            "*.[mM][dD]",
            "--glob",
            "*.[tT][xX][tT]",
        ]);
    } else {
        command.args([
            "-r",
            "-n",
            "-H",
            "-Z",
            "-i",
            "-F",
            "-I",
            "--include=*.[mM][dD]",
            "--include=*.[tT][xX][tT]",
        ]);
    }
    command.args(["--", query, "."]);
    command
}

fn run(
    root: &std::path::Path,
    query: &str,
    process: &Process,
    force_tool: Option<&str>,
) -> Results {
    if query.is_empty() {
        return Results::default();
    }
    if query.contains(['\n', '\r', '\0']) {
        return Results {
            error: Some("Use a single-line search query.".into()),
            ..Default::default()
        };
    }
    let child = if let Some(tool) = force_tool {
        command(tool, root, query).spawn()
    } else {
        command("rg", root, query).spawn().or_else(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                command("grep", root, query).spawn()
            } else {
                Err(error)
            }
        })
    };
    let mut child = match child {
        Ok(child) => child,
        Err(error) => {
            return Results {
                error: Some(format!("Could not run rg/grep: {error}")),
                ..Default::default()
            };
        }
    };
    let stdout = child.stdout.take().unwrap();
    {
        let mut slot = process.child.lock().unwrap();
        *slot = Some(child);
        if process.cancelled.load(Ordering::Relaxed) {
            let _ = slot.as_mut().unwrap().kill();
        }
    }
    let mut result = Results::default();
    let mut reader = BufReader::new(stdout);
    let mut total = 0;
    loop {
        let mut path = Vec::new();
        let mut record = Vec::new();
        // Limit each record too: a maliciously long source line must not exhaust memory.
        let read = (|| -> io::Result<bool> {
            use std::io::Read;
            if reader.by_ref().take(64 * 1024).read_until(0, &mut path)? == 0 {
                return Ok(false);
            }
            if path.last() != Some(&0) {
                return Err(io::Error::other("Search filename exceeds 64 KiB."));
            }
            reader
                .by_ref()
                .take(1024 * 1024)
                .read_until(b'\n', &mut record)?;
            if record.len() >= 1024 * 1024 {
                return Err(io::Error::other("Search result line exceeds 1 MiB."));
            }
            Ok(true)
        })();
        match read {
            Ok(false) => break,
            Err(error) => {
                result.error = Some(error.to_string());
                process.cancel();
                break;
            }
            Ok(true) => {}
        }
        total += path.len() + record.len();
        if result.hits.len() >= 1000 || total > 2 * 1024 * 1024 {
            result.truncated = true;
            process.cancel();
            break;
        }
        path.pop();
        let Some(colon) = record.iter().position(|b| *b == b':') else {
            continue;
        };
        let Ok(line) = String::from_utf8_lossy(&record[..colon]).parse::<usize>() else {
            continue;
        };
        #[cfg(unix)]
        let path = {
            use std::os::unix::ffi::OsStringExt;
            PathBuf::from(std::ffi::OsString::from_vec(path))
        };
        #[cfg(not(unix))]
        let path = PathBuf::from(String::from_utf8_lossy(&path).to_string());
        let text = String::from_utf8_lossy(&record[colon + 1..])
            .trim_end_matches(['\r', '\n'])
            .to_string();
        result.hits.push(Hit { path, line, text });
    }
    if let Some(mut child) = process.child.lock().unwrap().take() {
        match child.wait() {
            Ok(status)
                if !process.cancelled.load(Ordering::Relaxed)
                    && status.code().is_none_or(|code| code > 1) =>
            {
                result.error =
                    Some("Some files could not be searched (rg/grep reported an error).".into())
            }
            Err(error) => result.error = Some(error.to_string()),
            _ => {}
        }
    }
    result
        .hits
        .sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));
    result
}

pub struct GrepSearch {
    root: PathBuf,
    query: String,
    submitted: String,
    task: Option<Task>,
    results: Results,
    selected: usize,
    focus: bool,
    scroll: bool,
}
pub enum Action {
    Stay,
    Close,
    Open {
        path: PathBuf,
        line: usize,
        query: String,
    },
}
impl GrepSearch {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            query: String::new(),
            submitted: String::new(),
            task: None,
            results: Results::default(),
            selected: 0,
            focus: true,
            scroll: true,
        }
    }
    fn start(&mut self, ctx: &egui::Context) {
        self.task = None;
        self.submitted = self.query.clone();
        self.results = Results::default();
        self.selected = 0;
        self.scroll = true;
        if self.query.is_empty() {
            return;
        }
        let process = Arc::new(Process::default());
        let worker = process.clone();
        let root = self.root.clone();
        let query = self.query.clone();
        let ctx = ctx.clone();
        let (sender, receiver) = mpsc::channel();
        self.task = Some(Task { process, receiver });
        std::thread::spawn(move || {
            let result = run(&root, &query, &worker, None);
            let _ = sender.send(result);
            ctx.request_repaint();
        });
    }
    pub fn show(&mut self, ctx: &egui::Context) -> Action {
        if let Some(task) = &self.task {
            match task.receiver.try_recv() {
                Ok(results) => {
                    self.results = results;
                    self.task = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.results.error = Some("Search worker stopped unexpectedly.".into());
                    self.task = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        let mut action = Action::Stay;
        egui::Modal::new(egui::Id::new("grep-search")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 64.0).clamp(240.0, 850.0));
            ui.heading("Search all Markdown files");
            ui.label(self.root.display().to_string());
            ui.small("Enter to search / open · Tab / Shift+Tab to select · Esc to close");
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                action = Action::Close;
            }
            let previous = ctx.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, egui::Key::Tab));
            let next = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Tab));
            let enter = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
            let id = egui::Id::new("grep-query");
            if !self.focus {
                ui.memory_mut(|m| m.request_focus(id));
            }
            let field = ui.add(
                egui::TextEdit::singleline(&mut self.query)
                    .id(id)
                    .lock_focus(true)
                    .desired_width(f32::INFINITY)
                    .hint_text("Text to find in .md and .txt files…"),
            );
            if self.focus {
                field.request_focus();
                self.focus = false;
            }
            let changed = self.query != self.submitted;
            let submit = ui.button("Search").clicked()
                || (enter && (changed || self.results.hits.is_empty()));
            if submit {
                self.start(ctx);
            }
            if previous || next {
                self.selected = if next {
                    (self.selected + 1).min(self.results.hits.len().saturating_sub(1))
                } else {
                    self.selected.saturating_sub(1)
                };
                self.scroll = true;
            }
            if enter
                && !submit
                && !changed
                && self.task.is_none()
                && matches!(action, Action::Stay)
                && let Some(hit) = self.results.hits.get(self.selected)
            {
                action = Action::Open {
                    path: self.root.join(&hit.path),
                    line: hit.line,
                    query: self.submitted.clone(),
                };
            }
            if let Some(error) = &self.results.error {
                ui.label(error);
            }
            if self.task.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Searching…");
                });
            } else if !self.submitted.is_empty() {
                ui.label(format!(
                    "{} matching lines{}",
                    self.results.hits.len(),
                    if self.results.truncated {
                        " (result limit reached)"
                    } else {
                        ""
                    }
                ));
            }
            let height = ui
                .text_style_height(&egui::TextStyle::Body)
                .max(ui.spacing().interact_size.y);
            let max_height = (ctx.content_rect().height() * 0.5).max(100.0);
            let mut scroll = egui::ScrollArea::vertical()
                .id_salt("grep-results")
                .max_height(max_height);
            if self.scroll {
                scroll = scroll.vertical_scroll_offset(
                    (self.selected as f32 * (height + ui.spacing().item_spacing.y)
                        - max_height / 2.0)
                        .max(0.0),
                );
                self.scroll = false;
            }
            scroll.show_rows(ui, height, self.results.hits.len(), |ui, rows| {
                for index in rows {
                    let hit = &self.results.hits[index];
                    let label = format!("{}:{}  {}", hit.path.display(), hit.line, hit.text);
                    if ui
                        .add_sized(
                            [ui.available_width(), height],
                            egui::Button::selectable(index == self.selected, &label).truncate(),
                        )
                        .on_hover_text(&label)
                        .clicked()
                    {
                        action = Action::Open {
                            path: self.root.join(&hit.path),
                            line: hit.line,
                            query: self.submitted.clone(),
                        };
                    }
                }
            });
            if ui.button("Close").clicked() {
                action = Action::Close;
            }
        });
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn limits_results_and_treats_queries_as_literal_arguments() {
        let root = std::env::temp_dir().join(format!("lf-grep-limit-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("many.md"), "needle\n".repeat(1005)).unwrap();
        let results = run(&root, "needle", &Process::default(), None);
        assert_eq!(results.hits.len(), 1000);
        assert!(results.truncated);
        assert!(results.error.is_none());
        std::fs::write(root.join("literal.md"), "--flag $(not-a-command) [x]\n").unwrap();
        for tool in if cfg!(windows) {
            &["rg"][..]
        } else {
            &["rg", "grep"][..]
        } {
            let results = run(
                &root,
                "--flag $(not-a-command) [x]",
                &Process::default(),
                Some(tool),
            );
            assert_eq!(results.hits.len(), 1);
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    #[cfg(unix)]
    fn rg_and_grep_search_hidden_markdown_and_parse_unusual_filenames() {
        let root = std::env::temp_dir().join(format!("lf-grep-{}", std::process::id()));
        std::fs::create_dir_all(root.join(".hidden")).unwrap();
        std::fs::write(
            root.join(".hidden/colon:new\nline.MD"),
            "first\nA Needle [literal]\n",
        )
        .unwrap();
        std::fs::write(root.join("skip.txt"), "Needle").unwrap();
        for tool in ["rg", "grep"] {
            let result = run(&root, "needle [literal]", &Process::default(), Some(tool));
            assert!(result.error.is_none(), "{tool}: {:?}", result.error);
            assert_eq!(result.hits.len(), 1, "{tool}");
            assert_eq!(result.hits[0].line, 2);
            assert_eq!(result.hits[0].text, "A Needle [literal]");
            assert!(result.hits[0].path.ends_with("colon:new\nline.MD"));
            let empty = run(&root, "absent", &Process::default(), Some(tool));
            assert!(empty.error.is_none());
            assert!(empty.hits.is_empty());
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
