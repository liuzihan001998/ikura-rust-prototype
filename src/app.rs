mod assets;
mod cursor;
mod effects;
mod graphics;
mod history;
mod save;
mod save_menu;
mod settings;
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
use vm::{Instruction, Scripts, Step, TranslationPack, Vm};

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

fn record_history_page(history: &mut Vec<history::Entry>, vm: &Vm, ui: &mut VideoUi, root: &Path) {
    if let Some(page) = vm.text_page() {
        let voices = std::mem::take(&mut ui.page_voices)
            .into_iter()
            .filter_map(|path| path.strip_prefix(root).ok().map(Path::to_path_buf))
            .filter(|path| history::safe_voice(path))
            .collect();
        history.push(history::Entry {
            page: page.clone(),
            voices,
        });
        if history.len() > history::LIMIT {
            history.drain(..history.len() - history::LIMIT);
        }
    } else {
        ui.page_voices.clear();
    }
}

fn checkpoint_due(
    previous: Option<&(String, usize)>,
    current: Option<&(String, usize)>,
    choosing: bool,
    elapsed: Duration,
) -> bool {
    current.is_some()
        && previous != current
        && (previous.is_none()
            || choosing
            || elapsed >= Duration::from_secs(30)
            || previous.zip(current).is_some_and(|(a, b)| a.0 != b.0))
}

fn is_title_input(vm: &Vm) -> bool {
    vm.position() == Some(("TITLE", 0x7B9))
}

#[derive(Debug, PartialEq, Eq)]
enum WaitInput {
    Quit,
    Skip,
    Defer,
    Ignore,
}

fn wait_input(event: &Event) -> WaitInput {
    match event {
        Event::Quit { .. }
        | Event::KeyDown {
            keycode: Some(Keycode::Escape),
            ..
        } => WaitInput::Quit,
        Event::MouseButtonDown {
            mouse_btn: MouseButton::Left,
            ..
        }
        | Event::KeyDown {
            keycode: Some(Keycode::Space | Keycode::Return),
            repeat: false,
            ..
        } => WaitInput::Skip,
        Event::KeyDown { repeat: false, .. }
        | Event::MouseMotion { .. }
        | Event::MouseButtonDown {
            mouse_btn: MouseButton::Right,
            ..
        } => WaitInput::Defer,
        _ => WaitInput::Ignore,
    }
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
    cursors: cursor::GameCursors,
    masks: effects::EffectMasks,
    deferred_events: Vec<Event>,
    mouse_range: Option<Option<vm::MouseRange>>,
    settings: settings::Settings,
    music_started: Option<std::time::Instant>,
    voice_started: Option<std::time::Instant>,
    music_offset_ms: u64,
    music_duration_ms: Option<u64>,
    voice_offset_ms: u64,
    page_voices: Vec<PathBuf>,
    replay_voice: Option<Chunk>,
    replay_queue: std::collections::VecDeque<PathBuf>,
    modal_voice_paused: Option<std::time::Instant>,
}

