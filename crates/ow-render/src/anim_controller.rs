//! Animation controller for entity sprite playback.
//!
//! This module bridges `.COR` animation definitions (parsed by `ow_data::animation`)
//! and the binary `.DAT` sprite sheets (parsed by `ow_data::sprite`). A `.COR` file
//! is an index: each entry maps an (action, weapon_class, direction) triple to a range
//! of frames within the companion `.DAT` sprite sheet. The `AnimController` uses this
//! index to drive frame-by-frame playback at runtime.
//!
//! ## Mirror system
//!
//! To halve sprite storage, the original engine stores sprites for only one horizontal
//! half of the direction wheel (typically S, SW, W, NW, N) and mirrors them for the
//! opposite side (SE, E, NE). When a `.COR` entry has `mirror_flag == 2`, the renderer
//! should horizontally flip the sprite from the opposite direction rather than reading
//! unique pixel data. This is exposed via [`AnimController::mirror_horizontal`].

use ow_data::animation::{AnimationEntry, AnimationSet};
use tracing::{debug, trace, warn};

// ---------------------------------------------------------------------------
// Direction
// ---------------------------------------------------------------------------

/// Eight isometric facing directions, numbered clockwise from South.
///
/// The numbering matches the `.COR` direction field (0-7):
/// ```text
///          N (4)
///     NW (3)   NE (5)
///   W (2)         E (6)
///     SW (1)   SE (7)
///          S (0)
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Direction {
    S = 0,
    SW = 1,
    W = 2,
    NW = 3,
    N = 4,
    NE = 5,
    E = 6,
    SE = 7,
}

impl Direction {
    /// Convert a raw `.COR` direction value (0-7) into a `Direction`.
    /// Returns `None` for values outside the valid range.
    pub fn from_raw(value: u8) -> Option<Self> {
        match value {
            0 => Some(Direction::S),
            1 => Some(Direction::SW),
            2 => Some(Direction::W),
            3 => Some(Direction::NW),
            4 => Some(Direction::N),
            5 => Some(Direction::NE),
            6 => Some(Direction::E),
            7 => Some(Direction::SE),
            _ => None,
        }
    }

    /// Return the raw integer value for this direction.
    pub fn as_raw(self) -> u8 {
        self as u8
    }
}

// ---------------------------------------------------------------------------
// AnimAction
// ---------------------------------------------------------------------------

/// High-level animation actions for soldier entities.
///
/// These map from the entity-specific `action_id` values in `.COR` files.
/// The mapping is derived from label analysis of `JUNGSLD.COR`:
///
/// | action_id | AnimAction    |
/// |-----------|---------------|
/// | 0         | Walk          |
/// | 1         | Run           |
/// | 11        | Throw         |
/// | 26        | Crawl         |
/// | 31-36     | Die           |
/// | 41-44     | Melee         |
/// | 45-46     | Idle          |
/// | 50-52     | ShootStand    |
/// | 53-56     | ShootCrouch   |
/// | 58-59     | Die (posture) |
/// | 61        | Melee (animal)|
///
/// Not all action IDs map cleanly; posture transitions (2-7) and special
/// actions (23=kick door, 99=destruction) are not represented here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnimAction {
    /// Standing idle / rest sequence (action_id 45, 46).
    Idle,
    /// Walking movement (action_id 0).
    Walk,
    /// Running movement (action_id 1).
    Run,
    /// Fire weapon from standing posture (action_id 50, 51, 52).
    ShootStand,
    /// Fire weapon from kneeling/prone posture (action_id 53, 54, 55, 56).
    ShootCrouch,
    /// Hit reaction — entity takes damage.
    Hit,
    /// Death animation (action_id 31-36, 38, 40, 58, 59).
    Die,
    /// Prone crawling movement (action_id 26).
    Crawl,
    /// Grenade or object throw (action_id 11).
    Throw,
    /// Melee attack — knife slash, knife stab, punch (action_id 41-44, 61).
    Melee,
}

