//! Story text composition. Decoded PM pages are drawn into a frame image that
//! mirrors the original text window: `KA_M.GG2` for the message box and the
//! `SETFONTCOLOR` palette for speaker names and body text.

use fontdue::{Font, FontSettings};
use image::{Rgba, RgbaImage};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const PADDING: i32 = 12;
const LINE_SPACING: i32 = 2;
const NAME_GAP: i32 = 6;
const NAME_SCALE: f32 = 0.85;

/// Geometry and palette for one composed story page.
pub struct TextWindowLayout {
    /// Left edge of the text window rectangle in screen pixels.
    pub x: i32,
    /// Top edge of the text window rectangle in screen pixels.
    pub y: i32,
    /// Width of the text area in screen pixels.
    pub width: i32,
    /// Height of the text area; 0 grows to fit the page.
    pub height: i32,
    pub font_size: i32,
    /// Optional original window graphic drawn behind the text.
    pub frame: Option<RgbaImage>,
    /// `SETFONTCOLOR` palette keyed by the font byte from the script.
    pub colors: HashMap<u8, [u8; 3]>,
}

impl TextWindowLayout {
    fn body_color(&self) -> [u8; 3] {
        self.colors
            .get(&0)
            .copied()
            .unwrap_or([255, 255, 255])
    }

    fn name_color(&self) -> [u8; 3] {
        self.colors
            .get(&1)
            .copied()
            .or_else(|| self.colors.get(&0).copied())
            .unwrap_or([255, 255, 255])
    }

