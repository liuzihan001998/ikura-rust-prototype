mod graphics;
mod text;
mod video;
mod vm;

use graphics::Graphics;
use sdl2::event::Event;
use sdl2::keyboard::Keycode;
use sdl2::mixer::{self, AUDIO_S16SYS, Channel, Chunk, InitFlag, Music, Sdl2MixerContext};
use sdl2::mouse::MouseButton;
use sdl2::pixels::{Color, PixelFormatEnum};
use sdl2::rect::Rect;
use sdl2::render::{Canvas, Texture, TextureCreator};
use sdl2::video::{Window, WindowContext};
use std::collections::HashMap;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::Duration;
use text::TextRenderer;
use video::Playback;
use vm::{Instruction, Scripts, Step, Vm};

const WIDTH: u32 = 640;
const HEIGHT: u32 = 480;

struct VideoUi {
    _sdl: sdl2::Sdl,
    canvas: Canvas<Window>,
    events: sdl2::EventPump,
    audio: sdl2::AudioSubsystem,
    mixer_context: Option<Sdl2MixerContext>,
    music: Option<Music<'static>>,
    sounds: HashMap<String, Chunk>,
    voice: Option<Chunk>,
    quit_requested: bool,
}

impl VideoUi {
    fn new() -> Result<Self, Box<dyn Error>> {
        let sdl = sdl2::init()?;
        let video = sdl.video()?;
        let audio = sdl.audio()?;
        let window = video
            .window("家族計画 ～追憶～ | Rust title prototype", WIDTH, HEIGHT)
            .position_centered()
            .resizable()
            .build()?;
        let mut canvas = window.into_canvas().accelerated().build()?;
        canvas.set_logical_size(WIDTH, HEIGHT)?;
        canvas.set_draw_color(Color::BLACK);
        canvas.clear();
        canvas.present();
        let events = sdl.event_pump()?;
        Ok(Self {
            _sdl: sdl,
            canvas,
            events,
            audio,
            mixer_context: None,
            music: None,
            sounds: HashMap::new(),
            voice: None,
            quit_requested: false,
        })
    }

    fn start_music(&mut self, path: &Path) -> Result<(), Box<dyn Error>> {
        self.stop_music();
        self.ensure_mixer()?;
        let music = Music::from_file(path)?;
        music.play(-1)?;
        if !Music::is_playing() {
            return Err(format!("SDL_mixer did not start {}", path.display()).into());
        }
        self.music = Some(music);
        Ok(())
    }

    fn ensure_mixer(&mut self) -> Result<(), Box<dyn Error>> {
        if self.mixer_context.is_none() {
            let context = mixer::init(InitFlag::OGG)?;
            mixer::open_audio(48000, AUDIO_S16SYS, 2, 1024)?;
            mixer::reserve_channels(2);
            self.mixer_context = Some(context);
        }
        Ok(())
    }

    fn play_sound(&mut self, path: &Path) -> Result<(), Box<dyn Error>> {
        self.ensure_mixer()?;
        let key = path.to_string_lossy().into_owned();
        if !self.sounds.contains_key(&key) {
            self.sounds.insert(key.clone(), Chunk::from_file(path)?);
        }
        Channel(1).play(self.sounds.get(&key).ok_or("sound cache failed")?, 0)?;
        Ok(())
    }

    fn stop_sound(&mut self) {
        if self.mixer_context.is_some() {
            Channel(1).halt();
        }
    }

    fn play_voice(&mut self, path: &Path) -> Result<(), Box<dyn Error>> {
        self.ensure_mixer()?;
        self.stop_voice();
        self.voice = Some(Chunk::from_file(path)?);
        Channel(0).play(self.voice.as_ref().ok_or("voice load failed")?, 0)?;
        Ok(())
    }

    fn stop_voice(&mut self) {
        if self.mixer_context.is_some() {
            Channel(0).halt();
        }
        self.voice = None;
    }

    fn stop_music(&mut self) {
        if self.mixer_context.is_some() {
            Music::halt();
        }
        self.music = None;
    }

    fn fade_music(&mut self, duration_ms: i32) -> Result<(), Box<dyn Error>> {
        if self.music.is_some() && Music::is_playing() {
            Music::fade_out(duration_ms.max(0))?;
        }
        Ok(())
    }
}

