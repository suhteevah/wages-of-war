//! Configuration system for Open Wages.
//!
//! Handles loading, saving, and merging of game configuration from JSON files
//! and CLI arguments. All fields have sensible defaults so a missing config
//! file is never fatal.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur while reading or writing configuration.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config file: {0}")]
    Io(#[from] std::io::Error),

    #[error("failed to parse config JSON: {0}")]
    Parse(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// Key bindings
// ---------------------------------------------------------------------------

/// Keyboard bindings for in-game actions.
///
/// Stored as human-readable key names (e.g. "W", "Tab", "F5") so the config
/// file stays easy to hand-edit.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KeyBindings {
    /// Camera scroll up.
    pub scroll_up: String,
    /// Camera scroll down.
    pub scroll_down: String,
    /// Camera scroll left.
    pub scroll_left: String,
    /// Camera scroll right.
    pub scroll_right: String,
    /// Zoom camera in.
    pub zoom_in: String,
    /// Zoom camera out.
    pub zoom_out: String,
    /// End the current unit's turn.
    pub end_turn: String,
    /// Cycle to the next available unit.
    pub next_unit: String,
    /// Pause / unpause.
    pub pause: String,
    /// Quick-save to the default slot.
    pub quicksave: String,
    /// Quick-load from the default slot.
    pub quickload: String,
}

impl Default for KeyBindings {
    fn default() -> Self {
        Self {
            scroll_up: "W".into(),
            scroll_down: "S".into(),
            scroll_left: "A".into(),
            scroll_right: "D".into(),
            zoom_in: "=".into(),
            zoom_out: "-".into(),
            end_turn: "E".into(),
            next_unit: "Tab".into(),
            pause: "Space".into(),
            quicksave: "F5".into(),
            quickload: "F9".into(),
        }
    }
}

// ---------------------------------------------------------------------------
// Game config
// ---------------------------------------------------------------------------

/// Top-level configuration for the Open Wages engine.
///
/// Every field has a `serde(default)` annotation so that partially-specified
/// JSON files gracefully fall back to defaults.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GameConfig {
    /// Path to the directory containing original game data files.
    #[serde(default = "default_data_dir")]
    pub data_dir: PathBuf,

    /// Directory where save-game files are written.
    #[serde(default = "default_save_dir")]
    pub save_dir: PathBuf,

    /// Window width in pixels.
    #[serde(default = "default_window_width")]
    pub window_width: u32,

    /// Window height in pixels.
    #[serde(default = "default_window_height")]
    pub window_height: u32,

    /// Start in fullscreen mode.
    #[serde(default)]
    pub fullscreen: bool,

    /// Enable vertical sync.
    #[serde(default = "default_true")]
    pub vsync: bool,

    /// Master volume (0.0 – 1.0).
    #[serde(default = "default_master_volume")]
    pub master_volume: f32,

    /// Music volume (0.0 – 1.0).
    #[serde(default = "default_music_volume")]
    pub music_volume: f32,

    /// Sound effects volume (0.0 – 1.0).
    #[serde(default = "default_sfx_volume")]
    pub sfx_volume: f32,

    /// Camera scroll speed in pixels per second.
    #[serde(default = "default_scroll_speed")]
    pub scroll_speed: f32,

    /// Zoom multiplier per scroll step.
    #[serde(default = "default_zoom_speed")]
    pub zoom_speed: f32,

    /// Show an FPS counter overlay.
    #[serde(default)]
    pub show_fps: bool,

    /// `tracing` log level filter string (e.g. "info", "debug", "ow_data=trace").
    #[serde(default = "default_log_level")]
    pub log_level: String,

    /// Additional directories to scan for mod content.
    #[serde(default)]
    pub mod_dirs: Vec<PathBuf>,

    /// Keyboard bindings.
    #[serde(default)]
    pub key_bindings: KeyBindings,
}

// -- default-value helper functions for serde --------------------------------

fn default_data_dir() -> PathBuf {
    PathBuf::from("./data")
}

fn default_save_dir() -> PathBuf {
    PathBuf::from("saves/")
}

fn default_window_width() -> u32 {
    1280
}

fn default_window_height() -> u32 {
    720
}

fn default_true() -> bool {
    true
}

fn default_master_volume() -> f32 {
    0.8
}

fn default_music_volume() -> f32 {
    0.6
}

fn default_sfx_volume() -> f32 {
    0.8
}

fn default_scroll_speed() -> f32 {
    400.0
}

fn default_zoom_speed() -> f32 {
    1.25
}

fn default_log_level() -> String {
    "info".into()
}

