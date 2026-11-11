use eframe::egui::{self, Color32, Rect};
use egui_commonmark::RenderReport;
use std::ops::Range;

pub(crate) struct Glyph {
    pub(crate) bytes: Range<usize>,
    pub(crate) rect: Rect,
    source: usize,
    heading: Option<usize>,
}
#[derive(Default)]
pub struct TextIndex {
    pub text: String,
    pub(crate) glyphs: Vec<Glyph>,
    pub headings: Vec<(usize, Rect)>,
}
impl TextIndex {
    pub fn new(report: &RenderReport) -> Self {
        let mut index = Self::default();
        let mut last: Option<Rect> = None;
        for run in &report.runs {
            let rect = run.galley.rect.translate(run.pos.to_vec2());
            if last.is_some_and(|prev| rect.top() > prev.bottom() + 1.0)
                && !index.text.ends_with(char::is_whitespace)
            {
                index.text.push('\n');
            }
            let mut local_byte = 0;
            for row in &run.galley.rows {
                for glyph in &row.glyphs {
                    let start = index.text.len();
                    index.text.push(glyph.chr);
                    let rect = glyph
                        .logical_rect()
                        .translate(run.pos.to_vec2() + row.pos.to_vec2());
                    index.glyphs.push(Glyph {
                        bytes: start..index.text.len(),
                        rect,
                        source: (run.source.start + local_byte).min(run.source.end),
                        heading: run.heading,
                    });
                    local_byte += glyph.chr.len_utf8();
                    if let Some(id) = run.heading {
                        if let Some((_, heading_rect)) =
                            index.headings.iter_mut().find(|(i, _)| *i == id)
                        {
                            *heading_rect = heading_rect.union(rect);
                        } else {
                            index.headings.push((id, rect));
                        }
                    }
                }
                if row.ends_with_newline {
                    index.text.push('\n');
                    local_byte += 1;
                }
            }
            last = Some(rect);
        }
        index
    }
    fn rects(&self, range: &Range<usize>) -> Vec<Rect> {
        self.glyphs
            .iter()
            .filter(|g| g.bytes.start < range.end && g.bytes.end > range.start)
            .map(|g| g.rect)
            .collect()
    }
    fn match_rect(&self, range: &Range<usize>) -> Option<Rect> {
        self.rects(range).into_iter().reduce(Rect::union)
    }
}

