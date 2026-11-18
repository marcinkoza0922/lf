//! Back/forward navigation between documents and heading-link jumps.
use crate::reading_cursor::ReadingCursor;
use eframe::egui;
use std::path::PathBuf;

const LIMIT: usize = 200;

/// A place the reader has been: a document with its scroll position and cursor.
#[derive(Clone)]
pub struct Location {
    pub path: PathBuf,
    pub scroll: egui::Vec2,
    pub cursor: ReadingCursor,
}

#[derive(Default)]
pub struct History {
    back: Vec<Location>,
    forward: Vec<Location>,
}
impl History {
    /// Record the place being left for a new destination; forward history is dropped.
    pub fn visit(&mut self, from: Location) {
        self.back.push(from);
        if self.back.len() > LIMIT {
            self.back.remove(0);
        }
        self.forward.clear();
    }
    /// Step back (or forward) from `current`, returning the destination.
    pub fn step(&mut self, back: bool, current: Location) -> Option<Location> {
        let (from, to) = if back {
            (&mut self.back, &mut self.forward)
        } else {
            (&mut self.forward, &mut self.back)
        };
        let target = from.pop()?;
        to.push(current);
        Some(target)
    }
    /// Undo the bookkeeping of a step whose destination could not be opened. The
    /// unreachable destination is dropped, so stepping again continues past it.
    pub fn cancel(&mut self, back: bool) {
        if back {
            self.forward.pop();
        } else {
            self.back.pop();
        }
    }
    pub fn can_step(&self, back: bool) -> bool {
        if back {
            !self.back.is_empty()
        } else {
            !self.forward.is_empty()
        }
    }
}

/// GitHub-style heading anchors: lowercase, punctuation removed except `-` and `_`,
/// spaces become `-`; repeated slugs get `-1`, `-2`, … suffixes.
pub fn slugs<'a>(headings: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut seen = std::collections::HashMap::<String, usize>::new();
    headings
        .into_iter()
        .map(|heading| {
            let base: String = heading
                .trim()
                .chars()
                .flat_map(char::to_lowercase)
                .filter_map(|c| match c {
                    ' ' => Some('-'),
                    c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
                    _ => None,
                })
                .collect();
            let count = seen.entry(base.clone()).or_default();
            let slug = if *count == 0 {
                base.clone()
            } else {
                format!("{base}-{count}")
            };
            *count += 1;
            slug
        })
        .collect()
}

/// The heading a link fragment points at: an exact GitHub-style slug, or text that
/// slugifies to the same anchor (so `#Setup Steps` finds "Setup steps").
pub fn find_heading(
    index: &crate::text_search::TextIndex,
    fragment: &str,
) -> Option<(usize, egui::Rect)> {
    let headings = index.heading_texts();
    let slugs = slugs(headings.iter().map(|(_, _, text)| text.as_str()));
    let wanted = [fragment.to_string(), slugs_of(fragment)];
    headings
        .iter()
        .zip(&slugs)
        .find(|(_, slug)| wanted.contains(slug))
        .map(|((id, rect, _), _)| (*id, *rect))
}
fn slugs_of(text: &str) -> String {
    slugs([text]).pop().unwrap_or_default()
}

/// Decode `%XX` escapes in a URL fragment.
pub fn decode_fragment(fragment: &str) -> String {
    let bytes = fragment.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%'
            && let (Some(high), Some(low)) = (
                bytes.get(i + 1).copied().and_then(hex),
                bytes.get(i + 2).copied().and_then(hex),
            )
        {
            out.push((high * 16 + low) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn at(name: &str, y: f32) -> Location {
        Location {
            path: name.into(),
            scroll: egui::vec2(0.0, y),
            cursor: ReadingCursor::default(),
        }
    }
    #[test]
    fn back_and_forward_restore_positions_and_visits_drop_forward() {
        let mut history = History::default();
        assert!(!history.can_step(true));
        history.visit(at("a", 10.0));
        history.visit(at("b", 20.0));
        let back = history.step(true, at("c", 30.0)).unwrap();
        assert_eq!((back.path, back.scroll.y), ("b".into(), 20.0));
        let forward = history.step(false, at("b", 25.0)).unwrap();
        assert_eq!((forward.path, forward.scroll.y), ("c".into(), 30.0));
        let back = history.step(true, at("c", 30.0)).unwrap();
        assert_eq!(back.scroll.y, 25.0);
        history.visit(at("b", 25.0));
        assert!(!history.can_step(false));
        // A failed step drops the unreachable target and leaves the current place.
        let missing = history.step(true, at("d", 0.0)).unwrap();
        assert_eq!(missing.path, PathBuf::from("b"));
        history.cancel(true);
        assert!(!history.can_step(false));
        assert_eq!(
            history.step(true, at("d", 0.0)).unwrap().path,
            PathBuf::from("a")
        );
    }
    #[test]
    fn heading_slugs_follow_github_rules() {
        assert_eq!(
            slugs([
                "Baseline (to beat)",
                "13. SSAO resolution & quality tiers (*visual*)",
                "Baseline (to beat)",
                "Ünïcode Title"
            ]),
            [
                "baseline-to-beat",
                "13-ssao-resolution--quality-tiers-visual",
                "baseline-to-beat-1",
                "ünïcode-title"
            ]
        );
        assert_eq!(decode_fragment("caf%C3%A9-%zz"), "café-%zz");
    }
}
