//! Runtime mercenary representation.
//!
//! [`ActiveMerc`] wraps the static data from `ow-data` with mutable combat state:
//! hit points, action points, position, suppression, inventory, etc.

use serde::{Deserialize, Serialize};
use tracing::{debug, trace};

/// Tile position on the isometric map grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TilePos {
    pub x: i32,
    pub y: i32,
}

/// Current status of a mercenary in the campaign.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MercStatus {
    /// In the roster, not yet hired.
    Available,
    /// Hired and on payroll, sitting in base.
    Hired,
    /// Currently deployed on a mission.
    OnMission,
    /// Wounded in action — recovering, temporarily unavailable.
    WIA,
    /// Missing in action — fate unknown.
    MIA,
    /// Killed in action — permanently gone.
    KIA,
}

/// Unique identifier for a mercenary in the runtime roster.
pub type MercId = u32;

/// An inventory slot — for now, just a weapon/equipment name and weight.
/// This will expand as the equipment system matures.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InventoryItem {
    pub name: String,
    pub encumbrance: u32,
}

/// A mercenary actively participating in the game — the runtime counterpart
/// to [`ow_data::mercs::Mercenary`].
///
/// All base stats are copied in at creation time so we don't hold a borrow
/// across frames. Stat growth from experience is applied here, not in ow-data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveMerc {
    /// Unique runtime id.
    pub id: MercId,
    // -- Identity (copied from ow-data) --
    pub name: String,
    pub nickname: String,

    // -- Base stats (copied from ow-data, may be modified by experience) --
    pub exp: i32,
    pub str_stat: i32,
    pub agl: i32,
    pub wil: i32,
    pub wsk: i32,
    pub hhc: i32,
    pub tch: i32,
    pub enc: i32,
    pub base_aps: i32,
    pub dpr: i32,

    // -- Mutable combat state --
    /// Maximum hit points (derived from STR at hire time).
    pub max_hp: u32,
    /// Current hit points.
    pub current_hp: u32,
    /// Current action points remaining this turn.
    pub current_ap: u32,
    /// Campaign-level status.
    pub status: MercStatus,
    /// Position on the tactical map, if deployed.
    pub position: Option<TilePos>,
    /// Currently carried items.
    pub inventory: Vec<InventoryItem>,
    /// Whether the merc is currently suppressed by enemy fire.
    pub suppressed: bool,
    /// Experience points gained during the current mission.
    pub experience_gained: u32,
}

impl ActiveMerc {
    /// Create an `ActiveMerc` from parsed data-file stats.
    ///
    /// HP is derived as `str_stat` (strength maps directly to toughness).
    /// AP is copied from the base `aps` value.
    pub fn from_data(id: MercId, merc: &ow_data::mercs::Mercenary) -> Self {
        let max_hp = merc.str_stat.max(1) as u32;
        debug!(
            id,
            name = %merc.name,
            max_hp,
            aps = merc.aps,
            "Creating ActiveMerc from data"
        );
        Self {
            id,
            name: merc.name.clone(),
            nickname: merc.nickname.clone(),
            exp: merc.exp,
            str_stat: merc.str_stat,
            agl: merc.agl,
            wil: merc.wil,
            wsk: merc.wsk,
            hhc: merc.hhc,
            tch: merc.tch,
            enc: merc.enc,
            base_aps: merc.aps,
            dpr: merc.dpr,
            max_hp,
            current_hp: max_hp,
            current_ap: merc.aps.max(0) as u32,
            status: MercStatus::Hired,
            position: None,
            inventory: Vec::new(),
            suppressed: false,
            experience_gained: 0,
        }
    }

    /// True if the merc has positive HP and is not KIA/MIA.
    pub fn is_alive(&self) -> bool {
        self.current_hp > 0 && self.status != MercStatus::KIA && self.status != MercStatus::MIA
    }

    /// True if the merc is alive, on-mission, and has AP remaining.
    pub fn can_act(&self) -> bool {
        self.is_alive() && self.status == MercStatus::OnMission && self.current_ap > 0
    }

    /// Calculate effective initiative for turn ordering.
    ///
    /// Base initiative = EXP + WIL. Halved (rounded down) if suppressed.
    pub fn initiative(&self) -> u32 {
        let base = (self.exp.max(0) + self.wil.max(0)) as u32;
        let init = if self.suppressed { base / 2 } else { base };
        trace!(
            name = %self.name,
            base,
            suppressed = self.suppressed,
            initiative = init,
            "Calculated initiative"
        );
        init
    }

