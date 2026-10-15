mod graphics;
mod save;
mod text;
mod video;
mod vm;

use graphics::Graphics;
use image::RgbaImage;
use sdl2::event::Event;
use sdl2::keyboard::{Keycode, Scancode};
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

fn should_advance(
    auto_mode: bool,
    skip: bool,
    voice_playing: bool,
    elapsed: Duration,
    delay: Duration,
) -> bool {
    (skip && elapsed >= Duration::from_millis(40))
        || (auto_mode && !voice_playing && elapsed >= delay)
}

#[derive(Default)]
struct DialogueReveal {
    completed: bool,
    rendered: Option<(usize, bool)>,
}

impl DialogueReveal {
    fn visible(&self, total: usize, interval_ms: u64, elapsed: Duration) -> usize {
        if self.completed || interval_ms == 0 {
            total
        } else {
            (elapsed.as_millis() / u128::from(interval_ms)).min(total as u128) as usize
        }
    }

    /// The first click completes the text without advancing the VM or audio.
    fn consume_click(&mut self, total: usize, interval_ms: u64, elapsed: Duration) -> bool {
        if self.visible(total, interval_ms, elapsed) < total {
            self.completed = true;
            true
        } else {
            false
        }
    }
}

fn dialogue_chars(vm: &Vm) -> usize {
    vm.text_page()
        .map_or(0, |page| page.body.chars().filter(|&ch| ch != '\n').count())
}

struct VideoUi {
    _sdl: sdl2::Sdl,
    canvas: Canvas<Window>,
    events: sdl2::EventPump,
    audio: sdl2::AudioSubsystem,
    mixer_context: Option<Sdl2MixerContext>,
    music: Option<Music<'static>>,
    sounds: HashMap<String, Chunk>,
    voice: Option<Chunk>,
    music_path: Option<PathBuf>,
    voice_path: Option<PathBuf>,
    quit_requested: bool,
}

impl VideoUi {
    fn new() -> Result<Self, Box<dyn Error>> {
        let sdl = sdl2::init()?;
        // The game's own name belongs in the application name, which macOS
        // shows in the menu bar. The window title stays ASCII: SDL's Cocoa
        // backend mangles non-ASCII titles into overlapping mojibake on this
        // platform, which made the title bar unreadable.
        let _ = sdl2::hint::set("SDL_APP_NAME", "家族計画 ～追憶～");
        let video = sdl.video()?;
        let audio = sdl.audio()?;
        let window = video
            .window("Kazoku Keikaku - Rust prototype", WIDTH, HEIGHT)
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
            music_path: None,
            voice_path: None,
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
        self.music_path = Some(path.to_path_buf());
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
        self.voice_path = Some(path.to_path_buf());
        Ok(())
    }

    fn stop_voice(&mut self) {
        if self.mixer_context.is_some() {
            Channel(0).halt();
        }
        self.voice = None;
        self.voice_path = None;
    }

    fn voice_playing(&self) -> bool {
        self.mixer_context.is_some() && Channel(0).is_playing()
    }

    fn skip_held(&self) -> bool {
        let keys = self.events.keyboard_state();
        keys.is_scancode_pressed(Scancode::LCtrl) || keys.is_scancode_pressed(Scancode::RCtrl)
    }

    fn stop_music(&mut self) {
        if self.mixer_context.is_some() {
            Music::halt();
        }
        self.music = None;
        self.music_path = None;
    }

