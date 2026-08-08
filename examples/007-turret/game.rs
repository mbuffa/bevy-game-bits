use bevy::prelude::*;

use crate::audio::{self, PlaySfx, Sfx};
use crate::config::*;
use crate::enemy::{self, ActiveWave, SimpleRng};
use crate::waves::WAVES;
use crate::{hud, placement, scene, turret, weapons};

/// Top-level game flow. Starting in `Building` gives the player a free setup
/// phase to place turrets before wave 1.
#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GamePhase {
    #[default]
    Building,
    WaveActive,
    Victory,
    Defeat,
}

/// Combat and placement run in both live phases; everything freezes on
/// Victory/Defeat.
pub fn gameplay_active(state: Res<State<GamePhase>>) -> bool {
    matches!(state.get(), GamePhase::Building | GamePhase::WaveActive)
}

/// Damage requests, decoupling the weapons that deal damage from the
/// enemy systems that apply it (and trigger the hit flash).
#[derive(Message)]
pub struct DamageMessage {
    pub target: Entity,
    pub amount: f32,
}

/// One per enemy leaving the field, kill or leak; drives the wave-end tally
/// and base damage.
#[derive(Message)]
pub struct EnemyResolved {
    pub leaked: bool,
    pub leak_cost: u32,
}

/// Index into `WAVES` of the wave being played (or prepared for).
#[derive(Resource, Default)]
pub struct CurrentWave(pub usize);

/// Hit points of the base; leaked enemies chip away at it.
#[derive(Resource)]
pub struct BaseHealth(pub u32);

impl Default for BaseHealth {
    fn default() -> Self {
        Self(BASE_HP)
    }
}

/// Currency for placing turrets; earned at the end of each wave.
#[derive(Resource)]
pub struct Materials(pub u32);

impl Default for Materials {
    fn default() -> Self {
        Self(START_MATERIALS)
    }
}

/// Fired when a placement click can't be afforded; the HUD reacts.
#[derive(Message)]
pub struct PlacementRejected;

/// Countdown of the build phase; Space skips it.
#[derive(Resource)]
pub struct BuildTimer(pub Timer);

/// Shared mesh/material handles, created once at startup. Enemies all share
/// one material handle; the hit flash swaps the component to `flash_material`
/// instead of mutating the asset, so flashing one enemy never affects others.
#[derive(Resource)]
pub struct GameAssets {
    pub enemy_mesh: Handle<Mesh>,
    pub enemy_material: Handle<StandardMaterial>,
    pub runner_material: Handle<StandardMaterial>,
    pub brute_material: Handle<StandardMaterial>,
    pub flash_material: Handle<StandardMaterial>,
    pub leg_mesh: Handle<Mesh>,
    pub leg_material: Handle<StandardMaterial>,
    pub hub_mesh: Handle<Mesh>,
    pub kinetic_base_material: Handle<StandardMaterial>,
    pub laser_base_material: Handle<StandardMaterial>,
    pub rock_mesh: Handle<Mesh>,
    pub rock_material_a: Handle<StandardMaterial>,
    pub rock_material_b: Handle<StandardMaterial>,
    pub barrel_mesh: Handle<Mesh>,
    pub barrel_material: Handle<StandardMaterial>,
    pub sensor_mesh: Handle<Mesh>,
    pub sensor_material: Handle<StandardMaterial>,
    pub projectile_mesh: Handle<Mesh>,
    pub projectile_material: Handle<StandardMaterial>,
}

