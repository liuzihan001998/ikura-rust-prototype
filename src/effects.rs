use crate::assets::DataArchive;
use image::GrayImage;
use std::{collections::HashMap, path::Path};

pub struct EffectMasks {
    archive: Option<DataArchive>,
    cache: HashMap<String, GrayImage>,
    warned_missing: bool,
}

impl EffectMasks {
    pub fn new(data_dir: &Path) -> Result<Self, String> {
        Ok(Self {
            archive: DataArchive::load(data_dir, None)?,
            cache: HashMap::new(),
            warned_missing: false,
        })
    }
    pub fn get(&mut self, name: &str) -> Result<Option<GrayImage>, String> {
        if !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.')
            || name.is_empty()
        {
            return Err("invalid GGE mask resource name".into());
        }
        let Some(archive) = &self.archive else {
            if !self.warned_missing {
                eprintln!(
                    "GGE DATA unavailable: using cross-fades. Set IKURA_DATA to the original DATA cabinet."
                );
                self.warned_missing = true;
            }
            return Ok(None);
        };
        if !self.cache.contains_key(name) {
            let image =
                image::load_from_memory_with_format(archive.entry(name)?, image::ImageFormat::Bmp)
                    .map_err(|e| format!("GGE {name}: {e}"))?;
            if image.width() == 0
                || image.height() == 0
                || image.width() > 4096
                || image.height() > 4096
            {
                return Err(format!("invalid GGE mask dimensions: {name}"));
            }
            self.cache.insert(name.into(), image.to_luma8());
        }
        Ok(self.cache.get(name).cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_maps_referenced_by_the_game_are_bmp_grayscale_images() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        if !root.join("DATA").is_file() {
            return;
        }
        let mut masks = EffectMasks::new(root).unwrap();
        for name in [
            "EFF01.BIN",
            "EFF03.BIN",
            "EFF04.BIN",
            "EFF17.BIN",
            "SEF03.BIN",
            "SEF04.BIN",
            "SEF05.BIN",
        ] {
            let mask = masks.get(name).unwrap().unwrap();
            assert_eq!(mask.dimensions(), (320, 240));
            let min = mask.pixels().map(|p| p[0]).min().unwrap();
            let max = mask.pixels().map(|p| p[0]).max().unwrap();
            assert!(min < max, "{name} needs distinct reveal thresholds");
            assert!(
                masks.get(name).unwrap().unwrap() == mask,
                "cached map differs"
            );
        }
    }
}
