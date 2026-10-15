//! Render the settings, save and history panels with representative content.
#![allow(dead_code)]
#[path = "../src/graphics.rs"]
mod graphics;
#[path = "../src/history.rs"]
mod history;
#[path = "../src/save.rs"]
mod save;
#[path = "../src/save_menu.rs"]
mod save_menu;
#[path = "../src/settings.rs"]
mod settings;
#[path = "../src/text.rs"]
mod text;
#[path = "../src/vm.rs"]
mod vm;
use std::path::PathBuf;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "/tmp/ikura-interface-probe".into()),
    );
    std::fs::create_dir_all(&root)?;
    let renderer = text::TextRenderer::new()?;
    let background = image::RgbaImage::from_pixel(640, 480, image::Rgba([65, 95, 110, 255]));
    settings::Panel::new(background.clone())
        .draw(&renderer, &settings::Settings::default())
        .save(root.join("settings.png"))?;
    let entries:Vec<_>=(0..15).map(|i|history::Entry {page:vm::TextPage{speaker:Some("春花".into()),body:format!("{}: これは履歴の表示と音声再生のテストです。\n前の台詞をクリックして、もう一度聞くことができます。",i+1)},voices:vec![PathBuf::from("OGG-audio/VOICE1/V001.OGG")]}).collect();
    let mut panel = history::Panel::default();
    panel
        .draw(&renderer, &background, &entries)
        .save(root.join("history.png"))?;
    let scripts = vm::Scripts::from([(
        "TEST".into(),
        vm::Script {
            code_start: 0,
            jumps: vec![],
            instructions: vec![
                vm::Instruction {
                    offset: 0,
                    opcode: 0xF5,
                    payload: vec![1],
                },
                vm::Instruction {
                    offset: 1,
                    opcode: 0x2B,
                    payload: vec![0, 3, 0xff, b'A', 0],
                },
                vm::Instruction {
                    offset: 2,
                    opcode: 0x85,
                    payload: vec![1, 0, 2, 0, 1, 4],
                },
            ],
        },
    )]);
    let mut machine = vm::Vm::new(scripts, "TEST")?;
    machine.step()?;
    machine.step()?;
    let graphics = graphics::Graphics::new(PathBuf::from("PNG-images/GGD"), 640, 480);
    let session = history::Session { history: entries };
    for slot in [save::Slot::Manual(1), save::Slot::Auto] {
        save::write_with_session(
            &save::slot_path(&root, slot),
            42,
            &machine,
            &graphics,
            save::AudioState::default(),
            &background,
            &session,
        )?;
    }
    let mut menu =
        save_menu::SaveMenu::new(save_menu::Mode::Load, &root, 42, background, false, true);
    menu.draw(&renderer).save(root.join("save.png"))?;
    menu.handle(&sdl2::event::Event::KeyDown {
        timestamp: 0,
        window_id: 1,
        keycode: Some(sdl2::keyboard::Keycode::Delete),
        scancode: None,
        keymod: sdl2::keyboard::Mod::NOMOD,
        repeat: false,
    });
    menu.draw(&renderer).save(root.join("delete.png"))?;
    println!("Interface previews: {}", root.display());
    Ok(())
}