pub struct GamePlugin;

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<GamePhase>()
            .add_message::<DamageMessage>()
            .add_message::<EnemyResolved>()
            .add_message::<PlacementRejected>()
            .add_message::<PlaySfx>()
            .insert_resource(SimpleRng::from_time())
            .init_resource::<CurrentWave>()
            .init_resource::<ActiveWave>()
            .init_resource::<BaseHealth>()
            .init_resource::<Materials>()
            .insert_resource(BuildTimer(Timer::from_seconds(
                BUILD_PHASE_SECONDS,
                TimerMode::Once,
            )))
            .add_systems(
                Startup,
                (setup_assets, scene::setup, hud::setup, audio::load_sfx).chain(),
            )
            .add_systems(OnEnter(GamePhase::Building), reset_build_timer)
            .add_systems(OnEnter(GamePhase::WaveActive), start_wave)
            .add_systems(
                Update,
                tick_build_phase.run_if(in_state(GamePhase::Building)),
            )
            .add_systems(
                Update,
                enemy::spawn_wave_enemies.run_if(in_state(GamePhase::WaveActive)),
            )
            .add_systems(
                Update,
                (
                    turret::sweep_sensors,
                    turret::acquire_and_validate_targets,
                    weapons::fire_kinetic,
                    weapons::fire_lasers,
                    enemy::apply_damage,
                    enemy::update_hit_flash,
                    enemy::resolve_enemies,
                    apply_base_damage,
                    check_wave_end,
                )
                    .chain()
                    .run_if(gameplay_active),
            )
            .add_systems(
                Update,
                (
                    placement::place_turret_on_click,
                    turret::draw_sensor_cones,
                    weapons::draw_laser_beams,
                )
                    .run_if(gameplay_active),
            )
            .add_systems(
                Update,
                (
                    hud::update_materials,
                    hud::update_base_hp,
                    hud::update_status,
                    restart,
                    audio::verify_sfx,
                    audio::play_sfx.after(audio::verify_sfx),
                    audio::update_laser_loops,
                ),
            )
            .add_systems(
                OnEnter(GamePhase::Victory),
                hud::spawn_banner(GamePhase::Victory),
            )
            .add_systems(
                OnEnter(GamePhase::Defeat),
                hud::spawn_banner(GamePhase::Defeat),
            )
            .add_systems(
                FixedUpdate,
                enemy::move_enemies.run_if(in_state(GamePhase::WaveActive)),
            )
            .add_systems(
                FixedUpdate,
                (weapons::move_projectiles, weapons::collide_projectiles)
                    .chain()
                    .run_if(gameplay_active),
            );
    }
}

fn reset_build_timer(mut timer: ResMut<BuildTimer>, mut sfx: MessageWriter<PlaySfx>) {
    timer.0.reset();
    sfx.write(PlaySfx(Sfx::BuildPhase));
}

fn tick_build_phase(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mut timer: ResMut<BuildTimer>,
    mut next: ResMut<NextState<GamePhase>>,
) {
    if timer.0.tick(time.delta()).is_finished() || keys.just_pressed(KeyCode::Space) {
        next.set(GamePhase::WaveActive);
    }
}

fn start_wave(
    current: Res<CurrentWave>,
    mut wave: ResMut<ActiveWave>,
    mut sfx: MessageWriter<PlaySfx>,
) {
    *wave = ActiveWave {
        cursors: vec![0; WAVES[current.0].groups.len()],
        ..default()
    };
    sfx.write(PlaySfx(Sfx::WaveStart));
}

/// Runs before `check_wave_end` so a leak that empties the base wins the
/// same-frame race against wave completion.
fn apply_base_damage(
    mut resolved: MessageReader<EnemyResolved>,
    mut base: ResMut<BaseHealth>,
    mut next: ResMut<NextState<GamePhase>>,
) {
    for message in resolved.read() {
        if message.leaked {
            base.0 = base.0.saturating_sub(message.leak_cost);
        }
    }

    if base.0 == 0 {
        next.set(GamePhase::Defeat);
    }
}

/// A wave ends when its schedule is exhausted and every spawned enemy has
/// been resolved (killed or leaked).
#[allow(clippy::too_many_arguments)]
fn check_wave_end(
    mut resolved: MessageReader<EnemyResolved>,
    mut wave: ResMut<ActiveWave>,
    mut current: ResMut<CurrentWave>,
    mut materials: ResMut<Materials>,
    mut sfx: MessageWriter<PlaySfx>,
    base: Res<BaseHealth>,
    state: Res<State<GamePhase>>,
    mut next: ResMut<NextState<GamePhase>>,
) {
    wave.resolved += resolved.read().count() as u32;

    // A dead base takes precedence; don't overwrite the Defeat transition.
    if *state.get() != GamePhase::WaveActive || base.0 == 0 {
        return;
    }
    if !wave.all_spawned(current.0) || wave.resolved < wave.spawned {
        return;
    }

    let reward = WAVES[current.0].reward;
    if reward > 0 {
        materials.0 += reward;
        sfx.write(PlaySfx(Sfx::MaterialsGained));
    }
    current.0 += 1;
    if current.0 >= WAVES.len() {
        next.set(GamePhase::Victory);
    } else {
        next.set(GamePhase::Building);
    }
}

