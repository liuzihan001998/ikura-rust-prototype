//! Script control flow. ISF byte framing stays in the existing Python parser.

use encoding_rs::SHIFT_JIS;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, Deserialize)]
pub struct Instruction {
    pub offset: usize,
    pub opcode: u8,
    pub payload: Vec<u8>,
}

#[derive(Debug, Deserialize)]
pub struct Script {
    pub code_start: usize,
    pub jumps: Vec<usize>,
    pub instructions: Vec<Instruction>,
}

pub type Scripts = HashMap<String, Script>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HitArea {
    pub index: u8,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChoiceOption {
    pub value: u8,
    pub caption: String,
    /// Text box of this choice in screen pixels, taken from `CSET`.
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// One decoded `PM` page: an optional speaker name plus the body text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextPage {
    pub speaker: Option<String>,
    pub body: String,
}

impl TextPage {
    /// Display form: the speaker on its own line, then the body.
    pub fn render(&self) -> String {
        match &self.speaker {
            Some(speaker) if !speaker.is_empty() => format!("{speaker}\n{}", self.body),
            _ => self.body.clone(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextWindow {
    pub graphic: Option<String>,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub colors: HashMap<u8, [u8; 3]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlitOp {
    pub command: u8,
    pub source: i32,
    pub source_x: i32,
    pub source_y: i32,
    pub width: i32,
    pub height: i32,
    pub destination: i32,
    pub destination_x: i32,
    pub destination_y: i32,
}

impl BlitOp {
    /// Transition commands carry their duration in the destination field; all
    /// other blit commands copy immediately.
    pub fn duration(&self) -> i32 {
        if matches!(self.command, 7 | 19) {
            self.destination.max(0)
        } else {
            0
        }
    }
}

#[derive(Clone, Debug)]
struct ScriptFrame {
    name: String,
    pc: usize,
    returns: Vec<usize>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    ControlFlow,
    InputPoll,
    ChoicePoll,
    Yield,
    Wait(u64),
    FillSurface {
        slot: i32,
        color: [u8; 3],
    },
    PlayVideo {
        name: String,
        source_width: i32,
        source_height: i32,
        destination_width: i32,
        destination_height: i32,
    },
    LoadGraphic {
        slot: i32,
        name: String,
    },
    FadeTo {
        duration_ms: i32,
        color: [u8; 3],
    },
    ShowSurface {
        slot: i32,
        duration_ms: i32,
    },
    MusicStopped,
    MusicFade(i32),
    MusicRequested(String),
    SoundRequested(String),
    SoundStopped,
    VoiceRequested {
        bank: String,
        name: String,
    },
    VoiceStopped,
    GraphicBank(String),
    TextPage(TextPage),
    Blit(BlitOp),
    InitSurface {
        slot: i32,
        width: i32,
        height: i32,
    },
    Unsupported {
        script: String,
        offset: usize,
        opcode: u8,
    },
    Finished,
}

pub struct Vm {
    scripts: Scripts,
    frames: Vec<ScriptFrame>,
    variables: HashMap<u16, i32>,
    flags: HashSet<u16>,
    system_flags: HashSet<u8>,
    now_ms: u64,
    timer_start_ms: u64,
    selected: Option<i16>,
    click_pending: bool,
    hit_areas: HashMap<u8, HitArea>,
    text_interval: i32,
    font_size: i32,
    font_style: u8,
    cursor_style: u8,
    save_enabled: bool,
    popup_enabled: bool,
    delay_ms: u64,
    speaker_names: HashMap<u8, String>,
    text_window: TextWindow,
    text_page: Option<TextPage>,
    voice_bank: String,
    choice_options: Vec<ChoiceOption>,
    choice_selected: Option<u8>,
    waiting_choice: bool,
}

fn u16_at(data: &[u8], start: usize) -> Result<u16, String> {
    let bytes: [u8; 2] = data
        .get(start..start + 2)
        .ok_or_else(|| format!("expected u16 at payload byte {start}"))?
        .try_into()
        .map_err(|_| "invalid u16".to_string())?;
    Ok(u16::from_le_bytes(bytes))
}

fn u32_at(data: &[u8], start: usize) -> Result<u32, String> {
    let bytes: [u8; 4] = data
        .get(start..start + 4)
        .ok_or_else(|| format!("expected u32 at payload byte {start}"))?
        .try_into()
        .map_err(|_| "invalid u32".to_string())?;
    Ok(u32::from_le_bytes(bytes))
}

fn c_string(data: &[u8], start: usize) -> Result<String, String> {
    let tail = data
        .get(start..)
        .ok_or_else(|| format!("expected string at payload byte {start}"))?;
    let end = tail
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(tail.len());
    String::from_utf8(tail[..end].to_vec()).map_err(|error| error.to_string())
}

fn shift_jis_string(data: &[u8]) -> Result<String, String> {
    let (decoded, _, had_errors) = SHIFT_JIS.decode(data);
    if had_errors {
        return Err("invalid Shift-JIS text".into());
    }
    Ok(decoded.into_owned())
}

fn decode_text_page(data: &[u8], names: &HashMap<u8, String>) -> Result<TextPage, String> {
    let mut position = 1; // window number
    let mut lines = Vec::new();
    let mut speaker: Option<String> = None;
    while position < data.len() {
        let command = data[position];
        position += 1;
        match command {
            0 => {}
            3 => lines.clear(),
            1 => {
                let subtype = *data.get(position).ok_or("PM color command truncated")?;
                position += match subtype {
                    0 => 4, // subtype and RGB
                    1 => 3, // subtype and 16-bit argument
                    _ => return Err(format!("unknown PM style subtype {subtype}")),
                };
            }
            4 => {
                let index = *data.get(position).ok_or("PM speaker index missing")?;
                position += 1;
                if let Some(name) = names.get(&index) {
                    speaker = Some(name.clone());
                }
            }
            6 => {}
            8 | 0x11 => position += 4,
            0x13 => {
                let kind = *data.get(position).ok_or("PM voice command truncated")?;
                if kind == 4 {
                    let index = *data.get(position + 1).ok_or("PM speaker id missing")?;
                    if let Some(name) = names.get(&index) {
                        speaker = Some(name.clone());
                    }
                    position += 3;
                } else if kind > 0x1F {
                    position = data[position..]
                        .iter()
                        .position(|&byte| byte == 0)
                        .map(|end| position + end + 1)
                        .ok_or("PM voice name has no terminator")?;
                } else {
                    return Err(format!("unknown PM voice command {kind}"));
                }
            }
            0xFF => {
                let end = data[position..]
                    .iter()
                    .position(|&byte| byte == 0)
                    .map(|end| position + end)
                    .ok_or("PM text has no terminator")?;
                lines.push(shift_jis_string(&data[position..end])?);
                position = end + 1;
            }
            _ => return Err(format!("unknown PM inline command 0x{command:02X}")),
        }
        if position > data.len() {
            return Err("PM command exceeds payload".into());
        }
    }
    Ok(TextPage {
        speaker,
        body: lines.join("\n"),
    })
}

impl Vm {
    pub fn new(scripts: Scripts, entry: &str) -> Result<Self, String> {
        let pc = scripts
            .get(entry)
            .ok_or_else(|| format!("missing script {entry}"))?
            .code_start;
        Ok(Self {
            scripts,
            frames: vec![ScriptFrame {
                name: entry.to_owned(),
                pc,
                returns: Vec::new(),
            }],
            variables: HashMap::new(),
            flags: HashSet::new(),
            system_flags: HashSet::new(),
            now_ms: 0,
            timer_start_ms: 0,
            selected: None,
            click_pending: false,
            hit_areas: HashMap::new(),
            text_interval: 0,
            font_size: 0,
            font_style: 0,
            cursor_style: 0,
            save_enabled: false,
            popup_enabled: false,
            delay_ms: 0,
            speaker_names: HashMap::new(),
            text_window: TextWindow::default(),
            text_page: None,
            voice_bank: "VOICE2".into(),
            choice_options: Vec::new(),
            choice_selected: None,
            waiting_choice: false,
        })
    }

    pub fn position(&self) -> Option<(&str, usize)> {
        self.frames
            .last()
            .map(|frame| (frame.name.as_str(), frame.pc))
    }

    pub fn advance_clock(&mut self, milliseconds: u64) {
        self.now_ms = self.now_ms.saturating_add(milliseconds);
    }

    pub fn set_selected(&mut self, selected: Option<i16>) {
        self.selected = selected;
        self.click_pending = selected.is_some();
    }

    pub fn click(&mut self) {
        self.selected = None;
        self.click_pending = true;
    }

    pub fn hit_areas(&self) -> &HashMap<u8, HitArea> {
        &self.hit_areas
    }

    pub fn speaker_names(&self) -> &HashMap<u8, String> {
        &self.speaker_names
    }

    pub fn text_window(&self) -> &TextWindow {
        &self.text_window
    }

    pub fn font_size(&self) -> i32 {
        self.font_size
    }

    pub fn text_page(&self) -> Option<&TextPage> {
        self.text_page.as_ref()
    }

    pub fn clear_text_page(&mut self) {
        self.text_page = None;
    }

    pub fn waiting_for_choice(&self) -> bool {
        self.waiting_choice
    }

    pub fn choice_options(&self) -> &[ChoiceOption] {
        &self.choice_options
    }

    pub fn choose(&mut self, value: u8) -> Result<(), String> {
        if !self.waiting_choice
            || !self
                .choice_options
                .iter()
                .any(|option| option.value == value)
        {
            return Err(format!("choice {value} is not available"));
        }
        self.choice_selected = Some(value);
        Ok(())
    }

    #[cfg(test)]
    pub fn set_position(&mut self, offset: usize) -> Result<(), String> {
        let frame = self.frames.last_mut().ok_or("VM has finished")?;
        let script = self.scripts.get(&frame.name).ok_or("script disappeared")?;
        if script
            .instructions
            .binary_search_by_key(&offset, |item| item.offset)
            .is_err()
        {
            return Err(format!("{}: no instruction at 0x{offset:X}", frame.name));
        }
        frame.pc = offset;
        Ok(())
    }

    fn decode_value(&self, raw: u32) -> i32 {
        let value = if raw & 0x8000_0000 != 0 {
            *self
                .variables
                .get(&((raw & 0x7FFF_FFFF) as u16))
                .unwrap_or(&0) as u32
        } else {
            raw
        };
        if value & 0x4000_0000 != 0 {
            (value | 0x8000_0000) as i32
        } else {
            value as i32
        }
    }

    fn jump_target(&self, name: &str, index: u16) -> Result<usize, String> {
        self.scripts
            .get(name)
            .and_then(|script| script.jumps.get(index as usize))
            .copied()
            .ok_or_else(|| format!("{name}: invalid jump index {index}"))
    }

    fn enter_script(&mut self, name: String, call: bool) -> Result<(), String> {
        let pc = self
            .scripts
            .get(&name)
            .ok_or_else(|| format!("missing script {name}"))?
            .code_start;
        let frame = ScriptFrame {
            name,
            pc,
            returns: Vec::new(),
        };
        if call {
            self.frames.push(frame);
        } else {
            *self.frames.last_mut().ok_or("VM has finished")? = frame;
        }
        Ok(())
    }

    pub fn step(&mut self) -> Result<Step, String> {
        let Some(frame) = self.frames.last() else {
            return Ok(Step::Finished);
        };
        let script = self.scripts.get(&frame.name).ok_or("script disappeared")?;
        let index = script
            .instructions
            .binary_search_by_key(&frame.pc, |item| item.offset)
            .map_err(|_| format!("{}: no instruction at 0x{:X}", frame.name, frame.pc))?;
        let instruction = script.instructions[index].clone();
        let next_pc = script.instructions.get(index + 1).map(|item| item.offset);
        let name = frame.name.clone();
        let error_context = || {
            format!(
                "{name}:0x{:X} opcode 0x{:02X}",
                instruction.offset, instruction.opcode
            )
        };

        // An unsupported instruction stays at the current PC for inspection.
        if !matches!(
            instruction.opcode,
            0x01..=0x08
                | 0x13
                | 0x15..=0x16
                | 0x17
                | 0x18
                | 0x1B
                | 0x20
                | 0x21
                | 0x23
                | 0x25
                | 0x26
                | 0x27
                | 0x29
                | 0x2B
                | 0x2A
                | 0x30
                | 0x31
                | 0x32
                | 0x33
                | 0x37
                | 0x3A
                | 0x3B
                | 0x40
                | 0x44
                | 0x45
                | 0x47
                | 0x48
                | 0x49
                | 0x4A
                | 0x50
                | 0x51
                | 0x53
                | 0x55
                | 0x56
                | 0x57
                | 0x60
                | 0x70
                | 0x71
                | 0x72
                | 0x73
                | 0x74
                | 0x75
                | 0x76
                | 0x77
                | 0x78
                | 0x79
                | 0x7B
                | 0x7F
                | 0x80
                | 0x81
                | 0x84
                | 0x85..=0x87
                | 0x88
                | 0x89
                | 0xA0
                | 0xA1
                | 0xA5
                | 0xA6
                | 0xE0..=0xE4
                | 0xF0
                | 0xF4
                | 0xF5
                | 0xF7
                | 0xF8
                | 0xF9
                | 0xF1
                | 0xF2
                | 0xF3
                | 0xFA..=0xFC
        ) {
            return Ok(Step::Unsupported {
                script: name,
                offset: instruction.offset,
                opcode: instruction.opcode,
            });
        }
        if instruction.opcode == 0x85
            && self.text_page.is_some()
            && self.selected.is_none()
            && !self.click_pending
        {
            return Ok(Step::InputPoll);
        }
        if instruction.opcode == 0x16 && self.choice_selected.is_none() {
            self.waiting_choice = true;
            return Ok(Step::ChoicePoll);
        }
        if instruction.opcode == 0x18
            && instruction.payload.get(3) == Some(&0)
            && self.choice_selected.is_none()
        {
            self.waiting_choice = true;
            return Ok(Step::ChoicePoll);
        }
        if let Some(next_pc) = next_pc {
            self.frames.last_mut().unwrap().pc = next_pc;
        }

        match instruction.opcode {
            // The reference decoder accepts these setup instructions without
            // a runtime effect. Our variable/flag storage grows dynamically.
            0x13 | 0x23 | 0x30 | 0x40 | 0x48 | 0x49 | 0x80 | 0xA1 | 0xA5 | 0xA6 | 0xE0..=0xE4 => {}
            0xF0 => {
                let kind = *instruction
                    .payload
                    .first()
                    .ok_or("CNF missing asset kind")?;
                let bank = c_string(&instruction.payload, 1)?;
                match kind {
                    1 => return Ok(Step::GraphicBank(bank)),
                    4 => self.voice_bank = bank,
                    _ => return Err(format!("{}: unsupported CNF kind {kind}", error_context())),
                }
            }
            0x20 => {
                if instruction.payload.first() == Some(&0) {
                    self.text_window.x = self.decode_value(u32_at(&instruction.payload, 1)?);
                    self.text_window.y = self.decode_value(u32_at(&instruction.payload, 5)?);
                    let width = self.decode_value(u32_at(&instruction.payload, 9)?);
                    let height = self.decode_value(u32_at(&instruction.payload, 13)?);
                    if width > 0 && height > 0 {
                        self.text_window.width = width;
                        self.text_window.height = height;
                    }
                }
            }
            0x21 => {
                if u16_at(&instruction.payload, 0)? == 0 {
                    self.text_window.graphic = Some(c_string(&instruction.payload, 2)?);
                }
            }
            0x25 => {
                let index = *instruction.payload.get(1).ok_or("CNS missing name index")?;
                let bytes = instruction.payload.get(2..).ok_or("CNS missing name")?;
                let end = bytes
                    .iter()
                    .position(|&byte| byte == 0)
                    .unwrap_or(bytes.len());
                let (decoded, _, had_errors) = SHIFT_JIS.decode(&bytes[..end]);
                if had_errors {
                    return Err(format!(
                        "{}: invalid Shift-JIS speaker name",
                        error_context()
                    ));
                }
                self.speaker_names.insert(index, decoded.into_owned());
            }
            // These close the command and text windows. Drawing those windows
            // is separate from the VM's control flow.
            0x15 => {
                let value = *instruction.payload.get(1).ok_or("CSET value missing")?;
                let x = i32::from(u16_at(&instruction.payload, 2)?);
                let y = i32::from(u16_at(&instruction.payload, 6)?);
                let width = i32::from(u16_at(&instruction.payload, 10)?);
                let height = i32::from(u16_at(&instruction.payload, 14)?);
                let caption_bytes = instruction
                    .payload
                    .get(18..)
                    .ok_or("CSET caption missing")?;
                let end = caption_bytes
                    .iter()
                    .position(|&byte| byte == 0)
                    .unwrap_or(caption_bytes.len());
                let caption = shift_jis_string(&caption_bytes[..end])?;
                self.choice_options.push(ChoiceOption {
                    value,
                    caption,
                    x,
                    y,
                    width,
                    height,
                });
            }
            0x16 | 0x1B | 0x29 | 0x2A => {}
            0x17 => {
                self.choice_options.clear();
                self.waiting_choice = false;
            }
            0x18 => {
                let dst = *instruction.payload.get(1).ok_or("CC destination missing")?;
                let command = *instruction.payload.get(3).ok_or("CC missing command")?;
                match command {
                    0 => {
                        let value = self.choice_selected.take().ok_or("CC choice missing")?;
                        self.variables.insert(u16::from(dst), i32::from(value));
                        self.waiting_choice = false;
                        self.choice_options.clear();
                    }
                    1 => {
                        self.variables.insert(u16::from(dst), -1);
                    }
                    2 => {
                        self.choice_options.clear();
                        self.choice_selected = None;
                        self.waiting_choice = false;
                    }
                    3 => {
                        if let Some(value) = self.choice_selected {
                            self.variables.insert(u16::from(dst), i32::from(value));
                        }
                    }
                    _ => return Err(format!("{}: unknown CC command {command}", error_context())),
                }
            }
            0x26 => {
                if instruction.payload.first() == Some(&0) {
                    self.text_interval = self.decode_value(u32_at(&instruction.payload, 1)?);
                }
            }
            0x27 => {
                if instruction.payload.first() == Some(&0) {
                    self.font_size = self.decode_value(u32_at(&instruction.payload, 1)?);
                }
            }
            0x2B => {
                let page = decode_text_page(&instruction.payload, &self.speaker_names)?;
                if page.body.is_empty() && page.speaker.is_none() {
                    self.text_page = None;
                    return Ok(Step::ControlFlow);
                }
                self.text_page = Some(page.clone());
                return Ok(Step::TextPage(page));
            }
            0x01 | 0x02 => {
                let target = c_string(&instruction.payload, 0)
                    .map_err(|message| format!("{}: {message}", error_context()))?;
                self.enter_script(target, instruction.opcode == 0x02)?;
            }
            0x03 => {
                self.frames.pop();
            }
            0x04 | 0x05 => {
                let table = u16_at(&instruction.payload, 0)
                    .map_err(|message| format!("{}: {message}", error_context()))?;
                let target = self.jump_target(&name, table)?;
                if instruction.opcode == 0x05 {
                    self.frames
                        .last_mut()
                        .unwrap()
                        .returns
                        .push(next_pc.ok_or_else(|| {
                            format!("{}: call has no return position", error_context())
                        })?);
                }
                self.frames.last_mut().unwrap().pc = target;
            }
            0x06 => {
                let target = self
                    .frames
                    .last_mut()
                    .unwrap()
                    .returns
                    .pop()
                    .ok_or_else(|| format!("{}: return stack is empty", error_context()))?;
                self.frames.last_mut().unwrap().pc = target;
            }
            0x07 | 0x08 => {
                let selected = self.decode_value(u32_at(&instruction.payload, 0)?);
                let count = *instruction
                    .payload
                    .get(4)
                    .ok_or_else(|| format!("{}: ONJ missing count", error_context()))?
                    as i32;
                if selected >= 0 && selected < count {
                    let table = u16_at(&instruction.payload, 5 + selected as usize * 2)?;
                    let target = self.jump_target(&name, table)?;
                    if instruction.opcode == 0x08 {
                        self.frames
                            .last_mut()
                            .unwrap()
                            .returns
                            .push(next_pc.ok_or_else(|| {
                                format!("{}: ONJS has no return position", error_context())
                            })?);
                    }
                    self.frames.last_mut().unwrap().pc = target;
                }
            }
            0x31 => {
                let flag = u16_at(&instruction.payload, 0)?;
                let command = *instruction
                    .payload
                    .get(2)
                    .ok_or_else(|| format!("{}: SK missing command", error_context()))?;
                match command {
                    0 => {
                        self.flags.remove(&flag);
                    }
                    1 => {
                        self.flags.insert(flag);
                    }
                    2 => {
                        if !self.flags.remove(&flag) {
                            self.flags.insert(flag);
                        }
                    }
                    _ => return Err(format!("{}: unknown SK command {command}", error_context())),
                }
            }
            0x32 => {
                let start = u16_at(&instruction.payload, 0)?;
                let end = u16_at(&instruction.payload, 2)?;
                let command = *instruction.payload.get(4).ok_or("SKS missing command")?;
                for flag in start..end {
                    match command {
                        0 => {
                            self.flags.remove(&flag);
                        }
                        1 => {
                            self.flags.insert(flag);
                        }
                        2 => {
                            if !self.flags.remove(&flag) {
                                self.flags.insert(flag);
                            }
                        }
                        _ => {
                            return Err(format!(
                                "{}: unknown SKS command {command}",
                                error_context()
                            ));
                        }
                    }
                }
            }
            0x33 => {
                let flag = u16_at(&instruction.payload, 0)?;
                let table = u16_at(&instruction.payload, 2)?;
                if self.flags.contains(&flag) {
                    self.frames.last_mut().unwrap().pc = self.jump_target(&name, table)?;
                }
            }
            0x37 => {
                let dst = *instruction
                    .payload
                    .first()
                    .ok_or("STS missing system flag")?;
                let value = *instruction.payload.get(1).ok_or("STS missing value")?;
                if value == 0 {
                    self.system_flags.remove(&dst);
                } else {
                    self.system_flags.insert(dst);
                }
            }
            0x3A => {
                let command = *instruction
                    .payload
                    .first()
                    .ok_or_else(|| format!("{}: STX missing command", error_context()))?;
                let value = *instruction
                    .payload
                    .get(1)
                    .ok_or_else(|| format!("{}: STX missing value", error_context()))?;
                let table = u16_at(&instruction.payload, 2)?;
                let jump = match command {
                    0x08 => {
                        let clicked = self.click_pending;
                        self.click_pending = false;
                        clicked
                    }
                    0x0A | 0x0C => false,
                    0x0D => value == 1,
                    _ => self.system_flags.contains(&command) == (value != 0),
                };
                if jump {
                    self.frames.last_mut().unwrap().pc = self.jump_target(&name, table)?;
                }
            }
            0x3B => {
                let flag = u16_at(&instruction.payload, 0)?;
                let table = u16_at(&instruction.payload, 2)?;
                if !self.flags.contains(&flag) {
                    self.frames.last_mut().unwrap().pc = self.jump_target(&name, table)?;
                }
            }
            0x44 => {
                let dst = u16_at(&instruction.payload, 0)?;
                let mut result = 0i32;
                let mut pos = 2;
                loop {
                    let operation = *instruction
                        .payload
                        .get(pos)
                        .ok_or_else(|| format!("{}: CALC has no terminator", error_context()))?;
                    pos += 1;
                    if operation == 0x05 {
                        break;
                    }
                    let value = self.decode_value(u32_at(&instruction.payload, pos)?);
                    pos += 4;
                    result = match operation {
                        0 => result.wrapping_add(value),
                        1 => result.wrapping_sub(value),
                        2 => result.wrapping_mul(value),
                        3 if value != 0 => result / value,
                        4 if value != 0 => result % value,
                        _ => {
                            return Err(format!(
                                "{}: invalid CALC operation {operation}",
                                error_context()
                            ));
                        }
                    };
                }
                self.variables.insert(dst, result);
            }
            0x45 => {
                let start = u16_at(&instruction.payload, 0)?;
                let end = u16_at(&instruction.payload, 2)?;
                let value = self.decode_value(u32_at(&instruction.payload, 4)?);
                for variable in start..end {
                    self.variables.insert(variable, value);
                }
            }
            0x47 => {
                let mut pos = 0;
                loop {
                    let left = self.decode_value(u32_at(&instruction.payload, pos)?);
                    let comparison = *instruction
                        .payload
                        .get(pos + 4)
                        .ok_or_else(|| format!("{}: IF missing comparison", error_context()))?;
                    let right = self.decode_value(u32_at(&instruction.payload, pos + 5)?);
                    let result = match comparison {
                        0 => left == right,
                        1 => left < right,
                        2 => left <= right,
                        3 => left > right,
                        4 => left >= right,
                        5 => left != right,
                        _ => {
                            return Err(format!(
                                "{}: unknown IF comparison {comparison}",
                                error_context()
                            ));
                        }
                    };
                    if !result {
                        break;
                    }
                    let action = *instruction
                        .payload
                        .get(pos + 9)
                        .ok_or_else(|| format!("{}: IF missing action", error_context()))?;
                    match action {
                        0 => {
                            let table = u16_at(&instruction.payload, pos + 10)?;
                            self.frames.last_mut().unwrap().pc = self.jump_target(&name, table)?;
                            break;
                        }
                        1 => {
                            let dst = u16_at(&instruction.payload, pos + 10)?;
                            let value = self.decode_value(u32_at(&instruction.payload, pos + 12)?);
                            self.variables.insert(dst, value);
                            break;
                        }
                        2 => pos += 10,
                        0xFF => break,
                        _ => {
                            return Err(format!("{}: unknown IF action {action}", error_context()));
                        }
                    }
                }
            }
            0x50 => {
                return Ok(Step::InitSurface {
                    slot: self.decode_value(u32_at(&instruction.payload, 0)?),
                    width: self.decode_value(u32_at(&instruction.payload, 4)?),
                    height: self.decode_value(u32_at(&instruction.payload, 8)?),
                });
            }
            // The reference decoder accepts GN without a graphics effect.
            0x51 => {}
            0x53 => {
                let slot = i32::from(*instruction.payload.first().ok_or("GC missing slot")?);
                let color: [u8; 3] = instruction
                    .payload
                    .get(4..7)
                    .ok_or("GC missing color")?
                    .try_into()
                    .map_err(|_| "GC invalid color")?;
                return Ok(Step::FillSurface { slot, color });
            }
            0x56 => {
                let slot = self.decode_value(u32_at(&instruction.payload, 0)?);
                let graphic = c_string(&instruction.payload, 4)?;
                return Ok(Step::LoadGraphic {
                    slot,
                    name: graphic,
                });
            }
            0x57 => {
                let data = &instruction.payload;
                let command = *data
                    .first()
                    .ok_or_else(|| format!("{}: GP has no command", error_context()))?;
                let read = |position| -> Result<i32, String> {
                    Ok(self.decode_value(u32_at(data, position)?))
                };
                return Ok(Step::Blit(BlitOp {
                    command,
                    source: read(1)?,
                    source_x: read(5)?,
                    source_y: read(9)?,
                    width: read(13)?,
                    height: read(17)?,
                    destination: read(21)?,
                    destination_x: read(25)?,
                    destination_y: read(29)?,
                }));
            }
            0x60 => {
                return Ok(Step::ShowSurface {
                    slot: self.decode_value(u32_at(&instruction.payload, 4)?),
                    duration_ms: self.decode_value(u32_at(&instruction.payload, 8)?),
                });
            }
            // EXC is an explicit no-op in the reference implementation.
            0x4A => {}
            0x55 => {
                let duration_ms = self.decode_value(u32_at(&instruction.payload, 0)?);
                let color: [u8; 3] = instruction
                    .payload
                    .get(4..7)
                    .ok_or_else(|| format!("{}: GO missing color", error_context()))?
                    .try_into()
                    .map_err(|_| format!("{}: GO invalid color", error_context()))?;
                return Ok(Step::FadeTo { duration_ms, color });
            }
            0x70 => return Ok(Step::MusicRequested(c_string(&instruction.payload, 0)?)),
            // The reference engine starts music at ML and treats MP as a no-op.
            0x71 => {}
            0x72 => {
                return Ok(Step::MusicFade(
                    self.decode_value(u32_at(&instruction.payload, 0)?),
                ));
            }
            0x73 => return Ok(Step::MusicStopped),
            0x74 => return Ok(Step::SoundRequested(c_string(&instruction.payload, 0)?)),
            // SEP is a sound timing hint; the reference decoder has no effect.
            0x75 => {}
            0x76 => {
                let _channel = self.decode_value(u32_at(&instruction.payload, 0)?);
            }
            // PCMON and PCM cache are explicit no-ops in the reference decoder.
            0x77 | 0x7F => {}
            0x78 => {
                return Ok(Step::VoiceRequested {
                    bank: self.voice_bank.clone(),
                    name: c_string(&instruction.payload, 0)?,
                });
            }
            0x79 => return Ok(Step::VoiceStopped),
            0x7B => {
                let _channel = self.decode_value(u32_at(&instruction.payload, 0)?);
                return Ok(Step::SoundStopped);
            }
            0x81 => {
                self.cursor_style = *instruction
                    .payload
                    .first()
                    .ok_or("IC missing cursor style")?;
            }
            0x84 => {
                let data = &instruction.payload;
                let index = *data.first().ok_or("IH missing index")?;
                let x = self.decode_value(u32_at(data, 1)?);
                let y = self.decode_value(u32_at(data, 5)?);
                let right = self.decode_value(u32_at(data, 9)?);
                let bottom = self.decode_value(u32_at(data, 13)?);
                self.hit_areas.insert(
                    index,
                    HitArea {
                        index,
                        x,
                        y,
                        width: right - x,
                        height: bottom - y,
                    },
                );
            }
            0x85 => {
                let selection_register = u16_at(&instruction.payload, 0)?;
                let button_register = u16_at(&instruction.payload, 2)?;
                self.variables
                    .insert(selection_register, self.selected.map_or(-1, i32::from));
                self.variables
                    .insert(button_register, if self.click_pending { 2 } else { 0 });
                self.click_pending = false;
            }
            0x86 | 0x87 => self.click_pending = false,
            // Keyboard mapping and read-text marks are retained for a later
            // input/skip milestone; they have no branch side effects here.
            0x88 | 0x89 | 0xA0 => {}
            0xF4 => {
                self.popup_enabled = instruction.payload.first() == Some(&1);
            }
            0xF5 => {
                self.save_enabled = instruction.payload.first() == Some(&1);
            }
            // Savegame display name; no save UI exists yet.
            0xF7 => {
                let bytes = &instruction.payload;
                let end = bytes
                    .iter()
                    .position(|&byte| byte == 0)
                    .unwrap_or(bytes.len());
                let (_, _, had_errors) = SHIFT_JIS.decode(&bytes[..end]);
                if had_errors {
                    return Err(format!("{}: invalid Shift-JIS save title", error_context()));
                }
            }
            0xF8 => {
                self.font_style = *instruction
                    .payload
                    .get(1)
                    .ok_or("SETFONTSTYLE missing style")?;
            }
            0xF9 => {
                if instruction.payload.first() == Some(&0) {
                    let font = *instruction
                        .payload
                        .get(1)
                        .ok_or("SETFONTCOLOR missing font")?;
                    let color: [u8; 3] = instruction
                        .payload
                        .get(2..5)
                        .ok_or("SETFONTCOLOR missing RGB")?
                        .try_into()
                        .map_err(|_| "SETFONTCOLOR invalid RGB")?;
                    self.text_window.colors.insert(font, color);
                }
            }
            0xF1 => {
                self.delay_ms = self.decode_value(u32_at(&instruction.payload, 0)?).max(0) as u64;
            }
            0xF2 => return Ok(Step::Wait(self.delay_ms)),
            0xF3 => {
                return Ok(Step::PlayVideo {
                    source_width: self.decode_value(u32_at(&instruction.payload, 0)?),
                    source_height: self.decode_value(u32_at(&instruction.payload, 4)?),
                    destination_width: self.decode_value(u32_at(&instruction.payload, 8)?),
                    destination_height: self.decode_value(u32_at(&instruction.payload, 12)?),
                    name: c_string(&instruction.payload, 16)?,
                });
            }
            0xFA => {
                self.timer_start_ms = self
                    .now_ms
                    .saturating_add(u32_at(&instruction.payload, 0)? as u64);
            }
            0xFB => {}
            0xFC => {
                let dst = u16_at(&instruction.payload, 0)?;
                let elapsed = self
                    .now_ms
                    .saturating_sub(self.timer_start_ms)
                    .min(i32::MAX as u64) as i32;
                self.variables.insert(dst, elapsed);
                return Ok(Step::Yield);
            }
            _ => unreachable!(),
        }
        Ok(Step::ControlFlow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn script(code_start: usize, jumps: Vec<usize>, instructions: Vec<Instruction>) -> Script {
        Script {
            code_start,
            jumps,
            instructions,
        }
    }

    fn op(offset: usize, opcode: u8, payload: &[u8]) -> Instruction {
        Instruction {
            offset,
            opcode,
            payload: payload.to_vec(),
        }
    }

    #[test]
    fn text_page_waits_for_click_and_decodes_shift_jis() {
        let (encoded, _, _) = SHIFT_JIS.encode("どうも騙されたようだ。");
        let mut payload = vec![0, 3, 0xFF];
        payload.extend_from_slice(&encoded);
        payload.push(0);
        let mut scripts = Scripts::new();
        scripts.insert(
            "SNR00".into(),
            script(
                0,
                vec![],
                vec![op(0, 0x2B, &payload), op(1, 0x85, &[1, 0, 2, 0, 1, 4])],
            ),
        );
        let mut vm = Vm::new(scripts, "SNR00").unwrap();
        assert_eq!(
            vm.step().unwrap(),
            Step::TextPage(TextPage {
                speaker: None,
                body: "どうも騙されたようだ。".into(),
            })
        );
        assert_eq!(vm.step().unwrap(), Step::InputPoll);
        assert_eq!(vm.position(), Some(("SNR00", 1)));
        vm.click();
        assert_eq!(vm.step().unwrap(), Step::ControlFlow);
        assert_eq!(vm.variables.get(&2), Some(&2));
    }

    #[test]
    fn pm_speaker_command_becomes_the_page_header() {
        let (encoded_name, _, _) = SHIFT_JIS.encode("司");
        let mut name_payload = vec![0, 1];
        name_payload.extend_from_slice(&encoded_name);
        name_payload.push(0);
        let (encoded_body, _, _) = SHIFT_JIS.encode("生きてるか？");
        let mut page_payload = vec![0, 4, 1, 0xFF];
        page_payload.extend_from_slice(&encoded_body);
        page_payload.push(0);
        let mut scripts = Scripts::new();
        scripts.insert(
            "SNR00".into(),
            script(
                0,
                vec![],
                vec![
                    op(0, 0x25, &name_payload),
                    op(1, 0x2B, &page_payload),
                    op(2, 0x85, &[1, 0, 2, 0, 1, 1]),
                ],
            ),
        );
        let mut vm = Vm::new(scripts, "SNR00").unwrap();
        vm.step().unwrap();
        assert_eq!(
            vm.step().unwrap(),
            Step::TextPage(TextPage {
                speaker: Some("司".into()),
                body: "生きてるか？".into(),
            })
        );
        let page = vm.text_page().unwrap();
        assert_eq!(page.speaker.as_deref(), Some("司"));
        assert_eq!(page.body, "生きてるか？");
        assert_eq!(page.render(), "司\n生きてるか？");
    }

    #[test]
    fn ih_rectangles_and_ig_click_are_script_driven() {
        let mut scripts = Scripts::new();
        scripts.insert(
            "TITLE".into(),
            script(
                0,
                vec![],
                vec![
                    op(
                        0,
                        0x84,
                        &[
                            0, 10, 0, 0, 0, 20, 0, 0, 0, 40, 0, 0, 0, 60, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                        ],
                    ),
                    op(1, 0x85, &[7, 0, 8, 0, 4, 1]),
                    op(2, 0x85, &[9, 0, 10, 0, 5, 1]),
                ],
            ),
        );
        let mut vm = Vm::new(scripts, "TITLE").unwrap();
        vm.step().unwrap();
        assert_eq!(vm.hit_areas().get(&0).unwrap().width, 30);
        assert_eq!(vm.hit_areas().get(&0).unwrap().height, 40);
        vm.set_selected(Some(0));
        vm.step().unwrap();
        vm.step().unwrap();
        assert_eq!(vm.variables.get(&7), Some(&0));
        assert_eq!(vm.variables.get(&8), Some(&2));
        assert_eq!(vm.variables.get(&9), Some(&0));
        assert_eq!(vm.variables.get(&10), Some(&0));
    }

    #[test]
    fn avip_decodes_video_name_and_destination_size() {
        let mut scripts = Scripts::new();
        scripts.insert(
            "START".into(),
            script(
                0,
                vec![],
                vec![op(
                    0,
                    0xF3,
                    &[
                        0, 0, 0, 0, 0, 0, 0, 0, 0x80, 2, 0, 0, 0xE0, 1, 0, 0, b'D', b'O', b'L',
                        b'O', b'G', b'O', 0,
                    ],
                )],
            ),
        );
        let mut vm = Vm::new(scripts, "START").unwrap();
        assert_eq!(
            vm.step().unwrap(),
            Step::PlayVideo {
                name: "DOLOGO".into(),
                source_width: 0,
                source_height: 0,
                destination_width: 640,
                destination_height: 480,
            }
        );
    }

    #[test]
    fn startup_stores_speaker_names_and_text_window_configuration() {
        let (encoded_name, _, _) = SHIFT_JIS.encode("家族");
        let mut name_payload = vec![0, 7];
        name_payload.extend_from_slice(&encoded_name);
        name_payload.push(0);
        let mut scripts = Scripts::new();
        scripts.insert(
            "START".into(),
            script(
                0,
                vec![],
                vec![
                    op(
                        0,
                        0x21,
                        &[0, 0, b'K', b'A', b'_', b'M', b'.', b'G', b'G', b'2', 0],
                    ),
                    op(
                        1,
                        0x20,
                        &[0, 44, 0, 0, 0, 64, 1, 0, 0, 128, 2, 0, 0, 224, 1, 0, 0, 2],
                    ),
                    op(2, 0xF9, &[0, 0, 255, 240, 220]),
                    op(3, 0x25, &name_payload),
                ],
            ),
        );
        let mut vm = Vm::new(scripts, "START").unwrap();
        for _ in 0..4 {
            vm.step().unwrap();
        }
        assert_eq!(vm.speaker_names().get(&7).map(String::as_str), Some("家族"));
        assert_eq!(vm.text_window().graphic.as_deref(), Some("KA_M.GG2"));
        assert_eq!((vm.text_window().x, vm.text_window().y), (44, 320));
        assert_eq!(
            (vm.text_window().width, vm.text_window().height),
            (640, 480)
        );
        assert_eq!(vm.text_window().colors.get(&0), Some(&[255, 240, 220]));
    }

    #[test]
    fn jump_call_and_return_use_the_jump_table() {
        let mut scripts = Scripts::new();
        scripts.insert(
            "START".into(),
            script(
                10,
                vec![30],
                vec![
                    op(10, 0x05, &[0, 0]),
                    op(12, 0x56, &[0, 0, 0, 0, b'T', 0]),
                    op(30, 0x06, &[]),
                ],
            ),
        );
        let mut vm = Vm::new(scripts, "START").unwrap();
        assert_eq!(vm.step().unwrap(), Step::ControlFlow);
        assert_eq!(vm.position(), Some(("START", 30)));
        assert_eq!(vm.step().unwrap(), Step::ControlFlow);
        assert_eq!(vm.position(), Some(("START", 12)));
        assert_eq!(
            vm.step().unwrap(),
            Step::LoadGraphic {
                slot: 0,
                name: "T".into()
            }
        );
    }

    #[test]
    fn subscript_returns_to_caller() {
        let mut scripts = Scripts::new();
        scripts.insert(
            "START".into(),
            script(
                10,
                vec![],
                vec![
                    op(10, 0x02, b"INIT\0"),
                    op(20, 0x56, &[0, 0, 0, 0, b'T', 0]),
                ],
            ),
        );
        scripts.insert("INIT".into(), script(30, vec![], vec![op(30, 0x03, &[])]));
        let mut vm = Vm::new(scripts, "START").unwrap();
        vm.step().unwrap();
        assert_eq!(vm.position(), Some(("INIT", 30)));
        vm.step().unwrap();
        assert_eq!(vm.position(), Some(("START", 20)));
    }

    #[test]
    fn unsupported_instruction_does_not_advance() {
        let mut scripts = Scripts::new();
        scripts.insert("START".into(), script(10, vec![], vec![op(10, 0xFF, &[])]));
        let mut vm = Vm::new(scripts, "START").unwrap();
        assert_eq!(
            vm.step().unwrap(),
            Step::Unsupported {
                script: "START".into(),
                offset: 10,
                opcode: 0xFF
            }
        );
        assert_eq!(vm.position(), Some(("START", 10)));
    }

    #[test]
    fn calc_writes_register_and_gl_reads_it() {
        let mut scripts = Scripts::new();
        scripts.insert(
            "START".into(),
            script(
                10,
                vec![],
                vec![
                    // register 7 = 3 + 4
                    op(10, 0x44, &[7, 0, 0, 3, 0, 0, 0, 0, 4, 0, 0, 0, 5]),
                    // GL slot = register 7
                    op(20, 0x56, &[7, 0, 0, 0x80, b'T', 0]),
                ],
            ),
        );
        let mut vm = Vm::new(scripts, "START").unwrap();
        assert_eq!(vm.step().unwrap(), Step::ControlFlow);
        assert_eq!(
            vm.step().unwrap(),
            Step::LoadGraphic {
                slot: 7,
                name: "T".into()
            }
        );
    }

    #[test]
    fn ls_replaces_current_script() {
        let mut scripts = Scripts::new();
        scripts.insert(
            "START".into(),
            script(10, vec![], vec![op(10, 0x01, b"TITLE\0")]),
        );
        scripts.insert("TITLE".into(), script(30, vec![], vec![op(30, 0x03, &[])]));
        let mut vm = Vm::new(scripts, "START").unwrap();
        vm.step().unwrap();
        assert_eq!(vm.position(), Some(("TITLE", 30)));
        vm.step().unwrap();
        assert_eq!(vm.position(), None);
        assert_eq!(vm.step().unwrap(), Step::Finished);
    }

    #[test]
    fn flag_controls_conditional_jump() {
        let mut scripts = Scripts::new();
        scripts.insert(
            "START".into(),
            script(
                10,
                vec![40],
                vec![
                    op(10, 0x31, &[9, 0, 1]),
                    op(20, 0x33, &[9, 0, 0, 0]),
                    op(30, 0x56, &[0, 0, 0, 0, b'X', 0]),
                    op(40, 0x56, &[0, 0, 0, 0, b'Y', 0]),
                ],
            ),
        );
        let mut vm = Vm::new(scripts, "START").unwrap();
        vm.step().unwrap();
        vm.step().unwrap();
        assert_eq!(vm.position(), Some(("START", 40)));
    }

    #[test]
    fn if_jumps_only_when_comparison_matches() {
        let mut scripts = Scripts::new();
        scripts.insert(
            "START".into(),
            script(
                10,
                vec![40],
                vec![
                    op(10, 0x47, &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xFF]),
                    op(20, 0x47, &[0, 0, 0, 0, 5, 0, 0, 0, 0, 0, 0, 0, 0xFF]),
                    op(30, 0x03, &[]),
                    op(40, 0x03, &[]),
                ],
            ),
        );
        let mut vm = Vm::new(scripts, "START").unwrap();
        vm.step().unwrap();
        assert_eq!(vm.position(), Some(("START", 40)));
        vm.set_position(20).unwrap();
        vm.step().unwrap();
        assert_eq!(vm.position(), Some(("START", 30)));
    }

    #[test]
    fn onjs_selects_branch_and_returns() {
        let mut scripts = Scripts::new();
        scripts.insert(
            "START".into(),
            script(
                10,
                vec![30, 40],
                vec![
                    op(10, 0x44, &[7, 0, 0, 1, 0, 0, 0, 5]),
                    op(20, 0x08, &[7, 0, 0, 0x80, 2, 0, 0, 1, 0]),
                    op(25, 0x56, &[0, 0, 0, 0, b'T', 0]),
                    op(30, 0x06, &[]),
                    op(40, 0x06, &[]),
                ],
            ),
        );
        let mut vm = Vm::new(scripts, "START").unwrap();
        vm.step().unwrap();
        vm.step().unwrap();
        assert_eq!(vm.position(), Some(("START", 40)));
        vm.step().unwrap();
        assert_eq!(vm.position(), Some(("START", 25)));
    }

    #[test]
    fn timer_yields_and_input_updates_registers() {
        let mut scripts = Scripts::new();
        scripts.insert(
            "START".into(),
            script(
                10,
                vec![],
                vec![
                    op(10, 0xFA, &[0, 0, 0, 0]),
                    op(20, 0xFC, &[3, 0]),
                    op(30, 0x85, &[4, 0, 5, 0, 1, 1]),
                    op(40, 0x56, &[3, 0, 0, 0x80, b'T', 0]),
                ],
            ),
        );
        let mut vm = Vm::new(scripts, "START").unwrap();
        vm.step().unwrap();
        vm.advance_clock(32);
        assert_eq!(vm.step().unwrap(), Step::Yield);
        vm.set_selected(Some(2));
        vm.step().unwrap();
        assert_eq!(
            vm.step().unwrap(),
            Step::LoadGraphic {
                slot: 32,
                name: "T".into()
            }
        );
    }
}
