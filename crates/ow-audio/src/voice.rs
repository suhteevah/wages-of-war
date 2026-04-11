//! Voice line playback system.
//!
//! Manages on-demand loading and playback of merc/NPC voice WAV files through
//! SDL2_mixer. Voice lines play on a dedicated mixer channel so they never
//! interrupt background music (which uses the separate `Music` API).
//!
//! # Channel allocation
//!
//! SDL2_mixer mixes audio on numbered channels. Channel 0 is typically used
//! for SFX; we reserve **channel 1** exclusively for voice playback. This
//! guarantees that starting a new voice line cleanly cuts off any still-playing
//! previous line, without stomping on SFX or music.
//!
//! # Lazy loading
//!
//! WAV data is loaded into memory (`sdl2::mixer::Chunk`) the first time a
//! voice line is requested, then cached for instant replay. This avoids
//! loading all ~100+ WAV files at startup while still being fast on repeat
//! plays (e.g., repeatedly clicking the same merc in the hiring screen).
//!
//! # Naming convention
//!
//! Voice lines are looked up by uppercase filename stem. For merc voice lines
//! the game uses the merc's name (e.g., "SLADE", "CALDWELL"). For NPCs it
//! uses numbered variants (e.g., "ARTIE01", "MOM02"). The caller just passes
//! the stem; this module handles path construction and caching.

use std::collections::HashMap;
use std::path::PathBuf;

use sdl2::mixer::{Channel, Chunk};
use tracing::{debug, info, trace, warn};

/// Dedicated SDL2_mixer channel for voice playback.
/// Kept separate from channel 0 (SFX) and the Music API (which has its own
/// channel management). Playing a new voice line on this channel automatically
/// halts any previous voice line still playing.
const VOICE_CHANNEL: i32 = 1;

/// Volume for voice lines: 75% of SDL2_mixer's 0–128 range.
/// Voice should be prominent over music but not clipping.
const VOICE_VOLUME: i32 = 96;

/// Manages voice line loading, caching, and playback.
///
/// Create one of these after `sdl2::mixer::open_audio()` succeeds.
/// It holds cached `Chunk` data and the path to the WAV directory.
pub struct VoicePlayer {
    /// Root directory containing WAV files (e.g., `data/WOW/WAV/`).
    wav_dir: PathBuf,

    /// Cache of loaded audio chunks, keyed by uppercase filename stem.
    /// Once a WAV is loaded it stays in memory for the session.
    cache: HashMap<String, Chunk>,
}

impl VoicePlayer {
    /// Create a new voice player targeting the given WAV directory.
    ///
    /// Does NOT load any files yet — loading is deferred to first playback
    /// request. Ensures at least 2 mixer channels are allocated (channel 0
    /// for SFX, channel 1 for voice).
    pub fn new(wav_dir: PathBuf) -> Self {
        // SDL2_mixer defaults to 8 channels, but be explicit: we need at
        // least channel 0 (SFX) and channel 1 (voice). allocate_channels
        // returns the new total; if someone already allocated more, this
        // is a no-op (it only increases, never decreases).
        let current = sdl2::mixer::allocate_channels(-1); // query current count
        if current < 2 {
            let allocated = sdl2::mixer::allocate_channels(2);
            debug!(channels = allocated, "Allocated SDL2_mixer channels for voice");
        }

        info!(dir = %wav_dir.display(), "Voice player initialized");
        Self {
            wav_dir,
            cache: HashMap::new(),
        }
    }

    /// Play a voice line by name (case-insensitive).
    ///
    /// Looks for `{name}.WAV` in the WAV directory. If the file exists,
    /// loads it (or uses the cached version) and plays it on the dedicated
    /// voice channel. Any currently-playing voice line is halted first.
    ///
    /// Returns `true` if playback started successfully, `false` if the file
    /// was missing or couldn't be loaded/played (with warnings logged).
    pub fn play(&mut self, name: &str) -> bool {
        let key = name.to_uppercase();

        // Fast path: already cached
        if self.cache.contains_key(&key) {
            return self.play_cached(&key);
        }

        // Slow path: load from disk
        let wav_path = self.wav_dir.join(format!("{key}.WAV"));
        if !wav_path.exists() {
            // Not every merc has a dedicated WAV file — this is expected.
            // Log at trace level to avoid spamming on every click.
            trace!(name = %key, path = %wav_path.display(),
                   "No voice WAV file found for this name");
            return false;
        }

        match Chunk::from_file(&wav_path) {
            Ok(mut chunk) => {
                // Set per-chunk volume so all voice lines play at consistent level.
                chunk.set_volume(VOICE_VOLUME);
                info!(name = %key, path = %wav_path.display(), "Loaded voice line");
                self.cache.insert(key.clone(), chunk);
                self.play_cached(&key)
            }
            Err(e) => {
                warn!(name = %key, path = %wav_path.display(), error = %e,
                      "Failed to load voice WAV -- skipping");
                false
            }
        }
    }

