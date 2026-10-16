//! Story text composition. Decoded PM pages are drawn into a frame image that
//! mirrors the original text window: `KA_M.GG2` for the message box and the
//! `SETFONTCOLOR` palette for speaker names and body text.

use crate::vm::ChoiceOption;
use fontdue::{Font, FontSettings};
use image::{Rgba, RgbaImage};
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const PADDING: i32 = 12;
const LINE_SPACING: i32 = 2;
const NAME_GAP: i32 = 6;
const NAME_SCALE: f32 = 0.85;

fn localize_dynamic_label(text: &str) -> Cow<'_, str> {
    for (prefix, translated) in [
        ("SLOT ", "档位 "),
        ("Slot ", "档位 "),
        ("Quick: ", "快速存档："),
        ("Saved: ", "已保存："),
        ("Loaded: ", "已读取："),
        ("Deleted: ", "已删除："),
        ("Delete failed: ", "删除失败："),
        ("Voice replay failed: ", "语音重播失败："),
        ("Settings could not be saved: ", "设置保存失败："),
        ("Could not apply: ", "设置应用失败："),
    ] {
        if let Some(rest) = text.strip_prefix(prefix) {
            return Cow::Owned(format!("{translated}{}", localize_dynamic_label(rest)));
        }
    }
    for (english, translated) in [
        ("Quick save", "快速存档"),
        ("Automatic save", "自动存档"),
        ("Saving...", "正在保存…"),
        ("Loading...", "正在读取…"),
        (
            "Save failed. Check storage space and folder permissions.",
            "保存失败，请检查磁盘空间和文件夹权限。",
        ),
        (" (audio unavailable)", "（音频不可用）"),
        (" (backup recovered)", "（已从备份恢复）"),
    ] {
        if text == english {
            return Cow::Borrowed(translated);
        }
    }
    for (suffix, translated) in [
        (" (audio unavailable)", "（音频不可用）"),
        (" (backup recovered)", "（已从备份恢复）"),
    ] {
        if let Some(rest) = text.strip_suffix(suffix) {
            return Cow::Owned(format!("{}{translated}", localize_dynamic_label(rest)));
        }
    }
    if let Some(rest) = text.strip_suffix("% of script delay") {
        return Cow::Owned(format!("{}% 剧本速度", rest.trim_end_matches('%')));
    }
    if let Some(rest) = text.strip_suffix(" sec + reading time") {
        return Cow::Owned(format!("{rest} 秒 + 阅读时间"));
    }
    if let Some((action, rest)) = text.split_once(' ')
        && let Some(slot) = rest.strip_suffix('?')
        && matches!(action, "Delete" | "Overwrite" | "Load")
    {
        let action = match action {
            "Delete" => "删除",
            "Overwrite" => "覆盖",
            _ => "读取",
        };
        return Cow::Owned(format!("{action} {}？", localize_dynamic_label(slot)));
    }
    Cow::Borrowed(text)
}

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
    /// F8 bits as used by this game: 0x01 bold and 0x04 black shadow.
    pub font_style: u8,
    /// Optional original window graphic drawn behind the text.
    pub frame: Option<RgbaImage>,
    /// `SETFONTCOLOR` palette keyed by the font byte from the script.
    pub colors: HashMap<u8, [u8; 3]>,
}

