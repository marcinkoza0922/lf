use crate::playback::{Command, Playback};
use eframe::egui;
use std::{
    collections::HashMap,
    path::Path,
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
}
impl Attachments {
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn render(
        &mut self,
        ui: &mut egui::Ui,
        base: &url::Url,
        destination: &str,
        title: &str,
    ) -> bool {
        let Some(target) = resolve(base, destination) else {
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
        if target.kind == Kind::Image {
            ui.add(
                egui::Image::from_uri(target.url.as_str())
                    .max_width(ui.available_width())
                    .fit_to_original_size(1.0),
            )
            .on_hover_text(title);
            return true;
        }
        let id = ui.next_auto_id();
        ui.push_id(id, |ui| {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_max_width(ui.available_width().min(640.0));
                ui.vertical(|ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(match target.kind {
                            Kind::Audio => "♫",
                            Kind::Video => "▶",
                            _ => "▤",
                        });
                        egui_commonmark::attachment_label(
                            ui,
                            egui::RichText::new(if title.is_empty() { &name } else { title })
                                .strong(),
                        );
                    });
                    ui.weak(&name).on_hover_text(target.url.as_str());
                    match target.kind {
                        Kind::Audio | Kind::Video => self.media(ui, &target),
                        Kind::File => {
                            if ui.button("Open in default app ↗").clicked() {
                                let error = self.open_error.clone();
                                let ctx = ui.ctx().clone();
                                let target = target.url.clone();
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
                            if let Some(error) =
                                self.open_error.lock().unwrap().get(target.url.as_str())
                            {
                                ui.colored_label(ui.visuals().error_fg_color, error);
                            }
                        }
                        Kind::Image => unreachable!(),
                    }
                });
            });
        });
        true
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
    fn attachment_cards_are_reported_without_parsing_labels_as_markdown() {
        let ctx = egui::Context::default();
        let base = url::Url::from_directory_path(std::env::temp_dir()).unwrap();
        let attachments = std::rc::Rc::new(std::cell::RefCell::new(Attachments::default()));
        let render = move |ui: &mut egui::Ui, dest: &str, title: &str| {
            attachments.borrow_mut().render(ui, &base, dest, title)
        };
        let mut cache = egui_commonmark::CommonMarkCache::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                let (_, report) = egui_commonmark::CommonMarkViewer::new().render_attachment_fn(Some(&render)).show_with_report(ui, &mut cache, "Before [my document](paper.pdf) after.\n\n[Movie](clip.mp4)\n\n![Audio](track.mp3)");
                let text: String = report.runs.iter().map(|run| run.galley.text()).collect();
                assert!(text.contains("my document"));
                assert!(text.contains("Movie"));
                assert!(text.contains("Audio"));
                assert!(text.contains("Before"));
                assert!(text.contains("after."));
            });
        });
        output.textures_delta.clear();
    }
}
