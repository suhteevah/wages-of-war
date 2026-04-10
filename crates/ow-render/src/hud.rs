//! Combat HUD overlay renderer.
//!
//! Draws the in-combat heads-up display using SDL2 drawing primitives.
//! No font rendering yet — text labels are represented as colored bars
//! whose width/color encode the data they represent. Real text rendering
//! will replace these once we wire up SDL2_ttf or bitmap fonts.
//!
//! ## HUD Layout (640x480 reference resolution)
//!
//! ```text
//! ┌──────────────────────────────────────────────┐
//! │                              [Turn / Phase]  │  ← top-right info panel
//! │                                              │
//! │                                              │
//! │              (game viewport)                 │
//! │                                              │
//! │                                              │
//! ├──────────────────────────────────────────────┤
//! │ [Unit Info]  [Action Buttons]  [Message Log] │  ← bottom panel
//! └──────────────────────────────────────────────┘
//! ```
//!
//! - **Unit Info** (bottom-left): selected merc's name bar, HP bar, AP bar,
//!   weapon/ammo indicator.
//! - **Action Buttons** (bottom-center): Move, Shoot, End Turn, etc. as
//!   colored rectangles — sprite-backed buttons will replace these later.
//! - **Message Log** (bottom-right): last 5 combat messages as colored bars.
//! - **Turn/Phase** (top-right): turn number and phase indicator.

use sdl2::pixels::Color;
use sdl2::rect::Rect as SdlRect;
use sdl2::render::Canvas;
use sdl2::video::Window;
use tracing::trace;

// ---------------------------------------------------------------------------
// Constants — HUD layout geometry for a 640x480 reference resolution.
// ---------------------------------------------------------------------------

/// Total screen width the HUD is designed for.
const SCREEN_W: i32 = 640;

/// Height of the bottom HUD panel in pixels.
const BOTTOM_PANEL_H: i32 = 100;
/// Y position where the bottom panel starts.
const BOTTOM_PANEL_Y: i32 = 480 - BOTTOM_PANEL_H;

/// Unit info sub-panel (bottom-left).
const UNIT_INFO_X: i32 = 4;
const UNIT_INFO_Y: i32 = BOTTOM_PANEL_Y + 4;
const UNIT_INFO_W: i32 = 180;

/// Action buttons sub-panel (bottom-center).
const BUTTONS_X: i32 = 200;
const BUTTONS_Y: i32 = BOTTOM_PANEL_Y + 10;
const BUTTON_W: i32 = 70;
const BUTTON_H: i32 = 28;
const BUTTON_GAP: i32 = 6;

/// Message log sub-panel (bottom-right).
const MSG_LOG_X: i32 = 440;
const MSG_LOG_Y: i32 = BOTTOM_PANEL_Y + 6;
const MSG_LOG_W: i32 = 192;
const MSG_LOG_LINE_H: i32 = 16;

/// Top-right turn/phase indicator.
const TURN_PANEL_X: i32 = SCREEN_W - 140;
const TURN_PANEL_Y: i32 = 4;
const TURN_PANEL_W: i32 = 134;
const TURN_PANEL_H: i32 = 40;

/// Maximum number of combat messages retained in the log.
pub const MAX_MESSAGES: usize = 5;

/// Bar dimensions for HP and AP display.
const BAR_W: i32 = 120;
const BAR_H: i32 = 10;

// ---------------------------------------------------------------------------
// HUD state — the data layer that feeds the renderer.
// ---------------------------------------------------------------------------

/// Information about the currently selected unit, displayed in the bottom-left
/// panel. Fields mirror what a player needs at a glance during combat.
#[derive(Debug, Clone)]
pub struct SelectedUnitInfo {
    /// Display name of the mercenary.
    pub name: String,
    /// Current hit points.
    pub hp: u32,
    /// Maximum hit points (determines bar length and color thresholds).
    pub max_hp: u32,
    /// Current action points remaining this turn.
    pub ap: u32,
    /// Maximum action points (full bar = fresh turn).
    pub max_ap: u32,
    /// Name of the currently equipped weapon (e.g. "M16A2").
    pub weapon_name: String,
    /// Remaining ammunition for the equipped weapon.
    pub ammo: u32,
}

/// Complete HUD state for one frame of combat rendering.
///
/// This struct is rebuilt or mutated each frame by the game loop and passed
/// into [`render_hud`] as a read-only snapshot.
#[derive(Debug, Clone)]
pub struct HudState {
    /// The merc currently selected by the player (if any).
    pub selected_unit: Option<SelectedUnitInfo>,
    /// Current turn number (1-based).
    pub turn_number: u32,
    /// Label for the current combat phase (e.g. "Player Phase", "AI Phase",
    /// "Resolution").
    pub phase_label: String,
    /// Rolling log of recent combat events. Newest messages are pushed to the
    /// back; when the vec exceeds [`MAX_MESSAGES`] the oldest entry is dropped.
    pub message_log: Vec<String>,
}

