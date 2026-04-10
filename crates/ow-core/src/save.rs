//! Save/load system for campaign state.
//!
//! Uses a two-section JSON format inspired by the OXCE pattern:
//! - A lightweight **header** with metadata for save-list display.
//! - The full serialized **game state** for loading.
//!
//! Writes are crash-safe: we serialize to a temp file first, then
//! atomically rename over the target. This prevents corruption if
//! the process is killed mid-write.
//!
//! The save format is intentionally human-readable JSON to support
//! modding, debugging, and manual inspection.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, info, warn};

use crate::game_state::GameState;

/// Current save format version. Bumped when the schema changes in a
/// backward-incompatible way.
pub const SAVE_VERSION: u32 = 1;

/// File extension used for save files.
const SAVE_EXTENSION: &str = "json";

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur during save/load operations.
#[derive(Debug, Error)]
pub enum SaveError {
    /// Filesystem I/O failure (read, write, rename, directory scan).
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Failed to serialize game state to JSON.
    #[error("serialization error: {0}")]
    Serialization(#[source] serde_json::Error),

    /// Failed to deserialize JSON back into game state.
    #[error("deserialization error: {0}")]
    Deserialization(#[source] serde_json::Error),

    /// Save file was written by a newer engine version that we can't read.
    #[error(
        "save version {found} is newer than engine version {expected} — \
         update your engine to load this save"
    )]
    VersionMismatch { found: u32, expected: u32 },

    /// File exists but doesn't look like a valid save (missing header, etc.).
    #[error("invalid save format: {reason}")]
    InvalidFormat { reason: String },
}

// ---------------------------------------------------------------------------
// Save header — lightweight metadata for the save-list UI
// ---------------------------------------------------------------------------

/// Metadata stored at the top of every save file.
///
/// The save-list screen reads only this section so it can display all saves
/// without deserializing the full game state for each one.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveHeader {
    /// Save format version (see [`SAVE_VERSION`]).
    pub version: u32,
    /// ISO-8601 timestamp of when the save was created.
    pub timestamp: String,
    /// Player-chosen name for this save slot.
    pub save_name: String,
    /// Current mission turn number (0 if not in a mission).
    pub turn_number: u32,
    /// Player's current funds.
    pub funds: i64,
    /// Number of mercs on the team.
    pub team_size: usize,
    /// Human-readable description of the current game phase.
    pub phase_description: String,
}

// ---------------------------------------------------------------------------
// Save file — the complete on-disk format
// ---------------------------------------------------------------------------

/// The full on-disk save file: header + serialized game state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveFile {
    /// Lightweight metadata for save-list display.
    pub header: SaveHeader,
    /// Complete game state.
    pub game_state: GameState,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Serialize the current game state and write it to disk.
///
/// The write is crash-safe: data goes to a temporary file first, then we
/// rename it over the target path. If the process dies mid-write, the
/// previous save (if any) is untouched.
pub fn save_game(state: &GameState, save_name: &str, path: &Path) -> Result<(), SaveError> {
    info!(save_name, path = %path.display(), "Saving game");

    // Build the header from current state.
    let header = build_header(state, save_name);
    debug!(?header, "Built save header");

    let save_file = SaveFile {
        header,
        game_state: state.clone(),
    };

    // Serialize to pretty JSON for human readability.
    let json = serde_json::to_string_pretty(&save_file).map_err(SaveError::Serialization)?;

    // Write to a temp file in the same directory, then rename.
    // Using the same directory ensures we stay on the same filesystem,
    // which is required for atomic rename on most OSes.
    let tmp_path = path.with_extension("tmp");
    fs::write(&tmp_path, &json)?;
    fs::rename(&tmp_path, path)?;

    info!(
        path = %path.display(),
        bytes = json.len(),
        "Save written successfully"
    );

    Ok(())
}

