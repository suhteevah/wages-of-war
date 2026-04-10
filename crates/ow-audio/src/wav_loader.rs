//! WAV sound effect catalog.
//!
//! The original game stores standard PCM `.WAV` files in two directories:
//! - `WOW/WAV/` — mixed with VLA/VLS voice files (~133 WAV files)
//! - `WOW/SND/` — weapon/combat sound effects (PISTOL, RIFLE, SHOTGUN, etc.)
//!
//! The game references sounds by their filename stem (without extension). For example,
//! the weapon definition for a pistol references "PISTOL" which maps to `SND/PISTOL.WAV`.
//! This module catalogs all `.WAV` files by that stem name for O(1) lookup at runtime.
//!
//! We intentionally do NOT decode or load the audio data here — that happens lazily
//! when the sound is first played, via SDL2_mixer or rodio.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use tracing::{debug, info, warn};

use crate::AudioError;

/// Metadata for a single sound effect file.
///
/// The actual audio data is not loaded until playback is requested.
/// We store just enough to find and identify the file.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SoundEffect {
    /// The canonical name used to reference this sound (filename stem, uppercase).
    /// Example: "PISTOL", "SHOTGUN3", "14WIN"
    pub name: String,

    /// Full filesystem path to the .WAV file.
    pub path: PathBuf,

    /// Duration in milliseconds, if known. Populated lazily on first load,
    /// or by a pre-scan pass. `None` means we haven't read the file yet.
    pub duration_ms: Option<u32>,
}

/// A catalog of all discovered sound effect files, indexed by name.
///
/// Names are stored uppercase (matching the original game's convention).
/// Duplicate names from different directories will warn and keep the last one found.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SoundCatalog {
    /// Sound effects indexed by uppercase filename stem.
    pub effects: HashMap<String, SoundEffect>,
}

impl SoundCatalog {
    /// Returns the number of cataloged sound effects.
    pub fn len(&self) -> usize {
        self.effects.len()
    }

    /// Returns true if no sound effects were found.
    pub fn is_empty(&self) -> bool {
        self.effects.is_empty()
    }

    /// Look up a sound effect by name (case-insensitive).
    pub fn get(&self, name: &str) -> Option<&SoundEffect> {
        self.effects.get(&name.to_uppercase())
    }
}

/// Scans a directory for `.WAV` files and builds a [`SoundCatalog`].
///
/// Only files with the `.WAV` extension (case-insensitive) are included.
/// VLA and VLS files in the same directory are skipped.
///
/// # Arguments
/// * `wav_dir` — Path to the directory to scan (e.g., `WOW/WAV/` or `WOW/SND/`).
///
/// # Errors
/// Returns [`AudioError::NotFound`] if the directory does not exist.
/// Returns [`AudioError::Io`] if the directory cannot be read.
pub fn scan_wav_directory(wav_dir: &Path) -> Result<SoundCatalog, AudioError> {
    if !wav_dir.exists() {
        return Err(AudioError::not_found(wav_dir));
    }
    if !wav_dir.is_dir() {
        return Err(AudioError::invalid_format(
            wav_dir,
            "expected a directory, not a file",
        ));
    }

    info!(dir = %wav_dir.display(), "scanning for WAV sound effects");

    let mut effects = HashMap::new();

    let entries =
        std::fs::read_dir(wav_dir).map_err(|e| AudioError::io(wav_dir, e))?;

    for entry in entries {
        let entry = entry.map_err(|e| AudioError::io(wav_dir, e))?;
        let path = entry.path();

        // Only process regular files with .WAV extension.
        if !path.is_file() {
            continue;
        }
        let ext = match path.extension().and_then(|e| e.to_str()) {
            Some(e) => e.to_uppercase(),
            None => continue,
        };
        if ext != "WAV" {
            continue;
        }

        // The stem (filename without extension) is the lookup key.
        let stem = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s.to_uppercase(),
            None => {
                warn!(path = %path.display(), "skipping WAV file with unparseable name");
                continue;
            }
        };

        debug!(name = %stem, path = %path.display(), "cataloged WAV sound effect");

        if effects.contains_key(&stem) {
            warn!(
                name = %stem,
                path = %path.display(),
                "duplicate sound effect name; overwriting previous entry"
            );
        }

        effects.insert(
            stem.clone(),
            SoundEffect {
                name: stem,
                path,
                duration_ms: None,
            },
        );
    }

    info!(count = effects.len(), dir = %wav_dir.display(), "WAV scan complete");

    Ok(SoundCatalog { effects })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Create a minimal valid WAV file (44 bytes: header only, no audio data).
    /// We don't need valid audio — just a file with a .WAV extension.
    fn write_dummy_wav(path: &Path) {
        // RIFF header + WAVEfmt chunk + empty data chunk = 44 bytes
        let header: [u8; 44] = [
            b'R', b'I', b'F', b'F', // RIFF magic
            36, 0, 0, 0, // chunk size (36 = file size - 8)
            b'W', b'A', b'V', b'E', // WAVE format
            b'f', b'm', b't', b' ', // fmt subchunk
            16, 0, 0, 0, // subchunk size (16 for PCM)
            1, 0, // audio format (1 = PCM)
            1, 0, // num channels
            0x22, 0x56, 0, 0, // sample rate (22050)
            0x22, 0x56, 0, 0, // byte rate
            1, 0, // block align
            8, 0, // bits per sample
            b'd', b'a', b't', b'a', // data subchunk
            0, 0, 0, 0, // data size (0)
        ];
        fs::write(path, &header).expect("failed to write dummy WAV");
    }

    #[test]
    fn scan_finds_wav_files() {
        let dir = std::env::temp_dir().join("ow_audio_test_wav_scan");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        // Create some dummy WAV files
        write_dummy_wav(&dir.join("PISTOL.WAV"));
        write_dummy_wav(&dir.join("rifle1.wav")); // lowercase extension
        write_dummy_wav(&dir.join("SHOTGUN3.WAV"));

        // Create a non-WAV file that should be ignored
        fs::write(dir.join("ARTIE01.VLS"), b"VALS fake data").unwrap();
        fs::write(dir.join("readme.txt"), b"not audio").unwrap();

        let catalog = scan_wav_directory(&dir).unwrap();

        assert_eq!(catalog.len(), 3);
        assert!(catalog.get("pistol").is_some()); // case-insensitive lookup
        assert!(catalog.get("RIFLE1").is_some());
        assert!(catalog.get("SHOTGUN3").is_some());
        assert!(catalog.get("ARTIE01").is_none()); // VLS excluded
        assert!(catalog.get("readme").is_none()); // TXT excluded

        // Verify path is preserved
        let pistol = catalog.get("PISTOL").unwrap();
        assert_eq!(pistol.path, dir.join("PISTOL.WAV"));
        assert!(pistol.duration_ms.is_none());

        // Clean up
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_nonexistent_directory_returns_not_found() {
        let result = scan_wav_directory(Path::new("/nonexistent/audio/dir"));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, AudioError::NotFound { .. }));
    }

    #[test]
    fn empty_directory_returns_empty_catalog() {
        let dir = std::env::temp_dir().join("ow_audio_test_wav_empty");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let catalog = scan_wav_directory(&dir).unwrap();
        assert!(catalog.is_empty());
        assert_eq!(catalog.len(), 0);

        let _ = fs::remove_dir_all(&dir);
    }
}
