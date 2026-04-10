//! Full data validation tool for Open Wages.
//!
//! Loads and validates ALL original game data files, reporting comprehensive
//! statistics for each. Usage:
//!
//! ```bash
//! cargo run -p ow-tools --bin validate-all -- "J:/wages of war/data"
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use tracing::{error, info};

use ow_data::ai_nodes::parse_ai_nodes;
use ow_data::animation::parse_animation;
use ow_data::buttons::parse_buttons;
use ow_data::equip::parse_equipment;
use ow_data::mercs::parse_mercs;
use ow_data::mission::parse_mission;
use ow_data::moves::parse_moves;
use ow_data::shop::parse_shop_inventory;
use ow_data::sprite::parse_sprite_file;
use ow_data::strings::parse_string_table;
use ow_data::target::parse_hit_table;
use ow_data::weapons::parse_weapons;

/// Open Wages full data validation tool.
///
/// Loads every known game data file and reports parse success/failure
/// with detailed statistics.
#[derive(Parser, Debug)]
#[command(name = "validate-all", about = "Validate all Wages of War data files")]
struct Cli {
    /// Path to the directory containing original game data files.
    data_dir: PathBuf,
}

// ---------------------------------------------------------------------------
// Result tracking
// ---------------------------------------------------------------------------

struct ValidationResult {
    label: String,
    detail: String,
    ok: bool,
}

impl ValidationResult {
    fn ok(label: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            detail: detail.into(),
            ok: true,
        }
    }

    fn err(label: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            detail: detail.into(),
            ok: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Try to find a file case-insensitively under `dir`.
fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    let candidate = dir.join(name);
    if candidate.exists() {
        return Some(candidate);
    }
    // Try case-insensitive scan of the directory.
    let name_upper = name.to_uppercase();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().to_uppercase() == name_upper {
                return Some(entry.path());
            }
        }
    }
    None
}

/// Try to find a file in `dir` or `dir/subdir` case-insensitively.
fn find_file_in(dir: &Path, subdirs: &[&str], name: &str) -> Option<PathBuf> {
    if let Some(p) = find_file(dir, name) {
        return Some(p);
    }
    for sub in subdirs {
        let subpath = dir.join(sub);
        if subpath.is_dir() {
            if let Some(p) = find_file(&subpath, name) {
                return Some(p);
            }
        }
    }
    None
}

/// Collect files matching a case-insensitive extension under `dir` (non-recursive).
fn files_with_ext(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let ext_upper = ext.to_uppercase();
    let mut results = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(e) = path.extension() {
                    if e.to_string_lossy().to_uppercase() == ext_upper {
                        results.push(path);
                    }
                }
            }
        }
    }
    results.sort();
    results
}

/// Collect files matching a case-insensitive extension under `dir` and all subdirectories.
fn files_with_ext_recursive(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let ext_upper = ext.to_uppercase();
    let mut results = Vec::new();
    for entry in walkdir::WalkDir::new(dir).into_iter().flatten() {
        let path = entry.path();
        if path.is_file() {
            if let Some(e) = path.extension() {
                if e.to_string_lossy().to_uppercase() == ext_upper {
                    results.push(path.to_path_buf());
                }
            }
        }
    }
    results.sort();
    results
}

fn pad_label(label: &str, width: usize) -> String {
    if label.len() >= width {
        return label.to_string();
    }
    let dots_needed = width - label.len();
    format!("{} {}", label, ".".repeat(dots_needed))
}

// ---------------------------------------------------------------------------
// Validation functions
// ---------------------------------------------------------------------------

fn validate_mercs(dir: &Path, results: &mut Vec<ValidationResult>) {
    let label = "Global: MERCS.DAT";
    match find_file_in(dir, &["DATA"], "MERCS.DAT") {
        None => results.push(ValidationResult::err(label, "file not found")),
        Some(path) => match parse_mercs(&path) {
            Ok(mercs) => {
                let min_rating = mercs.iter().map(|m| m.rating).min().unwrap_or(0);
                let max_rating = mercs.iter().map(|m| m.rating).max().unwrap_or(0);
                results.push(ValidationResult::ok(
                    label,
                    format!("{} mercs, rating {}-{}", mercs.len(), min_rating, max_rating),
                ));
            }
            Err(e) => {
                error!("MERCS.DAT parse failed: {e}");
                results.push(ValidationResult::err(label, format!("PARSE ERROR: {e}")));
            }
        },
    }
}

