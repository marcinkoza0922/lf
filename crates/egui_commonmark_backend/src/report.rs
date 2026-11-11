//! Read-only layout observations used for search overlays and heading navigation.
use egui::{Context, Galley, Pos2, Ui, WidgetText};
use std::{
    ops::Range,
    sync::{Arc, Mutex},
};

#[derive(Clone)]
pub struct TextRun {
    pub galley: Arc<Galley>,
    pub pos: Pos2,
    pub source: Range<usize>,
    pub heading: Option<usize>,
}
#[derive(Clone, Default)]
pub struct RenderReport {
    pub runs: Vec<TextRun>,
}
#[derive(Default)]
struct Recording {
    report: RenderReport,
    source: Range<usize>,
    heading: Option<usize>,
}
type Shared = Arc<Mutex<Recording>>;
fn id() -> egui::Id {
    egui::Id::new("markdown-layout-report")
}
pub fn begin(ctx: &Context) {
    ctx.data_mut(|d| d.insert_temp(id(), Shared::default()));
}
pub fn finish(ctx: &Context) -> RenderReport {
    let state = ctx.data_mut(|d| d.remove_temp::<Shared>(id()));
    state
        .map(|s| std::mem::take(&mut s.lock().unwrap().report))
        .unwrap_or_default()
}
pub fn location(ctx: &Context, source: Range<usize>, heading: Option<usize>) {
    if let Some(shared) = ctx.data(|d| d.get_temp::<Shared>(id())) {
        let mut state = shared.lock().unwrap();
        state.source = source;
        state.heading = heading;
    }
}
pub fn record(ui: &Ui, galley: Arc<Galley>, pos: Pos2) {
    if let Some(shared) = ui.ctx().data(|d| d.get_temp::<Shared>(id())) {
        let mut state = shared.lock().unwrap();
        let run = TextRun {
            galley,
            pos,
            source: state.source.clone(),
            heading: state.heading,
        };
        state.report.runs.push(run);
    }
}
pub fn label(ui: &mut Ui, text: impl Into<WidgetText>) -> egui::Response {
    draw(ui, text.into(), false)
}
pub fn link(ui: &mut Ui, text: impl Into<WidgetText>) -> egui::Response {
    draw(ui, text.into(), true)
}
fn draw(ui: &mut Ui, text: WidgetText, link: bool) -> egui::Response {
    let mut label = egui::Label::new(text);
    if link {
        label = label.sense(egui::Sense::click());
    }
    let (pos, galley, response) = label.layout_in_ui(ui);
    record(ui, galley.clone(), pos);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            if link {
                egui::WidgetType::Link
            } else {
                egui::WidgetType::Label
            },
            ui.is_enabled(),
            galley.text(),
        )
    });
    if ui.is_rect_visible(response.rect) {
        let color = if link {
            ui.visuals().hyperlink_color
        } else {
            ui.visuals().text_color()
        };
        let underline = if link && (response.hovered() || response.has_focus()) {
            egui::Stroke::new(ui.style().interact(&response).fg_stroke.width, color)
        } else {
            egui::Stroke::NONE
        };
        if ui.style().interaction.selectable_labels {
            egui::text_selection::LabelSelectionState::label_text_selection(
                ui, &response, pos, galley, color, underline,
            );
        } else {
            ui.painter()
                .add(egui::epaint::TextShape::new(pos, galley, color).with_underline(underline));
        }
        if link && response.hovered() {
            ui.set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }
    response
}
