//! Top-level game state machine.
//!
//! Tracks the overall campaign phase (office, travel, mission, debrief),
//! the player's team, funds, reputation, and current mission context.

use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use crate::combat::CombatState;
use crate::merc::ActiveMerc;
use crate::weather::Weather;

/// Top-level game phase — where the player is in the campaign loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GamePhase {
    /// At the office: hiring, equipping, reviewing contracts.
    Office(OfficePhase),
    /// Traveling to a mission site.
    Travel,
    /// On a tactical mission.
    Mission(MissionPhase),
    /// Post-mission: tallying results, paying out, XP awards.
    Debrief,
}

/// Sub-phases within the office.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OfficePhase {
    /// Main office overview / dashboard.
    Overview,
    /// Browsing and hiring mercenaries.
    HireMercs,
    /// Buying / selling equipment and weapons.
    Equipment,
    /// Reading intelligence reports.
    Intel,
    /// Reviewing and accepting contracts.
    Contracts,
    /// Training mercs between missions.
    Training,
}

/// Sub-phases within a tactical mission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MissionPhase {
    /// Placing mercs on the map before combat starts.
    Deployment,
    /// Active turn-based combat.
    Combat,
    /// Moving to the extraction point after objectives complete.
    Extraction,
}

/// Context for the currently active mission, if any.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionContext {
    /// Mission name / identifier.
    pub name: String,
    /// Current weather on the battlefield.
    pub weather: Weather,
    /// Active combat state (initiative queue, units, etc.).
    pub combat: Option<CombatState>,
    /// Current tactical turn number within this mission.
    pub turn_number: u32,
}

/// The root game state — everything needed to save/load a campaign.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameState {
    /// Current phase of the campaign.
    pub phase: GamePhase,
    /// The player's team of hired mercenaries.
    pub team: Vec<ActiveMerc>,
    /// Current funds in dollars.
    pub funds: i64,
    /// Reputation / prestige score (can be negative).
    pub reputation: i32,
    /// Active mission context, if on a mission.
    pub current_mission: Option<MissionContext>,
    /// Global turn counter (incremented each mission).
    pub missions_completed: u32,
}

impl GameState {
    /// Create a new game with starting funds and empty roster.
    pub fn new(starting_funds: i64) -> Self {
        info!(funds = starting_funds, "Starting new game");
        Self {
            phase: GamePhase::Office(OfficePhase::Overview),
            team: Vec::new(),
            funds: starting_funds,
            reputation: 0,
            current_mission: None,
            missions_completed: 0,
        }
    }

    /// Transition to a new game phase.
    pub fn set_phase(&mut self, phase: GamePhase) {
        debug!(from = ?self.phase, to = ?phase, "Phase transition");
        self.phase = phase;
    }

    /// Add a merc to the player's team.
    pub fn hire_merc(&mut self, merc: ActiveMerc, hiring_fee: i64) {
        info!(
            name = %merc.name,
            fee = hiring_fee,
            funds_before = self.funds,
            "Hiring mercenary"
        );
        self.funds -= hiring_fee;
        self.team.push(merc);
    }

    /// Remove a merc from the team by id, returning them if found.
    pub fn fire_merc(&mut self, id: u32) -> Option<ActiveMerc> {
        if let Some(pos) = self.team.iter().position(|m| m.id == id) {
            let merc = self.team.remove(pos);
            info!(name = %merc.name, "Fired mercenary");
            Some(merc)
        } else {
            None
        }
    }

    /// Get all living team members.
    pub fn active_team(&self) -> Vec<&ActiveMerc> {
        self.team.iter().filter(|m| m.is_alive()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::merc::MercStatus;

    #[test]
    fn new_game_starts_in_office() {
        let state = GameState::new(500_000);
        assert!(matches!(
            state.phase,
            GamePhase::Office(OfficePhase::Overview)
        ));
        assert_eq!(state.funds, 500_000);
        assert!(state.team.is_empty());
    }

    #[test]
    fn phase_transitions() {
        let mut state = GameState::new(500_000);
        state.set_phase(GamePhase::Travel);
        assert_eq!(state.phase, GamePhase::Travel);
        state.set_phase(GamePhase::Mission(MissionPhase::Deployment));
        assert!(matches!(
            state.phase,
            GamePhase::Mission(MissionPhase::Deployment)
        ));
    }

    #[test]
    fn hire_deducts_funds() {
        let mut state = GameState::new(100_000);
        let merc = ActiveMerc {
            id: 1,
            name: "Test".into(),
            nickname: "T".into(),
            exp: 40,
            str_stat: 50,
            agl: 50,
            wil: 45,
            wsk: 50,
            hhc: 40,
            tch: 30,
            enc: 300,
            base_aps: 38,
            dpr: 100,
            max_hp: 50,
            current_hp: 50,
            current_ap: 38,
            status: MercStatus::Hired,
            position: None,
            inventory: Vec::new(),
            suppressed: false,
            experience_gained: 0,
        };
        state.hire_merc(merc, 25_000);
        assert_eq!(state.funds, 75_000);
        assert_eq!(state.team.len(), 1);
    }
}
