//! Menu/dialog UI system.
//!
//! Provides a lightweight immediate-mode-flavored UI layer built on SDL2
//! drawing primitives. The system converts parsed `.BTN` button layout data
//! (from `ow_data::buttons`) into renderable, interactive UI elements.
//!
//! ## Architecture
//!
//! ```text
//!  ┌──────────────┐      build_ui_from_buttons()      ┌──────────┐
//!  │ ButtonLayout  │ ──────────────────────────────►   │ UiState  │
//!  │ (.BTN file)   │                                   │          │
//!  └──────────────┘                                    └────┬─────┘
//!                                                           │
//!                          ┌────────────────────────────────┤
//!                          ▼                                ▼
//!                   handle_mouse_event()             render_ui()
//!                   (hit-test, returns               (SDL2 draw
//!                    clicked button ID)               primitives)
//! ```
//!
//! ## How .BTN files map to runtime UI
//!
//! Each `Button` in a `.BTN` `ButtonLayout` becomes a `UiElement::Button`:
//! - `Button::id` → `UiElement::Button::id` (used as the click return value).
//! - `Button::hit_rect` → `UiElement::Button::rect` (screen-space clickable area).
//! - `Button::page` → filters which buttons are visible (only page 0 is shown
//!   initially; page switching is handled by game logic).
//! - Sprite rects (`sprite_normal`, `sprite_hover`, etc.) are stored but not
//!   yet used — the renderer draws colored rectangles as placeholders until
//!   sprite sheet loading is implemented.
//!
//! Labels and Panels are created programmatically by game screens (menus,
//! dialogs, inventory) rather than from .BTN data.

use ow_data::buttons::{ButtonLayout, Rect as BtnRect};
use sdl2::pixels::Color;
use sdl2::rect::Rect as SdlRect;
use sdl2::render::Canvas;
use sdl2::video::Window;
use tracing::{debug, trace};

// ---------------------------------------------------------------------------
// Color constants for button visual states.
// ---------------------------------------------------------------------------

/// Normal (idle) button fill.
const COLOR_BTN_NORMAL: Color = Color::RGB(50, 55, 70);
/// Hovered button fill — slightly lighter to indicate mouse-over.
const COLOR_BTN_HOVER: Color = Color::RGB(75, 80, 100);
/// Pressed button fill — lightest, gives tactile feedback.
const COLOR_BTN_PRESSED: Color = Color::RGB(100, 110, 140);
/// Disabled button fill — greyed out, no interaction.
const COLOR_BTN_DISABLED: Color = Color::RGB(35, 35, 40);
/// Button border (all states except disabled).
const COLOR_BTN_BORDER: Color = Color::RGB(120, 125, 140);
/// Button border when disabled.
const COLOR_BTN_BORDER_DISABLED: Color = Color::RGB(55, 55, 60);

/// Panel background — dark, semi-transparent overlay.
const COLOR_PANEL_BG: Color = Color::RGBA(15, 15, 25, 200);
/// Panel border.
const COLOR_PANEL_BORDER: Color = Color::RGB(60, 60, 80);

/// Label placeholder color.
const COLOR_LABEL: Color = Color::RGB(200, 200, 210);

// ---------------------------------------------------------------------------
// UI element tree.
// ---------------------------------------------------------------------------

/// A single UI element in the element tree.
///
/// The engine uses a flat `Vec<UiElement>` rather than a recursive tree for
/// simplicity. Panels reference their children by index range (not stored
/// explicitly yet — the flat list is drawn in order via painter's algorithm).
#[derive(Debug, Clone)]
pub enum UiElement {
    /// A clickable button with visual state tracking.
    Button {
        /// Unique identifier returned by [`handle_mouse_event`] on click.
        /// Corresponds to `Button::id` from the .BTN file.
        id: u32,
        /// Screen-space bounding rectangle for hit-testing and rendering.
        rect: SdlRect,
        /// Human-readable label (not rendered yet — placeholder bars used).
        label: String,
        /// Whether the button accepts input. Disabled buttons are greyed out.
        enabled: bool,
        /// Whether the mouse cursor is currently over this button.
        hovered: bool,
        /// Whether the mouse button is held down on this button.
        pressed: bool,
    },

