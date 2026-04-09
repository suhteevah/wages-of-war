//! Initiative-based combat system.
//!
//! All units (player and enemy) are sorted into a single initiative queue
//! each round. This is NOT IGOUGO — a fast enemy acts before a slow player merc.

use std::collections::BinaryHeap;

use serde::{Deserialize, Serialize};
use tracing::{debug, info, trace};

use crate::merc::{ActiveMerc, MercId};

/// Which side a unit fights for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Faction {
    Player,
    Enemy,
    Neutral,
}

/// A combat unit — wraps an `ActiveMerc` with faction info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CombatUnit {
    pub merc: ActiveMerc,
    pub faction: Faction,
}

/// An entry in the initiative priority queue.
/// Higher initiative = acts first. Ties broken by unit id (lower id first).
#[derive(Debug, Clone, Eq, PartialEq)]
struct InitiativeEntry {
    initiative: u32,
    unit_id: MercId,
}

impl Ord for InitiativeEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.initiative
            .cmp(&other.initiative)
            .then_with(|| other.unit_id.cmp(&self.unit_id)) // lower id wins ties
    }
}

impl PartialOrd for InitiativeEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Current phase within a combat round.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CombatPhase {
    /// Waiting for round to begin.
    PreRound,
    /// Units are taking turns in initiative order.
    InProgress,
    /// All units have acted; ready for next round.
    RoundComplete,
}

/// Top-level combat state managing the initiative queue and turn flow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CombatState {
    /// All participating units, indexed by their `merc.id`.
    pub units: Vec<CombatUnit>,
    /// Current round number (1-based).
    pub turn_number: u32,
    /// Id of the unit currently acting, if any.
    pub current_unit_id: Option<MercId>,
    /// Current phase of the round.
    pub phase: CombatPhase,
    /// The initiative queue (not serialized — rebuilt each round).
    #[serde(skip)]
    queue: BinaryHeap<InitiativeEntry>,
}

impl CombatState {
    /// Create a new combat with the given units. Round starts at 0 (call `begin_round` first).
    pub fn new(units: Vec<CombatUnit>) -> Self {
        info!(unit_count = units.len(), "Initializing combat state");
        Self {
            units,
            turn_number: 0,
            current_unit_id: None,
            phase: CombatPhase::PreRound,
            queue: BinaryHeap::new(),
        }
    }

    /// Begin a new combat round: increment turn counter, recalculate all
    /// initiatives, build the priority queue.
    pub fn begin_round(&mut self) {
        self.turn_number += 1;
        self.phase = CombatPhase::InProgress;
        self.current_unit_id = None;
        self.queue.clear();

        for unit in &mut self.units {
            if unit.merc.is_alive() {
                unit.merc.reset_ap();
                let init = unit.merc.initiative();
                trace!(
                    id = unit.merc.id,
                    name = %unit.merc.name,
                    faction = ?unit.faction,
                    initiative = init,
                    "Queued unit"
                );
                self.queue.push(InitiativeEntry {
                    initiative: init,
                    unit_id: unit.merc.id,
                });
            }
        }

        info!(
            round = self.turn_number,
            queued = self.queue.len(),
            "Started combat round"
        );
    }

    /// Pop the next unit from the initiative queue.
    ///
    /// Returns `None` when all units have acted this round (sets phase to `RoundComplete`).
    pub fn next_unit(&mut self) -> Option<MercId> {
        // Skip dead units that may have been killed mid-round
        while let Some(entry) = self.queue.pop() {
            if let Some(unit) = self.find_unit(entry.unit_id) {
                if unit.merc.is_alive() && unit.merc.can_act() {
                    debug!(
                        id = entry.unit_id,
                        initiative = entry.initiative,
                        "Next unit to act"
                    );
                    self.current_unit_id = Some(entry.unit_id);
                    return Some(entry.unit_id);
                }
            }
        }

        debug!(round = self.turn_number, "All units have acted");
        self.phase = CombatPhase::RoundComplete;
        self.current_unit_id = None;
        None
    }

    /// Signal that the current unit has finished its turn.
    pub fn end_turn(&mut self) {
        if let Some(id) = self.current_unit_id {
            if let Some(unit) = self.find_unit_mut(id) {
                unit.merc.current_ap = 0; // force turn end
                trace!(id, "Unit ended turn");
            }
        }
        self.current_unit_id = None;
    }

    /// Look up a unit by id (immutable).
    pub fn find_unit(&self, id: MercId) -> Option<&CombatUnit> {
        self.units.iter().find(|u| u.merc.id == id)
    }

