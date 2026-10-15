use crate::{
    save::{self, Metadata, Slot},
    text::TextRenderer,
};
use image::{Rgba, RgbaImage};
use sdl2::{event::Event, keyboard::Keycode, mouse::MouseButton, rect::Rect};
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Save,
    Load,
}

#[derive(Clone, Copy)]
pub enum Action {
    None,
    Close,
    Save(Slot),
    Load(Slot),
    Delete(Slot),
}

struct Card {
    metadata: Option<Metadata>,
    thumbnail: Option<RgbaImage>,
    occupied: bool,
    problem: Option<&'static str>,
    backup: bool,
}

pub struct SaveMenu {
    mode: Mode,
    page: u8,
    selected: usize,
    confirmation: Option<Slot>,
    deleting: bool,
    cards: Vec<Card>,
    background: RgbaImage,
    can_save: bool,
    confirm_load: bool,
    pub status: String,
}

fn card_rect(index: usize) -> Rect {
    Rect::new(
        16 + (index % 2) as i32 * 312,
        70 + (index / 2) as i32 * 118,
        296,
        110,
    )
}
fn contains(rect: Rect, x: i32, y: i32) -> bool {
    rect.contains_point((x, y))
}
fn fill(frame: &mut RgbaImage, rect: Rect, color: [u8; 4]) {
    for y in rect.y().max(0)..(rect.y() + rect.height() as i32).min(frame.height() as i32) {
        for x in rect.x().max(0)..(rect.x() + rect.width() as i32).min(frame.width() as i32) {
            frame.put_pixel(x as u32, y as u32, Rgba(color));
        }
    }
}

/// Stable UTC display without a platform-specific timezone dependency.
pub fn timestamp(seconds: u64) -> String {
    let days = (seconds / 86400) as i64;
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}Z",
        seconds % 86400 / 3600,
        seconds % 3600 / 60
    )
}

pub fn slot_name(slot: Slot) -> String {
    match slot {
        Slot::Quick => "Quick save".into(),
        Slot::Auto => "Automatic save".into(),
        Slot::Manual(n) => format!("Slot {n:02}"),
    }
}

