use crate::text_search::TextIndex;
use eframe::egui::{self, Event, Key};

#[derive(Clone, Copy)]
enum Motion {
    Word,
    End,
    Back,
    BigWord,
    BigEnd,
    BigBack,
    Find(char, bool, bool),
    Left,
    Right,
    Vertical(bool),
    Select,
    Yank,
    Cancel,
    LineStart,
    LineEnd,
    DocumentFirst,
    DocumentLastLine,
    Viewport(bool),
}
#[derive(Default)]
pub struct ReadingCursor {
    position: usize,
    big_word: bool,
    character_highlight: bool,
    activated: bool,
    visible: bool,
    mouse_origin: Option<egui::Pos2>,
    anchor: Option<usize>,
    pending: Option<(bool, bool)>,
    motions: Vec<Motion>,
}
fn class(ch: char) -> u8 {
    if ch.is_whitespace() {
        0
    } else if ch.is_alphanumeric() || ch == '_' {
        1
    } else {
        2
    }
}
fn word_motion(chars: &[char], position: usize, motion: Motion) -> usize {
    if chars.is_empty() {
        return 0;
    }
    let big = matches!(motion, Motion::BigWord | Motion::BigEnd | Motion::BigBack);
    let class = |ch| {
        if big {
            u8::from(!char::is_whitespace(ch))
        } else {
            class(ch)
        }
    };
    let mut p = position.min(chars.len() - 1);
    match motion {
        Motion::Word | Motion::BigWord => {
            let kind = class(chars[p]);
            while p < chars.len() && class(chars[p]) == kind {
                p += 1;
            }
            while p < chars.len() && class(chars[p]) == 0 {
                p += 1;
            }
        }
        Motion::Back | Motion::BigBack => {
            p = p.saturating_sub(1);
            while p > 0 && class(chars[p]) == 0 {
                p -= 1;
            }
            let kind = class(chars[p]);
            while p > 0 && class(chars[p - 1]) == kind {
                p -= 1;
            }
        }
        Motion::End | Motion::BigEnd => {
            p = (p + 1).min(chars.len() - 1);
            while p + 1 < chars.len() && class(chars[p]) == 0 {
                p += 1;
            }
            let kind = class(chars[p]);
            while p + 1 < chars.len() && class(chars[p + 1]) == kind {
                p += 1;
            }
        }
        _ => {}
    }
    p.min(chars.len() - 1)
}
impl ReadingCursor {
    pub fn navigate(&mut self, commands: &[crate::navigation::Command]) {
        for command in commands {
            match command {
                crate::navigation::Command::CursorFirst => self.motions.push(Motion::DocumentFirst),
                crate::navigation::Command::CursorLastLine => {
                    self.motions.push(Motion::DocumentLastLine)
                }
                crate::navigation::Command::ViewportTop => {
                    self.motions.push(Motion::Viewport(false))
                }
                crate::navigation::Command::ViewportBottom => {
                    self.motions.push(Motion::Viewport(true))
                }
                crate::navigation::Command::LineStart => self.motions.push(Motion::LineStart),
                crate::navigation::Command::LineEnd => self.motions.push(Motion::LineEnd),
                _ => {}
            }
        }
    }

