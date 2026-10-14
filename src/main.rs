mod graphics;
mod text;
mod video;
mod vm;

use graphics::Graphics;
use image::RgbaImage;
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
use text::{TextRenderer, TextWindowLayout};
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
    if !preview
        && let Some(index) = hover
    {
        canvas.set_draw_color(Color::RGB(255, 225, 30));
        if let Some(area) = menu_rect(vm, index) {
            canvas.draw_rect(area)?;
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

enum FrameGraphic {
    Missing,
    Loaded(RgbaImage),
}

/// One timed graphical transition (`GO` fade or a `GP` command that carries a
/// duration). The VM applies the final frame immediately, so the transition
/// cross-fades the captured before/after surfaces over the scripted time.
struct Transition {
    from: RgbaImage,
    to: RgbaImage,
    duration: Duration,
    elapsed: Duration,
}

impl Transition {
    fn new(from: RgbaImage, to: RgbaImage, duration_ms: i32) -> Self {
        Self {
            from,
            to,
            duration: Duration::from_millis(duration_ms.max(0) as u64),
            elapsed: Duration::ZERO,
        }
    }

    fn progress(&self) -> f64 {
        if self.duration.is_zero() {
            return 1.0;
        }
        (self.elapsed.as_secs_f64() / self.duration.as_secs_f64()).clamp(0.0, 1.0)
    }

    fn advance(&mut self, delta: Duration) -> bool {
        self.elapsed += delta;
        self.elapsed >= self.duration
    }

    fn frame(&self) -> RgbaImage {
        blend_frames(&self.from, &self.to, self.progress())
    }
}

/// Why the runner stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RunOutcome {
    /// The script reached an input poll, story boundary, or its end.
    Reached,
    /// The step budget ran out while the script was waiting for external state.
    Stalled,
}

/// A running transition plus the composed frame to show once it lands.
struct PendingTransition {
    transition: Transition,
    final_frame: RgbaImage,
}

/// Linear blend between two frames; `progress` 0 keeps `from`, 1 keeps `to`.
fn blend_frames(from: &RgbaImage, to: &RgbaImage, progress: f64) -> RgbaImage {
    let progress = progress.clamp(0.0, 1.0);
    let mut frame = from.clone();
    for y in 0..frame.height().min(to.height()) {
        for x in 0..frame.width().min(to.width()) {
            let target = *to.get_pixel(x, y);
            let pixel = frame.get_pixel_mut(x, y);
            for channel in 0..4 {
                let start = f64::from(pixel[channel]);
                pixel[channel] = (start + (f64::from(target[channel]) - start) * progress)
                    .round()
                    .clamp(0.0, 255.0) as u8;
            }
        }
    }
    frame
}

/// The original message-box graphic plus the scripted window rectangle and
/// `SETFONTCOLOR` palette, cached between pages.
struct StoryFrame {
    graphic: Option<String>,
    template: FrameGraphic,
}

impl StoryFrame {
    fn new() -> Self {
        Self {
            graphic: None,
            template: FrameGraphic::Missing,
        }
    }

    fn layout(&mut self, graphics: &Graphics, data_dir: &Path, vm: &Vm) -> TextWindowLayout {
        let window = vm.text_window();
        if self.graphic.as_deref() != window.graphic.as_deref() {
            self.graphic = window.graphic.clone();
            self.template = window
                .graphic
                .as_deref()
                .and_then(|name| load_window_graphic(graphics, data_dir, name))
                .map_or(FrameGraphic::Missing, FrameGraphic::Loaded);
        }
        TextWindowLayout {
            x: window.x,
            y: window.y,
            width: window.width,
            height: window.height,
            font_size: vm.font_size(),
            frame: match &self.template {
                FrameGraphic::Loaded(image) => Some(image.clone()),
                FrameGraphic::Missing => None,
            },
            colors: window.colors.clone(),
        }
    }
}

fn find_graphic(data_dir: &Path, name: &str) -> Option<PathBuf> {
    let wanted = format!("{}.png", name.to_ascii_lowercase());
    for bank in ["GGD", "GGD2"] {
        let dir = data_dir.join("PNG-images").join(bank);
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let file_name = entry.file_name();
            let file_name = file_name.to_string_lossy();
            if file_name.to_ascii_lowercase() == wanted {
                return Some(entry.path());
            }
        }
    }
    None
}

