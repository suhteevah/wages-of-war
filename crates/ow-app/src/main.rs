//! Open Wages — main entry point.
//!
//! Loads game data via the unified Ruleset, initializes SDL2, and launches
//! the game loop. Pass `--data-only` to skip rendering and just validate
//! all data files (the original proof-of-concept mode).

mod game_loop;

use std::path::PathBuf;

use clap::Parser;
use tracing::{error, info};

#[derive(Parser, Debug)]
#[command(name = "open-wages", about = "Open-source Wages of War engine")]
struct Args {
    /// Path to original game data directory
    #[arg(long, default_value = "./data")]
    data_dir: PathBuf,

    /// Skip SDL2 rendering — just load and validate all data files
    #[arg(long)]
    data_only: bool,
}

fn main() -> anyhow::Result<()> {
    // -----------------------------------------------------------------------
    // Logging — honors RUST_LOG env var. Default: info for our crates,
    // warn for everything else to keep SDL2/system noise quiet.
    // -----------------------------------------------------------------------
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,ow_data=info,ow_core=info".parse().unwrap()),
        )
        .init();

    let args = Args::parse();
    info!(data_dir = %args.data_dir.display(), "Open Wages starting");

    // -----------------------------------------------------------------------
    // Validate game data — exit immediately if original files are missing.
    // The player must supply their own copy of Wages of War.
    // -----------------------------------------------------------------------
    match ow_data::validator::validate_game_data(&args.data_dir) {
        Ok(()) => info!("Game data validated"),
        Err(e) => {
            error!("Game data validation failed: {e}");
            eprintln!("\n{e}\n");
            std::process::exit(1);
        }
    }

    // -----------------------------------------------------------------------
    // Load all game data via the unified Ruleset.
    // This replaces the scattered per-file parse calls with one function
    // that loads MERCS, WEAPONS, EQUIP, ENGWOW, TARGET, and all 16 missions.
    // -----------------------------------------------------------------------
    let wow_data = args.data_dir.join("WOW").join("DATA");
    let ruleset = ow_core::ruleset::load_base_ruleset(&wow_data)?;
    info!(
        mercs = ruleset.mercs.len(),
        weapons = ruleset.weapons.len(),
        equipment = ruleset.equipment.len(),
        missions = ruleset.missions.len(),
        strings = ruleset.strings.len(),
        "Ruleset loaded"
    );

    if args.data_only {
        // Data-only mode: print summary and exit (the old proof-of-concept).
        println!("\n=== OPEN WAGES — RULESET SUMMARY ===");
        println!("  Mercenaries: {}", ruleset.mercs.len());
        println!("  Weapons:     {}", ruleset.weapons.len());
        println!("  Equipment:   {}", ruleset.equipment.len());
        println!("  Missions:    {}", ruleset.missions.len());
        println!("  Strings:     {}", ruleset.strings.len());
        println!("  Hit table:   {}x{}", ruleset.hit_table.row_count(), ruleset.hit_table.col_count());
        println!("\nAll data loaded successfully. Exiting (--data-only mode).\n");
        return Ok(());
    }

    // -----------------------------------------------------------------------
    // Initialize SDL2 — window, canvas, event pump.
    // The game runs at 1280x720 by default (configurable later via config.rs).
    // -----------------------------------------------------------------------
    info!("Initializing SDL2");
    let sdl_context = sdl2::init().map_err(|e| anyhow::anyhow!("SDL2 init failed: {e}"))?;
    let video = sdl_context
        .video()
        .map_err(|e| anyhow::anyhow!("SDL2 video init failed: {e}"))?;

    let window = video
        .window("Open Wages — Wages of War Engine", 1280, 720)
        .position_centered()
        .resizable()
        .build()?;

    let canvas = window
        .into_canvas()
        .accelerated()
        .present_vsync()
        .build()?;

    info!("SDL2 window created (1280x720)");

    // -----------------------------------------------------------------------
    // Create game state and launch the game loop.
    // Starting funds of $500,000 matches the original game's default.
    // -----------------------------------------------------------------------
    let game_state = ow_core::game_state::GameState::new(500_000);
    info!(phase = ?game_state.phase, "Entering game loop");

    game_loop::run_game_loop(&sdl_context, canvas, game_state, ruleset, &args.data_dir)?;

    info!("Open Wages shutting down cleanly");
    Ok(())
}
