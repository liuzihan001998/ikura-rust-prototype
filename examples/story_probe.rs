//! Visual probe: compose one story page with a speaker header.
#![allow(dead_code)]
#[path = "../src/text.rs"]
mod text;
#[path = "../src/vm.rs"]
mod vm;

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
        font_style: std::env::var("IKURA_PROBE_FONT_STYLE")
            .ok()
            .map(|value| value.parse().expect("font style byte"))
            .unwrap_or(5),
        frame: Some(graphic),
        colors: HashMap::from([(0, [255, 255, 255]), (1, [8, 12, 72])]),
    };
    let page = std::env::var("IKURA_PROBE_PAGE")
        .unwrap_or_else(|_| "どうも騙されたようだ。ずっと前からそう思っていた。".into());
    let speaker = std::env::var("IKURA_PROBE_SPEAKER").unwrap_or_else(|_| "司".into());
    let renderer = TextRenderer::new().expect("font");
    let visible = std::env::var("IKURA_PROBE_REVEAL")
        .ok()
        .map(|value| value.parse::<usize>().expect("visible character count"))
        .unwrap_or(usize::MAX);
    renderer
        .draw_page_revealed(
            &background,
            (!speaker.is_empty()).then_some(speaker.as_str()),
            &page,
            &layout,
            visible,
            visible == usize::MAX,
        )
        .save(&out)
        .expect("save");
    println!("wrote {out}");
}
