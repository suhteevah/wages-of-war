//! Combat sound effects manager.
//!
//! Pre-loads WAV files from `WOW/SND/` as SDL2_mixer `Chunk` objects and maps
//! them to combat events (gunshot, explosion, hit, miss, etc.). This avoids
//! disk I/O during combat — all sounds are decoded and resident in memory
//! before the first shot is fired.
//!
//! # Channel allocation
//!
//! Channels 0–1 are reserved for voice/music. SFX uses channels 2–7.
//! We round-robin through those 6 channels so overlapping sounds (e.g. two
//! shots in quick succession) don't cut each other off.

use std::collections::HashMap;
use std::path::Path;

use sdl2::mixer::Chunk;
use tracing::{debug, info, trace, warn};

/// First mixer channel available for SFX (0–1 reserved for voice/music).
const SFX_CHANNEL_START: i32 = 2;
/// Last mixer channel for SFX (inclusive).
const SFX_CHANNEL_END: i32 = 7;

/// Categories of combat sounds. Each maps to one or more WAV files.
/// When multiple files exist for a category (e.g. RIFLE1, RIFLE2), one is
/// chosen at random to avoid repetitive audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CombatSound {
    /// Small arms fire (pistol, SMG).
    Pistol,
    /// Rifle shot.
    Rifle,
    /// Shotgun blast.
    Shotgun,
    /// Generic gunshot fallback when weapon type is unknown.
    GenericShot,
    /// Bullet impact / hit confirmation.
    Hit,
    /// Shot missed — ricochet or whizz.
    Miss,
    /// Explosion (grenade, RPG, mine).
    Explosion,
    /// Unit killed.
    Kill,
    /// Training / drilling sound.
    Train,
}

/// Pre-loaded combat SFX manager.
///
/// Holds SDL2_mixer `Chunk` handles for each discovered sound file, grouped
/// by [`CombatSound`] category. Chunks are reference-counted by SDL2 internally,
/// so cloning this struct is cheap (but we don't need to — it lives in the
/// game loop as a single instance).
pub struct SfxManager {
    /// Sound chunks grouped by combat category. Multiple chunks per category
    /// allows random variation (e.g. RIFLE1.WAV vs RIFLE2.WAV).
    sounds: HashMap<CombatSound, Vec<Chunk>>,
    /// Round-robin counter for channel allocation. Wraps through channels 2–7.
    next_channel: i32,
    /// Whether audio is available. If false, all play calls are silently skipped
    /// rather than returning errors (graceful degradation).
    audio_available: bool,
}

