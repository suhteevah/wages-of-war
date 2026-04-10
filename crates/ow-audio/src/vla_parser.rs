//! VLA/VLS ("VALS") custom audio container parser.
//!
//! The original game uses a proprietary container format for voiced dialogue lines.
//! Files have `.VLA` (Voice + Lip Animation) or `.VLS` (Voice + Lip + Subtitles)
//! extensions, but the binary format is identical — both begin with "VALS" magic.
//!
//! # Format Layout (reverse-engineered from game data)
//!
//! ```text
//! Offset  Size    Content
//! ──────  ──────  ──────────────────────────────────────────────
//! 0x00    4       Magic: "VALS" (0x56 0x41 0x4C 0x53)
//! 0x04    u32     index_size — byte count from offset 0x08 to start of WRDS section
//! 0x08    u32     flags — 0xFFFFFFFF or 0x00000000 (purpose unclear)
//! 0x0C    u32     sentence_count — number of dialogue sentences/segments
//! 0x10    ...     Lip-sync index entries (variable count)
//!                 Each entry: i32 mouth_shape (-1 = closed), u32 sample_offset
//! +var    4       "WRDS" marker
//! +4      u32     wrds_size — byte count of word timing data that follows
//! +8      ...     Word boundary pairs: (u32 start_sample, u32 end_sample) × N
//!                 These are sample offsets at 22050 Hz for subtitle/lip-sync word timing
//! +var    4       "WAVE" marker
//! +4      u32     wave_size — byte count of the embedded RIFF/WAV that follows
//! +8      ...     Complete standard RIFF/WAV file (PCM, mono, 8-bit, 22050 Hz typical)
//! EOF
//! ```
//!
//! The embedded WAV is identical to the standalone `.WAV` file when one exists alongside
//! the VLA/VLS (confirmed by byte-for-byte comparison of ARTIE01.VLS vs ARTIE01.WAV).
//!
//! # File locations
//!
//! All VLA/VLS files live in `WOW/WAV/` alongside standard WAV sound effects.
//! - `MISHNxxY.VLA/VLS` — Mission briefing voice lines (A/B/C variants)
//! - `ARTIExx.VLS` — Artie (arms dealer) dialogue
//! - `VINNIExx.VLS` — Vinnie dialogue
//! - `MOMxx.VLS` — Mom dialogue
//! - `ACCT.VLA` — Accountant dialogue
//! - `SHARK.VLA` — Loan shark dialogue

use std::path::Path;

use tracing::{debug, info, trace, warn};

use crate::AudioError;

/// The 4-byte magic signature at the start of every VLA/VLS file.
const VALS_MAGIC: &[u8; 4] = b"VALS";

/// The 4-byte marker for the word-timing section.
const WRDS_MARKER: &[u8; 4] = b"WRDS";

/// The 4-byte marker for the embedded WAV section.
const WAVE_MARKER: &[u8; 4] = b"WAVE";

/// A single lip-sync animation keyframe.
///
/// Each entry maps a sample offset in the audio to a mouth shape index.
/// The game's character portrait renderer uses these to animate the speaker's mouth.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LipSyncEntry {
    /// Mouth shape index. Values 0..~19 observed in game data.
    /// `-1` (0xFFFFFFFF) means "mouth closed" (silence or pause).
    pub mouth_shape: i32,

    /// Sample offset into the audio data where this mouth shape begins.
    /// At 22050 Hz mono 8-bit, each sample = ~0.045 ms.
    pub sample_offset: u32,
}

/// A word boundary in the audio, used for subtitle timing.
///
/// Each entry defines the sample range for one spoken word.
/// The game displays subtitle text word-by-word, highlighting each word
/// as it is spoken.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WordTiming {
    /// Sample offset where this word starts being spoken.
    pub start_sample: u32,

    /// Sample offset where this word ends.
    pub end_sample: u32,
}

impl WordTiming {
    /// Convert the start sample to milliseconds, given a sample rate.
    pub fn start_ms(&self, sample_rate: u32) -> u32 {
        if sample_rate == 0 {
            return 0;
        }
        (self.start_sample as u64 * 1000 / sample_rate as u64) as u32
    }

