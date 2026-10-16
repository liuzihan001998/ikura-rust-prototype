//! Original Windows CUR/ANI assets, read directly from the DATA cabinet.
use crate::assets::DataArchive;
use image::{Rgba, RgbaImage};
use sdl2::{
    mouse::{Cursor, SystemCursor},
    pixels::PixelFormatEnum,
    surface::Surface,
};
use std::{collections::HashMap, path::Path, time::Instant};

fn word(bytes: &[u8], offset: usize) -> Result<u16, String> {
    Ok(u16::from_le_bytes(
        bytes
            .get(offset..offset + 2)
            .ok_or("truncated cursor word")?
            .try_into()
            .unwrap(),
    ))
}
fn dword(bytes: &[u8], offset: usize) -> Result<u32, String> {
    Ok(u32::from_le_bytes(
        bytes
            .get(offset..offset + 4)
            .ok_or("truncated cursor dword")?
            .try_into()
            .unwrap(),
    ))
}

pub struct CursorFrame {
    pub pixels: RgbaImage,
    pub hotspot: (i32, i32),
}

fn decode_cur(data: &[u8]) -> Result<CursorFrame, String> {
    if word(data, 0)? != 0 || word(data, 2)? != 2 || word(data, 4)? == 0 {
        return Err("invalid CUR header".into());
    }
    let hotspot = (i32::from(word(data, 10)?), i32::from(word(data, 12)?));
    let start = dword(data, 18)? as usize;
    let size = dword(data, 14)? as usize;
    let dib = data
        .get(start..start.checked_add(size).ok_or("CUR size overflow")?)
        .ok_or("truncated CUR image")?;
    if dword(dib, 0)? != 40 || dword(dib, 16)? != 0 || word(dib, 12)? != 1 {
        return Err("CUR requires an uncompressed BITMAPINFOHEADER".into());
    }
    let width = dword(dib, 4)? as usize;
    let doubled_height = dword(dib, 8)? as usize;
    let height = doubled_height / 2;
    if !(1..=256).contains(&width)
        || !(1..=256).contains(&height)
        || !doubled_height.is_multiple_of(2)
        || hotspot.0 < 0
        || hotspot.1 < 0
        || hotspot.0 as usize >= width
        || hotspot.1 as usize >= height
    {
        return Err("invalid CUR size or hotspot".into());
    }
    let depth = word(dib, 14)? as usize;
    if !matches!(depth, 1 | 4 | 8 | 24 | 32) {
        return Err(format!("unsupported CUR depth {depth}"));
    }
    let palette_size = if depth <= 8 {
        let declared = dword(dib, 32)? as usize;
        let size = if declared == 0 { 1 << depth } else { declared };
        if size > 1 << depth {
            return Err("invalid CUR palette".into());
        }
        size
    } else {
        0
    };
    let pixels_start = 40 + palette_size * 4;
    let stride = (width * depth).div_ceil(32) * 4;
    let mask_start = pixels_start + stride * height;
    let mask_stride = width.div_ceil(32) * 4;
    if dib.len() < mask_start + mask_stride * height {
        return Err("truncated CUR pixels or mask".into());
    }
    let alpha_present = depth == 32
        && (0..height).any(|y| (0..width).any(|x| dib[pixels_start + y * stride + x * 4 + 3] != 0));
    let mut pixels = RgbaImage::new(width as u32, height as u32);
    for y in 0..height {
        let row = height - 1 - y;
        for x in 0..width {
            let offset = pixels_start + row * stride;
            let (b, g, r, alpha) = if depth <= 8 {
                let index = match depth {
                    1 => (dib[offset + x / 8] >> (7 - x % 8)) & 1,
                    4 => (dib[offset + x / 2] >> (4 * (1 - x % 2))) & 15,
                    _ => dib[offset + x],
                } as usize;
                if index >= palette_size {
                    return Err("CUR palette index out of range".into());
                }
                let p = 40 + index * 4;
                (dib[p], dib[p + 1], dib[p + 2], 255)
            } else {
                let p = offset + x * (depth / 8);
                (
                    dib[p],
                    dib[p + 1],
                    dib[p + 2],
                    if alpha_present { dib[p + 3] } else { 255 },
                )
            };
            let transparent = dib[mask_start + row * mask_stride + x / 8] & (128 >> (x % 8)) != 0;
            pixels.put_pixel(
                x as u32,
                y as u32,
                Rgba([r, g, b, if transparent { 0 } else { alpha }]),
            );
        }
    }
    Ok(CursorFrame { pixels, hotspot })
}

