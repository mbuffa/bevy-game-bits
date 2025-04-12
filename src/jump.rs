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
    pub state: JumpingStates,
    pub jump_started_at: f32,
    pub current_velocity: f32,
}

pub fn handle_jumping_state(
    mut query: Query<&mut JumpingState>,
    keyboard: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
) {
    if query.is_empty() {
        return;
    }

    if keyboard.just_pressed(KeyCode::Space) {
        let mut jumping_state = query.single_mut();

        match jumping_state.state {
            JumpingStates::Idle => {
                jumping_state.state = JumpingStates::Airborne;
                jumping_state.jump_started_at = time.elapsed_secs();
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
        jumping_state.state = JumpingStates::Idle;
    }

    match jumping_state.state {
        JumpingStates::Airborne => {
            let x: f32 = tt - jumping_state.jump_started_at;
            // h\ +\ v\cdot x-\frac{1}{2}\cdot g\cdot x^{2}
            // h + v * x - 1/2 g * x²
            let y: f32 = 0.0 + (70.0 * x) - 0.5 * 160.0 * x.powi(2);
            jumping_state.current_velocity = y;
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
