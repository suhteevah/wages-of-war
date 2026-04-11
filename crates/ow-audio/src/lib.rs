//! # ow-audio — Sound & Music for Open Wages
//!
//! This crate handles all audio concerns for the Wages of War engine reimplementation.
//! It provides three subsystems:
//!
//! - **WAV loader** (`wav_loader`): Catalogs standard `.WAV` sound effects from `WOW/WAV/`
//!   and `WOW/SND/` directories. Files are indexed by name for lazy loading.
//!
//! - **MIDI music** (`music`): Catalogs `.MID` background music tracks from `WOW/MIDI/`.
//!   The game has ~18 MIDI tracks for missions, menus, and events.
//!
//! - **VLA/VLS parser** (`vla_parser`): Parses the custom "VALS" container format used for
//!   voiced dialogue. These files wrap a standard RIFF/WAV with lip-sync animation data
//!   and word-boundary timing (the "WRDS" section). VLA = voice + lip animation,
//!   VLS = voice + lip + subtitles (identical format in practice).
//!
//! # Architecture
//!
//! All three modules produce catalog/parsed structs only — no actual audio playback.
//! Playback will be handled by SDL2_mixer or rodio, driven by `ow-app`.
//! This separation keeps `ow-audio` testable without an audio device.
//!
//! # Additional dependencies needed (not yet in Cargo.toml)
//!
//! ```toml
//! thiserror = { workspace = true }
//! serde = { workspace = true }
//! ```

pub mod music;
pub mod sfx;
pub mod vla_parser;
pub mod voice;
pub mod wav_loader;

use std::path::PathBuf;

/// Errors that can occur during audio file loading and parsing.
#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    /// An I/O error occurred while reading a file or scanning a directory.
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },

    /// A file's contents did not match the expected format (wrong magic bytes,
    /// truncated data, missing required sections, etc.).
    #[error("invalid format in {path}: {detail}")]
    InvalidFormat { path: PathBuf, detail: String },

    /// A referenced file or directory was not found.
    #[error("not found: {path}")]
    NotFound { path: PathBuf },
}

impl AudioError {
    /// Convenience constructor for I/O errors with path context.
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }

    /// Convenience constructor for format errors.
    pub fn invalid_format(path: impl Into<PathBuf>, detail: impl Into<String>) -> Self {
        Self::InvalidFormat {
            path: path.into(),
            detail: detail.into(),
        }
    }

    /// Convenience constructor for not-found errors.
    pub fn not_found(path: impl Into<PathBuf>) -> Self {
        Self::NotFound { path: path.into() }
    }
}