impl HudState {
    /// Create an empty HUD state for the start of combat.
    pub fn new() -> Self {
        Self {
            selected_unit: None,
            turn_number: 1,
            phase_label: "Setup".into(),
            message_log: Vec::new(),
        }
    }

    /// Push a combat message into the log, evicting the oldest if full.
    pub fn push_message(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        trace!(msg = %msg, "HUD message pushed");
        if self.message_log.len() >= MAX_MESSAGES {
            self.message_log.remove(0);
        }
        self.message_log.push(msg);
    }
}

impl Default for HudState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Rendering — SDL2 drawing primitives only, no fonts.
// ---------------------------------------------------------------------------

/// Draw the full combat HUD overlay onto the canvas.
///
/// This composites several sub-panels:
/// 1. Bottom panel background (semi-transparent dark bar).
/// 2. Selected-unit info (name placeholder, HP bar, AP bar, weapon/ammo).
/// 3. Action button placeholders (Move, Shoot, End Turn, etc.).
/// 4. Message log (colored bars representing recent combat messages).
/// 5. Top-right turn/phase indicator.
///
/// All coordinates assume a 640x480 canvas. The caller is responsible for
/// presenting the canvas after this call returns.
pub fn render_hud(canvas: &mut Canvas<Window>, hud_state: &HudState) {
    trace!(turn = hud_state.turn_number, "Rendering combat HUD");

    draw_bottom_panel(canvas);
    draw_turn_phase_indicator(canvas, hud_state.turn_number, &hud_state.phase_label);
    draw_action_buttons(canvas);
    draw_message_log(canvas, &hud_state.message_log);

    if let Some(ref unit) = hud_state.selected_unit {
        draw_unit_info(canvas, unit);
    }
}

/// Draw the dark background bar across the bottom of the screen.
/// This provides visual separation between the game viewport and the HUD.
fn draw_bottom_panel(canvas: &mut Canvas<Window>) {
    // Dark, slightly transparent panel background.
    canvas.set_draw_color(Color::RGBA(20, 20, 30, 220));
    let _ = canvas.fill_rect(SdlRect::new(0, BOTTOM_PANEL_Y, SCREEN_W as u32, BOTTOM_PANEL_H as u32));

    // Thin top border line for crispness.
    canvas.set_draw_color(Color::RGB(80, 80, 100));
    let _ = canvas.draw_line(
        sdl2::rect::Point::new(0, BOTTOM_PANEL_Y),
        sdl2::rect::Point::new(SCREEN_W, BOTTOM_PANEL_Y),
    );
}

