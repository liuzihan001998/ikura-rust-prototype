use crate::vm::BlitOp;
use image::{DynamicImage, Rgba, RgbaImage};
use std::collections::HashMap;
use std::path::PathBuf;

struct Surface {
    pixels: RgbaImage,
    has_alpha: bool,
}

pub struct Graphics {
    root: PathBuf,
    surfaces: HashMap<i32, Surface>,
}

impl Graphics {
    pub fn new(root: PathBuf, width: u32, height: u32) -> Self {
        let mut surfaces = HashMap::new();
        surfaces.insert(
            0,
            Surface {
                pixels: RgbaImage::from_pixel(width, height, Rgba([0, 0, 0, 255])),
                has_alpha: false,
            },
        );
        Self { root, surfaces }
    }

    pub fn allocate(&mut self, slot: i32, width: i32, height: i32) -> Result<(), String> {
        if width <= 0 || height <= 0 || width > 4096 || height > 4096 {
            return Err(format!(
                "invalid VSET dimensions for slot {slot}: {width}x{height}"
            ));
        }
        self.surfaces.insert(
            slot,
            Surface {
                pixels: RgbaImage::from_pixel(width as u32, height as u32, Rgba([0, 0, 0, 255])),
                has_alpha: false,
            },
        );
        Ok(())
    }

    pub fn set_bank(&mut self, bank: &str) -> Result<(), String> {
        if !matches!(bank, "GGD" | "GGD2") {
            return Err(format!("unsupported graphic bank {bank}"));
        }
        let parent = self.root.parent().ok_or("graphic root has no parent")?;
        self.root = parent.join(bank);
        Ok(())
    }

    pub fn load(&mut self, slot: i32, name: &str) -> Result<(u32, u32), String> {
        let file = if name.rsplit_once('.').is_some_and(|(_, extension)| {
            extension.eq_ignore_ascii_case("GG0") || extension.eq_ignore_ascii_case("GG2")
        }) {
            format!("{name}.png")
        } else {
            format!("{name}.GGD.png")
        };
        let path = self.root.join(file);
        let image = image::open(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        let has_alpha = image.color().has_alpha();
        let pixels = DynamicImage::to_rgba8(&image);
        let dimensions = pixels.dimensions();
        self.surfaces.insert(slot, Surface { pixels, has_alpha });
        Ok(dimensions)
    }

    pub fn fill_screen(&mut self, color: [u8; 3]) {
        self.fill_surface(0, color).expect("screen surface exists");
    }

    pub fn fill_surface(&mut self, slot: i32, color: [u8; 3]) -> Result<(), String> {
        if let Some(surface) = self.surfaces.get_mut(&slot) {
            for pixel in surface.pixels.pixels_mut() {
                *pixel = Rgba([color[0], color[1], color[2], 255]);
            }
            Ok(())
        } else {
            Err(format!("GC destination slot {slot} is empty"))
        }
    }

    pub fn screen(&self) -> &RgbaImage {
        &self.surfaces.get(&0).expect("screen surface exists").pixels
    }

    pub fn show_surface(&mut self, slot: i32) -> Result<(), String> {
        let source = self
            .surfaces
            .get(&slot)
            .ok_or_else(|| format!("effect source slot {slot} is empty"))?;
        let pixels = source.pixels.clone();
        let target = self.surfaces.get_mut(&0).expect("screen surface exists");
        for y in 0..target.pixels.height().min(pixels.height()) {
            for x in 0..target.pixels.width().min(pixels.width()) {
                target.pixels.put_pixel(x, y, *pixels.get_pixel(x, y));
            }
        }
        Ok(())
    }

    pub fn blit(&mut self, op: &BlitOp) -> Result<(), String> {
        let (destination, transparent) = match op.command {
            0 | 20 => (op.destination, false),
            1 | 21 => (op.destination, true),
            // The destination field is a duration for transition commands.
            7 | 19 => (0, true),
            15..=17 => (0, false),
            command => return Err(format!("GP command {command} is not implemented")),
        };
        let source = self
            .surfaces
            .get(&op.source)
            .ok_or_else(|| format!("GP source slot {} is empty", op.source))?;
        let source_pixels = source.pixels.clone();
        let source_has_alpha = source.has_alpha;
        let target = self
            .surfaces
            .get_mut(&destination)
            .ok_or_else(|| format!("GP destination slot {destination} is empty"))?;
        if op.width <= 0 || op.height <= 0 {
            return Ok(());
        }
        for y in 0..op.height {
            for x in 0..op.width {
                let sx = op.source_x + x;
                let sy = op.source_y + y;
                let dx = op.destination_x + x;
                let dy = op.destination_y + y;
                if sx < 0 || sy < 0 || dx < 0 || dy < 0 {
                    continue;
                }
                let (sx, sy, dx, dy) = (sx as u32, sy as u32, dx as u32, dy as u32);
                if sx >= source_pixels.width()
                    || sy >= source_pixels.height()
                    || dx >= target.pixels.width()
                    || dy >= target.pixels.height()
                {
                    continue;
                }
                let pixel = *source_pixels.get_pixel(sx, sy);
                if transparent && !source_has_alpha && pixel.0[..3] == [0, 0, 0] {
                    continue;
                }
                if transparent && source_has_alpha && pixel[3] < 255 {
                    let dst = target.pixels.get_pixel_mut(dx, dy);
                    let alpha = u16::from(pixel[3]);
                    for channel in 0..3 {
                        dst[channel] = ((u16::from(pixel[channel]) * alpha
                            + u16::from(dst[channel]) * (255 - alpha))
                            / 255) as u8;
                    }
                    dst[3] = 255;
                } else {
                    target.pixels.put_pixel(dx, dy, pixel);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_blit_copies_and_clips() {
        let mut graphics = Graphics::new(PathBuf::new(), 2, 2);
        graphics.surfaces.insert(
            1,
            Surface {
                pixels: RgbaImage::from_pixel(2, 2, Rgba([10, 20, 30, 255])),
                has_alpha: false,
            },
        );
        graphics
            .blit(&BlitOp {
                command: 0,
                source: 1,
                source_x: 0,
                source_y: 0,
                width: 3,
                height: 3,
                destination: 0,
                destination_x: 1,
                destination_y: 1,
            })
            .unwrap();
        assert_eq!(graphics.screen().get_pixel(1, 1).0, [10, 20, 30, 255]);
        assert_eq!(graphics.screen().get_pixel(0, 0).0, [0, 0, 0, 255]);
    }

    #[test]
    fn transparent_blit_skips_black_for_rgb_source() {
        let mut graphics = Graphics::new(PathBuf::new(), 1, 1);
        graphics.fill_screen([200, 100, 50]);
        graphics.surfaces.insert(
            1,
            Surface {
                pixels: RgbaImage::from_pixel(1, 1, Rgba([0, 0, 0, 255])),
                has_alpha: false,
            },
        );
        graphics
            .blit(&BlitOp {
                command: 1,
                source: 1,
                source_x: 0,
                source_y: 0,
                width: 1,
                height: 1,
                destination: 0,
                destination_x: 0,
                destination_y: 0,
            })
            .unwrap();
        assert_eq!(graphics.screen().get_pixel(0, 0).0, [200, 100, 50, 255]);
    }
}
