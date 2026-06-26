use crate::input::{InputInjector, KeyCode, MouseButton};

#[derive(Debug, Clone, Default)]
pub struct GamepadState {
    pub left_stick_x: f32,  // -1.0 to 1.0
    pub left_stick_y: f32,  // -1.0 to 1.0
    pub right_stick_x: f32, // -1.0 to 1.0
    pub right_stick_y: f32, // -1.0 to 1.0
    pub button_a: bool,
    pub button_b: bool,
    pub button_x: bool,
    pub button_y: bool,
}

pub struct ControllerMapper;

impl ControllerMapper {
    pub fn new() -> Self {
        Self
    }

    pub fn map_gamepad_to_input(
        &self,
        state: &GamepadState,
        injector: &InputInjector,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // Apply deadzone filtering and convert left analog stick to mouse displacement
        let deadzone = 0.15;
        let mut dx = 0.0;
        let mut dy = 0.0;

        if state.left_stick_x.abs() > deadzone {
            dx = state.left_stick_x;
        }
        if state.left_stick_y.abs() > deadzone {
            dy = state.left_stick_y;
        }

        if dx != 0.0 || dy != 0.0 {
            // Scale velocity to motion delta (e.g. max 15 pixels per tick)
            let scale = 15.0;
            injector.move_mouse((dx * scale) as i32, (dy * scale) as i32)?;
        }

        // Map button states to mouse clicks / keyboard key presses
        if state.button_a {
            injector.click_mouse(MouseButton::Left, true)?;
        }
        if state.button_b {
            injector.press_key(KeyCode::Space, true)?;
        }
        if state.button_x {
            injector.click_mouse(MouseButton::Right, true)?;
        }
        if state.button_y {
            injector.press_key(KeyCode::Escape, true)?;
        }

        Ok(())
    }
}