    fn fade_music(&mut self, duration_ms: i32) -> Result<(), Box<dyn Error>> {
        if duration_ms <= 0 {
            self.stop_music();
            return Ok(());
        }
        if self.music.is_some() && Music::is_playing() {
            Music::fade_out(duration_ms)?;
        }
        // A fading track must not be saved as an indefinitely looping song.
        // Leave the Music handle alive while SDL finishes the fade.
        self.music_path = None;
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

fn choice_at(vm: &Vm, x: i32, y: i32) -> Option<u8> {
    vm.choice_options()
        .iter()
        .find(|option| {
            x >= option.x
                && x < option.x + option.width
                && y >= option.y
                && y < option.y + option.height
        })
        .map(|option| option.value)
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
    if !preview && let Some(index) = hover {
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

/// A timed effect over captured surfaces. The VM keeps the final frame while
/// presentation renders the intermediate frames without baking them into VRAM.
struct Transition {
    from: RgbaImage,
    to: RgbaImage,
    duration: Duration,
    elapsed: Duration,
    effect: TransitionEffect,
}

enum TransitionEffect {
    Fade,
    Blinds {
        vertical: bool,
        stripe: u32,
    },
    Wipe,
    Shake {
        count: u16,
        x: u8,
        y: u8,
    },
    Scroll {
        source: RgbaImage,
        destination: [i32; 4],
        start: [i32; 2],
        end: [i32; 2],
    },
}

impl Transition {
    fn new(from: RgbaImage, to: RgbaImage, duration_ms: i32) -> Self {
        Self {
            from,
            to,
            duration: Duration::from_millis(duration_ms.max(0) as u64),
            elapsed: Duration::ZERO,
            effect: TransitionEffect::Fade,
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
        let progress = self.progress();
        if progress >= 1.0 {
            return self.to.clone();
        }
        match &self.effect {
            TransitionEffect::Fade => blend_frames(&self.from, &self.to, progress),
            TransitionEffect::Blinds { vertical, stripe } => {
                let mut frame = self.from.clone();
                for (x, y, pixel) in frame.enumerate_pixels_mut() {
                    let axis = if *vertical { x } else { y };
                    if f64::from(axis % stripe) < f64::from(*stripe) * progress {
                        *pixel = *self.to.get_pixel(x, y);
                    }
                }
                frame
            }
            TransitionEffect::Wipe => {
                let mut frame = self.from.clone();
                let edge = (f64::from(frame.width()) * progress) as u32;
                for y in 0..frame.height() {
                    for x in 0..edge {
                        frame.put_pixel(x, y, *self.to.get_pixel(x, y));
                    }
                }
                frame
            }
            TransitionEffect::Shake { count, x, y } => {
                let phase = progress * f64::from(*count) * std::f64::consts::TAU;
                let dx = (phase.sin() * f64::from(*x)).round() as i32;
                let dy = (phase.sin() * f64::from(*y)).round() as i32;
                let mut frame = RgbaImage::from_pixel(
                    self.from.width(),
                    self.from.height(),
                    image::Rgba([0, 0, 0, 255]),
                );
                for (x, y, pixel) in self.from.enumerate_pixels() {
                    let tx = x as i32 + dx;
                    let ty = y as i32 + dy;
                    if tx >= 0 && ty >= 0 && tx < frame.width() as i32 && ty < frame.height() as i32
                    {
                        frame.put_pixel(tx as u32, ty as u32, *pixel);
                    }
                }
                frame
            }
            TransitionEffect::Scroll {
                source,
                destination,
                start,
                end,
            } => {
                let mut frame = self.from.clone();
                let sx =
                    (f64::from(start[0]) + f64::from(end[0] - start[0]) * progress).round() as i32;
                let sy =
                    (f64::from(start[1]) + f64::from(end[1] - start[1]) * progress).round() as i32;
                for y in 0..destination[3].max(0) {
                    for x in 0..destination[2].max(0) {
                        let dx = destination[0] + x;
                        let dy = destination[1] + y;
                        if dx >= 0
                            && dy >= 0
                            && dx < frame.width() as i32
                            && dy < frame.height() as i32
                            && sx + x >= 0
                            && sy + y >= 0
                            && sx + x < source.width() as i32
                            && sy + y < source.height() as i32
                        {
                            frame.put_pixel(
                                dx as u32,
                                dy as u32,
                                *source.get_pixel((sx + x) as u32, (sy + y) as u32),
                            );
                        }
                    }
                }
                frame
            }
        }
    }
}

/// Run each scripted effect before fetching the next instruction. Event pumping
/// keeps close/skip responsive and prevents later effects replacing earlier ones.
fn play_transition(
    ui: &mut VideoUi,
    vm: &mut Vm,
    mut effect: Transition,
) -> Result<(), Box<dyn Error>> {
    let creator = ui.canvas.texture_creator();
    let started = std::time::Instant::now();
    let mut previous = started;
    loop {
        for event in ui.events.poll_iter() {
            match event {
                Event::Quit { .. } => {
                    ui.quit_requested = true;
                    return Ok(());
                }
                Event::KeyDown {
                    keycode: Some(Keycode::Escape),
                    ..
                }
                | Event::MouseButtonDown {
                    mouse_btn: MouseButton::Left,
                    ..
                } => {
                    effect.elapsed = effect.duration;
                }
                _ => {}
            }
        }
        if ui.skip_held() {
            effect.elapsed = effect.duration;
        }
        let now = std::time::Instant::now();
        let delta = now.duration_since(previous);
        previous = now;
        let finished = effect.advance(delta);
        vm.advance_clock(delta.as_millis() as u64);
        vm.tick_frame();
        let texture = texture_from_rgba(&creator, &effect.frame())?;
        ui.canvas.clear();
        ui.canvas.copy(&texture, None, None)?;
        ui.canvas.present();
        if finished {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(8));
    }
}

fn dispatch_transition(
    ui: &mut Option<&mut VideoUi>,
    vm: &mut Vm,
    pending: &mut Option<Transition>,
    effect: Transition,
) -> Result<(), Box<dyn Error>> {
    if let Some(ui) = ui.as_deref_mut() {
        play_transition(ui, vm, effect)?;
    } else {
        vm.advance_clock(effect.duration.as_millis() as u64);
        *pending = Some(effect);
    }
    Ok(())
}

fn demo_animations(data_dir: &Path, ui: &mut VideoUi, vm: &mut Vm) -> Result<(), Box<dyn Error>> {
    let root = data_dir.join("PNG-images/GGD");
    let from = image::open(root.join("BG01AC.GG0.png"))?.to_rgba8();
    let to = image::open(root.join("BG32AC.GG0.png"))?.to_rgba8();
    let mut strip = RgbaImage::new(from.width() * 3, from.height() * 6);
    let mut wide = RgbaImage::new(from.width() * 2, from.height());
    image::imageops::replace(&mut wide, &from, 0, 0);
    image::imageops::replace(&mut wide, &to, i64::from(from.width()), 0);
    for (row, kind) in [
        TransitionEffect::Fade,
        TransitionEffect::Blinds {
            vertical: false,
            stripe: 16,
        },
        TransitionEffect::Blinds {
            vertical: true,
            stripe: 16,
        },
        TransitionEffect::Wipe,
        TransitionEffect::Shake {
            count: 3,
            x: 12,
            y: 5,
        },
        TransitionEffect::Scroll {
            source: wide,
            destination: [0, 0, from.width() as i32, from.height() as i32],
            start: [0, 0],
            end: [from.width() as i32, 0],
        },
    ]
    .into_iter()
    .enumerate()
    {
        let final_frame = if row == 4 { from.clone() } else { to.clone() };
        let mut effect = Transition::new(from.clone(), final_frame, 700);
        effect.effect = kind;
        for (column, ms) in [175, 350, 525].into_iter().enumerate() {
            effect.elapsed = Duration::from_millis(ms);
            image::imageops::replace(
                &mut strip,
                &effect.frame(),
                (column as u32 * from.width()) as i64,
                (row as u32 * from.height()) as i64,
            );
        }
        effect.elapsed = Duration::ZERO;
        play_transition(ui, vm, effect)?;
        if ui.quit_requested {
            return Ok(());
        }
    }
    if let Some(path) = std::env::var_os("IKURA_DUMP_ANIMATION") {
        strip.save(path)?;
    }
    println!("Animation demo completed: fade, horizontal/vertical blinds, wipe, shake, scroll");
    Ok(())
}

/// Why the runner stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RunOutcome {
    /// The script reached an input poll, story boundary, or its end.
    Reached,
    /// The step budget ran out while the script was waiting for external state.
    Stalled,
    /// Bytecode/graphics operation has no implementation; the probe must fail.
    Unsupported,
    Finished,
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
/// How many times the runner may repeat one instruction cycle before it treats
/// the script as waiting for a rendered frame. Scene scripts poll an animation
/// frame counter in a tight loop, and that counter only moves when the window
/// draws a frame.
const CYCLE_LIMIT: u32 = 64;
/// Longest instruction cycle recognised as an animation wait.
const CYCLE_WINDOW: usize = 32;

/// The scene scripts poll a frame counter while an animation plays. Hand out
/// frames until either the script reaches a boundary or the budget runs out,
/// so the window can redraw between frames.
const FRAME_CATCHUP: usize = 600;
/// Frames the boot path may consume before giving up on reaching the title.
const BOOT_FRAME_BUDGET: usize = 600;

#[allow(clippy::too_many_arguments)]
fn pump_frames_until_stable(
    vm: &mut Vm,
    graphics: &mut Graphics,
    data_dir: &Path,
    ui: &mut VideoUi,
    mut transition: Option<Transition>,
    stalled: bool,
) -> Result<Option<Transition>, Box<dyn Error>> {
    let mut stalled = stalled;
    for _ in 0..FRAME_CATCHUP {
        if !stalled {
            break;
        }
        vm.tick_frame();
        let (next, outcome) =
            run_until_input_or_story(vm, graphics, data_dir, true, Some(ui), false)?;
        if next.is_some() {
            transition = next;
        }
        stalled = outcome == RunOutcome::Stalled;
        if ui.quit_requested {
            break;
        }
    }
    Ok(transition)
}

#[allow(clippy::too_many_arguments)]
fn run_until_input_or_story(
    vm: &mut Vm,
    graphics: &mut Graphics,
    data_dir: &Path,
    play_movies: bool,
    mut ui: Option<&mut VideoUi>,
    stop_at_story: bool,
) -> Result<(Option<Transition>, RunOutcome), Box<dyn Error>> {
    let mut transition = None;
    let mut history: Vec<(String, usize)> = Vec::new();
    let mut last_cycle: Option<usize> = None;
    let mut repeats = 0u32;
    for step_index in 0..STEP_BUDGET {
        if ui.as_ref().is_some_and(|ui| ui.quit_requested) {
            return Ok((transition, RunOutcome::Reached));
        }
        if let Some(ui) = ui.as_deref_mut() {
            // A held Ctrl can run many opcodes before the outer event loop.
            // Refresh keyboard state so releasing it stops fast-forward promptly.
            if step_index % 64 == 0 {
                ui.events.pump_events();
            }
            vm.set_playback_state(ui.skip_held(), ui.voice_playing());
        }
        let Some((script, offset)) = vm.position() else {
            return Ok((transition, RunOutcome::Finished));
        };
        if (stop_at_story && script == "SNR00") || (script == "TITLE" && offset == 0x7B9) {
            return Ok((transition, RunOutcome::Reached));
        }
        let here = (script.to_owned(), offset);
        let cycle = history
            .iter()
            .rev()
            .take(CYCLE_WINDOW)
            .position(|seen| *seen == here)
            .map(|back| back + 1);
        if let Some(length) = cycle {
            if Some(length) == last_cycle {
                repeats += 1;
            } else {
                last_cycle = Some(length);
                repeats = 0;
            }
        } else {
            last_cycle = None;
            repeats = 0;
        }
        if repeats >= CYCLE_LIMIT {
            println!(
                "VM at {script}:0x{offset:X} is waiting for a rendered frame (cycle of {last_cycle:?})"
            );
            return Ok((transition, RunOutcome::Stalled));
        }
        history.push(here);
        if history.len() > CYCLE_WINDOW {
            history.remove(0);
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
                if duration_ms > 0 {
                    dispatch_transition(
                        &mut ui,
                        vm,
                        &mut transition,
                        Transition::new(before, graphics.screen().clone(), duration_ms),
                    )?;
                }
            }
            Step::ShowSurface { slot, duration_ms } => {
                let before = graphics.screen().clone();
                graphics.show_surface(slot)?;
                println!("VM {position}: GGE slot={slot} final frame ({duration_ms}ms)");
                if duration_ms > 0 {
                    dispatch_transition(
                        &mut ui,
                        vm,
                        &mut transition,
                        Transition::new(before, graphics.screen().clone(), duration_ms),
                    )?;
                }
            }
            Step::Shake {
                count,
                x,
                y,
                duration_ms,
            } => {
                let screen = graphics.screen().clone();
                let total = duration_ms.max(0).saturating_mul(i32::from(count));
                let mut effect = Transition::new(screen.clone(), screen, total);
                effect.effect = TransitionEffect::Shake { count, x, y };
                dispatch_transition(&mut ui, vm, &mut transition, effect)?;
            }
            Step::Scroll {
                slot,
                destination,
                mut start,
                mut end,
                duration_ms,
            } => {
                let source = graphics
                    .surface(slot)
                    .ok_or("GSCRL source surface missing")?
                    .clone();
                if destination[2] <= 0 || destination[3] <= 0 {
                    return Err("GSCRL viewport must have positive dimensions".into());
                }
                let max_x = (source.width() as i32 - destination[2]).max(0);
                let max_y = (source.height() as i32 - destination[3]).max(0);
                for point in [&mut start, &mut end] {
                    point[0] = point[0].clamp(0, max_x);
                    point[1] = point[1].clamp(0, max_y);
                }
                let before = graphics.screen().clone();
                graphics.blit(&vm::BlitOp {
                    command: 0,
                    source: slot,
                    source_x: end[0],
                    source_y: end[1],
                    width: destination[2],
                    height: destination[3],
                    destination: 0,
                    destination_x: destination[0],
                    destination_y: destination[1],
                })?;
                let mut effect = Transition::new(before, graphics.screen().clone(), duration_ms);
                effect.effect = TransitionEffect::Scroll {
                    source,
                    destination,
                    start,
                    end,
                };
                dispatch_transition(&mut ui, vm, &mut transition, effect)?;
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
            Step::TextPage(page) => match &page.speaker {
                Some(speaker) => println!("VM {position}: PM {speaker}: {}", page.body),
                None => println!("VM {position}: PM text: {}", page.body),
            },
            Step::InputPoll | Step::ChoicePoll => return Ok((transition, RunOutcome::Reached)),
            Step::Blit(operation) => {
                if matches!(operation.command, 24 | 25 | 28 | 29) {
                    println!(
                        "VM {position}: GP {} uses cross-fade approximation; extended effect parameters are pending",
                        operation.command
                    );
                }
                let before = graphics.screen().clone();
                if let Err(error) = graphics.blit(&operation) {
                    println!("VM paused at {position}: {error}");
                    return Ok((transition, RunOutcome::Unsupported));
                }
                if operation.duration() > 0 {
                    let mut effect =
                        Transition::new(before, graphics.screen().clone(), operation.duration());
                    effect.effect = match operation.command {
                        15 => TransitionEffect::Blinds {
                            vertical: false,
                            stripe: 16,
                        },
                        16 => TransitionEffect::Blinds {
                            vertical: true,
                            stripe: 16,
                        },
                        17 => TransitionEffect::Wipe,
                        _ => TransitionEffect::Fade,
                    };
                    dispatch_transition(&mut ui, vm, &mut transition, effect)?;
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
                return Ok((transition, RunOutcome::Unsupported));
            }
            Step::Yield => {
                vm.advance_clock(16);
                // A timer poll is making progress even if its PC cycles.
                history.clear();
                last_cycle = None;
                repeats = 0;
            }
            Step::Wait(milliseconds) => {
                if ui.is_some() {
                    std::thread::sleep(Duration::from_millis(milliseconds));
                }
                vm.advance_clock(milliseconds);
            }
            Step::Finished => return Ok((transition, RunOutcome::Finished)),
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
    let probe_save = mode.as_deref() == Some("--probe-save");
    let probe_story = probe_save || mode.as_deref() == Some("--probe-story");
    let probe_start = probe_story || mode.as_deref() == Some("--probe-start");
    let script_bytes = std::fs::read(&script_path)?;
    let script_signature = save::script_signature(&script_bytes);
    let scripts: Scripts = serde_json::from_slice(&script_bytes)?;
    let quicksave_path = std::env::var_os("IKURA_SAVE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| data_dir.join("rust-saves"))
        .join("quicksave.json");
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
    if mode.as_deref() == Some("--demo-animation") {
        demo_animations(
            &data_dir,
            video_ui.as_mut().ok_or("SDL window missing")?,
            &mut vm,
        )?;
        return Ok(());
    }
    // Begin at START's real code_start. The VM executes initialization,
    // video playback, and title loading without jumping over setup code. The
    // boot scripts poll frame counters, so hand out frames until the title
    // input poll is reached.
    let mut boot_frames = 0;
    let mut transition = loop {
        let (transition, outcome) = run_until_input_or_story(
            &mut vm,
            &mut graphics,
            &data_dir,
            !probe_start && std::env::var_os("IKURA_SKIP_VIDEO").is_none(),
            video_ui.as_mut(),
            true,
        )?;
        if outcome != RunOutcome::Stalled || boot_frames >= BOOT_FRAME_BUDGET {
            break transition;
        }
        boot_frames += 1;
        vm.tick_frame();
    };
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
        let mut boot_frames = 0;
        loop {
            let (_, outcome) = run_until_input_or_story(
                &mut vm,
                &mut graphics,
                &data_dir,
                false,
                None,
                !probe_story,
            )?;
            if outcome != RunOutcome::Stalled || boot_frames >= BOOT_FRAME_BUDGET {
                break;
            }
            boot_frames += 1;
            vm.tick_frame();
        }
        println!("VM after Start: {:?}", vm.position());
        if probe_story {
            let page = vm
                .text_page()
                .ok_or("story probe did not reach a text page")?;
            println!("First story page: {}", page.render());
            let save_probe = if probe_save {
                let path = std::env::var_os("IKURA_PROBE_SAVE_OUTPUT")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| {
                        std::env::temp_dir()
                            .join(format!("ikura-real-save-probe-{}.json", std::process::id()))
                    });
                save::write(
                    &path,
                    script_signature,
                    &vm,
                    &graphics,
                    save::AudioState::default(),
                )?;
                Some((path, serde_json::to_value(&vm)?, graphics.screen().clone()))
            } else {
                None
            };
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
                .clamp(1, 100_000);
            let choice_value = std::env::var("IKURA_PROBE_CHOICE")
                .ok()
                .and_then(|value| value.parse::<u8>().ok())
                .unwrap_or(0);
            let frame_budget = std::env::var("IKURA_PROBE_FRAMES")
                .ok()
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0);
            let mut frames_left = frame_budget;
            let mut page_count = 1;
            let mut stop_reason = "page_limit";
            while page_count < page_limit {
                if vm.waiting_for_choice() {
                    println!("Story choices: {:?}", vm.choice_options());
                    if let Some(path) = std::env::var_os("IKURA_DUMP_CHOICE") {
                        let renderer = TextRenderer::new()?;
                        renderer
                            .draw_choices(
                                graphics.screen(),
                                vm.choice_options(),
                                Some(choice_value),
                            )
                            .save(path)?;
                    }
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
                if outcome == RunOutcome::Stalled {
                    if frames_left > 0 {
                        // The script is polling an animation frame; advance it
                        // the way the window would and try again.
                        frames_left -= 1;
                        vm.tick_frame();
                        continue;
                    }
                    return Err(format!(
                        "Story probe stalled after {page_count} pages at {:?}",
                        vm.position()
                    )
                    .into());
                }
                if outcome == RunOutcome::Unsupported {
                    return Err(format!(
                        "Story probe hit an unsupported operation after {page_count} pages at {:?}",
                        vm.position()
                    )
                    .into());
                }
                if outcome == RunOutcome::Finished {
                    stop_reason = "finished";
                    break;
                }
                if vm.waiting_for_choice() {
                    continue;
                } else if let Some(page) = vm.text_page() {
                    println!("Next story page: {}", page.render());
                    page_count += 1;
                } else {
                    println!("Story probe paused at {:?}", vm.position());
                    stop_reason = "no_text_boundary";
                    break;
                }
            }
            println!(
                "Story probe summary: pages={page_count} reason={stop_reason} position={:?}",
                vm.position()
            );
            if let Some((path, expected_vm, expected_screen)) = save_probe {
                save::read(&path, script_signature, &mut vm, &mut graphics)?;
                let actual_vm = serde_json::to_value(&vm)?;
                let restored = actual_vm == expected_vm && graphics.screen() == &expected_screen;
                if std::env::var_os("IKURA_PROBE_SAVE_OUTPUT").is_none() {
                    std::fs::remove_file(path)?;
                }
                if !restored {
                    if let Some(fields) = expected_vm.as_object() {
                        for (key, value) in fields {
                            if actual_vm.get(key) != Some(value) {
                                eprintln!("Save mismatch in VM field: {key}");
                            }
                        }
                    }
                    eprintln!(
                        "Save screen pixels restored: {}",
                        graphics.screen() == &expected_screen
                    );
                    return Err("save probe did not restore the original state and screen".into());
                }
                println!(
                    "Save probe restored the original VM state and screen at {:?}",
                    vm.position()
                );
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
    let mut text_renderer: Option<TextRenderer> = None;
    let mut story_frame = StoryFrame::new();
    let mut choosing = false;
    let mut choice_hover: Option<u8> = None;
    let mut auto_mode = false;
    let mut dialogue_hidden = false;
    let mut page_started = std::time::Instant::now();
    let mut reveal_started = page_started;
    let mut reveal = DialogueReveal::default();
    let mut next_transition = transition.take().map(|transition| PendingTransition {
        transition,
        final_frame: graphics.screen().clone(),
    });

    'running: loop {
        let mut pending_events: Vec<_> = if next_transition.is_some() {
            // Keep animating while still handling input between frames.
            ui.events.poll_iter().collect()
        } else {
            ui.events.wait_timeout_iter(16).collect()
        };
        let auto_delay = vm.text_page().map_or(1500, |page| {
            (800 + page.body.chars().count() as u64 * 55).clamp(1500, 8000)
        }) + dialogue_chars(&vm) as u64 * vm.text_interval_ms();
        let advance = preview
            && !choosing
            && !dialogue_hidden
            && vm.text_visible()
            && next_transition.is_none()
            && should_advance(
                auto_mode,
                ui.skip_held(),
                ui.voice_playing(),
                page_started.elapsed(),
                Duration::from_millis(auto_delay),
            );
        if advance && pending_events.is_empty() {
            pending_events.push(Event::MouseButtonDown {
                timestamp: 0,
                window_id: ui.canvas.window().id(),
                which: 0,
                mouse_btn: MouseButton::Left,
                clicks: 1,
                x: 0,
                y: 0,
            });
        }
        for event in pending_events {
            let event = match event {
                Event::KeyDown {
                    keycode: Some(Keycode::Space | Keycode::Return),
                    repeat: false,
                    ..
                } if preview && !choosing => Event::MouseButtonDown {
                    timestamp: 0,
                    window_id: ui.canvas.window().id(),
                    which: 0,
                    mouse_btn: MouseButton::Left,
                    clicks: 1,
                    x: 0,
                    y: 0,
                },
                Event::MouseButtonDown {
                    x,
                    y,
                    mouse_btn: MouseButton::Left,
                    ..
                } if !preview && menu_at(&vm, x, y) == Some(1) => Event::KeyDown {
                    timestamp: 0,
                    window_id: ui.canvas.window().id(),
                    keycode: Some(Keycode::F9),
                    scancode: None,
                    keymod: sdl2::keyboard::Mod::NOMOD,
                    repeat: false,
                },
                event => event,
            };
            match event {
                Event::Quit { .. } => break 'running,
                Event::KeyDown {
                    keycode: Some(Keycode::A),
                    repeat: false,
                    ..
                } if preview && !choosing => {
                    auto_mode = !auto_mode;
                    page_started = std::time::Instant::now();
                    ui.canvas.window_mut().set_title(if auto_mode {
                        "Kazoku Keikaku - AUTO (A to stop)"
                    } else {
                        "Kazoku Keikaku - manual"
                    })?;
                }
                Event::KeyDown {
                    keycode: Some(Keycode::H),
                    repeat: false,
                    ..
                }
                | Event::MouseButtonDown {
                    mouse_btn: MouseButton::Right,
                    ..
                } if preview && !choosing => {
                    dialogue_hidden = !dialogue_hidden;
                    page_started = std::time::Instant::now();
                }
                Event::KeyDown {
                    keycode: Some(Keycode::F5),
                    repeat: false,
                    ..
                } if preview && next_transition.is_none() => {
                    let audio = save::AudioState {
                        music: if ui.mixer_context.is_some() && Music::is_playing() {
                            ui.music_path.clone()
                        } else {
                            None
                        },
                        voice: if ui.voice_playing() {
                            ui.voice_path.clone()
                        } else {
                            None
                        },
                    };
                    match save::write(&quicksave_path, script_signature, &vm, &graphics, audio) {
                        Ok(()) => {
                            println!("Quick save: {}", quicksave_path.display());
                            ui.canvas
                                .window_mut()
                                .set_title("Kazoku Keikaku - quick save stored")?;
                        }
                        Err(error) => {
                            eprintln!("Quick save failed: {error}");
                            ui.canvas
                                .window_mut()
                                .set_title("Quick save failed - see console")?;
                        }
                    }
                }
                Event::KeyDown {
                    keycode: Some(Keycode::F9),
                    repeat: false,
                    ..
                } => match save::read(&quicksave_path, script_signature, &mut vm, &mut graphics) {
                    Ok(audio) => {
                        ui.stop_music();
                        ui.stop_sound();
                        ui.stop_voice();
                        if let Some(path) = audio.music.filter(|_| vm.should_restore_music()) {
                            ui.start_music(&path)?;
                        }
                        if let Some(path) = audio.voice {
                            ui.play_voice(&path)?;
                        }
                        preview = true;
                        choosing = vm.waiting_for_choice();
                        choice_hover = None;
                        auto_mode = false;
                        dialogue_hidden = false;
                        next_transition = None;
                        page_started = std::time::Instant::now();
                        reveal_started = page_started;
                        reveal = DialogueReveal {
                            completed: true,
                            rendered: None,
                        };
                        story_frame = StoryFrame::new();
                        let renderer = TextRenderer::new()?;
                        let mut frame = graphics.screen().clone();
                        if let Some(page) = vm.text_page() {
                            let layout = story_frame.layout(&graphics, &data_dir, &vm);
                            frame = renderer.draw_page(
                                &frame,
                                page.speaker.as_deref(),
                                &page.body,
                                &layout,
                            );
                        }
                        text_renderer = Some(renderer);
                        background = texture_from_rgba(&creator, &frame)?;
                        println!("Quick load: {:?}", vm.position());
                        ui.canvas
                            .window_mut()
                            .set_title("Kazoku Keikaku - quick save restored")?;
                    }
                    Err(error) => {
                        eprintln!("Quick load failed: {error}");
                        ui.canvas
                            .window_mut()
                            .set_title("Quick load failed - see console")?;
                    }
                },
                Event::KeyDown {
                    keycode: Some(Keycode::Escape),
                    ..
                } if preview => break 'running,
                Event::KeyDown {
                    keycode: Some(Keycode::Escape),
                    ..
                } => break 'running,
                Event::MouseMotion { x, y, .. } if choosing => {
                    choice_hover = choice_at(&vm, x, y);
                }
                Event::MouseMotion { x, y, .. } if !preview => {
                    hover = menu_at(&vm, x, y);
                }
                Event::MouseButtonDown {
                    x,
                    y,
                    mouse_btn: MouseButton::Left,
                    ..
                } if choosing => {
                    if let Some(value) = choice_at(&vm, x, y) {
                        auto_mode = false;
                        ui.stop_voice();
                        vm.choose(value)?;
                        println!("Choice {value} selected");
                        let (story_transition, outcome) = run_until_input_or_story(
                            &mut vm,
                            &mut graphics,
                            &data_dir,
                            true,
                            Some(&mut ui),
                            false,
                        )?;
                        let story_transition = pump_frames_until_stable(
                            &mut vm,
                            &mut graphics,
                            &data_dir,
                            &mut ui,
                            story_transition,
                            outcome == RunOutcome::Stalled,
                        )?;
                        if ui.quit_requested {
                            break 'running;
                        }
                        choosing = vm.waiting_for_choice();
                        page_started = std::time::Instant::now();
                        reveal_started = page_started;
                        reveal = DialogueReveal::default();
                        choice_hover = None;
                        let mut frame = graphics.screen().clone();
                        if let (Some(page), Some(renderer)) =
                            (vm.text_page(), text_renderer.as_ref())
                        {
                            let layout = story_frame.layout(&graphics, &data_dir, &vm);
                            frame = renderer.draw_page(
                                graphics.screen(),
                                page.speaker.as_deref(),
                                &page.body,
                                &layout,
                            );
                        }
                        if choosing {
                            vm.clear_text_page();
                        }
                        if let Some(story_transition) = story_transition {
                            next_transition = Some(PendingTransition {
                                transition: story_transition,
                                final_frame: frame,
                            });
                        } else if !choosing {
                            background = texture_from_rgba(&creator, &frame)?;
                        }
                    }
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
                        if index == 0 {
                            // TITLE clears its BGM registers before LS SNR00,
                            // but that branch contains no standalone MS/MF.
                            // End the menu track when starting a new game so
                            // the unvoiced opening cannot inherit title music.
                            ui.stop_music();
                            println!("Title -> new game: stopped title music");
                        }
                        vm.set_selected(Some(index as i16));
                        // Consume the pending IG, then execute its script branch.
                        vm.step()?;
                        vm.set_selected(None);
                        let (story_transition, outcome) = run_until_input_or_story(
                            &mut vm,
                            &mut graphics,
                            &data_dir,
                            true,
                            Some(&mut ui),
                            index != 0,
                        )?;
                        let story_transition = pump_frames_until_stable(
                            &mut vm,
                            &mut graphics,
                            &data_dir,
                            &mut ui,
                            story_transition,
                            outcome == RunOutcome::Stalled,
                        )?;
                        if ui.quit_requested {
                            break 'running;
                        }
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
                                .set_title("Kazoku Keikaku - click for next line")?;
                            preview = true;
                        }
                        if ui.quit_requested {
                            break 'running;
                        }
                        println!("Menu choice {index}: VM position {:?}", vm.position());
                        choosing = vm.waiting_for_choice();
                        page_started = std::time::Instant::now();
                        reveal_started = page_started;
                        reveal = DialogueReveal::default();
                        if choosing {
                            vm.clear_text_page();
                        }
                        if let Some(story_transition) = story_transition {
                            next_transition = Some(PendingTransition {
                                transition: story_transition,
                                final_frame: frame,
                            });
                        } else if !choosing {
                            background = texture_from_rgba(&creator, &frame)?;
                        }
                    }
                }
                Event::MouseButtonDown {
                    mouse_btn: MouseButton::Left,
                    ..
                } if preview => {
                    if dialogue_hidden {
                        dialogue_hidden = false;
                        page_started = std::time::Instant::now();
                        continue;
                    }
                    if !ui.skip_held()
                        && reveal.consume_click(
                            dialogue_chars(&vm),
                            vm.text_interval_ms(),
                            reveal_started.elapsed(),
                        )
                    {
                        // Completing the visible sentence must keep its voice
                        // playing and leave the pending IG untouched.
                        page_started = std::time::Instant::now();
                        continue;
                    }
                    // Advancing ends the current line even when the next page
                    // is unvoiced or the script starts a scene transition.
                    // Do this before running the VM so PCMGETPOS sees it too.
                    ui.stop_voice();
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
                    let story_transition = pump_frames_until_stable(
                        &mut vm,
                        &mut graphics,
                        &data_dir,
                        &mut ui,
                        story_transition,
                        outcome == RunOutcome::Stalled,
                    )?;
                    if ui.quit_requested {
                        break 'running;
                    }
                    choosing = vm.waiting_for_choice();
                    if choosing {
                        auto_mode = false;
                    }
                    page_started = std::time::Instant::now();
                    reveal_started = page_started;
                    reveal = DialogueReveal::default();
                    if choosing {
                        vm.clear_text_page();
                    }
                    if ui.quit_requested {
                        break 'running;
                    }
                    let mut frame = graphics.screen().clone();
                    if let (Some(page), Some(renderer)) = (vm.text_page(), &text_renderer) {
                        let layout = story_frame.layout(&graphics, &data_dir, &vm);
                        frame = renderer.draw_page(
                            graphics.screen(),
                            page.speaker.as_deref(),
                            &page.body,
                            &layout,
                        );
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
                // Keep the VM surface free of composed text: the box graphic is
                // translucent, so text baked in here would show through the
                // next page's box as ghosting.
                next_transition = None;
                reveal_started = std::time::Instant::now();
                page_started = reveal_started;
                reveal = DialogueReveal::default();
            }
        }
        vm.tick_frame();
        if preview
            && !choosing
            && next_transition.is_none()
            && (dialogue_hidden || !vm.text_visible())
        {
            let scene = texture_from_rgba(&creator, graphics.screen())?;
            draw(&mut ui.canvas, &title, &scene, preview, hover, &vm)?;
        } else if choosing && !vm.choice_options().is_empty() {
            let frame = TextRenderer::new()?.draw_choices(
                graphics.screen(),
                vm.choice_options(),
                choice_hover,
            );
            let overlay = texture_from_rgba(&creator, &frame)?;
            draw(&mut ui.canvas, &title, &overlay, preview, hover, &vm)?;
        } else if preview && next_transition.is_none() && vm.text_page().is_some() {
            let total = dialogue_chars(&vm);
            if ui.skip_held() {
                reveal.completed = true;
            }
            let visible = reveal.visible(total, vm.text_interval_ms(), reveal_started.elapsed());
            let marker =
                visible == total && (reveal_started.elapsed().as_millis() / 500).is_multiple_of(2);
            if reveal.rendered != Some((visible, marker)) {
                if let (Some(page), Some(renderer)) = (vm.text_page(), text_renderer.as_ref()) {
                    let layout = story_frame.layout(&graphics, &data_dir, &vm);
                    let frame = renderer.draw_page_revealed(
                        graphics.screen(),
                        page.speaker.as_deref(),
                        &page.body,
                        &layout,
                        visible,
                        marker,
                    );
                    background = texture_from_rgba(&creator, &frame)?;
                }
                reveal.rendered = Some((visible, marker));
            }
            draw(&mut ui.canvas, &title, &background, preview, hover, &vm)?;
        } else {
            draw(&mut ui.canvas, &title, &background, preview, hover, &vm)?;
        }
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
    fn dialogue_click_completes_before_it_advances_and_zero_interval_is_instant() {
        let mut reveal = DialogueReveal::default();
        let total = "春花\n「こんにちは」"
            .chars()
            .filter(|&c| c != '\n')
            .count();
        let elapsed = Duration::from_millis(75);
        assert_eq!(reveal.visible(total, 25, elapsed), 3);
        assert!(reveal.consume_click(total, 25, elapsed));
        assert_eq!(reveal.visible(total, 25, elapsed), total);
        assert!(!reveal.consume_click(total, 25, elapsed));
        let mut instant = DialogueReveal::default();
        assert_eq!(instant.visible(total, 0, Duration::ZERO), total);
        assert!(!instant.consume_click(total, 0, Duration::ZERO));
        assert_eq!(instant.visible(total, 25, Duration::from_secs(20)), total);
    }

    #[test]
    fn auto_waits_for_voice_and_reading_delay_while_skip_can_interrupt_voice() {
        let elapsed = Duration::from_secs(2);
        let delay = Duration::from_millis(1500);
        assert!(should_advance(true, false, false, elapsed, delay));
        assert!(!should_advance(true, false, true, elapsed, delay));
        assert!(!should_advance(
            true,
            false,
            false,
            Duration::from_millis(500),
            delay
        ));
        assert!(should_advance(false, true, true, elapsed, delay));
        assert!(!should_advance(
            false,
            true,
            false,
            Duration::from_millis(10),
            delay
        ));
        assert!(!should_advance(false, false, false, elapsed, delay));
    }

    #[test]
    fn blend_frames_interpolates_between_the_captured_states() {
        let from = image([0, 0, 0, 255]);
        let to = image([200, 100, 50, 255]);
        assert_eq!(
            blend_frames(&from, &to, 0.0).get_pixel(0, 0).0,
            [0, 0, 0, 255]
        );
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
        let mut transition =
            Transition::new(image([0, 0, 0, 255]), image([255, 255, 255, 255]), 100);
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

    #[test]
    fn runner_settles_consecutive_text_blocks_at_the_actual_input_poll() {
        let mut scripts = Scripts::new();
        scripts.insert(
            "TEXT".into(),
            vm::Script {
                code_start: 0,
                jumps: vec![],
                instructions: vec![
                    Instruction {
                        offset: 0,
                        opcode: 0x2B,
                        payload: vec![0, 3, 0xFF, b'A', 0],
                    },
                    Instruction {
                        offset: 1,
                        opcode: 0x2B,
                        payload: vec![0, 6, 0xFF, b'B', 0],
                    },
                    Instruction {
                        offset: 2,
                        opcode: 0x86,
                        payload: vec![],
                    },
                    Instruction {
                        offset: 3,
                        opcode: 0x85,
                        payload: vec![1, 0, 2, 0, 1, 4],
                    },
                ],
            },
        );
        let mut vm = Vm::new(scripts, "TEXT").unwrap();
        let mut graphics = Graphics::new(PathBuf::new(), 1, 1);
        let (_, result) =
            run_until_input_or_story(&mut vm, &mut graphics, Path::new(""), false, None, false)
                .unwrap();
        assert_eq!(result, RunOutcome::Reached);
        assert_eq!(vm.position(), Some(("TEXT", 3)));
        assert_eq!(vm.text_page().unwrap().body, "A\nB");
        vm.click();
        assert_eq!(vm.step().unwrap(), Step::ControlFlow);
    }

    #[test]
    fn runner_distinguishes_unsupported_bytecode_from_normal_completion() {
        for (opcode, expected) in [
            (0xFF, RunOutcome::Unsupported),
            (0x03, RunOutcome::Finished),
        ] {
            let mut scripts = Scripts::new();
            scripts.insert(
                "END".into(),
                vm::Script {
                    code_start: 0,
                    jumps: vec![],
                    instructions: vec![Instruction {
                        offset: 0,
                        opcode,
                        payload: vec![],
                    }],
                },
            );
            let mut vm = Vm::new(scripts, "END").unwrap();
            let mut graphics = Graphics::new(PathBuf::new(), 1, 1);
            let (_, outcome) =
                run_until_input_or_story(&mut vm, &mut graphics, Path::new(""), false, None, false)
                    .unwrap();
            assert_eq!(outcome, expected);
            if expected == RunOutcome::Unsupported {
                assert_eq!(vm.position(), Some(("END", 0)));
            }
        }
    }

    #[test]
    fn blinds_reveal_stripes_then_land_on_the_exact_final_frame() {
        let from = RgbaImage::from_pixel(8, 8, image::Rgba([0, 0, 0, 255]));
        let to = RgbaImage::from_pixel(8, 8, image::Rgba([255, 255, 255, 255]));
        for vertical in [false, true] {
            let mut effect = Transition::new(from.clone(), to.clone(), 100);
            effect.effect = TransitionEffect::Blinds {
                vertical,
                stripe: 4,
            };
            effect.advance(Duration::from_millis(50));
            let frame = effect.frame();
            assert_eq!(frame.pixels().filter(|pixel| pixel[0] == 255).count(), 32);
            assert_ne!(
                frame.get_pixel(0, 0),
                frame.get_pixel(if vertical { 2 } else { 0 }, if vertical { 0 } else { 2 })
            );
            effect.advance(Duration::from_millis(50));
            assert_eq!(effect.frame(), to);
        }
    }

    #[test]
    fn shake_moves_pixels_without_modifying_the_persistent_screen() {
        let mut screen = RgbaImage::from_pixel(8, 8, image::Rgba([0, 0, 0, 255]));
        screen.put_pixel(2, 2, image::Rgba([255, 0, 0, 255]));
        let mut effect = Transition::new(screen.clone(), screen.clone(), 100);
        effect.effect = TransitionEffect::Shake {
            count: 1,
            x: 2,
            y: 1,
        };
        effect.advance(Duration::from_millis(25));
        assert_eq!(effect.frame().get_pixel(4, 3)[0], 255);
        effect.advance(Duration::from_millis(75));
        assert_eq!(effect.frame(), screen);
    }

    #[test]
    fn scroll_samples_the_intermediate_viewport_and_restores_the_final_frame() {
        let source = RgbaImage::from_fn(8, 2, |x, _| image::Rgba([x as u8, 0, 0, 255]));
        let before = RgbaImage::from_pixel(2, 2, image::Rgba([0, 0, 0, 255]));
        let after = RgbaImage::from_fn(2, 2, |x, _| *source.get_pixel(x + 6, 0));
        let mut effect = Transition::new(before, after.clone(), 100);
        effect.effect = TransitionEffect::Scroll {
            source,
            destination: [0, 0, 2, 2],
            start: [0, 0],
            end: [6, 0],
        };
        effect.advance(Duration::from_millis(50));
        assert_eq!(effect.frame().get_pixel(0, 0)[0], 3);
        effect.advance(Duration::from_millis(50));
        assert_eq!(effect.frame(), after);
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

    /// The post-choice animation loop at SNR00:0x4590F compares the engine
    /// frame counter (read as register 0x1F) with a snapshot. Giving it frames
    /// must break the cycle.
    #[test]
    fn animation_loop_exits_when_frames_are_pumped() {
        let path = Path::new("data/scripts.json");
        if !path.is_file() {
            return;
        }
        let scripts: Scripts = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let mut vm = Vm::new(scripts, "SNR00").unwrap();
        vm.set_position(0x4590F).unwrap();
        let mut graphics = Graphics::new(PathBuf::new(), WIDTH, HEIGHT);
        let mut stalls = 0;
        for _ in 0..(CYCLE_LIMIT + 8) {
            // The fragment normally runs as a subroutine; reaching its `RET`
            // with an empty stack is the expected way out of the guard.
            let outcome = match run_until_input_or_story(
                &mut vm,
                &mut graphics,
                Path::new("."),
                false,
                None,
                false,
            ) {
                Ok((_, outcome)) => outcome,
                Err(_) => break,
            };
            if outcome == RunOutcome::Reached {
                break;
            }
            stalls += 1;
            vm.tick_frame();
        }
        assert!(
            vm.position().is_some_and(|(_, offset)| offset != 0x4590F),
            "the animation loop should leave its guard after {stalls} frame ticks, at {:?}",
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
