//! # sprite-viewer — CLI entry point
//!
//! Developer tool for visually inspecting decoded sprites from the game's
//! binary sprite container files (.OBJ, .SPR, .TIL, ANIM/*.DAT).
//!
//! ## Usage
//!
//! ```text
//! sprite-viewer <sprite-file> <pcx-palette-file>
//! ```
//!
//! ## Controls
//!
//! - Left/Right arrows: cycle through sprites
//! - ESC: quit

use std::path::PathBuf;

use clap::Parser;
use tracing_subscriber::EnvFilter;

/// Interactive sprite viewer for Wages of War sprite containers.
#[derive(Parser, Debug)]
#[command(name = "sprite-viewer", about = "View decoded sprites from WoW data files")]
struct Args {
    /// Path to a sprite container file (.OBJ, .SPR, .TIL, or ANIM .DAT).
    sprite_file: PathBuf,

    /// Path to a PCX file to extract the 256-color palette from.
    palette_pcx: PathBuf,
}

fn main() -> anyhow::Result<()> {
    // Initialize tracing with env-filter support.
    // Default to `info` level; override with RUST_LOG env var.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();

    tracing::info!(
        sprite = %args.sprite_file.display(),
        palette = %args.palette_pcx.display(),
        "starting sprite viewer"
    );

    ow_render::viewer::run_viewer(&args.sprite_file, &args.palette_pcx)
}
