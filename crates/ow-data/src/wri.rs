//! Parser for Windows Write (`.WRI`) files — binary rich text from Windows 3.1.
//!
//! The game stores contract descriptions and mission briefings for all 16
//! missions as `.WRI` files (`CONTR##.WRI`, `BRIEF##A.WRI`, `BRIEF##B.WRI`).
//! Missions 1–3 also have plaintext `.DAT` equivalents, but missions 4–16
//! exist only in `.WRI` format, making this parser essential.
//!
//! # Format overview
//!
//! A `.WRI` file is a simple binary document format:
//!
//! | Offset | Size | Field | Description |
//! |--------|------|-------|-------------|
//! | 0x00   | 2    | magic | `0xBE31` — Windows Write signature |
//! | 0x04   | 2    | —     | `0x00AB` — secondary magic word |
//! | 0x0E   | 4    | fcMac | Byte offset of end-of-text (measured from file start) |
//! | 0x80   | —    | text  | Body text in Windows-1252 encoding, `\r\n` line endings |
//!
//! Everything after `fcMac` is formatting metadata (font tables, paragraph
//! runs, character properties) followed by a partial text duplicate — a known
//! Write format artifact. We ignore all of it and extract only the raw text.
//!
//! # Encoding notes
//!
//! The `.DAT` plaintext files use Windows-1252 `0x92` for right single quote.
//! The `.WRI` files instead use `0xC6` for the same character — likely a
//! Write-internal encoding quirk. Both are mapped to Unicode `U+2019` (')
//! during text extraction.

use std::io;
use std::path::Path;

use tracing::{debug, info, trace};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Windows Write magic number at offset 0x00 (little-endian u16).
const WRI_MAGIC_PRIMARY: u16 = 0xBE31;

/// Secondary magic word at offset 0x04 (little-endian u16).
const WRI_MAGIC_SECONDARY: u16 = 0x00AB;

/// Fixed byte offset where body text begins in every .WRI file.
const TEXT_START_OFFSET: usize = 0x80;

/// Minimum valid file size: 128-byte header + at least 1 byte of text.
const MIN_FILE_SIZE: usize = TEXT_START_OFFSET + 1;

/// Offset of the `fcMac` field (end-of-text pointer), a little-endian u32.
const FC_MAC_OFFSET: usize = 0x0E;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur when parsing `.WRI` files.
#[derive(Debug, thiserror::Error)]
pub enum WriError {
    /// Underlying I/O error when reading the file.
    #[error("I/O error reading .WRI file: {0}")]
    Io(#[from] io::Error),

    /// The file does not have a valid Windows Write header.
    #[error("invalid .WRI format: {0}")]
    InvalidFormat(String),

    /// The text region could not be extracted (e.g. fcMac points outside file).
    #[error("text extraction failed: {0}")]
    TextExtraction(String),
}

// ---------------------------------------------------------------------------
// Output types
// ---------------------------------------------------------------------------

/// A parsed Windows Write document with its text content.
///
/// Formatting information (fonts, bold/italic runs) is intentionally discarded;
/// only the plaintext body is preserved.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WriDocument {
    /// The full document text as a single UTF-8 string.
    /// Line endings are normalized to `\n` (Unix-style).
    pub text: String,