impl TextWindowLayout {
    fn body_color(&self) -> [u8; 3] {
        self.colors.get(&0).copied().unwrap_or([255, 255, 255])
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

fn chinese_mode() -> bool {
    cfg!(target_os = "android")
        || matches!(
            std::env::var("IKURA_LANG").as_deref(),
            Ok("zh") | Ok("zh-CN")
        )
}

impl TextRenderer {
    /// A clipped, single-line label for menus and notifications.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_label(
        &self,
        frame: &mut RgbaImage,
        text: &str,
        x: i32,
        y: i32,
        width: i32,
        px: f32,
        color: [u8; 3],
    ) {
        let text = if chinese_mode() {
            match text {
                "SETTINGS" => "设置",
                "Music volume" => "音乐音量",
                "Sound volume" => "音效音量",
                "Voice volume" => "语音音量",
                "Text delay" => "文字速度",
                "Auto wait" => "自动播放等待",
                "Automatic saves" => "自动存档",
                "Fullscreen" => "全屏",
                "Instant" => "立即显示",
                "OFF" => "关闭",
                "ON" => "开启",
                "Back" => "返回",
                "Reset defaults" => "恢复默认",
                "F10 / Esc: back     Arrow keys: adjust" => "F10 / Esc：返回    方向键：调整",
                "SAVE GAME" => "保存游戏",
                "LOAD GAME" => "读取游戏",
                "Auto" => "自动",
                "Quick" => "快速",
                "Save" => "保存",
                "Load" => "读取",
                "Delete" => "删除",
                "Backup" => "备份",
                "DAMAGED" => "已损坏",
                "EMPTY" => "空档位",
                "Confirm" => "确认",
                "Cancel" => "取消",
                "Overwrite" => "覆盖",
                "HISTORY - click a voiced line to replay" => "历史记录 · 点击有语音的台词重播",
                "No dialogue history yet." => "暂无对话记录。",
                "Wheel / drag: scroll    Home / End    B / Esc: back" => {
                    "滚轮 / 拖动：滚动    Home / End    B / Esc：返回"
                }
                "Arrows / Enter    PgUp/PgDn    Right click: delete" => {
                    "方向键 / 回车    PgUp/PgDn    右键：删除"
                }
                "Settings F10" => "设置 F10",
                "Backup only" => "仅有备份",
                "Different game data" => "游戏数据不匹配",
                "Damaged save" => "存档已损坏",
                "This slot is empty." => "此档位为空。",
                "Saving is disabled at this point." => "当前位置无法存档。",
                "Quick save: damaged (check backup)" => "快速存档已损坏（请检查备份）",
                "Quick save: empty" => "快速存档为空",
                "Save and its recovery backup will be deleted." => "存档和恢复备份将被删除。",
                "Existing data will be replaced." => "原有存档将被覆盖。",
                "Unsaved progress will be lost." => "未保存的进度将丢失。",
                "Settings saved" => "设置已保存",
                _ => text,
            }
        } else {
            text
        };
        let text = if chinese_mode() {
            localize_dynamic_label(text)
        } else {
            Cow::Borrowed(text)
        };
        self.draw_line(
            frame,
            &text,
            x,
            y + self.ascent(px).ceil() as i32,
            width,
            px,
            color,
        );
    }
    pub fn new() -> Result<Self, String> {
        let candidates: Vec<PathBuf> = if let Some(path) = std::env::var_os("IKURA_FONT") {
            vec![PathBuf::from(path)]
        } else if chinese_mode() {
            [
                "/System/Library/Fonts/Hiragino Sans GB.ttc",
                "/System/Library/Fonts/STHeiti Medium.ttc",
                "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
                "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
                "/system/fonts/NotoSansCJK-Regular.ttc",
                "C:/Windows/Fonts/meiryo.ttc",
                "C:/Windows/Fonts/YuGothR.ttc",
                "C:/Windows/Fonts/msgothic.ttc",
                "C:/Windows/Fonts/msmincho.ttc",
            ]
            .iter()
            .map(PathBuf::from)
            .collect()
        } else {
            [
                "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
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
        self.draw_page_revealed(background, speaker, body, layout, usize::MAX, true)
    }

    /// Lay out the full page before revealing glyphs so font fitting and line
    /// wrapping do not change as the visible prefix grows. Newlines do not
    /// consume reveal time. Speaker names appear immediately.
    pub fn draw_page_revealed(
        &self,
        background: &RgbaImage,
        speaker: Option<&str>,
        body: &str,
        layout: &TextWindowLayout,
        visible_chars: usize,
        show_marker: bool,
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
        let body_px = self.fit_font_size(body, speaker.is_some(), area_w, area_h, layout.font_px());
        let body_line = self.line_height(body_px) + LINE_SPACING;
        let mut baseline = area_y + self.ascent(body_px).ceil() as i32;
        if let Some(name) = speaker {
            let name_px = (body_px * NAME_SCALE).max(10.0);
            baseline = area_y + self.ascent(name_px).ceil() as i32;
            self.draw_styled_line(
                &mut frame,
                name,
                area_x,
                baseline,
                area_w,
                name_px,
                layout.body_color(),
                layout.font_style,
            );
            baseline += body_line.max(self.line_height(name_px) + LINE_SPACING) + NAME_GAP;
        }
        let bottom = area_y + area_h;
        let mut remaining = visible_chars;
        'body: for line in body.split('\n') {
            for chunk in self.wrap_line(line, body_px, area_w) {
                let glyph_top = baseline - self.ascent(body_px).ceil() as i32;
                if glyph_top + self.line_height(body_px) > bottom {
                    break 'body;
                }
                let count = chunk.chars().count();
                let visible: String = chunk.chars().take(remaining).collect();
                self.draw_styled_line(
                    &mut frame,
                    &visible,
                    area_x,
                    baseline,
                    area_w,
                    body_px,
                    layout.body_color(),
                    layout.font_style,
                );
                remaining = remaining.saturating_sub(count);
                baseline += body_line;
            }
        }
        if !show_marker || visible_chars < body.chars().filter(|&c| c != '\n').count() {
            return frame;
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

    /// Simultaneous dialogue can contain up to five speakers. Keep the script's
    /// requested size when it fits, and reduce only overflowing pages so the
    /// final speakers are not silently clipped from the message window.
    fn fit_font_size(
        &self,
        body: &str,
        speaker: bool,
        width: i32,
        height: i32,
        requested: f32,
    ) -> f32 {
        let minimum = requested.min(12.0);
        let mut px = requested;
        loop {
            let lines: usize = body
                .split('\n')
                .map(|line| self.wrap_line(line, px, width).len())
                .sum();
            let line_height = self.line_height(px) + LINE_SPACING;
            let header = if speaker { line_height + NAME_GAP } else { 0 };
            if lines as i32 * line_height + header <= height || px <= minimum {
                return px;
            }
            px = (px - 1.0).max(minimum);
        }
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

    /// Compose the choice overlay over `background`. Mouse hover lightens the
    /// row under the cursor; both rows use the scripted `CSET` rectangles.
    pub fn draw_choices(
        &self,
        background: &RgbaImage,
        choices: &[ChoiceOption],
        hover: Option<u8>,
    ) -> RgbaImage {
        let mut frame = background.clone();
        let px = 20.0;
        for choice in choices {
            if choice.width <= 0 || choice.height <= 0 {
                continue;
            }
            let hovered = hover == Some(choice.value);
            for y in choice.y.max(0)..(choice.y + choice.height).min(frame.height() as i32) {
                for x in choice.x.max(0)..(choice.x + choice.width).min(frame.width() as i32) {
                    let pixel = frame.get_pixel_mut(x as u32, y as u32);
                    for channel in 0..3 {
                        pixel[channel] = if hovered { 96 } else { 40 };
                    }
                    pixel[3] = 255;
                }
            }
            let baseline = choice.y + self.ascent(px).ceil() as i32;
            self.draw_line(
                &mut frame,
                &choice.caption,
                choice.x + 6,
                baseline,
                choice.width - 12,
                px,
                if hovered {
                    [255, 236, 140]
                } else {
                    [235, 235, 235]
                },
            );
        }
        frame
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_styled_line(
        &self,
        frame: &mut RgbaImage,
        line: &str,
        x: i32,
        baseline: i32,
        max_width: i32,
        px: f32,
        color: [u8; 3],
        style: u8,
    ) {
        // Match the reference decoder: bits 0/1 select bold and also enable
        // its mask shadow; bit 2 enables the same black (2, 2) shadow alone.
        // The game's actual style bytes are 4 (shadow) and 5 (bold + shadow).
        let bold = style & 0x03 != 0;
        if style & 0x07 != 0 {
            self.draw_line(
                frame,
                line,
                x + 2,
                baseline + 2,
                max_width - 2,
                px,
                [0, 0, 0],
            );
            if bold {
                self.draw_line(
                    frame,
                    line,
                    x + 3,
                    baseline + 2,
                    max_width - 3,
                    px,
                    [0, 0, 0],
                );
            }
        }
        self.draw_line(frame, line, x, baseline, max_width, px, color);
        if bold {
            // fontdue has no style switch: synthesize bold with a one-pixel
            // horizontal stroke while keeping Japanese character advances fixed.
            self.draw_line(frame, line, x + 1, baseline, max_width - 1, px, color);
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
                    if dx < 0 || dy < 0 || dx >= frame.width() as i32 || dy >= frame.height() as i32
                    {
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

    pub(crate) fn wrap_line(&self, line: &str, px: f32, max_width: i32) -> Vec<String> {
        let mut out = Vec::new();
        let mut current = String::new();
        let mut used = 0i32;
        for ch in line.chars() {
            let advance = self.font.rasterize(ch, px).0.advance_width.ceil().max(1.0) as i32;
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
                    .max(px) as i32
            })
    }
}

#[cfg(test)]
mod font_probe {
    use super::*;

    /// Prints per-glyph rasterisation stats for the rare characters in the
    /// script, to tell a missing glyph apart from a rendering bug.
    #[test]
    #[ignore = "diagnostic only"]
    fn report_glyph_coverage() {
        let renderer = TextRenderer::new().expect("font");
        for ch in ['騙', '謎', '追', '憶', '司', '。', '、', '「'] {
            let (metrics, bitmap) = renderer.font.rasterize(ch, 22.0);
            let filled = bitmap.iter().filter(|&&value| value > 0).count();
            println!(
                "{ch}: advance={:.1} w={} h={} filled={}",
                metrics.advance_width, metrics.width, metrics.height, filled
            );
        }
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
            font_style: 0,
            frame,
            colors: HashMap::from([(0, [255, 240, 220]), (1, [8, 12, 72])]),
        }
    }

    #[test]
    fn font_style_adds_shadow_and_bold_without_revealing_future_characters() {
        let Some(renderer) = renderer() else {
            return;
        };
        let background = RgbaImage::from_pixel(640, 480, Rgba([160, 160, 160, 255]));
        let mut layout = layout(None, 152);
        let body = "春花の声が聞こえる。";
        let plain =
            renderer.draw_page_revealed(&background, None, body, &layout, usize::MAX, false);
        layout.font_style = 4;
        let shadow =
            renderer.draw_page_revealed(&background, None, body, &layout, usize::MAX, false);
        assert!(shadow != plain);
        assert!(
            shadow.pixels().any(|pixel| pixel[0] < 100),
            "black shadow over gray background"
        );
        layout.font_style = 5;
        let bold = renderer.draw_page_revealed(&background, None, body, &layout, usize::MAX, false);
        assert!(bold != shadow);
        let empty = renderer.draw_page_revealed(&background, None, body, &layout, 0, false);
        assert!(empty == background, "hidden glyphs must not cast shadows");
        layout.font_style = 0;
        let reset =
            renderer.draw_page_revealed(&background, None, body, &layout, usize::MAX, false);
        assert!(reset == plain);
    }

    #[test]
    fn reveal_keeps_full_page_layout_and_hides_marker_until_complete() {
        let Some(renderer) = renderer() else {
            return;
        };
        let background = RgbaImage::from_pixel(640, 480, Rgba([0, 0, 0, 255]));
        let layout = layout(None, 152);
        let body = "どうも騙されたようだ。\n春花の声が聞こえる。".repeat(8);
        let empty = renderer.draw_page_revealed(&background, None, &body, &layout, 0, true);
        assert!(
            empty == background,
            "no glyphs or marker before reveal begins"
        );
        let partial = renderer.draw_page_revealed(&background, None, &body, &layout, 8, true);
        let full =
            renderer.draw_page_revealed(&background, None, &body, &layout, usize::MAX, false);
        assert!(partial != empty);
        assert!(partial != full);
        for ((shown, complete), untouched) in
            partial.pixels().zip(full.pixels()).zip(background.pixels())
        {
            if shown != untouched {
                assert_eq!(shown, complete, "revealed glyph moved or changed size");
            }
        }
        let finished =
            renderer.draw_page_revealed(&background, None, &body, &layout, usize::MAX, true);
        assert!(finished != full, "finished page has a continue marker");
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
        assert!(
            changed > 100,
            "expected glyphs inside the window, got {changed}"
        );
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
        assert!(
            changed > 50,
            "expected body glyphs, got {changed} (color {body:?})"
        );
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
    fn five_speaker_page_fits_before_drawing_the_continue_marker() {
        let Some(renderer) = renderer() else {
            return;
        };
        let background = RgbaImage::from_pixel(640, 480, Rgba([0, 0, 0, 255]));
        let mut window = layout(None, 136);
        window.font_size = 30;
        let page = "一人目 おかえり\n二人目 おかえりなさい\n三人目 おかえりなさーい\n四人目 おかえり\n五人目 おかえりなさい";
        let frame = renderer.draw_page(&background, None, page, &window);
        let (x, y, width, height) = renderer.text_area(&frame, &window);
        // The marker is reached only if every wrapped line was rendered.
        assert_eq!(
            frame
                .get_pixel((x + width - 10) as u32, (y + height - 10) as u32)
                .0,
            [245, 245, 245, 255]
        );
    }

    /// The message box graphic is translucent, so composed text must never be
    /// baked back into the scene: the next page would then draw its own text
    /// over the previous page's and show through as ghosting.
    #[test]
    fn composing_a_page_never_mutates_the_scene() {
        let Some(renderer) = renderer() else {
            return;
        };
        let scene = RgbaImage::from_pixel(640, 480, Rgba([30, 40, 50, 255]));
        let graphic = RgbaImage::from_pixel(480, 120, Rgba([60, 60, 60, 180]));
        let mut layout = layout(Some(graphic), 120);
        layout.x = 40;
        layout.y = 340;
        layout.width = 480;
        let page_one = renderer.draw_page(&scene, None, "一つ目の文章です。", &layout);
        let page_two = renderer.draw_page(&scene, None, "二つ目の文章です。", &layout);
        // The scene stays scene-only, so each page composites onto clean pixels.
        assert_eq!(
            scene.get_pixel(300, 400).0,
            [30, 40, 50, 255],
            "page composition must not write into the scene surface"
        );
        // Each page keeps its own glyphs.
        assert!(renderer_has_text(&page_one, &scene, 40, 340, 480, 120));
        assert!(renderer_has_text(&page_two, &scene, 40, 340, 480, 120));
        assert_ne!(
            page_one.as_raw(),
            page_two.as_raw(),
            "the two pages must not render identically"
        );
    }

    /// True when some pixel inside `area` differs from the graphic fill, which
    /// only happens where glyphs were drawn.
    fn renderer_has_text(
        frame: &RgbaImage,
        scene: &RgbaImage,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) -> bool {
        let graphic = [60u8, 60, 60];
        let scene_only = scene.get_pixel(0, 0).0;
        (y..y + height).any(|py| {
            (x..x + width).any(|px| {
                let pixel = frame.get_pixel(px as u32, py as u32).0;
                pixel[..3] != graphic && pixel != scene_only
            })
        })
    }

    #[test]
    fn choice_overlay_fills_the_scripted_rectangles_only() {
        let Some(renderer) = renderer() else {
            return;
        };
        let background = RgbaImage::from_pixel(640, 480, Rgba([10, 20, 30, 255]));
        let choices = vec![
            ChoiceOption {
                value: 0,
                caption: "我慢する".into(),
                x: 16,
                y: 16,
                width: 520,
                height: 20,
            },
            ChoiceOption {
                value: 1,
                caption: "振り返る".into(),
                x: 16,
                y: 40,
                width: 520,
                height: 20,
            },
        ];
        let frame = renderer.draw_choices(&background, &choices, Some(1));
        assert_eq!(frame.get_pixel(300, 300).0, [10, 20, 30, 255]);
        assert_eq!(frame.get_pixel(20, 18).0, [40, 40, 40, 255]);
        assert_eq!(frame.get_pixel(20, 42).0, [96, 96, 96, 255]);
        // Captions leave brighter pixels than the row fill.
        let caption_pixels = (16..120)
            .flat_map(|x| (16..60).map(move |y| (x, y)))
            .filter(|&(x, y)| frame.get_pixel(x, y)[0] > 150)
            .count();
        assert!(
            caption_pixels > 40,
            "expected captions, got {caption_pixels}"
        );
    }

    #[test]
    fn speaker_inherits_text_color_instead_of_unrelated_palette_entries() {
        let Some(renderer) = renderer() else {
            return;
        };
        // START defines white text plus dark auxiliary palette entries. A PM
        // speaker command prints through the same text printer, so index 1
        // must not turn the speaker's foreground dark blue.
        let background = RgbaImage::from_pixel(640, 480, Rgba([24, 24, 24, 255]));
        let mut configured = layout(None, 152);
        configured.colors.insert(0, [255, 255, 255]);
        configured.font_style = 5;
        let actual = renderer.draw_page_revealed(
            &background,
            Some("司"),
            "「……ったく」",
            &configured,
            0,
            false,
        );
        let mut alternate = configured;
        alternate.colors.insert(1, [255, 0, 0]);
        alternate.colors.insert(2, [0, 255, 0]);
        let comparison = renderer.draw_page_revealed(
            &background,
            Some("司"),
            "「……ったく」",
            &alternate,
            0,
            false,
        );
        assert_eq!(
            actual, comparison,
            "auxiliary palette entries changed the speaker foreground"
        );
        assert!(
            actual.enumerate_pixels().any(|(x, y, p)| x >= 44
                && (320..360).contains(&y)
                && p[0] > 220
                && p[1] > 220
                && p[2] > 220),
            "speaker must be legible in the script's white text color even before body reveal"
        );
        alternate.colors.insert(0, [255, 210, 120]);
        let tinted = renderer.draw_page_revealed(&background, Some("司"), "", &alternate, 0, false);
        assert!(
            tinted
                .pixels()
                .any(|p| p[0] > 220 && p[1] > 180 && p[1] < 220 && p[2] < 150),
            "speaker must follow a changed primary text color, not a hard-coded white"
        );
    }
}