    /// A background panel that groups other elements visually.
    Panel {
        /// Screen-space rectangle for the panel background.
        rect: SdlRect,
        /// Indices into `UiState::elements` for child elements drawn on top.
        children: Vec<usize>,
    },

    /// A text label (rendered as a colored bar until font support is added).
    Label {
        /// Screen position (top-left corner).
        pos: (i32, i32),
        /// The text content (used to determine placeholder bar width).
        text: String,
    },
}

/// Root state for the UI system.
///
/// Holds the flat list of all UI elements and tracks which element (if any)
/// has keyboard/gamepad focus for accessibility and controller support.
#[derive(Debug, Clone)]
pub struct UiState {
    /// All UI elements, rendered in order (painter's algorithm — later elements
    /// draw on top of earlier ones).
    pub elements: Vec<UiElement>,
    /// Index of the currently focused element (for keyboard navigation).
    /// `None` means no element has focus.
    pub focused: Option<usize>,
}

impl UiState {
    /// Create an empty UI state with no elements.
    pub fn new() -> Self {
        Self {
            elements: Vec::new(),
            focused: None,
        }
    }
}

impl Default for UiState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Input handling — mouse hit-testing.
// ---------------------------------------------------------------------------

/// Process a mouse event against the UI element tree.
///
/// Updates hover/pressed state on all buttons, and returns the `id` of the
/// button that was clicked (if any). A "click" requires the mouse to be inside
/// the button rect AND `clicked` to be true.
///
/// Call this once per frame (or per mouse event) with the current cursor
/// position and button state.
///
/// # Returns
///
/// - `Some(id)` if an enabled button was clicked this frame.
/// - `None` if no button was clicked (mouse outside all buttons, or button
///   is disabled, or mouse was not clicked).
pub fn handle_mouse_event(
    ui_state: &mut UiState,
    mouse_x: i32,
    mouse_y: i32,
    clicked: bool,
) -> Option<u32> {
    let mut clicked_id: Option<u32> = None;

    for element in ui_state.elements.iter_mut() {
        match element {
            UiElement::Button {
                id,
                rect,
                enabled,
                ref mut hovered,
                ref mut pressed,
                ..
            } => {
                let inside = point_in_rect(mouse_x, mouse_y, *rect);

                *hovered = inside && *enabled;
                *pressed = inside && clicked && *enabled;

                if *pressed {
                    trace!(button_id = *id, "Button clicked");
                    clicked_id = Some(*id);
                }
            }
            // Panels and Labels don't respond to mouse events.
            UiElement::Panel { .. } | UiElement::Label { .. } => {}
        }
    }

    clicked_id
}

/// Point-in-rectangle test using SDL2 rect coordinates.
fn point_in_rect(x: i32, y: i32, rect: SdlRect) -> bool {
    x >= rect.x()
        && x < rect.x() + rect.width() as i32
        && y >= rect.y()
        && y < rect.y() + rect.height() as i32
}

// ---------------------------------------------------------------------------
// Rendering.
// ---------------------------------------------------------------------------

/// Render all UI elements onto the canvas.
///
/// Elements are drawn in order — panels first (as backgrounds), then buttons
/// and labels on top. The caller should ensure that `UiState::elements` is
/// ordered appropriately (panels before their children).
pub fn render_ui(canvas: &mut Canvas<Window>, ui_state: &UiState) {
    for element in &ui_state.elements {
        match element {
            UiElement::Panel { rect, .. } => {
                draw_panel(canvas, *rect);
            }
            UiElement::Button {
                rect,
                label,
                enabled,
                hovered,
                pressed,
                ..
            } => {
                draw_button(canvas, *rect, label, *enabled, *hovered, *pressed);
            }
            UiElement::Label { pos, text } => {
                draw_label(canvas, pos.0, pos.1, text);
            }
        }
    }
}

