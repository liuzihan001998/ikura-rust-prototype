//! Visual probe: compose one story page with a speaker header.
#[path = "../src/text.rs"]
mod text;

use image::{Rgba, RgbaImage};
use std::collections::HashMap;
use text::{TextRenderer, TextWindowLayout};

fn main() {
    let data = std::env::args().nth(1).expect("data dir");
    let out = std::env::args().nth(2).expect("output png");
    let background = RgbaImage::from_pixel(640, 480, Rgba([20, 20, 30, 255]));
    let graphic = image::open(format!("{data}/PNG-images/GGD/KA_M.GG2.png"))
        .expect("window graphic")
        .to_rgba8();
    let layout = TextWindowLayout {
        x: 44,
        y: 320,
        width: 0,
        height: 0,
        font_size: 20,
        frame: Some(graphic),
        colors: HashMap::from([(0, [255, 255, 255]), (1, [8, 12, 72])]),
    };
    let page = std::env::var("IKURA_PROBE_PAGE")
        .unwrap_or_else(|_| "どうも騙されたようだ。ずっと前からそう思っていた。".into());
    let renderer = TextRenderer::new().expect("font");
    renderer
        .draw_page(&background, Some("司"), &page, &layout)
        .save(&out)
        .expect("save");
    println!("wrote {out}");
}
