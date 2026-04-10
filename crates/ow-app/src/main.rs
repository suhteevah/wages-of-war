//! Open Wages — main entry point.
//!
//! This binary serves as a proof-of-concept data loader. It validates the
//! presence of original game files, parses every supported data format, and
//! prints a summary report. No rendering happens here yet — this is purely
//! the "can we read everything?" milestone.

use std::collections::HashMap;
use std::path::PathBuf;

use clap::Parser;
use tracing::{error, info, warn};

#[derive(Parser, Debug)]
#[command(name = "open-wages", about = "Open-source Wages of War engine")]
struct Args {
    /// Path to original game data directory
    #[arg(long, default_value = "./data")]
    data_dir: PathBuf,
}

fn main() -> anyhow::Result<()> {
    // -----------------------------------------------------------------------
    // 1. Logging setup — honors RUST_LOG env var, defaults to info + debug
    //    for our own crates so we see parser progress without drowning in noise.
    // -----------------------------------------------------------------------
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,ow_data=debug,ow_core=debug".parse().unwrap()),
        )
        .init();

    // -----------------------------------------------------------------------
    // 2. Parse CLI args and validate game data directory.
    //    We exit hard on validation failure — there's nothing to do without
    //    original data files.
    // -----------------------------------------------------------------------
    let args = Args::parse();
    info!(data_dir = %args.data_dir.display(), "Open Wages starting");

    match ow_data::validator::validate_game_data(&args.data_dir) {
        Ok(()) => info!("Game data validated successfully"),
        Err(e) => {
            error!("Game data validation failed: {e}");
            eprintln!("\n{e}\n");
            std::process::exit(1);
        }
    }

    let data_dir = &args.data_dir;

    // The game files live under WOW/ inside the data directory.
    // Text data files are in WOW/DATA/, buttons in WOW/BUTTONS/, sprites in WOW/SPR/.
    let wow_data = data_dir.join("WOW").join("DATA");
    let wow_buttons = data_dir.join("WOW").join("BUTTONS");
    let wow_spr = data_dir.join("WOW").join("SPR");

    // Track which loads succeeded/failed for the final summary banner.
    let mut successes: Vec<String> = Vec::new();
    let mut failures: Vec<String> = Vec::new();

    // -----------------------------------------------------------------------
    // 3. MERCS.DAT — Mercenary roster (names, stats, bios).
    //    This is the heart of the game: ~40 mercs with full stat blocks.
    // -----------------------------------------------------------------------
    info!("--- Loading MERCS.DAT ---");
    match ow_data::mercs::parse_mercs(&wow_data.join("MERCS.DAT")) {
        Ok(mercs) => {
            let count = mercs.len();
            info!(count, "Parsed mercenary roster");

            // Show a few sample names so we can eyeball correctness.
            let sample: Vec<&str> = mercs.iter().take(5).map(|m| m.name.as_str()).collect();
            info!(?sample, "Sample merc names");

            // Quick stat range check.
            if let (Some(cheapest), Some(priciest)) = (
                mercs.iter().min_by_key(|m| m.dpr),
                mercs.iter().max_by_key(|m| m.dpr),
            ) {
                info!(
                    cheapest_name = %cheapest.name, cheapest_dpr = cheapest.dpr,
                    priciest_name = %priciest.name, priciest_dpr = priciest.dpr,
                    "DPR range"
                );
            }
            successes.push(format!("MERCS.DAT: {count} mercenaries"));
        }
        Err(e) => {
            warn!("Failed to parse MERCS.DAT: {e}");
            failures.push(format!("MERCS.DAT: {e}"));
        }
    }

    // -----------------------------------------------------------------------
    // 4. WEAPONS.DAT — Weapon definitions with ballistic parameters.
    //    We also break down weapons by category to verify type parsing.
    // -----------------------------------------------------------------------
    info!("--- Loading WEAPONS.DAT ---");
    match ow_data::weapons::parse_weapons(&wow_data.join("WEAPONS.DAT")) {
        Ok(weapons) => {
            let count = weapons.len();
            info!(count, "Parsed weapon definitions");

            // Category breakdown — how many rifles vs pistols vs grenades, etc.
            let mut by_type: HashMap<String, usize> = HashMap::new();
            for w in &weapons {
                *by_type.entry(format!("{:?}", w.weapon_type)).or_default() += 1;
            }
            info!(?by_type, "Weapon category breakdown");
            successes.push(format!("WEAPONS.DAT: {count} weapons ({} categories)", by_type.len()));
        }
        Err(e) => {
            warn!("Failed to parse WEAPONS.DAT: {e}");
            failures.push(format!("WEAPONS.DAT: {e}"));
        }
    }

    // -----------------------------------------------------------------------
    // 5. EQUIP.DAT — Non-weapon equipment (armor, medkits, tools, etc.).
    // -----------------------------------------------------------------------
    info!("--- Loading EQUIP.DAT ---");
    match ow_data::equip::parse_equipment(&wow_data.join("EQUIP.DAT")) {
        Ok(items) => {
            let count = items.len();
            info!(count, "Parsed equipment items");

            let sample: Vec<&str> = items.iter().take(5).map(|e| e.name.as_str()).collect();
            info!(?sample, "Sample equipment names");
            successes.push(format!("EQUIP.DAT: {count} equipment items"));
        }
        Err(e) => {
            warn!("Failed to parse EQUIP.DAT: {e}");
            failures.push(format!("EQUIP.DAT: {e}"));
        }
    }

    // -----------------------------------------------------------------------
    // 6. ENGWOW.DAT — Localized string table. Every UI string, dialog line,
    //    and status message lives here, referenced by 1-based index.
    // -----------------------------------------------------------------------
    info!("--- Loading ENGWOW.DAT ---");
    match ow_data::strings::parse_string_table(&wow_data.join("ENGWOW.DAT")) {
        Ok(table) => {
            let count = table.len();
            info!(count, "Parsed string table");

            // Show a handful of strings to spot-check encoding.
            for idx in [1, 2, 3, 10, 50] {
                if let Some(s) = table.get(idx) {
                    info!(index = idx, text = %s, "Sample string");
                }
            }
            successes.push(format!("ENGWOW.DAT: {count} strings"));
        }
        Err(e) => {
            warn!("Failed to parse ENGWOW.DAT: {e}");
            failures.push(format!("ENGWOW.DAT: {e}"));
        }
    }

    // -----------------------------------------------------------------------
    // 7. MSSN01.DAT — Mission 1 contract. Proves we can read the full
    //    mission structure: contract terms, enemy roster, weather, etc.
    // -----------------------------------------------------------------------
    info!("--- Loading MSSN01.DAT ---");
    match ow_data::mission::parse_mission(&wow_data.join("MSSN01.DAT")) {
        Ok(mission) => {
            let contract = &mission.contract;
            info!(
                from = %contract.from,
                terms = %contract.terms,
                advance = contract.advance,
                bonus = contract.bonus,
                "Mission 1 contract summary"
            );
            successes.push(format!(
                "MSSN01.DAT: contract from '{}', advance ${}, bonus ${}",
                contract.from, contract.advance, contract.bonus
            ));
        }
        Err(e) => {
            warn!("Failed to parse MSSN01.DAT: {e}");
            failures.push(format!("MSSN01.DAT: {e}"));
        }
    }

    // -----------------------------------------------------------------------
    // 8. AI node graphs — Parse a few AINODE files to verify the pathfinding
    //    graph reader. These define waypoint connectivity for enemy AI.
    // -----------------------------------------------------------------------
    info!("--- Loading AINODE files ---");
    for filename in ["AINODE01.DAT", "AINODE02.DAT", "AINODE03.DAT"] {
        let path = wow_data.join(filename);
        if !path.exists() {
            info!("{filename} not found, skipping");
            continue;
        }
        match ow_data::ai_nodes::parse_ai_nodes(&path) {
            Ok(graph) => {
                info!(
                    file = filename,
                    total_nodes = graph.total_nodes,
                    "Parsed AI node graph"
                );
                successes.push(format!("{filename}: {0} nodes", graph.total_nodes));
            }
            Err(e) => {
                warn!("Failed to parse {filename}: {e}");
                failures.push(format!("{filename}: {e}"));
            }
        }
    }

    // -----------------------------------------------------------------------
    // 9. TARGET.DAT — Hit probability lookup table. A 2D grid mapping some
    //    combination of range/skill to base hit chance (0-100%).
    // -----------------------------------------------------------------------
    info!("--- Loading TARGET.DAT ---");
    match ow_data::target::parse_hit_table(&wow_data.join("TARGET.DAT")) {
        Ok(table) => {
            let rows = table.row_count();
            let cols = table.col_count();
            info!(rows, cols, "Parsed hit probability table");

            // Spot-check: row 0 should be high values (point-blank / max skill).
            if let Some(val) = table.lookup(0, 0) {
                info!(row0_col0 = val, "Top-left cell (point-blank base hit %)");
            }
            successes.push(format!("TARGET.DAT: {rows}x{cols} hit table"));
        }
        Err(e) => {
            warn!("Failed to parse TARGET.DAT: {e}");
            failures.push(format!("TARGET.DAT: {e}"));
        }
    }

    // -----------------------------------------------------------------------
    // 10. MAIN.BTN — UI button layout for the main screen. Proves the .BTN
    //     parser can read the header/block structure and extract all rects.
    // -----------------------------------------------------------------------
    info!("--- Loading MAIN.BTN ---");
    match ow_data::buttons::parse_buttons(&wow_buttons.join("MAIN.BTN")) {
        Ok(layout) => {
            let count = layout.buttons.len();
            info!(count, "Parsed main screen button layout");

            // Show a few button IDs and their hit rects.
            for btn in layout.buttons.iter().take(3) {
                info!(
                    id = btn.id,
                    page = btn.page,
                    hit_rect = ?btn.hit_rect,
                    "Sample button"
                );
            }
            successes.push(format!("MAIN.BTN: {count} buttons"));
        }
        Err(e) => {
            warn!("Failed to parse MAIN.BTN: {e}");
            failures.push(format!("MAIN.BTN: {e}"));
        }
    }

    // -----------------------------------------------------------------------
    // 11. Sprite proof-of-concept — Parse MENUSPR.OBJ (menu sprite sheet).
    //     This validates the binary container reader: header, offset table,
    //     per-frame headers, and RLE compressed data extraction.
    //     We also decode one frame to verify the RLE decompressor works.
    // -----------------------------------------------------------------------
    info!("--- Loading sprite file (MENUSPR.OBJ) ---");
    match ow_data::sprite::parse_sprite_file(&wow_spr.join("MENUSPR.OBJ")) {
        Ok(sheet) => {
            let sprite_count = sheet.frames.len();
            let total_compressed: usize = sheet.frames.iter().map(|f| f.compressed_data.len()).sum();
            info!(
                sprite_count,
                total_compressed_bytes = total_compressed,
                "Parsed sprite sheet"
            );

            // Decode every frame to get total pixel bytes — proves the RLE path works.
            let mut total_pixels: usize = 0;
            let mut decode_errors: usize = 0;
            for (i, frame) in sheet.frames.iter().enumerate() {
                match ow_data::sprite::decode_rle(
                    &frame.compressed_data,
                    frame.header.width,
                    frame.header.height,
                    i,
                ) {
                    Ok(pixels) => total_pixels += pixels.len(),
                    Err(e) => {
                        warn!(sprite = i, "RLE decode error: {e}");
                        decode_errors += 1;
                    }
                }
            }
            info!(
                total_decoded_pixels = total_pixels,
                decode_errors,
                "RLE decode complete"
            );
            successes.push(format!(
                "MENUSPR.OBJ: {sprite_count} sprites, {total_pixels} decoded pixels"
            ));
        }
        Err(e) => {
            warn!("Failed to parse MENUSPR.OBJ: {e}");
            failures.push(format!("MENUSPR.OBJ: {e}"));
        }
    }

    // -----------------------------------------------------------------------
    // 12. Game state initialization — Create a fresh campaign in Office phase.
    //     This proves ow-core's state machine compiles and links correctly.
    //     Starting funds of $500,000 matches the original game's default.
    // -----------------------------------------------------------------------
    info!("--- Initializing game state ---");
    let starting_funds: i64 = 500_000;
    let game_state = ow_core::game_state::GameState::new(starting_funds);
    info!(
        phase = ?game_state.phase,
        funds = game_state.funds,
        reputation = game_state.reputation,
        team_size = game_state.team.len(),
        missions_completed = game_state.missions_completed,
        "Game state initialized"
    );
    successes.push(format!(
        "GameState: Office phase, ${} funds, {} mercs",
        game_state.funds,
        game_state.team.len()
    ));

    // -----------------------------------------------------------------------
    // 13. Summary banner — Show at a glance what loaded and what didn't.
    // -----------------------------------------------------------------------
    println!();
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║              OPEN WAGES — DATA LOAD REPORT                 ║");
    println!("╠══════════════════════════════════════════════════════════════╣");
    for s in &successes {
        println!("║  OK  {:<55}║", s);
    }
    for f in &failures {
        println!("║  ERR {:<55}║", f);
    }
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!(
        "║  {} passed, {} failed{:<38}║",
        successes.len(),
        failures.len(),
        ""
    );
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();

    if failures.is_empty() {
        info!("All data files loaded successfully. Engine ready for Phase 2.");
    } else {
        warn!(
            failed = failures.len(),
            "Some data files failed to load — see errors above"
        );
    }

    Ok(())
}