/// Draw a panel background with border.
fn draw_panel(canvas: &mut Canvas<Window>, rect: SdlRect) {
    // Semi-transparent dark background.
    canvas.set_draw_color(COLOR_PANEL_BG);
    let _ = canvas.fill_rect(rect);

    // Border.
    canvas.set_draw_color(COLOR_PANEL_BORDER);
    let _ = canvas.draw_rect(rect);
}

/// Draw a button with visual state feedback.
///
/// State priority: disabled > pressed > hovered > normal.
fn draw_button(
    canvas: &mut Canvas<Window>,
    rect: SdlRect,
    label: &str,
    enabled: bool,
    hovered: bool,
    pressed: bool,
) {
    // Choose fill color based on state.
    let fill = if !enabled {
        COLOR_BTN_DISABLED
    } else if pressed {
        COLOR_BTN_PRESSED
    } else if hovered {
        COLOR_BTN_HOVER
    } else {
        COLOR_BTN_NORMAL
    };

    // Fill.
    canvas.set_draw_color(fill);
    let _ = canvas.fill_rect(rect);

    // Border.
    let border = if enabled {
        COLOR_BTN_BORDER
    } else {
        COLOR_BTN_BORDER_DISABLED
    };
    canvas.set_draw_color(border);
    let _ = canvas.draw_rect(rect);

    // Label placeholder: a small bar inside the button, centered vertically.
    // Width is proportional to label length. Color dims when disabled.
    if !label.is_empty() {
        let label_w = (label.len() as u32 * 6).min(rect.width().saturating_sub(8));
        let label_x = rect.x() + 4;
        let label_y = rect.y() + (rect.height() as i32 / 2) - 3;
        let label_color = if enabled {
            COLOR_LABEL
        } else {
            Color::RGB(80, 80, 90)
        };
        canvas.set_draw_color(label_color);
        let _ = canvas.fill_rect(SdlRect::new(label_x, label_y, label_w, 6));
    }
}

/// Draw a text label as a colored bar (placeholder for real text rendering).
fn draw_label(canvas: &mut Canvas<Window>, x: i32, y: i32, text: &str) {
    if text.is_empty() {
        return;
    }
    canvas.set_draw_color(COLOR_LABEL);
    let w = (text.len() as u32 * 6).min(300);
    let _ = canvas.fill_rect(SdlRect::new(x, y, w, 10));
}

// ---------------------------------------------------------------------------
// .BTN → UiState conversion.
// ---------------------------------------------------------------------------