/// Draw selected-unit info in the bottom-left: name placeholder, HP bar,
/// AP bar, weapon/ammo indicator.
fn draw_unit_info(canvas: &mut Canvas<Window>, unit: &SelectedUnitInfo) {
    let x = UNIT_INFO_X;
    let mut y = UNIT_INFO_Y;

    // -- Name placeholder: a white bar whose length loosely represents the name.
    // Real text rendering replaces this later.
    canvas.set_draw_color(Color::RGB(220, 220, 220));
    let name_bar_w = (unit.name.len() as i32 * 8).min(UNIT_INFO_W);
    let _ = canvas.fill_rect(SdlRect::new(x, y, name_bar_w as u32, 10));
    y += 16;

    // -- HP bar: green >50%, yellow 25-50%, red <25%.
    // Background (empty portion).
    canvas.set_draw_color(Color::RGB(40, 40, 40));
    let _ = canvas.fill_rect(SdlRect::new(x, y, BAR_W as u32, BAR_H as u32));

    // Filled portion.
    let hp_ratio = if unit.max_hp > 0 {
        unit.hp as f32 / unit.max_hp as f32
    } else {
        0.0
    };
    let hp_fill_w = (hp_ratio * BAR_W as f32) as u32;

    let hp_color = if hp_ratio > 0.5 {
        // Healthy: green.
        Color::RGB(40, 200, 40)
    } else if hp_ratio > 0.25 {
        // Wounded: yellow.
        Color::RGB(220, 200, 30)
    } else {
        // Critical: red.
        Color::RGB(220, 40, 40)
    };
    canvas.set_draw_color(hp_color);
    if hp_fill_w > 0 {
        let _ = canvas.fill_rect(SdlRect::new(x, y, hp_fill_w, BAR_H as u32));
    }

    // HP bar border.
    canvas.set_draw_color(Color::RGB(120, 120, 120));
    let _ = canvas.draw_rect(SdlRect::new(x, y, BAR_W as u32, BAR_H as u32));
    y += BAR_H + 4;

    // -- AP bar: discrete blue segments, one per AP point.
    // Each segment is a small rectangle. We draw max_ap slots and fill current_ap of them.
    let segment_gap = 1;
    let total_segments = unit.max_ap.min(20); // cap visual segments to avoid overflow
    let segment_w = if total_segments > 0 {
        ((BAR_W - (total_segments as i32 - 1) * segment_gap) / total_segments as i32).max(2)
    } else {
        4
    };

    for i in 0..total_segments {
        let sx = x + i as i32 * (segment_w + segment_gap);

        if i < unit.ap {
            // Filled AP segment: bright blue.
            canvas.set_draw_color(Color::RGB(60, 120, 230));
        } else {
            // Empty AP segment: dark grey.
            canvas.set_draw_color(Color::RGB(40, 40, 50));
        }
        let _ = canvas.fill_rect(SdlRect::new(sx, y, segment_w as u32, BAR_H as u32));

        // Segment border.
        canvas.set_draw_color(Color::RGB(80, 80, 100));
        let _ = canvas.draw_rect(SdlRect::new(sx, y, segment_w as u32, BAR_H as u32));
    }
    y += BAR_H + 6;

    // -- Weapon/ammo indicator.
    // Weapon name placeholder: cyan bar.
    canvas.set_draw_color(Color::RGB(100, 200, 200));
    let weapon_bar_w = (unit.weapon_name.len() as i32 * 6).min(UNIT_INFO_W - 40);
    let _ = canvas.fill_rect(SdlRect::new(x, y, weapon_bar_w as u32, 8));

    // Ammo count: small orange pips, one per round (capped at 30 for display).
    let ammo_display = unit.ammo.min(30);
    let ammo_x = x + weapon_bar_w + 6;
    canvas.set_draw_color(Color::RGB(230, 160, 40));
    for i in 0..ammo_display {
        let pip_x = ammo_x + (i as i32 * 4);
        if pip_x + 3 > x + UNIT_INFO_W {
            break; // don't overflow the sub-panel
        }
        let _ = canvas.fill_rect(SdlRect::new(pip_x, y, 3, 8));
    }
}

/// Draw placeholder action buttons in the bottom-center area.
///
/// These are colored rectangles standing in for the real sprite-backed buttons
/// that will be loaded from .BTN files + companion sprite sheets. Each button
/// gets a distinct color so they are visually distinguishable during development.
fn draw_action_buttons(canvas: &mut Canvas<Window>) {
    // Button definitions: (label-for-comments, fill color, border color).
    // The label is not rendered yet — it documents what each rectangle represents.
    let buttons: &[(&str, Color, Color)] = &[
        // Move button — green tint, the most common action.
        ("Move", Color::RGB(40, 120, 50), Color::RGB(80, 180, 90)),
        // Shoot button — red tint, offensive action.
        ("Shoot", Color::RGB(140, 40, 40), Color::RGB(200, 80, 80)),
        // End Turn button — grey/blue, finishes the current unit's activation.
        ("End Turn", Color::RGB(50, 60, 100), Color::RGB(90, 100, 160)),
    ];

    for (i, (_label, fill, border)) in buttons.iter().enumerate() {
        let bx = BUTTONS_X + i as i32 * (BUTTON_W + BUTTON_GAP);
        let by = BUTTONS_Y;

        // Button fill.
        canvas.set_draw_color(*fill);
        let _ = canvas.fill_rect(SdlRect::new(bx, by, BUTTON_W as u32, BUTTON_H as u32));

        // Button border.
        canvas.set_draw_color(*border);
        let _ = canvas.draw_rect(SdlRect::new(bx, by, BUTTON_W as u32, BUTTON_H as u32));

        trace!(button = _label, x = bx, y = by, "Drew action button placeholder");
    }
}