type RiffChunk<'a> = (&'a [u8], &'a [u8]);

fn chunks(data: &[u8]) -> Result<Vec<RiffChunk<'_>>, String> {
    let mut result = Vec::new();
    let mut offset = 0;
    while offset < data.len() {
        let size = dword(data, offset + 4)? as usize;
        let end = (offset + 8).checked_add(size).ok_or("ANI chunk overflow")?;
        let payload = data.get(offset + 8..end).ok_or("truncated ANI chunk")?;
        result.push((&data[offset..offset + 4], payload));
        offset = end + (size & 1);
        if offset > data.len() {
            return Err("missing ANI padding".into());
        }
    }
    Ok(result)
}

pub struct CursorAnimation {
    pub frames: Vec<CursorFrame>,
    sequence: Vec<usize>,
    rates: Vec<u32>, // Windows jiffies: 1/60 second.
}

impl CursorAnimation {
    pub fn decode(data: &[u8]) -> Result<Self, String> {
        if data.get(..4) != Some(b"RIFF") {
            return Ok(Self {
                frames: vec![decode_cur(data)?],
                sequence: vec![0],
                rates: vec![1],
            });
        }
        if data.get(8..12) != Some(b"ACON") {
            return Err("RIFF cursor is not ACON".into());
        }
        let declared = dword(data, 4)? as usize;
        // The game's ANI files put the entire file length in the RIFF size
        // field, rather than length minus 8. Accept that exact legacy form;
        // individual chunks and frames still undergo full bounds checks.
        let end = if declared == data.len() {
            declared
        } else {
            declared.checked_add(8).ok_or("ANI size overflow")?
        };
        let mut header = None;
        let mut frames = Vec::new();
        let mut rates = None;
        let mut sequence = None;
        for (tag, payload) in chunks(data.get(12..end).ok_or("truncated ANI RIFF")?)? {
            match tag {
                b"anih" => {
                    header = Some(payload);
                }
                b"rate" | b"seq " => {
                    if payload.len() % 4 != 0 {
                        return Err("invalid ANI rate/sequence table".into());
                    }
                    let table: Result<Vec<_>, _> = (0..payload.len())
                        .step_by(4)
                        .map(|i| dword(payload, i))
                        .collect();
                    if tag == b"rate" {
                        rates = Some(table?);
                    } else {
                        sequence = Some(table?);
                    }
                }
                b"LIST" if payload.get(..4) == Some(b"fram") => {
                    for (tag, frame) in chunks(&payload[4..])? {
                        if tag == b"icon" {
                            if frames.len() >= 256 {
                                return Err("too many ANI frames".into());
                            }
                            frames.push(decode_cur(frame)?);
                        }
                    }
                }
                _ => {}
            }
        }
        let header = header.ok_or("ANI has no anih header")?;
        let count = dword(header, 4)? as usize;
        let steps = dword(header, 8)? as usize;
        let rate = dword(header, 28)?;
        let flags = dword(header, 32)?;
        if dword(header, 0)? != 36
            || count == 0
            || count != frames.len()
            || !(1..=4096).contains(&steps)
            || flags & 1 == 0
        {
            return Err("invalid ANI frame header".into());
        }
        let sequence: Vec<usize> = if flags & 2 != 0 {
            sequence
                .ok_or("ANI sequence missing")?
                .into_iter()
                .map(|n| n as usize)
                .collect()
        } else {
            (0..steps).collect()
        };
        let rates = rates.unwrap_or_else(|| vec![rate; steps]);
        if sequence.len() != steps
            || rates.len() != steps
            || rates.contains(&0)
            || sequence.iter().any(|&n| n >= frames.len())
        {
            return Err("invalid ANI timing or frame sequence".into());
        }
        Ok(Self {
            frames,
            sequence,
            rates,
        })
    }

