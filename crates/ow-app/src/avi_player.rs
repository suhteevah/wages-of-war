//! # AVI Video Player
//!
//! Plays the original Wages of War AVI cutscenes (intro, credits, logos)
//! by decoding them through ffmpeg and rendering frames to SDL2.
//!
//! The original AVIs use:
//! - **Video:** MSRLE (Microsoft RLE), 640x480, 8bpp palette, 15fps
//! - **Audio:** ADPCM MS, 22050 Hz, mono
//!
//! We use `ffmpeg-sidecar` to shell out to the user's local ffmpeg binary,
//! which decodes frames to raw RGBA and pipes them back. This avoids
//! linking against C libraries while supporting all legacy codecs.

use sdl2::pixels::PixelFormatEnum;
use sdl2::rect::Rect;
use sdl2::render::{Canvas, TextureCreator};
use sdl2::video::{Window, WindowContext};
use std::path::Path;
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

/// Play an AVI file fullscreen on the SDL2 canvas.
///
/// Blocks until the video finishes or the user presses Escape/Space/Enter.
/// Returns `true` if playback completed, `false` if the user skipped it.
pub fn play_avi(
    canvas: &mut Canvas<Window>,
    texture_creator: &TextureCreator<WindowContext>,
    event_pump: &mut sdl2::EventPump,
    avi_path: &Path,
) -> bool {
    info!(path = %avi_path.display(), "playing AVI cutscene");

    if !avi_path.exists() {
        warn!(path = %avi_path.display(), "AVI file not found, skipping");
        return true;
    }

    // Probe the video first to get resolution and framerate.
    let (width, height, fps) = match probe_avi(avi_path) {
        Some(info) => info,
        None => {
            warn!("failed to probe AVI, using defaults 640x480 @ 15fps");
            (640u32, 480u32, 15.0f64)
        }
    };

    info!(width, height, fps, "AVI probed");

    // Ensure SDL2_mixer is initialized for audio playback.
    let _ = sdl2::mixer::open_audio(44100, sdl2::mixer::AUDIO_S16LSB, 2, 4096);
    sdl2::mixer::allocate_channels(4);

    // Extract audio to a temp WAV file so SDL2_mixer can play it
    // alongside the video. The original AVIs use ADPCM MS audio.
    let temp_dir = std::env::temp_dir();
    let audio_path = temp_dir.join("ow_cutscene_audio.wav");
    // Extract audio to a temp WAV file matching SDL2_mixer's expected format:
    // 16-bit signed LE, 22050 Hz (matches the original AVI audio), mono.
    // Force the WAV to exactly match our mixer's opened format to avoid
    // "Unrecognized audio format" errors from SDL2_mixer.
    let audio_ok = std::process::Command::new("ffmpeg")
        .args([
            "-y", "-i",
            avi_path.to_str().unwrap_or(""),
            "-vn",                   // no video
            "-acodec", "pcm_s16le",  // signed 16-bit little-endian PCM
            "-ar", "44100",          // match mixer sample rate
            "-ac", "2",              // stereo to match mixer channels
            audio_path.to_str().unwrap_or(""),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    // Play via Chunk on channel 0 — more reliable than Music for WAV files.
    let _chunk = if audio_ok {
        match sdl2::mixer::Chunk::from_file(&audio_path) {
            Ok(chunk) => {
                match sdl2::mixer::Channel::all().play(&chunk, 0) {
                    Ok(_) => {
                        info!("cutscene audio playing");
                        Some(chunk)
                    }
                    Err(e) => {
                        warn!(error = %e, "failed to play cutscene audio chunk");
                        None
                    }
                }
            }
            Err(e) => {
                warn!(error = %e, "failed to load cutscene audio WAV");
                None
            }
        }
    } else {
        warn!("failed to extract audio from AVI");
        None
    };

    // Start ffmpeg to decode video frames to raw RGB24 on stdout.
    // Using rgb24 (3 bytes/pixel) avoids RGBA/BGRA byte-order confusion
    // and is simpler — the original AVIs have no alpha channel anyway.
    let mut child = match ffmpeg_sidecar::command::FfmpegCommand::new()
        .input(avi_path.to_str().unwrap_or(""))
        .args(["-an", "-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            error!(error = %e, "failed to spawn ffmpeg — is it installed?");
            return true;
        }
    };

    // Create a streaming texture for RGB24 frames (no alpha needed).
    let mut texture = match texture_creator.create_texture_streaming(
        PixelFormatEnum::RGB24,
        width,
        height,
    ) {
        Ok(t) => t,
        Err(e) => {
            error!(error = %e, "failed to create video texture");
            let _ = child.kill();
            return true;
        }
    };

    let frame_size = (width * height * 3) as usize; // RGB24 = 3 bytes/pixel
    let frame_duration = Duration::from_secs_f64(1.0 / fps);
    let pitch = (width * 3) as usize;

    // Stretch video to fill the entire window — the AVIs are 640x480
    // and the game was designed for fullscreen playback.
    // Pass None as dst to canvas.copy() to fill the whole window.

    let mut frames_shown = 0u32;
    let start_time = Instant::now();
    let mut skipped = false;

    // Read decoded frames from ffmpeg's output iterator.
    let iter = child.iter().expect("failed to get ffmpeg output iterator");

    let mut frame_buf = Vec::with_capacity(frame_size);

    for event in iter {
        match event {
            ffmpeg_sidecar::event::FfmpegEvent::OutputFrame(frame) => {
                // Each OutputFrame contains raw pixel data for one video frame.
                let data = &frame.data;
                frame_buf.clear();
                frame_buf.extend_from_slice(data);

                // Pad or truncate to exact frame size if needed.
                frame_buf.resize(frame_size, 0);

                // Upload to GPU texture.
                if let Err(e) = texture.update(None, &frame_buf, pitch) {
                    warn!(error = %e, frame = frames_shown, "texture upload failed");
                    continue;
                }

                // Clear and blit.
                canvas.set_draw_color(sdl2::pixels::Color::RGB(0, 0, 0));
                canvas.clear();
                // None for dst = stretch to fill entire window.
                if let Err(e) = canvas.copy(&texture, None, None) {
                    warn!(error = %e, "canvas copy failed");
                }
                canvas.present();

                frames_shown += 1;

                // Frame pacing: wait until the next frame time.
                let target = start_time + frame_duration * frames_shown;
                let now = Instant::now();
                if now < target {
                    std::thread::sleep(target - now);
                }

                // Check for skip input (Escape, Space, Enter).
                for event in event_pump.poll_iter() {
                    match event {
                        sdl2::event::Event::Quit { .. } => {
                            skipped = true;
                        }
                        sdl2::event::Event::KeyDown { keycode: Some(k), .. }
                            if k == sdl2::keyboard::Keycode::Escape
                                || k == sdl2::keyboard::Keycode::Space
                                || k == sdl2::keyboard::Keycode::Return =>
                        {
                            info!(frame = frames_shown, "user skipped cutscene");
                            skipped = true;
                        }
                        _ => {}
                    }
                }

                if skipped {
                    break;
                }
            }
            ffmpeg_sidecar::event::FfmpegEvent::Progress(p) => {
                debug!(frame = p.frame, fps = p.fps, "ffmpeg progress");
            }
            ffmpeg_sidecar::event::FfmpegEvent::Error(e) => {
                warn!(error = %e, "ffmpeg error during playback");
            }
            _ => {}
        }
    }

    // Stop the cutscene audio and clean up temp file.
    sdl2::mixer::Channel::all().halt();
    let _ = std::fs::remove_file(&audio_path);

    let elapsed = start_time.elapsed();
    info!(
        frames = frames_shown,
        elapsed_ms = elapsed.as_millis(),
        skipped,
        "AVI playback finished"
    );

    !skipped
}

/// Probe an AVI file to get resolution and framerate.
/// Returns (width, height, fps) or None if probing fails.
fn probe_avi(path: &Path) -> Option<(u32, u32, f64)> {
    // Use ffprobe (bundled with ffmpeg) to get video stream info.
    let output = std::process::Command::new("ffprobe")
        .args([
            "-v", "quiet",
            "-print_format", "json",
            "-show_streams",
            path.to_str()?,
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    let streams = json.get("streams")?.as_array()?;

    for stream in streams {
        if stream.get("codec_type")?.as_str()? == "video" {
            let w = stream.get("width")?.as_u64()? as u32;
            let h = stream.get("height")?.as_u64()? as u32;

            // Parse framerate from "r_frame_rate" (e.g. "15/1").
            let rate_str = stream.get("r_frame_rate")?.as_str()?;
            let fps = if let Some((num, den)) = rate_str.split_once('/') {
                let n: f64 = num.parse().ok()?;
                let d: f64 = den.parse().ok()?;
                if d > 0.0 { n / d } else { 15.0 }
            } else {
                rate_str.parse().unwrap_or(15.0)
            };

            return Some((w, h, fps));
        }
    }

    None
}