/// Draw the turn number and phase indicator in the top-right corner.
///
/// The turn number is represented as a series of white pips (one per turn,
/// capped for display). The phase is a colored bar: green for player phase,
/// red for AI phase, yellow for resolution.
fn draw_turn_phase_indicator(canvas: &mut Canvas<Window>, turn_number: u32, phase_label: &str) {
    // Panel background.
    canvas.set_draw_color(Color::RGBA(20, 20, 30, 200));
    let _ = canvas.fill_rect(SdlRect::new(
        TURN_PANEL_X,
        TURN_PANEL_Y,
        TURN_PANEL_W as u32,
        TURN_PANEL_H as u32,
    ));

    // Border.
    canvas.set_draw_color(Color::RGB(80, 80, 100));
    let _ = canvas.draw_rect(SdlRect::new(
        TURN_PANEL_X,
        TURN_PANEL_Y,
        TURN_PANEL_W as u32,
        TURN_PANEL_H as u32,
    ));

    // Turn number pips: small white squares, one per turn (cap at 20).
    let turn_display = turn_number.min(20);
    canvas.set_draw_color(Color::RGB(220, 220, 220));
    for i in 0..turn_display {
        let px = TURN_PANEL_X + 4 + (i as i32 * 6);
        if px + 4 > TURN_PANEL_X + TURN_PANEL_W - 4 {
            break;
        }
        let _ = canvas.fill_rect(SdlRect::new(px, TURN_PANEL_Y + 4, 4, 4));
    }

    // Phase color bar — color encodes the phase type.
    let phase_color = match phase_label {
        s if s.contains("Player") || s.contains("player") => Color::RGB(40, 180, 40),
        s if s.contains("AI") || s.contains("Enemy") || s.contains("enemy") => {
            Color::RGB(200, 50, 50)
        }
        s if s.contains("Resolution") || s.contains("resolution") => Color::RGB(200, 200, 50),
        _ => Color::RGB(120, 120, 140), // neutral/setup
    };
    canvas.set_draw_color(phase_color);
    let phase_bar_w = (phase_label.len() as i32 * 6).min(TURN_PANEL_W - 8);
    let _ = canvas.fill_rect(SdlRect::new(
        TURN_PANEL_X + 4,
        TURN_PANEL_Y + 14,
        phase_bar_w as u32,
        12,
    ));
}

/// Draw the message log in the bottom-right area.
///
/// Each message is rendered as a colored bar whose length is proportional
/// to the message string length. Newer messages are brighter; older ones
/// fade toward grey. This gives a visual sense of recency even without
/// actual text rendering.
fn draw_message_log(canvas: &mut Canvas<Window>, messages: &[String]) {
    for (i, msg) in messages.iter().enumerate() {
        let age = messages.len() - 1 - i; // 0 = newest
        let brightness = (220 - age as u8 * 30).max(80);

        // Color-code by message content keywords.
        let color = if msg.contains("hit") || msg.contains("damage") {
            Color::RGB(brightness, brightness / 3, brightness / 4)
        } else if msg.contains("suppressed") || msg.contains("pinned") {
            Color::RGB(brightness, brightness, brightness / 3)
        } else if msg.contains("miss") {
            Color::RGB(brightness / 2, brightness / 2, brightness)
        } else {
            Color::RGB(brightness, brightness, brightness)
        };

        canvas.set_draw_color(color);
        let bar_w = (msg.len() as i32 * 4).min(MSG_LOG_W);
        let my = MSG_LOG_Y + i as i32 * MSG_LOG_LINE_H;
        let _ = canvas.fill_rect(SdlRect::new(MSG_LOG_X, my, bar_w as u32, 10));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hud_state_new_is_empty() {
        let state = HudState::new();
        assert!(state.selected_unit.is_none());
        assert_eq!(state.turn_number, 1);
        assert_eq!(state.phase_label, "Setup");
        assert!(state.message_log.is_empty());
    }

    #[test]
    fn push_message_appends() {
        let mut state = HudState::new();
        state.push_message("Sarge hits Enemy #3 for 12 damage");
        assert_eq!(state.message_log.len(), 1);
        assert_eq!(state.message_log[0], "Sarge hits Enemy #3 for 12 damage");
    }

    #[test]
    fn push_message_evicts_oldest_at_capacity() {
        let mut state = HudState::new();
        for i in 0..MAX_MESSAGES + 3 {
            state.push_message(format!("Message {i}"));
        }
        // Should have exactly MAX_MESSAGES entries.
        assert_eq!(state.message_log.len(), MAX_MESSAGES);
        // Oldest surviving message should be #3 (0,1,2 evicted).
        assert_eq!(state.message_log[0], "Message 3");
        // Newest should be #7 (MAX_MESSAGES + 3 - 1 = 7).
        assert_eq!(
            state.message_log[MAX_MESSAGES - 1],
            format!("Message {}", MAX_MESSAGES + 2)
        );
    }

    #[test]
    fn default_matches_new() {
        let a = HudState::new();
        let b = HudState::default();
        assert_eq!(a.turn_number, b.turn_number);
        assert_eq!(a.phase_label, b.phase_label);
    }

    #[test]
    fn selected_unit_info_fields() {
        let info = SelectedUnitInfo {
            name: "Sarge".into(),
            hp: 30,
            max_hp: 50,
            ap: 4,
            max_ap: 8,
            weapon_name: "M16A2".into(),
            ammo: 24,
        };
        assert_eq!(info.hp, 30);
        assert_eq!(info.max_hp, 50);
        assert_eq!(info.ammo, 24);
    }
}