    pub fn input(&mut self, ctx: &egui::Context, enabled: bool, reserve_e: bool) {
        if !enabled {
            self.pending = None;
            self.motions.clear();
            return;
        }
        ctx.input_mut(|i| {
            let mut prefix_text = false;
            let mut navigation_prefix = reserve_e;
            i.events.retain(|event| match event {
                Event::Key {
                    key: Key::Escape,
                    pressed: true,
                    ..
                } => {
                    self.pending = None;
                    self.motions.push(Motion::Cancel);
                    true
                }
                Event::Copy if self.anchor.is_some() => {
                    self.motions.push(Motion::Yank);
                    false
                }
                Event::Key {
                    key: Key::C,
                    modifiers,
                    pressed: true,
                    ..
                } if modifiers.ctrl && self.anchor.is_some() => {
                    self.motions.push(Motion::Yank);
                    false
                }
                Event::Key {
                    key,
                    modifiers,
                    pressed: true,
                    ..
                } if !modifiers.ctrl && !modifiers.alt && !modifiers.command => {
                    if self.pending.is_some() {
                        return false;
                    }
                    if *key == Key::G || navigation_prefix {
                        navigation_prefix = true;
                        return true;
                    }
                    let motion = match key {
                        Key::H => Some(Motion::Left),
                        Key::L => Some(Motion::Right),
                        Key::J if !modifiers.shift => Some(Motion::Vertical(true)),
                        Key::K if !modifiers.shift => Some(Motion::Vertical(false)),
                        Key::V => Some(Motion::Select),
                        Key::Y => Some(Motion::Yank),
                        Key::W => Some(if modifiers.shift {
                            Motion::BigWord
                        } else {
                            Motion::Word
                        }),
                        Key::E if !reserve_e || modifiers.shift => Some(if modifiers.shift {
                            Motion::BigEnd
                        } else {
                            Motion::End
                        }),
                        Key::B => Some(if modifiers.shift {
                            Motion::BigBack
                        } else {
                            Motion::Back
                        }),
                        Key::F | Key::T => {
                            prefix_text = true;
                            self.pending = Some((*key == Key::T, modifiers.shift));
                            return false;
                        }
                        _ => None,
                    };
                    if let Some(motion) = motion {
                        self.motions.push(motion);
                        false
                    } else {
                        true
                    }
                }
                Event::Text(text) => {
                    if prefix_text {
                        prefix_text = false;
                        return false;
                    }
                    if let Some((till, backward)) = self.pending.take() {
                        if let Some(ch) = text.chars().next() {
                            self.motions.push(Motion::Find(ch, till, backward));
                        }
                        false
                    } else if navigation_prefix {
                        true
                    } else if text == "0" || text == "$" {
                        self.motions.push(if text == "0" {
                            Motion::LineStart
                        } else {
                            Motion::LineEnd
                        });
                        false
                    } else {
                        !matches!(
                            text.as_str(),
                            "w" | "e"
                                | "b"
                                | "f"
                                | "t"
                                | "W"
                                | "E"
                                | "B"
                                | "F"
                                | "T"
                                | "h"
                                | "j"
                                | "k"
                                | "l"
                                | "v"
                                | "y"
                        )
                    }
                }
                _ => true,
            });
        });
    }
    pub fn render(&mut self, ui: &egui::Ui, index: &TextIndex) {
        let chars: Vec<char> = index.text.chars().collect();
        if chars.is_empty() {
            self.position = 0;
            self.motions.clear();
            return;
        }
        self.position = self.position.min(chars.len() - 1);
        let offsets: Vec<usize> = index.text.char_indices().map(|(b, _)| b).collect();
        let rect = |p: usize| {
            index
                .glyphs
                .iter()
                .find(|g| g.bytes.start == offsets[p])
                .map(|g| g.rect)
        };
        let (pressed, released, pointer) = ui.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.primary_released(),
                i.pointer.interact_pos(),
            )
        });
        {
            if pressed && pointer.is_some_and(|p| ui.clip_rect().contains(p)) {
                self.visible = false;
                self.anchor = None;
                self.pending = None;
                self.mouse_origin = pointer;
            }
            if released && let Some(origin) = self.mouse_origin.take() {
                let selected = ui.ctx().with_plugin(
                    |state: &mut egui::text_selection::LabelSelectionState| state.has_selection(),
                );
                if let Some(pointer) = pointer
                    && (selected.unwrap_or(false)
                        || (pointer.distance(origin) > 6.0
                            && index.glyphs.iter().any(|g| g.rect.contains(origin))))
                    && let Some((p, _)) = (0..chars.len())
                        .filter_map(|p| rect(p).map(|r| (p, r)))
                        .min_by(|(_, a), (_, b)| {
                            a.distance_sq_to_pos(pointer)
                                .total_cmp(&b.distance_sq_to_pos(pointer))
                        })
                {
                    self.position = p;
                    self.activated = true;
                }
            }
        }
        let mut moved = false;
        for motion in self.motions.drain(..) {
            if !matches!(motion, Motion::Cancel | Motion::Yank) {
                self.visible = true;
                ui.ctx()
                    .with_plugin(|state: &mut egui::text_selection::LabelSelectionState| {
                        state.clear_selection()
                    });
            }
            if matches!(motion, Motion::Left | Motion::Right | Motion::Vertical(_)) {
                self.character_highlight = true;
                if !self.activated {
                    self.activated = true;
                    self.position = chars.iter().position(|ch| !ch.is_whitespace()).unwrap_or(0);
                    moved = true;
                    continue;
                }
            } else if !matches!(motion, Motion::Select | Motion::Yank | Motion::Cancel) {
                self.character_highlight = false;
            }
            if !matches!(motion, Motion::Cancel | Motion::Yank) {
                self.activated = true;
            }
            if !matches!(motion, Motion::Find(..)) {
                self.big_word =
                    matches!(motion, Motion::BigWord | Motion::BigEnd | Motion::BigBack);
            }
            let target = match motion {
                Motion::Select => {
                    self.anchor = if self.anchor.is_some() {
                        None
                    } else {
                        Some(self.position)
                    };
                    self.position
                }
                Motion::Cancel => {
                    self.visible = false;
                    self.anchor = None;
                    self.position
                }
                Motion::Yank => {
                    if let Some(anchor) = self.anchor.take() {
                        let start = anchor.min(self.position).min(chars.len() - 1);
                        let end = anchor.max(self.position).min(chars.len() - 1);
                        ui.ctx().copy_text(chars[start..=end].iter().collect());
                    }
                    self.position
                }
                Motion::Viewport(bottom) => {
                    let clip = ui.clip_rect();
                    let visible: Vec<_> = (0..chars.len())
                        .filter_map(|p| rect(p).map(|r| (p, r)))
                        .filter(|(_, r)| r.top() >= clip.top() && r.bottom() <= clip.bottom())
                        .collect();
                    let line = if bottom {
                        visible.last()
                    } else {
                        visible.first()
                    };
                    line.and_then(|(_, line)| {
                        visible.iter().find(|(_, r)| {
                            (r.center().y - line.center().y).abs()
                                < r.height().min(line.height()) * 0.5
                        })
                    })
                    .map_or(self.position, |(p, _)| *p)
                }
                Motion::DocumentFirst => {
                    chars.iter().position(|ch| !ch.is_whitespace()).unwrap_or(0)
                }
                Motion::DocumentLastLine => {
                    let last = (0..chars.len()).rev().find_map(|p| rect(p).map(|r| (p, r)));
                    last.map_or(0, |(end, line)| {
                        (0..=end)
                            .rev()
                            .take_while(|&p| {
                                rect(p).is_some_and(|r| {
                                    (r.center().y - line.center().y).abs()
                                        < r.height().min(line.height()) * 0.5
                                })
                            })
                            .last()
                            .unwrap_or(end)
                    })
                }
                Motion::Left => self.position.saturating_sub(1),
                Motion::Right => (self.position + 1).min(chars.len() - 1),
                Motion::Vertical(down) => rect(self.position)
                    .and_then(|current| {
                        let line_y = (0..chars.len())
                            .filter_map(&rect)
                            .filter(|r| {
                                if down {
                                    r.center().y > current.center().y + current.height() * 0.5
                                } else {
                                    r.center().y < current.center().y - current.height() * 0.5
                                }
                            })
                            .min_by(|a, b| {
                                (a.center().y - current.center().y)
                                    .abs()
                                    .total_cmp(&(b.center().y - current.center().y).abs())
                            })?
                            .center()
                            .y;
                        (0..chars.len())
                            .filter_map(|p| rect(p).map(|r| (p, r)))
                            .filter(|(_, r)| (r.center().y - line_y).abs() < r.height() * 0.5)
                            .min_by(|(_, a), (_, b)| {
                                (a.center().x - current.center().x)
                                    .abs()
                                    .total_cmp(&(b.center().x - current.center().x).abs())
                            })
                            .map(|(p, _)| p)
                    })
                    .unwrap_or(self.position),

                Motion::LineStart | Motion::LineEnd => {
                    let line = rect(self.position);
                    let candidates: Box<dyn Iterator<Item = usize>> =
                        if matches!(motion, Motion::LineStart) {
                            Box::new((0..self.position).rev())
                        } else {
                            Box::new((self.position + 1)..chars.len())
                        };
                    candidates
                        .take_while(|&p| {
                            chars[p] != '\n'
                                && line.zip(rect(p)).is_some_and(|(a, b)| {
                                    (a.center().y - b.center().y).abs()
                                        < a.height().min(b.height()) * 0.5
                                })
                        })
                        .last()
                        .unwrap_or(self.position)
                }
                Motion::Find(ch, till, backward) => {
                    let line = rect(self.position);
                    let candidates: Box<dyn Iterator<Item = usize>> = if backward {
                        Box::new((0..self.position).rev())
                    } else {
                        Box::new((self.position + 1)..chars.len())
                    };
                    candidates
                        .take_while(|&p| {
                            chars[p] != '\n'
                                && line.zip(rect(p)).is_some_and(|(a, b)| {
                                    (a.center().y - b.center().y).abs()
                                        < a.height().min(b.height()) * 0.5
                                })
                        })
                        .find(|&p| chars[p] == ch)
                        .map_or(self.position, |p| {
                            if backward {
                                p + usize::from(till)
                            } else {
                                p - usize::from(till)
                            }
                        })
                }
                _ => word_motion(&chars, self.position, motion),
            };
            moved |= target != self.position && !matches!(motion, Motion::Viewport(_));
            self.position = target;
        }
        if !self.visible {
            return;
        }
        let class = |ch| {
            if self.big_word {
                u8::from(!char::is_whitespace(ch))
            } else {
                class(ch)
            }
        };
        let kind = class(chars[self.position]);
        let mut start = self.position;
        let mut end = start + 1;
        if kind != 0 && !self.character_highlight {
            while start > 0 && class(chars[start - 1]) == kind {
                start -= 1;
            }
            while end < chars.len() && class(chars[end]) == kind {
                end += 1;
            }
        }
        if let Some(anchor) = self.anchor {
            start = anchor.min(self.position).min(chars.len() - 1);
            end = (anchor.max(self.position) + 1).min(chars.len());
        }
        for r in crate::text_search::highlight_rects((start..end).filter_map(rect)) {
            ui.painter()
                .rect_filled(r, 1.0, ui.visuals().selection.bg_fill.gamma_multiply(0.45));
        }
        if moved && let Some(r) = rect(self.position) {
            ui.scroll_to_rect_animation(r, None, egui::style::ScrollAnimation::none());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn g_prefix_routes_line_and_viewport_commands_before_cursor_motions() {
        let ctx = egui::Context::default();
        ctx.options_mut(|options| options.max_passes = std::num::NonZeroUsize::new(1).unwrap());
        let mut cursor = ReadingCursor::default();
        let mut nav = crate::navigation::Navigator::default();
        let mut cache = egui_commonmark::CommonMarkCache::default();
        for (key, expected) in [(Key::T, 5), (Key::B, 14), (Key::L, 17), (Key::H, 14)] {
            let events = [Key::G, key]
                .into_iter()
                .flat_map(|key| {
                    [true, false].map(move |pressed| Event::Key {
                        key,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    })
                })
                .collect();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    cursor.input(ui.ctx(), true, nav.word_end_reserved(ui.ctx()));
                    let commands = nav.commands(ui.ctx(), true);
                    assert_eq!(commands.len(), 1);
                    cursor.navigate(&commands);
                    egui::CentralPanel::default().show(ui, |ui| {
                        let (_, report) = egui_commonmark::CommonMarkViewer::new()
                            .show_with_report(
                                ui,
                                &mut cache,
                                "```text\nzero\n  middle\nlast\nhidden\n```",
                            );
                        let index = TextIndex::new(&report);
                        let first = index
                            .glyphs
                            .iter()
                            .find(|g| g.bytes.start == 5)
                            .unwrap()
                            .rect;
                        let last = index
                            .glyphs
                            .iter()
                            .find(|g| g.bytes.start == 14)
                            .unwrap()
                            .rect;
                        ui.set_clip_rect(egui::Rect::from_min_max(
                            egui::pos2(0.0, first.top() - 0.1),
                            egui::pos2(1000.0, last.bottom() + 0.1),
                        ));
                        cursor.render(ui, &index);
                    });
                },
            );
            output.textures_delta.clear();
            assert_eq!(cursor.position, expected);
        }
    }

    #[test]
    fn document_jumps_move_cursor_and_extend_selection() {
        let ctx = egui::Context::default();
        let mut cursor = ReadingCursor::default();
        let mut cache = egui_commonmark::CommonMarkCache::default();
        let mut frame = |cursor: &mut ReadingCursor, command| {
            cursor.navigate(&[command]);
            let mut text = String::new();
            let mut output =
                ctx.run_ui(egui::RawInput::default(), |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        let (_, report) = egui_commonmark::CommonMarkViewer::new()
                            .show_with_report(ui, &mut cache, "```text\n  αbc\n    last\n```");
                        let index = TextIndex::new(&report);
                        text = index.text.clone();
                        cursor.render(ui, &index);
                    });
                });
            output.textures_delta.clear();
            text
        };
        use crate::navigation::Command;
        let text = frame(&mut cursor, Command::CursorLastLine);
        let last_line = text
            .chars()
            .collect::<Vec<_>>()
            .windows(8)
            .position(|chars| chars.iter().collect::<String>() == "    last")
            .unwrap();
        assert_eq!(cursor.position, last_line);
        assert!(cursor.visible);
        cursor.anchor = Some(last_line);
        frame(&mut cursor, Command::CursorFirst);
        assert_eq!(cursor.position, 2);
        assert_eq!(cursor.anchor, Some(last_line));
        frame(&mut cursor, Command::CursorLastLine);
        assert_eq!(cursor.position, last_line);
    }

    #[test]
    fn hide_resume_and_mouse_selection_remember_position() {
        let ctx = egui::Context::default();
        let mut cursor = ReadingCursor::default();
        let mut cache = egui_commonmark::CommonMarkCache::default();
        let mut frame = |cursor: &mut ReadingCursor, events| {
            let mut rects = Vec::new();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    cursor.input(ui.ctx(), true, false);
                    egui::CentralPanel::default().show(ui, |ui| {
                        let (_, report) = egui_commonmark::CommonMarkViewer::new()
                            .show_with_report(ui, &mut cache, "```text\n  abc def\n```");
                        let index = TextIndex::new(&report);
                        rects = index.glyphs.iter().map(|g| g.rect).collect();
                        cursor.render(ui, &index);
                    });
                },
            );
            output.textures_delta.clear();
            rects
        };
        let key = |key| Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let button = |pos, pressed| Event::PointerButton {
            pos,
            pressed,
            button: egui::PointerButton::Primary,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&mut cursor, vec![]);
        assert!(!cursor.visible);
        let rects = frame(&mut cursor, vec![key(Key::L)]);
        assert_eq!(cursor.position, 2);
        assert!(cursor.visible);
        frame(&mut cursor, vec![key(Key::Escape)]);
        assert!(!cursor.visible);
        assert_eq!(cursor.position, 2);
        frame(&mut cursor, vec![key(Key::L)]);
        assert_eq!(cursor.position, 3);
        assert!(cursor.visible);
        let start = rects[2].center();
        frame(
            &mut cursor,
            vec![Event::PointerMoved(start), button(start, true)],
        );
        assert!(!cursor.visible);
        frame(&mut cursor, vec![button(start, false)]);
        assert_eq!(cursor.position, 3);
        frame(&mut cursor, vec![key(Key::L)]);
        assert_eq!(cursor.position, 4);
        // A real mouse drag replaces the remembered keyboard position.
        let end = rects[7].center();
        frame(
            &mut cursor,
            vec![Event::PointerMoved(start), button(start, true)],
        );
        frame(&mut cursor, vec![Event::PointerMoved(end)]);
        frame(&mut cursor, vec![button(end, false)]);
        assert!(!cursor.visible);
        assert_eq!(cursor.position, 7);
        frame(&mut cursor, vec![key(Key::H)]);
        assert_eq!(cursor.position, 6);
        assert!(cursor.visible);
    }
    #[test]
    fn shifted_vertical_keys_scroll_without_touching_cursor_or_selection() {
        let ctx = egui::Context::default();
        let mut cursor = ReadingCursor {
            position: 4,
            anchor: Some(1),
            ..Default::default()
        };
        let mut navigator = crate::navigation::Navigator::default();
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: [Key::J, Key::K]
                    .into_iter()
                    .map(|key| Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::SHIFT,
                    })
                    .collect(),
                ..Default::default()
            },
            |ui| {
                cursor.input(ui.ctx(), true, false);
                assert!(cursor.motions.is_empty());
                assert_eq!(cursor.position, 4);
                assert_eq!(cursor.anchor, Some(1));
                assert!(!cursor.activated);
                assert_eq!(
                    navigator.commands(ui.ctx(), true),
                    vec![
                        crate::navigation::Command::Down,
                        crate::navigation::Command::Up,
                    ]
                );
            },
        );
        output.textures_delta.clear();
    }
    #[test]
    fn first_character_motion_starts_at_document_beginning() {
        let ctx = egui::Context::default();
        let mut cache = egui_commonmark::CommonMarkCache::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                let (_, report) = egui_commonmark::CommonMarkViewer::new().show_with_report(
                    ui,
                    &mut cache,
                    "first word\n\nsecond line",
                );
                let index = TextIndex::new(&report);
                for motion in [Motion::Right, Motion::Vertical(true)] {
                    let mut cursor = ReadingCursor::default();
                    // Rendering alone must not count as the first cursor interaction.
                    cursor.render(ui, &index);
                    cursor.motions.push(motion);
                    cursor.render(ui, &index);
                    assert_eq!(cursor.position, 0);
                    assert!(cursor.character_highlight);
                    cursor.motions.push(motion);
                    cursor.render(ui, &index);
                    assert!(cursor.position > 0);
                    assert!(cursor.character_highlight);
                    cursor.motions.push(Motion::Word);
                    cursor.render(ui, &index);
                    assert!(!cursor.character_highlight);
                    cursor.motions.push(Motion::Left);
                    cursor.render(ui, &index);
                    assert!(cursor.character_highlight);
                }
            });
        });
        output.textures_delta.clear();
    }
    #[test]
    fn character_selection_copies_unicode_and_arrows_remain_available() {
        let ctx = egui::Context::default();
        let mut cursor = ReadingCursor::default();
        let mut cache = egui_commonmark::CommonMarkCache::default();
        let key = |key| Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let mut output =
            ctx.run_ui(
                egui::RawInput {
                    events: vec![
                        key(Key::V),
                        key(Key::L),
                        key(Key::L),
                        key(Key::Y),
                        key(Key::ArrowDown),
                    ],
                    ..Default::default()
                },
                |ui| {
                    cursor.input(ui.ctx(), true, false);
                    assert!(ui.input(|i| i.key_pressed(Key::ArrowDown)));
                    assert!(!ui.input(|i| i.key_pressed(Key::L)));
                    egui::CentralPanel::default().show(ui, |ui| {
                        let (_, report) = egui_commonmark::CommonMarkViewer::new()
                            .show_with_report(ui, &mut cache, "aé日z\n\nnext");
                        let index = TextIndex::new(&report);
                        cursor.render(ui, &index);
                        assert_eq!(cursor.position, 2);
                        assert!(cursor.anchor.is_none());
                        cursor.motions.push(Motion::Vertical(true));
                        cursor.render(ui, &index);
                        assert!(cursor.position > 3);
                        cursor.motions.push(Motion::Vertical(false));
                        cursor.render(ui, &index);
                        assert!(cursor.position < 4);
                    });
                },
            );
        assert!(output.platform_output.commands.iter().any(
            |command| matches!(command, egui::OutputCommand::CopyText(text) if text == "aé日")
        ));
        output.textures_delta.clear();
    }
    #[test]
    fn line_edges_use_visual_wrapping_and_preserve_find_targets() {
        let ctx = egui::Context::default();
        let mut cursor = ReadingCursor::default();
        let mut cache = egui_commonmark::CommonMarkCache::default();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(180.0, 400.0),
                )),
                events: vec![Event::Text("$".into())],
                ..Default::default()
            },
            |ui| {
                cursor.input(ui.ctx(), true, false);
                assert!(matches!(cursor.motions.as_slice(), [Motion::LineEnd]));
                egui::CentralPanel::default().show(ui, |ui| {
                    let (_, report) = egui_commonmark::CommonMarkViewer::new().show_with_report(
                        ui,
                        &mut cache,
                        "one two three four five six seven eight nine ten eleven twelve",
                    );
                    let index = TextIndex::new(&report);
                    let y = index.glyphs[0].rect.center().y;
                    let first_line_end = index
                        .glyphs
                        .iter()
                        .take_while(|g| (g.rect.center().y - y).abs() < 1.0)
                        .count()
                        - 1;
                    assert!(first_line_end < index.text.chars().count() - 1);
                    cursor.render(ui, &index);
                    assert_eq!(cursor.position, first_line_end);
                    cursor.motions.push(Motion::LineStart);
                    cursor.render(ui, &index);
                    assert_eq!(cursor.position, 0);
                    cursor.position = first_line_end + 2;
                    cursor.motions.push(Motion::LineStart);
                    cursor.render(ui, &index);
                    assert_eq!(cursor.position, first_line_end + 1);
                });
            },
        );
        output.textures_delta.clear();
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![Event::Text("0".into())],
                ..Default::default()
            },
            |ui| cursor.input(ui.ctx(), true, false),
        );
        output.textures_delta.clear();
        assert!(matches!(cursor.motions.as_slice(), [Motion::LineStart]));
        cursor.motions.clear();
        cursor.pending = Some((false, false));
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![Event::Text("$".into())],
                ..Default::default()
            },
            |ui| cursor.input(ui.ctx(), true, false),
        );
        output.textures_delta.clear();
        assert!(matches!(
            cursor.motions.as_slice(),
            [Motion::Find('$', false, false)]
        ));
    }
    #[test]
    fn big_words_include_punctuation_and_respect_unicode_whitespace() {
        let chars: Vec<_> = "one,two\t日本語!\u{2003}last.".chars().collect();
        assert_eq!(word_motion(&chars, 0, Motion::BigWord), 8);
        assert_eq!(word_motion(&chars, 0, Motion::BigEnd), 6);
        assert_eq!(word_motion(&chars, 6, Motion::BigEnd), 11);
        assert_eq!(word_motion(&chars, 11, Motion::BigBack), 8);
        assert_eq!(word_motion(&chars, 8, Motion::BigBack), 0);
        assert_eq!(word_motion(&chars, 0, Motion::BigBack), 0);
        assert_eq!(
            word_motion(&chars, chars.len() - 1, Motion::BigWord),
            chars.len() - 1
        );
    }
    #[test]
    fn shifted_keys_queue_big_motions_and_backward_find() {
        let ctx = egui::Context::default();
        let mut cursor = ReadingCursor::default();
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: [Key::W, Key::E, Key::B, Key::T]
                    .into_iter()
                    .map(|key| Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::SHIFT,
                    })
                    .chain([Event::Text("T".into()), Event::Text("é".into())])
                    .collect(),
                ..Default::default()
            },
            |ui| cursor.input(ui.ctx(), true, false),
        );
        output.textures_delta.clear();
        assert!(matches!(
            cursor.motions.as_slice(),
            [
                Motion::BigWord,
                Motion::BigEnd,
                Motion::BigBack,
                Motion::Find('é', true, true)
            ]
        ));
    }
    #[test]
    fn words_punctuation_unicode_and_boundaries() {
        let chars: Vec<_> = "one, 日本語 two".chars().collect();
        assert_eq!(word_motion(&chars, 0, Motion::Word), 3);
        assert_eq!(word_motion(&chars, 3, Motion::Word), 5);
        assert_eq!(word_motion(&chars, 5, Motion::End), 7);
        assert_eq!(word_motion(&chars, 7, Motion::End), 11);
        assert_eq!(word_motion(&chars, 11, Motion::Back), 9);
        assert_eq!(word_motion(&chars, 9, Motion::Back), 5);
        assert_eq!(word_motion(&chars, 0, Motion::Back), 0);
        assert_eq!(word_motion(&chars, 11, Motion::Word), 11);
    }
    #[test]
    fn character_find_prefix_waits_for_target_and_suppresses_shortcuts() {
        let ctx = egui::Context::default();
        let mut cursor = ReadingCursor::default();
        let key = |key| Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let frame = |cursor: &mut ReadingCursor, events, enabled| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| cursor.input(ui.ctx(), enabled, false),
            );
            output.textures_delta.clear();
        };
        frame(
            &mut cursor,
            vec![key(Key::F), Event::Text("f".into())],
            true,
        );
        assert_eq!(cursor.pending, Some((false, false)));
        assert!(cursor.motions.is_empty());
        frame(
            &mut cursor,
            vec![key(Key::S), Event::Text("s".into())],
            true,
        );
        assert!(matches!(
            cursor.motions.as_slice(),
            [Motion::Find('s', false, false)]
        ));
        frame(&mut cursor, vec![key(Key::W)], false);
        assert!(cursor.motions.is_empty());
    }
    #[test]
    fn find_and_till_follow_rendered_line_and_highlight_cursor() {
        let ctx = egui::Context::default();
        let mut cache = egui_commonmark::CommonMarkCache::default();
        let mut cursor = ReadingCursor::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                let (_, report) = egui_commonmark::CommonMarkViewer::new().show_with_report(
                    ui,
                    &mut cache,
                    "one two three\n\nlast z",
                );
                let index = TextIndex::new(&report);
                cursor.motions.push(Motion::Find('t', false, false));
                cursor.render(ui, &index);
                assert_eq!(cursor.position, 4);
                cursor.motions.push(Motion::Find('t', true, false));
                cursor.render(ui, &index);
                assert_eq!(cursor.position, 7);
                cursor.motions.push(Motion::Find('z', false, false));
                cursor.render(ui, &index);
                assert_eq!(cursor.position, 7);
                cursor.position = 12;
                cursor.motions.push(Motion::Find('t', false, true));
                cursor.render(ui, &index);
                assert_eq!(cursor.position, 8);
                cursor.motions.push(Motion::Find('t', true, true));
                cursor.render(ui, &index);
                assert_eq!(cursor.position, 5);
                cursor.motions.push(Motion::Find('z', false, true));
                cursor.render(ui, &index);
                assert_eq!(cursor.position, 5);
                cursor.position = 0;
                cursor.motions.push(Motion::Find('o', false, true));
                cursor.render(ui, &index);
                assert_eq!(cursor.position, 0);
            });
        });
        output.textures_delta.clear();
    }
}