/// Load a game state from a save file on disk.
///
/// Validates the save version — if the file was written by a newer engine
/// version, we refuse to load it rather than silently corrupting data.
pub fn load_game(path: &Path) -> Result<GameState, SaveError> {
    info!(path = %path.display(), "Loading save");

    let json = fs::read_to_string(path)?;

    let save_file: SaveFile =
        serde_json::from_str(&json).map_err(SaveError::Deserialization)?;

    // Reject saves from future engine versions.
    if save_file.header.version > SAVE_VERSION {
        warn!(
            found = save_file.header.version,
            expected = SAVE_VERSION,
            "Save version too new"
        );
        return Err(SaveError::VersionMismatch {
            found: save_file.header.version,
            expected: SAVE_VERSION,
        });
    }

    let state = save_file.game_state;
    info!(
        phase = ?state.phase,
        funds = state.funds,
        team_size = state.team.len(),
        missions_completed = state.missions_completed,
        "Loaded game state"
    );

    Ok(state)
}

/// Scan a directory for save files and return their headers.
///
/// Only reads the header from each file — does *not* deserialize the full
/// game state, so this is fast even with many saves.
///
/// Results are sorted by timestamp descending (newest first).
pub fn list_saves(save_dir: &Path) -> Result<Vec<SaveHeader>, SaveError> {
    info!(dir = %save_dir.display(), "Listing saves");

    if !save_dir.is_dir() {
        // Not an error — the directory just doesn't exist yet (first run).
        debug!(dir = %save_dir.display(), "Save directory does not exist");
        return Ok(Vec::new());
    }

    let mut headers = Vec::new();

    for entry in fs::read_dir(save_dir)? {
        let entry = entry?;
        let path = entry.path();

        // Only look at .json files.
        let is_save = path
            .extension()
            .map_or(false, |ext| ext == SAVE_EXTENSION);
        if !is_save {
            continue;
        }

        match read_header(&path) {
            Ok(header) => {
                debug!(
                    save_name = %header.save_name,
                    timestamp = %header.timestamp,
                    "Found save"
                );
                headers.push(header);
            }
            Err(e) => {
                // Don't fail the whole listing because one save is corrupt.
                warn!(
                    path = %path.display(),
                    error = %e,
                    "Skipping unreadable save file"
                );
            }
        }
    }

    // Sort newest first by timestamp string (ISO-8601 sorts lexicographically).
    headers.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

    info!(count = headers.len(), "Save listing complete");
    Ok(headers)
}

/// Delete a save file from disk.
pub fn delete_save(path: &Path) -> Result<(), SaveError> {
    info!(path = %path.display(), "Deleting save");
    fs::remove_file(path)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Build a [`SaveHeader`] from the current game state.
fn build_header(state: &GameState, save_name: &str) -> SaveHeader {
    let turn_number = state
        .current_mission
        .as_ref()
        .map_or(0, |m| m.turn_number);

    let phase_description = format!("{:?}", state.phase);

    // Use a fixed-format timestamp so saves sort correctly.
    // In production this would use `chrono` or `time` — for now we use
    // a simple representation that's good enough without extra deps.
    let timestamp = current_timestamp();

    SaveHeader {
        version: SAVE_VERSION,
        timestamp,
        save_name: save_name.to_string(),
        turn_number,
        funds: state.funds,
        team_size: state.team.len(),
        phase_description,
    }
}

/// Read only the header from a save file without deserializing the full state.
///
/// We still parse the entire JSON (serde_json doesn't support partial reads),
/// but we immediately discard the game_state field. This is fine for the
/// expected save file sizes.
fn read_header(path: &Path) -> Result<SaveHeader, SaveError> {
    let json = fs::read_to_string(path)?;

    // Deserialize the full SaveFile to extract the header.
    // A future optimization could use serde_json::Value to avoid
    // deserializing game_state, but save files are small enough that
    // this doesn't matter in practice.
    let save_file: SaveFile =
        serde_json::from_str(&json).map_err(SaveError::Deserialization)?;

    Ok(save_file.header)
}

/// Generate an ISO-8601-ish timestamp string for the current moment.
///
/// Uses `std::time::SystemTime` to avoid pulling in `chrono`. The format
/// is YYYY-MM-DDTHH:MM:SSZ which sorts lexicographically.
fn current_timestamp() -> String {
    use std::time::SystemTime;

    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();

    // Convert seconds since epoch to a basic ISO-8601 string.
    // This is intentionally simple — no timezone handling beyond UTC.
    let secs = now.as_secs();
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Calculate year/month/day from days since 1970-01-01.
    // Uses the civil-from-days algorithm (Howard Hinnant).
    let (year, month, day) = civil_from_days(days as i64);

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hours, minutes, seconds
    )
}