    pub fn frame_at(&self, elapsed_ms: u128) -> usize {
        let period: u128 = self.rates.iter().map(|&r| u128::from(r) * 1000).sum();
        let mut time = elapsed_ms.saturating_mul(60) % period;
        for (&frame, &rate) in self.sequence.iter().zip(&self.rates) {
            let duration = u128::from(rate) * 1000;
            if time < duration {
                return frame;
            }
            time -= duration;
        }
        self.sequence[0]
    }
}

struct NativeAnimation {
    timing: CursorAnimation,
    cursors: Vec<Cursor>,
}

pub struct GameCursors {
    data: Option<DataArchive>,
    system: Option<Cursor>,
    cache: HashMap<String, NativeAnimation>,
    active: Option<(u8, Option<String>)>,
    frame: Option<usize>,
    started: Instant,
}

impl GameCursors {
    pub fn new(data_dir: &Path) -> Result<Self, String> {
        let data = DataArchive::load(data_dir, Some("IKURA_CURSOR_DATA"))?;
        Ok(Self {
            data,
            system: if cfg!(target_os = "android") {
                None
            } else {
                Some(Cursor::from_system(SystemCursor::Arrow)?)
            },
            cache: HashMap::new(),
            active: None,
            frame: None,
            started: Instant::now(),
        })
    }