    /// Convert the end sample to milliseconds, given a sample rate.
    pub fn end_ms(&self, sample_rate: u32) -> u32 {
        if sample_rate == 0 {
            return 0;
        }
        (self.end_sample as u64 * 1000 / sample_rate as u64) as u32
    }
}

/// A fully parsed VALS container file (VLA or VLS).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ValsFile {
    /// The original "VALS" magic bytes (always `b"VALS"`).
    pub magic: [u8; 4],

    /// Flags field from offset 0x08. Observed values: 0x00000000 or 0xFFFFFFFF.
    /// Purpose not fully understood — may distinguish VLA from VLS behavior.
    pub flags: u32,

    /// Number of dialogue sentences/segments declared in the header.
    pub sentence_count: u32,

    /// Lip-sync animation keyframes, ordered by sample offset.
    /// Each entry maps an audio position to a mouth shape for character animation.
    pub lip_sync: Vec<LipSyncEntry>,

    /// Word boundary timing data from the WRDS section.
    /// Each entry gives the sample range for one spoken word.
    /// Used for subtitle display and lip-sync word highlighting.
    pub word_timings: Vec<WordTiming>,

    /// The embedded RIFF/WAV audio data, extracted verbatim.
    /// This is a complete, valid WAV file that can be played directly.
    pub embedded_wav_data: Vec<u8>,
}

/// Reads a little-endian u32 from a byte slice at the given offset.
///
/// Returns an error if the slice is too short.
fn read_u32(data: &[u8], offset: usize, path: &Path, context: &str) -> Result<u32, AudioError> {
    if offset + 4 > data.len() {
        return Err(AudioError::invalid_format(
            path,
            format!(
                "unexpected EOF reading u32 at offset 0x{:X} ({}); file is {} bytes",
                offset,
                context,
                data.len()
            ),
        ));
    }
    Ok(u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]))
}

/// Reads a little-endian i32 from a byte slice at the given offset.
fn read_i32(data: &[u8], offset: usize, path: &Path, context: &str) -> Result<i32, AudioError> {
    if offset + 4 > data.len() {
        return Err(AudioError::invalid_format(
            path,
            format!(
                "unexpected EOF reading i32 at offset 0x{:X} ({}); file is {} bytes",
                offset,
                context,
                data.len()
            ),
        ));
    }
    Ok(i32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]))
}

/// Searches for a 4-byte marker in the data starting from a given offset.
///
/// Returns the offset of the first occurrence, or `None` if not found.
fn find_marker(data: &[u8], marker: &[u8; 4], start: usize) -> Option<usize> {
    if data.len() < start + 4 {
        return None;
    }
    data[start..]
        .windows(4)
        .position(|w| w == marker)
        .map(|pos| pos + start)
}

/// Checks whether the given data starts with the VALS magic signature.
///
/// Useful for quickly validating a file before attempting a full parse.
pub fn has_vals_magic(data: &[u8]) -> bool {
    data.len() >= 4 && &data[0..4] == VALS_MAGIC
}

/// Parses a VLA or VLS file from disk.
///
/// Reads the entire file into memory and extracts:
/// 1. Lip-sync animation entries (mouth shapes keyed to audio sample offsets)
/// 2. Word timing boundaries from the WRDS section
/// 3. The embedded RIFF/WAV audio data
///
/// # Arguments
/// * `path` — Path to a `.VLA` or `.VLS` file.
///
/// # Errors
/// - [`AudioError::NotFound`] if the file does not exist.
/// - [`AudioError::Io`] if the file cannot be read.
/// - [`AudioError::InvalidFormat`] if the magic is wrong, sections are missing,
///   or the file is truncated.
pub fn parse_vals(path: &Path) -> Result<ValsFile, AudioError> {
    if !path.exists() {
        return Err(AudioError::not_found(path));
    }

    info!(path = %path.display(), "parsing VALS file");

    let data = std::fs::read(path).map_err(|e| AudioError::io(path, e))?;

    parse_vals_from_bytes(&data, path)
}