#[derive(Default)]
pub struct Search {
    pub open: bool,
    pub query: String,
    pub focus: bool,
    pub active: usize,
    pub count: usize,
    pub heading: Option<usize>,
    heading_anchor: Option<usize>,
    pub heading_step: Option<bool>,
    pub source_target: Option<usize>,
    jump: bool,
    previous_query: String,
    case_sensitive: bool,
    fuzzy: bool,
    regex: bool,
    previous_options: (bool, bool, bool),
    move_match: Option<bool>,
}
impl Search {
    pub fn clear(&mut self, ctx: &egui::Context) {
        *self = Self::default();
        let id = egui::Id::new("document-search-input");
        ctx.memory_mut(|m| m.surrender_focus(id));
    }
    pub fn begin(&mut self) {
        self.open = true;
        self.focus = true;
        self.heading = None;
        self.heading_anchor = None;
    }
    pub fn target(&mut self, query: String, source: usize) {
        self.query = query;
        self.source_target = Some(source);
        self.jump = true;
    }
    #[cfg(test)]
    pub fn needs_report(&self) -> bool {
        !self.query.is_empty() || self.heading.is_some() || self.heading_step.is_some()
    }
    pub fn open_shortcut(&mut self, ctx: &egui::Context) -> bool {
        let typing = ctx.text_edit_focused();
        let open = ctx.input_mut(|input| {
            let control_f = input.consume_key(egui::Modifiers::CTRL, egui::Key::F);
            let slash = !typing
                && (input.consume_key(egui::Modifiers::NONE, egui::Key::Slash)
                    || (!input.modifiers.ctrl
                        && !input.modifiers.alt
                        && !input.modifiers.command
                        && input
                            .events
                            .iter()
                            .any(|event| matches!(event, egui::Event::Text(text) if text == "/"))));
            if slash {
                input
                    .events
                    .retain(|event| !matches!(event, egui::Event::Text(text) if text == "/"));
            }
            control_f || slash
        });
        if open {
            self.begin();
        }
        open
    }
    pub fn option_shortcuts(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::I)) {
            self.case_sensitive = !self.case_sensitive;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::U)) {
            self.fuzzy = !self.fuzzy;
            if self.fuzzy {
                self.regex = false;
            }
        }
        if ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::R)
        }) {
            self.regex = !self.regex;
            if self.regex {
                self.fuzzy = false;
            }
        }
    }
    pub fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.text_edit_focused() || self.query.is_empty() {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, egui::Key::N)) {
            self.move_match = Some(false);
        } else if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::N)) {
            self.move_match = Some(true);
        }
    }
    pub fn bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let previous = ctx.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, egui::Key::Enter));
        let next = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        ui.horizontal(|ui| {
            ui.label("Find");
            let mut output = egui::TextEdit::singleline(&mut self.query)
                .id(egui::Id::new("document-search-input"))
                .hint_text("Search document…")
                .desired_width(240.0)
                .show(ui);
            let response = &output.response;
            if self.focus {
                response.request_focus();
                output
                    .state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0),
                        egui::text::CCursor::new(self.query.chars().count()),
                    )));
                output.state.store(&ctx, response.id);
                self.focus = false;
            }
            if previous || next {
                self.open = false;
                response.surrender_focus();
            }
            if ui.small_button("Previous").clicked() {
                self.move_match = Some(false);
            }
            if ui.small_button("Next").clicked() {
                self.move_match = Some(true);
            }
            ui.label(if self.count == 0 {
                "0 matches".into()
            } else {
                format!("{} / {}", self.active + 1, self.count)
            });
            if ui.small_button("Close (Esc)").clicked() {
                self.clear(&ctx);
            }
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.case_sensitive, "Match case (Ctrl+I)");
            if ui.checkbox(&mut self.fuzzy, "Fuzzy (Ctrl+U)").changed() && self.fuzzy {
                self.regex = false;
            }
            if ui
                .checkbox(&mut self.regex, "Regex (Ctrl+Shift+R)")
                .changed()
                && self.regex
            {
                self.fuzzy = false;
            }
        });
        if self.regex
            && let Err(error) = self.pattern()
        {
            ui.colored_label(Color32::LIGHT_RED, error.to_string());
        }
    }
    fn pattern(&self) -> Result<regex::Regex, regex::Error> {
        regex::RegexBuilder::new(&self.query)
            .case_insensitive(!self.case_sensitive)
            .build()
    }
    fn matches(&self, text: &str) -> Vec<Range<usize>> {
        if self.query.is_empty() {
            return Vec::new();
        }
        if self.regex {
            return self
                .pattern()
                .map(|re| {
                    re.find_iter(text)
                        .filter(|m| !m.is_empty())
                        .map(|m| m.range())
                        .collect()
                })
                .unwrap_or_default();
        }
        if self.fuzzy {
            // Ordered subsequences within a line, highlighting the complete matched span.
            let parts: Vec<String> = self
                .query
                .chars()
                .map(|ch| regex::escape(&ch.to_string()))
                .collect();
            let pattern = parts.join("[^\\n]*?");
            return regex::RegexBuilder::new(&pattern)
                .case_insensitive(!self.case_sensitive)
                .build()
                .map(|re| re.find_iter(text).map(|m| m.range()).collect())
                .unwrap_or_default();
        }
        if self.case_sensitive {
            text.match_indices(&self.query)
                .map(|(start, _)| start..start + self.query.len())
                .collect()
        } else {
            find_matches(text, &self.query)
        }
    }
    pub fn render(&mut self, ui: &egui::Ui, index: &TextIndex, viewport_top: f32) {
        let matches = self.matches(&index.text);
        self.count = matches.len();
        let options = (self.case_sensitive, self.fuzzy, self.regex);
        if self.previous_query != self.query || self.previous_options != options {
            self.previous_options = options;
            self.previous_query = self.query.clone();
            self.active = 0;
            self.jump = true;
        }
        if let Some(source) = self.source_target.take() {
            if matches.is_empty()
                && let Some(glyph) = index
                    .glyphs
                    .iter()
                    .min_by_key(|g| g.source.abs_diff(source))
            {
                ui.scroll_to_rect_animation(
                    glyph.rect,
                    Some(egui::Align::Center),
                    egui::style::ScrollAnimation::none(),
                );
            }
            if let Some((i, _)) = matches.iter().enumerate().min_by_key(|(_, range)| {
                index
                    .glyphs
                    .iter()
                    .find(|g| g.bytes.start >= range.start)
                    .map_or(usize::MAX, |g| g.source.abs_diff(source))
            }) {
                self.active = i;
                self.jump = true;
            }
        }
        if let Some(next) = self.move_match.take()
            && !matches.is_empty()
        {
            self.active = if next {
                (self.active + 1) % matches.len()
            } else {
                (self.active + matches.len() - 1) % matches.len()
            };
            self.jump = true;
        }
        self.active = self.active.min(matches.len().saturating_sub(1));
        for (i, range) in matches.iter().enumerate() {
            let color = if i == self.active {
                Color32::from_rgba_unmultiplied(255, 140, 0, 100)
            } else {
                Color32::from_rgba_unmultiplied(255, 210, 40, 65)
            };
            for rect in index.rects(range) {
                ui.painter().rect_filled(rect, 1.0, color);
            }
        }
        if self.jump {
            if let Some(rect) = matches.get(self.active).and_then(|r| index.match_rect(r)) {
                ui.scroll_to_rect_animation(
                    rect,
                    Some(egui::Align::Center),
                    egui::style::ScrollAnimation::none(),
                );
            }
            self.jump = false;
        }
        if let Some(next) = self.heading_step.take() {
            let current = self
                .heading_anchor
                .and_then(|id| index.headings.iter().position(|(i, _)| *i == id));
            let target = if let Some(current) = current {
                if next {
                    index.headings.get(current + 1)
                } else {
                    current.checked_sub(1).and_then(|i| index.headings.get(i))
                }
            } else if next {
                index
                    .headings
                    .iter()
                    .find(|(_, rect)| rect.top() > viewport_top + 2.0)
            } else {
                index
                    .headings
                    .iter()
                    .rev()
                    .find(|(_, rect)| rect.top() < viewport_top - 2.0)
            };
            if let Some((id, rect)) = target {
                self.heading = Some(*id);
                self.heading_anchor = Some(*id);
                ui.scroll_to_rect_animation(
                    *rect,
                    Some(egui::Align::TOP),
                    egui::style::ScrollAnimation::none(),
                );
            }
        }
        if let Some(id) = self.heading {
            for glyph in index.glyphs.iter().filter(|g| g.heading == Some(id)) {
                ui.painter().rect_filled(
                    glyph.rect,
                    1.0,
                    Color32::from_rgba_unmultiplied(60, 210, 210, 85),
                );
            }
        }
    }
    pub fn manually_scrolled(&mut self) {
        self.heading_anchor = None;
    }
}