/// Full reset from the end screens: clear the field, restore resources,
/// back to the first build phase. HUD and scenery persist.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn restart(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    state: Res<State<GamePhase>>,
    entities: Query<
        Entity,
        Or<(
            With<crate::enemy::Enemy>,
            With<turret::TurretKind>,
            With<weapons::Projectile>,
        )>,
    >,
    mut base: ResMut<BaseHealth>,
    mut materials: ResMut<Materials>,
    mut current: ResMut<CurrentWave>,
    mut wave: ResMut<ActiveWave>,
    mut next: ResMut<NextState<GamePhase>>,
) {
    if !matches!(state.get(), GamePhase::Victory | GamePhase::Defeat)
        || !keys.just_pressed(KeyCode::KeyR)
    {
        return;
    }

    for entity in &entities {
        commands.entity(entity).despawn();
    }
    *base = BaseHealth::default();
    *materials = Materials::default();
    *current = CurrentWave::default();
    *wave = ActiveWave::default();
    next.set(GamePhase::Building);
}

fn setup_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(GameAssets {
        enemy_mesh: meshes.add(Cuboid::new(ENEMY_SIZE.x, ENEMY_SIZE.y, ENEMY_SIZE.z)),
        enemy_material: materials.add(StandardMaterial {
            base_color: ENEMY_COLOR,
            perceptual_roughness: 0.8,
            ..default()
        }),
        runner_material: materials.add(StandardMaterial {
            base_color: RUNNER_COLOR,
            perceptual_roughness: 0.8,
            ..default()
        }),
        brute_material: materials.add(StandardMaterial {
            base_color: BRUTE_COLOR,
            perceptual_roughness: 0.8,
            ..default()
        }),
        // Unlit so the flash reads as a pure white blink regardless of lighting.
        flash_material: materials.add(StandardMaterial {
            base_color: Color::WHITE,
            unlit: true,
            ..default()
        }),
        leg_mesh: meshes.add(Cuboid::new(
            TRIPOD_LEG_THICKNESS,
            (TRIPOD_APEX_HEIGHT * TRIPOD_APEX_HEIGHT + TRIPOD_FOOT_RADIUS * TRIPOD_FOOT_RADIUS)
                .sqrt(),
            TRIPOD_LEG_THICKNESS,
        )),
        leg_material: materials.add(StandardMaterial {
            base_color: LEG_COLOR,
            perceptual_roughness: 0.6,
            ..default()
        }),
        hub_mesh: meshes.add(Sphere::new(0.3)),
        kinetic_base_material: materials.add(StandardMaterial {
            base_color: KINETIC_BASE_COLOR,
            perceptual_roughness: 0.9,
            ..default()
        }),
        laser_base_material: materials.add(StandardMaterial {
            base_color: LASER_BASE_COLOR,
            perceptual_roughness: 0.9,
            ..default()
        }),
        rock_mesh: meshes.add(Sphere::new(1.0)),
        rock_material_a: materials.add(StandardMaterial {
            base_color: ROCK_COLOR_A,
            perceptual_roughness: 1.0,
            ..default()
        }),
        rock_material_b: materials.add(StandardMaterial {
            base_color: ROCK_COLOR_B,
            perceptual_roughness: 1.0,
            ..default()
        }),
        barrel_mesh: meshes.add(Cuboid::new(0.25, 0.25, BARREL_LENGTH)),
        barrel_material: materials.add(StandardMaterial {
            base_color: BARREL_COLOR,
            perceptual_roughness: 0.6,
            ..default()
        }),
        sensor_mesh: meshes.add(Sphere::new(0.25)),
        sensor_material: materials.add(StandardMaterial {
            base_color: SENSOR_COLOR,
            perceptual_roughness: 0.4,
            ..default()
        }),
        projectile_mesh: meshes.add(Sphere::new(PROJECTILE_RADIUS)),
        projectile_material: materials.add(StandardMaterial {
            base_color: PROJECTILE_COLOR,
            unlit: true,
            ..default()
        }),
    });
}
