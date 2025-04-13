use bevy::prelude::*;

pub struct JumpPlugin {
    pub screen_unit: f32,
}

#[derive(Resource)]
pub struct JumpConfig {
    screen_unit: f32,
}

impl Plugin for JumpPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(JumpConfig {
            screen_unit: self.screen_unit,
        });
    }
}

pub enum JumpingStates {
    Idle,
    Airborne,
}

#[derive(Component)]
pub struct JumpingState {
    state: JumpingStates,
    jump_started_at: f32,
    current_velocity: f32,
    key_was_released: bool,
}

impl JumpingState {
    pub fn default() -> Self {
        Self {
            state: JumpingStates::Idle,
            jump_started_at: 0.0,
            current_velocity: 0.0,
            key_was_released: false,
        }
    }

    pub fn reset(&mut self) {
        self.state = JumpingStates::Idle;
        self.jump_started_at = 0.0;
        self.key_was_released = false;
        self.current_velocity = 0.0;
    }
}

pub fn handle_jumping_state(
    mut query: Query<&mut JumpingState>,
    keyboard: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
) {
    if query.is_empty() {
        return;
    }

    let mut jumping_state = query.single_mut();

    if keyboard.just_pressed(KeyCode::Space) {
        match jumping_state.state {
            JumpingStates::Idle => {
                jumping_state.state = JumpingStates::Airborne;
                jumping_state.jump_started_at = time.elapsed_secs();
            }
            _ => {}
        }
    }

    if keyboard.just_released(KeyCode::Space) {
        match jumping_state.state {
            JumpingStates::Airborne => {
                jumping_state.key_was_released = true;
            }

            _ => {}
        }
    }
}

pub fn update_player_velocity(mut query: Query<&mut JumpingState>, time: Res<Time>) {
    if query.is_empty() {
        return;
    }

    let tt = time.elapsed_secs();

    let mut jumping_state = query.single_mut();

    if jumping_state.current_velocity < 0.0 {
        jumping_state.reset();
    }

    match jumping_state.state {
        JumpingStates::Airborne => {
            let x: f32 = tt - jumping_state.jump_started_at;

            // Formula:
            // h + v * x - 1/2 g * x²
            // Copy-Paste to Desmos:
            // h\ +\ v\cdot x-\frac{1}{2}\cdot g\cdot x^{2}

            let h: f32;
            let v: f32;
            let g: f32;
            let y: f32;

            // A jump lasts precisely 0.75 seconds.
            // If we release the space bar, we want to fall quicker.
            // FIXME: There's a "bounce" appearing if we release the jump key lately. I'm not sure how to fix it yet.
            if x > (0.2) && jumping_state.key_was_released {
                h = 0.0;
                v = 70.0;
                g = 200.0;
            } else {
                h = 0.0;
                v = 70.0;
                g = 160.0;
            }

            y = h + (v * x) - 0.5 * g * x.powi(2);
            println!("{:?}", y);

            if y < 0.0 {
                jumping_state.reset();
            } else {
                jumping_state.current_velocity = y;
            }
        }

        _ => {
            jumping_state.current_velocity = 0.0;
            jumping_state.jump_started_at = 0.0;
        }
    }
}

pub fn update_player_transform(
    mut query: Query<(&mut Transform, &JumpingState)>,
    jump_config: Res<JumpConfig>,
) {
    if query.is_empty() {
        return;
    }

    let (mut transform, jumping_state) = query.single_mut();

    if transform.translation.y < 0.0 {
        transform.translation.y = 0.0;
    } else {
        transform.translation.y = jumping_state.current_velocity * jump_config.screen_unit;
    }
}