fn validate_weapons(dir: &Path, results: &mut Vec<ValidationResult>) {
    let label = "Global: WEAPONS.DAT";
    match find_file_in(dir, &["DATA"], "WEAPONS.DAT") {
        None => results.push(ValidationResult::err(label, "file not found")),
        Some(path) => match parse_weapons(&path) {
            Ok(weapons) => {
                // Count weapons per category.
                let mut categories: HashMap<String, usize> = HashMap::new();
                for w in &weapons {
                    *categories.entry(format!("{:?}", w.weapon_type)).or_insert(0) += 1;
                }
                results.push(ValidationResult::ok(
                    label,
                    format!(
                        "{} weapons, {} categories",
                        weapons.len(),
                        categories.len()
                    ),
                ));
            }
            Err(e) => {
                error!("WEAPONS.DAT parse failed: {e}");
                results.push(ValidationResult::err(label, format!("PARSE ERROR: {e}")));
            }
        },
    }
}

fn validate_equip(dir: &Path, results: &mut Vec<ValidationResult>) {
    let label = "Global: EQUIP.DAT";
    match find_file_in(dir, &["DATA"], "EQUIP.DAT") {
        None => results.push(ValidationResult::err(label, "file not found")),
        Some(path) => match parse_equipment(&path) {
            Ok(items) => {
                results.push(ValidationResult::ok(
                    label,
                    format!("{} equipment items", items.len()),
                ));
            }
            Err(e) => {
                error!("EQUIP.DAT parse failed: {e}");
                results.push(ValidationResult::err(label, format!("PARSE ERROR: {e}")));
            }
        },
    }
}

fn validate_strings(dir: &Path, results: &mut Vec<ValidationResult>) {
    let label = "Global: ENGWOW.DAT";
    match find_file_in(dir, &["DATA"], "ENGWOW.DAT") {
        None => results.push(ValidationResult::err(label, "file not found")),
        Some(path) => match parse_string_table(&path) {
            Ok(table) => {
                results.push(ValidationResult::ok(
                    label,
                    format!("{} strings", table.len()),
                ));
            }
            Err(e) => {
                error!("ENGWOW.DAT parse failed: {e}");
                results.push(ValidationResult::err(label, format!("PARSE ERROR: {e}")));
            }
        },
    }
}

fn validate_target(dir: &Path, results: &mut Vec<ValidationResult>) {
    let label = "Global: TARGET.DAT";
    match find_file_in(dir, &["DATA"], "TARGET.DAT") {
        None => results.push(ValidationResult::err(label, "file not found")),
        Some(path) => match parse_hit_table(&path) {
            Ok(table) => {
                results.push(ValidationResult::ok(
                    label,
                    format!(
                        "{}x{} primary table, {} aux sections",
                        table.row_count(),
                        table.col_count(),
                        table.aux_section_count()
                    ),
                ));
            }
            Err(e) => {
                error!("TARGET.DAT parse failed: {e}");
                results.push(ValidationResult::err(label, format!("PARSE ERROR: {e}")));
            }
        },
    }
}

fn validate_buttons(dir: &Path, results: &mut Vec<ValidationResult>) {
    let btn_files = files_with_ext_recursive(dir, "BTN");
    if btn_files.is_empty() {
        results.push(ValidationResult::err("Global: *.BTN files", "no .BTN files found"));
        return;
    }
    let mut ok_count = 0usize;
    let mut fail_count = 0usize;
    let mut total_buttons = 0usize;
    for path in &btn_files {
        let fname = path.file_name().unwrap_or_default().to_string_lossy();
        match parse_buttons(path) {
            Ok(layout) => {
                total_buttons += layout.buttons.len();
                ok_count += 1;
            }
            Err(e) => {
                let label = format!("Global: BTN {}", fname);
                error!("{} parse failed: {e}", fname);
                results.push(ValidationResult::err(&label, format!("PARSE ERROR: {e}")));
                fail_count += 1;
            }
        }
    }
    results.push(if fail_count == 0 {
        ValidationResult::ok(
            "Global: *.BTN files",
            format!("{} files, {} buttons total", ok_count, total_buttons),
        )
    } else {
        ValidationResult::err(
            "Global: *.BTN files",
            format!(
                "{}/{} OK, {} failed, {} buttons parsed",
                ok_count,
                ok_count + fail_count,
                fail_count,
                total_buttons
            ),
        )
    });
}