/// Convert a parsed `.BTN` button layout into a renderable [`UiState`].
///
/// Only page-0 buttons are included (the initial visible page). Game logic
/// can switch pages by rebuilding or filtering the UiState.
///
/// Each `Button` from the layout maps to a `UiElement::Button`:
/// - `id` is preserved for click identification.
/// - `hit_rect` becomes the SDL2 rect used for rendering and hit-testing.
/// - All buttons start enabled, not hovered, not pressed.
/// - Labels are set to `"btn_<id>"` as placeholders until we have string
///   table lookups wired in.
///
/// If any button has a non-zero `hit_rect`, a parent `Panel` is created
/// to span the bounding box of all buttons as a background.
pub fn build_ui_from_buttons(layout: &ButtonLayout) -> UiState {
    debug!(
        total_buttons = layout.buttons.len(),
        "Building UiState from ButtonLayout"
    );

    let mut elements: Vec<UiElement> = Vec::new();

    // Collect page-0 buttons and compute a bounding box for the parent panel.
    let page0_buttons: Vec<_> = layout.buttons.iter().filter(|b| b.page == 0).collect();

    if !page0_buttons.is_empty() {
        // Compute bounding box of all page-0 button hit rects.
        let mut min_x = i32::MAX;
        let mut min_y = i32::MAX;
        let mut max_x = i32::MIN;
        let mut max_y = i32::MIN;

        for btn in &page0_buttons {
            if !btn.hit_rect.is_empty() {
                min_x = min_x.min(btn.hit_rect.x1);
                min_y = min_y.min(btn.hit_rect.y1);
                max_x = max_x.max(btn.hit_rect.x2);
                max_y = max_y.max(btn.hit_rect.y2);
            }
        }

        // Only create a panel if we got valid bounds.
        if min_x < max_x && min_y < max_y {
            let panel_rect = SdlRect::new(
                min_x - 2,
                min_y - 2,
                (max_x - min_x + 4) as u32,
                (max_y - min_y + 4) as u32,
            );

            // Panel's children will be the button indices that follow.
            let child_start = 1; // index right after this panel
            let child_end = child_start + page0_buttons.len();
            let children: Vec<usize> = (child_start..child_end).collect();

            elements.push(UiElement::Panel {
                rect: panel_rect,
                children,
            });

            trace!(
                panel = ?panel_rect,
                "Created parent panel for page-0 buttons"
            );
        }
    }

    // Add each page-0 button as a UiElement::Button.
    for btn in &page0_buttons {
        let rect = btn_rect_to_sdl(&btn.hit_rect);
        let label = format!("btn_{}", btn.id);

        trace!(id = btn.id, rect = ?rect, "Added button from BTN layout");

        elements.push(UiElement::Button {
            id: btn.id,
            rect,
            label,
            enabled: true,
            hovered: false,
            pressed: false,
        });
    }

    debug!(
        element_count = elements.len(),
        "UiState built from ButtonLayout"
    );

    UiState {
        elements,
        focused: None,
    }
}

