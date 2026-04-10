//! MIDI music track catalog.
//!
//! The original game stores standard MIDI files in `WOW/MIDI/`. There are ~18 tracks
//! covering mission music, office ambience, arrival fanfares, departure themes, etc.
//!
//! Known track names and their contexts:
//! - `WOWMIS01`..`WOWMIS09` — Mission combat/briefing music (one per scenario)
//! - `WOWOFICE` — Office/headquarters background music
//! - `WOWARIVE` — Arrival theme (returning from a mission)
//! - `WOWDPARL` / `WOWDPARW` — Departure themes (loss/win variants?)
//! - `WOWC01` — Contract/cutscene music
//! - `NEWORLD1`..`NEWORLD3` — New World Computing logo music
//! - `TEST` — Developer test track
//!
//! Like the WAV catalog, this module only indexes file paths for lazy loading.
//! Actual MIDI playback is handled by SDL2_mixer or a dedicated MIDI synthesizer.

use std::path::{Path, PathBuf};

use tracing::{debug, info, warn};

use crate::AudioError;

/// Metadata for a single MIDI music track.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MusicTrack {
    /// The canonical name for this track (filename stem, uppercase).
    /// Example: "WOWMIS01", "WOWOFICE"
    pub name: String,

    /// Full filesystem path to the .MID file.
    pub path: PathBuf,
}

/// A catalog of all discovered MIDI music tracks.
///
/// Tracks are stored in the order they were found (directory iteration order).
/// Use [`MusicCatalog::get`] for name-based lookup.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MusicCatalog {
    /// All discovered MIDI tracks.
    pub tracks: Vec<MusicTrack>,
}

impl MusicCatalog {
    /// Returns the number of cataloged music tracks.
    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    /// Returns true if no tracks were found.
    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    /// Look up a music track by name (case-insensitive).
    pub fn get(&self, name: &str) -> Option<&MusicTrack> {
        let upper = name.to_uppercase();
        self.tracks.iter().find(|t| t.name == upper)
    }
}

/// Scans a directory for `.MID` files and builds a [`MusicCatalog`].
///
/// Only files with the `.MID` extension (case-insensitive) are included.
/// Non-MIDI files (like `PRINTME.TXT` which exists in the original game's MIDI dir)
/// are silently skipped.
///
/// # Arguments
/// * `midi_dir` — Path to the MIDI directory (e.g., `WOW/MIDI/`).
///
/// # Errors
/// Returns [`AudioError::NotFound`] if the directory does not exist.
/// Returns [`AudioError::Io`] if the directory cannot be read.
pub fn scan_midi_directory(midi_dir: &Path) -> Result<MusicCatalog, AudioError> {
    if !midi_dir.exists() {
        return Err(AudioError::not_found(midi_dir));
    }
    if !midi_dir.is_dir() {
        return Err(AudioError::invalid_format(
            midi_dir,
            "expected a directory, not a file",
        ));
    }

    info!(dir = %midi_dir.display(), "scanning for MIDI music tracks");

    let mut tracks = Vec::new();

    let entries =
        std::fs::read_dir(midi_dir).map_err(|e| AudioError::io(midi_dir, e))?;

    for entry in entries {
        let entry = entry.map_err(|e| AudioError::io(midi_dir, e))?;
        let path = entry.path();

        if !path.is_file() {
            continue;
        }
        let ext = match path.extension().and_then(|e| e.to_str()) {
            Some(e) => e.to_uppercase(),
            None => continue,
        };
        if ext != "MID" {
            continue;
        }

        let stem = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s.to_uppercase(),
            None => {
                warn!(path = %path.display(), "skipping MIDI file with unparseable name");
                continue;
            }
        };

        debug!(name = %stem, path = %path.display(), "cataloged MIDI track");

        tracks.push(MusicTrack {
            name: stem,
            path,
        });
    }

    // Sort by name for deterministic ordering (directory iteration is OS-dependent).
    tracks.sort_by(|a, b| a.name.cmp(&b.name));

    info!(count = tracks.len(), dir = %midi_dir.display(), "MIDI scan complete");

    Ok(MusicCatalog { tracks })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn scan_finds_mid_files() {
        let dir = std::env::temp_dir().join("ow_audio_test_midi_scan");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        // Create dummy MIDI files (content doesn't matter for cataloging)
        fs::write(dir.join("WOWMIS01.MID"), b"MThd fake midi").unwrap();
        fs::write(dir.join("WOWOFICE.MID"), b"MThd fake midi").unwrap();
        fs::write(dir.join("test.mid"), b"MThd fake midi").unwrap();

        // Non-MIDI files that should be skipped
        fs::write(dir.join("PRINTME.TXT"), b"print me").unwrap();
        fs::write(dir.join("notes.doc"), b"notes").unwrap();

        let catalog = scan_midi_directory(&dir).unwrap();

        assert_eq!(catalog.len(), 3);
        assert!(catalog.get("WOWMIS01").is_some());
        assert!(catalog.get("wowofice").is_some()); // case-insensitive
        assert!(catalog.get("TEST").is_some());
        assert!(catalog.get("PRINTME").is_none());

        // Verify sorted order
        assert_eq!(catalog.tracks[0].name, "TEST");
        assert_eq!(catalog.tracks[1].name, "WOWMIS01");
        assert_eq!(catalog.tracks[2].name, "WOWOFICE");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_nonexistent_directory_returns_not_found() {
        let result = scan_midi_directory(Path::new("/nonexistent/midi/dir"));
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), AudioError::NotFound { .. }));
    }

    #[test]
    fn empty_directory_returns_empty_catalog() {
        let dir = std::env::temp_dir().join("ow_audio_test_midi_empty");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let catalog = scan_midi_directory(&dir).unwrap();
        assert!(catalog.is_empty());

        let _ = fs::remove_dir_all(&dir);
    }
}
