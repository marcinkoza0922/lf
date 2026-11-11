use eframe::egui::{self, Event, Key, Modifiers, Vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    Top,
    Bottom,
    CursorFirst,
    CursorLastLine,
    ViewportTop,
    ViewportBottom,
    LineStart,
    LineEnd,
}

#[derive(Default)]
pub struct Navigator {
    pending_g: Option<f64>,
}

impl Navigator {
    pub fn word_end_reserved(&self, _ctx: &egui::Context) -> bool {
        self.pending_g.is_some()
    }

    fn palette(&mut self, ctx: &egui::Context) -> Option<Command> {
        self.pending_g?;
        let mut selected = None;
        let mut open = true;
        egui::Window::new("Go to")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                for (shortcut, label, command) in [
                    ("gg", "First non-whitespace character", Command::CursorFirst),
                    (
                        "ge / Shift+G",
                        "Beginning of last line",
                        Command::CursorLastLine,
                    ),
                    ("gt", "Top of viewport", Command::ViewportTop),
                    ("gb", "Bottom of viewport", Command::ViewportBottom),
                    ("gh", "Beginning of line", Command::LineStart),
                    ("gl", "End of line", Command::LineEnd),
                ] {
                    if ui.button(format!("{shortcut}   {label}")).clicked() {
                        selected = Some(command);
                    }
                }
                ui.weak("Escape to close");
            });
        if !open || selected.is_some() {
            self.reset();
        }
        selected
    }

    pub fn reset(&mut self) {
        self.pending_g = None;
    }

    pub fn commands(&mut self, ctx: &egui::Context, enabled: bool) -> Vec<Command> {
        if !enabled {
            self.reset();
            return Vec::new();
        }
        let mut commands = ctx.input_mut(|input| {
            let now = input.time;
            let mut commands = Vec::new();
            input.events.retain(|event| {
                let Event::Key {
                    key,
                    modifiers,
                    pressed: true,
                    repeat,
                    ..
                } = event
                else {
                    return true;
                };
                let (handled, command) = self.key(*key, *modifiers, *repeat, now);
                commands.extend(command);
                !handled
            });
            commands
        });
        commands.extend(self.palette(ctx));
        commands
    }

    fn key(
        &mut self,
        key: Key,
        modifiers: Modifiers,
        repeat: bool,
        now: f64,
    ) -> (bool, Option<Command>) {
        if modifiers.ctrl || modifiers.alt || modifiers.command || modifiers.mac_cmd {
            self.reset();
            return (false, None);
        }
        let pending = self.pending_g.is_some();
        let command = match key {
            Key::G if modifiers.shift => Some(Command::CursorLastLine),
            Key::G => {
                if repeat {
                    return (true, None);
                }
                self.pending_g = if pending { None } else { Some(now) };
                return (true, pending.then_some(Command::CursorFirst));
            }
            Key::E if pending && !modifiers.shift => Some(Command::CursorLastLine),
            Key::T if pending && !modifiers.shift => Some(Command::ViewportTop),
            Key::B if pending && !modifiers.shift => Some(Command::ViewportBottom),
            Key::H if pending && !modifiers.shift => Some(Command::LineStart),
            Key::L if pending && !modifiers.shift => Some(Command::LineEnd),
            Key::Escape if pending => {
                self.reset();
                return (true, None);
            }
            Key::Home => Some(Command::Top),
            Key::End => Some(Command::Bottom),
            Key::ArrowUp | Key::K => Some(Command::Up),
            Key::ArrowDown | Key::J => Some(Command::Down),
            Key::ArrowLeft | Key::H => Some(Command::Left),
            Key::ArrowRight | Key::L => Some(Command::Right),
            Key::PageUp => Some(Command::PageUp),
            Key::PageDown => Some(Command::PageDown),
            _ => None,
        };
        self.reset();
        (command.is_some(), command)
    }
}

pub fn apply(ui: &egui::Ui, viewport: egui::Rect, commands: &[Command], line: f32) {
    if commands.is_empty() {
        return;
    }
    let offset = viewport.min.to_vec2();
    let target = target_offset(offset, ui.min_size(), viewport.size(), commands, line);
    ui.scroll_with_delta_animation(offset - target, egui::style::ScrollAnimation::none());
}

