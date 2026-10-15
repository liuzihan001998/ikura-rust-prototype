use crate::{settings::fill, text::TextRenderer, vm::TextPage};
use image::RgbaImage;
use sdl2::{event::Event, keyboard::Keycode, mouse::MouseButton, rect::Rect};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
pub const LIMIT: usize = 2000;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub page: TextPage,
    #[serde(default)]
    pub voices: Vec<PathBuf>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Session {
    #[serde(default)]
    pub history: Vec<Entry>,
}
impl Session {
    pub fn validate(&self) -> Result<(), String> {
        if self.history.len() > LIMIT
            || self.history.iter().any(|entry| {
                entry.page.body.len() > 1024 * 1024
                    || entry.voices.len() > 256
                    || entry.voices.iter().any(|p| !safe_voice(p))
            })
        {
            return Err("invalid saved history".into());
        }
        Ok(())
    }
}
pub fn safe_voice(path: &Path) -> bool {
    let parts: Vec<_> = path.components().collect();
    parts.len() == 3
        && parts[0].as_os_str() == "OGG-audio"
        && matches!(parts[1].as_os_str().to_str(), Some("VOICE1" | "VOICE2"))
        && matches!(parts[2], std::path::Component::Normal(_))
        && path
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("ogg"))
}
pub enum Action {
    None,
    Close,
    Replay(Vec<PathBuf>),
}
#[derive(Default)]
pub struct Panel {
    pub offset: usize,
    pub status: String,
    dragging: bool,
    total: usize,
    capacity: usize,
    hits: Vec<(Rect, Vec<PathBuf>)>,
}
impl Panel {
    fn max_offset(&self) -> usize {
        self.total.saturating_sub(self.capacity)
    }
    fn drag(&mut self, y: i32) {
        let max = self.max_offset();
        self.offset = max.saturating_sub(((y - 70).clamp(0, 354) as usize) * max / 354);
    }
    pub fn handle(&mut self, event: &Event) -> Action {
        match event {
            Event::KeyDown {
                keycode: Some(Keycode::Escape | Keycode::B),
                repeat: false,
                ..
            }
            | Event::MouseButtonDown {
                mouse_btn: MouseButton::Right,
                ..
            } => return Action::Close,
            Event::KeyDown {
                keycode: Some(Keycode::Up | Keycode::PageUp),
                ..
            } => {
                self.offset = (self.offset
                    + if matches!(
                        event,
                        Event::KeyDown {
                            keycode: Some(Keycode::PageUp),
                            ..
                        }
                    ) {
                        8
                    } else {
                        1
                    })
                .min(self.max_offset());
            }
            Event::KeyDown {
                keycode: Some(Keycode::Down | Keycode::PageDown),
                ..
            } => {
                self.offset = self.offset.saturating_sub(
                    if matches!(
                        event,
                        Event::KeyDown {
                            keycode: Some(Keycode::PageDown),
                            ..
                        }
                    ) {
                        8
                    } else {
                        1
                    },
                )
            }
            Event::KeyDown {
                keycode: Some(Keycode::Home),
                ..
            } => self.offset = self.max_offset(),
            Event::KeyDown {
                keycode: Some(Keycode::End),
                ..
            } => self.offset = 0,
            Event::MouseWheel { y, direction, .. } => {
                let y = if *direction == sdl2::mouse::MouseWheelDirection::Flipped {
                    -*y
                } else {
                    *y
                };
                self.offset = (self.offset as i64 + i64::from(y) * 3)
                    .clamp(0, self.max_offset() as i64) as usize;
            }
            Event::MouseButtonDown {
                mouse_btn: MouseButton::Left,
                x,
                y,
                ..
            } => {
                if Rect::new(528, 24, 80, 28).contains_point((*x, *y)) {
                    return Action::Close;
                }
                if Rect::new(596, 64, 16, 370).contains_point((*x, *y)) {
                    self.dragging = true;
                    self.drag(*y);
                } else if let Some((_, paths)) = self
                    .hits
                    .iter()
                    .find(|(rect, _)| rect.contains_point((*x, *y)))
                {
                    return Action::Replay(paths.clone());
                }
            }
            Event::MouseButtonUp {
                mouse_btn: MouseButton::Left,
                ..
            } => self.dragging = false,
            Event::MouseMotion { y, .. } if self.dragging => self.drag(*y),
            _ => {}
        }
        Action::None
    }
    pub fn draw(
        &mut self,
        text: &TextRenderer,
        background: &RgbaImage,
        entries: &[Entry],
    ) -> RgbaImage {
        let mut frame = background.clone();
        for p in frame.pixels_mut() {
            for c in 0..3 {
                p[c] /= 4;
            }
        }
        text.draw_label(
            &mut frame,
            "HISTORY - click a voiced line to replay",
            30,
            25,
            490,
            17.,
            [250, 222, 158],
        );
        text.draw_label(&mut frame, "Back", 540, 28, 70, 15., [235, 235, 235]);
        let mut lines = Vec::new();
        for entry in entries {
            if let Some(speaker) = entry.page.speaker.as_deref() {
                lines.push((speaker.to_owned(), true, &entry.voices));
            }
            for body in entry.page.body.lines() {
                for line in text.wrap_line(body, 15., 530) {
                    lines.push((line, false, &entry.voices));
                }
            }
            lines.push((String::new(), false, &entry.voices));
        }
        if lines.is_empty() {
            text.draw_label(
                &mut frame,
                "No dialogue history yet.",
                48,
                90,
                520,
                17.,
                [205, 215, 225],
            );
        }
        self.total = lines.len();
        self.capacity = 17;
        self.offset = self.offset.min(self.max_offset());
        self.hits.clear();
        let end = lines.len().saturating_sub(self.offset);
        let start = end.saturating_sub(self.capacity);
        for (row, (line, speaker, voices)) in lines[start..end].iter().enumerate() {
            let y = 64 + row as i32 * 21;
            let color = if *speaker {
                [255, 219, 147]
            } else {
                [233, 237, 242]
            };
            text.draw_label(&mut frame, line, 48, y, 530, 15., color);
            if !voices.is_empty() && !line.is_empty() {
                text.draw_label(&mut frame, ">", 30, y, 16, 15., [157, 210, 238]);
                self.hits
                    .push((Rect::new(25, y, 560, 21), (*voices).clone()));
            }
        }
        fill(&mut frame, Rect::new(598, 64, 10, 370), [48, 62, 80, 255]);
        let height = (370 * self.capacity / self.total.max(self.capacity)).max(16) as u32;
        let max = self.max_offset();
        let y = 64
            + ((370 - height as usize) * (max - self.offset))
                .checked_div(max)
                .unwrap_or(0) as i32;
        fill(
            &mut frame,
            Rect::new(598, y, 10, height),
            [193, 170, 117, 255],
        );
        text.draw_label(
            &mut frame,
            if self.status.is_empty() {
                "Wheel / drag: scroll    Home / End    B / Esc: back"
            } else {
                &self.status
            },
            30,
            443,
            580,
            12.,
            [170, 188, 206],
        );
        frame
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wheel_drag_and_voice_click_use_the_visible_history_rows() {
        let renderer = TextRenderer::new().unwrap();
        let entries: Vec<_> = (0..40)
            .map(|i| Entry {
                page: TextPage {
                    speaker: None,
                    body: format!("Line {i}"),
                },
                voices: vec![PathBuf::from(format!("OGG-audio/VOICE1/V{i}.OGG"))],
            })
            .collect();
        let mut panel = Panel::default();
        let background = RgbaImage::new(640, 480);
        panel.draw(&renderer, &background, &entries);
        let newest_paths = panel.hits[0].1.clone();
        let wheel = Event::MouseWheel {
            timestamp: 0,
            window_id: 1,
            which: 0,
            x: 0,
            y: 3,
            direction: sdl2::mouse::MouseWheelDirection::Normal,
            precise_x: 0.,
            precise_y: 3.,
            mouse_x: 100,
            mouse_y: 100,
        };
        panel.handle(&wheel);
        assert_eq!(panel.offset, 9);
        panel.draw(&renderer, &background, &entries);
        assert_ne!(panel.hits[0].1, newest_paths);
        let click = Event::MouseButtonDown {
            timestamp: 0,
            window_id: 1,
            which: 0,
            mouse_btn: MouseButton::Left,
            clicks: 1,
            x: 60,
            y: 65,
        };
        match panel.handle(&click) {
            Action::Replay(paths) => assert_eq!(paths, panel.hits[0].1),
            _ => panic!("visible voice row was not replayable"),
        }
        panel.handle(&Event::MouseButtonDown {
            timestamp: 0,
            window_id: 1,
            which: 0,
            mouse_btn: MouseButton::Left,
            clicks: 1,
            x: 600,
            y: 70,
        });
        assert_eq!(panel.offset, panel.max_offset());
        panel.drag(424);
        assert_eq!(panel.offset, 0);
    }

    #[test]
    fn history_paths_cannot_escape_game_data() {
        assert!(safe_voice(Path::new("OGG-audio/VOICE1/V001.OGG")));
        assert!(!safe_voice(Path::new("/tmp/x.ogg")));
        assert!(!safe_voice(Path::new("OGG-audio/VOICE1/../x.ogg")));
    }
}
