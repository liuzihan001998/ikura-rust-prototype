use crate::text::TextRenderer;
use image::{Rgba, RgbaImage};
use sdl2::{event::Event, keyboard::Keycode, mouse::MouseButton, rect::Rect};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub music: u8,
    pub sound: u8,
    pub voice: u8,
    pub text_percent: u16,
    pub auto_wait_ms: u64,
    pub autosave: bool,
    pub fullscreen: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            music: 80,
            sound: 80,
            voice: 100,
            text_percent: 100,
            auto_wait_ms: 1500,
            autosave: true,
            fullscreen: cfg!(target_os = "android"),
        }
    }
}
impl Settings {
    pub fn load(root: &Path) -> Self {
        let path = root.join("settings.json");
        match fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<Self>(&bytes) {
                Ok(mut value) => {
                    value.normalize();
                    value
                }
                Err(e) => {
                    eprintln!("Invalid settings {}: {e}; using defaults", path.display());
                    Self::default()
                }
            },
            Err(e) => {
                if e.kind() != std::io::ErrorKind::NotFound {
                    eprintln!("Settings unavailable: {e}");
                }
                Self::default()
            }
        }
    }
    fn normalize(&mut self) {
        if cfg!(target_os = "android") {
            self.fullscreen = true;
        }
        self.music = self.music.min(100);
        self.sound = self.sound.min(100);
        self.voice = self.voice.min(100);
        self.text_percent = self.text_percent.min(400);
        self.auto_wait_ms = self.auto_wait_ms.clamp(500, 10000);
    }
    pub fn store(&self, root: &Path) -> Result<(), Box<dyn std::error::Error>> {
        fs::create_dir_all(root)?;
        let path = root.join("settings.json");
        let temporary = root.join(format!("settings.{}.tmp", std::process::id()));
        let result = (|| -> Result<(), Box<dyn std::error::Error>> {
            use std::io::Write;
            let mut f = fs::File::create(&temporary)?;
            f.write_all(&serde_json::to_vec_pretty(self)?)?;
            f.sync_all()?;
            fs::rename(&temporary, path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(temporary);
        }
        result
    }
    pub fn text_interval(&self, scripted: u64) -> u64 {
        scripted.saturating_mul(u64::from(self.text_percent)) / 100
    }
    pub fn auto_delay(&self, characters: usize) -> u64 {
        self.auto_wait_ms
            .saturating_add((characters as u64).saturating_mul(25))
    }
    fn value(&self, row: usize) -> u64 {
        match row {
            0 => self.music.into(),
            1 => self.sound.into(),
            2 => self.voice.into(),
            3 => self.text_percent.into(),
            4 => self.auto_wait_ms,
            5 => self.autosave as u64,
            _ => self.fullscreen as u64,
        }
    }
    fn set(&mut self, row: usize, value: u64) {
        match row {
            0 => self.music = value.min(100) as u8,
            1 => self.sound = value.min(100) as u8,
            2 => self.voice = value.min(100) as u8,
            3 => self.text_percent = value.min(400) as u16,
            4 => self.auto_wait_ms = value.clamp(500, 10000),
            5 => self.autosave = value != 0,
            _ => self.fullscreen = cfg!(target_os = "android") || value != 0,
        }
    }
}
fn limits(row: usize) -> (u64, u64, u64) {
    match row {
        0..=2 => (0, 100, 5),
        3 => (0, 400, 25),
        4 => (500, 10000, 250),
        _ => (0, 1, 1),
    }
}
pub fn fill(frame: &mut RgbaImage, rect: Rect, color: [u8; 4]) {
    for y in rect.y().max(0)..(rect.y() + rect.height() as i32).min(frame.height() as i32) {
        for x in rect.x().max(0)..(rect.x() + rect.width() as i32).min(frame.width() as i32) {
            frame.put_pixel(x as u32, y as u32, Rgba(color));
        }
    }
}
pub struct Panel {
    pub selected: usize,
    dragging: bool,
    pub background: RgbaImage,
    pub status: String,
}
impl Panel {
    pub fn new(background: RgbaImage) -> Self {
        Self {
            selected: 0,
            dragging: false,
            background,
            status: String::new(),
        }
    }
    fn at_x(&self, value: &mut Settings, x: i32) {
        let (low, high, _) = limits(self.selected);
        value.set(
            self.selected,
            low + ((x - 290).clamp(0, 270) as u64) * (high - low) / 270,
        );
    }
    /// Returns true when the panel should close. Changes apply immediately.
    pub fn handle(&mut self, event: &Event, value: &mut Settings) -> bool {
        match event {
            Event::KeyDown {
                keycode: Some(Keycode::Escape | Keycode::F10),
                repeat: false,
                ..
            } => return true,
            Event::KeyDown {
                keycode: Some(Keycode::Up),
                ..
            } => self.selected = self.selected.saturating_sub(1),
            Event::KeyDown {
                keycode: Some(Keycode::Down),
                ..
            } => self.selected = (self.selected + 1).min(6),
            Event::KeyDown {
                keycode: Some(key @ (Keycode::Left | Keycode::Right | Keycode::Return)),
                ..
            } => {
                let (low, high, step) = limits(self.selected);
                let current = value.value(self.selected);
                let next = if self.selected >= 5 {
                    1 - current
                } else if *key == Keycode::Left {
                    current.saturating_sub(step).max(low)
                } else {
                    (current + step).min(high)
                };
                value.set(self.selected, next);
            }
            Event::MouseButtonDown {
                mouse_btn: MouseButton::Left,
                x,
                y,
                ..
            } => {
                if Rect::new(536, 24, 80, 32).contains_point((*x, *y)) {
                    return true;
                }
                if Rect::new(36, 424, 145, 30).contains_point((*x, *y)) {
                    *value = Settings::default();
                }
                if (80..416).contains(y) {
                    self.selected = ((*y - 80) / 48) as usize;
                    if self.selected >= 5 {
                        value.set(self.selected, 1 - value.value(self.selected));
                    } else if (280..580).contains(x) {
                        self.dragging = true;
                        self.at_x(value, *x);
                    }
                }
            }
            Event::MouseButtonUp {
                mouse_btn: MouseButton::Left,
                ..
            } => self.dragging = false,
            Event::MouseMotion { x, .. } if self.dragging => self.at_x(value, *x),
            _ => {}
        }
        false
    }
    pub fn draw(&self, text: &TextRenderer, value: &Settings) -> RgbaImage {
        let mut frame = self.background.clone();
        for p in frame.pixels_mut() {
            for c in 0..3 {
                p[c] /= 5;
            }
        }
        text.draw_label(&mut frame, "SETTINGS", 36, 24, 350, 25., [245, 235, 210]);
        for (row, label) in [
            "Music volume",
            "Sound volume",
            "Voice volume",
            "Text delay",
            "Auto wait",
            "Automatic saves",
            "Fullscreen",
        ]
        .iter()
        .enumerate()
        {
            let label = if row == 6 && cfg!(target_os = "android") {
                "横屏全屏（固定）"
            } else {
                label
            };
            let y = 80 + row as i32 * 48;
            if self.selected == row {
                fill(&mut frame, Rect::new(28, y - 2, 584, 44), [35, 50, 70, 255]);
            }
            text.draw_label(&mut frame, label, 40, y + 5, 240, 17., [225, 230, 240]);
            let v = value.value(row);
            let (low, high, _) = limits(row);
            if row < 5 {
                fill(
                    &mut frame,
                    Rect::new(290, y + 25, 270, 4),
                    [72, 84, 100, 255],
                );
                let x = 290 + ((v - low) * 270 / (high - low)) as i32;
                fill(
                    &mut frame,
                    Rect::new(x - 4, y + 18, 8, 18),
                    [228, 198, 131, 255],
                );
            }
            let display = match row {
                0..=2 => format!("{v}%"),
                3 => {
                    if v == 0 {
                        "Instant".into()
                    } else {
                        format!("{v}% of script delay")
                    }
                }
                4 => format!("{:.2} sec + reading time", v as f64 / 1000.),
                _ => {
                    if v == 0 {
                        "OFF".into()
                    } else {
                        "ON".into()
                    }
                }
            };
            text.draw_label(&mut frame, &display, 290, y, 290, 14., [255, 222, 162]);
        }
        for (x, w, label) in [(536, 80, "Back"), (36, 145, "Reset defaults")] {
            let y = if x == 536 { 24 } else { 424 };
            fill(&mut frame, Rect::new(x, y, w, 30), [64, 80, 103, 255]);
            text.draw_label(
                &mut frame,
                label,
                x + 10,
                y + 6,
                w as i32 - 15,
                14.,
                [245, 245, 245],
            );
        }
        text.draw_label(
            &mut frame,
            if self.status.is_empty() {
                "F10 / Esc: back     Arrow keys: adjust"
            } else {
                &self.status
            },
            200,
            432,
            415,
            12.,
            [180, 195, 210],
        );
        frame
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn controls_adjust_independent_values_and_close_without_advancing_story() {
        let mut value = Settings::default();
        let mut panel = Panel::new(RgbaImage::new(640, 480));
        let click = |x, y| Event::MouseButtonDown {
            timestamp: 0,
            window_id: 1,
            which: 0,
            mouse_btn: MouseButton::Left,
            clicks: 1,
            x,
            y,
        };
        assert!(!panel.handle(&click(425, 100), &mut value));
        assert_eq!(value.music, 50);
        assert_eq!(value.voice, 100);
        panel.handle(&click(400, 340), &mut value);
        assert!(!value.autosave);
        panel.handle(&click(400, 390), &mut value);
        assert!(value.fullscreen);
        let escape = Event::KeyDown {
            timestamp: 0,
            window_id: 1,
            keycode: Some(Keycode::Escape),
            scancode: None,
            keymod: sdl2::keyboard::Mod::NOMOD,
            repeat: false,
        };
        assert!(panel.handle(&escape, &mut value));
    }

    #[test]
    fn configuration_round_trip_and_bounds() {
        let dir = std::env::temp_dir().join(format!("ikura-settings-{}", std::process::id()));
        let value = Settings {
            music: 23,
            text_percent: 0,
            auto_wait_ms: 750,
            ..Settings::default()
        };
        value.store(&dir).unwrap();
        assert_eq!(Settings::load(&dir), value);
        assert_eq!(value.text_interval(25), 0);
        assert_eq!(value.auto_delay(100), 3250);
        fs::write(
            dir.join("settings.json"),
            br#"{"music":255,"auto_wait_ms":0}"#,
        )
        .unwrap();
        let clamped = Settings::load(&dir);
        assert_eq!(clamped.music, 100);
        assert_eq!(clamped.auto_wait_ms, 500);
        let _ = fs::remove_dir_all(dir);
    }
}
