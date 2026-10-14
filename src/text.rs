use fontdue::{Font, FontSettings};
use image::{Rgba, RgbaImage};
use std::path::{Path, PathBuf};

pub struct TextRenderer {
    font: Font,
}

impl TextRenderer {
    pub fn new() -> Result<Self, String> {
        let candidates: Vec<PathBuf> = if let Some(path) = std::env::var_os("IKURA_FONT") {
            vec![PathBuf::from(path)]
        } else {
            [
                "/System/Library/Fonts/ヒラギノ角ゴシック W8.ttc",
                "/System/Library/Fonts/Hiragino Sans GB.ttc",
                "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
                "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
            ]
            .iter()
            .map(PathBuf::from)
            .collect()
        };
        let path = candidates
            .iter()
            .find(|path| path.is_file())
            .ok_or("no Japanese font found; set IKURA_FONT to a TTF/TTC font")?;
        Self::from_path(path)
    }

    fn from_path(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
        let font = Font::from_bytes(bytes, FontSettings::default())
            .map_err(|error| format!("{}: {error}", path.display()))?;
        Ok(Self { font })
    }

    pub fn draw_page(&self, background: &RgbaImage, page: &str) -> RgbaImage {
        let mut frame = background.clone();
        let width = frame.width() as i32;
        let height = frame.height() as i32;
        let top = height - 148;
        for y in top.max(0)..height {
            for x in 0..width {
                let pixel = frame.get_pixel_mut(x as u32, y as u32);
                for channel in 0..3 {
                    pixel[channel] = ((u16::from(pixel[channel]) * 65 + 190 * 20) / 255) as u8;
                }
            }
        }
        let px = 22.0;
        let left = 28;
        let right = width - 28;
        let line_height = 30;
        let ascent = self
            .font
            .horizontal_line_metrics(px)
            .map_or(23.0, |m| m.ascent);
        let mut x = left;
        let mut baseline = top + 20 + ascent.ceil() as i32;
        for ch in page.chars() {
            if ch == '\n' {
                x = left;
                baseline += line_height;
                continue;
            }
            let (metrics, bitmap) = self.font.rasterize(ch, px);
            let advance = metrics.advance_width.ceil() as i32;
            if x + advance > right {
                x = left;
                baseline += line_height;
            }
            let glyph_top = baseline - metrics.ymin - metrics.height as i32;
            for gy in 0..metrics.height {
                for gx in 0..metrics.width {
                    let dx = x + metrics.xmin + gx as i32;
                    let dy = glyph_top + gy as i32;
                    if dx < 0 || dy < 0 || dx >= width || dy >= height {
                        continue;
                    }
                    let alpha = u16::from(bitmap[gy * metrics.width + gx]);
                    let target = frame.get_pixel_mut(dx as u32, dy as u32);
                    for channel in 0..3 {
                        target[channel] = ((u16::from(target[channel]) * (255 - alpha)
                            + 245 * alpha)
                            / 255) as u8;
                    }
                    target[3] = 255;
                }
            }
            x += advance.max(1);
        }
        // Small marker: click to continue.
        for y in (height - 17).max(0)..(height - 11).max(0) {
            for x in (width - 18).max(0)..(width - 12).max(0) {
                frame.put_pixel(x as u32, y as u32, Rgba([245, 245, 245, 255]));
            }
        }
        frame
    }
}