impl Drop for VideoUi {
    fn drop(&mut self) {
        self.stop_music();
        self.stop_voice();
        Channel::all().halt();
        self.sounds.clear();
        if self.mixer_context.is_some() {
            mixer::close_audio();
            self.mixer_context = None;
        }
    }
}
fn menu_rect(vm: &Vm, index: usize) -> Option<Rect> {
    let area = vm.hit_areas().get(&(index as u8))?;
    if area.width <= 0 || area.height <= 0 {
        return None;
    }
    Some(Rect::new(
        area.x,
        area.y,
        area.width as u32,
        area.height as u32,
    ))
}

fn contains_named_instruction(script: &[Instruction], opcode: u8, name: &str) -> bool {
    let needle = name.as_bytes();
    script.iter().any(|instruction| {
        instruction.opcode == opcode
            && instruction
                .payload
                .windows(needle.len())
                .any(|window| window == needle)
    })
}

fn texture_from_rgba<'a>(
    creator: &'a TextureCreator<WindowContext>,
    rgba: &image::RgbaImage,
) -> Result<Texture<'a>, Box<dyn Error>> {
    let (width, height) = rgba.dimensions();
    let mut texture = creator.create_texture_static(PixelFormatEnum::RGBA32, width, height)?;
    texture.update(None, rgba.as_raw(), (width * 4) as usize)?;
    Ok(texture)
}

fn menu_at(vm: &Vm, x: i32, y: i32) -> Option<usize> {
    (0..4).find(|&index| menu_rect(vm, index).is_some_and(|area| area.contains_point((x, y))))
}

fn draw(
    canvas: &mut Canvas<Window>,
    title: &Texture,
    background: &Texture,
    preview: bool,
    hover: Option<usize>,
    vm: &Vm,
) -> Result<(), Box<dyn Error>> {
    canvas.set_draw_color(Color::BLACK);
    canvas.clear();
    canvas.copy(if preview { background } else { title }, None, None)?;
    if !preview {
        if let Some(index) = hover {
            canvas.set_draw_color(Color::RGB(255, 225, 30));
            if let Some(area) = menu_rect(vm, index) {
                canvas.draw_rect(area)?;
            }
        }
    }
    canvas.present();
    Ok(())
}

fn play_video(
    data_dir: &Path,
    name: &str,
    width: i32,
    height: i32,
    ui: &mut VideoUi,
) -> Result<(), Box<dyn Error>> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(format!("invalid AVIP video name {name:?}").into());
    }
    if width <= 0 || height <= 0 || width > 4096 || height > 4096 {
        return Err(format!("invalid AVIP destination size {width}x{height}").into());
    }
    let file_name = format!("{name}.MPG");
    let mut roots = Vec::new();
    if let Some(override_dir) = std::env::var_os("IKURA_VIDEO_DIR") {
        roots.push(PathBuf::from(override_dir));
    }
    roots.push(data_dir.to_path_buf());
    if let Some(parent) = data_dir.parent() {
        roots.push(parent.to_path_buf());
    }
    if let Some(parent) = std::env::current_dir()?.parent() {
        roots.push(parent.to_path_buf());
    }
    let path = roots
    .iter()
    .map(|root| root.join(&file_name))
    .find(|path| path.is_file())
    .ok_or_else(|| {
        format!(
            "AVIP video {file_name} was not found; set IKURA_VIDEO_DIR to the original game directory"
        )
    })?;
    println!(
        "Playing {} ({}x{}) in SDL; press Esc to skip",
        path.display(),
        width,
        height
    );
    match video::play(
        &mut ui.canvas,
        &mut ui.events,
        &ui.audio,
        &path,
        width as u32,
        height as u32,
    )? {
        Playback::Quit => ui.quit_requested = true,
        Playback::Skipped => println!("Video skipped"),
        Playback::Finished => println!("Video finished"),
    }
    Ok(())
}