    pub fn update(&mut self, id: u8, name: Option<&str>) -> Result<(), String> {
        if cfg!(target_os = "android") {
            return Ok(());
        }
        let changed = self
            .active
            .as_ref()
            .is_none_or(|(old_id, old_name)| *old_id != id || old_name.as_deref() != name);
        if changed {
            if let (Some(name), Some(data)) = (name, &self.data) {
                if !self.cache.contains_key(name) {
                    let timing = CursorAnimation::decode(data.entry(name)?)?;
                    let mut cursors = Vec::new();
                    for frame in &timing.frames {
                        let mut surface = Surface::new(
                            frame.pixels.width(),
                            frame.pixels.height(),
                            PixelFormatEnum::RGBA32,
                        )?;
                        let pitch = surface.pitch() as usize;
                        let row_bytes = frame.pixels.width() as usize * 4;
                        surface.with_lock_mut(|pixels| {
                            for (y, row) in frame.pixels.as_raw().chunks(row_bytes).enumerate() {
                                pixels[y * pitch..y * pitch + row_bytes].copy_from_slice(row);
                            }
                        });
                        cursors.push(Cursor::from_surface(
                            &surface,
                            frame.hotspot.0,
                            frame.hotspot.1,
                        )?);
                    }
                    println!("IM cursor {id}: {name}, {} frames", cursors.len());
                    self.cache
                        .insert(name.into(), NativeAnimation { timing, cursors });
                }
            } else {
                if let Some(system) = &self.system {
                    system.set();
                }
                if name.is_some() {
                    eprintln!(
                        "Cursor DATA unavailable; using system arrow for IC {id}. Set IKURA_CURSOR_DATA."
                    );
                }
            }
            self.active = Some((id, name.map(str::to_owned)));
            self.started = Instant::now();
            self.frame = None;
        }
        if let Some(animation) = name.and_then(|name| self.cache.get(name)) {
            let frame = animation
                .timing
                .frame_at(self.started.elapsed().as_millis());
            if self.frame != Some(frame) {
                animation.cursors[frame].set();
                self.frame = Some(frame);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::cabinet_entry;

    fn cur_fixture() -> Vec<u8> {
        let mut cur = vec![0; 86];
        cur[2..4].copy_from_slice(&2u16.to_le_bytes());
        cur[4..6].copy_from_slice(&1u16.to_le_bytes());
        cur[6] = 2;
        cur[7] = 2;
        cur[10..12].copy_from_slice(&1u16.to_le_bytes());
        cur[14..18].copy_from_slice(&64u32.to_le_bytes());
        cur[18..22].copy_from_slice(&22u32.to_le_bytes());
        let dib = &mut cur[22..];
        for (offset, value) in [(0, 40u32), (4, 2), (8, 4), (32, 2)] {
            dib[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        dib[12..14].copy_from_slice(&1u16.to_le_bytes());
        dib[14..16].copy_from_slice(&8u16.to_le_bytes());
        dib[46] = 255; // Palette entry 1: red in BGRA order.
        dib[48] = 1; // Bottom row starts red.
        dib[53] = 1; // Top row ends red.
        dib[60] = 128; // Top-left pixel transparent in the AND mask.
        cur
    }

    fn chunk(tag: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = tag.to_vec();
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(payload);
        if !payload.len().is_multiple_of(2) {
            out.push(0);
        }
        out
    }

    fn ani_fixture() -> Vec<u8> {
        let header: Vec<u8> = [36u32, 2, 3, 0, 0, 0, 0, 6, 3]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect();
        let mut body = b"ACON".to_vec();
        body.extend(chunk(b"anih", &header));
        body.extend(chunk(
            b"rate",
            &[1u32, 2, 3]
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
        ));
        body.extend(chunk(
            b"seq ",
            &[1u32, 0, 1]
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect::<Vec<_>>(),
        ));
        let mut frames = b"fram".to_vec();
        frames.extend(chunk(b"icon", &cur_fixture()));
        frames.extend(chunk(b"icon", &cur_fixture()));
        body.extend(chunk(b"LIST", &frames));
        chunk(b"RIFF", &body)
    }

    #[test]
    fn cur_preserves_hotspot_palette_mask_and_bottom_up_rows() {
        let frame = decode_cur(&cur_fixture()).unwrap();
        assert_eq!(frame.hotspot, (1, 0));
        assert_eq!(frame.pixels.dimensions(), (2, 2));
        assert_eq!(frame.pixels.get_pixel(0, 0).0, [0, 0, 0, 0]);
        assert_eq!(frame.pixels.get_pixel(1, 0).0, [255, 0, 0, 255]);
        assert_eq!(frame.pixels.get_pixel(0, 1).0, [255, 0, 0, 255]);
        assert_eq!(frame.pixels.get_pixel(1, 1).0, [0, 0, 0, 255]);
    }

    #[test]
    fn ani_obeys_sequence_rates_and_loops_without_rounding_drift() {
        let animation = CursorAnimation::decode(&ani_fixture()).unwrap();
        assert_eq!(animation.frames.len(), 2);
        for (ms, expected) in [
            (0, 1),
            (16, 1),
            (17, 0),
            (49, 0),
            (50, 1),
            (99, 1),
            (100, 1),
            (117, 0),
            (1_000_017, 0),
        ] {
            assert_eq!(animation.frame_at(ms), expected, "at {ms} ms");
        }
        let mut legacy = ani_fixture();
        let size = legacy.len() as u32;
        legacy[4..8].copy_from_slice(&size.to_le_bytes());
        assert_eq!(CursorAnimation::decode(&legacy).unwrap().frame_at(17), 0);
    }

    #[test]
    fn malformed_resources_are_rejected() {
        let cur = cur_fixture();
        for size in [0, 5, 21, 30, 85] {
            assert!(CursorAnimation::decode(&cur[..size]).is_err());
        }
        let mut bad = cur.clone();
        bad[10..12].copy_from_slice(&2u16.to_le_bytes());
        assert!(CursorAnimation::decode(&bad).is_err());
        let mut bad = ani_fixture();
        bad[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(CursorAnimation::decode(&bad).is_err());
        assert!(cabinet_entry(b"bad", "KA_CUR").is_err());
    }

    #[test]
    fn actual_game_cursor_resources_decode() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("DATA");
        if !path.is_file() {
            return;
        }
        let data = std::fs::read(path).unwrap();
        for (name, frames, hotspot) in [
            ("KA_CUR", 1, (0, 0)),
            ("KA_TEXT", 5, (15, 15)),
            ("KA_AUTO", 8, (15, 15)),
            ("KA_BACK", 5, (15, 15)),
        ] {
            let animation = CursorAnimation::decode(cabinet_entry(&data, name).unwrap()).unwrap();
            assert_eq!(animation.frames.len(), frames);
            for frame in &animation.frames {
                assert_eq!(frame.hotspot, hotspot);
                assert_eq!(frame.pixels.dimensions(), (32, 32));
                assert!(frame.pixels.pixels().any(|pixel| pixel[3] == 0));
                assert!(frame.pixels.pixels().any(|pixel| pixel[3] == 255));
            }
        }
    }
}