impl SaveMenu {
    pub fn new(
        mode: Mode,
        root: &Path,
        signature: u64,
        background: RgbaImage,
        can_save: bool,
        in_story: bool,
    ) -> Self {
        let mut menu = Self {
            mode,
            page: 0,
            selected: 0,
            confirmation: None,
            deleting: false,
            cards: vec![],
            background,
            can_save,
            confirm_load: in_story,
            status: String::new(),
        };
        menu.refresh(root, signature);
        menu
    }
    pub fn refresh(&mut self, root: &Path, signature: u64) {
        self.cards = (0..=save::SLOT_COUNT + 1)
            .map(|index| {
                let slot = if index == 0 {
                    Slot::Quick
                } else if index == save::SLOT_COUNT + 1 {
                    Slot::Auto
                } else {
                    Slot::Manual(index)
                };
                let path = save::load_path(root, slot);
                let backup = save::backup_path(&path).exists();
                if !path.exists() {
                    return Card {
                        metadata: None,
                        thumbnail: None,
                        occupied: backup,
                        problem: backup.then_some("Backup only"),
                        backup,
                    };
                }
                match save::metadata(&path) {
                    Ok(metadata) => {
                        let thumbnail =
                            image::load_from_memory(&metadata.thumbnail).ok().map(|i| {
                                image::imageops::resize(
                                    &i.to_rgba8(),
                                    112,
                                    84,
                                    image::imageops::FilterType::Triangle,
                                )
                            });
                        let problem = (metadata.script_signature != signature)
                            .then_some("Different game data");
                        Card {
                            metadata: Some(metadata),
                            thumbnail,
                            occupied: true,
                            problem,
                            backup,
                        }
                    }
                    Err(_) => Card {
                        metadata: None,
                        thumbnail: None,
                        occupied: true,
                        problem: Some("Damaged save"),
                        backup,
                    },
                }
            })
            .collect();
    }
    fn request_delete(&mut self, slot: Slot) -> Action {
        let index = match slot {
            Slot::Quick => 0,
            Slot::Auto => save::SLOT_COUNT as usize + 1,
            Slot::Manual(n) => n as usize,
        };
        if !self.cards[index].occupied {
            self.status = "This slot is empty.".into();
            return Action::None;
        }
        self.confirmation = Some(slot);
        self.deleting = true;
        Action::None
    }
    fn request(&mut self, slot: Slot) -> Action {
        self.deleting = false;
        if slot != Slot::Auto && self.mode == Mode::Save && !self.can_save {
            self.status = "Saving is disabled at this point.".into();
            return Action::None;
        }
        let index = match slot {
            Slot::Quick => 0,
            Slot::Auto => save::SLOT_COUNT as usize + 1,
            Slot::Manual(n) => n as usize,
        };
        if (self.mode == Mode::Load || slot == Slot::Auto) && !self.cards[index].occupied {
            self.status = "This slot is empty.".into();
            return Action::None;
        }
        if (slot != Slot::Auto && self.mode == Mode::Save && self.cards[index].occupied)
            || ((self.mode == Mode::Load || slot == Slot::Auto) && self.confirm_load)
        {
            self.confirmation = Some(slot);
            return Action::None;
        }
        self.action(slot)
    }
    fn action(&self, slot: Slot) -> Action {
        if self.deleting {
            return Action::Delete(slot);
        }
        if slot == Slot::Auto {
            return Action::Load(slot);
        }
        match self.mode {
            Mode::Save => Action::Save(slot),
            Mode::Load => Action::Load(slot),
        }
    }
    pub fn handle(&mut self, event: &Event) -> Action {
        if let Some(slot) = self.confirmation {
            match event {
                Event::KeyDown {
                    keycode: Some(Keycode::Return | Keycode::Y),
                    repeat: false,
                    ..
                } => {
                    self.confirmation = None;
                    return self.action(slot);
                }
                Event::KeyDown {
                    keycode: Some(Keycode::Escape | Keycode::N),
                    repeat: false,
                    ..
                } => self.confirmation = None,
                Event::MouseButtonDown {
                    mouse_btn: MouseButton::Left,
                    x,
                    y,
                    ..
                } => {
                    if contains(Rect::new(140, 258, 150, 32), *x, *y) {
                        self.confirmation = None;
                        return self.action(slot);
                    }
                    if contains(Rect::new(350, 258, 150, 32), *x, *y) {
                        self.confirmation = None;
                    }
                }
                _ => {}
            }
            return Action::None;
        }
        match event {
            Event::KeyDown {
                keycode: Some(key),
                repeat: false,
                ..
            } => match *key {
                Keycode::Escape => return Action::Close,
                Keycode::Delete | Keycode::Backspace => {
                    return self
                        .request_delete(Slot::Manual(self.page * 6 + self.selected as u8 + 1));
                }
                Keycode::F6 => self.mode = Mode::Save,
                Keycode::F8 => self.mode = Mode::Load,
                Keycode::F7 => return self.request(Slot::Auto),
                Keycode::Q => return self.request(Slot::Quick),
                Keycode::Tab => {
                    self.mode = if self.mode == Mode::Save {
                        Mode::Load
                    } else {
                        Mode::Save
                    }
                }
                Keycode::PageUp => self.page = self.page.saturating_sub(1),
                Keycode::PageDown => self.page = (self.page + 1).min(1),
                Keycode::Left => self.selected = self.selected.saturating_sub(1),
                Keycode::Right => self.selected = (self.selected + 1).min(5),
                Keycode::Up => self.selected = self.selected.saturating_sub(2),
                Keycode::Down => self.selected = (self.selected + 2).min(5),
                Keycode::Return => {
                    return self.request(Slot::Manual(self.page * 6 + self.selected as u8 + 1));
                }
                _ => {}
            },
            Event::MouseButtonDown {
                mouse_btn: MouseButton::Right,
                x,
                y,
                ..
            } => {
                if contains(Rect::new(174, 14, 80, 28), *x, *y) {
                    return self.request_delete(Slot::Auto);
                }
                if contains(Rect::new(262, 14, 80, 28), *x, *y) {
                    return self.request_delete(Slot::Quick);
                }
                if let Some(index) = (0..6).find(|&i| contains(card_rect(i), *x, *y)) {
                    self.selected = index;
                    return self.request_delete(Slot::Manual(self.page * 6 + index as u8 + 1));
                }
            }
            Event::MouseWheel { y, .. } => {
                if *y < 0 {
                    self.page = (self.page + 1).min(1)
                } else if *y > 0 {
                    self.page = self.page.saturating_sub(1)
                }
            }
            Event::MouseMotion { x, y, .. } => {
                if let Some(index) = (0..6).find(|&i| contains(card_rect(i), *x, *y)) {
                    self.selected = index;
                }
            }
            Event::MouseButtonDown {
                mouse_btn: MouseButton::Left,
                x,
                y,
                ..
            } => {
                if contains(Rect::new(20, 442, 90, 26), *x, *y) {
                    return self
                        .request_delete(Slot::Manual(self.page * 6 + self.selected as u8 + 1));
                }
                if contains(Rect::new(174, 14, 80, 28), *x, *y) {
                    return self.request(Slot::Auto);
                }
                if contains(Rect::new(556, 14, 68, 28), *x, *y) {
                    return Action::Close;
                }
                if contains(Rect::new(350, 14, 82, 28), *x, *y) {
                    self.mode = Mode::Save;
                }
                if contains(Rect::new(438, 14, 82, 28), *x, *y) {
                    self.mode = Mode::Load;
                }
                if contains(Rect::new(262, 14, 80, 28), *x, *y) {
                    return self.request(Slot::Quick);
                }
                if contains(Rect::new(448, 442, 36, 26), *x, *y) {
                    self.page = self.page.saturating_sub(1);
                }
                if contains(Rect::new(558, 442, 36, 26), *x, *y) {
                    self.page = (self.page + 1).min(1);
                }
                if let Some(index) = (0..6).find(|&i| contains(card_rect(i), *x, *y)) {
                    self.selected = index;
                    return self.request(Slot::Manual(self.page * 6 + index as u8 + 1));
                }
            }
            _ => {}
        }
        Action::None
    }
    pub fn draw(&self, text: &TextRenderer) -> RgbaImage {
        let mut frame = self.background.clone();
        for pixel in frame.pixels_mut() {
            for channel in 0..3 {
                pixel[channel] = pixel[channel] / 5 + 5;
            }
        }
        let label = |frame: &mut RgbaImage, value: &str, x, y, width, px, color| {
            text.draw_label(frame, value, x, y, width, px, color)
        };
        label(
            &mut frame,
            if self.mode == Mode::Save {
                "SAVE GAME"
            } else {
                "LOAD GAME"
            },
            20,
            13,
            145,
            21.,
            [245, 239, 219],
        );
        for (rect, value, active) in [
            (Rect::new(174, 14, 80, 28), "Auto", false),
            (Rect::new(262, 14, 80, 28), "Quick", false),
            (Rect::new(350, 14, 82, 28), "Save", self.mode == Mode::Save),
            (Rect::new(438, 14, 82, 28), "Load", self.mode == Mode::Load),
            (Rect::new(556, 14, 68, 28), "Back", false),
        ] {
            fill(
                &mut frame,
                rect,
                if active {
                    [70, 87, 108, 255]
                } else {
                    [32, 40, 55, 255]
                },
            );
            label(
                &mut frame,
                value,
                rect.x() + 12,
                rect.y() + 4,
                rect.width() as i32 - 16,
                16.,
                [240, 240, 240],
            );
        }
        let quick = &self.cards[0];
        let summary = quick.metadata.as_ref().map_or_else(
            || {
                if quick.occupied {
                    "Quick save: damaged (check backup)".into()
                } else {
                    "Quick save: empty".into()
                }
            },
            |m| format!("Quick: {}  {}", timestamp(m.saved_at), m.chapter),
        );
        label(&mut frame, &summary, 20, 47, 600, 12., [180, 190, 202]);
        for index in 0..6 {
            let number = self.page * 6 + index as u8 + 1;
            let card = &self.cards[number as usize];
            let rect = card_rect(index);
            fill(
                &mut frame,
                rect,
                if index == self.selected {
                    [180, 154, 96, 255]
                } else {
                    [58, 67, 82, 255]
                },
            );
            fill(
                &mut frame,
                Rect::new(rect.x() + 2, rect.y() + 2, 292, 106),
                if index == self.selected {
                    [39, 53, 72, 255]
                } else {
                    [25, 32, 45, 255]
                },
            );
            label(
                &mut frame,
                &format!("SLOT {number:02}"),
                rect.x() + 8,
                rect.y() + 3,
                100,
                14.,
                [220, 225, 235],
            );
            if card.backup {
                label(
                    &mut frame,
                    "Backup",
                    rect.x() + 88,
                    rect.y() + 6,
                    36,
                    8.5,
                    [218, 189, 118],
                );
            }
            let x = rect.x() + 130;
            if let Some(metadata) = &card.metadata {
                if let Some(thumbnail) = &card.thumbnail {
                    image::imageops::replace(
                        &mut frame,
                        thumbnail,
                        i64::from(rect.x() + 8),
                        i64::from(rect.y() + 22),
                    );
                }
                label(
                    &mut frame,
                    &metadata.chapter,
                    x,
                    rect.y() + 7,
                    158,
                    16.,
                    [245, 238, 216],
                );
                label(
                    &mut frame,
                    &timestamp(metadata.saved_at),
                    x,
                    rect.y() + 34,
                    158,
                    11.,
                    [168, 184, 202],
                );
                let lines: Vec<char> = metadata.excerpt.chars().collect();
                for row in 0..2 {
                    let excerpt: String = lines.iter().skip(row * 12).take(12).collect();
                    label(
                        &mut frame,
                        &excerpt,
                        x,
                        rect.y() + 57 + row as i32 * 19,
                        158,
                        12.,
                        [210, 217, 225],
                    );
                }
            } else {
                label(
                    &mut frame,
                    if card.occupied { "DAMAGED" } else { "EMPTY" },
                    x,
                    rect.y() + 43,
                    158,
                    19.,
                    if card.occupied {
                        [240, 154, 142]
                    } else {
                        [117, 135, 155]
                    },
                );
            }
            if let Some(problem) = card.problem {
                label(
                    &mut frame,
                    problem,
                    x,
                    rect.y() + 89,
                    158,
                    10.,
                    [240, 154, 142],
                );
            }
        }
        label(&mut frame, &self.status, 20, 425, 604, 14., [248, 216, 151]);
        label(
            &mut frame,
            "Arrows / Enter    PgUp/PgDn    Right click: delete",
            122,
            451,
            320,
            10.,
            [170, 185, 201],
        );
        for (x, value) in [(448, "<"), (558, ">")] {
            fill(&mut frame, Rect::new(x, 442, 36, 26), [44, 55, 73, 255]);
            label(&mut frame, value, x + 12, 444, 22, 17., [240, 240, 240]);
        }
        label(
            &mut frame,
            &format!("{} / 2", self.page + 1),
            492,
            447,
            60,
            14.,
            [225, 225, 225],
        );
        fill(&mut frame, Rect::new(20, 442, 90, 26), [100, 55, 62, 255]);
        label(&mut frame, "Delete", 35, 447, 70, 14., [250, 230, 230]);
        if let Some(slot) = self.confirmation {
            for pixel in frame.pixels_mut() {
                for channel in 0..3 {
                    pixel[channel] /= 2;
                }
            }
            fill(&mut frame, Rect::new(96, 166, 448, 148), [37, 47, 65, 255]);
            let title = format!(
                "{} {}?",
                if self.deleting {
                    "Delete"
                } else if self.mode == Mode::Save && slot != Slot::Auto {
                    "Overwrite"
                } else {
                    "Load"
                },
                slot_name(slot)
            );
            label(&mut frame, &title, 118, 186, 410, 23., [250, 239, 213]);
            label(
                &mut frame,
                if self.deleting {
                    "Save and its recovery backup will be deleted."
                } else if self.mode == Mode::Save && slot != Slot::Auto {
                    "Existing data will be replaced."
                } else {
                    "Unsaved progress will be lost."
                },
                118,
                222,
                410,
                15.,
                [192, 204, 222],
            );
            for (x, value) in [(140, "Confirm"), (350, "Cancel")] {
                fill(&mut frame, Rect::new(x, 258, 150, 32), [65, 82, 106, 255]);
                label(&mut frame, value, x + 36, 264, 110, 16., [250, 250, 250]);
            }
        }
        frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(code: Keycode) -> Event {
        Event::KeyDown {
            timestamp: 0,
            window_id: 1,
            keycode: Some(code),
            scancode: None,
            keymod: sdl2::keyboard::Mod::NOMOD,
            repeat: false,
        }
    }
    #[test]
    fn deletion_requires_confirmation_and_automatic_slot_is_load_only() {
        let root = std::env::temp_dir().join(format!("ikura-delete-menu-{}", std::process::id()));
        let mut menu = SaveMenu::new(Mode::Save, &root, 42, RgbaImage::new(640, 480), false, true);
        menu.cards[1].occupied = true;
        assert!(matches!(menu.handle(&key(Keycode::Delete)), Action::None));
        assert!(matches!(menu.handle(&key(Keycode::Escape)), Action::None));
        assert!(menu.confirmation.is_none());
        menu.handle(&key(Keycode::Delete));
        assert!(matches!(
            menu.handle(&key(Keycode::Return)),
            Action::Delete(Slot::Manual(1))
        ));
        menu.cards[save::SLOT_COUNT as usize + 1].occupied = true;
        assert!(matches!(menu.handle(&key(Keycode::F7)), Action::None));
        assert!(matches!(
            menu.handle(&key(Keycode::Return)),
            Action::Load(Slot::Auto)
        ));
    }

    #[test]
    fn slot_navigation_and_overwrite_confirmation_are_separate_actions() {
        let root = std::env::temp_dir().join(format!("ikura-empty-menu-{}", std::process::id()));
        let mut menu = SaveMenu::new(Mode::Save, &root, 42, RgbaImage::new(640, 480), true, true);
        menu.handle(&key(Keycode::PageDown));
        assert!(matches!(
            menu.handle(&key(Keycode::Return)),
            Action::Save(Slot::Manual(7))
        ));
        menu.cards[7].occupied = true;
        assert!(matches!(menu.handle(&key(Keycode::Return)), Action::None));
        assert_eq!(menu.confirmation, Some(Slot::Manual(7)));
        menu.handle(&key(Keycode::Escape));
        assert!(menu.confirmation.is_none());
        menu.handle(&key(Keycode::Return));
        assert!(matches!(
            menu.handle(&key(Keycode::Return)),
            Action::Save(Slot::Manual(7))
        ));
        menu.can_save = false;
        assert!(matches!(menu.handle(&key(Keycode::Return)), Action::None));
        assert!(menu.status.contains("disabled"));
    }
    #[test]
    fn empty_loads_are_blocked_and_ingame_loads_require_confirmation() {
        let root =
            std::env::temp_dir().join(format!("ikura-empty-load-menu-{}", std::process::id()));
        let mut menu = SaveMenu::new(Mode::Load, &root, 42, RgbaImage::new(640, 480), true, true);
        assert!(matches!(menu.handle(&key(Keycode::Return)), Action::None));
        assert!(menu.status.contains("empty"));
        menu.cards[1].occupied = true;
        menu.handle(&key(Keycode::Return));
        assert_eq!(menu.confirmation, Some(Slot::Manual(1)));
        assert!(matches!(
            menu.handle(&key(Keycode::Return)),
            Action::Load(Slot::Manual(1))
        ));
        menu.confirm_load = false;
        assert!(matches!(
            menu.handle(&key(Keycode::Return)),
            Action::Load(Slot::Manual(1))
        ));
        assert_eq!(timestamp(0), "1970-01-01 00:00Z");
        assert_eq!(timestamp(951782400), "2000-02-29 00:00Z");
    }
}