impl AnimAction {
    /// Convert a raw `.COR` `action_id` into an `AnimAction`.
    ///
    /// Returns `None` for action IDs that don't map to a known high-level
    /// action (e.g. posture transitions, door kicks, destruction).
    pub fn from_action_id(id: u32) -> Option<Self> {
        match id {
            0 => Some(AnimAction::Walk),
            1 => Some(AnimAction::Run),
            11 => Some(AnimAction::Throw),
            25 => Some(AnimAction::Die),       // animal death
            26 => Some(AnimAction::Crawl),
            29 | 30 => Some(AnimAction::Hit),   // weapon-specific hit reactions
            31..=36 => Some(AnimAction::Die),
            38 | 40 => Some(AnimAction::Die),   // posture death transitions
            41..=44 => Some(AnimAction::Melee),
            45 | 46 => Some(AnimAction::Idle),
            50..=52 => Some(AnimAction::ShootStand),
            53..=56 => Some(AnimAction::ShootCrouch),
            58 | 59 => Some(AnimAction::Die),   // kneel/prone death
            61 => Some(AnimAction::Melee),      // animal attack
            _ => None,
        }
    }

    /// Whether this action should loop when it reaches the last frame.
    ///
    /// Walk, Run, Idle, and Crawl loop continuously. All other actions
    /// (shooting, dying, throwing, melee) play once and hold on the final frame.
    pub fn is_looping(self) -> bool {
        matches!(self, AnimAction::Idle | AnimAction::Walk | AnimAction::Run | AnimAction::Crawl)
    }
}

// ---------------------------------------------------------------------------
// AnimState
// ---------------------------------------------------------------------------

/// Current playback state for an animation sequence.
#[derive(Debug, Clone)]
pub struct AnimState {
    /// The action being animated.
    pub action: AnimAction,
    /// Facing direction.
    pub direction: Direction,
    /// Weapon class (maps to `.COR` weapon_id field).
    pub weapon_class: u32,
    /// Current frame within the animation (0-based).
    pub current_frame: u32,
    /// Total number of frames in this animation sequence.
    pub total_frames: u32,
    /// Milliseconds elapsed since the current frame started.
    pub elapsed_ms: f32,
    /// Duration of each frame in milliseconds.
    pub frame_duration_ms: f32,
    /// Whether the animation loops back to frame 0 after the last frame.
    pub looping: bool,
    /// Whether the sprite should be horizontally flipped (mirror_flag == 2).
    pub mirror: bool,
}

// ---------------------------------------------------------------------------
// AnimController
// ---------------------------------------------------------------------------

/// Default frame duration in milliseconds (10 fps).
const DEFAULT_FRAME_DURATION_MS: f32 = 100.0;

/// Drives animation playback by mapping high-level actions to `.COR` entries
/// and advancing frames over time.
///
/// Usage:
/// 1. Construct with an [`AnimationSet`] parsed from a `.COR` file.
/// 2. Call [`set_action`](AnimController::set_action) to pick an animation.
/// 3. Call [`update`](AnimController::update) each tick with the delta time.
/// 4. Read [`current_frame_index`](AnimController::current_frame_index) to get the
///    sprite index into the companion `.DAT` sheet.
/// 5. Check [`mirror_horizontal`](AnimController::mirror_horizontal) to know if the
///    renderer should flip the sprite.
#[derive(Debug, Clone)]
pub struct AnimController {
    /// The full animation set from a `.COR` file.
    anim_set: AnimationSet,
    /// Current playback state. `None` until `set_action` is called.
    state: Option<AnimState>,
    /// The `.COR` entry currently driving playback.
    active_entry: Option<AnimationEntry>,
    /// Frame duration override. Applies to all actions uniformly.
    frame_duration_ms: f32,
}

impl AnimController {
    /// Create a new controller from a parsed `.COR` animation set.
    pub fn new(anim_set: AnimationSet) -> Self {
        debug!(
            dat = %anim_set.dat_filename,
            entries = anim_set.entries.len(),
            "AnimController created"
        );
        Self {
            anim_set,
            state: None,
            active_entry: None,
            frame_duration_ms: DEFAULT_FRAME_DURATION_MS,
        }
    }

    /// Override the per-frame duration (in milliseconds) for all animations.
    pub fn set_frame_duration(&mut self, duration_ms: f32) {
        self.frame_duration_ms = duration_ms;
    }