fn target_offset(
    mut offset: Vec2,
    content: Vec2,
    viewport: Vec2,
    commands: &[Command],
    line: f32,
) -> Vec2 {
    let max = (content - viewport).max(Vec2::ZERO);
    let page = (viewport.y * 0.9).max(line);
    for command in commands {
        match command {
            Command::Up => offset.y -= line,
            Command::Down => offset.y += line,
            Command::Left => offset.x -= line,
            Command::Right => offset.x += line,
            Command::PageUp => offset.y -= page,
            Command::PageDown => offset.y += page,
            Command::Top => offset.y = 0.0,
            Command::Bottom => offset.y = max.y,
            Command::CursorFirst
            | Command::CursorLastLine
            | Command::ViewportTop
            | Command::ViewportBottom
            | Command::LineStart
            | Command::LineEnd => {}
        }
        offset = offset.clamp(Vec2::ZERO, max);
    }
    offset
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sequences_reset_and_ignore_g_repeat() {
        let mut nav = Navigator::default();
        let none = Modifiers::NONE;
        assert_eq!(nav.key(Key::G, none, false, 0.0), (true, None));
        assert_eq!(nav.key(Key::G, none, true, 0.1), (true, None));
        assert_eq!(
            nav.key(Key::G, none, false, 0.2).1,
            Some(Command::CursorFirst)
        );
        nav.key(Key::G, none, false, 1.0);
        assert_eq!(
            nav.key(Key::E, none, false, 1.1).1,
            Some(Command::CursorLastLine)
        );
        assert_eq!(
            nav.key(Key::G, Modifiers::SHIFT, false, 2.0).1,
            Some(Command::CursorLastLine)
        );
        nav.key(Key::G, none, false, 3.0);
        assert_eq!(
            nav.key(Key::E, none, false, 4.1).1,
            Some(Command::CursorLastLine)
        );
        nav.key(Key::G, none, false, 5.0);
        nav.key(Key::X, none, false, 5.1);
        assert_eq!(nav.key(Key::E, none, false, 5.2), (false, None));
        nav.key(Key::G, none, false, 6.0);
        assert_eq!(nav.key(Key::J, Modifiers::CTRL, false, 6.1), (false, None));
        assert_eq!(nav.key(Key::E, none, false, 6.2), (false, None));
    }

    #[test]
    fn modal_suppression_clears_prefix_and_preserves_events() {
        let ctx = egui::Context::default();
        let mut nav = Navigator::default();
        nav.key(Key::G, Modifiers::NONE, false, 0.0);
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![Event::Key {
                    key: Key::J,
                    physical_key: None,
                    modifiers: Modifiers::NONE,
                    pressed: true,
                    repeat: false,
                }],
                ..Default::default()
            },
            |ui| {
                assert!(nav.commands(ui.ctx(), false).is_empty());
                assert!(ui.input(|i| i.key_pressed(Key::J)));
            },
        );
        output.textures_delta.clear();
        assert_eq!(nav.key(Key::E, Modifiers::NONE, false, 0.5), (false, None));
    }

    #[test]
    fn keys_scroll_actual_area_and_clamp_to_its_bounds() {
        let ctx = egui::Context::default();
        let mut nav = Navigator::default();
        let mut frame = |key: Option<Key>, content: Vec2| {
            let mut offset = Vec2::ZERO;
            let events = key
                .map(|key| Event::Key {
                    key,
                    physical_key: None,
                    modifiers: Modifiers::NONE,
                    pressed: true,
                    repeat: false,
                })
                .into_iter()
                .collect();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(400.0, 300.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let commands = nav.commands(ui.ctx(), true);
                    egui::CentralPanel::default().show(ui, |ui| {
                        let result = egui::ScrollArea::both()
                            .animated(false)
                            .auto_shrink([false, false])
                            .show_viewport(ui, |ui, viewport| {
                                ui.allocate_exact_size(content, egui::Sense::hover());
                                apply(ui, viewport, &commands, 24.0);
                            });
                        offset = result.state.offset;
                    });
                },
            );
            output.textures_delta.clear();
            offset
        };
        let content = egui::vec2(800.0, 2000.0);
        frame(None, content);
        assert_eq!(frame(Some(Key::J), content).y, 24.0);
        assert_eq!(frame(Some(Key::ArrowDown), content).y, 48.0);
        assert_eq!(frame(Some(Key::K), content).y, 24.0);
        assert_eq!(frame(Some(Key::ArrowUp), content).y, 0.0);
        assert_eq!(frame(Some(Key::L), content).x, 24.0);
        assert_eq!(frame(Some(Key::ArrowRight), content).x, 48.0);
        assert_eq!(frame(Some(Key::H), content).x, 24.0);
        assert_eq!(frame(Some(Key::ArrowLeft), content).x, 0.0);
        assert!(frame(Some(Key::PageDown), content).y > 200.0);
        assert_eq!(frame(Some(Key::PageUp), content).y, 0.0);
        let end = frame(Some(Key::End), content).y;
        assert!(end > 1700.0);
        assert_eq!(frame(Some(Key::PageDown), content).y, end);
        assert_eq!(frame(Some(Key::Home), content).y, 0.0);
        assert_eq!(frame(Some(Key::End), egui::vec2(50.0, 50.0)), Vec2::ZERO);
    }
}
