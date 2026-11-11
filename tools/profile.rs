//! Repeatable headless CPU timings; excludes compositor/window/GPU startup.
use crate::{document::Document, reading_cursor::ReadingCursor, text_search::TextIndex};
use eframe::egui;
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use std::time::Instant;

pub fn run() {
    let path = std::env::args_os().nth(2).expect("--profile FILE [FRAMES]");
    let frames = std::env::args()
        .nth(3)
        .map(|s| s.parse::<usize>().unwrap())
        .unwrap_or(3);
    let start = Instant::now();
    let doc = Document::open(path.into()).unwrap();
    println!(
        "bytes={} read_ms={:.3}",
        doc.text.len(),
        start.elapsed().as_secs_f64() * 1000.0
    );
    let start = Instant::now();
    let mut fonts = fontdb::Database::new();
    fonts.load_system_fonts();
    println!("font_scan_ms={:.3}", start.elapsed().as_secs_f64() * 1000.0);
    let ctx = egui::Context::default();
    let mut cache = CommonMarkCache::default();
    let mut cursor = ReadingCursor::default();
    let mut index = TextIndex::default();
    for frame in 0..frames {
        let start = Instant::now();
        let mut output = ctx.run_ui(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0,800.0))),
            ..Default::default()
        }, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                egui::ScrollArea::both().show(ui, |ui| {
                    ui.set_width(760.0);
                    let render = Instant::now();
                    let (_, report) = CommonMarkViewer::new().show_with_report(ui, &mut cache, &doc.text);
                    let render_ms = render.elapsed().as_secs_f64()*1000.0;
                    let index_start = Instant::now();
                    index.update(&report);
                    let index_ms = index_start.elapsed().as_secs_f64()*1000.0;
                    let cursor_start = Instant::now();
                    cursor.render(ui, &index);
                    println!("frame={frame} render_ms={render_ms:.3} index_ms={index_ms:.3} cursor_ms={:.3}", cursor_start.elapsed().as_secs_f64()*1000.0);
                });
            });
        });
        output.textures_delta.clear();
        println!(
            "frame={frame} total_ms={:.3}",
            start.elapsed().as_secs_f64() * 1000.0
        );
    }
}