    /// Look up and activate an animation for the given action, direction, and weapon class.
    ///
    /// Searches the `.COR` entries for one matching the requested parameters.
    /// If no exact match is found, a warning is logged and the state is cleared.
    pub fn set_action(&mut self, action: AnimAction, direction: Direction, weapon_class: u32) {
        let entry = self.find_entry(action, direction, weapon_class);

        match entry {
            Some(e) => {
                let mirror = e.mirror_flag == 2;
                let total_frames = e.frame_count;
                let looping = action.is_looping();

                trace!(
                    ?action,
                    ?direction,
                    weapon_class,
                    frame_offset = e.frame_offset,
                    total_frames,
                    mirror,
                    looping,
                    "Animation set"
                );

                self.state = Some(AnimState {
                    action,
                    direction,
                    weapon_class,
                    current_frame: 0,
                    total_frames,
                    elapsed_ms: 0.0,
                    frame_duration_ms: self.frame_duration_ms,
                    looping,
                    mirror,
                });
                self.active_entry = Some(e);
            }
            None => {
                warn!(
                    ?action,
                    ?direction,
                    weapon_class,
                    "No matching animation entry found in .COR data"
                );
                self.state = None;
                self.active_entry = None;
            }
        }
    }

    /// Advance the animation by `delta_ms` milliseconds.
    ///
    /// Increments the elapsed time and advances to the next frame when the
    /// frame duration is exceeded. For looping animations, wraps back to
    /// frame 0. For non-looping animations, holds on the final frame.
    pub fn update(&mut self, delta_ms: f32) {
        let state = match self.state.as_mut() {
            Some(s) => s,
            None => return,
        };

        if state.total_frames == 0 {
            return;
        }

        state.elapsed_ms += delta_ms;

        while state.elapsed_ms >= state.frame_duration_ms {
            state.elapsed_ms -= state.frame_duration_ms;

            let next = state.current_frame + 1;
            if next >= state.total_frames {
                if state.looping {
                    state.current_frame = 0;
                    trace!(action = ?state.action, "Animation looped");
                } else {
                    // Hold on the last frame.
                    state.current_frame = state.total_frames.saturating_sub(1);
                    state.elapsed_ms = 0.0;
                    trace!(action = ?state.action, "Animation finished (holding last frame)");
                    return;
                }
            } else {
                state.current_frame = next;
            }
        }
    }

    /// Return the current sprite index into the companion `.DAT` sprite sheet.
    ///
    /// This is `frame_offset + current_frame` from the active `.COR` entry.
    /// Returns `0` if no animation is active.
    pub fn current_frame_index(&self) -> u32 {
        match (&self.state, &self.active_entry) {
            (Some(state), Some(entry)) => entry.frame_offset + state.current_frame,
            _ => 0,
        }
    }

    /// Whether the current animation has completed (non-looping only).
    ///
    /// Always returns `false` for looping animations and when no animation is active.
    pub fn is_finished(&self) -> bool {
        match &self.state {
            Some(state) => {
                !state.looping
                    && state.total_frames > 0
                    && state.current_frame >= state.total_frames - 1
            }
            None => false,
        }
    }

    /// Whether the renderer should horizontally flip the current sprite.
    ///
    /// Returns `true` when the active `.COR` entry has `mirror_flag == 2`,
    /// meaning the sprite data is stored for the opposite direction and
    /// should be mirrored at render time (e.g. E uses W sprites, flipped).
    pub fn mirror_horizontal(&self) -> bool {
        self.state.as_ref().map_or(false, |s| s.mirror)
    }

    /// Return a reference to the current animation state, if any.
    pub fn state(&self) -> Option<&AnimState> {
        self.state.as_ref()
    }

    /// Return a reference to the underlying animation set.
    pub fn animation_set(&self) -> &AnimationSet {
        &self.anim_set
    }

    // -- Private helpers -----------------------------------------------------

