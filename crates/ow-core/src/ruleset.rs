//! # Ruleset — Central Data Store (OXCE Pattern)
//!
//! The `Ruleset` is the single owner of all parsed game data, indexed by
//! string keys. This follows the OXCE/OpenXcom architecture where a central
//! `Mod` object owns typed `HashMap<String, Rule*>` collections for every
//! game concept.
//!
//! ## Why string keys?
//!
//! String keys are fundamental to mod support. When a mod wants to override
//! the stats for "Hatchet" or add a new mercenary named "Viper", it does so
//! by name — the same name that appears in save files, UI, and debug logs.
//! Integer indices would break as soon as a mod inserts or reorders entries.
//! String keys make modding, debugging, and save compatibility trivial.
//!
//! ## Overlay merge semantics
//!
//! The base game data is loaded first via [`load_base_ruleset`]. Mods are
//! then applied on top via [`apply_mod_overlay`]. For rule definitions
//! (mercs, weapons, equipment), the merge is **last-writer-wins**: if a mod
//! provides an entry with the same name as a base entry, the mod's version
//! completely replaces the base. New names are simply inserted. This matches
//! OXCE's rule-merging behavior (distinct from its first-writer-wins policy
//! for asset *files*).
//!
//! Missions are keyed by their file stem ("MSSN01" through "MSSN16"), so a
//! mod can replace specific missions by providing files with the same names.

use std::collections::HashMap;
use std::path::Path;

use thiserror::Error;
use tracing::{debug, info};

