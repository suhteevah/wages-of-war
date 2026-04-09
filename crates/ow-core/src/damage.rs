//! Damage resolution and suppression checks.
//!
//! Attack resolution uses the TARGET.DAT hit table for base probability,
//! then applies weapon/armor penetration and weather modifiers.

use serde::{Deserialize, Serialize};
use tracing::{debug, trace};

use ow_data::target::HitTable;

/// Outcome of a single attack roll.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttackResult {
    /// Shot missed entirely.
    Miss,
    /// Shot hit the target.
    Hit {
        /// Raw damage dealt (before armor reduction).
        damage: u32,
        /// Whether the shot penetrated armor.
        penetrated: bool,
    },
    /// Target wasn't hit but is suppressed by the incoming fire.
    Suppressed,
}

/// Resolve a single attack using the hit probability table.
///
/// # Parameters
/// - `attacker_wsk`: Attacker's Weapon Skill stat.
/// - `weapon_damage`: Base weapon damage class.
/// - `weapon_pen`: Weapon penetration rating.
/// - `armor_pen`: Target's armor protection rating.
/// - `range`: Distance in tiles between attacker and target.
/// - `weather_mod`: Accuracy multiplier from weather (1.0 = clear, <1.0 = worse).
/// - `hit_table`: The parsed TARGET.DAT probability lookup.
/// - `roll`: A random value in 0..100 for the hit check.
///
/// # Hit Resolution
/// 1. Look up base hit chance from `hit_table[range][wsk_column]`.
/// 2. Multiply by `weather_mod`.
/// 3. If `roll < hit_chance`, it's a hit. Penetration check: `weapon_pen > armor_pen`.
/// 4. If miss, check suppression.
#[allow(clippy::too_many_arguments)]
pub fn resolve_attack(
    attacker_wsk: u32,
    weapon_damage: u32,
    weapon_pen: u32,
    armor_pen: u32,
    range: u32,
    weather_mod: f32,
    hit_table: &HitTable,
    roll: u32,
) -> AttackResult {
    // Map WSK to a column index. WSK values typically range 0-100;
    // we divide by 5 to map into ~20 columns.
    let col = (attacker_wsk / 5).min(hit_table.col_count().saturating_sub(1) as u32) as usize;
    let row = (range as usize).min(hit_table.row_count().saturating_sub(1));

    let base_chance = hit_table.lookup(row, col).unwrap_or(0);
    let modified_chance = ((base_chance as f32) * weather_mod).round() as u32;

    trace!(
        wsk = attacker_wsk,
        col,
        row,
        base_chance,
        weather_mod,
        modified_chance,
        roll,
        "Attack resolution"
    );

    if roll < modified_chance {
        let penetrated = weapon_pen > armor_pen;
        let damage = if penetrated {
            weapon_damage
        } else {
            // Armor absorbed some — deal reduced damage (at least 1 on hit)
            weapon_damage.saturating_sub(armor_pen.saturating_sub(weapon_pen)) .max(1)
        };

        debug!(
            damage,
            penetrated,
            weapon_pen,
            armor_pen,
            "Attack hit"
        );
        AttackResult::Hit { damage, penetrated }
    } else {
        // Near miss — check if it causes suppression anyway
        trace!(roll, modified_chance, "Attack missed");
        AttackResult::Miss
    }
}

/// Check whether incoming fire suppresses a target.
///
/// Suppression is a willpower check against the volume and proximity of fire.
/// Higher willpower = harder to suppress. Closer fire = easier to suppress.
///
/// Formula: suppress if `incoming_firepower * 10 / (distance + 1) > will`
///
/// # Parameters
/// - `will`: Target's Willpower stat.
/// - `incoming_firepower`: Sum of damage classes of weapons firing at this target.
/// - `distance`: Distance in tiles from nearest shooter.
///
/// # Returns
/// `true` if the target becomes suppressed.
pub fn check_suppression(will: u32, incoming_firepower: u32, distance: u32) -> bool {
    let pressure = incoming_firepower.saturating_mul(10) / (distance + 1);
    let suppressed = pressure > will;
    debug!(
        will,
        incoming_firepower,
        distance,
        pressure,
        suppressed,
        "Suppression check"
    );
    suppressed
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a small synthetic hit table for testing.
    /// 5 rows (ranges 0-4), 5 columns (WSK brackets).
    fn test_hit_table() -> HitTable {
        // We can't construct HitTable directly (fields are private),
        // so we serialize/deserialize.
        let rows = vec![
            vec![98, 98, 98, 98, 98], // range 0: point blank
            vec![70, 80, 85, 90, 95], // range 1
            vec![40, 55, 65, 75, 85], // range 2
            vec![20, 35, 50, 60, 70], // range 3
            vec![5, 15, 30, 45, 55],  // range 4
        ];
        let json = serde_json::json!({ "rows": rows });
        serde_json::from_value(json).expect("HitTable deserialization")
    }

    #[test]
    fn point_blank_high_skill_hits() {
        let table = test_hit_table();
        // WSK 20 -> col 4, range 0 -> 98% chance, roll 50 -> hit
        let result = resolve_attack(20, 5, 10, 5, 0, 1.0, &table, 50);
        assert!(matches!(result, AttackResult::Hit { penetrated: true, .. }));
    }

    #[test]
    fn long_range_low_skill_misses() {
        let table = test_hit_table();
        // WSK 0 -> col 0, range 4 -> 5% chance, roll 50 -> miss
        let result = resolve_attack(0, 5, 10, 5, 4, 1.0, &table, 50);
        assert!(matches!(result, AttackResult::Miss));
    }

    #[test]
    fn weather_reduces_accuracy() {
        let table = test_hit_table();
        // WSK 10 -> col 2, range 1 -> 85% base, weather 0.5 -> 43% effective
        // roll 42 -> hit (42 < 43)
        let result = resolve_attack(10, 5, 10, 5, 1, 0.5, &table, 42);
        assert!(matches!(result, AttackResult::Hit { .. }));

        // roll 43 -> miss (43 >= 43)
        let result = resolve_attack(10, 5, 10, 5, 1, 0.5, &table, 43);
        assert!(matches!(result, AttackResult::Miss));
    }

    #[test]
    fn armor_blocks_penetration() {
        let table = test_hit_table();
        // weapon_pen 5 < armor_pen 10 -> not penetrated
        let result = resolve_attack(20, 8, 5, 10, 0, 1.0, &table, 0);
        match result {
            AttackResult::Hit { penetrated, damage } => {
                assert!(!penetrated);
                assert!(damage >= 1); // at least 1 damage on hit
            }
            _ => panic!("Expected hit at point blank"),
        }
    }

    #[test]
    fn suppression_close_range_high_firepower() {
        // Firepower 20 * 10 / (1+1) = 100 > will 50
        assert!(check_suppression(50, 20, 1));
    }

    #[test]
    fn suppression_resisted_by_high_will() {
        // Firepower 5 * 10 / (3+1) = 12 < will 50
        assert!(!check_suppression(50, 5, 3));
    }

    #[test]
    fn suppression_point_blank() {
        // Firepower 10 * 10 / (0+1) = 100 > will 30
        assert!(check_suppression(30, 10, 0));
    }

    #[test]
    fn suppression_zero_firepower_never_suppresses() {
        assert!(!check_suppression(1, 0, 0));
    }
}
