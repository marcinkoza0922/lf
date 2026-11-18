use crate::playback::{Command, Playback};
use eframe::egui;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Image,
    Audio,
    Video,
    File,
}
struct Target {
    url: url::Url,
    kind: Kind,
}
fn resolve(base: &url::Url, destination: &str) -> Option<Target> {
    if destination.starts_with('#') {
        return None;
    }
    let url = base.join(destination).ok()?;
    if !matches!(url.scheme(), "file" | "https" | "http") {
        return None;
    }
    let path = if url.scheme() == "file" {
        url.to_file_path().ok()?
    } else {
        url.path().into()
    };
    let extension = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let kind = match extension.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "tiff" | "tga" | "avif" => {
            Kind::Image
        }
        "mp4" | "m4v" | "mkv" | "webm" | "mov" | "avi" | "ogv" | "mpeg" | "mpg" => Kind::Video,
        "mp3" | "m4a" | "aac" | "wav" | "flac" | "ogg" | "oga" | "opus" | "aiff" => Kind::Audio,
        _ if url.scheme() == "file" => Kind::File,
        _ => return None,
    };
    Some(Target { url, kind })
}
struct Active {
    url: String,
    player: Playback,
    texture: Option<egui::TextureHandle>,
    volume: f64,
}
#[derive(Default)]
pub struct Attachments {
    active: Option<Active>,
    open_error: Arc<Mutex<HashMap<String, String>>>,
    /// Inline media the reader expanded, keyed by source offset.
    expanded: HashSet<usize>,
    /// Line height of body text, so link chips align with the surrounding prose.
    pub line_height: Option<f32>,
    /// A document or heading link the reader followed.
    jump: Option<Jump>,
}
/// A followed link handled by the reader: another document, a heading, or both.
#[derive(Debug, PartialEq, Eq)]
pub struct Jump {
    /// None for a heading in the current document.
    pub path: Option<PathBuf>,
    /// Decoded heading anchor.
    pub fragment: Option<String>,
}
impl Attachments {
    pub fn clear(&mut self) {
        let line_height = self.line_height;
        *self = Self {
            line_height,
            ..Self::default()
        };
    }
    /// Render a link or image destination. `text` is the document source the
    /// attachment's span refers to.
    pub fn render(
        &mut self,
        ui: &mut egui::Ui,
        base: &url::Url,
        text: &str,
        attachment: &egui_commonmark::Attachment<'_>,
    ) -> bool {
        if let Some(fragment) = attachment.destination.strip_prefix('#') {
            let response = egui_commonmark::attachment_link(ui, attachment.title);
            if response.clicked() {
                self.follow(ui.ctx(), None, Some(fragment));
            }
            return true;
        }
        let Some(target) = resolve(base, attachment.destination) else {
            return false;
        };
        let path = target.url.to_file_path().ok();
        let name = path
            .as_deref()
            .and_then(Path::file_name)
            .map(|s| s.to_string_lossy().into_owned())
            .or_else(|| {
                target
                    .url
                    .path_segments()
                    .and_then(|mut segments| segments.next_back())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| target.url.to_string());
        let title = if attachment.title.trim().is_empty() {
            name.as_str()
        } else {
            attachment.title
        };
        let id = ui.next_auto_id();
        ui.push_id(id, |ui| {
            if target.kind == Kind::File {
                self.file_link(ui, &target, title, path.as_deref());
            } else if standalone(text, attachment.source.clone()) {
                self.show_media(ui, &target, title, &name);
            } else {
                // Media mentioned within other content stays collapsed until requested.
                let key = attachment.source.start;
                let expanded = self.expanded.contains(&key);
                let hint = if expanded { "Hide" } else { "Show" };
                let response = self
                    .chip(ui, target.kind, title, Some(expanded))
                    .on_hover_text(format!("{name}\n{hint} {}", noun(target.kind)));
                if response.clicked() && !self.expanded.remove(&key) {
                    self.expanded.insert(key);
                }
                if expanded {
                    egui_commonmark::attachment_label(ui, "\n");
                    self.show_media(ui, &target, title, &name);
                    egui_commonmark::attachment_label(ui, "\n");
                }
            }
        });
        true
    }
    /// A compact inline link: icon and label on a link-tinted background, plus a
    /// disclosure arrow when `expanded` is given.
    fn chip(
        &self,
        ui: &mut egui::Ui,
        kind: Kind,
        label: &str,
        expanded: Option<bool>,
    ) -> egui::Response {
        let font = egui::FontSelection::Default.resolve(ui.style());
        let font_height = ui.fonts_mut(|f| f.row_height(&font));
        let line_height = self.line_height.unwrap_or(font_height).max(font_height);
        let link = ui.visuals().hyperlink_color;
        let (padding, gap, icon) = (font_height * 0.2, font_height * 0.25, font_height * 0.7);
        let fixed = padding * 2.0 + icon + gap + expanded.map_or(0.0, |_| gap + icon * 0.6);
        // Labels never wrap inside a chip; the chip moves to the next line instead.
        let text = egui::WidgetText::from(
            egui::RichText::new(label)
                .font(font.clone())
                .color(link)
                .line_height(Some(line_height)),
        )
        .into_galley(
            ui,
            Some(egui::TextWrapMode::Truncate),
            (ui.max_rect().width() - fixed).max(1.0),
            font,
        );
        let size = egui::vec2(fixed + text.size().x, line_height);
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
        let text_pos = egui::pos2(rect.left() + padding + icon + gap, rect.top());
        egui_commonmark::attachment_record(ui, text.clone(), text_pos);
        if ui.is_rect_visible(rect) {
            // Glyphs sit at the top of rows taller than the font; frame just the glyphs.
            let band = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), font_height));
            let hovered = response.hovered() || response.has_focus();
            let painter = ui.painter();
            painter.rect_filled(
                band.expand2(egui::vec2(0.0, 1.0)),
                font_height * 0.25,
                link.gamma_multiply(if hovered { 0.22 } else { 0.12 }),
            );
            let square = |x: f32, size: f32| {
                egui::Rect::from_center_size(
                    egui::pos2(x + size / 2.0, band.center().y),
                    egui::vec2(size, size),
                )
            };
            paint_icon(painter, kind, square(rect.left() + padding, icon), link);
            if let Some(expanded) = expanded {
                let arrow = square(text_pos.x + text.size().x + gap, icon * 0.6);
                paint_arrow(painter, arrow, expanded, link.gamma_multiply(0.7));
            }
            let underline = if hovered {
                egui::Stroke::new(1.0, link)
            } else {
                egui::Stroke::NONE
            };
            painter
                .add(egui::epaint::TextShape::new(text_pos, text, link).with_underline(underline));
        }
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    }
    /// Take the link the reader followed, if any, for the reader to open.
    pub fn take_jump(&mut self) -> Option<Jump> {
        self.jump.take()
    }
    fn follow(&mut self, ctx: &egui::Context, path: Option<&Path>, fragment: Option<&str>) {
        self.jump = Some(Jump {
            path: path.map(Path::to_path_buf),
            fragment: fragment
                .filter(|f| !f.is_empty())
                .map(crate::history::decode_fragment),
        });
        ctx.request_repaint();
    }
    fn file_link(&mut self, ui: &mut egui::Ui, target: &Target, title: &str, path: Option<&Path>) {
        let location = path.map_or_else(|| target.url.to_string(), |p| p.display().to_string());
        let document = path.filter(|path| is_document(path));
        let action = if document.is_some() {
            "Open in Leaf · right-click for more"
        } else {
            "Open in default app"
        };
        let response = self
            .chip(ui, target.kind, title, None)
            .on_hover_text(format!("{location}\n{action}"));
        if response.clicked() {
            match document {
                Some(document) => self.follow(ui.ctx(), Some(document), target.url.fragment()),
                None => self.open_external(ui.ctx(), &target.url),
            }
        }
        response.context_menu(|ui| {
            if let Some(document) = document
                && ui.button("Open in Leaf").clicked()
            {
                self.follow(ui.ctx(), Some(document), target.url.fragment());
                ui.close();
            }
            if ui.button("Open in default app").clicked() {
                self.open_external(ui.ctx(), &target.url);
                ui.close();
            }
        });
        if let Some(error) = self.open_error.lock().unwrap().get(target.url.as_str()) {
            ui.colored_label(ui.visuals().error_fg_color, " ⚠")
                .on_hover_text(error);
        }
    }
    fn open_external(&self, ctx: &egui::Context, target: &url::Url) {
        let error = self.open_error.clone();
        let ctx = ctx.clone();
        let target = target.clone();
        std::thread::spawn(move || {
            let result = open_file(&target);
            let mut errors = error.lock().unwrap();
            if let Err(message) = result {
                errors.insert(target.to_string(), message);
            } else {
                errors.remove(target.as_str());
            }
            drop(errors);
            ctx.request_repaint();
        });
    }
    fn show_media(&mut self, ui: &mut egui::Ui, target: &Target, title: &str, name: &str) {
        if target.kind == Kind::Image {
            ui.add(
                egui::Image::from_uri(target.url.as_str())
                    .max_width(ui.available_width())
                    .fit_to_original_size(1.0),
            )
            .on_hover_text(title);
            return;
        }
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_max_width(ui.available_width().min(640.0));
            ui.vertical(|ui| {
                ui.horizontal_wrapped(|ui| {
                    let size = ui.text_style_height(&egui::TextStyle::Body);
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(size * 1.2, size), egui::Sense::hover());
                    let icon = egui::Rect::from_min_size(rect.min, egui::vec2(size, size))
                        .shrink(size * 0.15);
                    paint_icon(ui.painter(), target.kind, icon, ui.visuals().text_color());
                    egui_commonmark::attachment_label(ui, egui::RichText::new(title).strong());
                });
                ui.weak(name).on_hover_text(target.url.as_str());
                self.media(ui, target);
            });
        });
    }
    fn media(&mut self, ui: &mut egui::Ui, target: &Target) {
        let is_active = self
            .active
            .as_ref()
            .is_some_and(|active| active.url == target.url.as_str());
        if !is_active {
            if target.kind == Kind::Video {
                let size = egui::vec2(ui.available_width().clamp(1.0, 640.0), 180.0);
                let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                ui.painter().rect_filled(rect, 4.0, egui::Color32::BLACK);
            }
            if ui.button("▶ Play").clicked() {
                let player = Playback::start(
                    target.url.to_string(),
                    target.kind == Kind::Video,
                    ui.ctx().clone(),
                );
                self.active = Some(Active {
                    url: target.url.to_string(),
                    player,
                    texture: None,
                    volume: 100.0,
                });
            }
            return;
        }
        let active = self.active.as_mut().unwrap();
        let mut state = active.player.state.lock().unwrap();
        if let Some(frame) = state.frame.take() {
            if let Some(texture) = &mut active.texture {
                texture.set(frame, egui::TextureOptions::LINEAR);
            } else {
                active.texture = Some(ui.ctx().load_texture(
                    "inline-video",
                    frame,
                    egui::TextureOptions::LINEAR,
                ));
            }
        }
        if let Some(texture) = &active.texture {
            ui.add(
                egui::Image::from_texture(texture)
                    .max_width(ui.available_width())
                    .fit_to_original_size(1.0),
            );
        }
        let mut stop = false;
        ui.horizontal_wrapped(|ui| {
            if ui
                .button(if state.ended {
                    "▶ Replay"
                } else if state.paused {
                    "▶ Play"
                } else {
                    "⏸ Pause"
                })
                .clicked()
            {
                if state.ended {
                    active.player.send(Command::Seek(0.0));
                }
                active
                    .player
                    .send(Command::Pause(!state.paused && !state.ended));
            }
            if ui.button("■ Stop").clicked() {
                stop = true;
            }
            ui.label(format!(
                "{} / {}",
                timestamp(state.position),
                timestamp(state.duration)
            ));
        });
        if state.duration.is_finite() && state.duration > 0.0 {
            let mut position = state.position;
            let response = ui.add(
                egui::Slider::new(&mut position, 0.0..=state.duration)
                    .text("Seek")
                    .show_value(false),
            );
            if response.changed() {
                active.player.send(Command::Seek(position));
            }
        }
        if ui
            .add(egui::Slider::new(&mut active.volume, 0.0..=100.0).text("Volume"))
            .changed()
        {
            active.player.send(Command::Volume(active.volume));
        }
        if !state.loaded && state.error.is_none() {
            ui.weak("Loading…");
        }
        if let Some(error) = &state.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        drop(state);
        if stop {
            self.active = None;
        }
    }
}
/// Draw a kind icon with shapes, independent of which glyphs the chosen font has.
fn paint_icon(painter: &egui::Painter, kind: Kind, r: egui::Rect, color: egui::Color32) {
    use egui::{Shape, Stroke, pos2};
    let stroke = Stroke::new((r.width() * 0.1).max(1.0), color);
    let at = |x: f32, y: f32| pos2(r.left() + r.width() * x, r.top() + r.height() * y);
    match kind {
        Kind::File => {
            // A page with a folded corner and two text lines.
            let page = vec![
                at(0.15, 0.0),
                at(0.6, 0.0),
                at(0.85, 0.25),
                at(0.85, 1.0),
                at(0.15, 1.0),
            ];
            painter.add(Shape::closed_line(page, stroke));
            painter.add(Shape::line(
                vec![at(0.6, 0.0), at(0.6, 0.25), at(0.85, 0.25)],
                stroke,
            ));
            painter.line_segment([at(0.32, 0.55), at(0.68, 0.55)], stroke);
            painter.line_segment([at(0.32, 0.77), at(0.68, 0.77)], stroke);
        }
        Kind::Image => {
            // A frame with a mountain and a sun.
            painter.rect_stroke(
                r.shrink2(egui::vec2(0.0, r.height() * 0.1)),
                1.0,
                stroke,
                egui::StrokeKind::Inside,
            );
            painter.add(Shape::convex_polygon(
                vec![at(0.12, 0.82), at(0.42, 0.42), at(0.72, 0.82)],
                color,
                Stroke::NONE,
            ));
            painter.circle_filled(at(0.7, 0.35), r.width() * 0.1, color);
        }
        Kind::Audio => {
            // A beamed pair of notes.
            painter.add(Shape::line(
                vec![at(0.35, 0.8), at(0.35, 0.12), at(0.85, 0.02), at(0.85, 0.7)],
                stroke,
            ));
            painter.circle_filled(at(0.25, 0.82), r.width() * 0.14, color);
            painter.circle_filled(at(0.75, 0.72), r.width() * 0.14, color);
        }
        Kind::Video => {
            painter.add(Shape::convex_polygon(
                vec![at(0.2, 0.1), at(0.9, 0.5), at(0.2, 0.9)],
                color,
                Stroke::NONE,
            ));
        }
    }
}
/// A disclosure triangle: pointing right when collapsed, down when expanded.
fn paint_arrow(painter: &egui::Painter, r: egui::Rect, expanded: bool, color: egui::Color32) {
    let points = if expanded {
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
/// Formats the reader displays itself; links to them open in this window.
fn is_document(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        ["md", "markdown", "txt"]
            .iter()
            .any(|d| e.eq_ignore_ascii_case(d))
    })
}
fn noun(kind: Kind) -> &'static str {
    match kind {
        Kind::Image => "image",
        Kind::Audio => "audio player",
        Kind::Video => "video player",
        Kind::File => "file",
    }
}
/// Whether the element at `span` is alone on its source line, ignoring list and
/// block quote markers and an enclosing link (`[![alt](image)](target)`).
fn standalone(text: &str, span: std::ops::Range<usize>) -> bool {
    let (Some(before), Some(after)) = (text.get(..span.start), text.get(span.end..)) else {
        return false;
    };
    let line = &before[before.rfind('\n').map_or(0, |i| i + 1)..];
    let mut after = &after[..after.find('\n').unwrap_or(after.len())];
    let before = line.strip_suffix('[').unwrap_or(line);
    if before.len() < line.len()
        && let Some(rest) = after.strip_prefix("](")
    {
        after = rest.split_once(')').map_or("", |(_, rest)| rest);
    }
    let marker = |token: &str| {
        matches!(token, "-" | "*" | "+")
            || token.chars().all(|c| c == '>')
            || token
                .strip_suffix(['.', ')'])
                .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
    };
    before.split_whitespace().all(marker) && after.trim_end_matches('\\').trim().is_empty()
}
fn timestamp(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}
fn open_file(url: &url::Url) -> Result<(), String> {
    let path = url
        .to_file_path()
        .map_err(|_| "Invalid local file URL".to_string())?;
    if !path.exists() {
        return Err(format!("File does not exist: {}", path.display()));
    }
    open_path(&path)
}