impl Default for GameConfig {
    fn default() -> Self {
        Self {
            data_dir: default_data_dir(),
            save_dir: default_save_dir(),
            window_width: default_window_width(),
            window_height: default_window_height(),
            fullscreen: false,
            vsync: true,
            master_volume: default_master_volume(),
            music_volume: default_music_volume(),
            sfx_volume: default_sfx_volume(),
            scroll_speed: default_scroll_speed(),
            zoom_speed: default_zoom_speed(),
            show_fps: false,
            log_level: default_log_level(),
            mod_dirs: Vec::new(),
            key_bindings: KeyBindings::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Load configuration from a JSON file at `path`.
///
/// Missing fields in the file are filled with defaults thanks to
/// `#[serde(default)]`. If the file does not exist at all, a full default
/// config is returned with a warning logged.
pub fn load_config(path: &Path) -> Result<GameConfig, ConfigError> {
    info!("loading config from {}", path.display());

    match std::fs::read_to_string(path) {
        Ok(contents) => {
            let config: GameConfig = serde_json::from_str(&contents)?;
            debug!("config loaded successfully: {:?}", config);
            Ok(config)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            warn!(
                "config file not found at {}, using defaults",
                path.display()
            );
            Ok(GameConfig::default())
        }
        Err(e) => Err(ConfigError::Io(e)),
    }
}

/// Persist `config` as pretty-printed JSON to `path`.
///
/// Parent directories are created automatically if they don't exist.
pub fn save_config(config: &GameConfig, path: &Path) -> Result<(), ConfigError> {
    info!("saving config to {}", path.display());

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let json = serde_json::to_string_pretty(config)?;
    std::fs::write(path, json)?;
    debug!("config saved successfully");
    Ok(())
}

/// Returns the default config file path for the current platform.
///
/// For now this is simply `./open-wages.json` next to the executable.
/// A future version may use platform-specific directories
/// (e.g. `%APPDATA%` on Windows, `~/.config` on Linux).
pub fn config_path() -> PathBuf {
    PathBuf::from("./open-wages.json")
}

/// Merge CLI arguments into an existing config.
///
/// Explicitly-provided CLI values take priority over whatever was loaded
/// from the config file.
pub fn merge_cli_args(config: &mut GameConfig, data_dir: Option<PathBuf>) {
    if let Some(dir) = data_dir {
        info!("CLI override: data_dir = {}", dir.display());
        config.data_dir = dir;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Default construction should match all documented default values.
    #[test]
    fn test_defaults() {
        let cfg = GameConfig::default();

        assert_eq!(cfg.data_dir, PathBuf::from("./data"));
        assert_eq!(cfg.save_dir, PathBuf::from("saves/"));
        assert_eq!(cfg.window_width, 1280);
        assert_eq!(cfg.window_height, 720);
        assert!(!cfg.fullscreen);
        assert!(cfg.vsync);
        assert!((cfg.master_volume - 0.8).abs() < f32::EPSILON);
        assert!((cfg.music_volume - 0.6).abs() < f32::EPSILON);
        assert!((cfg.sfx_volume - 0.8).abs() < f32::EPSILON);
        assert!((cfg.scroll_speed - 400.0).abs() < f32::EPSILON);
        assert!((cfg.zoom_speed - 1.25).abs() < f32::EPSILON);
        assert!(!cfg.show_fps);
        assert_eq!(cfg.log_level, "info");
        assert!(cfg.mod_dirs.is_empty());

        // Key bindings defaults.
        assert_eq!(cfg.key_bindings.scroll_up, "W");
        assert_eq!(cfg.key_bindings.end_turn, "E");
        assert_eq!(cfg.key_bindings.quicksave, "F5");
        assert_eq!(cfg.key_bindings.quickload, "F9");
    }

    /// Writing then reading a config should produce an identical struct.
    #[test]
    fn test_save_load_round_trip() {
        let dir = std::env::temp_dir().join("ow_config_test_round_trip");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test_config.json");

        let mut original = GameConfig::default();
        original.window_width = 1920;
        original.window_height = 1080;
        original.fullscreen = true;
        original.master_volume = 0.5;
        original.log_level = "debug".into();
        original.mod_dirs = vec![PathBuf::from("mods/core"), PathBuf::from("mods/extra")];
        original.key_bindings.scroll_up = "Up".into();

        save_config(&original, &path).unwrap();
        let loaded = load_config(&path).unwrap();

        assert_eq!(original, loaded);

        // Clean up.
        let _ = fs::remove_dir_all(&dir);
    }

    /// A JSON file with only a subset of fields should deserialize successfully,
    /// filling missing fields with their defaults.
    #[test]
    fn test_missing_fields_fall_back_to_defaults() {
        let dir = std::env::temp_dir().join("ow_config_test_partial");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("partial.json");

        // Only specify two fields — everything else must come from defaults.
        let partial_json = r#"{ "window_width": 1920, "fullscreen": true }"#;
        fs::write(&path, partial_json).unwrap();

        let cfg = load_config(&path).unwrap();

        // Explicit values.
        assert_eq!(cfg.window_width, 1920);
        assert!(cfg.fullscreen);

        // Everything else should be default.
        assert_eq!(cfg.window_height, 720);
        assert!(cfg.vsync);
        assert_eq!(cfg.data_dir, PathBuf::from("./data"));
        assert!((cfg.master_volume - 0.8).abs() < f32::EPSILON);
        assert_eq!(cfg.log_level, "info");
        assert!(cfg.mod_dirs.is_empty());
        assert_eq!(cfg.key_bindings, KeyBindings::default());

        let _ = fs::remove_dir_all(&dir);
    }

    /// `merge_cli_args` should override the data_dir when a value is provided.
    #[test]
    fn test_cli_override() {
        let mut cfg = GameConfig::default();
        assert_eq!(cfg.data_dir, PathBuf::from("./data"));

        // No override — should remain unchanged.
        merge_cli_args(&mut cfg, None);
        assert_eq!(cfg.data_dir, PathBuf::from("./data"));

        // With override.
        merge_cli_args(&mut cfg, Some(PathBuf::from("/opt/wages-of-war")));
        assert_eq!(cfg.data_dir, PathBuf::from("/opt/wages-of-war"));
    }

    /// Loading from a nonexistent path should return defaults, not an error.
    #[test]
    fn test_missing_file_returns_defaults() {
        let cfg = load_config(Path::new("/tmp/does_not_exist_ow_config.json")).unwrap();
        assert_eq!(cfg, GameConfig::default());
    }

    /// `config_path` should return a usable path.
    #[test]
    fn test_config_path() {
        let p = config_path();
        assert_eq!(p, PathBuf::from("./open-wages.json"));
    }
}