impl SfxManager {
    /// Create a new SFX manager by scanning the SND directory and loading all
    /// recognized WAV files as SDL2_mixer chunks.
    ///
    /// If the SND directory doesn't exist or individual files fail to load,
    /// warnings are logged but the manager is still created (with fewer or no
    /// sounds). This allows the game to run without audio gracefully.
    ///
    /// # Arguments
    /// * `snd_dir` — Path to the `WOW/SND/` directory containing combat WAVs.
    /// * `audio_available` — Whether SDL2_mixer was successfully initialized.
    ///   If false, no files are loaded and all play calls become no-ops.
    pub fn new(snd_dir: &Path, audio_available: bool) -> Self {
        let mut manager = SfxManager {
            sounds: HashMap::new(),
            next_channel: SFX_CHANNEL_START,
            audio_available,
        };

        if !audio_available {
            info!("SFX manager created in silent mode (no audio device)");
            return manager;
        }

        // Allocate enough mixer channels for SFX (SDL2_mixer defaults to 8,
        // but let's be explicit to ensure channels 2–7 exist).
        let total_channels = SFX_CHANNEL_END + 1;
        sdl2::mixer::allocate_channels(total_channels);
        debug!(channels = total_channels, "SDL2_mixer channels allocated for SFX");

        if !snd_dir.exists() {
            warn!(dir = %snd_dir.display(), "SND directory not found — no combat SFX loaded");
            return manager;
        }

        // Scan directory and classify each WAV by filename.
        let entries = match std::fs::read_dir(snd_dir) {
            Ok(entries) => entries,
            Err(e) => {
                warn!(error = %e, dir = %snd_dir.display(), "Failed to read SND directory");
                return manager;
            }
        };

        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    warn!(error = %e, "Failed to read directory entry in SND");
                    continue;
                }
            };

            let path = entry.path();

            // Only process .WAV files.
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_uppercase();
            if ext != "WAV" {
                continue;
            }

            let stem = match path.file_stem().and_then(|s| s.to_str()) {
                Some(s) => s.to_uppercase(),
                None => continue,
            };

            // Classify the sound by filename pattern.
            // The original game uses descriptive names: PISTOL, RIFLE1, SHOTGUN3, etc.
            let category = classify_sound(&stem);

            // Load the WAV as an SDL2_mixer Chunk.
            match Chunk::from_file(&path) {
                Ok(chunk) => {
                    debug!(
                        file = %stem,
                        category = ?category,
                        "Loaded SFX chunk"
                    );
                    manager.sounds.entry(category).or_default().push(chunk);
                }
                Err(e) => {
                    warn!(
                        file = %path.display(),
                        error = %e,
                        "Failed to load WAV as mixer Chunk — skipping"
                    );
                }
            }
        }

        let total: usize = manager.sounds.values().map(|v| v.len()).sum();
        info!(
            total_chunks = total,
            categories = manager.sounds.len(),
            "SFX manager initialized"
        );

        manager
    }

    /// Play a combat sound effect on the next available SFX channel.
    ///
    /// If multiple WAV files exist for the given category, one is chosen
    /// randomly for variety. If no sounds are loaded for the category,
    /// the call is silently ignored (combat still works without audio).
    pub fn play(&mut self, sound: CombatSound) {
        if !self.audio_available {
            trace!(sound = ?sound, "SFX play skipped (no audio)");
            return;
        }

        let chunks = match self.sounds.get(&sound) {
            Some(c) if !c.is_empty() => c,
            _ => {
                // Fall back to GenericShot for any weapon-type sound that's missing.
                match sound {
                    CombatSound::Pistol | CombatSound::Rifle | CombatSound::Shotgun => {
                        match self.sounds.get(&CombatSound::GenericShot) {
                            Some(c) if !c.is_empty() => c,
                            _ => {
                                trace!(sound = ?sound, "No SFX loaded for category (and no fallback)");
                                return;
                            }
                        }
                    }
                    _ => {
                        trace!(sound = ?sound, "No SFX loaded for category");
                        return;
                    }
                }
            }
        };

        // Pick a random chunk from the available variants for this category.
        let idx = if chunks.len() == 1 {
            0
        } else {
            // Simple pseudo-random: use the channel counter as entropy.
            // Not cryptographic, but perfectly fine for audio variation.
            (self.next_channel as usize) % chunks.len()
        };

        let channel = sdl2::mixer::Channel(self.next_channel);
        match channel.play(&chunks[idx], 0) {
            Ok(_) => {
                trace!(
                    sound = ?sound,
                    channel = self.next_channel,
                    "Playing SFX"
                );
            }
            Err(e) => {
                warn!(
                    sound = ?sound,
                    channel = self.next_channel,
                    error = %e,
                    "Failed to play SFX on channel"
                );
            }
        }

        // Advance to next channel, wrapping within the SFX range.
        self.next_channel += 1;
        if self.next_channel > SFX_CHANNEL_END {
            self.next_channel = SFX_CHANNEL_START;
        }
    }
}

/// Classify a WAV filename stem into a [`CombatSound`] category.
///
/// Uses prefix matching because the original game appends variant suffixes
/// (PISTOLA, PISTOLB, RIFLE1, RIFLE2, SHOTGUN3, etc.). Unrecognized files
/// are mapped to [`CombatSound::GenericShot`] as a catch-all.
fn classify_sound(stem: &str) -> CombatSound {
    // Order matters — check more specific prefixes first.
    if stem.starts_with("SHOTGUN") {
        CombatSound::Shotgun
    } else if stem.starts_with("RIFLE") {
        CombatSound::Rifle
    } else if stem.starts_with("PISTOL") {
        CombatSound::Pistol
    } else if stem.starts_with("EXPLO") || stem.starts_with("BOOM") || stem.starts_with("GRENAD") {
        CombatSound::Explosion
    } else if stem.starts_with("HIT") || stem.starts_with("IMPACT") {
        CombatSound::Hit
    } else if stem.starts_with("MISS") || stem.starts_with("RICO") || stem.starts_with("WHIZ") {
        CombatSound::Miss
    } else if stem.starts_with("TRAIN") || stem.starts_with("8LTRAIN") {
        CombatSound::Train
    } else if stem.starts_with("KILL") || stem.starts_with("DEATH") || stem.starts_with("DIE") {
        CombatSound::Kill
    } else {
        // Unknown file — still useful as a generic gunshot fallback.
        debug!(stem = %stem, "Unrecognized SFX filename, mapping to GenericShot");
        CombatSound::GenericShot
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_pistol_variants() {
        assert_eq!(classify_sound("PISTOL"), CombatSound::Pistol);
        assert_eq!(classify_sound("PISTOLA"), CombatSound::Pistol);
        assert_eq!(classify_sound("PISTOLB"), CombatSound::Pistol);
    }

    #[test]
    fn classify_rifle_variants() {
        assert_eq!(classify_sound("RIFLE1"), CombatSound::Rifle);
        assert_eq!(classify_sound("RIFLE2"), CombatSound::Rifle);
    }

    #[test]
    fn classify_shotgun() {
        assert_eq!(classify_sound("SHOTGUN3"), CombatSound::Shotgun);
    }

    #[test]
    fn classify_training() {
        assert_eq!(classify_sound("8LTRAIN"), CombatSound::Train);
    }

    #[test]
    fn classify_unknown_falls_back_to_generic() {
        assert_eq!(classify_sound("FOOBAR"), CombatSound::GenericShot);
    }
}