#[cfg(windows)]
fn open_path(path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    // ShellExecuteW opens the file association directly, without cmd.exe parsing.
    let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: the path is NUL-terminated and lives through the call; optional
    // arguments and the parent window are null. The returned value is not owned.
    let result = unsafe {
        windows_sys::Win32::UI::Shell::ShellExecuteW(
            std::ptr::null_mut(),
            std::ptr::null(),
            path.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
        )
    } as isize;
    if result > 32 {
        Ok(())
    } else {
        Err(format!(
            "Could not open file (Windows shell error {result})"
        ))
    }
}

#[cfg(not(windows))]
fn open_path(path: &Path) -> Result<(), String> {
    // Paths are process arguments, never shell commands. The absolute path
    // cannot be interpreted as an xdg-open option.
    let result = std::process::Command::new("xdg-open")
        .arg(path)
        .output()
        .map_err(|e| format!("Could not open file: {e}"))?;
    if result.status.success() {
        Ok(())
    } else {
        Err(format!(
            "Could not open file: {}",
            String::from_utf8_lossy(&result.stderr).trim()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn classifies_attachments_and_preserves_web_and_anchor_links() {
        let root = std::env::temp_dir();
        let base = url::Url::from_directory_path(root.join("docs folder")).unwrap();
        for (destination, kind) in [
            ("../clip.MP4", Kind::Video),
            ("song.opus", Kind::Audio),
            ("picture.png", Kind::Image),
            ("notes.txt", Kind::File),
            ("../paper.pdf", Kind::File),
            ("https://example.com/movie.webm?token=x#part", Kind::Video),
        ] {
            assert_eq!(resolve(&base, destination).unwrap().kind, kind);
        }
        assert_eq!(
            resolve(&base, "../other%20folder/a%23b.pdf")
                .unwrap()
                .url
                .to_file_path()
                .unwrap(),
            root.join("other folder/a#b.pdf")
        );
        for destination in [
            "#heading",
            "https://example.com/page",
            "mailto:a@example.com",
            "javascript:alert(1)",
        ] {
            assert!(resolve(&base, destination).is_none());
        }
    }
    #[test]
    fn standalone_means_alone_on_its_source_line() {
        let check = |text: &str, element: &str| {
            let start = text.find(element).unwrap();
            standalone(text, start..start + element.len())
        };
        assert!(check("![a](x.png)", "![a](x.png)"));
        assert!(check("Intro\n\n  ![a](x.png)  \nnext", "![a](x.png)"));
        assert!(check("- ![a](x.png)", "![a](x.png)"));
        assert!(check("> 1. [Clip](c.mp4)\\", "[Clip](c.mp4)"));
        assert!(check("[![a](x.png)](https://example.com)", "![a](x.png)"));
        assert!(!check("See ![a](x.png)", "![a](x.png)"));
        assert!(!check("![a](x.png) then text", "![a](x.png)"));
        assert!(!check("| [Clip](c.mp4) | 1 |", "[Clip](c.mp4)"));
        assert!(!check("2.5 ![a](x.png)", "![a](x.png)"));
    }
    fn render(
        text: &str,
        attachments: &std::rc::Rc<std::cell::RefCell<Attachments>>,
    ) -> egui_commonmark::RenderReport {
        render_with(&egui::Context::default(), Vec::new(), text, attachments)
    }
    fn render_with(
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        text: &str,
        attachments: &std::rc::Rc<std::cell::RefCell<Attachments>>,
    ) -> egui_commonmark::RenderReport {
        let base = url::Url::from_directory_path(std::env::temp_dir()).unwrap();
        let shared = attachments.clone();
        let render = move |ui: &mut egui::Ui, attachment: &egui_commonmark::Attachment<'_>| {
            shared.borrow_mut().render(ui, &base, text, attachment)
        };
        let mut cache = egui_commonmark::CommonMarkCache::default();
        let mut report = None;
        let input = egui::RawInput {
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                report = Some(
                    egui_commonmark::CommonMarkViewer::new()
                        .render_attachment_fn(Some(&render))
                        .show_with_report(ui, &mut cache, text)
                        .1,
                );
            });
        });
        output.textures_delta.clear();
        report.unwrap()
    }
    fn run<'a>(
        report: &'a egui_commonmark::RenderReport,
        text: &str,
    ) -> &'a egui_commonmark::TextRun {
        report
            .runs
            .iter()
            .find(|run| run.galley.text() == text)
            .unwrap()
    }
    #[test]
    fn file_links_are_compact_inline_chips_reported_as_text() {
        let attachments = Default::default();
        let report = render(
            "Before [my *document*](paper.pdf) after.\n\n[Movie](clip.mp4)\n\n![Audio](track.mp3)",
            &attachments,
        );
        let text: String = report.runs.iter().map(|run| run.galley.text()).collect();
        for expected in ["Before", "my document", "after.", "Movie", "Audio"] {
            assert!(text.contains(expected), "{expected}");
        }
        let (before, link) = (run(&report, "Before "), run(&report, "my document"));
        // Same line as the surrounding prose, and only one line tall.
        assert!((link.pos.y - before.pos.y).abs() < 1.0);
        assert!(link.pos.x > before.pos.x + before.galley.size().x);
        assert_eq!(link.galley.rows.len(), 1);
    }
    #[test]
    fn clicking_a_document_link_opens_it_in_the_reader() {
        assert!(is_document(Path::new("a/NOTES.Markdown")));
        assert!(is_document(Path::new("log.TXT")));
        assert!(!is_document(Path::new("paper.pdf")));
        let attachments: std::rc::Rc<std::cell::RefCell<Attachments>> = Default::default();
        let text = "See [the plan](plans/README.md#Setup%20Steps) first.";
        let ctx = egui::Context::default();
        let report = render_with(&ctx, Vec::new(), text, &attachments);
        let link = run(&report, "the plan");
        let pos = link.pos + link.galley.size() / 2.0;
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        render_with(
            &ctx,
            vec![egui::Event::PointerMoved(pos), button(true)],
            text,
            &attachments,
        );
        assert_eq!(attachments.borrow_mut().take_jump(), None);
        render_with(&ctx, vec![button(false)], text, &attachments);
        assert_eq!(
            attachments.borrow_mut().take_jump(),
            Some(Jump {
                path: Some(std::env::temp_dir().join("plans/README.md")),
                fragment: Some("Setup Steps".into()),
            })
        );
        assert_eq!(attachments.borrow_mut().take_jump(), None);
    }
    #[test]
    fn inline_media_is_collapsed_until_expanded() {
        let attachments: std::rc::Rc<std::cell::RefCell<Attachments>> = Default::default();
        let text = "Listen to [the clip](clip.mp4) here.\n\nNext paragraph.";
        let collapsed = render(text, &attachments);
        let (label, next) = (
            run(&collapsed, "the clip"),
            run(&collapsed, "Next paragraph."),
        );
        assert!(next.pos.y - label.pos.y < 3.0 * label.galley.size().y);
        attachments
            .borrow_mut()
            .expanded
            .insert(text.find('[').unwrap());
        let expanded = render(text, &attachments);
        // The player card opens below the link and pushes later content down.
        assert!(run(&expanded, "Next paragraph.").pos.y > next.pos.y + 40.0);
        assert!(run(&expanded, "the clip").pos.y < run(&expanded, "Next paragraph.").pos.y);
    }
}