    /// Search the `.COR` entries for one matching the given action, direction,
    /// and weapon class.
    ///
    /// The match is done by converting each entry's `action_id` through
    /// [`AnimAction::from_action_id`] and comparing. For actions that map from
    /// multiple `action_id` values (e.g. Die maps from 31-36, 38, 40, 58, 59),
    /// the first matching entry is returned.
    fn find_entry(
        &self,
        action: AnimAction,
        direction: Direction,
        weapon_class: u32,
    ) -> Option<AnimationEntry> {
        self.anim_set
            .entries
            .iter()
            .find(|e| {
                let action_match = AnimAction::from_action_id(e.action_id) == Some(action);
                let dir_match = e.direction == direction.as_raw();
                let weapon_match = e.weapon_id as u32 == weapon_class;
                action_match && dir_match && weapon_match
            })
            .cloned()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ow_data::animation::{AnimationEntry, AnimationSet};

    /// Build a minimal AnimationSet for testing.
    fn make_test_anim_set() -> AnimationSet {
        AnimationSet {
            dat_filename: "TEST.dat".to_string(),
            add_filename: "TEST.add".to_string(),
            header_value: 1,
            total_animations: 5,
            entries: vec![
                // Walk S, weapon 0, 8 frames starting at offset 0
                AnimationEntry {
                    mirror_flag: 1,
                    frame_offset: 0,
                    action_id: 0, // Walk
                    weapon_id: 0,
                    direction: 0, // S
                    frame_count: 8,
                    sound_id: 10,
                    field8: 0,
                    field9: 1,
                },
                // Walk W (mirrored from E), weapon 0, 8 frames at offset 0
                AnimationEntry {
                    mirror_flag: 2,
                    frame_offset: 0,
                    action_id: 0, // Walk
                    weapon_id: 0,
                    direction: 2, // W
                    frame_count: 8,
                    sound_id: 10,
                    field8: 0,
                    field9: 1,
                },
                // Die S, weapon 0, 15 frames at offset 100
                AnimationEntry {
                    mirror_flag: 1,
                    frame_offset: 100,
                    action_id: 31, // Die (forward #1)
                    weapon_id: 0,
                    direction: 0, // S
                    frame_count: 15,
                    sound_id: 0,
                    field8: 0,
                    field9: 1,
                },
                // Idle S, weapon 0, 4 frames at offset 200
                AnimationEntry {
                    mirror_flag: 1,
                    frame_offset: 200,
                    action_id: 45, // Rest Sequence (standing)
                    weapon_id: 0,
                    direction: 0, // S
                    frame_count: 4,
                    sound_id: 0,
                    field8: 0,
                    field9: 1,
                },
                // ShootStand S, weapon 2, 3 frames at offset 300
                AnimationEntry {
                    mirror_flag: 1,
                    frame_offset: 300,
                    action_id: 51, // Fire Weapon
                    weapon_id: 2,  // Pistol
                    direction: 0,  // S
                    frame_count: 3,
                    sound_id: 20,
                    field8: 0,
                    field9: 1,
                },
            ],
        }
    }

    // -- Direction tests --

    #[test]
    fn direction_from_raw_valid() {
        assert_eq!(Direction::from_raw(0), Some(Direction::S));
        assert_eq!(Direction::from_raw(4), Some(Direction::N));
        assert_eq!(Direction::from_raw(7), Some(Direction::SE));
    }

    #[test]
    fn direction_from_raw_invalid() {
        assert_eq!(Direction::from_raw(8), None);
        assert_eq!(Direction::from_raw(255), None);
    }

    #[test]
    fn direction_round_trip() {
        for raw in 0..8u8 {
            let dir = Direction::from_raw(raw).unwrap();
            assert_eq!(dir.as_raw(), raw);
        }
    }

    // -- AnimAction tests --

    #[test]
    fn action_from_known_ids() {
        assert_eq!(AnimAction::from_action_id(0), Some(AnimAction::Walk));
        assert_eq!(AnimAction::from_action_id(1), Some(AnimAction::Run));
        assert_eq!(AnimAction::from_action_id(11), Some(AnimAction::Throw));
        assert_eq!(AnimAction::from_action_id(26), Some(AnimAction::Crawl));
        assert_eq!(AnimAction::from_action_id(31), Some(AnimAction::Die));
        assert_eq!(AnimAction::from_action_id(36), Some(AnimAction::Die));
        assert_eq!(AnimAction::from_action_id(42), Some(AnimAction::Melee));
        assert_eq!(AnimAction::from_action_id(45), Some(AnimAction::Idle));
        assert_eq!(AnimAction::from_action_id(51), Some(AnimAction::ShootStand));
        assert_eq!(AnimAction::from_action_id(54), Some(AnimAction::ShootCrouch));
        assert_eq!(AnimAction::from_action_id(61), Some(AnimAction::Melee));
    }

    #[test]
    fn action_from_unknown_id() {
        // Posture transitions, door kick, destruction
        assert_eq!(AnimAction::from_action_id(2), None);
        assert_eq!(AnimAction::from_action_id(23), None);
        assert_eq!(AnimAction::from_action_id(99), None);
    }

    #[test]
    fn looping_actions() {
        assert!(AnimAction::Idle.is_looping());
        assert!(AnimAction::Walk.is_looping());
        assert!(AnimAction::Run.is_looping());
        assert!(AnimAction::Crawl.is_looping());
    }

    #[test]
    fn non_looping_actions() {
        assert!(!AnimAction::Die.is_looping());
        assert!(!AnimAction::ShootStand.is_looping());
        assert!(!AnimAction::ShootCrouch.is_looping());
        assert!(!AnimAction::Hit.is_looping());
        assert!(!AnimAction::Throw.is_looping());
        assert!(!AnimAction::Melee.is_looping());
    }

    // -- AnimController tests --

    #[test]
    fn set_action_walk_south() {
        let mut ctrl = AnimController::new(make_test_anim_set());
        ctrl.set_action(AnimAction::Walk, Direction::S, 0);

        let state = ctrl.state().expect("state should be set");
        assert_eq!(state.action, AnimAction::Walk);
        assert_eq!(state.direction, Direction::S);
        assert_eq!(state.weapon_class, 0);
        assert_eq!(state.current_frame, 0);
        assert_eq!(state.total_frames, 8);
        assert!(state.looping);
        assert!(!state.mirror);
    }

    #[test]
    fn set_action_walk_west_mirrored() {
        let mut ctrl = AnimController::new(make_test_anim_set());
        ctrl.set_action(AnimAction::Walk, Direction::W, 0);

        let state = ctrl.state().expect("state should be set");
        assert!(state.mirror);
        assert!(ctrl.mirror_horizontal());
    }

    #[test]
    fn set_action_no_match_clears_state() {
        let mut ctrl = AnimController::new(make_test_anim_set());
        // First set a valid action
        ctrl.set_action(AnimAction::Walk, Direction::S, 0);
        assert!(ctrl.state().is_some());

        // Now set one that doesn't exist
        ctrl.set_action(AnimAction::Run, Direction::N, 5);
        assert!(ctrl.state().is_none());
    }

    #[test]
    fn update_advances_frames() {
        let mut ctrl = AnimController::new(make_test_anim_set());
        ctrl.set_action(AnimAction::Walk, Direction::S, 0);

        assert_eq!(ctrl.current_frame_index(), 0); // offset 0 + frame 0

        // Advance one full frame (100ms at default 10fps)
        ctrl.update(100.0);
        assert_eq!(ctrl.current_frame_index(), 1); // offset 0 + frame 1

        // Advance two more frames
        ctrl.update(200.0);
        assert_eq!(ctrl.current_frame_index(), 3); // offset 0 + frame 3
    }

    #[test]
    fn update_partial_frame_does_not_advance() {
        let mut ctrl = AnimController::new(make_test_anim_set());
        ctrl.set_action(AnimAction::Walk, Direction::S, 0);

        ctrl.update(50.0); // half a frame
        assert_eq!(ctrl.current_frame_index(), 0);

        ctrl.update(49.0); // still not quite a full frame
        assert_eq!(ctrl.current_frame_index(), 0);

        ctrl.update(1.0); // now exactly 100ms total
        assert_eq!(ctrl.current_frame_index(), 1);
    }

    #[test]
    fn looping_animation_wraps() {
        let mut ctrl = AnimController::new(make_test_anim_set());
        ctrl.set_action(AnimAction::Walk, Direction::S, 0);

        // 8 frames at 100ms each = advance to frame 7 (last), then wrap
        ctrl.update(800.0);
        assert_eq!(ctrl.state().unwrap().current_frame, 0); // wrapped
        assert!(!ctrl.is_finished());
    }

    #[test]
    fn non_looping_animation_holds_last_frame() {
        let mut ctrl = AnimController::new(make_test_anim_set());
        ctrl.set_action(AnimAction::Die, Direction::S, 0);

        // 15 frames, advance way past the end
        ctrl.update(2000.0);
        assert_eq!(ctrl.state().unwrap().current_frame, 14); // last frame
        assert!(ctrl.is_finished());
    }

    #[test]
    fn is_finished_false_during_playback() {
        let mut ctrl = AnimController::new(make_test_anim_set());
        ctrl.set_action(AnimAction::Die, Direction::S, 0);

        ctrl.update(100.0); // only 1 frame in
        assert!(!ctrl.is_finished());
    }

    #[test]
    fn is_finished_false_for_looping() {
        let mut ctrl = AnimController::new(make_test_anim_set());
        ctrl.set_action(AnimAction::Walk, Direction::S, 0);

        ctrl.update(10000.0); // lots of time
        assert!(!ctrl.is_finished());
    }

    #[test]
    fn is_finished_false_when_no_state() {
        let ctrl = AnimController::new(make_test_anim_set());
        assert!(!ctrl.is_finished());
    }

    #[test]
    fn current_frame_index_includes_offset() {
        let mut ctrl = AnimController::new(make_test_anim_set());
        ctrl.set_action(AnimAction::Die, Direction::S, 0);

        // Die has frame_offset = 100
        assert_eq!(ctrl.current_frame_index(), 100);

        ctrl.update(100.0);
        assert_eq!(ctrl.current_frame_index(), 101);
    }

    #[test]
    fn current_frame_index_zero_when_no_state() {
        let ctrl = AnimController::new(make_test_anim_set());
        assert_eq!(ctrl.current_frame_index(), 0);
    }

    #[test]
    fn mirror_horizontal_false_when_no_state() {
        let ctrl = AnimController::new(make_test_anim_set());
        assert!(!ctrl.mirror_horizontal());
    }

    #[test]
    fn weapon_class_filtering() {
        let mut ctrl = AnimController::new(make_test_anim_set());

        // ShootStand with pistol (weapon 2) exists
        ctrl.set_action(AnimAction::ShootStand, Direction::S, 2);
        assert!(ctrl.state().is_some());
        assert_eq!(ctrl.current_frame_index(), 300);

        // ShootStand with rifle (weapon 0) does not exist in test data
        ctrl.set_action(AnimAction::ShootStand, Direction::S, 0);
        assert!(ctrl.state().is_none());
    }

    #[test]
    fn custom_frame_duration() {
        let mut ctrl = AnimController::new(make_test_anim_set());
        ctrl.set_frame_duration(50.0); // 20fps
        ctrl.set_action(AnimAction::Walk, Direction::S, 0);

        ctrl.update(50.0);
        assert_eq!(ctrl.state().unwrap().current_frame, 1);

        ctrl.update(50.0);
        assert_eq!(ctrl.state().unwrap().current_frame, 2);
    }

    #[test]
    fn set_action_resets_frame() {
        let mut ctrl = AnimController::new(make_test_anim_set());
        ctrl.set_action(AnimAction::Walk, Direction::S, 0);
        ctrl.update(300.0); // advance to frame 3

        // Re-set the same action — should reset to frame 0
        ctrl.set_action(AnimAction::Walk, Direction::S, 0);
        assert_eq!(ctrl.state().unwrap().current_frame, 0);
        assert_eq!(ctrl.state().unwrap().elapsed_ms, 0.0);
    }

    #[test]
    fn update_with_no_state_is_noop() {
        let mut ctrl = AnimController::new(make_test_anim_set());
        ctrl.update(1000.0); // should not panic
        assert_eq!(ctrl.current_frame_index(), 0);
    }

    #[test]
    fn zero_frame_animation_does_not_advance() {
        let anim_set = AnimationSet {
            dat_filename: "ZERO.dat".to_string(),
            add_filename: "ZERO.add".to_string(),
            header_value: 1,
            total_animations: 1,
            entries: vec![AnimationEntry {
                mirror_flag: 1,
                frame_offset: 50,
                action_id: 45, // Idle
                weapon_id: 0,
                direction: 0,
                frame_count: 0, // zero frames (static placeholder)
                sound_id: 0,
                field8: 0,
                field9: 1,
            }],
        };

        let mut ctrl = AnimController::new(anim_set);
        ctrl.set_action(AnimAction::Idle, Direction::S, 0);

        ctrl.update(500.0);
        // Should not panic; frame stays at 0
        assert_eq!(ctrl.state().unwrap().current_frame, 0);
        assert_eq!(ctrl.current_frame_index(), 50);
    }
}
