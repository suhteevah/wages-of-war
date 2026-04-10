//! # ow-core — Game Rules Engine
//!
//! All game logic with zero rendering dependencies. Combat resolution,
//! economy, AI, pathfinding, line of sight, suppression, weather.

pub mod merc;
pub mod combat;
pub mod damage;
pub mod weather;
pub mod game_state;

pub mod pathfinding;
pub mod los;

pub mod economy;
pub mod hiring;
pub mod inventory;
pub mod contract;

pub mod mission_setup;
pub mod actions;
pub mod ai;
pub mod save;
pub mod config;
pub mod ruleset;