fn validate_animations(dir: &Path, results: &mut Vec<ValidationResult>) {
    // Look for .COR files in ANIM/ subdirectory and root.
    let mut cor_files = files_with_ext_recursive(dir, "COR");
    if cor_files.is_empty() {
        results.push(ValidationResult::err(
            "Global: *.COR files",
            "no .COR files found",
        ));
        return;
    }
    cor_files.sort();

    let mut ok_count = 0usize;
    let mut fail_count = 0usize;
    let mut total_entries = 0usize;
    for path in &cor_files {
        let fname = path.file_name().unwrap_or_default().to_string_lossy();
        match parse_animation(path) {
            Ok(anim) => {
                total_entries += anim.entries.len();
                ok_count += 1;
            }
            Err(e) => {
                let label = format!("Global: COR {}", fname);
                error!("{} parse failed: {e}", fname);
                results.push(ValidationResult::err(&label, format!("PARSE ERROR: {e}")));
                fail_count += 1;
            }
        }
    }
    results.push(if fail_count == 0 {
        ValidationResult::ok(
            "Global: *.COR files",
            format!(
                "{} files, {} animation entries total",
                ok_count, total_entries
            ),
        )
    } else {
        ValidationResult::err(
            "Global: *.COR files",
            format!(
                "{}/{} OK, {} failed, {} entries parsed",
                ok_count,
                ok_count + fail_count,
                fail_count,
                total_entries
            ),
        )
    });
}

fn validate_sprites(dir: &Path, results: &mut Vec<ValidationResult>) {
    // Collect a sample of sprite files across extensions.
    let mut sprite_files: Vec<PathBuf> = Vec::new();
    for ext in &["OBJ", "SPR"] {
        sprite_files.extend(files_with_ext_recursive(dir, ext));
    }
    // Also check ANIM/*.DAT sprite data files.
    let anim_dir = dir.join("ANIM");
    if anim_dir.is_dir() {
        sprite_files.extend(files_with_ext(&anim_dir, "DAT"));
    }

    if sprite_files.is_empty() {
        results.push(ValidationResult::err(
            "Global: sprite files",
            "no .OBJ/.SPR files found",
        ));
        return;
    }

    sprite_files.sort();

    // Parse a sample: up to 20 files to keep validation fast.
    let sample_size = sprite_files.len().min(20);
    let sample = &sprite_files[..sample_size];

    let mut ok_count = 0usize;
    let mut fail_count = 0usize;
    let mut total_frames = 0usize;
    for path in sample {
        let fname = path.file_name().unwrap_or_default().to_string_lossy();
        match parse_sprite_file(path) {
            Ok(sheet) => {
                total_frames += sheet.frames.len();
                ok_count += 1;
            }
            Err(e) => {
                error!("sprite {} parse failed: {e}", fname);
                fail_count += 1;
            }
        }
    }
    results.push(if fail_count == 0 {
        ValidationResult::ok(
            "Global: sprite files",
            format!(
                "{}/{} sampled OK, {} frames total",
                ok_count, sample_size, total_frames
            ),
        )
    } else {
        ValidationResult::err(
            "Global: sprite files",
            format!(
                "{}/{} sampled OK, {} failed, {} frames",
                ok_count, sample_size, fail_count, total_frames
            ),
        )
    });
}

// ---------------------------------------------------------------------------
// Per-mission validation
// ---------------------------------------------------------------------------

fn validate_mission_file(
    dir: &Path,
    nn: u8,
    results: &mut Vec<ValidationResult>,
) {
    let nn_str = format!("{:02}", nn);
    let filename = format!("MSSN{}.DAT", nn_str);
    let label = format!("Mission {:02}: {}", nn, filename);

    match find_file_in(dir, &["DATA"], &filename) {
        None => results.push(ValidationResult::err(&label, "file not found")),
        Some(path) => match parse_mission(&path) {
            Ok(mission) => {
                let enemy_count = mission.enemy_count;
                let weather_dominant = dominant_weather(&mission.weather);
                let advance = mission.contract.advance;
                let detail = format!(
                    "{} enemies, {}, ${:.0}K advance",
                    enemy_count,
                    weather_dominant,
                    advance as f64 / 1000.0
                );
                results.push(ValidationResult::ok(&label, detail));
            }
            Err(e) => {
                error!("{} parse failed: {e}", filename);
                results.push(ValidationResult::err(&label, format!("PARSE ERROR: {e}")));
            }
        },
    }
}

fn dominant_weather(w: &ow_data::mission::WeatherTable) -> String {
    let entries = [
        (w.clear, "clear"),
        (w.foggy, "foggy"),
        (w.overcast, "overcast"),
        (w.light_rain, "light rain"),
        (w.heavy_rain, "heavy rain"),
        (w.storm, "storm"),
    ];
    entries
        .iter()
        .max_by_key(|(pct, _)| *pct)
        .map(|(pct, name)| {
            if *pct >= 100 {
                name.to_string()
            } else {
                format!("{} ({}%)", name, pct)
            }
        })
        .unwrap_or_else(|| "unknown".to_string())
}