fn run_until_input_or_story(
    vm: &mut Vm,
    graphics: &mut Graphics,
    data_dir: &Path,
    play_movies: bool,
    mut ui: Option<&mut VideoUi>,
    stop_at_story: bool,
) -> Result<(), Box<dyn Error>> {
    let mut page_seen = false;
    for _ in 0..10000 {
        let (script, offset) = vm.position().ok_or("VM finished unexpectedly")?;
        if (stop_at_story && script == "SNR00") || (script == "TITLE" && offset == 0x7B9) {
            return Ok(());
        }
        let position = format!("{script}:0x{offset:X}");
        if std::env::var_os("IKURA_TRACE_VM").is_some() {
            println!("TRACE {position}");
        }
        match vm.step()? {
            Step::LoadGraphic { slot, name } => {
                let (width, height) = graphics.load(slot, &name)?;
                println!("VM {position}: GL slot={slot} name={name} ({width}x{height})");
            }
            Step::FadeTo { duration_ms, color } => {
                graphics.fill_screen(color);
                println!(
                    "VM {position}: GO fade {duration_ms}ms to #{:02X}{:02X}{:02X}",
                    color[0], color[1], color[2]
                );
            }
            Step::ShowSurface { slot, duration_ms } => {
                graphics.show_surface(slot)?;
                println!("VM {position}: GGE slot={slot} final frame ({duration_ms}ms)");
            }
            Step::MusicStopped => {
                if let Some(ui) = ui.as_deref_mut() {
                    ui.stop_music();
                }
                println!("VM {position}: MS stop music");
            }
            Step::MusicFade(duration_ms) => {
                if let Some(ui) = ui.as_deref_mut() {
                    ui.fade_music(duration_ms)?;
                }
                println!("VM {position}: MF fade {duration_ms}ms");
            }
            Step::MusicRequested(name) => {
                let stem = Path::new(&name)
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .ok_or("ML has an invalid music name")?;
                let path = data_dir.join("OGG-audio/WMSC").join(format!("{stem}.OGG"));
                if let Some(ui) = ui.as_deref_mut() {
                    ui.start_music(&path)?;
                    println!("VM {position}: ML playing {name} -> {}", path.display());
                } else {
                    println!(
                        "VM {position}: ML {name} -> {} (headless probe)",
                        path.display()
                    );
                }
            }
            Step::SoundRequested(name) => {
                let path = data_dir.join("OGG-audio/SE").join(format!("{name}.OGG"));
                if let Some(ui) = ui.as_deref_mut() {
                    ui.play_sound(&path)?;
                    println!("VM {position}: SER playing {name}");
                } else {
                    println!("VM {position}: SER {name} (headless probe)");
                }
            }
            Step::SoundStopped => {
                if let Some(ui) = ui.as_deref_mut() {
                    ui.stop_sound();
                }
            }
            Step::VoiceRequested { bank, name } => {
                if !matches!(bank.as_str(), "VOICE1" | "VOICE2")
                    || !name
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                {
                    return Err(format!("invalid voice asset {bank}/{name}").into());
                }
                let path = data_dir
                    .join("OGG-audio")
                    .join(&bank)
                    .join(format!("{name}.OGG"));
                if let Some(ui) = ui.as_deref_mut() {
                    ui.play_voice(&path)?;
                    println!("VM {position}: PCML playing {bank}/{name}");
                } else {
                    println!("VM {position}: PCML {bank}/{name} (headless probe)");
                }
            }
            Step::VoiceStopped => {
                if let Some(ui) = ui.as_deref_mut() {
                    ui.stop_voice();
                }
            }
            Step::GraphicBank(bank) => graphics.set_bank(&bank)?,
            Step::TextPage(text) => {
                println!("VM {position}: PM text: {text}");
                if page_seen {
                    return Ok(());
                }
                page_seen = true;
            }
            Step::InputPoll => return Ok(()),
            Step::ChoicePoll => return Ok(()),
            Step::Blit(operation) => {
                if let Err(error) = graphics.blit(&operation) {
                    println!("VM paused at {position}: {error}");
                    return Ok(());
                }
            }
            Step::InitSurface {
                slot,
                width,
                height,
            } => graphics.allocate(slot, width, height)?,
            Step::FillSurface { slot, color } => graphics.fill_surface(slot, color)?,
            Step::PlayVideo {
                name,
                source_width,
                source_height,
                destination_width,
                destination_height,
            } => {
                println!(
                    "VM {position}: AVIP {name} source={source_width}x{source_height} destination={destination_width}x{destination_height}"
                );
                if play_movies {
                    let ui = ui.as_deref_mut().ok_or("AVIP needs an SDL window")?;
                    play_video(data_dir, &name, destination_width, destination_height, ui)?;
                    if ui.quit_requested {
                        return Ok(());
                    }
                } else {
                    println!("VM {position}: video skipped");
                }
            }
            Step::Unsupported { opcode, .. } => {
                println!("VM paused at {position}: opcode 0x{opcode:02X} is not implemented yet");
                return Ok(());
            }
            Step::Yield => vm.advance_clock(16),
            Step::Wait(milliseconds) => {
                if ui.is_some() {
                    std::thread::sleep(Duration::from_millis(milliseconds));
                }
                vm.advance_clock(milliseconds);
            }
            Step::Finished => return Ok(()),
            Step::ControlFlow => {}
        }
    }
    Err(format!(
        "VM exceeded 10000 steps before reaching an input or story boundary; position {:?}",
        vm.position()
    )
    .into())
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let data_dir = PathBuf::from(
        args.next()
            .ok_or("usage: cargo run -- <game_data directory> [scripts.json]")?,
    );
    let script_path = PathBuf::from(args.next().unwrap_or_else(|| "data/scripts.json".into()));
    let mode = args.next();
    let probe_story = mode.as_deref() == Some("--probe-story");
    let probe_start = probe_story || mode.as_deref() == Some("--probe-start");
    let scripts: Scripts = serde_json::from_slice(&std::fs::read(&script_path)?)?;
    let start = scripts.get("START").ok_or("START script missing")?;
    let title_script = scripts.get("TITLE").ok_or("TITLE script missing")?;

    for (opcode, name) in [(0x56, "TITLE"), (0x56, "TITLEX"), (0x01, "TITLE")] {
        if !contains_named_instruction(&start.instructions, opcode, name) {
            return Err(format!("START does not contain opcode {opcode:02X} for {name}").into());
        }
    }
    println!(
        "START: {} instructions; TITLE: {} instructions",
        start.instructions.len(),
        title_script.instructions.len()
    );
    println!("Boot path markers: GL TITLE -> GL TITLEX -> LS TITLE");
    println!("The VM executes the startup scripts and pauses at the TITLE input poll.");
    if let Some(first) = title_script.instructions.first() {
        println!(
            "TITLE entry: 0x{:X}, opcode 0x{:02X}",
            first.offset, first.opcode
        );
    }

    let mut vm = Vm::new(scripts, "START")?;
    let image_dir = data_dir.join("PNG-images/GGD");
    let mut graphics = Graphics::new(image_dir.clone(), WIDTH, HEIGHT);
    let mut video_ui = if probe_start {
        None
    } else {
        Some(VideoUi::new()?)
    };
    // Begin at START's real code_start. The VM executes initialization,
    // video playback, and title loading without jumping over setup code.
    run_until_input_or_story(
        &mut vm,
        &mut graphics,
        &data_dir,
        !probe_start && std::env::var_os("IKURA_SKIP_VIDEO").is_none(),
        video_ui.as_mut(),
        true,
    )?;
    if video_ui.as_ref().is_some_and(|ui| ui.quit_requested) {
        return Ok(());
    }
    if vm.position() != Some(("TITLE", 0x7B9)) {
        return Err(format!("startup did not reach TITLE input: {:?}", vm.position()).into());
    }
    println!("VM registered {} speaker names", vm.speaker_names().len());
    if let Some(name) = &vm.text_window().graphic {
        println!("VM text window graphic: {name}");
    }
    println!("VM probe position: {:?}", vm.position());
    if let Some(path) = std::env::var_os("IKURA_DUMP_TITLE") {
        graphics.screen().save(path)?;
    }
    if probe_start {
        vm.set_selected(Some(0));
        vm.step()?;
        vm.set_selected(None);
        run_until_input_or_story(&mut vm, &mut graphics, &data_dir, false, None, !probe_story)?;
        println!("VM after Start: {:?}", vm.position());
        if probe_story {
            let page = vm
                .text_page()
                .ok_or("story probe did not reach a text page")?;
            println!("First story page: {page}");
            if let Some(path) = std::env::var_os("IKURA_DUMP_STORY") {
                let frame = TextRenderer::new()?.draw_page(graphics.screen(), page);
                frame.save(path)?;
            }
            let page_limit = std::env::var("IKURA_PROBE_PAGES")
                .ok()
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(3)
                .clamp(1, 1000);
            let choice_value = std::env::var("IKURA_PROBE_CHOICE")
                .ok()
                .and_then(|value| value.parse::<u8>().ok())
                .unwrap_or(0);
            let mut page_count = 1;
            while page_count < page_limit {
                if vm.waiting_for_choice() {
                    println!("Story choices: {:?}", vm.choice_options());
                    vm.choose(choice_value)?;
                    println!("Story probe selected choice {choice_value}");
                } else {
                    vm.clear_text_page();
                    vm.click();
                }
                run_until_input_or_story(&mut vm, &mut graphics, &data_dir, false, None, false)?;
                if vm.waiting_for_choice() {
                    continue;
                } else if let Some(page) = vm.text_page() {
                    println!("Next story page: {page}");
                    page_count += 1;
                } else {
                    println!("Story probe paused at {:?}", vm.position());
                    break;
                }
            }
        }
        return Ok(());
    }

    let mut ui = video_ui.ok_or("SDL window was not created")?;
    let creator = ui.canvas.texture_creator();
    let title = texture_from_rgba(&creator, graphics.screen())?;
    let mut background = texture_from_rgba(&creator, graphics.screen())?;
    let mut hover = None;
    let mut preview = false;
    let mut text_renderer = None;

    'running: loop {
        let pending_events: Vec<_> = ui.events.poll_iter().collect();
        for event in pending_events {
            match event {
                Event::Quit { .. } => break 'running,
                Event::KeyDown {
                    keycode: Some(Keycode::Escape),
                    ..
                } if preview => break 'running,
                Event::KeyDown {
                    keycode: Some(Keycode::Escape),
                    ..
                } => break 'running,
                Event::MouseMotion { x, y, .. } if !preview => {
                    hover = menu_at(&vm, x, y);
                }
                Event::MouseButtonDown {
                    x,
                    y,
                    mouse_btn: MouseButton::Left,
                    ..
                } if !preview => {
                    if let Some(index) = menu_at(&vm, x, y) {
                        if index == 3 {
                            break 'running;
                        }
                        vm.set_selected(Some(index as i16));
                        // Consume the pending IG, then execute its script branch.
                        vm.step()?;
                        vm.set_selected(None);
                        run_until_input_or_story(
                            &mut vm,
                            &mut graphics,
                            &data_dir,
                            true,
                            Some(&mut ui),
                            index != 0,
                        )?;
                        if ui.quit_requested {
                            break 'running;
                        }
                        println!("Menu choice {index}: VM position {:?}", vm.position());
                        if let Some(page) = vm.text_page() {
                            let renderer = TextRenderer::new()?;
                            let frame = renderer.draw_page(graphics.screen(), page);
                            background = texture_from_rgba(&creator, &frame)?;
                            text_renderer = Some(renderer);
                            ui.canvas
                                .window_mut()
                                .set_title("家族計画 ～追憶～ | Click for next line")?;
                            preview = true;
                        }
                    }
                }
                Event::MouseButtonDown {
                    mouse_btn: MouseButton::Left,
                    ..
                } if preview => {
                    vm.clear_text_page();
                    vm.click();
                    run_until_input_or_story(
                        &mut vm,
                        &mut graphics,
                        &data_dir,
                        true,
                        Some(&mut ui),
                        false,
                    )?;
                    if ui.quit_requested {
                        break 'running;
                    }
                    if let (Some(page), Some(renderer)) = (vm.text_page(), &text_renderer) {
                        let frame = renderer.draw_page(graphics.screen(), page);
                        background = texture_from_rgba(&creator, &frame)?;
                    } else {
                        background = texture_from_rgba(&creator, graphics.screen())?;
                        ui.canvas
                            .window_mut()
                            .set_title("Script paused | Esc to quit")?;
                    }
                }
                _ => {}
            }
        }
        draw(&mut ui.canvas, &title, &background, preview, hover, &vm)?;
        std::thread::sleep(Duration::from_millis(16));
    }
    Ok(())
}
