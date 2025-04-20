// Features a small Vampire Survivor-like movement and experience points grab.
// Hit Space to spawn XP Points.

use bevy::math::bounding::{Aabb2d, BoundingCircle, IntersectsVolume};
use bevy::prelude::*;

const GOLD: Srgba = bevy::color::palettes::css::GOLD;
const GREEN: Srgba = bevy::color::palettes::css::GREEN;
const WHITE: Srgba = bevy::color::palettes::css::BLACK;

const PLAYER_SPEED: f32 = 100.0;
const XP_SPEED: f32 = 20.0;

const PLAYER_SIZE: f32 = 32.0;
const COLLIDER_SIZE: f32 = 96.0;

#[derive(Component)]
struct XP;

#[derive(Component)]
struct Player;

#[derive(Component)]
struct PlayerCollider(f32);

#[derive(Resource)]
struct Experience(u128);

#[derive(Component)]
struct XpGauge;

#[derive(Event)]
struct CollisionEvent;

fn main() {
    App::new()
        .add_event::<CollisionEvent>()
        .insert_resource(Experience(0))
        .add_plugins(DefaultPlugins)
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                move_player,
                maybe_spawn_xp,
                detect_xp_collider_collision,
                detect_xp_player_collision,
                maybe_increase_score,
            ),
        )
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    window_q: Single<&Window>,
) {
    let window: &Window = window_q.into_inner();

    commands.spawn(Camera2d);

    commands.spawn((
        Node {
            width: Val::Px(0.0),
            max_width: Val::Px(window.width()),
            height: Val::Px(20.0),
            align_self: bevy::ui::AlignSelf::Auto,
            ..Default::default()
        },
        BackgroundColor(Color::from(GOLD)),
        XpGauge,
        Transform::from_xyz(0.0, 0.0 + 48.0 + 16.0, 0.0),
    ));

    commands
        .spawn((
            Player,
            Transform {
                translation: Vec3::new(0.0, 0.0, 0.0),
                scale: Vec3::new(PLAYER_SIZE, PLAYER_SIZE, 1.0),
                ..default()
            },
            Mesh2d(meshes.add(Rectangle::new(1.0, 1.0))),
            MeshMaterial2d(materials.add(Color::from(WHITE))),
        ))
        .with_children(|parent: &mut ChildBuilder<'_>| {
            parent.spawn((
                PlayerCollider(COLLIDER_SIZE),
                Transform {
                    translation: Vec3::new(0.0, 0.0, -1.0),
                    ..default()
                },
                Mesh2d(meshes.add(Circle::new(COLLIDER_SIZE / PLAYER_SIZE))),
                MeshMaterial2d(materials.add(Color::from(GREEN.with_alpha(0.1)))),
            ));
        });
}

fn move_player(
    player_q: Single<&mut Transform, With<Player>>,
    keyboard: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
) {
    let mut player = player_q.into_inner();

    let elapsed: f32 = time.delta_secs();

    if keyboard.pressed(KeyCode::ArrowLeft) {
        player.translation.x -= PLAYER_SPEED * elapsed;
    }

    if keyboard.pressed(KeyCode::ArrowUp) {
        player.translation.y += PLAYER_SPEED * elapsed;
    }

    if keyboard.pressed(KeyCode::ArrowRight) {
        player.translation.x += PLAYER_SPEED * elapsed;
    }

    if keyboard.pressed(KeyCode::ArrowDown) {
        player.translation.y -= PLAYER_SPEED * elapsed;
    }
}

fn maybe_spawn_xp(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    keyboard: Res<ButtonInput<KeyCode>>,
) {
    if keyboard.just_pressed(KeyCode::Space) {
        do_spawn_xp(&mut commands, &mut meshes, &mut materials);
    }
}

fn do_spawn_xp(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ColorMaterial>,
) {
    commands.spawn((
        XP,
        Transform::from_xyz(-100.0, 0.0, 1.0),
        Mesh2d(meshes.add(Rectangle::new(4.0, 4.0))),
        MeshMaterial2d(materials.add(Color::from(GOLD))),
    ));
}

fn detect_xp_collider_collision(
    collider_q: Single<&PlayerCollider>,
    player_q: Single<&Transform, With<Player>>,
    mut xp_q: Query<&mut Transform, (With<XP>, Without<PlayerCollider>, Without<Player>)>,
    time: Res<Time>,
) {
    let elapsed = time.delta_secs();

    let collider = collider_q.into_inner();
    let player_t = player_q.into_inner();

    let bounding_circle = BoundingCircle::new(player_t.translation.truncate(), collider.0);

    for mut xp_t in xp_q.iter_mut() {
        if do_detect_cllision(
            bounding_circle,
            Aabb2d::new(xp_t.translation.truncate(), xp_t.scale.truncate() / 2.0),
        ) {
            let diff = player_t.translation.truncate() - xp_t.translation.truncate();

            if diff.x > 0.0 {
                xp_t.translation.x += XP_SPEED * elapsed;
            } else {
                xp_t.translation.x -= XP_SPEED * elapsed;
            }

            if diff.y > 0.0 {
                xp_t.translation.y += XP_SPEED * elapsed;
            } else {
                xp_t.translation.y -= XP_SPEED * elapsed;
            }
        }
    }
}

fn do_detect_cllision(bounding_circle: BoundingCircle, aabb: Aabb2d) -> bool {
    if bounding_circle.intersects(&aabb) {
        return true;
    }
    return false;
}

fn detect_xp_player_collision(
    mut commands: Commands,
    player_q: Single<&Transform, With<Player>>,
    xp_q: Query<(Entity, &Transform), With<XP>>,
    mut events: EventWriter<CollisionEvent>,
) {
    let player_t = player_q.into_inner();

    let player_aabb = Aabb2d::new(
        player_t.translation.truncate(),
        player_t.scale.truncate() / 2.0,
    );

    for (xp_entity, xp_t) in xp_q.iter() {
        let xp_aabb = Aabb2d::new(xp_t.translation.truncate(), xp_t.scale.truncate() / 2.0);

        if player_aabb.intersects(&xp_aabb.bounding_circle()) {
            events.send(CollisionEvent);

            commands.entity(xp_entity).despawn();
        }
    }
}

fn maybe_increase_score(
    mut experience: ResMut<Experience>,
    gauge_q: Single<&mut Node, With<XpGauge>>,
    mut events: EventReader<CollisionEvent>,
) {
    if events.is_empty() {
        return;
    }

    let mut gauge_node = gauge_q.into_inner();

    let max_width: f32 = match gauge_node.max_width {
        Val::Px(max_width) => max_width,
        _ => 0.0,
    };

    for _event in events.read() {
        experience.0 += 100;
        gauge_node.width = Val::Px(experience.0 as f32 / 1000.0) * max_width;
    }
}