fn validate_ainode(dir: &Path, nn: u8, results: &mut Vec<ValidationResult>) {
    let nn_str = format!("{:02}", nn);
    let filename = format!("AINODE{}.DAT", nn_str);
    let label = format!("Mission {:02}: {}", nn, filename);

    match find_file_in(dir, &["DATA"], &filename) {
        None => results.push(ValidationResult::err(&label, "file not found")),
        Some(path) => match parse_ai_nodes(&path) {
            Ok(graph) => {
                results.push(ValidationResult::ok(
                    &label,
                    format!("{} nodes", graph.nodes.len()),
                ));
            }
            Err(e) => {
                error!("{} parse failed: {e}", filename);
                results.push(ValidationResult::err(&label, format!("PARSE ERROR: {e}")));
            }
        },
    }
}

fn validate_moves_file(dir: &Path, nn: u8, results: &mut Vec<ValidationResult>) {
    let nn_str = format!("{:02}", nn);
    let filename = format!("MOVES{}.DAT", nn_str);
    let label = format!("Mission {:02}: {}", nn, filename);

    match find_file_in(dir, &["DATA"], &filename) {
        None => results.push(ValidationResult::err(&label, "file not found")),
        Some(path) => match parse_moves(&path) {
            Ok(script) => {
                let mut parts = Vec::new();
                parts.push(format!("{} enemies", script.enemy_count));
                if script.npc_count > 0 {
                    parts.push(format!("{} NPC", script.npc_count));
                }
                if script.vehicle_count > 0 {
                    parts.push(format!("{} vehicles", script.vehicle_count));
                }
                results.push(ValidationResult::ok(&label, parts.join(", ")));
            }
            Err(e) => {
                error!("{} parse failed: {e}", filename);
                results.push(ValidationResult::err(&label, format!("PARSE ERROR: {e}")));
            }
        },
    }
}

fn validate_shop_file(
    dir: &Path,
    nn: u8,
    prefix: &str,
    shop_name: &str,
    results: &mut Vec<ValidationResult>,
) {
    let nn_str = format!("{:02}", nn);
    let filename = format!("{}{}.DAT", prefix, nn_str);
    let label = format!("Mission {:02}: {} ({})", nn, filename, shop_name);

    match find_file_in(dir, &["DATA"], &filename) {
        None => results.push(ValidationResult::err(&label, "file not found")),
        Some(path) => match parse_shop_inventory(&path) {
            Ok(inv) => {
                results.push(ValidationResult::ok(
                    &label,
                    format!("{} items", inv.items.len()),
                ));
            }
            Err(e) => {
                error!("{} parse failed: {e}", filename);
                results.push(ValidationResult::err(&label, format!("PARSE ERROR: {e}")));
            }
        },
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() -> ExitCode {
    // Initialize tracing.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let cli = Cli::parse();
    let dir = &cli.data_dir;

    if !dir.exists() {
        eprintln!("ERROR: data directory does not exist: {}", dir.display());
        return ExitCode::FAILURE;
    }

    info!("data directory: {}", dir.display());

    let mut results: Vec<ValidationResult> = Vec::new();

    // -----------------------------------------------------------------------
    // Global files
    // -----------------------------------------------------------------------
    println!();
    println!("=== OPEN WAGES — FULL DATA VALIDATION ===");
    println!();

    validate_mercs(dir, &mut results);
    validate_weapons(dir, &mut results);
    validate_equip(dir, &mut results);
    validate_strings(dir, &mut results);
    validate_target(dir, &mut results);
    validate_buttons(dir, &mut results);
    validate_animations(dir, &mut results);
    validate_sprites(dir, &mut results);

    // -----------------------------------------------------------------------
    // Per-mission files (missions 01-16)
    // -----------------------------------------------------------------------
    for nn in 1..=16u8 {
        validate_mission_file(dir, nn, &mut results);
        validate_ainode(dir, nn, &mut results);
        validate_moves_file(dir, nn, &mut results);
        validate_shop_file(dir, nn, "LOCK", "Locker", &mut results);
        validate_shop_file(dir, nn, "SERG", "Serg's shop", &mut results);
        validate_shop_file(dir, nn, "ABDULS", "Abdul's shop", &mut results);
    }

    // -----------------------------------------------------------------------
    // Print report
    // -----------------------------------------------------------------------
    println!();

    let label_width = 42;
    let mut pass_count = 0usize;
    let mut fail_count = 0usize;

    for r in &results {
        let status = if r.ok { "OK" } else { "FAIL" };
        let padded = pad_label(&r.label, label_width);
        if r.ok {
            println!("{} {} ({})", padded, status, r.detail);
            pass_count += 1;
        } else {
            println!("{} {} — {}", padded, status, r.detail);
            fail_count += 1;
        }
    }

    let total = pass_count + fail_count;
    println!();
    println!(
        "TOTAL: {}/{} files validated, {} errors",
        pass_count, total, fail_count
    );
    println!();

    if fail_count > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
