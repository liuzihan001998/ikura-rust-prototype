//! Decode MPEG with FFmpeg and present it through the game's SDL window.

use sdl2::audio::AudioSpecDesired;
use sdl2::event::Event;
use sdl2::keyboard::Keycode;
use sdl2::pixels::{Color, PixelFormatEnum};
use sdl2::rect::Rect;
use sdl2::render::Canvas;
use sdl2::video::Window;
use sdl2::{AudioSubsystem, EventPump};
use std::error::Error;
use std::io::{BufReader, ErrorKind, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Playback {
    Finished,
    Skipped,
    Quit,
}

fn tool(name: &str, override_var: &str) -> std::ffi::OsString {
    std::env::var_os(override_var).unwrap_or_else(|| name.into())
}

fn frame_rate(path: &Path) -> Result<f64, Box<dyn Error>> {
    let output = Command::new(tool("ffprobe", "IKURA_FFPROBE"))
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=avg_frame_rate",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .output()?;
    if !output.status.success() {
        return Err(format!("ffprobe could not inspect {}", path.display()).into());
    }
    let rate = String::from_utf8(output.stdout)?;
    let rate = rate.trim();
    let fps = if let Some((numerator, denominator)) = rate.split_once('/') {
        numerator.parse::<f64>()? / denominator.parse::<f64>()?
    } else {
        rate.parse::<f64>()?
    };
    if !fps.is_finite() || !(1.0..=120.0).contains(&fps) {
        return Err(format!("invalid video frame rate {rate:?}").into());
    }
    Ok(fps)
}

// `chunks_exact(2)` is the stable spelling of the byte-pair chunking we need.
#[allow(clippy::chunks_exact_to_as_chunks)]
fn decode_audio(path: &Path) -> Result<Vec<i16>, Box<dyn Error>> {
    let output = Command::new(tool("ffmpeg", "IKURA_FFMPEG"))
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(path)
        .args([
            "-vn",
            "-ac",
            "2",
            "-ar",
            "48000",
            "-c:a",
            "pcm_s16le",
            "-f",
            "s16le",
            "-",
        ])
        .output()?;
    if !output.status.success() {
        return Err(format!("ffmpeg could not decode audio from {}", path.display()).into());
    }
    Ok(output
        .stdout
        .chunks_exact(2)
        .map(|bytes| i16::from_le_bytes([bytes[0], bytes[1]]))
        .collect())
}

pub fn play(
    canvas: &mut Canvas<Window>,
    events: &mut EventPump,
    audio: &AudioSubsystem,
    path: &Path,
    width: u32,
    height: u32,
) -> Result<Playback, Box<dyn Error>> {
    let fps = frame_rate(path)?;
    let samples = decode_audio(path)?;
    let audio_queue = audio.open_queue::<i16, _>(
        None,
        &AudioSpecDesired {
            freq: Some(48000),
            channels: Some(2),
            samples: Some(2048),
        },
    )?;
    audio_queue.queue_audio(&samples)?;

    let mut decoder = Command::new(tool("ffmpeg", "IKURA_FFMPEG"))
        .args(["-nostdin", "-v", "error", "-i"])
        .arg(path)
        .arg("-vf")
        .arg(format!("fps={fps},scale={width}:{height}"))
        .args(["-an", "-pix_fmt", "rgba", "-f", "rawvideo", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let stdout = decoder.stdout.take().ok_or("ffmpeg did not open stdout")?;
    let mut frames = BufReader::new(stdout);
    let mut frame = vec![0u8; width as usize * height as usize * 4];
    let creator = canvas.texture_creator();
    let mut texture = creator.create_texture_streaming(PixelFormatEnum::RGBA32, width, height)?;
    let target = Rect::new(
        (640 - width as i32) / 2,
        (480 - height as i32) / 2,
        width,
        height,
    );

    let mut result = Playback::Finished;
    let mut frame_index = 0u64;
    let mut started = None;
    let render_result: Result<(), Box<dyn Error>> = (|| {
        loop {
            for event in events.poll_iter() {
                match event {
                    Event::Quit { .. } => result = Playback::Quit,
                    Event::KeyDown {
                        keycode: Some(Keycode::Escape),
                        ..
                    } => result = Playback::Skipped,
                    _ => {}
                }
            }
            if result != Playback::Finished {
                break;
            }
            match frames.read_exact(&mut frame) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::UnexpectedEof => break,
                Err(error) => return Err(error.into()),
            }
            let start = *started.get_or_insert_with(|| {
                audio_queue.resume();
                Instant::now()
            });
            let target_time = start + Duration::from_secs_f64(frame_index as f64 / fps);
            if let Some(delay) = target_time.checked_duration_since(Instant::now()) {
                std::thread::sleep(delay);
            }
            texture.update(None, &frame, width as usize * 4)?;
            canvas.set_draw_color(Color::BLACK);
            canvas.clear();
            canvas.copy(&texture, None, target)?;
            canvas.present();
            frame_index += 1;
        }
        Ok(())
    })();
    if result != Playback::Finished || render_result.is_err() {
        let _ = decoder.kill();
    }
    let status = decoder.wait()?;
    audio_queue.clear();
    render_result?;
    if result == Playback::Finished && !status.success() {
        return Err(format!(
            "ffmpeg video decoder failed for {} with {status}",
            path.display()
        )
        .into());
    }
    Ok(result)
}