fn load_window_graphic(graphics: &Graphics, data_dir: &Path, name: &str) -> Option<RgbaImage> {
    if let Some(slot) = graphics.slot_of(name) {
        let pixels = graphics.surface(slot)?.clone();
        println!("Story window graphic {name} from graphic slot {slot}");
        return Some(pixels);
    }
    let path = find_graphic(data_dir, name)?;
    let image = image::open(&path).ok()?;
    println!("Story window graphic {name} from {}", path.display());
    Some(image.to_rgba8())
}

/// Instructions the runner may execute before it hands control back to the
/// event loop. A script that spins on state only the window can change (input,
/// frame timers) is reported as stalled instead of failing.
const STEP_BUDGET: usize = 10_000;

#[allow(clippy::too_many_arguments)]
fn run_until_input_or_story(
    vm: &mut Vm,
    graphics: &mut Graphics,
    data_dir: &Path,
    play_movies: bool,
    mut ui: Option<&mut VideoUi>,
    stop_at_story: bool,
) -> Result<(Option<Transition>, RunOutcome), Box<dyn Error>> {
    let mut page_seen = false;
    let mut transition = None;
    for _ in 0..STEP_BUDGET {
        let (script, offset) = vm.position().ok_or("VM finished unexpectedly")?;
        if (stop_at_story && script == "SNR00") || (script == "TITLE" && offset == 0x7B9) {
            return Ok((transition, RunOutcome::Reached));
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
                let before = graphics.screen().clone();
                graphics.fill_screen(color);
                println!(
                    "VM {position}: GO fade {duration_ms}ms to #{:02X}{:02X}{:02X}",
                    color[0], color[1], color[2]
                );
                if transition.is_none() && duration_ms > 0 {
                    transition = Some(Transition::new(
                        before,
                        graphics.screen().clone(),
                        duration_ms,
                    ));
                }
            }
            Step::ShowSurface { slot, duration_ms } => {
                let before = graphics.screen().clone();
                graphics.show_surface(slot)?;
                println!("VM {position}: GGE slot={slot} final frame ({duration_ms}ms)");
                if transition.is_none() && duration_ms > 0 {
                    transition = Some(Transition::new(
                        before,
                        graphics.screen().clone(),
                        duration_ms,
                    ));
                }
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
            Step::TextPage(page) => {
                match &page.speaker {
                    Some(speaker) => println!("VM {position}: PM {speaker}: {}", page.body),
                    None => println!("VM {position}: PM text: {}", page.body),
                }
                if page_seen {
                    return Ok((transition, RunOutcome::Reached));
                }
                page_seen = true;
            }
            Step::InputPoll | Step::ChoicePoll => return Ok((transition, RunOutcome::Reached)),
            Step::Blit(operation) => {
                let before = graphics.screen().clone();
                if let Err(error) = graphics.blit(&operation) {
                    println!("VM paused at {position}: {error}");
                    return Ok((transition, RunOutcome::Reached));
                }
                if transition.is_none() && operation.duration() > 0 {
                    transition = Some(Transition::new(
                        before,
                        graphics.screen().clone(),
                        operation.duration(),
                    ));
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
                        return Ok((transition, RunOutcome::Reached));
                    }
                } else {
                    println!("VM {position}: video skipped");
                }
            }
            Step::Unsupported { opcode, .. } => {
                println!("VM paused at {position}: opcode 0x{opcode:02X} is not implemented yet");
                return Ok((transition, RunOutcome::Reached));
            }
            Step::Yield => vm.advance_clock(16),
            Step::Wait(milliseconds) => {
                if ui.is_some() {
                    std::thread::sleep(Duration::from_millis(milliseconds));
                }
                vm.advance_clock(milliseconds);
            }
            Step::Finished => return Ok((transition, RunOutcome::Reached)),
            Step::ControlFlow => {}
        }
    }
    println!(
        "VM at {:?} is waiting for external state after {STEP_BUDGET} instructions",
        vm.position()
    );
    Ok((transition, RunOutcome::Stalled))
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
    let (mut transition, _) = run_until_input_or_story(
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
        let (_, outcome) = run_until_input_or_story(
            &mut vm,
            &mut graphics,
            &data_dir,
            false,
            None,
            !probe_story,
        )?;
        if outcome == RunOutcome::Stalled {
            println!("Story probe stalled before the first page");
            return Ok(());
        }
        println!("VM after Start: {:?}", vm.position());
        if probe_story {
            let page = vm
                .text_page()
                .ok_or("story probe did not reach a text page")?;
            println!("First story page: {}", page.render());
            if let Some(path) = std::env::var_os("IKURA_DUMP_STORY") {
                let mut story = StoryFrame::new();
                let layout = story.layout(&graphics, &data_dir, &vm);
                let frame = TextRenderer::new()?.draw_page(
                    graphics.screen(),
                    page.speaker.as_deref(),
                    &page.body,
                    &layout,
                );
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
                let (_, outcome) = run_until_input_or_story(
                    &mut vm,
                    &mut graphics,
                    &data_dir,
                    false,
                    None,
                    false,
                )?;
                if outcome == RunOutcome::Stalled && vm.text_page().is_none() {
                    println!("Story probe stalled at {:?}", vm.position());
                    break;
                }
                if vm.waiting_for_choice() {
                    continue;
                } else if let Some(page) = vm.text_page() {
                    println!("Next story page: {}", page.render());
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
    let mut story_frame = StoryFrame::new();
    // A stalled script keeps its state, so further clicks can still advance it.
    let mut stalled = false;
    let mut next_transition = transition
        .take()
        .map(|transition| PendingTransition {
            transition,
            final_frame: graphics.screen().clone(),
        });

    'running: loop {
        let pending_events: Vec<_> = if next_transition.is_some() {
            // Keep animating while still handling input between frames.
            ui.events.poll_iter().collect()
        } else {
            ui.events.wait_timeout_iter(16).collect()
        };
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
                        let (story_transition, _) = run_until_input_or_story(
                            &mut vm,
                            &mut graphics,
                            &data_dir,
                            true,
                            Some(&mut ui),
                            index != 0,
                        )?;
                        let mut frame = graphics.screen().clone();
                        if let Some(page) = vm.text_page() {
                            let renderer = TextRenderer::new()?;
                            let layout = story_frame.layout(&graphics, &data_dir, &vm);
                            frame = renderer.draw_page(
                                graphics.screen(),
                                page.speaker.as_deref(),
                                &page.body,
                                &layout,
                            );
                            text_renderer = Some(renderer);
                            ui.canvas
                                .window_mut()
                                .set_title("家族計画 ～追憶～ | Click for next line")?;
                            preview = true;
                        }
                        if ui.quit_requested {
                            break 'running;
                        }
                        println!("Menu choice {index}: VM position {:?}", vm.position());
                        if let Some(story_transition) = story_transition {
                            next_transition = Some(PendingTransition {
                                transition: story_transition,
                                final_frame: frame,
                            });
                        } else {
                            background = texture_from_rgba(&creator, &frame)?;
                        }
                    }
                }
                Event::MouseButtonDown {
                    mouse_btn: MouseButton::Left,
                    ..
                } if preview && !stalled => {
                    vm.clear_text_page();
                    vm.click();
                    let (story_transition, outcome) = run_until_input_or_story(
                        &mut vm,
                        &mut graphics,
                        &data_dir,
                        true,
                        Some(&mut ui),
                        false,
                    )?;
                    if outcome == RunOutcome::Stalled {
                        stalled = true;
                    }
                    if ui.quit_requested {
                        break 'running;
                    }
                    let mut frame = graphics.screen().clone();
                    if let (Some(page), Some(renderer)) = (vm.text_page(), &text_renderer) {
                        let layout = story_frame.layout(&graphics, &data_dir, &vm);
                        frame =
                            renderer.draw_page(graphics.screen(), page.speaker.as_deref(), &page.body, &layout);
                    } else if stalled {
                        ui.canvas
                            .window_mut()
                            .set_title("Script awaiting input | Esc to quit")?;
                    } else {
                        ui.canvas
                            .window_mut()
                            .set_title("Script paused | Esc to quit")?;
                    }
                    if let Some(story_transition) = story_transition {
                        next_transition = Some(PendingTransition {
                            transition: story_transition,
                            final_frame: frame,
                        });
                    } else {
                        background = texture_from_rgba(&creator, &frame)?;
                    }
                }
                _ => {}
            }
        }
        if let Some(pending) = &mut next_transition {
            let finished = pending.transition.advance(Duration::from_millis(16));
            let frame = if finished {
                pending.final_frame.clone()
            } else {
                pending.transition.frame()
            };
            background = texture_from_rgba(&creator, &frame)?;
            if finished {
                graphics.set_screen(pending.final_frame.clone());
                next_transition = None;
            }
        }
        draw(&mut ui.canvas, &title, &background, preview, hover, &vm)?;
        if next_transition.is_none() {
            std::thread::sleep(Duration::from_millis(4));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::BlitOp;

    fn image(color: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(2, 2, image::Rgba(color))
    }

    #[test]
    fn blend_frames_interpolates_between_the_captured_states() {
        let from = image([0, 0, 0, 255]);
        let to = image([200, 100, 50, 255]);
        assert_eq!(blend_frames(&from, &to, 0.0).get_pixel(0, 0).0, [0, 0, 0, 255]);
        assert_eq!(
            blend_frames(&from, &to, 1.0).get_pixel(0, 0).0,
            [200, 100, 50, 255]
        );
        assert_eq!(
            blend_frames(&from, &to, 0.5).get_pixel(0, 0).0,
            [100, 50, 25, 255]
        );
    }

    #[test]
    fn transition_progress_runs_from_zero_to_complete() {
        let mut transition = Transition::new(image([0, 0, 0, 255]), image([255, 255, 255, 255]), 100);
        assert_eq!(transition.progress(), 0.0);
        assert!(!transition.advance(Duration::from_millis(50)));
        assert!((transition.progress() - 0.5).abs() < 0.01);
        assert!(transition.advance(Duration::from_millis(50)));
        assert_eq!(transition.progress(), 1.0);
    }

    #[test]
    fn zero_duration_transition_is_already_complete() {
        let transition = Transition::new(image([0, 0, 0, 255]), image([1, 2, 3, 255]), 0);
        assert_eq!(transition.progress(), 1.0);
        assert_eq!(transition.frame().get_pixel(0, 0).0, [1, 2, 3, 255]);
    }

    fn empty_scripts() -> Scripts {
        let mut scripts = Scripts::new();
        scripts.insert(
            "LOOP".into(),
            vm::Script {
                code_start: 0,
                jumps: vec![10, 0],
                instructions: vec![
                    Instruction {
                        offset: 0,
                        opcode: 0x04,
                        payload: vec![1, 0],
                    },
                    Instruction {
                        offset: 10,
                        opcode: 0x04,
                        payload: vec![0, 0],
                    },
                ],
            },
        );
        scripts
    }

    #[test]
    fn runner_reports_a_stalled_script_instead_of_failing() {
        let mut vm = Vm::new(empty_scripts(), "LOOP").unwrap();
        let mut graphics = Graphics::new(PathBuf::new(), WIDTH, HEIGHT);
        let (transition, outcome) =
            run_until_input_or_story(&mut vm, &mut graphics, Path::new("."), false, None, false)
                .unwrap();
        assert_eq!(outcome, RunOutcome::Stalled);
        assert!(transition.is_none());
        assert!(
            matches!(vm.position(), Some(("LOOP", 0)) | Some(("LOOP", 10))),
            "the stalled position must be preserved: {:?}",
            vm.position()
        );
    }

    #[test]
    fn gp_transition_commands_expose_their_duration() {
        let mut op = BlitOp {
            command: 7,
            source: 1,
            source_x: 0,
            source_y: 0,
            width: 1,
            height: 1,
            destination: 1500,
            destination_x: 0,
            destination_y: 0,
        };
        assert_eq!(op.duration(), 1500);
        op.command = 1;
        assert_eq!(op.duration(), 0);
        op.command = 19;
        op.destination = -5;
        assert_eq!(op.duration(), 0);
    }
}