    /// Look up a unit by id (mutable).
    pub fn find_unit_mut(&mut self, id: MercId) -> Option<&mut CombatUnit> {
        self.units.iter_mut().find(|u| u.merc.id == id)
    }

    /// Get all living units of a given faction.
    pub fn living_units(&self, faction: Faction) -> Vec<&CombatUnit> {
        self.units
            .iter()
            .filter(|u| u.faction == faction && u.merc.is_alive())
            .collect()
    }

    /// Check if combat is over (one side eliminated).
    pub fn is_combat_over(&self) -> bool {
        let players_alive = self.living_units(Faction::Player).len();
        let enemies_alive = self.living_units(Faction::Enemy).len();
        players_alive == 0 || enemies_alive == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::merc::{MercStatus, TilePos};

    fn make_unit(id: MercId, faction: Faction, exp: i32, wil: i32) -> CombatUnit {
        CombatUnit {
            merc: ActiveMerc {
                id,
                name: format!("Unit_{id}"),
                nickname: format!("U{id}"),
                exp,
                str_stat: 50,
                agl: 50,
                wil,
                wsk: 50,
                hhc: 40,
                tch: 30,
                enc: 300,
                base_aps: 20,
                dpr: 100,
                max_hp: 50,
                current_hp: 50,
                current_ap: 20,
                status: MercStatus::OnMission,
                position: Some(TilePos { x: 0, y: 0 }),
                inventory: Vec::new(),
                suppressed: false,
                experience_gained: 0,
            },
            faction,
        }
    }

    #[test]
    fn initiative_order_highest_first() {
        let units = vec![
            make_unit(1, Faction::Player, 20, 20), // init 40
            make_unit(2, Faction::Enemy, 50, 40),   // init 90
            make_unit(3, Faction::Player, 30, 30),  // init 60
        ];

        let mut combat = CombatState::new(units);
        combat.begin_round();

        assert_eq!(combat.next_unit(), Some(2)); // 90
        combat.end_turn();
        assert_eq!(combat.next_unit(), Some(3)); // 60
        combat.end_turn();
        assert_eq!(combat.next_unit(), Some(1)); // 40
        combat.end_turn();
        assert_eq!(combat.next_unit(), None);
        assert_eq!(combat.phase, CombatPhase::RoundComplete);
    }

    #[test]
    fn dead_units_skipped() {
        let units = vec![
            make_unit(1, Faction::Player, 50, 50), // init 100
            make_unit(2, Faction::Enemy, 40, 40),   // init 80
        ];

        let mut combat = CombatState::new(units);

        // Kill unit 1 before the round
        combat.units[0].merc.current_hp = 0;
        combat.units[0].merc.status = MercStatus::KIA;

        combat.begin_round();
        assert_eq!(combat.next_unit(), Some(2));
        combat.end_turn();
        assert_eq!(combat.next_unit(), None);
    }

    #[test]
    fn mixed_factions_interleaved() {
        let units = vec![
            make_unit(1, Faction::Player, 30, 30),  // 60
            make_unit(2, Faction::Enemy, 50, 50),    // 100
            make_unit(3, Faction::Player, 40, 40),   // 80
            make_unit(4, Faction::Enemy, 35, 35),    // 70
        ];

        let mut combat = CombatState::new(units);
        combat.begin_round();

        // Should interleave: Enemy(100), Player(80), Enemy(70), Player(60)
        let order: Vec<MercId> = std::iter::from_fn(|| {
            let id = combat.next_unit()?;
            combat.end_turn();
            Some(id)
        })
        .collect();

        assert_eq!(order, vec![2, 3, 4, 1]);
    }

    #[test]
    fn combat_over_detection() {
        let units = vec![
            make_unit(1, Faction::Player, 30, 30),
            make_unit(2, Faction::Enemy, 30, 30),
        ];

        let mut combat = CombatState::new(units);
        assert!(!combat.is_combat_over());

        combat.units[1].merc.current_hp = 0;
        combat.units[1].merc.status = MercStatus::KIA;
        assert!(combat.is_combat_over());
    }

    #[test]
    fn multiple_rounds() {
        let units = vec![
            make_unit(1, Faction::Player, 30, 30),
            make_unit(2, Faction::Enemy, 40, 40),
        ];

        let mut combat = CombatState::new(units);

        combat.begin_round();
        assert_eq!(combat.turn_number, 1);
        while combat.next_unit().is_some() {
            combat.end_turn();
        }

        combat.begin_round();
        assert_eq!(combat.turn_number, 2);
        assert_eq!(combat.phase, CombatPhase::InProgress);
        assert_eq!(combat.next_unit(), Some(2)); // higher init goes first again
    }
}