use ow_data::equip::{Equipment, EquipError};
use ow_data::mercs::{Mercenary, MercsError};
use ow_data::mission::{Mission, MissionError};
use ow_data::strings::{StringTable, StringsError};
use ow_data::target::{HitTable, TargetError};
use ow_data::weapons::{Weapon, WeaponsError};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur during ruleset loading or mod overlay application.
#[derive(Debug, Error)]
pub enum RulesetError {
    #[error("failed to parse MERCS.DAT: {0}")]
    Mercs(#[from] MercsError),

    #[error("failed to parse WEAPONS.DAT: {0}")]
    Weapons(#[from] WeaponsError),

    #[error("failed to parse EQUIP.DAT: {0}")]
    Equipment(#[from] EquipError),

    #[error("failed to parse mission file: {0}")]
    Mission(#[from] MissionError),

    #[error("failed to parse ENGWOW.DAT: {0}")]
    Strings(#[from] StringsError),

    #[error("failed to parse TARGET.DAT: {0}")]
    HitTable(#[from] TargetError),

    #[error("mod directory not found: {0}")]
    ModNotFound(String),

    #[error("merge conflict in {category} for key '{key}': {detail}")]
    MergeConflict {
        category: String,
        key: String,
        detail: String,
    },

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

// ---------------------------------------------------------------------------
// Ruleset
// ---------------------------------------------------------------------------

/// Central data store owning all parsed game data, indexed by string keys.
///
/// This is the Rust equivalent of OXCE's `Mod` class. Every game concept
/// has a `HashMap<String, T>` so that mods can insert, replace, or extend
/// entries by name. Runtime game state (saves, combat instances) references
/// ruleset entries by string key and resolves them through the typed getters.
#[derive(Debug, Clone)]
pub struct Ruleset {
    /// Mercenary definitions keyed by merc name (e.g. "Hatchet", "Fidel").
    pub mercs: HashMap<String, Mercenary>,

    /// Weapon definitions keyed by weapon name (e.g. "M16", "Dragunov").
    pub weapons: HashMap<String, Weapon>,

    /// Non-weapon equipment keyed by item name (e.g. "Kevlar Vest").
    pub equipment: HashMap<String, Equipment>,

    /// Mission definitions keyed by file stem ("MSSN01" .. "MSSN16").
    pub missions: HashMap<String, Mission>,

    /// Engine string table (localized UI text from ENGWOW.DAT).
    pub strings: StringTable,

    /// Combat hit-location probability lookup table (TARGET.DAT).
    pub hit_table: HitTable,

    /// Identifies this ruleset layer. "base" for original game data,
    /// or the mod's directory name for overlay rulesets.
    pub mod_name: String,
}

impl Ruleset {
    /// Look up a mercenary by name.
    pub fn get_merc(&self, name: &str) -> Option<&Mercenary> {
        self.mercs.get(name)
    }

    /// Look up a weapon by name.
    pub fn get_weapon(&self, name: &str) -> Option<&Weapon> {
        self.weapons.get(name)
    }

    /// Look up an equipment item by name.
    pub fn get_equipment(&self, name: &str) -> Option<&Equipment> {
        self.equipment.get(name)
    }

    /// Look up a mission by key (e.g. "MSSN01").
    pub fn get_mission(&self, key: &str) -> Option<&Mission> {
        self.missions.get(key)
    }

    /// Return a sorted list of mission keys (e.g. ["MSSN01", "MSSN02", ...]).
    ///
    /// Sorted lexicographically, which for the "MSSN##" naming convention
    /// also gives numeric order.
    pub fn mission_ids(&self) -> Vec<&str> {
        let mut ids: Vec<&str> = self.missions.keys().map(|s| s.as_str()).collect();
        ids.sort_unstable();
        ids
    }
}

// ---------------------------------------------------------------------------
// Base loading
// ---------------------------------------------------------------------------

/// Load all game data files from a WOW/ data directory into a single Ruleset.
///
/// This replaces the scattered `parse_*` calls that were previously spread
/// across `main.rs`. A single call to `load_base_ruleset` produces a fully
/// populated Ruleset ready for use (or for mod overlay application).
///
/// Expected directory structure:
/// ```text
/// data_dir/
///   MERCS.DAT
///   WEAPONS.DAT
///   EQUIP.DAT
///   ENGWOW.DAT
///   TARGET.DAT
///   MSSN01.DAT .. MSSN16.DAT
/// ```
pub fn load_base_ruleset(data_dir: &Path) -> Result<Ruleset, RulesetError> {
    info!(path = %data_dir.display(), "Loading base ruleset from data directory");

    // --- Mercs ---
    let mercs_path = data_dir.join("MERCS.DAT");
    info!(path = %mercs_path.display(), "Parsing mercenary roster");
    let merc_list = ow_data::mercs::parse_mercs(&mercs_path)?;
    let mercs: HashMap<String, Mercenary> = merc_list
        .into_iter()
        .map(|m| (m.name.clone(), m))
        .collect();
    debug!(count = mercs.len(), "Loaded mercenaries into ruleset");

    // --- Weapons ---
    let weapons_path = data_dir.join("WEAPONS.DAT");
    info!(path = %weapons_path.display(), "Parsing weapon definitions");
    let weapon_list = ow_data::weapons::parse_weapons(&weapons_path)?;
    let weapons: HashMap<String, Weapon> = weapon_list
        .into_iter()
        .map(|w| (w.name.clone(), w))
        .collect();
    debug!(count = weapons.len(), "Loaded weapons into ruleset");

    // --- Equipment ---
    let equip_path = data_dir.join("EQUIP.DAT");
    info!(path = %equip_path.display(), "Parsing equipment definitions");
    let equip_list = ow_data::equip::parse_equipment(&equip_path)?;
    let equipment: HashMap<String, Equipment> = equip_list
        .into_iter()
        .map(|e| (e.name.clone(), e))
        .collect();
    debug!(count = equipment.len(), "Loaded equipment into ruleset");

    // --- String table ---
    let strings_path = data_dir.join("ENGWOW.DAT");
    info!(path = %strings_path.display(), "Parsing string table");
    let strings = ow_data::strings::parse_string_table(&strings_path)?;

    // --- Hit table ---
    let target_path = data_dir.join("TARGET.DAT");
    info!(path = %target_path.display(), "Parsing hit location table");
    let hit_table = ow_data::target::parse_hit_table(&target_path)?;

    // --- Missions (MSSN01..MSSN16) ---
    let mut missions = HashMap::new();
    for i in 1..=16 {
        let filename = format!("MSSN{:02}.DAT", i);
        let mission_path = data_dir.join(&filename);
        if mission_path.exists() {
            let key = format!("MSSN{:02}", i);
            info!(path = %mission_path.display(), key = %key, "Parsing mission file");
            let mission = ow_data::mission::parse_mission(&mission_path)?;
            missions.insert(key, mission);
        } else {
            debug!(file = %filename, "Mission file not found, skipping");
        }
    }
    debug!(count = missions.len(), "Loaded missions into ruleset");

    let ruleset = Ruleset {
        mercs,
        weapons,
        equipment,
        missions,
        strings,
        hit_table,
        mod_name: "base".to_string(),
    };

    info!(
        mercs = ruleset.mercs.len(),
        weapons = ruleset.weapons.len(),
        equipment = ruleset.equipment.len(),
        missions = ruleset.missions.len(),
        "Base ruleset loaded successfully"
    );

    Ok(ruleset)
}

// ---------------------------------------------------------------------------
// Mod overlay
// ---------------------------------------------------------------------------

/// Apply a mod overlay directory on top of an existing base ruleset.
///
/// This implements the OXCE "last-writer-wins" pattern for rule definitions:
/// for each known data file found in `mod_dir`, parse it and merge the
/// results into `base`. Entries with matching names replace the base entry;
/// entries with new names are added.
///
/// The mod directory should mirror the base data directory structure:
/// ```text
/// mod_dir/
///   MERCS.DAT      (optional — only include files you want to override)
///   WEAPONS.DAT    (optional)
///   EQUIP.DAT      (optional)
///   MSSN01.DAT     (optional — replace specific missions)
///   ...
/// ```
///
/// Files not present in the mod directory are left untouched in the base
/// ruleset. This allows small mods that only tweak a few weapons or add
/// one new mercenary without having to ship the entire data set.
pub fn apply_mod_overlay(base: &mut Ruleset, mod_dir: &Path) -> Result<(), RulesetError> {
    if !mod_dir.exists() {
        return Err(RulesetError::ModNotFound(
            mod_dir.display().to_string(),
        ));
    }

    let mod_name = mod_dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    info!(mod_name = %mod_name, path = %mod_dir.display(), "Applying mod overlay");

    // --- Mercs overlay ---
    let mercs_path = mod_dir.join("MERCS.DAT");
    if mercs_path.exists() {
        info!(mod_name = %mod_name, "Merging mod MERCS.DAT");
        let mod_mercs = ow_data::mercs::parse_mercs(&mercs_path)?;
        for merc in mod_mercs {
            let replaced = base.mercs.contains_key(&merc.name);
            if replaced {
                debug!(
                    mod_name = %mod_name,
                    merc = %merc.name,
                    "Overwriting base mercenary (last-writer-wins)"
                );
            } else {
                debug!(
                    mod_name = %mod_name,
                    merc = %merc.name,
                    "Adding new mercenary from mod"
                );
            }
            base.mercs.insert(merc.name.clone(), merc);
        }
    }

    // --- Weapons overlay ---
    let weapons_path = mod_dir.join("WEAPONS.DAT");
    if weapons_path.exists() {
        info!(mod_name = %mod_name, "Merging mod WEAPONS.DAT");
        let mod_weapons = ow_data::weapons::parse_weapons(&weapons_path)?;
        for weapon in mod_weapons {
            let replaced = base.weapons.contains_key(&weapon.name);
            if replaced {
                debug!(
                    mod_name = %mod_name,
                    weapon = %weapon.name,
                    "Overwriting base weapon (last-writer-wins)"
                );
            } else {
                debug!(
                    mod_name = %mod_name,
                    weapon = %weapon.name,
                    "Adding new weapon from mod"
                );
            }
            base.weapons.insert(weapon.name.clone(), weapon);
        }
    }

    // --- Equipment overlay ---
    let equip_path = mod_dir.join("EQUIP.DAT");
    if equip_path.exists() {
        info!(mod_name = %mod_name, "Merging mod EQUIP.DAT");
        let mod_equip = ow_data::equip::parse_equipment(&equip_path)?;
        for item in mod_equip {
            let replaced = base.equipment.contains_key(&item.name);
            if replaced {
                debug!(
                    mod_name = %mod_name,
                    item = %item.name,
                    "Overwriting base equipment (last-writer-wins)"
                );
            } else {
                debug!(
                    mod_name = %mod_name,
                    item = %item.name,
                    "Adding new equipment from mod"
                );
            }
            base.equipment.insert(item.name.clone(), item);
        }
    }

    // --- Mission overlays (MSSN01..MSSN16) ---
    for i in 1..=16 {
        let filename = format!("MSSN{:02}.DAT", i);
        let mission_path = mod_dir.join(&filename);
        if mission_path.exists() {
            let key = format!("MSSN{:02}", i);
            info!(
                mod_name = %mod_name,
                key = %key,
                "Merging mod mission file"
            );
            let mission = ow_data::mission::parse_mission(&mission_path)?;
            if base.missions.contains_key(&key) {
                debug!(
                    mod_name = %mod_name,
                    key = %key,
                    "Overwriting base mission (last-writer-wins)"
                );
            } else {
                debug!(
                    mod_name = %mod_name,
                    key = %key,
                    "Adding new mission from mod"
                );
            }
            base.missions.insert(key, mission);
        }
    }

    // --- String table overlay (ENGWOW.DAT) ---
    let strings_path = mod_dir.join("ENGWOW.DAT");
    if strings_path.exists() {
        info!(
            mod_name = %mod_name,
            "Replacing string table with mod version"
        );
        base.strings = ow_data::strings::parse_string_table(&strings_path)?;
    }

    // --- Hit table overlay (TARGET.DAT) ---
    let target_path = mod_dir.join("TARGET.DAT");
    if target_path.exists() {
        info!(
            mod_name = %mod_name,
            "Replacing hit table with mod version"
        );
        base.hit_table = ow_data::target::parse_hit_table(&target_path)?;
    }

    base.mod_name = mod_name.clone();

    info!(
        mod_name = %mod_name,
        mercs = base.mercs.len(),
        weapons = base.weapons.len(),
        equipment = base.equipment.len(),
        missions = base.missions.len(),
        "Mod overlay applied successfully"
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Create a unique temporary directory for a test, returning its path.
    /// Caller is responsible for cleanup via `fs::remove_dir_all`.
    fn make_temp_dir(test_name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ow_core_test_{test_name}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir); // clean up any leftover from prior run
        fs::create_dir_all(&dir).expect("failed to create temp dir");
        dir
    }

    // Helper: write a minimal EQUIP.DAT.
    fn write_test_equip_dat(dir: &Path, items: &[(&str, u32, u32)]) {
        let mut content = String::new();
        for (name, pen, enc) in items {
            content.push_str(&format!("{name}\nPEN: {pen}    ENC: {enc}\n"));
        }
        content.push_str("~\n");
        fs::write(dir.join("EQUIP.DAT"), content).unwrap();
    }

    /// Construct a minimal StringTable via JSON deserialization.
    fn make_test_string_table() -> StringTable {
        serde_json::from_str(r#"{ "entries": ["Hello", "World"] }"#)
            .expect("test StringTable should deserialize")
    }

    /// Construct a minimal HitTable via JSON deserialization.
    fn make_test_hit_table() -> HitTable {
        serde_json::from_str(
            r#"{
                "rows": [[50, 40, 30], [60, 50, 40]],
                "aux_sections": []
            }"#,
        )
        .expect("test HitTable should deserialize")
    }

    /// Construct a test Mercenary with all required fields.
    fn make_test_merc(name: &str) -> Mercenary {
        Mercenary {
            name: name.to_string(),
            nickname: name.chars().take(2).collect(),
            age: 30,
            height_feet: 5,
            height_inches: 10,
            weight: 180,
            nation: "US".to_string(),
            rating: 50,
            dpr: 100,
            psg: 10,
            avail: 1,
            exp: 5,
            str_stat: 50,
            agl: 50,
            wil: 50,
            wsk: 50,
            hhc: 50,
            tch: 50,
            enc: 50,
            aps: 10,
            fee_hire: 500,
            fee_bonus: 200,
            fee_death: 1000,
            mail: 0,
            biography: "A test mercenary.".to_string(),
        }
    }

    /// Construct a test Weapon with all required fields.
    fn make_test_weapon(name: &str) -> Weapon {
        use ow_data::weapons::{AttackDieFormula, WeaponType};
        Weapon {
            name: name.to_string(),
            weapon_range: 10,
            damage_class: 5,
            penetration: 3,
            encumbrance: 8,
            attack_dice: AttackDieFormula { min: 1, max: 3 },
            ap_cost: 5,
            area_of_impact: 0,
            delivery_behavior: 1,
            cost: 500,
            ammo_per_clip: 30,
            ammo_encumbrance: 2,
            ammo_cost: 50,
            ammo_name: "None".to_string(),
            weapon_type: WeaponType::Rifle,
        }
    }

    /// Construct a minimal test Mission with all sub-structs populated.
    fn make_test_mission() -> Mission {
        use ow_data::mission::*;
        Mission {
            animation_files: AnimationFiles {
                good_guys: "testsld.cor".to_string(),
                bad_guys: "testemy.cor".to_string(),
                dogs: None,
                npc1: None,
                npc2: None,
                npc3_vhc1: None,
                npc4_vhc2: None,
            },
            contract: ContractTerms {
                date_day: 1,
                date_year: 1996,
                from: "Test Client".to_string(),
                terms: "Test objective".to_string(),
                bonus_text: "Bonus condition".to_string(),
                advance: 5000,
                bonus: 10000,
                deadline_day: 30,
                deadline_year: 1996,
            },
            negotiation: Negotiation {
                advance: [6000, 7000, 8000, 9000],
                bonus: [11000, 12000, 13000, 14000],
                deadline: [25, 20, 15, 10],
                chance: [80, 60, 40, 20],
                counter_values: [5500, 6000, 6500, 7000, 28, 26, 24, 22],
                counter_advance: [1, 2, 3, 4, 5, 6, 7, 8],
                counter_bonus: [1, 2, 3, 4, 5, 6, 7, 8],
                counter_deadline: [1, 2, 3, 4, 5, 6, 7, 8],
            },
            prestige: PrestigeConfig {
                mission_type: 1,
                entrance: 0,
                num_maps: 1,
                success1: 10,
                success2: 0,
                wia: -1,
                mia: -2,
                kia: -5,
            },
            intelligence: IntelligenceConfig {
                tiers: [
                    IntelTier { name: "Basic".to_string(), cost: 100, per_item: 10 },
                    IntelTier { name: "Standard".to_string(), cost: 200, per_item: 20 },
                    IntelTier { name: "Premium".to_string(), cost: 500, per_item: 50 },
                ],
                men: 10,
                exp: 3,
                fire_power: 5,
                success: 70,
                casualties: 2,
                scene_type: 0,
                attachments: 1,
            },
            enemy_count: 1,
            npc_count: 0,
            enemy_ratings: vec![EnemyRating {
                rating: 5, dpr: 3, exp: 2, str_: 50, agl: 50, wil: 50,
                wsk: 50, hhc: 50, tch: 50, enc: 100, aps: 8,
                presence_chance: 100, enemy_type: 2,
            }],
            enemy_weapons: vec![EnemyWeapon {
                weapon1: 0, weapon2: -1, ammo1: 3, ammo2: 0,
                weapon3: -1, extra: -1,
            }],
            preloaded_equipment: EquipmentCounts { weapons: 1, ammo: 3, equipment: 0 },
            recommended_equipment: EquipmentCounts { weapons: 0, ammo: 0, equipment: 0 },
            recommended_item: None,
            start_hour: 8,
            start_minute: 0,
            weather: WeatherTable {
                clear: 60, foggy: 10, overcast: 15,
                light_rain: 10, heavy_rain: 3, storm: 2,
            },
            travel: TravelTable {
                cost1: 500, cost2: 1000, cost3: 2000,
                days1: 5, days2: 3, days3: 1,
            },
            special: SpecialConfig {
                turns: 0, special_type: 0, item: 0, damage: 0,
                damage_message: None,
            },
        }
    }

    /// Build a Ruleset with empty collections for testing (no file I/O).
    fn make_empty_ruleset() -> Ruleset {
        Ruleset {
            mercs: HashMap::new(),
            weapons: HashMap::new(),
            equipment: HashMap::new(),
            missions: HashMap::new(),
            strings: make_test_string_table(),
            hit_table: make_test_hit_table(),
            mod_name: "test".to_string(),
        }
    }

    #[test]
    fn test_ruleset_get_merc() {
        let mut ruleset = make_empty_ruleset();

        assert!(ruleset.get_merc("Ghost").is_none());

        let merc = make_test_merc("Ghost");
        ruleset.mercs.insert("Ghost".to_string(), merc);

        let found = ruleset.get_merc("Ghost");
        assert!(found.is_some());
        assert_eq!(found.unwrap().nickname, "Gh");
        assert_eq!(found.unwrap().name, "Ghost");
    }

    #[test]
    fn test_ruleset_get_weapon() {
        let mut ruleset = make_empty_ruleset();

        assert!(ruleset.get_weapon("M16").is_none());

        let weapon = make_test_weapon("M16");
        ruleset.weapons.insert("M16".to_string(), weapon);

        let found = ruleset.get_weapon("M16");
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "M16");
        assert_eq!(found.unwrap().weapon_range, 10);
    }

    #[test]
    fn test_ruleset_get_equipment() {
        let mut ruleset = make_empty_ruleset();

        assert!(ruleset.get_equipment("Helmet").is_none());

        ruleset.equipment.insert(
            "Helmet".to_string(),
            Equipment { name: "Helmet".to_string(), penetration: 3, encumbrance: 7 },
        );

        let found = ruleset.get_equipment("Helmet");
        assert!(found.is_some());
        assert_eq!(found.unwrap().penetration, 3);
    }

    #[test]
    fn test_ruleset_get_mission() {
        let mut ruleset = make_empty_ruleset();

        assert!(ruleset.get_mission("MSSN01").is_none());

        ruleset.missions.insert("MSSN01".to_string(), make_test_mission());

        let found = ruleset.get_mission("MSSN01");
        assert!(found.is_some());
        assert_eq!(found.unwrap().contract.advance, 5000);
    }

    #[test]
    fn test_mission_ids_sorted() {
        let mut ruleset = make_empty_ruleset();

        // Insert out of order to verify sorting.
        ruleset.missions.insert("MSSN05".to_string(), make_test_mission());
        ruleset.missions.insert("MSSN01".to_string(), make_test_mission());
        ruleset.missions.insert("MSSN12".to_string(), make_test_mission());

        let ids = ruleset.mission_ids();
        assert_eq!(ids, vec!["MSSN01", "MSSN05", "MSSN12"]);
    }

    #[test]
    fn test_mission_ids_empty() {
        let ruleset = make_empty_ruleset();
        let ids = ruleset.mission_ids();
        assert!(ids.is_empty());
    }

    #[test]
    fn test_overlay_replaces_existing_equipment() {
        // Simulate overlay merge at the HashMap level (no file I/O).
        let mut base_equip = HashMap::new();
        base_equip.insert(
            "Kevlar Vest".to_string(),
            Equipment { name: "Kevlar Vest".to_string(), penetration: 10, encumbrance: 20 },
        );
        base_equip.insert(
            "Helmet".to_string(),
            Equipment { name: "Helmet".to_string(), penetration: 3, encumbrance: 7 },
        );

        // Mod overrides Kevlar Vest with better stats and adds a new item.
        let mod_items = vec![
            Equipment { name: "Kevlar Vest".to_string(), penetration: 15, encumbrance: 18 },
            Equipment { name: "Night Vision Goggles".to_string(), penetration: 0, encumbrance: 3 },
        ];

        // Apply last-writer-wins merge.
        for item in mod_items {
            base_equip.insert(item.name.clone(), item);
        }

        assert_eq!(base_equip.len(), 3);
        assert_eq!(base_equip["Kevlar Vest"].penetration, 15); // mod version
        assert_eq!(base_equip["Helmet"].penetration, 3); // untouched
        assert!(base_equip.contains_key("Night Vision Goggles")); // new
    }

    #[test]
    fn test_overlay_mercs_last_writer_wins() {
        // Verify that mod mercenaries overwrite base mercs by name.
        let mut base_mercs: HashMap<String, Mercenary> = HashMap::new();
        let mut hatchet = make_test_merc("Hatchet");
        hatchet.str_stat = 60;
        base_mercs.insert("Hatchet".to_string(), hatchet);

        // Mod version has buffed strength.
        let mut mod_hatchet = make_test_merc("Hatchet");
        mod_hatchet.str_stat = 80;

        // Last-writer-wins: mod overwrites base.
        base_mercs.insert(mod_hatchet.name.clone(), mod_hatchet);

        assert_eq!(base_mercs["Hatchet"].str_stat, 80);
    }

    #[test]
    fn test_overlay_adds_new_entries() {
        // Verify that mod entries with new names are added, not rejected.
        let mut base_weapons: HashMap<String, Weapon> = HashMap::new();
        base_weapons.insert("M16".to_string(), make_test_weapon("M16"));

        let laser_rifle = make_test_weapon("Laser Rifle");
        base_weapons.insert(laser_rifle.name.clone(), laser_rifle);

        assert_eq!(base_weapons.len(), 2);
        assert!(base_weapons.contains_key("M16"));
        assert!(base_weapons.contains_key("Laser Rifle"));
    }

    #[test]
    fn test_overlay_mod_not_found() {
        let mut ruleset = make_empty_ruleset();

        let result = apply_mod_overlay(
            &mut ruleset,
            Path::new("/nonexistent/mod/directory"),
        );
        assert!(result.is_err());
        match result.unwrap_err() {
            RulesetError::ModNotFound(_) => {} // expected
            other => panic!("Expected ModNotFound, got: {:?}", other),
        }
    }

    #[test]
    fn test_overlay_empty_mod_dir_is_noop() {
        let mod_dir = make_temp_dir("empty_noop");
        let mut ruleset = make_empty_ruleset();

        // Insert one base item to verify it survives untouched.
        ruleset.equipment.insert(
            "Helmet".to_string(),
            Equipment { name: "Helmet".to_string(), penetration: 3, encumbrance: 7 },
        );

        let result = apply_mod_overlay(&mut ruleset, &mod_dir);
        assert!(result.is_ok());
        assert_eq!(ruleset.equipment.len(), 1);
        assert_eq!(ruleset.equipment["Helmet"].penetration, 3);

        let _ = fs::remove_dir_all(&mod_dir);
    }

    #[test]
    fn test_overlay_updates_mod_name() {
        let mod_dir = make_temp_dir("mod_name");
        let mut ruleset = make_empty_ruleset();
        ruleset.mod_name = "base".to_string();

        let result = apply_mod_overlay(&mut ruleset, &mod_dir);
        assert!(result.is_ok());
        // mod_name should be updated to the directory name.
        assert_ne!(ruleset.mod_name, "base");

        let _ = fs::remove_dir_all(&mod_dir);
    }

    #[test]
    fn test_overlay_equip_dat_merge() {
        let mod_dir = make_temp_dir("equip_merge");

        // Write a mod EQUIP.DAT that overrides one item and adds another.
        write_test_equip_dat(&mod_dir, &[("Kevlar Vest", 15, 18), ("Laser Sight", 0, 1)]);

        let mut ruleset = make_empty_ruleset();

        // Base has Kevlar Vest with original stats.
        ruleset.equipment.insert(
            "Kevlar Vest".to_string(),
            Equipment { name: "Kevlar Vest".to_string(), penetration: 10, encumbrance: 20 },
        );

        let result = apply_mod_overlay(&mut ruleset, &mod_dir);
        assert!(result.is_ok());

        // Kevlar Vest should be overwritten by mod (last-writer-wins).
        assert_eq!(ruleset.equipment["Kevlar Vest"].penetration, 15);
        assert_eq!(ruleset.equipment["Kevlar Vest"].encumbrance, 18);

        // Laser Sight should be added as a new entry.
        assert!(ruleset.equipment.contains_key("Laser Sight"));
        assert_eq!(ruleset.equipment["Laser Sight"].encumbrance, 1);

        let _ = fs::remove_dir_all(&mod_dir);
    }

    #[test]
    fn test_overlay_preserves_unaffected_categories() {
        let mod_dir = make_temp_dir("preserve_cats");

        // Only write EQUIP.DAT — mercs and weapons should be untouched.
        write_test_equip_dat(&mod_dir, &[("New Item", 5, 10)]);

        let mut ruleset = make_empty_ruleset();
        ruleset.mercs.insert("Hatchet".to_string(), make_test_merc("Hatchet"));
        ruleset.weapons.insert("M16".to_string(), make_test_weapon("M16"));

        let result = apply_mod_overlay(&mut ruleset, &mod_dir);
        assert!(result.is_ok());

        // Mercs and weapons untouched.
        assert_eq!(ruleset.mercs.len(), 1);
        assert!(ruleset.mercs.contains_key("Hatchet"));
        assert_eq!(ruleset.weapons.len(), 1);
        assert!(ruleset.weapons.contains_key("M16"));

        // Equipment got the new item.
        assert_eq!(ruleset.equipment.len(), 1);
        assert!(ruleset.equipment.contains_key("New Item"));

        let _ = fs::remove_dir_all(&mod_dir);
    }
}
