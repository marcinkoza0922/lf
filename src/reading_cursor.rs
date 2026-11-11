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
    LineStart,
    LineEnd,
}
#[derive(Default)]
pub struct ReadingCursor {
    position: usize,
    big_word: bool,
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
        Motion::Find(..) | Motion::LineStart | Motion::LineEnd => {}
    }
    p.min(chars.len() - 1)
}
impl ReadingCursor {
    pub fn input(&mut self, ctx: &egui::Context, enabled: bool, reserve_e: bool) {
        if !enabled {
            self.pending = None;
            self.motions.clear();
            return;
        }
        ctx.input_mut(|i| {
            let mut prefix_text = false;
            i.events.retain(|event| match event {
                Event::Key {
                    key: Key::Escape,
                    pressed: true,
                    ..
                } if self.pending.is_some() => {
                    self.pending = None;
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
                    let motion = match key {
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
                            "w" | "e" | "b" | "f" | "t" | "W" | "E" | "B" | "F" | "T"
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
        let mut moved = false;
        for motion in self.motions.drain(..) {
            if !matches!(motion, Motion::Find(..)) {
                self.big_word =
                    matches!(motion, Motion::BigWord | Motion::BigEnd | Motion::BigBack);
            }
            let target = match motion {
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
            moved |= target != self.position;
            self.position = target;
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
        if kind != 0 {
            while start > 0 && class(chars[start - 1]) == kind {
                start -= 1;
            }
            while end < chars.len() && class(chars[end]) == kind {
                end += 1;
            }
        }
        for p in start..end {
            if let Some(r) = rect(p) {
                ui.painter().rect_filled(
                    r,
                    1.0,
                    ui.visuals().selection.bg_fill.gamma_multiply(0.45),
                );
            }
        }
        if let Some(r) = rect(self.position) {
            ui.painter().line_segment(
                [r.left_top(), r.left_bottom()],
                egui::Stroke::new(2.0, ui.visuals().text_color()),
            );
            if moved {
                ui.scroll_to_rect_animation(r, None, egui::style::ScrollAnimation::none());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