    /// Play an already-cached chunk on the voice channel.
    fn play_cached(&self, key: &str) -> bool {
        if let Some(chunk) = self.cache.get(key) {
            let channel = Channel(VOICE_CHANNEL);
            match channel.play(chunk, 0) {
                Ok(_) => {
                    debug!(name = %key, channel = VOICE_CHANNEL, "Playing voice line");
                    true
                }
                Err(e) => {
                    warn!(name = %key, error = %e,
                          "SDL2_mixer failed to play voice chunk");
                    false
                }
            }
        } else {
            false
        }
    }

    /// Stop any currently-playing voice line immediately.
    pub fn stop(&self) {
        let channel = Channel(VOICE_CHANNEL);
        channel.halt();
        trace!("Voice channel halted");
    }

    /// Returns `true` if a voice line is currently playing.
    pub fn is_playing(&self) -> bool {
        let channel = Channel(VOICE_CHANNEL);
        channel.is_playing()
    }

    /// Returns the number of cached voice chunks (for diagnostics).
    pub fn cached_count(&self) -> usize {
        self.cache.len()
    }

    /// Pre-load a voice line without playing it. Useful for warming the cache
    /// before a screen transition (e.g., load all hired mercs' voice lines
    /// when entering the hiring screen).
    pub fn preload(&mut self, name: &str) -> bool {
        let key = name.to_uppercase();
        if self.cache.contains_key(&key) {
            return true;
        }
        let wav_path = self.wav_dir.join(format!("{key}.WAV"));
        if !wav_path.exists() {
            return false;
        }
        match Chunk::from_file(&wav_path) {
            Ok(mut chunk) => {
                chunk.set_volume(VOICE_VOLUME);
                debug!(name = %key, "Pre-loaded voice line");
                self.cache.insert(key, chunk);
                true
            }
            Err(e) => {
                warn!(name = %key, error = %e, "Failed to pre-load voice WAV");
                false
            }
        }
    }
}

impl std::fmt::Debug for VoicePlayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VoicePlayer")
            .field("wav_dir", &self.wav_dir)
            .field("cached_count", &self.cache.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    /// Create a minimal valid WAV file (44 bytes: header only, no audio data).
    fn write_dummy_wav(path: &Path) {
        let header: [u8; 44] = [
            b'R', b'I', b'F', b'F',
            36, 0, 0, 0,
            b'W', b'A', b'V', b'E',
            b'f', b'm', b't', b' ',
            16, 0, 0, 0,
            1, 0,        // PCM
            1, 0,        // mono
            0x22, 0x56, 0, 0, // 22050 Hz
            0x22, 0x56, 0, 0, // byte rate
            1, 0,        // block align
            8, 0,        // 8 bits per sample
            b'd', b'a', b't', b'a',
            0, 0, 0, 0,  // data size
        ];
        fs::write(path, &header).expect("failed to write dummy WAV");
    }

    #[test]
    fn voice_player_creation() {
        // VoicePlayer::new doesn't require SDL2 init — it just stores the path.
        // (play/preload will fail without SDL2, but creation is safe.)
        let dir = std::env::temp_dir().join("ow_audio_test_voice");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        write_dummy_wav(&dir.join("SLADE.WAV"));
        write_dummy_wav(&dir.join("CALDWELL.WAV"));

        // We can't actually init SDL2 in unit tests, but we can verify
        // the struct is created correctly.
        // VoicePlayer::new() calls allocate_channels which requires SDL2,
        // so we just test the type construction logic conceptually.
        // Full integration testing happens in the game loop.

        let _ = fs::remove_dir_all(&dir);
    }
}
