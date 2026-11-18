//! The Ctrl+H popup: the document's headings, filtered by a fuzzy query.
use crate::{quick_select::fuzzy_rank, text_search::TextIndex};
use eframe::egui;

struct Entry {
    id: usize,
    level: u8,
    text: String,
}

pub struct HeadingJump {
    entries: Vec<Entry>,
    /// The entry at the reading position, preselected while the query is empty.
    current: usize,
    pub query: String,
    filtered_query: Option<String>,
    matches: Vec<usize>,
    selected: usize,
    scroll_to_selection: bool,
    focus_search: bool,
}

pub enum Action {
    Stay,
    Close,
    /// Jump to the heading with this report identity.
    Jump(usize),
}

impl HeadingJump {
    /// `levels` is indexed by heading identity; `viewport_top` is the screen
    /// position of the document viewport's top edge in the index's coordinates.
    pub fn new(index: &TextIndex, levels: &[u8], viewport_top: f32) -> Self {
        let headings = index.heading_texts();
        let current = headings
            .iter()
            .rposition(|(_, rect, _)| rect.top() <= viewport_top + 2.0)
            .unwrap_or(0);
        let entries = headings
            .into_iter()
            .map(|(id, _, text)| Entry {
                id,
                level: levels.get(id).copied().unwrap_or(1).clamp(1, 6),
                text: text.split_whitespace().collect::<Vec<_>>().join(" "),
            })
            .collect();
        Self {
            entries,
            current,
            query: String::new(),
            filtered_query: None,
            matches: Vec::new(),
            selected: 0,
            scroll_to_selection: true,
            focus_search: true,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Action {
        let mut action = Action::Stay;
        egui::Modal::new(egui::Id::new("heading-jump")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 64.0).clamp(240.0, 640.0));
            ui.heading("Go to heading");
            ui.small(
                "Type to filter · Up/Down or Tab/Shift+Tab to select · Enter to jump · Esc to close",
            );
            ui.separator();
            let (close, previous, next, open) = ctx.input_mut(|i| {
                (
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
                        | i.consume_key(egui::Modifiers::CTRL, egui::Key::H),
                    i.consume_key(egui::Modifiers::SHIFT, egui::Key::Tab)
                        | i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Tab)
                        | i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Enter),
                )
            });
            if close {
                action = Action::Close;
            }
            let search = ui.add(
                egui::TextEdit::singleline(&mut self.query)
                    .id(ui.make_persistent_id("heading-jump-query"))
                    .lock_focus(true)
                    .hint_text("Find a heading…")
                    .desired_width(f32::INFINITY),
            );
            if self.focus_search || previous || next || !search.has_focus() {
                search.request_focus();
                self.focus_search = false;
            }
            if self.filtered_query.as_deref() != Some(&self.query) {
                self.matches = fuzzy_rank(self.entries.iter().map(|e| &e.text), &self.query);
                self.selected = if self.query.trim().is_empty() {
                    self.current.min(self.matches.len().saturating_sub(1))
                } else {
                    0
                };
                self.filtered_query = Some(self.query.clone());
                self.scroll_to_selection = true;
            }
            if (previous || next) && !self.matches.is_empty() {
                self.selected = if next {
                    (self.selected + 1).min(self.matches.len() - 1)
                } else {
                    self.selected.saturating_sub(1)
                };
                self.scroll_to_selection = true;
            }
            if open
                && matches!(action, Action::Stay)
                && let Some(&entry) = self.matches.get(self.selected)
            {
                action = Action::Jump(self.entries[entry].id);
            }
            if self.entries.is_empty() {
                ui.label("This document has no headings.");
            } else if self.matches.is_empty() {
                ui.label("No matching headings.");
            } else {
                ui.small(format!(
                    "{} of {} headings",
                    self.matches.len(),
                    self.entries.len()
                ));
                let row_height = ui
                    .text_style_height(&egui::TextStyle::Body)
                    .max(ui.spacing().interact_size.y);
                let row_stride = row_height + ui.spacing().item_spacing.y;
                let max_height = (ctx.content_rect().height() * 0.55).max(100.0);
                let mut scroll = egui::ScrollArea::vertical()
                    .id_salt("heading-jump-list")
                    .max_height(max_height);
                if self.scroll_to_selection {
                    scroll = scroll.vertical_scroll_offset(
                        (self.selected as f32 * row_stride - max_height / 2.0).max(0.0),
                    );
                    self.scroll_to_selection = false;
                }
                scroll.show_rows(ui, row_height, self.matches.len(), |ui, rows| {
                    for row in rows {
                        let entry = &self.entries[self.matches[row]];
                        ui.horizontal(|ui| {
                            ui.add_space(f32::from(entry.level - 1) * 14.0);
                            let label = egui::RichText::new(&entry.text);
                            let label = if entry.level == 1 {
                                label.strong()
                            } else {
                                label
                            };
                            if ui
                                .add(
                                    // Not add_sized, which would center the label.
                                    egui::Button::selectable(self.selected == row, label)
                                        .truncate()
                                        .right_text(
                                            egui::RichText::new(format!("H{}", entry.level))
                                                .small()
                                                .weak(),
                                        )
                                        .min_size(egui::vec2(ui.available_width(), row_height)),
                                )
                                .on_hover_text(&entry.text)
                                .clicked()
                            {
                                action = Action::Jump(entry.id);
                            }
                        });
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

#[cfg(test)]
mod tests {
    use super::*;

    fn index(text: &str) -> (TextIndex, Vec<u8>) {
        let ctx = egui::Context::default();
        let mut index = TextIndex::default();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let mut cache = egui_commonmark::CommonMarkCache::default();
            let report = egui_commonmark::CommonMarkViewer::new()
                .show_with_report(ui, &mut cache, text)
                .1;
            index = TextIndex::new(&report);
        });
        output.textures_delta.clear();
        (index, egui_commonmark::heading_levels(text))
    }

    fn run(popup: &mut HeadingJump, events: Vec<egui::Event>) -> Action {
        let ctx = egui::Context::default();
        let mut action = Action::Stay;
        // Two frames: the first takes focus, the second delivers the input.
        for events in [Vec::new(), events] {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| action = popup.show(ui.ctx()),
            );
            output.textures_delta.clear();
        }
        action
    }

    fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    const DOC: &str = "# Guide\n\nIntro.\n\n## Install steps\n\nText.\n\n```\n# not a heading\n```\n\nSetup\n-----\n\n### Usage *notes*\n";

    #[test]
    fn lists_parsed_headings_with_levels_in_document_order() {
        let (index, levels) = index(DOC);
        assert_eq!(levels, vec![1, 2, 2, 3]);
        let popup = HeadingJump::new(&index, &levels, f32::NEG_INFINITY);
        let listed: Vec<_> = popup
            .entries
            .iter()
            .map(|e| (e.id, e.level, e.text.as_str()))
            .collect();
        assert_eq!(
            listed,
            vec![
                (0, 1, "Guide"),
                (1, 2, "Install steps"),
                (2, 2, "Setup"),
                (3, 3, "Usage notes"),
            ]
        );
    }

    #[test]
    fn fuzzy_query_and_keys_select_the_jump_target() {
        let (index, levels) = index(DOC);
        let mut popup = HeadingJump::new(&index, &levels, f32::NEG_INFINITY);
        // Empty query: Enter jumps to the first heading; arrows move through the list.
        let none = egui::Modifiers::NONE;
        assert!(matches!(
            run(&mut popup, vec![key(egui::Key::ArrowDown, none)]),
            Action::Stay
        ));
        assert!(matches!(
            run(&mut popup, vec![key(egui::Key::Enter, none)]),
            Action::Jump(1)
        ));
        // "stp" matches "Setup" and "Install steps"; the shorter, word-initial match ranks first.
        run(&mut popup, vec![egui::Event::Text("stp".into())]);
        assert_eq!(popup.matches, vec![2, 1]);
        assert!(matches!(
            run(&mut popup, vec![key(egui::Key::Tab, none)]),
            Action::Stay
        ));
        assert!(matches!(
            run(&mut popup, vec![key(egui::Key::Enter, none)]),
            Action::Jump(1)
        ));
        run(&mut popup, vec![egui::Event::Text("zzz".into())]);
        assert!(matches!(
            run(&mut popup, vec![key(egui::Key::Enter, none)]),
            Action::Stay
        ));
        assert!(matches!(
            run(&mut popup, vec![key(egui::Key::H, egui::Modifiers::CTRL)]),
            Action::Close
        ));
        assert!(matches!(
            run(&mut popup, vec![key(egui::Key::Escape, none)]),
            Action::Close
        ));
    }

    #[test]
    fn empty_query_preselects_the_heading_at_the_reading_position() {
        let (index, levels) = index(DOC);
        let setup = index.headings[2].1.top();
        let mut popup = HeadingJump::new(&index, &levels, setup + 5.0);
        assert!(matches!(
            run(
                &mut popup,
                vec![key(egui::Key::Enter, egui::Modifiers::NONE)]
            ),
            Action::Jump(2)
        ));
    }
}