pub fn find_matches(text: &str, query: &str) -> Vec<Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }
    let mut folded = String::new();
    let mut map = Vec::new();
    for (start, ch) in text.char_indices() {
        for lower in ch.to_lowercase() {
            let begin = folded.len();
            folded.push(lower);
            map.resize(folded.len(), start..start + ch.len_utf8());
            debug_assert!(folded.len() > begin);
        }
    }
    let query = query.to_lowercase();
    folded
        .match_indices(&query)
        .map(|(start, _)| map[start].start..map[start + query.len() - 1].end)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn option_shortcuts_are_scoped_and_consume_toggle_keys() {
        let ctx = egui::Context::default();
        let mut search = Search::default();
        let press = |search: &mut Search, key, modifiers| {
            let mut consumed = false;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events: vec![egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers,
                    }],
                    ..Default::default()
                },
                |ui| {
                    search.option_shortcuts(ui.ctx());
                    consumed = !ui.input_mut(|i| i.consume_key(modifiers, key));
                },
            );
            output.textures_delta.clear();
            consumed
        };
        assert!(!press(
            &mut search,
            egui::Key::R,
            egui::Modifiers::CTRL | egui::Modifiers::SHIFT
        ));
        assert!(!search.regex);
        search.begin();
        assert!(!press(&mut search, egui::Key::X, egui::Modifiers::CTRL));
        assert!(!press(&mut search, egui::Key::R, egui::Modifiers::CTRL));
        assert!(press(
            &mut search,
            egui::Key::R,
            egui::Modifiers::CTRL | egui::Modifiers::SHIFT
        ));
        assert!(search.regex);
        assert!(press(&mut search, egui::Key::U, egui::Modifiers::CTRL));
        assert!(search.fuzzy && !search.regex);
        assert!(press(
            &mut search,
            egui::Key::R,
            egui::Modifiers::CTRL | egui::Modifiers::SHIFT
        ));
        assert!(search.regex && !search.fuzzy);
        assert!(press(
            &mut search,
            egui::Key::R,
            egui::Modifiers::CTRL | egui::Modifiers::SHIFT
        ));
        assert!(!search.regex);
        assert!(press(&mut search, egui::Key::I, egui::Modifiers::CTRL));
        assert!(search.case_sensitive);
        assert!(press(&mut search, egui::Key::I, egui::Modifiers::CTRL));
        assert!(!search.case_sensitive);
    }
    #[test]
    fn matching_options_and_invalid_regex() {
        let mut search = Search {
            query: "Écho".into(),
            ..Default::default()
        };
        assert_eq!(search.matches("Écho écho").len(), 2);
        search.case_sensitive = true;
        assert_eq!(search.matches("Écho écho"), vec![0..5]);
        search.query = "nle".into();
        search.fuzzy = true;
        assert_eq!(search.matches("needle\nneedle"), vec![0..6, 7..13]);
        assert!(search.matches("n\nle").is_empty());
        search.fuzzy = false;
        search.regex = true;
        search.query = "n[a-z]+".into();
        assert_eq!(search.matches("needle"), vec![0..6]);
        search.query = "[".into();
        assert!(search.pattern().is_err());
        assert!(search.matches("needle").is_empty());
        search.query = "^".into();
        assert!(search.matches("needle").is_empty());
    }
    #[test]
    fn literal_unicode_matching_preserves_original_byte_ranges() {
        let text = "Écho éCHO 日本語 [x]";
        let matches = find_matches(text, "écho");
        assert_eq!(
            matches.iter().map(|r| &text[r.clone()]).collect::<Vec<_>>(),
            vec!["Écho", "éCHO"]
        );
        assert_eq!(find_matches(text, "[x]").len(), 1);
        assert!(find_matches(text, "").is_empty());
    }
    #[test]
    fn highlights_and_escape_preserve_user_drag_selection() {
        let ctx = egui::Context::default();
        let mut cache = egui_commonmark::CommonMarkCache::default();
        let mut search = Search::default();
        let mut frame = |search: &mut Search, events| {
            let mut bounds = Rect::NOTHING;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(600.0, 300.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                        search.clear(ui.ctx());
                    }
                    egui::CentralPanel::default().show(ui, |ui| {
                        let (_, report) = egui_commonmark::CommonMarkViewer::new()
                            .show_with_report(ui, &mut cache, "select this needle");
                        let index = TextIndex::new(&report);
                        bounds = index
                            .glyphs
                            .iter()
                            .map(|g| g.rect)
                            .reduce(Rect::union)
                            .unwrap();
                        search.render(ui, &index, 0.0);
                    });
                },
            );
            let copied = output.platform_output.commands.iter().find_map(|c| {
                if let egui::OutputCommand::CopyText(text) = c {
                    Some(text.clone())
                } else {
                    None
                }
            });
            output.textures_delta.clear();
            (bounds, copied)
        };
        let bounds = frame(&mut search, vec![]).0;
        let start = egui::pos2(bounds.left() + 0.1, bounds.center().y);
        let end = egui::pos2(bounds.right() - 0.1, bounds.center().y);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(
            &mut search,
            vec![egui::Event::PointerMoved(start), button(start, true)],
        );
        frame(&mut search, vec![egui::Event::PointerMoved(end)]);
        frame(&mut search, vec![button(end, false)]);
        let selected = frame(&mut search, vec![egui::Event::Copy]).1.unwrap();
        assert_eq!(selected, "select this needle");
        search.query = "needle".into();
        assert_eq!(
            frame(&mut search, vec![egui::Event::Copy]).1,
            Some(selected.clone())
        );
        let escape = egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        assert_eq!(
            frame(&mut search, vec![escape, egui::Event::Copy]).1,
            Some(selected)
        );
        assert!(!search.needs_report());
    }
    #[test]
    fn reopening_search_selects_existing_unicode_query_for_replacement() {
        let ctx = egui::Context::default();
        let mut search = Search::default();
        let frame = |search: &mut Search, events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    if search.open {
                        egui::CentralPanel::default().show(ui, |ui| search.bar(ui));
                    }
                },
            );
            output.textures_delta.clear();
        };
        search.begin();
        frame(&mut search, vec![]);
        frame(&mut search, vec![egui::Event::Text("Écho 日本語".into())]);
        frame(
            &mut search,
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(!search.open);
        assert_eq!(search.query, "Écho 日本語");
        search.begin();
        frame(&mut search, vec![]);
        frame(&mut search, vec![egui::Event::Text("replacement".into())]);
        assert_eq!(search.query, "replacement");
    }
    #[test]
    fn slash_opens_and_reopens_search_on_shifted_keyboard_layouts() {
        let ctx = egui::Context::default();
        let mut search = Search::default();
        let key = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        for events in [
            vec![
                key(egui::Key::Slash, egui::Modifiers::NONE),
                egui::Event::Text("/".into()),
            ],
            vec![
                key(egui::Key::Num7, egui::Modifiers::SHIFT),
                egui::Event::Text("/".into()),
            ],
        ] {
            search.clear(&ctx);
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    assert!(search.open_shortcut(ui.ctx()));
                    egui::CentralPanel::default().show(ui, |ui| search.bar(ui));
                },
            );
            output.textures_delta.clear();
            assert!(search.open);
            assert!(ctx.text_edit_focused());
            assert!(search.query.is_empty());
        }
    }

    #[test]
    fn enter_closes_search_without_advancing_and_n_navigates_matches() {
        let ctx = egui::Context::default();
        let mut search = Search::default();
        search.begin();
        let mut send = |events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    search.shortcuts(ui.ctx());
                    egui::CentralPanel::default().show(ui, |ui| search.bar(ui));
                },
            );
            output.textures_delta.clear();
        };
        send(vec![]);
        send(vec![egui::Event::Text("needle".into())]);
        send(vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        send(vec![egui::Event::Text(" more".into())]);
        drop(send);
        assert!(!search.open);
        assert!(search.move_match.is_none());
        let mut send = |events| {
            let _ = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| search.shortcuts(ui.ctx()),
            );
        };
        assert!(!ctx.text_edit_focused());
        send(vec![egui::Event::Key {
            key: egui::Key::N,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        assert_eq!(search.move_match.take(), Some(true));
        let _ = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::N,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::SHIFT,
                }],
                ..Default::default()
            },
            |ui| search.shortcuts(ui.ctx()),
        );
        assert_eq!(search.move_match.take(), Some(false));
        assert_eq!(search.query, "needle");
        search.clear(&ctx);
        assert!(!search.open);
        assert!(!ctx.text_edit_focused());
    }
    #[test]
    fn offscreen_match_and_heading_navigation_scroll_actual_document() {
        let ctx = egui::Context::default();
        let text = format!(
            "# First\n\n{}\n\n# Last heading\n\nneedle at the end",
            "A paragraph of filler.\n\n".repeat(100)
        );
        let mut cache = egui_commonmark::CommonMarkCache::default();
        let mut search = Search::default();
        let mut frame = |search: &mut Search| {
            let mut offset = 0.0;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(600.0, 300.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        let out = egui::ScrollArea::vertical().animated(false).show(ui, |ui| {
                            let (_, report) = egui_commonmark::CommonMarkViewer::new()
                                .show_with_report(ui, &mut cache, &text);
                            search.render(ui, &TextIndex::new(&report), ui.clip_rect().top());
                        });
                        offset = out.state.offset.y;
                    });
                },
            );
            output.textures_delta.clear();
            offset
        };
        frame(&mut search);
        search.heading_step = Some(true);
        assert!(frame(&mut search) < 20.0);
        assert_eq!(search.heading, Some(0));
        search.heading_step = Some(true);
        assert!(frame(&mut search) > 1000.0);
        assert_eq!(search.heading, Some(1));
        search.heading_step = Some(false);
        assert!(frame(&mut search) < 20.0);
        search.query = "needle".into();
        assert!(frame(&mut search) > 1000.0);
        assert_eq!(search.count, 1);
        search.clear(&ctx);
        assert!(!search.needs_report());
    }
    #[test]
    fn report_indexes_offscreen_code_links_and_parsed_headings() {
        let ctx = egui::Context::default();
        let text = "# First **heading**\n\nA **formatted** phrase and [link](https://example.com).\n\n```rust\nlet needle = 1;\n# not a heading\n```\n\nSecond\n---\n";
        let mut cache = egui_commonmark::CommonMarkCache::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                let (_, report) =
                    egui_commonmark::CommonMarkViewer::new().show_with_report(ui, &mut cache, text);
                let index = TextIndex::new(&report);
                assert_eq!(index.headings.len(), 2);
                assert!(index.text.contains("formatted phrase"));
                assert!(index.text.contains("link"));
                assert!(index.text.contains("needle"));
                assert_eq!(find_matches(&index.text, "First heading").len(), 1);
                let mut search = Search::default();
                search.target("needle".into(), text.find("needle").unwrap());
                search.render(ui, &index, 0.0);
                assert_eq!(search.count, 1);
                search.clear(ui.ctx());
                assert!(search.query.is_empty());
                assert!(search.heading.is_none());
            });
        });
        output.textures_delta.clear();
    }
}