impl VideoUi {
    fn new(data_dir: &Path) -> Result<Self, Box<dyn Error>> {
        let sdl = sdl2::init()?;
        // The game's own name belongs in the application name, which macOS
        // shows in the menu bar. The window title stays ASCII: SDL's Cocoa
        // backend mangles non-ASCII titles into overlapping mojibake on this
        // platform, which made the title bar unreadable.
        let _ = sdl2::hint::set("SDL_APP_NAME", "家族計画 ～追憶～");
        if cfg!(target_os = "android") {
            let _ = sdl2::hint::set("SDL_IOS_ORIENTATIONS", "LandscapeLeft LandscapeRight");
        }
        let video = sdl.video()?;
        let audio = sdl.audio()?;
        let mut window_builder = video.window("Kazoku Keikaku - Rust prototype", WIDTH, HEIGHT);
        window_builder.position_centered();
        if cfg!(target_os = "android") {
            window_builder.fullscreen_desktop();
        } else {
            window_builder.resizable();
        }
        let window = window_builder.build()?;
        let mut canvas = window.into_canvas().accelerated().build()?;
        canvas.set_logical_size(WIDTH, HEIGHT)?;
        canvas.set_draw_color(Color::BLACK);
        canvas.clear();
        canvas.present();
        let events = sdl.event_pump()?;
        let cursors = cursor::GameCursors::new(data_dir)?;
        let masks = effects::EffectMasks::new(data_dir)?;
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
            cursors,
            masks,
            deferred_events: Vec::new(),
            mouse_range: None,
            settings: settings::Settings::default(),
            music_started: None,
            voice_started: None,
            music_offset_ms: 0,
            music_duration_ms: None,
            voice_offset_ms: 0,
            page_voices: Vec::new(),
            replay_voice: None,
            replay_queue: Default::default(),
            modal_voice_paused: None,
        })
    }

    fn sync_cursor(&mut self, vm: &Vm, override_id: Option<u8>) -> Result<(), String> {
        if cfg!(target_os = "android") {
            return Ok(());
        }
        let id = override_id.unwrap_or(vm.cursor_style());
        self.cursors.update(id, vm.cursor_resource(id))?;
        let range = vm.mouse_range();
        if self.mouse_range != Some(range) {
            let bounds = range.map(|range| scale_mouse_range(range, self.canvas.window().size()));
            self.canvas
                .window()
                .set_mouse_rect(bounds)
                .map_err(|error| format!("IMS could not set mouse range: {error}"))?;
            self.mouse_range = Some(range);
        }
        Ok(())
    }

    fn warp_mouse(&self, x: i32, y: i32) {
        let (window_width, window_height) = self.canvas.window().size();
        let x = x.clamp(0, WIDTH as i32 - 1) * window_width as i32 / WIDTH as i32;
        let y = y.clamp(0, HEIGHT as i32 - 1) * window_height as i32 / HEIGHT as i32;
        self._sdl
            .mouse()
            .warp_mouse_in_window(self.canvas.window(), x, y);
    }

    fn wait_script(&mut self, vm: &Vm, milliseconds: u64) -> Result<bool, String> {
        if milliseconds == 0 {
            return Ok(!self.quit_requested);
        }
        let started = std::time::Instant::now();
        let duration = Duration::from_millis(milliseconds);
        loop {
            let mut skip = false;
            for event in self.events.poll_iter().collect::<Vec<_>>() {
                match wait_input(&event) {
                    WaitInput::Quit => self.quit_requested = true,
                    WaitInput::Skip => skip = true,
                    WaitInput::Defer => {
                        // Keep the last mouse position and bounded command
                        // input for the outer UI, without replaying skip clicks.
                        if matches!(event, Event::MouseMotion { .. }) {
                            self.deferred_events
                                .retain(|event| !matches!(event, Event::MouseMotion { .. }));
                        }
                        if self.deferred_events.len() < 128 {
                            self.deferred_events.push(event);
                        }
                    }
                    WaitInput::Ignore => {}
                }
            }
            if self.quit_requested {
                return Ok(false);
            }
            self.sync_cursor(vm, None)?;
            if skip || self.skip_held() || started.elapsed() >= duration {
                return Ok(true);
            }
            std::thread::sleep(
                duration
                    .saturating_sub(started.elapsed())
                    .min(Duration::from_millis(8)),
            );
        }
    }

    fn apply_volumes(&self) {
        if self.mixer_context.is_some() {
            Music::set_volume(i32::from(self.settings.music) * 128 / 100);
            Channel(1).set_volume(i32::from(self.settings.sound) * 128 / 100);
            for channel in [0, 2] {
                Channel(channel).set_volume(i32::from(self.settings.voice) * 128 / 100);
            }
        }
    }
    fn apply_settings(&mut self) -> Result<(), String> {
        self.apply_volumes();
        let desired = if self.settings.fullscreen {
            sdl2::video::FullscreenType::Desktop
        } else {
            sdl2::video::FullscreenType::Off
        };
        if self.canvas.window().fullscreen_state() != desired {
            self.canvas.window_mut().set_fullscreen(desired)?;
        }
        Ok(())
    }
    fn audio_state(&self) -> save::AudioState {
        save::AudioState {
            music: if self.mixer_context.is_some() && Music::is_playing() {
                self.music_path.clone()
            } else {
                None
            },
            voice: if self.voice_playing() {
                self.voice_path.clone()
            } else {
                None
            },
            music_position_ms: {
                let elapsed = self.music_offset_ms
                    + self
                        .music_started
                        .map_or(0, |t| t.elapsed().as_millis() as u64);
                self.music_duration_ms
                    .filter(|duration| *duration > 0)
                    .map_or(elapsed, |duration| elapsed % duration)
            },
            voice_position_ms: self.voice_offset_ms
                + self
                    .voice_started
                    .map_or(0, |t| t.elapsed().as_millis() as u64),
        }
    }
    fn pause_dialogue_voice(&mut self) {
        if self.voice_playing() {
            Channel(0).pause();
            self.modal_voice_paused = Some(std::time::Instant::now());
        }
    }
    fn resume_dialogue_voice(&mut self) {
        if let Some(start) = self.modal_voice_paused.take() {
            if let Some(voice_start) = &mut self.voice_started {
                *voice_start += start.elapsed();
            }
            Channel(0).resume();
        }
    }
    fn stop_replay(&mut self) {
        if self.mixer_context.is_some() {
            Channel(2).halt();
        }
        self.replay_voice = None;
        self.replay_queue.clear();
    }
    fn queue_replay(&mut self, paths: Vec<PathBuf>, root: &Path) -> Result<(), Box<dyn Error>> {
        self.ensure_mixer()?;
        self.stop_replay();
        self.replay_queue = paths
            .into_iter()
            .filter(|p| history::safe_voice(p))
            .map(|p| root.join(p))
            .collect();
        self.tick_replay()
    }
    fn tick_replay(&mut self) -> Result<(), Box<dyn Error>> {
        if self.mixer_context.is_some() && !Channel(2).is_playing() {
            self.replay_voice = None;
            if let Some(path) = self.replay_queue.pop_front() {
                let chunk = Chunk::from_file(path)?;
                Channel(2).play(&chunk, 0)?;
                self.replay_voice = Some(chunk);
            }
        }
        Ok(())
    }
    fn restore_voice(&mut self, path: &Path, offset_ms: u64) -> Result<(), Box<dyn Error>> {
        if offset_ms == 0 {
            return self.play_voice(path);
        }
        if cfg!(target_os = "android") {
            return self.play_voice(path);
        }
        self.ensure_mixer()?;
        self.stop_voice();
        let output = std::process::Command::new(
            std::env::var_os("IKURA_FFMPEG").unwrap_or_else(|| "ffmpeg".into()),
        )
        .args(["-nostdin", "-v", "error", "-ss"])
        .arg(format!("{:.3}", offset_ms as f64 / 1000.))
        .arg("-i")
        .arg(path)
        .args(["-vn", "-ac", "2", "-ar", "48000", "-f", "wav", "pipe:1"])
        .output()?;
        if !output.status.success() {
            return Err("could not seek saved voice".into());
        }
        use sdl2::mixer::LoaderRWops;
        let chunk = sdl2::rwops::RWops::from_bytes(&output.stdout)?.load_wav()?;
        Channel(0).play(&chunk, 0)?;
        self.voice = Some(chunk);
        self.voice_path = Some(path.to_path_buf());
        self.voice_offset_ms = offset_ms;
        self.voice_started = Some(std::time::Instant::now());
        Ok(())
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
        self.music_started = Some(std::time::Instant::now());
        self.music_offset_ms = 0;
        self.music_duration_ms = if cfg!(target_os = "android") {
            None
        } else {
            std::process::Command::new(
                std::env::var_os("IKURA_FFPROBE").unwrap_or_else(|| "ffprobe".into()),
            )
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=duration",
                "-of",
                "default=noprint_wrappers=1:nokey=1",
            ])
            .arg(path)
            .output()
            .ok()
            .filter(|out| out.status.success())
            .and_then(|out| String::from_utf8(out.stdout).ok())
            .and_then(|out| out.trim().parse::<f64>().ok())
            .filter(|v| v.is_finite() && *v > 0.)
            .map(|v| (v * 1000.) as u64)
        };
        Ok(())
    }

    fn ensure_mixer(&mut self) -> Result<(), Box<dyn Error>> {
        if self.mixer_context.is_none() {
            let context = mixer::init(InitFlag::OGG)?;
            mixer::open_audio(48000, AUDIO_S16SYS, 2, 1024)?;
            mixer::reserve_channels(3);
            self.mixer_context = Some(context);
            self.apply_volumes();
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
        self.voice_started = Some(std::time::Instant::now());
        self.voice_offset_ms = 0;
        Ok(())
    }

    fn stop_voice(&mut self) {
        if self.mixer_context.is_some() {
            Channel(0).halt();
        }
        self.voice = None;
        self.voice_path = None;
        self.voice_started = None;
        self.voice_offset_ms = 0;
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
        self.music_started = None;
        self.music_offset_ms = 0;
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
        self.stop_replay();
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
    (0..=u8::MAX).find_map(|index| {
        menu_rect(vm, usize::from(index))
            .filter(|area| area.contains_point((x, y)))
            .map(|_| usize::from(index))
    })
}

fn scale_mouse_range(range: vm::MouseRange, size: (u32, u32)) -> Rect {
    let left = range.left.clamp(0, WIDTH as i32) as u32;
    let top = range.top.clamp(0, HEIGHT as i32) as u32;
    let right = range.right.clamp(left as i32, WIDTH as i32) as u32;
    let bottom = range.bottom.clamp(top as i32, HEIGHT as i32) as u32;
    let x = left * size.0 / WIDTH;
    let y = top * size.1 / HEIGHT;
    let right = right * size.0 / WIDTH;
    let bottom = bottom * size.1 / HEIGHT;
    Rect::new(
        x as i32,
        y as i32,
        right.saturating_sub(x).max(1),
        bottom.saturating_sub(y).max(1),
    )
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

fn keyboard_direction(key: Keycode) -> Option<usize> {
    match key {
        Keycode::Up => Some(0),
        Keycode::Down => Some(1),
        Keycode::Left => Some(2),
        Keycode::Right => Some(3),
        _ => None,
    }
}

fn draw(
    canvas: &mut Canvas<Window>,
    background: &Texture,
    preview: bool,
    hover: Option<usize>,
    vm: &Vm,
) -> Result<(), Box<dyn Error>> {
    canvas.set_draw_color(Color::BLACK);
    canvas.clear();
    canvas.copy(background, None, None)?;
    if !preview && let Some(index) = hover {
        canvas.set_draw_color(Color::RGB(255, 225, 30));
        if let Some(area) = menu_rect(vm, index) {
            canvas.draw_rect(area)?;
        }
    }
    Ok(())
}

fn play_video(
    data_dir: &Path,
    name: &str,
    width: i32,
    height: i32,
    ui: &mut VideoUi,
) -> Result<(), Box<dyn Error>> {
    if cfg!(target_os = "android") {
        println!("Skipping startup video on Android");
        return Ok(());
    }
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
    Mask {
        pixels: image::GrayImage,
        reversed: bool,
    },
    Blinds {
        vertical: bool,
        stripe: u32,
        rect: [i32; 4],
        color: [u8; 3],
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
            TransitionEffect::Mask { pixels, reversed } => {
                let mut frame = self.from.clone();
                let width = frame.width();
                let height = frame.height();
                for (x, y, pixel) in frame.enumerate_pixels_mut() {
                    // Effect maps are 320x240 and the game is 640x480. Use
                    // nearest-neighbour scaling so every original mask cell
                    // retains its threshold, including at non-native sizes.
                    let mx = (u64::from(x) * u64::from(pixels.width()) / u64::from(width)) as u32;
                    let my = (u64::from(y) * u64::from(pixels.height()) / u64::from(height)) as u32;
                    let value = pixels.get_pixel(mx, my)[0];
                    let threshold = if *reversed { 255 - value } else { value };
                    if f64::from(threshold) < progress * 256.0 {
                        *pixel = *self.to.get_pixel(x, y);
                    }
                }
                frame
            }
            TransitionEffect::Blinds {
                vertical,
                stripe,
                rect,
                color,
            } => {
                let mut frame = self.to.clone();
                let [rx, ry, rw, rh] = *rect;
                let span = if *vertical { rw } else { rh }.max(0) as u32;
                let pitch = (*stripe).max(1).min(span.max(1));
                let advanced = (f64::from(pitch) * progress).floor() as i32;
                let color = image::Rgba([color[0], color[1], color[2], 255]);
                for cell in (0..span).step_by(pitch as usize) {
                    let start = cell as i32 + advanced;
                    let end = (cell as i32 + pitch as i32).min(span as i32);
                    let start = start.min(end);
                    for axis in start..end {
                        if *vertical {
                            for y in ry.max(0)..(ry + rh).min(frame.height() as i32) {
                                let x = rx + axis;
                                if x >= 0 && x < frame.width() as i32 {
                                    frame.put_pixel(x as u32, y as u32, color);
                                }
                            }
                        } else {
                            let y = ry + axis;
                            if y >= 0 && y < frame.height() as i32 {
                                for x in rx.max(0)..(rx + rw).min(frame.width() as i32) {
                                    if x >= 0 {
                                        frame.put_pixel(x as u32, y as u32, color);
                                    }
                                }
                            }
                        }
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
        ui.sync_cursor(vm, None)?;
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
            stripe: 32,
            rect: [0, 0, from.width() as i32, from.height() as i32],
            color: [0, 0, 0],
        },
        TransitionEffect::Blinds {
            vertical: true,
            stripe: 32,
            rect: [0, 0, from.width() as i32, from.height() as i32],
            color: [0, 0, 0],
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

fn demo_masks(data_dir: &Path, ui: &mut VideoUi, vm: &mut Vm) -> Result<(), Box<dyn Error>> {
    let root = data_dir.join("PNG-images/GGD");
    let from = image::open(root.join("BG01AC.GG0.png"))?.to_rgba8();
    let to = image::open(root.join("BG32AC.GG0.png"))?.to_rgba8();
    let mut preview = RgbaImage::new(320 * 6, 240 * 7);
    for (row, name) in [
        "EFF01.BIN",
        "EFF03.BIN",
        "EFF04.BIN",
        "EFF17.BIN",
        "SEF03.BIN",
        "SEF04.BIN",
        "SEF05.BIN",
    ]
    .into_iter()
    .enumerate()
    {
        let pixels = ui
            .masks
            .get(name)?
            .ok_or("mask demo needs the original DATA cabinet")?;
        for (direction, reversed) in [false, true].into_iter().enumerate() {
            let mut transition = Transition::new(from.clone(), to.clone(), 500);
            transition.effect = TransitionEffect::Mask {
                pixels: pixels.clone(),
                reversed,
            };
            for (column, ms) in [125, 250, 375].into_iter().enumerate() {
                transition.elapsed = Duration::from_millis(ms);
                let image = image::imageops::resize(
                    &transition.frame(),
                    320,
                    240,
                    image::imageops::FilterType::Nearest,
                );
                image::imageops::replace(
                    &mut preview,
                    &image,
                    ((direction * 3 + column) * 320) as i64,
                    (row * 240) as i64,
                );
            }
            transition.elapsed = Duration::ZERO;
            println!("Mask demo: {name} direction={direction}");
            play_transition(ui, vm, transition)?;
            if ui.quit_requested {
                return Ok(());
            }
        }
    }
    if let Some(path) = std::env::var_os("IKURA_DUMP_MASKS") {
        preview.save(path)?;
    }
    println!("Mask demo completed: 7 maps, both directions");
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
            font_style: vm.font_style(),
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
    let translated = format!(
        "{}.ggd.png",
        name.split('.').next().unwrap_or(name).to_ascii_lowercase()
    );
    let banks: &[&str] = if matches!(
        std::env::var("IKURA_LANG").as_deref(),
        Ok("zh") | Ok("zh-CN")
    ) {
        &["ggd_sc", "GGD", "GGD2"]
    } else {
        &["GGD", "GGD2"]
    };
    for bank in banks {
        let dir = data_dir.join("PNG-images").join(bank);
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let file_name = entry.file_name();
            let file_name = file_name.to_string_lossy();
            if file_name.to_ascii_lowercase()
                == if *bank == "ggd_sc" {
                    translated.as_str()
                } else {
                    wanted.as_str()
                }
            {
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
            ui.sync_cursor(vm, None)?;
        }
        let Some((script, offset)) = vm.position() else {
            if let Some(ui) = ui.as_deref_mut() {
                ui.quit_requested = true;
            }
            return Ok((transition, RunOutcome::Finished));
        };
        if (stop_at_story && script == "SNR00") || is_title_input(vm) {
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
            Step::ShowSurface {
                slot,
                duration_ms,
                effect_kind,
                direction,
                parameter,
                mask,
            } => {
                let before = graphics.screen().clone();
                graphics.show_surface(slot)?;
                println!(
                    "VM {position}: GGE slot={slot} mask={mask} direction={direction} parameter={parameter} ({duration_ms}ms)"
                );
                if duration_ms > 0 {
                    let mut effect =
                        Transition::new(before, graphics.screen().clone(), duration_ms);
                    if effect_kind == 0 && parameter == 2 && matches!(direction, 0 | 1) {
                        if let Some(ui) = ui.as_deref_mut()
                            && let Some(pixels) = ui.masks.get(&mask)?
                        {
                            effect.effect = TransitionEffect::Mask {
                                pixels,
                                reversed: direction == 1,
                            };
                        }
                    } else {
                        eprintln!(
                            "GGE kind {effect_kind}/parameter {parameter}/direction {direction} uses a cross-fade fallback"
                        );
                    }
                    dispatch_transition(&mut ui, vm, &mut transition, effect)?;
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
                    effect_color: 0,
                    color_cut: 0,
                    extension: [0; 4],
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
                    ui.page_voices.push(path.clone());
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
            Step::MouseRange(range) => {
                if let Some(ui) = ui.as_deref_mut() {
                    let bounds =
                        range.map(|range| scale_mouse_range(range, ui.canvas.window().size()));
                    ui.canvas
                        .window()
                        .set_mouse_rect(bounds)
                        .map_err(|error| format!("IMS could not set mouse range: {error}"))?;
                    ui.mouse_range = Some(range);
                }
            }
            Step::MousePosition { x, y } => {
                if let Some(ui) = ui.as_deref_mut() {
                    ui.warp_mouse(x, y);
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
                        "VM {position}: GP {} uses cross-fade approximation; extra parameters=[{}, {}, {}, {}, {}, {}]",
                        operation.command,
                        operation.effect_color,
                        operation.color_cut,
                        operation.extension[0],
                        operation.extension[1],
                        operation.extension[2],
                        operation.extension[3],
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
                        15..=17 => {
                            let packed = operation.effect_color;
                            let color = [
                                ((packed & 0x3f) * 4) as u8,
                                (((packed >> 6) & 0x3f) * 4) as u8,
                                (((packed >> 12) & 0x3f) * 4) as u8,
                            ];
                            TransitionEffect::Blinds {
                                vertical: operation.command != 15,
                                stripe: if operation.command == 17 {
                                    operation.width.max(1) as u32
                                } else {
                                    32
                                },
                                rect: [
                                    operation.destination_x,
                                    operation.destination_y,
                                    operation.width,
                                    operation.height,
                                ],
                                color,
                            }
                        }
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
                if let Some(ui) = ui.as_deref_mut()
                    && !ui.wait_script(vm, 16)?
                {
                    return Ok((transition, RunOutcome::Reached));
                }
                vm.advance_clock(16);
                // A timer poll is making progress even if its PC cycles.
                history.clear();
                last_cycle = None;
                repeats = 0;
            }
            Step::Wait(milliseconds) => {
                println!("VM {position}: WRF wait {milliseconds}ms");
                if let Some(ui) = ui.as_deref_mut()
                    && !ui.wait_script(vm, milliseconds)?
                {
                    return Ok((transition, RunOutcome::Reached));
                }
                vm.advance_clock(milliseconds);
            }
            Step::Finished => {
                if let Some(ui) = ui.as_deref_mut() {
                    ui.quit_requested = true;
                }
                return Ok((transition, RunOutcome::Finished));
            }
            Step::ControlFlow => {}
        }
    }
    println!(
        "VM at {:?} is waiting for external state after {STEP_BUDGET} instructions",
        vm.position()
    );
    Ok((transition, RunOutcome::Stalled))
}

pub fn run() -> Result<(), Box<dyn Error>> {
    run_with_args(std::env::args().skip(1))
}

fn run_with_args(mut args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
    let data_dir = if let Some(path) = args.next() {
        PathBuf::from(path)
    } else if cfg!(target_os = "android") {
        PathBuf::from(sdl2::filesystem::pref_path("ikura", "game")?).join("game_data")
    } else {
        return Err("usage: cargo run -- <game_data directory> [scripts.json]".into());
    };
    let script_path = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            if cfg!(target_os = "android") {
                data_dir.join("scripts.json")
            } else {
                PathBuf::from("data/scripts.json")
            }
        });
    let mode = args.next();
    let probe_save = mode.as_deref() == Some("--probe-save");
    let probe_story = probe_save || mode.as_deref() == Some("--probe-story");
    let probe_start = probe_story || mode.as_deref() == Some("--probe-start");
    let script_bytes = std::fs::read(&script_path)?;
    let chinese = matches!(
        std::env::var("IKURA_LANG").as_deref(),
        Ok("zh") | Ok("zh-CN")
    ) || mode.as_deref() == Some("--lang=zh-CN");
    let translation_bytes = if chinese {
        let path = std::env::var_os("IKURA_TRANSLATIONS")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                if cfg!(target_os = "android") {
                    data_dir.join("zh-CN.json")
                } else {
                    PathBuf::from("data/zh-CN.json")
                }
            });
        Some(std::fs::read(&path).map_err(|error| {
            format!(
                "Chinese text pack {} is unavailable: {error}",
                path.display()
            )
        })?)
    } else {
        None
    };
    let script_signature = save::script_signature(&script_bytes);
    let scripts: Scripts = serde_json::from_slice(&script_bytes)?;
    let save_root = std::env::var_os("IKURA_SAVE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let save_name = if chinese {
                "rust-saves-zh"
            } else {
                "rust-saves"
            };
            if cfg!(target_os = "android") {
                data_dir.parent().unwrap_or(&data_dir).join(save_name)
            } else {
                data_dir.join(save_name)
            }
        });
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
    if let Some(bytes) = &translation_bytes {
        let pack: TranslationPack = serde_json::from_slice(bytes)?;
        println!(
            "Chinese pack: {} pages, {} choices, {} chapters, {} names",
            pack.pages.len(),
            pack.choices.len(),
            pack.chapters.len(),
            pack.names.len()
        );
        vm.set_translations(pack);
    }
    let image_dir = data_dir.join("PNG-images/GGD");
    let mut graphics = Graphics::new(image_dir.clone(), WIDTH, HEIGHT);
    if chinese {
        graphics.set_localized_root(data_dir.join("PNG-images/ggd_sc"));
    }
    let mut video_ui = if probe_start {
        None
    } else {
        Some(VideoUi::new(&data_dir)?)
    };
    if let Some(ui) = &mut video_ui {
        ui.settings = settings::Settings::load(&save_root);
        if let Err(e) = ui.apply_settings() {
            eprintln!("Display setting unavailable: {e}");
            ui.settings.fullscreen = false;
        }
    }
    if mode.as_deref() == Some("--probe-audio-ui") {
        let ui = video_ui.as_mut().ok_or("SDL window missing")?;
        ui.settings.music = 23;
        ui.settings.sound = 47;
        ui.settings.voice = 65;
        ui.ensure_mixer()?;
        ui.apply_volumes();
        assert_eq!(Music::get_volume(), 23 * 128 / 100);
        assert_eq!(Channel(1).get_volume(), 47 * 128 / 100);
        assert_eq!(Channel(0).get_volume(), 65 * 128 / 100);
        assert_eq!(Channel(2).get_volume(), 65 * 128 / 100);
        let voice = data_dir.join("OGG-audio/VOICE2/MSM2712.OGG");
        ui.start_music(&data_dir.join("OGG-audio/WMSC/25.OGG"))?;
        ui.play_voice(&voice)?;
        ui.pause_dialogue_voice();
        assert!(Channel(0).is_paused());
        ui.queue_replay(
            vec![PathBuf::from("OGG-audio/VOICE2/MSM2712.OGG")],
            &data_dir,
        )?;
        assert!(Channel(2).is_playing());
        assert!(Channel(0).is_paused());
        ui.stop_replay();
        ui.resume_dialogue_voice();
        assert!(!Channel(0).is_paused());
        ui.restore_voice(&voice, 500)?;
        assert!(ui.voice_playing());
        assert!(ui.audio_state().voice_position_ms >= 500);
        Music::set_pos(1.0)?;
        println!(
            "Audio UI probe passed: independent volumes, isolated history replay, pause/resume, voice/music seeking"
        );
        return Ok(());
    }
    if mode.as_deref() == Some("--demo-animation") {
        demo_animations(
            &data_dir,
            video_ui.as_mut().ok_or("SDL window missing")?,
            &mut vm,
        )?;
        return Ok(());
    }
    if mode.as_deref() == Some("--demo-masks") {
        demo_masks(
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
            let choice_sequence = std::env::var("IKURA_PROBE_CHOICES")
                .ok()
                .map(|value| {
                    value
                        .split(',')
                        .filter(|item| !item.trim().is_empty())
                        .map(|item| item.trim().parse::<u8>())
                        .collect::<Result<Vec<_>, _>>()
                })
                .transpose()?;
            let mut choice_index = 0usize;
            let mut visited_scripts = std::collections::BTreeSet::new();
            let frame_budget = std::env::var("IKURA_PROBE_FRAMES")
                .ok()
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0);
            let mut frames_left = frame_budget;
            let mut page_count = 1;
            let mut stop_reason = "page_limit";
            while page_count < page_limit {
                if let Some((script, _)) = vm.position() {
                    visited_scripts.insert(script.to_owned());
                }
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
                    let selected = choice_sequence
                        .as_ref()
                        .and_then(|choices| choices.get(choice_index))
                        .copied()
                        .unwrap_or(choice_value);
                    vm.choose(selected)?;
                    println!("Story probe choice #{choice_index}: selected {selected}");
                    choice_index += 1;
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
                } else if is_title_input(&vm) {
                    visited_scripts.insert("TITLE".into());
                    stop_reason = "returned_to_title";
                    break;
                } else if vm.waiting_for_menu_input() {
                    // Credits and a few scene scripts poll clicks without
                    // showing a dialogue page. The next probe iteration
                    // supplies that click just like the GUI event loop.
                    continue;
                } else {
                    println!("Story probe paused at {:?}", vm.position());
                    stop_reason = "no_text_boundary";
                    break;
                }
            }
            println!(
                "Story probe summary: pages={page_count} choices={choice_index} reason={stop_reason} position={:?}",
                vm.position()
            );
            println!(
                "Story probe visited scripts: {}",
                visited_scripts.into_iter().collect::<Vec<_>>().join(", ")
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
    let menu_text = TextRenderer::new()?;
    if cfg!(target_os = "android") {
        sdl2::log::log("Ikura: title screen and menu font are ready");
    }
    #[cfg(target_os = "android")]
    unsafe {
        unsafe extern "C" {
            fn SDL_AndroidSendMessage(command: u32, param: i32) -> i32;
        }
        SDL_AndroidSendMessage(0x8001, 0);
    }
    let mut save_menu: Option<save_menu::SaveMenu> = None;
    let mut menu_started = std::time::Instant::now();
    let mut notice: Option<(String, std::time::Instant)> = None;
    let creator = ui.canvas.texture_creator();
    let mut background = texture_from_rgba(&creator, graphics.screen())?;
    let mut hover = None;
    let mut preview = false;
    let mut text_renderer: Option<TextRenderer> = None;
    let mut story_frame = StoryFrame::new();
    let mut story_history: Vec<history::Entry> = Vec::new();
    let mut history_panel = history::Panel::default();
    let mut settings_panel: Option<settings::Panel> = None;
    let mut autosave_position: Option<(String, usize)> = None;
    let mut autosave_at = std::time::Instant::now() - Duration::from_secs(30);
    let mut history_open = false;
    let mut touch_menu_started: Option<std::time::Instant> = None;
    let mut touch_menu_freezes_dialogue = false;
    let mut touch_menu_paused_voice = false;

    let mut history_background: Option<RgbaImage> = None;
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
        ui.sync_cursor(
            &vm,
            (preview && auto_mode && !choosing && save_menu.is_none()).then_some(4),
        )?;
        let mut pending_events = std::mem::take(&mut ui.deferred_events);
        if next_transition.is_some() || !pending_events.is_empty() {
            pending_events.extend(ui.events.poll_iter());
        } else {
            pending_events.extend(ui.events.wait_timeout_iter(16));
        }
        let text_interval = ui.settings.text_interval(vm.text_interval_ms());
        let auto_delay = ui.settings.auto_delay(dialogue_chars(&vm))
            + dialogue_chars(&vm) as u64 * text_interval;
        let advance = preview
            && touch_menu_started.is_none()
            && save_menu.is_none()
            && settings_panel.is_none()
            && !history_open
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
            if matches!(event, Event::Quit { .. }) {
                break 'running;
            }
            if cfg!(target_os = "android")
                && matches!(event, Event::KeyDown { keycode: Some(Keycode::F11), repeat: false, .. })
            {
                if let Some(started) = touch_menu_started.take() {
                    if touch_menu_freezes_dialogue {
                        let paused = started.elapsed();
                        page_started += paused;
                        reveal_started += paused;
                    }
                    if touch_menu_paused_voice {
                        ui.resume_dialogue_voice();
                    }
                } else {
                    touch_menu_started = Some(std::time::Instant::now());
                    touch_menu_freezes_dialogue = save_menu.is_none()
                        && settings_panel.is_none() && !history_open;
                    touch_menu_paused_voice = ui.modal_voice_paused.is_none();
                    if touch_menu_paused_voice {
                        ui.pause_dialogue_voice();
                    }
                }
                continue;
            }
            if touch_menu_started.is_some() {
                continue;
            }
            if let Some(panel) = &mut settings_panel {
                let previous = ui.settings.clone();
                let close = panel.handle(&event, &mut ui.settings);
                if previous != ui.settings {
                    if let Err(e) = ui.apply_settings() {
                        ui.settings = previous;
                        ui.apply_volumes();
                        panel.status = format!("Could not apply: {e}");
                    }
                    reveal.rendered = None;
                }
                if close {
                    match ui.settings.store(&save_root) {
                        Ok(()) => {
                            notice = Some(("Settings saved".into(), std::time::Instant::now()))
                        }
                        Err(e) => {
                            notice = Some((
                                format!("Settings could not be saved: {e}"),
                                std::time::Instant::now(),
                            ))
                        }
                    }
                    settings_panel = None;
                    ui.resume_dialogue_voice();
                    let paused = menu_started.elapsed();
                    page_started += paused;
                    reveal_started += paused;
                }
                continue;
            }
            if history_open {
                match history_panel.handle(&event) {
                    history::Action::Close => {
                        history_open = false;
                        history_background = None;
                        ui.stop_replay();
                        ui.resume_dialogue_voice();
                        let paused = menu_started.elapsed();
                        page_started += paused;
                        reveal_started += paused;
                    }
                    history::Action::Replay(paths) => {
                        history_panel.status.clear();
                        if let Err(e) = ui.queue_replay(paths, &data_dir) {
                            ui.stop_replay();
                            history_panel.status = format!("Voice replay failed: {e}");
                            notice = Some((
                                format!("Voice replay failed: {e}"),
                                std::time::Instant::now(),
                            ));
                        }
                    }
                    history::Action::None => {}
                }
                continue;
            }
            if save_menu.is_none()
                && next_transition.is_none()
                && (matches!(
                    event,
                    Event::KeyDown {
                        keycode: Some(Keycode::F10),
                        repeat: false,
                        ..
                    }
                ) || matches!(event,Event::MouseButtonDown{mouse_btn:MouseButton::Left,x,y,..} if Rect::new(530,8,104,26).contains_point((x,y))))
            {
                auto_mode = false;
                ui.pause_dialogue_voice();
                menu_started = std::time::Instant::now();
                let mut frame = graphics.screen().clone();
                if let Some(page) = vm.text_page() {
                    let layout = story_frame.layout(&graphics, &data_dir, &vm);
                    frame =
                        menu_text.draw_page(&frame, page.speaker.as_deref(), &page.body, &layout);
                }
                settings_panel = Some(settings::Panel::new(frame));
                continue;
            }
            let mut target_slot = save::Slot::Quick;
            let event = if let Some(menu) = &mut save_menu {
                match menu.handle(&event) {
                    save_menu::Action::None => continue,
                    save_menu::Action::Delete(slot) => {
                        match save::delete_slot(&save_root, slot) {
                            Ok(()) => {
                                menu.status = format!("Deleted: {}", save_menu::slot_name(slot))
                            }
                            Err(e) => menu.status = format!("Delete failed: {e}"),
                        }
                        menu.refresh(&save_root, script_signature);
                        continue;
                    }
                    save_menu::Action::Close => {
                        save_menu = None;
                        let paused = menu_started.elapsed();
                        page_started += paused;
                        reveal_started += paused;
                        continue;
                    }
                    action @ (save_menu::Action::Save(slot) | save_menu::Action::Load(slot)) => {
                        target_slot = slot;
                        menu.status = if matches!(action, save_menu::Action::Save(_)) {
                            "Saving..."
                        } else {
                            "Loading..."
                        }
                        .into();
                        let texture = texture_from_rgba(&creator, &menu.draw(&menu_text))?;
                        draw(&mut ui.canvas, &texture, true, None, &vm)?;
                        ui.canvas.present();
                        Event::KeyDown {
                            timestamp: 0,
                            window_id: ui.canvas.window().id(),
                            keycode: Some(if matches!(action, save_menu::Action::Save(_)) {
                                Keycode::F5
                            } else {
                                Keycode::F9
                            }),
                            scancode: None,
                            keymod: sdl2::keyboard::Mod::NOMOD,
                            repeat: false,
                        }
                    }
                }
            } else {
                event
            };
            if let Event::KeyDown {
                keycode: Some(key),
                repeat: false,
                ..
            } = &event
                && let Some(direction) = keyboard_direction(*key)
            {
                if preview && choosing {
                    let next = match choice_hover {
                        Some(current) => vm.key_neighbor(Some(current), direction),
                        None => vm.key_default(),
                    };
                    if let Some(value) = next.filter(|value| {
                        vm.choice_options()
                            .iter()
                            .any(|option| option.value == *value)
                    }) {
                        choice_hover = Some(value);
                    }
                } else if !preview && vm.waiting_for_menu_input() {
                    hover = Some(match hover {
                        Some(current) => vm.key_neighbor(Some(current as u8), direction),
                        None => vm.key_default(),
                    })
                    .flatten()
                    .map(usize::from);
                }
                continue;
            }
            let event = match event {
                Event::KeyDown {
                    keycode: Some(Keycode::Return),
                    repeat: false,
                    ..
                } if preview && choosing && choice_hover.or(vm.key_default()).is_some() => {
                    let selected = choice_hover.or(vm.key_default());
                    let option = vm
                        .choice_options()
                        .iter()
                        .find(|option| Some(option.value) == selected)
                        .expect("keyboard choice selection is present");
                    Event::MouseButtonDown {
                        timestamp: 0,
                        window_id: ui.canvas.window().id(),
                        which: 0,
                        mouse_btn: MouseButton::Left,
                        clicks: 1,
                        x: option.x + option.width / 2,
                        y: option.y + option.height / 2,
                    }
                }
                Event::KeyDown {
                    keycode: Some(Keycode::Return),
                    repeat: false,
                    ..
                } if !preview
                    && vm.waiting_for_menu_input()
                    && hover.or(vm.key_default().map(usize::from)).is_some() =>
                {
                    let selected = hover.or(vm.key_default().map(usize::from)).unwrap();
                    let rect = menu_rect(&vm, selected).expect("selected title area exists");
                    Event::MouseButtonDown {
                        timestamp: 0,
                        window_id: ui.canvas.window().id(),
                        which: 0,
                        mouse_btn: MouseButton::Left,
                        clicks: 1,
                        x: rect.x + rect.width() as i32 / 2,
                        y: rect.y + rect.height() as i32 / 2,
                    }
                }
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
                } if !preview && is_title_input(&vm) && menu_at(&vm, x, y) == Some(1) => {
                    Event::KeyDown {
                        timestamp: 0,
                        window_id: ui.canvas.window().id(),
                        keycode: Some(Keycode::F8),
                        scancode: None,
                        keymod: sdl2::keyboard::Mod::NOMOD,
                        repeat: false,
                    }
                }
                event => event,
            };
            match event {
                Event::Quit { .. } => break 'running,
                Event::KeyDown {
                    keycode: Some(code @ (Keycode::F6 | Keycode::F8)),
                    repeat: false,
                    ..
                } if next_transition.is_none() => {
                    auto_mode = false;
                    let mut frame = graphics.screen().clone();
                    if preview && !dialogue_hidden && vm.text_visible() {
                        if choosing {
                            frame = menu_text.draw_choices(&frame, vm.choice_options(), None);
                        } else if let Some(page) = vm.text_page() {
                            let layout = story_frame.layout(&graphics, &data_dir, &vm);
                            frame = menu_text.draw_page(
                                &frame,
                                page.speaker.as_deref(),
                                &page.body,
                                &layout,
                            );
                        }
                    }
                    save_menu = Some(save_menu::SaveMenu::new(
                        if code == Keycode::F6 {
                            save_menu::Mode::Save
                        } else {
                            save_menu::Mode::Load
                        },
                        &save_root,
                        script_signature,
                        frame,
                        preview && vm.can_save(),
                        preview,
                    ));
                    menu_started = std::time::Instant::now();
                }
                Event::KeyDown {
                    keycode: Some(Keycode::A),
                    repeat: false,
                    ..
                } if preview && !choosing => {
                    auto_mode = !auto_mode;
                    if cfg!(target_os = "android") {
                        notice = Some((
                            if auto_mode { "自动阅读已开启" } else { "自动阅读已关闭" }.into(),
                            std::time::Instant::now(),
                        ));
                    }
                    page_started = std::time::Instant::now();
                    ui.canvas.window_mut().set_title(if auto_mode {
                        "Kazoku Keikaku - AUTO (A to stop)"
                    } else {
                        "Kazoku Keikaku - manual"
                    })?;
                }
                Event::KeyDown {
                    keycode: Some(Keycode::B),
                    repeat: false,
                    ..
                } if preview && !choosing && save_menu.is_none() => {
                    auto_mode = false;
                    let mut frame = graphics.screen().clone();
                    if let Some(page) = vm.text_page() {
                        let layout = story_frame.layout(&graphics, &data_dir, &vm);
                        frame = menu_text.draw_page(
                            &frame,
                            page.speaker.as_deref(),
                            &page.body,
                            &layout,
                        );
                    }
                    history_background = Some(frame);
                    history_panel = history::Panel::default();
                    ui.pause_dialogue_voice();
                    menu_started = std::time::Instant::now();
                    history_open = true;
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
                    let audio = ui.audio_state();
                    let path = save::slot_path(&save_root, target_slot);
                    let mut frame = graphics.screen().clone();
                    if choosing {
                        frame = menu_text.draw_choices(&frame, vm.choice_options(), None);
                    } else if let Some(page) = vm.text_page() {
                        let layout = story_frame.layout(&graphics, &data_dir, &vm);
                        frame = menu_text.draw_page(
                            &frame,
                            page.speaker.as_deref(),
                            &page.body,
                            &layout,
                        );
                    }
                    match save::write_with_session(
                        &path,
                        script_signature,
                        &vm,
                        &graphics,
                        audio,
                        &frame,
                        &history::Session {
                            history: story_history.clone(),
                        },
                    ) {
                        Ok(()) => {
                            println!(
                                "Quick save: {} (chapter: {})",
                                path.display(),
                                vm.save_title().unwrap_or("untitled")
                            );
                            let message = format!("Saved: {}", save_menu::slot_name(target_slot));
                            if let Some(menu) = &mut save_menu {
                                menu.refresh(&save_root, script_signature);
                                menu.status = message.clone();
                            }
                            notice = Some((message, std::time::Instant::now()));
                            ui.canvas
                                .window_mut()
                                .set_title("Kazoku Keikaku - save stored")?;
                        }
                        Err(error) => {
                            eprintln!("Quick save failed: {error}");
                            let message = if !vm.can_save() {
                                "Saving is disabled at this point."
                            } else {
                                "Save failed. Check storage space and folder permissions."
                            };
                            if let Some(menu) = &mut save_menu {
                                menu.status = message.into();
                            }
                            notice = Some((message.into(), std::time::Instant::now()));
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
                } => match save::read_session_with_backup(
                    &save::load_path(&save_root, target_slot),
                    script_signature,
                    &mut vm,
                    &mut graphics,
                ) {
                    Ok((audio, session, recovered)) => {
                        story_history = session.history;
                        ui.page_voices.clear();
                        autosave_position = vm.position().map(|(n, o)| (n.to_owned(), o));
                        autosave_at = std::time::Instant::now();
                        ui.stop_music();
                        ui.stop_sound();
                        ui.stop_voice();
                        let mut audio_failed = false;
                        if let Some(path) = audio.music.filter(|_| vm.should_restore_music())
                            && let Err(error) = ui.start_music(&path)
                        {
                            audio_failed = true;
                            eprintln!("Loaded save, music unavailable: {error}");
                        } else if ui.music_path.is_some() && audio.music_position_ms > 0 {
                            if let Err(e) = Music::set_pos(audio.music_position_ms as f64 / 1000.) {
                                audio_failed = true;
                                eprintln!("Music seek failed: {e}");
                            } else {
                                ui.music_offset_ms = audio.music_position_ms;
                                ui.music_started = Some(std::time::Instant::now());
                            }
                        }
                        if let Some(path) = audio.voice
                            && let Err(error) = ui.restore_voice(&path, audio.voice_position_ms)
                        {
                            audio_failed = true;
                            eprintln!("Loaded save, voice unavailable: {error}");
                        }
                        preview = true;
                        save_menu = None;
                        ui.deferred_events.clear();
                        notice = Some((
                            format!(
                                "Loaded: {}{}",
                                save_menu::slot_name(target_slot),
                                if audio_failed {
                                    " (audio unavailable)"
                                } else if recovered {
                                    " (backup recovered)"
                                } else {
                                    ""
                                }
                            ),
                            std::time::Instant::now(),
                        ));
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
                        println!(
                            "Quick load: {:?} (chapter: {})",
                            vm.position(),
                            vm.save_title().unwrap_or("untitled")
                        );
                        ui.canvas
                            .window_mut()
                            .set_title("Kazoku Keikaku - save restored")?;
                    }
                    Err(error) => {
                        eprintln!("Quick load failed: {error}");
                        let message = if error.to_string().contains("different script") {
                            "This save belongs to different game data."
                        } else {
                            "Could not load save: missing, damaged, or unavailable assets."
                        };
                        if let Some(menu) = &mut save_menu {
                            menu.status = message.into();
                        }
                        notice = Some((message.into(), std::time::Instant::now()));
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
                        record_history_page(&mut story_history, &vm, &mut ui, &data_dir);
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
                    if let Some(index) = menu_at(&vm, x, y).filter(|_| vm.waiting_for_menu_input())
                    {
                        let is_title = is_title_input(&vm);
                        if index == 3 && is_title {
                            break 'running;
                        }
                        if index == 0 && is_title {
                            // TITLE clears its BGM registers before LS SNR00,
                            // but that branch contains no standalone MS/MF.
                            // End the menu track when starting a new game so
                            // the unvoiced opening cannot inherit title music.
                            ui.stop_music();
                            story_history.clear();
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
                        record_history_page(&mut story_history, &vm, &mut ui, &data_dir);
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
                            ui.settings.text_interval(vm.text_interval_ms()),
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
                    record_history_page(&mut story_history, &vm, &mut ui, &data_dir);
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
            if ui.quit_requested {
                break 'running;
            }
            if preview && is_title_input(&vm) {
                break;
            }
        }
        if touch_menu_started.is_some() {
            std::thread::sleep(Duration::from_millis(16));
            continue;
        }
        if let Some(panel) = &settings_panel {
            let frame = panel.draw(&menu_text, &ui.settings);
            let texture = texture_from_rgba(&creator, &frame)?;
            draw(&mut ui.canvas, &texture, true, None, &vm)?;
            ui.canvas.present();
            continue;
        }
        if history_open {
            if let Err(e) = ui.tick_replay() {
                ui.stop_replay();
                history_panel.status = format!("Voice replay failed: {e}");
                notice = Some((
                    format!("Voice replay failed: {e}"),
                    std::time::Instant::now(),
                ));
            }
            if let Some(frame) = &history_background {
                let frame = history_panel.draw(&menu_text, frame, &story_history);
                let texture = texture_from_rgba(&creator, &frame)?;
                draw(&mut ui.canvas, &texture, true, None, &vm)?;
                ui.canvas.present();
                continue;
            }
        }
        if let Some(menu) = &save_menu {
            let frame = menu.draw(&menu_text);
            let texture = texture_from_rgba(&creator, &frame)?;
            draw(&mut ui.canvas, &texture, true, None, &vm)?;
            ui.canvas.present();
            std::thread::sleep(Duration::from_millis(4));
            continue;
        }
        if preview && is_title_input(&vm) {
            // The script has rebuilt TITLE and reached its menu poll. Refresh
            // the scene texture and discard the story UI before accepting menu
            // input; otherwise the final dialogue can cover the title screen.
            preview = false;
            choosing = false;
            choice_hover = None;
            hover = None;
            auto_mode = false;
            dialogue_hidden = false;
            next_transition = None;
            text_renderer = None;
            story_frame = StoryFrame::new();
            reveal = DialogueReveal::default();
            vm.clear_text_page();
            vm.set_selected(None);
            ui.deferred_events.clear();
            ui.stop_voice();
            ui.stop_sound();
            background = texture_from_rgba(&creator, graphics.screen())?;
            ui.canvas
                .window_mut()
                .set_title("Kazoku Keikaku - Rust prototype")?;
            println!("Story -> TITLE: restored menu input and title frame");
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
        if preview && ui.settings.autosave && vm.can_save() && next_transition.is_none() {
            let position = vm.position().map(|(n, o)| (n.to_owned(), o));
            if checkpoint_due(
                autosave_position.as_ref(),
                position.as_ref(),
                choosing,
                autosave_at.elapsed(),
            ) {
                let mut frame = graphics.screen().clone();
                if choosing {
                    frame = menu_text.draw_choices(&frame, vm.choice_options(), None);
                } else if let Some(page) = vm.text_page() {
                    let layout = story_frame.layout(&graphics, &data_dir, &vm);
                    frame =
                        menu_text.draw_page(&frame, page.speaker.as_deref(), &page.body, &layout);
                }
                match save::write_with_session(
                    &save::slot_path(&save_root, save::Slot::Auto),
                    script_signature,
                    &vm,
                    &graphics,
                    ui.audio_state(),
                    &frame,
                    &history::Session {
                        history: story_history.clone(),
                    },
                ) {
                    Ok(()) => {}
                    Err(e) => {
                        eprintln!("Automatic save failed: {e}");
                        notice = Some((
                            "Automatic save failed; check storage permissions".into(),
                            std::time::Instant::now(),
                        ));
                    }
                }
                autosave_position = position;
                autosave_at = std::time::Instant::now();
            }
        }
        vm.tick_frame();
        if preview
            && !choosing
            && next_transition.is_none()
            && (dialogue_hidden || !vm.text_visible())
        {
            let scene = texture_from_rgba(&creator, graphics.screen())?;
            draw(&mut ui.canvas, &scene, preview, hover, &vm)?;
        } else if choosing && !vm.choice_options().is_empty() {
            let frame =
                menu_text.draw_choices(graphics.screen(), vm.choice_options(), choice_hover);
            let overlay = texture_from_rgba(&creator, &frame)?;
            draw(&mut ui.canvas, &overlay, preview, hover, &vm)?;
        } else if preview && next_transition.is_none() && vm.text_page().is_some() {
            let total = dialogue_chars(&vm);
            if ui.skip_held() {
                reveal.completed = true;
            }
            let visible = reveal.visible(
                total,
                ui.settings.text_interval(vm.text_interval_ms()),
                reveal_started.elapsed(),
            );
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
            draw(&mut ui.canvas, &background, preview, hover, &vm)?;
        } else {
            draw(&mut ui.canvas, &background, preview, hover, &vm)?;
        }
        if next_transition.is_none() {
            let mut button = RgbaImage::from_pixel(104, 26, image::Rgba([35, 48, 65, 235]));
            menu_text.draw_label(&mut button,
                if cfg!(target_os = "android") { "设置" } else { "Settings F10" },
                8, 5, 95, 12., [220, 230, 240]);
            let texture = texture_from_rgba(&creator, &button)?;
            ui.canvas
                .copy(&texture, None, Some(Rect::new(530, 8, 104, 26)))?;
        }
        if let Some((message, started)) = &notice
            && started.elapsed() < Duration::from_secs(4)
        {
            let mut banner = RgbaImage::from_pixel(600, 32, image::Rgba([25, 35, 50, 255]));
            menu_text.draw_label(&mut banner, message, 10, 6, 580, 15., [245, 230, 190]);
            let texture = texture_from_rgba(&creator, &banner)?;
            ui.canvas
                .copy(&texture, None, Some(Rect::new(20, 12, 600, 32)))?;
        }
        ui.canvas.present();
        if next_transition.is_none() {
            std::thread::sleep(Duration::from_millis(4));
        }
    }
    if let Err(error) = ui.settings.store(&save_root) {
        eprintln!("Settings could not be saved on exit: {error}");
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
    fn automatic_checkpoints_are_throttled_but_preserve_choice_and_chapter_boundaries() {
        let a = ("A".to_owned(), 100);
        let b = ("A".to_owned(), 200);
        let chapter = ("B".to_owned(), 100);
        assert!(checkpoint_due(None, Some(&a), false, Duration::ZERO));
        assert!(!checkpoint_due(
            Some(&a),
            Some(&a),
            true,
            Duration::from_secs(60)
        ));
        assert!(!checkpoint_due(
            Some(&a),
            Some(&b),
            false,
            Duration::from_secs(5)
        ));
        assert!(checkpoint_due(Some(&a), Some(&b), true, Duration::ZERO));
        assert!(checkpoint_due(
            Some(&a),
            Some(&chapter),
            false,
            Duration::ZERO
        ));
        assert!(checkpoint_due(
            Some(&a),
            Some(&b),
            false,
            Duration::from_secs(30)
        ));
    }

    #[test]
    fn ims_range_scales_to_resized_window_and_clamps_to_canvas() {
        let rect = scale_mouse_range(
            vm::MouseRange {
                left: -10,
                top: 120,
                right: 320,
                bottom: 600,
            },
            (1280, 960),
        );
        assert_eq!((rect.x(), rect.y()), (0, 240));
        assert_eq!((rect.width(), rect.height()), (640, 720));
    }

    #[test]
    fn wait_input_consumes_skip_keys_and_preserves_ui_commands() {
        let key = |code| Event::KeyDown {
            timestamp: 0,
            window_id: 1,
            keycode: Some(code),
            scancode: None,
            keymod: sdl2::keyboard::Mod::NOMOD,
            repeat: false,
        };
        for code in [Keycode::Space, Keycode::Return] {
            assert_eq!(wait_input(&key(code)), WaitInput::Skip);
        }
        assert_eq!(wait_input(&key(Keycode::Escape)), WaitInput::Quit);
        assert_eq!(wait_input(&Event::Quit { timestamp: 0 }), WaitInput::Quit);
        for code in [Keycode::A, Keycode::H, Keycode::F5, Keycode::F9] {
            assert_eq!(wait_input(&key(code)), WaitInput::Defer);
        }
        let click = Event::MouseButtonDown {
            timestamp: 0,
            window_id: 1,
            which: 0,
            mouse_btn: MouseButton::Left,
            clicks: 1,
            x: 10,
            y: 20,
        };
        assert_eq!(wait_input(&click), WaitInput::Skip);
        assert_eq!(
            wait_input(&Event::KeyUp {
                timestamp: 0,
                window_id: 1,
                keycode: Some(Keycode::Return),
                scancode: None,
                keymod: sdl2::keyboard::Mod::NOMOD,
                repeat: false
            }),
            WaitInput::Ignore
        );
    }

    #[test]
    fn script_return_to_title_stops_at_the_menu_input() {
        let mut scripts = Scripts::new();
        scripts.insert(
            "STORY".into(),
            vm::Script {
                code_start: 0,
                jumps: vec![],
                instructions: vec![
                    Instruction {
                        offset: 0,
                        opcode: 0x2B,
                        payload: vec![0, 3, 255, b'A', 0],
                    },
                    Instruction {
                        offset: 1,
                        opcode: 0x01,
                        payload: b"TITLE\0".to_vec(),
                    },
                ],
            },
        );
        scripts.insert(
            "TITLE".into(),
            vm::Script {
                code_start: 0x7B9,
                jumps: vec![],
                instructions: vec![Instruction {
                    offset: 0x7B9,
                    opcode: 0x85,
                    payload: vec![7, 0, 8, 0, 5, 1],
                }],
            },
        );
        let mut vm = Vm::new(scripts, "STORY").unwrap();
        let mut graphics = Graphics::new(PathBuf::new(), 1, 1);
        let (_, outcome) =
            run_until_input_or_story(&mut vm, &mut graphics, Path::new(""), false, None, false)
                .unwrap();
        assert_eq!(outcome, RunOutcome::Reached);
        assert!(is_title_input(&vm));
        vm.clear_text_page();
        vm.set_selected(None);
        assert!(vm.text_page().is_none());
        // TITLE's IG uses menu mode 1; a selection consumes that IG rather
        // than using the dialogue-only InputPoll mode 4.
        vm.set_selected(Some(0));
        assert_eq!(vm.step().unwrap(), Step::ControlFlow);
    }

    #[test]
    fn mask_transition_preserves_timing_direction_scaling_and_final_frame() {
        let before = RgbaImage::from_pixel(4, 2, image::Rgba([0, 0, 0, 255]));
        let after = RgbaImage::from_pixel(4, 2, image::Rgba([255, 255, 255, 255]));
        let mask = image::GrayImage::from_raw(2, 1, vec![0, 255]).unwrap();
        let mut effect = Transition::new(before.clone(), after.clone(), 1000);
        effect.effect = TransitionEffect::Mask {
            pixels: mask.clone(),
            reversed: false,
        };
        assert!(effect.frame() == before);
        effect.advance(Duration::from_millis(500));
        let halfway = effect.frame();
        assert_eq!(halfway.get_pixel(0, 0).0, [255, 255, 255, 255]);
        assert_eq!(halfway.get_pixel(1, 1).0, [255, 255, 255, 255]);
        assert_eq!(halfway.get_pixel(2, 0).0, [0, 0, 0, 255]);
        effect.effect = TransitionEffect::Mask {
            pixels: mask,
            reversed: true,
        };
        let reverse = effect.frame();
        assert_eq!(reverse.get_pixel(0, 0).0, [0, 0, 0, 255]);
        assert_eq!(reverse.get_pixel(2, 1).0, [255, 255, 255, 255]);
        assert!(effect.advance(Duration::from_millis(500)));
        assert!(effect.frame() == after);
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
    fn blinds_overlay_shrinking_color_slats_then_land_on_final_frame() {
        let from = RgbaImage::from_pixel(8, 8, image::Rgba([0, 0, 0, 255]));
        let to = RgbaImage::from_pixel(8, 8, image::Rgba([255, 255, 255, 255]));
        for vertical in [false, true] {
            let mut effect = Transition::new(from.clone(), to.clone(), 100);
            effect.effect = TransitionEffect::Blinds {
                vertical,
                stripe: 4,
                rect: [0, 0, 8, 8],
                color: [200, 10, 20],
            };
            effect.advance(Duration::from_millis(50));
            let frame = effect.frame();
            assert_eq!(frame.pixels().filter(|pixel| pixel[0] == 255).count(), 32);
            assert_eq!(
                frame
                    .get_pixel(if vertical { 2 } else { 0 }, if vertical { 0 } else { 2 })
                    .0,
                [200, 10, 20, 255]
            );
            assert_eq!(
                frame
                    .get_pixel(if vertical { 0 } else { 2 }, if vertical { 2 } else { 0 })
                    .0,
                [255, 255, 255, 255]
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
            effect_color: 0,
            color_cut: 0,
            extension: [0; 4],
        };
        assert_eq!(op.duration(), 1500);
        op.command = 1;
        assert_eq!(op.duration(), 0);
        op.command = 19;
        op.destination = -5;
        assert_eq!(op.duration(), 0);
    }
}
