use eframe::egui::{self, Color32};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    Paper,
    SolarizedLight,
    GruvboxLight,
    Dracula,
    GruvboxDark,
    Nord,
    SolarizedDark,
    TokyoNight,
}
impl Theme {
    pub const ALL: [Self; 8] = [
        Self::Paper,
        Self::SolarizedLight,
        Self::GruvboxLight,
        Self::Dracula,
        Self::GruvboxDark,
        Self::Nord,
        Self::SolarizedDark,
        Self::TokyoNight,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Paper => "Paper",
            Self::SolarizedLight => "Solarized Light",
            Self::GruvboxLight => "Gruvbox Light",
            Self::Dracula => "Dracula",
            Self::GruvboxDark => "Gruvbox Dark",
            Self::Nord => "Nord",
            Self::SolarizedDark => "Solarized Dark",
            Self::TokyoNight => "Tokyo Night",
        }
    }
    pub fn visuals(self) -> egui::Visuals {
        let (dark, bg, fg, accent) = match self {
            Self::Paper => (false, 0xfaf9f6, 0x292b30, 0x3467a5),
            Self::SolarizedLight => (false, 0xfdf6e3, 0x586e75, 0x268bd2),
            Self::GruvboxLight => (false, 0xfbf1c7, 0x3c3836, 0x9d0006),
            Self::Dracula => (true, 0x282a36, 0xf8f8f2, 0xbd93f9),
            Self::GruvboxDark => (true, 0x282828, 0xebdbb2, 0xfabd2f),
            Self::Nord => (true, 0x2e3440, 0xe5e9f0, 0x88c0d0),
            Self::SolarizedDark => (true, 0x002b36, 0x93a1a1, 0x2aa198),
            Self::TokyoNight => (true, 0x1a1b26, 0xc0caf5, 0x7aa2f7),
        };
        let mut v = if dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        v.panel_fill = color(bg);
        v.window_fill = color(bg);
        v.override_text_color = Some(color(fg));
        v.hyperlink_color = color(accent);
        v.selection.bg_fill = color(accent).gamma_multiply(0.4);
        v
    }
}
fn color(n: u32) -> Color32 {
    Color32::from_rgb((n >> 16) as u8, (n >> 8) as u8, n as u8)
}

pub fn is_day(hour: u32, start: u32, end: u32) -> bool {
    if start <= end {
        (start..end).contains(&hour)
    } else {
        hour >= start || hour < end
    }
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub automatic: bool,
    pub auto_reload: bool,
    pub manual: Theme,
    pub day: Theme,
    pub night: Theme,
    pub day_start: u32,
    pub night_start: u32,
    pub size: f32,
    pub font: String,
    pub line_height: f32,
    pub paragraph_gap: f32,
    pub width: f32,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            automatic: true,
            auto_reload: false,
            manual: Theme::Dracula,
            day: Theme::Paper,
            night: Theme::Dracula,
            day_start: 7,
            night_start: 19,
            size: 18.0,
            font: "Built-in".into(),
            line_height: 1.4,
            paragraph_gap: 10.0,
            width: 850.0,
        }
    }
}
impl Preferences {
    pub fn theme(&self, hour: u32) -> Theme {
        if !self.automatic {
            self.manual
        } else if is_day(hour, self.day_start, self.night_start) {
            self.day
        } else {
            self.night
        }
    }
    pub fn sanitize(&mut self) {
        self.line_height = finite_clamp(self.line_height, 1.0, 2.4, 1.4);
        self.paragraph_gap = finite_clamp(self.paragraph_gap, 0.0, 40.0, 10.0);
        self.width = finite_clamp(self.width, 360.0, 1400.0, 850.0);
        self.day_start = self.day_start.min(23);
        self.night_start = self.night_start.min(23);
        self.size = if self.size.is_finite() {
            self.size.clamp(10.0, 36.0)
        } else {
            18.0
        };
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn schedule_boundaries() {
        assert!(!is_day(6, 7, 19));
        assert!(is_day(7, 7, 19));
        assert!(is_day(18, 7, 19));
        assert!(!is_day(19, 7, 19));
        assert!(is_day(23, 19, 7));
        assert!(is_day(0, 19, 7));
        assert!(!is_day(7, 19, 7));
        assert!(!is_day(12, 7, 7));
    }
    #[test]
    fn manual_overrides_schedule() {
        let p = Preferences {
            automatic: false,
            manual: Theme::Nord,
            ..Default::default()
        };
        assert_eq!(p.theme(12), Theme::Nord);
    }
}

fn finite_clamp(value: f32, min: f32, max: f32, default: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        default
    }
}