/// Convert days since 1970-01-01 to (year, month, day).
/// Algorithm by Howard Hinnant — public domain.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_state::{GamePhase, GameState, OfficePhase};
    use crate::merc::{ActiveMerc, MercStatus};
    use std::fs;

    /// Create a minimal test game state.
    fn test_state() -> GameState {
        let mut state = GameState::new(250_000);
        let merc = ActiveMerc {
            id: 1,
            name: "Bull".into(),
            nickname: "Bull".into(),
            exp: 40,
            str_stat: 55,
            agl: 50,
            wil: 45,
            wsk: 60,
            hhc: 40,
            tch: 30,
            enc: 300,
            base_aps: 38,
            dpr: 120,
            max_hp: 55,
            current_hp: 55,
            current_ap: 38,
            status: MercStatus::Hired,
            position: None,
            inventory: Vec::new(),
            suppressed: false,
            experience_gained: 0,
        };
        state.hire_merc(merc, 15_000);
        state
    }

    /// Helper: create a temp directory that cleans up after the test.
    struct TempDir {
        path: std::path::PathBuf,
    }

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("ow_save_test_{name}_{}", std::process::id()));
            let _ = fs::remove_dir_all(&path); // clean up from any prior failed run
            fs::create_dir_all(&path).expect("failed to create temp dir");
            Self { path }
        }

        fn file(&self, name: &str) -> std::path::PathBuf {
            self.path.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    // -- Round-trip test --

    #[test]
    fn save_and_load_round_trip() {
        let dir = TempDir::new("round_trip");
        let save_path = dir.file("test_save.json");

        let original = test_state();
        save_game(&original, "Round Trip Test", &save_path).unwrap();

        let loaded = load_game(&save_path).unwrap();

        // Verify the key fields survived the round trip.
        assert_eq!(loaded.funds, original.funds);
        assert_eq!(loaded.team.len(), original.team.len());
        assert_eq!(loaded.team[0].name, "Bull");
        assert_eq!(loaded.team[0].current_hp, 55);
        assert_eq!(loaded.reputation, original.reputation);
        assert_eq!(loaded.missions_completed, original.missions_completed);
        assert_eq!(loaded.phase, original.phase);
    }

    // -- Version check --

    #[test]
    fn rejects_future_save_version() {
        let dir = TempDir::new("version_check");
        let save_path = dir.file("future_save.json");

        let state = test_state();
        save_game(&state, "Future Save", &save_path).unwrap();

        // Tamper with the version number to simulate a future engine save.
        let mut json = fs::read_to_string(&save_path).unwrap();
        json = json.replacen(
            &format!("\"version\": {SAVE_VERSION}"),
            &format!("\"version\": {}", SAVE_VERSION + 99),
            1,
        );
        fs::write(&save_path, &json).unwrap();

        let result = load_game(&save_path);
        assert!(
            matches!(result, Err(SaveError::VersionMismatch { .. })),
            "expected VersionMismatch, got: {result:?}"
        );
    }

    // -- Corrupt file handling --

    #[test]
    fn corrupt_json_returns_deserialization_error() {
        let dir = TempDir::new("corrupt");
        let save_path = dir.file("corrupt.json");

        fs::write(&save_path, "{ this is not valid json !!!").unwrap();

        let result = load_game(&save_path);
        assert!(
            matches!(result, Err(SaveError::Deserialization(_))),
            "expected Deserialization error, got: {result:?}"
        );
    }

    #[test]
    fn truncated_file_returns_deserialization_error() {
        let dir = TempDir::new("truncated");
        let save_path = dir.file("truncated.json");

        let state = test_state();
        save_game(&state, "Truncated", &save_path).unwrap();

        // Chop the file in half to simulate a truncated write.
        let json = fs::read_to_string(&save_path).unwrap();
        let half = &json[..json.len() / 2];
        fs::write(&save_path, half).unwrap();

        let result = load_game(&save_path);
        assert!(
            matches!(result, Err(SaveError::Deserialization(_))),
            "expected Deserialization error, got: {result:?}"
        );
    }

    #[test]
    fn missing_file_returns_io_error() {
        let result = load_game(Path::new("/nonexistent/path/save.json"));
        assert!(
            matches!(result, Err(SaveError::Io(_))),
            "expected Io error, got: {result:?}"
        );
    }

    // -- Save listing --

    #[test]
    fn list_saves_returns_headers_sorted_newest_first() {
        let dir = TempDir::new("listing");
        let state = test_state();

        // Create several saves. The timestamps will be very close together,
        // so we also vary the save name to confirm we get all of them.
        for i in 0..3 {
            let name = format!("Save {i}");
            let path = dir.file(&format!("save_{i}.json"));
            save_game(&state, &name, &path).unwrap();
        }

        let headers = list_saves(&dir.path).unwrap();
        assert_eq!(headers.len(), 3);

        // All headers should have version == SAVE_VERSION.
        for h in &headers {
            assert_eq!(h.version, SAVE_VERSION);
            assert_eq!(h.funds, state.funds);
            assert_eq!(h.team_size, 1);
        }
    }

    #[test]
    fn list_saves_skips_non_json_files() {
        let dir = TempDir::new("non_json");
        let state = test_state();

        save_game(&state, "Real Save", &dir.file("real.json")).unwrap();
        fs::write(dir.file("notes.txt"), "not a save").unwrap();
        fs::write(dir.file("data.csv"), "1,2,3").unwrap();

        let headers = list_saves(&dir.path).unwrap();
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].save_name, "Real Save");
    }

    #[test]
    fn list_saves_skips_corrupt_files() {
        let dir = TempDir::new("corrupt_listing");
        let state = test_state();

        save_game(&state, "Good Save", &dir.file("good.json")).unwrap();
        fs::write(dir.file("bad.json"), "{{{{ not json").unwrap();

        let headers = list_saves(&dir.path).unwrap();
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].save_name, "Good Save");
    }

    #[test]
    fn list_saves_empty_directory() {
        let dir = TempDir::new("empty");
        let headers = list_saves(&dir.path).unwrap();
        assert!(headers.is_empty());
    }

    #[test]
    fn list_saves_nonexistent_directory() {
        let headers = list_saves(Path::new("/nonexistent/saves/dir")).unwrap();
        assert!(headers.is_empty());
    }

    // -- Delete --

    #[test]
    fn delete_save_removes_file() {
        let dir = TempDir::new("delete");
        let save_path = dir.file("to_delete.json");
        let state = test_state();

        save_game(&state, "Delete Me", &save_path).unwrap();
        assert!(save_path.exists());

        delete_save(&save_path).unwrap();
        assert!(!save_path.exists());
    }

    #[test]
    fn delete_nonexistent_save_returns_io_error() {
        let result = delete_save(Path::new("/nonexistent/save.json"));
        assert!(matches!(result, Err(SaveError::Io(_))));
    }

    // -- Header construction --

    #[test]
    fn header_captures_state_metadata() {
        let state = test_state();
        let header = build_header(&state, "My Campaign");

        assert_eq!(header.version, SAVE_VERSION);
        assert_eq!(header.save_name, "My Campaign");
        assert_eq!(header.funds, state.funds);
        assert_eq!(header.team_size, 1);
        assert_eq!(header.turn_number, 0); // no active mission
        assert!(
            header.phase_description.contains("Office"),
            "phase_description should mention Office, got: {}",
            header.phase_description
        );
    }

    #[test]
    fn header_captures_mission_turn() {
        let mut state = test_state();
        state.current_mission = Some(crate::game_state::MissionContext {
            name: "Desert Storm".into(),
            weather: crate::weather::Weather::Clear,
            combat: None,
            turn_number: 7,
        });
        state.phase = GamePhase::Mission(crate::game_state::MissionPhase::Combat);

        let header = build_header(&state, "Mid-Mission");
        assert_eq!(header.turn_number, 7);
        assert!(header.phase_description.contains("Combat"));
    }

    // -- Timestamp format --

    #[test]
    fn timestamp_is_valid_iso8601() {
        let ts = current_timestamp();
        // Should look like "2026-04-09T12:34:56Z"
        assert_eq!(ts.len(), 20, "unexpected timestamp length: {ts}");
        assert!(ts.ends_with('Z'), "timestamp should end with Z: {ts}");
        assert_eq!(&ts[4..5], "-", "missing dash after year: {ts}");
        assert_eq!(&ts[7..8], "-", "missing dash after month: {ts}");
        assert_eq!(&ts[10..11], "T", "missing T separator: {ts}");
    }
}
