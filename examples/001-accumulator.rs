use bevy::prelude::*;

const GAUGE_Y: f32 = 0.0;
const DT_MULTIPLIER: f32 = 100.0;
const ACCUMULATOR_TO_SCALE_DIVIDER: f32 = 100.0;
const ACCUMULATOR_TO_MESH_HEIGHT_MULTIPLIER: f32 = 3.0;

#[derive(Component)]
struct Accumulator {
    maximum: u8,
    minimum: u8,
    value: u8,
}

#[derive(Component)]
struct Position(f32);

impl Accumulator {
    pub fn new(minimum: u8, maximum: u8) -> Self {
        Self {
            minimum,
            maximum,
            value: minimum,
        }
    }

    pub fn increment(&mut self, value: u8) {
        match vec![self.value + value, self.maximum].iter().min() {
            None => {}
            Some(acc) => {
                self.value = *acc;
            }
        }
    }

    pub fn decrement(&mut self, value: u8) {
        if value > self.value {
            return;
        }

        let projection = self.value - value;
        let values = vec![projection, self.minimum];
        let max = values.iter().max();

        match max {
            None => {}
            Some(acc) => {
                self.value = *acc;
            }
        }
    }

    // pub fn value(&self) -> u8 {
    //     self.value
    // }
}

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_systems(Startup, setup)
        .add_systems(Update, accumulate)
        .add_systems(Update, update_transform)
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    commands.spawn(Camera2d);

    commands.spawn((
        Transform::from_xyz(0.0, GAUGE_Y, -2.0),
        Mesh2d(meshes.add(Rectangle::new(300.0, 400.0))),
        MeshMaterial2d(materials.add(Color::srgb(0.5, 0.5, 0.5))),
    ));

    commands.spawn((
        Transform::from_xyz(0.0, GAUGE_Y, -1.0),
        Mesh2d(meshes.add(Rectangle::new(110.0, 310.0))),
        MeshMaterial2d(materials.add(Color::srgb(0.0, 0.0, 0.0))),
    ));

    commands.spawn((
        Accumulator::new(0, 100),
        Position(GAUGE_Y),
        Transform::from_xyz(0.0, GAUGE_Y, 0.0),
        Mesh2d(meshes.add(Rectangle::new(100.0, 300.0))),
        MeshMaterial2d(materials.add(Color::srgb(1.0, 1.0, 1.0))),
    ));
}

fn accumulate(
    mut query: Query<&mut Accumulator>,
    keyboard: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
) {
    let dt = time.delta_secs();

    if query.is_empty() {
        return;
    }

    let mut accumulator = query.single_mut();

    let diff = get_capped_gauge_variation(dt);

    if keyboard.pressed(KeyCode::Space) {
        accumulator.increment(diff);
    } else {
        accumulator.decrement(diff);
    }
}

fn get_capped_gauge_variation(dt: f32) -> u8 {
    let value: u8 = (DT_MULTIPLIER * dt).ceil() as u8;
    let mut values = vec![1, 3, value];
    values.sort();
    values[1]
}

fn update_transform(mut query: Query<(&Accumulator, &Position, &mut Transform)>) {
    if query.is_empty() {
        return;
    }

    let (accumulator, original_position, mut transform) = query.single_mut();
    transform.scale.y = accumulator.value as f32 / ACCUMULATOR_TO_SCALE_DIVIDER;

    let offset_maximum = accumulator.maximum as f32 / 2.0 * ACCUMULATOR_TO_MESH_HEIGHT_MULTIPLIER;
    let offset_value = accumulator.value as f32 / 2.0 * ACCUMULATOR_TO_MESH_HEIGHT_MULTIPLIER;
    transform.translation.y = original_position.0 - offset_maximum + offset_value;
}