    /// Total encumbrance of all carried items.
    pub fn total_encumbrance(&self) -> u32 {
        self.inventory.iter().map(|i| i.encumbrance).sum()
    }

    /// Movement cost per tile based on encumbrance ratio.
    ///
    /// Base cost is 2 AP per tile. Each 25% of capacity used adds +1 AP.
    /// Minimum cost is always 2.
    pub fn movement_cost_per_tile(&self) -> u32 {
        let capacity = self.enc.max(1) as u32;
        let load = self.total_encumbrance();
        let ratio = (load * 100) / capacity; // percent of capacity
        let extra = ratio / 25; // +1 per 25% bracket
        let cost = 2 + extra;
        trace!(
            name = %self.name,
            load,
            capacity,
            ratio,
            cost,
            "Calculated movement cost per tile"
        );
        cost
    }

    /// Reset AP to base value at the start of a new turn.
    /// Suppressed mercs get half AP.
    pub fn reset_ap(&mut self) {
        let full_ap = self.base_aps.max(0) as u32;
        self.current_ap = if self.suppressed {
            full_ap / 2
        } else {
            full_ap
        };
        trace!(name = %self.name, ap = self.current_ap, "Reset AP for new turn");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal test merc without needing ow-data.
    fn test_merc(exp: i32, wil: i32, suppressed: bool) -> ActiveMerc {
        ActiveMerc {
            id: 1,
            name: "Test".into(),
            nickname: "T".into(),
            exp,
            str_stat: 50,
            agl: 50,
            wil,
            wsk: 50,
            hhc: 40,
            tch: 30,
            enc: 300,
            base_aps: 38,
            dpr: 100,
            max_hp: 50,
            current_hp: 50,
            current_ap: 38,
            status: MercStatus::OnMission,
            position: Some(TilePos { x: 5, y: 3 }),
            inventory: Vec::new(),
            suppressed,
            experience_gained: 0,
        }
    }

    #[test]
    fn initiative_normal() {
        let m = test_merc(40, 45, false);
        assert_eq!(m.initiative(), 85); // 40 + 45
    }

    #[test]
    fn initiative_suppressed_halved() {
        let m = test_merc(40, 45, true);
        assert_eq!(m.initiative(), 42); // (40 + 45) / 2 = 42
    }

    #[test]
    fn initiative_zero_stats() {
        let m = test_merc(0, 0, false);
        assert_eq!(m.initiative(), 0);
    }

    #[test]
    fn is_alive_checks() {
        let mut m = test_merc(40, 45, false);
        assert!(m.is_alive());

        m.current_hp = 0;
        assert!(!m.is_alive());

        m.current_hp = 10;
        m.status = MercStatus::KIA;
        assert!(!m.is_alive());

        m.status = MercStatus::MIA;
        assert!(!m.is_alive());
    }

    #[test]
    fn can_act_requires_on_mission_and_ap() {
        let mut m = test_merc(40, 45, false);
        assert!(m.can_act());

        m.current_ap = 0;
        assert!(!m.can_act());

        m.current_ap = 10;
        m.status = MercStatus::Hired;
        assert!(!m.can_act());
    }

    #[test]
    fn movement_cost_scales_with_load() {
        let mut m = test_merc(40, 45, false);
        // Empty inventory: 0% load -> cost 2
        assert_eq!(m.movement_cost_per_tile(), 2);

        // 50% load (150 / 300) -> cost 2 + 2 = 4
        m.inventory.push(InventoryItem {
            name: "Rifle".into(),
            encumbrance: 150,
        });
        assert_eq!(m.movement_cost_per_tile(), 4);

        // 100% load (300 / 300) -> cost 2 + 4 = 6
        m.inventory.push(InventoryItem {
            name: "Ammo".into(),
            encumbrance: 150,
        });
        assert_eq!(m.movement_cost_per_tile(), 6);
    }

    #[test]
    fn reset_ap_normal_and_suppressed() {
        let mut m = test_merc(40, 45, false);
        m.current_ap = 0;
        m.reset_ap();
        assert_eq!(m.current_ap, 38);

        m.suppressed = true;
        m.reset_ap();
        assert_eq!(m.current_ap, 19); // 38 / 2
    }
}
