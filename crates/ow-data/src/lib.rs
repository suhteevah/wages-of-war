//! # ow-data — Original Game File Parsers
//!
//! Parsers for Wages of War data files. All parsers produce strongly-typed
//! Rust structs from the original game's data files.
//!
//! ## Modules
//!
//! - [`dat_parser`] — Low-level line-oriented `.dat` file tokenizer
//! - [`validator`] — Checks that required original game files are present
//! - [`mercs`] — Mercenary roster and stats (MERCS.DAT)
//! - [`weapons`] — Weapon definitions and ballistics (WEAPONS.DAT)
//! - [`equip`] — Equipment / armor / gear (EQUIP.DAT)
//! - [`strings`] — Localized string table (ENGWOW.DAT)
//! - [`mission`] — Mission parameters and objectives (MSSN*.DAT)
//! - [`ai_nodes`] — AI pathing / behavior nodes (AINODE*.DAT)
//! - [`moves`] — Unit movement costs per terrain (MOVES*.DAT)
//! - [`shop`] — Shop inventories — Lock, Serg, Abduls (LOCK*.DAT)
//! - [`buttons`] — UI button layout definitions (*.BTN)
//! - [`animation`] — Sprite animation sequences (*.COR)
//! - [`target`] — Hit location probability table (TARGET.DAT)
//! - [`textrect`] — Text rectangle / UI text layout (TEXTRECT*.DAT)
//! - [`sprite`] — Binary sprite container format (.OBJ, .SPR, .TIL, ANIM .DAT)

pub mod dat_parser;
pub mod validator;

pub mod mercs;
pub mod weapons;
pub mod equip;
pub mod strings;
pub mod mission;
pub mod ai_nodes;
pub mod moves;
pub mod shop;
pub mod buttons;
pub mod animation;
pub mod target;
pub mod textrect;
pub mod sprite;
