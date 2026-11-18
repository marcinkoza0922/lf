//! `LF_SCREENSHOT=out.ppm lf FILE` captures the window after it settles, writes a
//! binary PPM, and exits. Used to check rendering without depending on window focus.
//! `LF_SCREENSHOT_SIDEBAR=opened|files` shows a sidebar tab. Preferences are read
//! but never saved in this mode, so captures do not alter recent files.
use eframe::egui;
use std::{io::Write, path::PathBuf};

pub struct Screenshot {
    path: PathBuf,
    frames: u32,
}
impl Screenshot {
    pub fn from_env() -> Option<Self> {
        let path = std::env::var_os("LF_SCREENSHOT")?;
        Some(Self {
            path: path.into(),
            frames: 0,
        })
    }
    /// The sidebar tab requested for the capture.
    pub fn sidebar() -> Option<crate::sidebar::Tab> {
        match std::env::var("LF_SCREENSHOT_SIDEBAR").ok()?.as_str() {
            "opened" => Some(crate::sidebar::Tab::Opened),
            "files" => Some(crate::sidebar::Tab::Files),
            _ => None,
        }
    }
    /// Call once per frame. Requests the capture after layout and images settle.
    pub fn update(&mut self, ctx: &egui::Context) {
        self.frames += 1;
        ctx.request_repaint();
        if self.frames == 20 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        let image = ctx.input(|i| {
            i.events.iter().find_map(|event| match event {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = image {
            let mut data = format!("P6\n{} {}\n255\n", image.size[0], image.size[1]).into_bytes();
            for pixel in &image.pixels {
                data.extend_from_slice(&pixel.to_srgba_unmultiplied()[..3]);
            }
            let result = std::fs::File::create(&self.path).and_then(|mut f| f.write_all(&data));
            if let Err(error) = result {
                eprintln!("screenshot: {error}");
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}