    fn font_px(&self) -> f32 {
        if self.font_size > 0 {
            self.font_size as f32
        } else {
            22.0
        }
    }
}

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

    /// Compose one story page: the window graphic, an optional speaker name,
    /// and the wrapped body text.
    pub fn draw_page(
        &self,
        background: &RgbaImage,
        speaker: Option<&str>,
        body: &str,
        layout: &TextWindowLayout,
    ) -> RgbaImage {
        let mut frame = background.clone();
        if let Some(graphic) = &layout.frame {
            self.blit_frame(&mut frame, graphic, layout);
        }
        let (area_x, area_y, area_w, area_h) = self.text_area(&frame, layout);
        if area_w <= 0 || area_h <= 0 {
            return frame;
        }
        let speaker = speaker.filter(|name| !name.is_empty());
        let body_px = layout.font_px();
        let body_line = self.line_height(body_px) + LINE_SPACING;
        let mut baseline = area_y + self.ascent(body_px).ceil() as i32;
        if let Some(name) = speaker {
            let name_px = (body_px * NAME_SCALE).max(10.0);
            baseline = area_y + self.ascent(name_px).ceil() as i32;
            self.draw_line(
                &mut frame,
                name,
                area_x,
                baseline,
                area_w,
                name_px,
                layout.name_color(),
            );
            baseline += body_line.max(self.line_height(name_px) + LINE_SPACING) + NAME_GAP;
        }
        let bottom = area_y + area_h;
        for line in body.split('\n') {
            for chunk in self.wrap_line(line, body_px, area_w) {
                let glyph_top = baseline - self.ascent(body_px).ceil() as i32;
                if glyph_top + self.line_height(body_px) > bottom {
                    return frame;
                }
                self.draw_line(
                    &mut frame,
                    &chunk,
                    area_x,
                    baseline,
                    area_w,
                    body_px,
                    layout.body_color(),
                );
                baseline += body_line;
            }
        }
        // Click-to-continue marker in the bottom-right corner of the window.
        let marker_x = area_x + area_w - 10;
        let marker_y = area_y + area_h - 10;
        for y in marker_y.max(0)..(marker_y + 6).min(frame.height() as i32) {
            for x in marker_x.max(0)..(marker_x + 6).min(frame.width() as i32) {
                frame.put_pixel(x as u32, y as u32, Rgba([245, 245, 245, 255]));
            }
        }
        frame
    }

    /// Text area in screen pixels: the scripted window rectangle, clamped to
    /// the screen and to the window graphic when one is present.
    fn text_area(&self, frame: &RgbaImage, layout: &TextWindowLayout) -> (i32, i32, i32, i32) {
        let screen_w = frame.width() as i32;
        let screen_h = frame.height() as i32;
        let x = layout.x;
        let y = layout.y;
        // The message-box graphic carries the real box size; the scripted
        // rectangle is only reliable for its origin.
        let graphic = layout
            .frame
            .as_ref()
            .map(|graphic| (graphic.width() as i32, graphic.height() as i32));
        let width = match graphic {
            Some((graphic_w, _)) => layout.width.max(graphic_w),
            None => layout.width,
        }
        .clamp(0, screen_w);
        let height = if layout.height > 0 {
            layout.height
        } else {
            // A zero scripted height means the window grows to fit the page.
            (screen_h - y).max(0)
        };
        let right = (x + width).min(screen_w);
        let bottom = (y + height).min(screen_h);
        (
            x + PADDING,
            y + PADDING,
            (right - x - PADDING * 2).max(0),
            (bottom - y - PADDING).max(0),
        )
    }

    fn blit_frame(&self, frame: &mut RgbaImage, graphic: &RgbaImage, layout: &TextWindowLayout) {
        let has_alpha = graphic.pixels().any(|pixel| pixel[3] < 255);
        for y in 0..graphic.height() as i32 {
            for x in 0..graphic.width() as i32 {
                let dx = layout.x + x;
                let dy = layout.y + y;
                if dx < 0 || dy < 0 || dx >= frame.width() as i32 || dy >= frame.height() as i32 {
                    continue;
                }
                let pixel = *graphic.get_pixel(x as u32, y as u32);
                if has_alpha && pixel[3] < 255 {
                    let dst = frame.get_pixel_mut(dx as u32, dy as u32);
                    let alpha = u16::from(pixel[3]);
                    for channel in 0..3 {
                        dst[channel] = ((u16::from(pixel[channel]) * alpha
                            + u16::from(dst[channel]) * (255 - alpha))
                            / 255) as u8;
                    }
                    dst[3] = 255;
                } else if has_alpha && pixel[3] == 0 {
                    continue;
                } else {
                    frame.put_pixel(dx as u32, dy as u32, pixel);
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_line(
        &self,
        frame: &mut RgbaImage,
        line: &str,
        x: i32,
        baseline: i32,
        max_width: i32,
        px: f32,
        color: [u8; 3],
    ) {
        let mut cursor = x;
        for ch in line.chars() {
            let (metrics, bitmap) = self.font.rasterize(ch, px);
            let advance = metrics.advance_width.ceil().max(0.0) as i32;
            if cursor + advance > x + max_width {
                break;
            }
            let glyph_top = baseline - metrics.ymin - metrics.height as i32;
            for gy in 0..metrics.height {
                for gx in 0..metrics.width {
                    let dx = cursor + metrics.xmin + gx as i32;
                    let dy = glyph_top + gy as i32;
                    if dx < 0 || dy < 0 || dx >= frame.width() as i32 || dy >= frame.height() as i32 {
                        continue;
                    }
                    let alpha = u16::from(bitmap[gy * metrics.width + gx]);
                    if alpha == 0 {
                        continue;
                    }
                    let target = frame.get_pixel_mut(dx as u32, dy as u32);
                    for channel in 0..3 {
                        target[channel] = ((u16::from(target[channel]) * (255 - alpha)
                            + u16::from(color[channel]) * alpha)
                            / 255) as u8;
                    }
                    target[3] = 255;
                }
            }
            cursor += advance.max(1);
        }
    }

    fn wrap_line(&self, line: &str, px: f32, max_width: i32) -> Vec<String> {
        let mut out = Vec::new();
        let mut current = String::new();
        let mut used = 0i32;
        for ch in line.chars() {
            let advance = self
                .font
                .rasterize(ch, px)
                .0
                .advance_width
                .ceil()
                .max(1.0) as i32;
            if used + advance > max_width && !current.is_empty() {
                out.push(std::mem::take(&mut current));
                used = 0;
            }
            current.push(ch);
            used += advance;
        }
        out.push(current);
        out
    }

    fn ascent(&self, px: f32) -> f32 {
        self.font
            .horizontal_line_metrics(px)
            .map_or(px * 1.05, |metrics| metrics.ascent)
    }

    fn line_height(&self, px: f32) -> i32 {
        self.font
            .horizontal_line_metrics(px)
            .map_or((px * 1.35).ceil() as i32, |metrics| {
                (metrics.ascent - metrics.descent + metrics.line_gap)
                    .ceil()
                    .max(px)
                    as i32
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn renderer() -> Option<TextRenderer> {
        TextRenderer::new().ok()
    }

    fn layout(frame: Option<RgbaImage>, height: i32) -> TextWindowLayout {
        TextWindowLayout {
            x: 44,
            y: 320,
            width: 512,
            height,
            font_size: 20,
            frame,
            colors: HashMap::from([(0, [255, 240, 220]), (1, [8, 12, 72])]),
        }
    }

    #[test]
    fn page_renders_inside_the_window_frame() {
        let Some(renderer) = renderer() else {
            return;
        };
        let background = RgbaImage::from_pixel(640, 480, Rgba([0, 0, 0, 255]));
        let graphic = RgbaImage::from_pixel(552, 152, Rgba([64, 64, 64, 255]));
        let frame = renderer.draw_page(
            &background,
            Some("浩一"),
            "どうも騙されたようだ。",
            &layout(Some(graphic), 0),
        );
        // Outside the window stays background.
        assert_eq!(frame.get_pixel(10, 10).0, [0, 0, 0, 255]);
        // The window graphic is copied at the scripted position.
        assert_eq!(frame.get_pixel(44, 320).0, [64, 64, 64, 255]);
        assert_eq!(frame.get_pixel(44 + 551, 320 + 151).0, [64, 64, 64, 255]);
        // Text pixels differ from the flat graphic.
        let changed = (320..472)
            .flat_map(|y| (44..596).map(move |x| (x, y)))
            .filter(|&(x, y)| frame.get_pixel(x as u32, y as u32).0 != [64, 64, 64, 255])
            .count();
        assert!(changed > 100, "expected glyphs inside the window, got {changed}");
    }

    #[test]
    fn page_without_graphic_leaves_the_background_untouched() {
        let Some(renderer) = renderer() else {
            return;
        };
        let background = RgbaImage::from_pixel(640, 480, Rgba([70, 70, 70, 255]));
        let frame = renderer.draw_page(&background, None, "本文だけ", &layout(None, 152));
        assert_eq!(frame.get_pixel(0, 0).0, [70, 70, 70, 255]);
        let body = layout(None, 152).body_color();
        let changed = (320..472)
            .flat_map(|y| (44..596).map(move |x| (x, y)))
            .filter(|&(x, y)| frame.get_pixel(x as u32, y as u32).0 != [70, 70, 70, 255])
            .count();
        assert!(changed > 50, "expected body glyphs, got {changed} (color {body:?})");
    }

    #[test]
    fn long_text_wraps_and_stops_at_the_window_bottom() {
        let Some(renderer) = renderer() else {
            return;
        };
        let background = RgbaImage::from_pixel(640, 480, Rgba([0, 0, 0, 255]));
        let graphic = RgbaImage::from_pixel(552, 152, Rgba([64, 64, 64, 0]));
        let long = "あ".repeat(400);
        let frame = renderer.draw_page(&background, None, &long, &layout(Some(graphic), 136));
        // The page overflows, so the last rows below the scripted box stay clear.
        let rows_below = (470..480)
            .flat_map(|y| (44..596).map(move |x| (x, y)))
            .filter(|&(x, y)| frame.get_pixel(x as u32, y as u32).0 != [0, 0, 0, 255])
            .count();
        assert_eq!(rows_below, 0, "text leaked outside the window box");
    }

    #[test]
    fn name_uses_the_secondary_palette_color() {
        let Some(renderer) = renderer() else {
            return;
        };
        let background = RgbaImage::from_pixel(640, 480, Rgba([0, 0, 0, 255]));
        let graphic = RgbaImage::from_pixel(552, 152, Rgba([64, 64, 64, 255]));
        let frame = renderer.draw_page(&background, Some("浩一"), "本文", &layout(Some(graphic), 0));
        let name_color = layout(None, 0).name_color();
        let matched = (320..472)
            .flat_map(|y| (44..300).map(move |x| (x, y)))
            .any(|(x, y)| {
                let pixel = frame.get_pixel(x as u32, y as u32).0;
                pixel[..3] != [64, 64, 64] && pixel[2] > pixel[0]
            });
        assert!(matched, "expected a blue-tinted speaker name ({name_color:?})");
    }
}