/// Parses VALS format from an in-memory byte buffer.
///
/// This is the inner implementation, separated from I/O for testability.
pub fn parse_vals_from_bytes(data: &[u8], path: &Path) -> Result<ValsFile, AudioError> {
    // -- Validate magic --
    if !has_vals_magic(data) {
        let actual = if data.len() >= 4 {
            format!("{:02X} {:02X} {:02X} {:02X}", data[0], data[1], data[2], data[3])
        } else {
            format!("only {} bytes", data.len())
        };
        return Err(AudioError::invalid_format(
            path,
            format!("expected VALS magic, got: {}", actual),
        ));
    }

    let magic = [data[0], data[1], data[2], data[3]];

    // -- Parse header --
    let index_size = read_u32(data, 0x04, path, "index_size")?;
    let flags = read_u32(data, 0x08, path, "flags")?;
    let sentence_count = read_u32(data, 0x0C, path, "sentence_count")?;

    debug!(
        index_size = index_size,
        flags = format_args!("0x{:08X}", flags),
        sentence_count = sentence_count,
        "VALS header parsed"
    );

    // The index region runs from 0x10 to 0x08 + index_size.
    // Each entry is 8 bytes: i32 mouth_shape + u32 sample_offset.
    let index_end = 0x08usize.checked_add(index_size as usize).ok_or_else(|| {
        AudioError::invalid_format(path, "index_size overflow")
    })?;

    if index_end > data.len() {
        return Err(AudioError::invalid_format(
            path,
            format!(
                "index_size 0x{:X} extends past end of file ({}  bytes)",
                index_size,
                data.len()
            ),
        ));
    }

    // -- Parse lip-sync entries --
    let lip_sync_region = &data[0x10..index_end];
    if lip_sync_region.len() % 8 != 0 {
        warn!(
            remainder = lip_sync_region.len() % 8,
            "lip-sync region size is not a multiple of 8; trailing bytes will be ignored"
        );
    }
    let lip_sync_count = lip_sync_region.len() / 8;
    let mut lip_sync = Vec::with_capacity(lip_sync_count);

    for i in 0..lip_sync_count {
        let base = 0x10 + i * 8;
        let mouth_shape = read_i32(data, base, path, "lip_sync.mouth_shape")?;
        let sample_offset = read_u32(data, base + 4, path, "lip_sync.sample_offset")?;
        lip_sync.push(LipSyncEntry {
            mouth_shape,
            sample_offset,
        });
    }

    trace!(count = lip_sync.len(), "parsed lip-sync entries");

    // -- Find and parse WRDS section --
    let wrds_offset = find_marker(data, WRDS_MARKER, index_end).ok_or_else(|| {
        AudioError::invalid_format(path, "WRDS marker not found after index region")
    })?;

    let wrds_size = read_u32(data, wrds_offset + 4, path, "wrds_size")? as usize;
    let wrds_data_start = wrds_offset + 8;
    let wrds_data_end = wrds_data_start + wrds_size;

    if wrds_data_end > data.len() {
        return Err(AudioError::invalid_format(
            path,
            format!(
                "WRDS section (0x{:X} + {}) extends past EOF",
                wrds_offset, wrds_size
            ),
        ));
    }

    // Word timings are stored as pairs of u32 (start_sample, end_sample).
    if wrds_size % 8 != 0 {
        warn!(
            wrds_size = wrds_size,
            "WRDS size is not a multiple of 8; trailing bytes ignored"
        );
    }
    let word_count = wrds_size / 8;
    let mut word_timings = Vec::with_capacity(word_count);

    for i in 0..word_count {
        let base = wrds_data_start + i * 8;
        let start_sample = read_u32(data, base, path, "wrds.start_sample")?;
        let end_sample = read_u32(data, base + 4, path, "wrds.end_sample")?;
        word_timings.push(WordTiming {
            start_sample,
            end_sample,
        });
    }

    debug!(
        wrds_offset = format_args!("0x{:X}", wrds_offset),
        word_count = word_timings.len(),
        "parsed WRDS word timings"
    );

    // -- Find and extract embedded WAV --
    let wave_offset = find_marker(data, WAVE_MARKER, wrds_data_end).ok_or_else(|| {
        AudioError::invalid_format(path, "WAVE marker not found after WRDS section")
    })?;

    let wave_size = read_u32(data, wave_offset + 4, path, "wave_size")? as usize;
    let wav_data_start = wave_offset + 8;
    let wav_data_end = wav_data_start + wave_size;

    if wav_data_end > data.len() {
        return Err(AudioError::invalid_format(
            path,
            format!(
                "WAVE section (0x{:X} + {}) extends past EOF ({})",
                wave_offset,
                wave_size,
                data.len()
            ),
        ));
    }

    // The embedded data should start with "RIFF" (standard WAV container).
    if wav_data_start + 4 <= data.len() && &data[wav_data_start..wav_data_start + 4] != b"RIFF" {
        warn!(
            offset = format_args!("0x{:X}", wav_data_start),
            "embedded WAV data does not start with RIFF magic"
        );
    }

    let embedded_wav_data = data[wav_data_start..wav_data_end].to_vec();

    info!(
        lip_sync_entries = lip_sync.len(),
        word_timings = word_timings.len(),
        wav_bytes = embedded_wav_data.len(),
        "VALS parse complete"
    );

    Ok(ValsFile {
        magic,
        flags,
        sentence_count,
        lip_sync,
        word_timings,
        embedded_wav_data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal valid VALS file in memory for testing.
    fn build_test_vals(
        flags: u32,
        sentence_count: u32,
        lip_entries: &[(i32, u32)],
        word_pairs: &[(u32, u32)],
    ) -> Vec<u8> {
        let mut buf = Vec::new();

        // -- Lip-sync index data --
        let mut lip_data = Vec::new();
        for &(shape, offset) in lip_entries {
            lip_data.extend_from_slice(&shape.to_le_bytes());
            lip_data.extend_from_slice(&offset.to_le_bytes());
        }

        // index_size = distance from 0x08 to WRDS marker
        // WRDS is at 0x08 + index_size, so index_size = 0x08 + lip_data.len()
        // Wait: index_end = 0x08 + index_size, and lip data starts at 0x10.
        // So lip_data occupies 0x10..index_end, meaning index_size = 0x08 + lip_data.len().
        let index_size = 0x08u32 + lip_data.len() as u32;

        // VALS header
        buf.extend_from_slice(b"VALS");
        buf.extend_from_slice(&index_size.to_le_bytes());
        buf.extend_from_slice(&flags.to_le_bytes());
        buf.extend_from_slice(&sentence_count.to_le_bytes());

        // Lip-sync entries
        buf.extend_from_slice(&lip_data);

        // WRDS section
        let wrds_data_size = (word_pairs.len() * 8) as u32;
        buf.extend_from_slice(b"WRDS");
        buf.extend_from_slice(&wrds_data_size.to_le_bytes());
        for &(start, end) in word_pairs {
            buf.extend_from_slice(&start.to_le_bytes());
            buf.extend_from_slice(&end.to_le_bytes());
        }

        // WAVE section with a minimal embedded RIFF/WAV
        let wav_data: Vec<u8> = {
            let mut w = Vec::new();
            let pcm_data = [0x80u8; 100]; // 100 samples of silence (8-bit unsigned)
            let data_chunk_size = pcm_data.len() as u32;
            let riff_size = 4 + 24 + 8 + data_chunk_size; // WAVE + fmt chunk + data header + data
            w.extend_from_slice(b"RIFF");
            w.extend_from_slice(&riff_size.to_le_bytes());
            w.extend_from_slice(b"WAVE");
            w.extend_from_slice(b"fmt ");
            w.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
            w.extend_from_slice(&1u16.to_le_bytes()); // PCM format
            w.extend_from_slice(&1u16.to_le_bytes()); // mono
            w.extend_from_slice(&22050u32.to_le_bytes()); // sample rate
            w.extend_from_slice(&22050u32.to_le_bytes()); // byte rate
            w.extend_from_slice(&1u16.to_le_bytes()); // block align
            w.extend_from_slice(&8u16.to_le_bytes()); // bits per sample
            w.extend_from_slice(b"data");
            w.extend_from_slice(&data_chunk_size.to_le_bytes());
            w.extend_from_slice(&pcm_data);
            w
        };

        buf.extend_from_slice(b"WAVE");
        buf.extend_from_slice(&(wav_data.len() as u32).to_le_bytes());
        buf.extend_from_slice(&wav_data);

        buf
    }

    #[test]
    fn has_vals_magic_detects_valid_signature() {
        assert!(has_vals_magic(b"VALS\x00\x00\x00\x00"));
        assert!(has_vals_magic(b"VALSextra"));
        assert!(!has_vals_magic(b"RIFF"));
        assert!(!has_vals_magic(b"VAL")); // too short
        assert!(!has_vals_magic(b""));
    }

    #[test]
    fn parse_minimal_vals_file() {
        let lip_entries = vec![
            (13i32, 100u32), // mouth shape 13 at sample 100
            (-1, 200),       // closed mouth at sample 200
            (6, 350),        // mouth shape 6 at sample 350
        ];
        let word_pairs = vec![
            (100u32, 199u32), // word 1: samples 100-199
            (200, 350),       // word 2: samples 200-350
        ];

        let data = build_test_vals(0xFFFFFFFF, 1, &lip_entries, &word_pairs);
        let path = Path::new("test.VLA");
        let result = parse_vals_from_bytes(&data, path).unwrap();

        assert_eq!(&result.magic, b"VALS");
        assert_eq!(result.flags, 0xFFFFFFFF);
        assert_eq!(result.sentence_count, 1);

        // Lip-sync entries
        assert_eq!(result.lip_sync.len(), 3);
        assert_eq!(result.lip_sync[0].mouth_shape, 13);
        assert_eq!(result.lip_sync[0].sample_offset, 100);
        assert_eq!(result.lip_sync[1].mouth_shape, -1);
        assert_eq!(result.lip_sync[1].sample_offset, 200);
        assert_eq!(result.lip_sync[2].mouth_shape, 6);
        assert_eq!(result.lip_sync[2].sample_offset, 350);

        // Word timings
        assert_eq!(result.word_timings.len(), 2);
        assert_eq!(result.word_timings[0].start_sample, 100);
        assert_eq!(result.word_timings[0].end_sample, 199);
        assert_eq!(result.word_timings[1].start_sample, 200);
        assert_eq!(result.word_timings[1].end_sample, 350);

        // Embedded WAV starts with RIFF
        assert!(result.embedded_wav_data.starts_with(b"RIFF"));
        assert!(result.embedded_wav_data.len() > 44); // at least a WAV header
    }

    #[test]
    fn parse_rejects_wrong_magic() {
        let data = b"RIFF\x00\x00\x00\x00not a vals file";
        let result = parse_vals_from_bytes(data, Path::new("bad.VLA"));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, AudioError::InvalidFormat { .. }));
    }

    #[test]
    fn parse_rejects_truncated_header() {
        let data = b"VAL"; // too short for magic
        let result = parse_vals_from_bytes(data, Path::new("short.VLA"));
        assert!(result.is_err());
    }

    #[test]
    fn parse_empty_lip_sync_and_words() {
        // A VALS file with zero lip-sync entries and zero word timings is valid
        // (degenerate but structurally sound).
        let data = build_test_vals(0, 0, &[], &[]);
        let result = parse_vals_from_bytes(&data, Path::new("empty.VLA")).unwrap();
        assert!(result.lip_sync.is_empty());
        assert!(result.word_timings.is_empty());
        assert!(result.embedded_wav_data.starts_with(b"RIFF"));
    }

    #[test]
    fn word_timing_millisecond_conversion() {
        let wt = WordTiming {
            start_sample: 22050,
            end_sample: 44100,
        };
        assert_eq!(wt.start_ms(22050), 1000);
        assert_eq!(wt.end_ms(22050), 2000);

        // Zero sample rate should not panic
        assert_eq!(wt.start_ms(0), 0);
    }
}
