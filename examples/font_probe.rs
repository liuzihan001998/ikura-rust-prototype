//! Temporary diagnostic: print fontdue metrics for the story fonts.
use fontdue::{Font, FontSettings};

fn probe(path: &str) {
    println!("=== {path}");
    if !std::path::Path::new(path).is_file() {
        println!("    MISSING");
        return;
    }
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            println!("    read failed: {e}");
            return;
        }
    };
    let font = match Font::from_bytes(bytes, FontSettings::default()) {
        Ok(f) => f,
        Err(e) => {
            println!("    load FAILED: {e}");
            return;
        }
    };
    println!("    loaded ok");
    for px in [20.0f32, 22.0] {
        match font.horizontal_line_metrics(px) {
            Some(m) => println!(
                "    px={px}: ascent={:.2} descent={:.2} line_gap={:.2} new_line={:.2}",
                m.ascent, m.descent, m.line_gap, m.new_line_size
            ),
            None => println!("    px={px}: horizontal_line_metrics = None"),
        }
        let mut total = 0.0f32;
        let sample = "騙されたかな、";
        for ch in sample.chars() {
            let (m, bmp) = font.rasterize(ch, px);
            println!(
                "        {:?} advance={:.3} w={} h={} xmin={} ymin={} bitmap={}",
                ch,
                m.advance_width,
                m.width,
                m.height,
                m.xmin,
                m.ymin,
                bmp.len()
            );
            total += m.advance_width;
        }
        println!(
            "    sum advance = {total:.2} for {} chars; expected ~{:.0}",
            sample.chars().count(),
            sample.chars().count() as f32 * px
        );
    }
}

fn main() {
    probe("/System/Library/Fonts/ヒラギノ角ゴシック W8.ttc");
    probe("/System/Library/Fonts/Hiragino Sans GB.ttc");
}