    /// The text split into paragraphs. A paragraph boundary is defined as
    /// one or more consecutive blank lines (or a single `\n` within the
    /// original `\r\n`-delimited text). Leading/trailing whitespace on each
    /// paragraph is trimmed, and empty paragraphs are removed.
    pub paragraphs: Vec<String>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parse a Windows Write (`.WRI`) file and extract its plaintext content.
///
/// The parser validates the Write magic bytes, reads the `fcMac` end-of-text
/// pointer from the header, extracts the text region at offset `0x80`, converts
/// from the Write character encoding to UTF-8, and splits into paragraphs.
///
/// # Errors
///
/// Returns [`WriError::Io`] on file-read failures, [`WriError::InvalidFormat`]
/// if magic bytes are wrong or the file is too small, and
/// [`WriError::TextExtraction`] if the text region is malformed.
pub fn parse_wri(path: &Path) -> Result<WriDocument, WriError> {
    info!(path = %path.display(), "parsing .WRI file");

    let data = std::fs::read(path).map_err(|e| {
        debug!(path = %path.display(), error = %e, "failed to read .WRI file");
        e
    })?;

    parse_wri_bytes(&data, path)
}

/// Inner parsing logic that operates on an already-loaded byte buffer.
/// `source_path` is used only for log messages.
fn parse_wri_bytes(data: &[u8], source_path: &Path) -> Result<WriDocument, WriError> {
    // --- Header validation ---------------------------------------------------

    if data.len() < MIN_FILE_SIZE {
        return Err(WriError::InvalidFormat(format!(
            "file is {} bytes, minimum is {}",
            data.len(),
            MIN_FILE_SIZE
        )));
    }

    let magic_primary = u16::from_le_bytes([data[0], data[1]]);
    let magic_secondary = u16::from_le_bytes([data[4], data[5]]);

    if magic_primary != WRI_MAGIC_PRIMARY {
        return Err(WriError::InvalidFormat(format!(
            "expected primary magic 0x{WRI_MAGIC_PRIMARY:04X}, got 0x{magic_primary:04X}"
        )));
    }
    if magic_secondary != WRI_MAGIC_SECONDARY {
        return Err(WriError::InvalidFormat(format!(
            "expected secondary magic 0x{WRI_MAGIC_SECONDARY:04X}, got 0x{magic_secondary:04X}"
        )));
    }

    trace!(
        path = %source_path.display(),
        file_size = data.len(),
        "header magic validated"
    );

    // --- Locate text region --------------------------------------------------

    let fc_mac = u32::from_le_bytes([
        data[FC_MAC_OFFSET],
        data[FC_MAC_OFFSET + 1],
        data[FC_MAC_OFFSET + 2],
        data[FC_MAC_OFFSET + 3],
    ]) as usize;

    debug!(
        path = %source_path.display(),
        fc_mac = fc_mac,
        text_start = TEXT_START_OFFSET,
        text_len = fc_mac.saturating_sub(TEXT_START_OFFSET),
        "located text region"
    );

    if fc_mac <= TEXT_START_OFFSET {
        return Err(WriError::TextExtraction(format!(
            "fcMac (0x{fc_mac:04X}) is at or before text start (0x{TEXT_START_OFFSET:02X})"
        )));
    }
    if fc_mac > data.len() {
        return Err(WriError::TextExtraction(format!(
            "fcMac (0x{fc_mac:04X}) exceeds file size (0x{:04X})",
            data.len()
        )));
    }

    let raw_text = &data[TEXT_START_OFFSET..fc_mac];

    // --- Character encoding conversion ---------------------------------------

    let text = decode_wri_text(raw_text);

    trace!(
        path = %source_path.display(),
        text_chars = text.len(),
        "text decoded to UTF-8"
    );

    // --- Paragraph splitting -------------------------------------------------

    let paragraphs = split_paragraphs(&text);

    debug!(
        path = %source_path.display(),
        paragraph_count = paragraphs.len(),
        "split into paragraphs"
    );

    Ok(WriDocument { text, paragraphs })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Decode a raw byte slice from the Write-internal encoding to a UTF-8 String.
///
/// - Strips carriage returns (`\r`) so line endings become plain `\n`.
/// - Maps `0xC6` to right single quote (`\u{2019}`) — the Write-specific
///   apostrophe encoding observed in the Wages of War `.WRI` files.
/// - Maps `0x92` to right single quote as well (Windows-1252 encoding used
///   in the `.DAT` files; included for robustness).
/// - Passes through printable ASCII, tabs, and newlines.
/// - Replaces any other control characters (0x00–0x08, 0x0E–0x1F, 0x7F)
///   with the Unicode replacement character.
fn decode_wri_text(raw: &[u8]) -> String {
    let mut result = String::with_capacity(raw.len());

    for &byte in raw {
        match byte {
            // Strip carriage return — we keep only \n for line endings
            b'\r' => {}

            // Printable ASCII + whitespace pass through
            b'\n' | b'\t' | 0x20..=0x7E => result.push(byte as char),

            // Write-internal right single quote (observed in WoW .WRI files)
            0xC6 => result.push('\u{2019}'),

            // Windows-1252 right single quote (belt-and-suspenders)
            0x92 => result.push('\u{2019}'),

            // Other Windows-1252 high bytes: decode via the standard mapping.
            // For the bytes we've observed in game files this covers accented
            // Latin characters like e-acute (0xE9).
            0x80..=0xFF => {
                result.push(windows_1252_to_char(byte));
            }

            // Control characters that aren't tab/newline — replace
            _ => {
                trace!(byte = byte, "replacing control character");
                result.push('\u{FFFD}');
            }
        }
    }

    result
}

/// Map a Windows-1252 byte (0x80–0xFF) to its Unicode character.
///
/// The 0x80–0x9F range is the only region where Windows-1252 diverges from
/// ISO-8859-1. For 0xA0–0xFF the mapping is identity (same codepoint).
fn windows_1252_to_char(byte: u8) -> char {
    // We already handle 0x92 and 0xC6 before calling this function,
    // but include them in the table for completeness.
    match byte {
        0x80 => '\u{20AC}', // Euro sign
        0x82 => '\u{201A}', // Single low-9 quotation mark
        0x83 => '\u{0192}', // Latin small letter f with hook
        0x84 => '\u{201E}', // Double low-9 quotation mark
        0x85 => '\u{2026}', // Horizontal ellipsis
        0x86 => '\u{2020}', // Dagger
        0x87 => '\u{2021}', // Double dagger
        0x88 => '\u{02C6}', // Modifier letter circumflex accent
        0x89 => '\u{2030}', // Per mille sign
        0x8A => '\u{0160}', // Latin capital letter S with caron
        0x8B => '\u{2039}', // Single left-pointing angle quotation mark
        0x8C => '\u{0152}', // Latin capital ligature OE
        0x8E => '\u{017D}', // Latin capital letter Z with caron
        0x91 => '\u{2018}', // Left single quotation mark
        0x92 => '\u{2019}', // Right single quotation mark
        0x93 => '\u{201C}', // Left double quotation mark
        0x94 => '\u{201D}', // Right double quotation mark
        0x95 => '\u{2022}', // Bullet
        0x96 => '\u{2013}', // En dash
        0x97 => '\u{2014}', // Em dash
        0x98 => '\u{02DC}', // Small tilde
        0x99 => '\u{2122}', // Trade mark sign
        0x9A => '\u{0161}', // Latin small letter s with caron
        0x9B => '\u{203A}', // Single right-pointing angle quotation mark
        0x9C => '\u{0153}', // Latin small ligature oe
        0x9E => '\u{017E}', // Latin small letter z with caron
        0x9F => '\u{0178}', // Latin capital letter Y with diaeresis
        // 0x81, 0x8D, 0x8F, 0x90 are undefined in Windows-1252
        0x81 | 0x8D | 0x8F | 0x90 => '\u{FFFD}',
        // 0xA0–0xFF: identity mapping to Unicode
        b => char::from(b),
    }
}

/// Split decoded text into paragraphs.
///
/// Paragraphs are separated by one or more blank lines. Each paragraph is
/// trimmed of leading/trailing whitespace. Empty paragraphs are discarded.
fn split_paragraphs(text: &str) -> Vec<String> {
    text.split("\n\n")
        .map(|chunk| {
            // Within a paragraph, collapse internal newlines to spaces —
            // the Write format hard-wraps lines at column ~70, but these
            // are soft wraps within a single logical paragraph.
            let collapsed: String = chunk
                .lines()
                .map(str::trim)
                .collect::<Vec<_>>()
                .join(" ");
            collapsed.trim().to_string()
        })
        .filter(|p| !p.is_empty())
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal valid .WRI file from the given body text.
    /// Sets magic bytes and fcMac, fills the rest of the 128-byte header with zeros.
    fn make_wri(body: &[u8]) -> Vec<u8> {
        let fc_mac = (TEXT_START_OFFSET + body.len()) as u32;
        let mut buf = vec![0u8; TEXT_START_OFFSET + body.len()];

        // Primary magic at 0x00
        buf[0] = WRI_MAGIC_PRIMARY as u8;
        buf[1] = (WRI_MAGIC_PRIMARY >> 8) as u8;

        // Secondary magic at 0x04
        buf[4] = WRI_MAGIC_SECONDARY as u8;
        buf[5] = (WRI_MAGIC_SECONDARY >> 8) as u8;

        // fcMac at 0x0E (LE u32)
        buf[FC_MAC_OFFSET] = fc_mac as u8;
        buf[FC_MAC_OFFSET + 1] = (fc_mac >> 8) as u8;
        buf[FC_MAC_OFFSET + 2] = (fc_mac >> 16) as u8;
        buf[FC_MAC_OFFSET + 3] = (fc_mac >> 24) as u8;

        // Body text
        buf[TEXT_START_OFFSET..].copy_from_slice(body);

        buf
    }

    #[test]
    fn parse_simple_text() {
        let body = b"Hello, world!\r\n";
        let data = make_wri(body);
        let doc = parse_wri_bytes(&data, Path::new("test.wri")).unwrap();

        assert_eq!(doc.text, "Hello, world!\n");
        assert_eq!(doc.paragraphs.len(), 1);
        assert_eq!(doc.paragraphs[0], "Hello, world!");
    }

    #[test]
    fn parse_multiple_paragraphs() {
        let body = b"First paragraph.\r\n\r\nSecond paragraph.\r\n";
        let data = make_wri(body);
        let doc = parse_wri_bytes(&data, Path::new("test.wri")).unwrap();

        assert_eq!(doc.paragraphs.len(), 2);
        assert_eq!(doc.paragraphs[0], "First paragraph.");
        assert_eq!(doc.paragraphs[1], "Second paragraph.");
    }

    #[test]
    fn line_wrapping_collapsed_within_paragraph() {
        // Simulates hard-wrapped lines that belong to the same paragraph
        let body = b"This is a long line that\r\nwraps to the next line.\r\n";
        let data = make_wri(body);
        let doc = parse_wri_bytes(&data, Path::new("test.wri")).unwrap();

        assert_eq!(doc.paragraphs.len(), 1);
        assert_eq!(
            doc.paragraphs[0],
            "This is a long line that wraps to the next line."
        );
    }

    #[test]
    fn apostrophe_0xc6_decoded() {
        // 0xC6 is the Write-internal right single quote
        let body = b"LeClure\xC6s daughter\r\n";
        let data = make_wri(body);
        let doc = parse_wri_bytes(&data, Path::new("test.wri")).unwrap();

        assert!(doc.text.contains("LeClure\u{2019}s"));
    }

    #[test]
    fn apostrophe_0x92_decoded() {
        // 0x92 is the Windows-1252 right single quote
        let body = b"LeClure\x92s daughter\r\n";
        let data = make_wri(body);
        let doc = parse_wri_bytes(&data, Path::new("test.wri")).unwrap();

        assert!(doc.text.contains("LeClure\u{2019}s"));
    }

    #[test]
    fn carriage_returns_stripped() {
        let body = b"Line one.\r\nLine two.\r\n";
        let data = make_wri(body);
        let doc = parse_wri_bytes(&data, Path::new("test.wri")).unwrap();

        assert!(!doc.text.contains('\r'));
        assert!(doc.text.contains('\n'));
    }

    #[test]
    fn control_characters_replaced() {
        // 0x01 is a control character that should become U+FFFD
        let body = b"Hello\x01World\r\n";
        let data = make_wri(body);
        let doc = parse_wri_bytes(&data, Path::new("test.wri")).unwrap();

        assert!(doc.text.contains('\u{FFFD}'));
        assert!(doc.text.contains("Hello"));
        assert!(doc.text.contains("World"));
    }

    #[test]
    fn empty_paragraphs_discarded() {
        let body = b"First.\r\n\r\n\r\n\r\nSecond.\r\n";
        let data = make_wri(body);
        let doc = parse_wri_bytes(&data, Path::new("test.wri")).unwrap();

        assert_eq!(doc.paragraphs.len(), 2);
    }

    #[test]
    fn bad_magic_rejected() {
        let mut data = make_wri(b"text");
        data[0] = 0xFF; // corrupt primary magic
        let err = parse_wri_bytes(&data, Path::new("bad.wri")).unwrap_err();
        assert!(matches!(err, WriError::InvalidFormat(_)));
    }

    #[test]
    fn bad_secondary_magic_rejected() {
        let mut data = make_wri(b"text");
        data[4] = 0xFF; // corrupt secondary magic
        let err = parse_wri_bytes(&data, Path::new("bad.wri")).unwrap_err();
        assert!(matches!(err, WriError::InvalidFormat(_)));
    }

    #[test]
    fn file_too_small_rejected() {
        let data = vec![0u8; 10]; // way too small
        let err = parse_wri_bytes(&data, Path::new("tiny.wri")).unwrap_err();
        assert!(matches!(err, WriError::InvalidFormat(_)));
    }

    #[test]
    fn fc_mac_before_text_start_rejected() {
        let mut data = make_wri(b"text");
        // Set fcMac to 0x10 — before text start at 0x80
        data[FC_MAC_OFFSET] = 0x10;
        data[FC_MAC_OFFSET + 1] = 0x00;
        data[FC_MAC_OFFSET + 2] = 0x00;
        data[FC_MAC_OFFSET + 3] = 0x00;
        let err = parse_wri_bytes(&data, Path::new("bad.wri")).unwrap_err();
        assert!(matches!(err, WriError::TextExtraction(_)));
    }

    #[test]
    fn fc_mac_beyond_file_rejected() {
        let mut data = make_wri(b"text");
        // Set fcMac past file end
        data[FC_MAC_OFFSET] = 0xFF;
        data[FC_MAC_OFFSET + 1] = 0xFF;
        data[FC_MAC_OFFSET + 2] = 0x00;
        data[FC_MAC_OFFSET + 3] = 0x00;
        let err = parse_wri_bytes(&data, Path::new("bad.wri")).unwrap_err();
        assert!(matches!(err, WriError::TextExtraction(_)));
    }

    #[test]
    fn trailing_data_after_fc_mac_ignored() {
        // Simulate the Write format artifact: text followed by binary junk
        let body = b"Real text.\r\n";
        let mut data = make_wri(body);
        // Append trailing junk (like font tables / duplicate text)
        data.extend_from_slice(b"\x00\x00\x00JUNK trailing data");
        let doc = parse_wri_bytes(&data, Path::new("test.wri")).unwrap();

        assert_eq!(doc.text, "Real text.\n");
        assert!(!doc.text.contains("JUNK"));
    }

    #[test]
    fn windows_1252_high_bytes_decoded() {
        // 0xE9 = e-acute in Windows-1252 (and ISO-8859-1)
        let body = b"caf\xE9\r\n";
        let data = make_wri(body);
        let doc = parse_wri_bytes(&data, Path::new("test.wri")).unwrap();

        assert!(doc.text.contains("caf\u{00E9}"));
    }

    #[test]
    fn tab_preserved() {
        let body = b"Column1\tColumn2\r\n";
        let data = make_wri(body);
        let doc = parse_wri_bytes(&data, Path::new("test.wri")).unwrap();

        assert!(doc.text.contains('\t'));
    }
}