/// Convert an `ow_data::buttons::Rect` to an `sdl2::rect::Rect`.
///
/// The .BTN rect uses (x1, y1, x2, y2) inclusive corners; SDL2 uses
/// (x, y, width, height). Width/height are clamped to at least 1.
fn btn_rect_to_sdl(r: &BtnRect) -> SdlRect {
    let w = (r.x2 - r.x1).max(1) as u32;
    let h = (r.y2 - r.y1).max(1) as u32;
    SdlRect::new(r.x1, r.y1, w, h)
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ow_data::buttons::{Button, ButtonLayout, Rect as BtnRect};

    /// Helper: create a minimal Button with the given id and hit rect.
    fn make_button(id: u32, page: u32, x1: i32, y1: i32, x2: i32, y2: i32) -> Button {
        let zero_rect = BtnRect {
            x1: 0,
            y1: 0,
            x2: 0,
            y2: 0,
        };
        Button {
            field_1: 0,
            field_2: 0,
            page,
            id,
            hit_rect: BtnRect { x1, y1, x2, y2 },
            sprite_normal: zero_rect,
            sprite_pressed: zero_rect,
            sprite_hover: zero_rect,
            sprite_disabled: zero_rect,
            param_1: 0,
            param_2: 0,
            param_3: 0,
            param_4: 0,
        }
    }

    // -- Hit-testing tests --

    #[test]
    fn point_in_rect_inside() {
        let r = SdlRect::new(10, 20, 100, 50);
        assert!(point_in_rect(10, 20, r)); // top-left corner (inclusive)
        assert!(point_in_rect(50, 40, r)); // center
        assert!(point_in_rect(109, 69, r)); // bottom-right edge (exclusive boundary is 110,70)
    }

    #[test]
    fn point_in_rect_outside() {
        let r = SdlRect::new(10, 20, 100, 50);
        assert!(!point_in_rect(9, 20, r)); // just left
        assert!(!point_in_rect(10, 19, r)); // just above
        assert!(!point_in_rect(110, 20, r)); // just right (exclusive)
        assert!(!point_in_rect(10, 70, r)); // just below (exclusive)
        assert!(!point_in_rect(200, 200, r)); // far away
    }

    #[test]
    fn handle_mouse_click_returns_button_id() {
        let mut state = UiState {
            elements: vec![UiElement::Button {
                id: 7,
                rect: SdlRect::new(100, 100, 80, 40),
                label: "Test".into(),
                enabled: true,
                hovered: false,
                pressed: false,
            }],
            focused: None,
        };

        // Mouse inside, clicked → should return the button id.
        let result = handle_mouse_event(&mut state, 120, 120, true);
        assert_eq!(result, Some(7));

        // Verify state was updated.
        if let UiElement::Button { hovered, pressed, .. } = &state.elements[0] {
            assert!(*hovered);
            assert!(*pressed);
        } else {
            panic!("Expected Button element");
        }
    }

    #[test]
    fn handle_mouse_no_click_returns_none() {
        let mut state = UiState {
            elements: vec![UiElement::Button {
                id: 7,
                rect: SdlRect::new(100, 100, 80, 40),
                label: "Test".into(),
                enabled: true,
                hovered: false,
                pressed: false,
            }],
            focused: None,
        };

        // Mouse inside but not clicked → hover but no click.
        let result = handle_mouse_event(&mut state, 120, 120, false);
        assert_eq!(result, None);

        if let UiElement::Button { hovered, pressed, .. } = &state.elements[0] {
            assert!(*hovered);
            assert!(!(*pressed));
        } else {
            panic!("Expected Button element");
        }
    }

    #[test]
    fn handle_mouse_outside_returns_none() {
        let mut state = UiState {
            elements: vec![UiElement::Button {
                id: 7,
                rect: SdlRect::new(100, 100, 80, 40),
                label: "Test".into(),
                enabled: true,
                hovered: false,
                pressed: false,
            }],
            focused: None,
        };

        // Mouse outside, clicked → should not register.
        let result = handle_mouse_event(&mut state, 50, 50, true);
        assert_eq!(result, None);

        if let UiElement::Button { hovered, pressed, .. } = &state.elements[0] {
            assert!(!(*hovered));
            assert!(!(*pressed));
        } else {
            panic!("Expected Button element");
        }
    }

    #[test]
    fn handle_mouse_disabled_button_ignored() {
        let mut state = UiState {
            elements: vec![UiElement::Button {
                id: 7,
                rect: SdlRect::new(100, 100, 80, 40),
                label: "Test".into(),
                enabled: false,
                hovered: false,
                pressed: false,
            }],
            focused: None,
        };

        // Mouse inside and clicked, but button is disabled → None.
        let result = handle_mouse_event(&mut state, 120, 120, true);
        assert_eq!(result, None);

        if let UiElement::Button { hovered, pressed, .. } = &state.elements[0] {
            assert!(!(*hovered));
            assert!(!(*pressed));
        } else {
            panic!("Expected Button element");
        }
    }

    #[test]
    fn handle_mouse_multiple_buttons_returns_last_hit() {
        let mut state = UiState {
            elements: vec![
                UiElement::Button {
                    id: 1,
                    rect: SdlRect::new(100, 100, 80, 40),
                    label: "A".into(),
                    enabled: true,
                    hovered: false,
                    pressed: false,
                },
                UiElement::Button {
                    id: 2,
                    rect: SdlRect::new(150, 100, 80, 40), // overlaps with button 1
                    label: "B".into(),
                    enabled: true,
                    hovered: false,
                    pressed: false,
                },
            ],
            focused: None,
        };

        // Click in the overlap region (150-179, 100-139) — both buttons match,
        // the last one wins (topmost in painter's algorithm).
        let result = handle_mouse_event(&mut state, 160, 120, true);
        assert_eq!(result, Some(2));
    }

    // -- Button state tests --

    #[test]
    fn button_hover_clears_when_mouse_leaves() {
        let mut state = UiState {
            elements: vec![UiElement::Button {
                id: 1,
                rect: SdlRect::new(100, 100, 80, 40),
                label: "X".into(),
                enabled: true,
                hovered: false,
                pressed: false,
            }],
            focused: None,
        };

        // Hover.
        handle_mouse_event(&mut state, 120, 120, false);
        if let UiElement::Button { hovered, .. } = &state.elements[0] {
            assert!(*hovered);
        }

        // Move away.
        handle_mouse_event(&mut state, 0, 0, false);
        if let UiElement::Button { hovered, .. } = &state.elements[0] {
            assert!(!(*hovered));
        }
    }

    // -- build_ui_from_buttons tests --

    #[test]
    fn build_from_empty_layout() {
        let layout = ButtonLayout {
            buttons: Vec::new(),
        };
        let state = build_ui_from_buttons(&layout);
        assert!(state.elements.is_empty());
        assert!(state.focused.is_none());
    }

    #[test]
    fn build_creates_panel_and_buttons_for_page_0() {
        let layout = ButtonLayout {
            buttons: vec![
                make_button(1, 0, 10, 20, 90, 50),
                make_button(2, 0, 100, 20, 180, 50),
                make_button(3, 1, 200, 20, 280, 50), // page 1 — should be excluded
            ],
        };

        let state = build_ui_from_buttons(&layout);

        // Should have: 1 panel + 2 buttons = 3 elements.
        assert_eq!(state.elements.len(), 3);

        // First element is the panel.
        match &state.elements[0] {
            UiElement::Panel { rect, children } => {
                // Panel should span both page-0 buttons with 2px margin.
                assert_eq!(rect.x(), 8); // 10 - 2
                assert_eq!(rect.y(), 18); // 20 - 2
                assert_eq!(children.len(), 2);
            }
            _ => panic!("Expected Panel as first element"),
        }

        // Second and third elements are buttons.
        match &state.elements[1] {
            UiElement::Button { id, enabled, .. } => {
                assert_eq!(*id, 1);
                assert!(*enabled);
            }
            _ => panic!("Expected Button"),
        }
        match &state.elements[2] {
            UiElement::Button { id, .. } => assert_eq!(*id, 2),
            _ => panic!("Expected Button"),
        }
    }

    #[test]
    fn build_excludes_non_page_0_buttons() {
        let layout = ButtonLayout {
            buttons: vec![
                make_button(10, 1, 10, 20, 90, 50),
                make_button(11, 2, 100, 20, 180, 50),
            ],
        };

        let state = build_ui_from_buttons(&layout);
        // No page-0 buttons → no elements at all.
        assert!(state.elements.is_empty());
    }

    #[test]
    fn btn_rect_to_sdl_conversion() {
        let r = BtnRect {
            x1: 344,
            y1: 432,
            x2: 414,
            y2: 455,
        };
        let sdl = btn_rect_to_sdl(&r);
        assert_eq!(sdl.x(), 344);
        assert_eq!(sdl.y(), 432);
        assert_eq!(sdl.width(), 70); // 414 - 344
        assert_eq!(sdl.height(), 23); // 455 - 432
    }

    #[test]
    fn btn_rect_to_sdl_zero_size_clamped() {
        let r = BtnRect {
            x1: 0,
            y1: 0,
            x2: 0,
            y2: 0,
        };
        let sdl = btn_rect_to_sdl(&r);
        assert_eq!(sdl.width(), 1); // clamped to minimum 1
        assert_eq!(sdl.height(), 1);
    }

    // -- UiState construction --

    #[test]
    fn ui_state_default_is_empty() {
        let state = UiState::default();
        assert!(state.elements.is_empty());
        assert!(state.focused.is_none());
    }
}
