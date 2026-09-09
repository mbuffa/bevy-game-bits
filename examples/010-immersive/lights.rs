//! Warehouse lighting and the wall switch that controls it (SPEC.md Phase 9).
//!
//! The room loads dark: `GlobalAmbientLight` sits at `config::AMBIENT_DARK`,
//! there is no sun, and the big overhead bank of `LightFixture` lamps is
//! *off*. Four bracket lamps — one on each platform pillar — stay lit (on a
//! `targetname` no switch drives), and the switch carries its own red
//! `PointLight` on platform B's wall as a beacon. Stack crates onto platform B
//! (it has no ladder — that's Phase 8), walk to the switch, press E, and the
//! warehouse comes on.
//!
//! | system / observer | when | job |
//! |---|---|---|
//! | `setup_lamp_assets` | `Startup` | shared shade / lens / plate / indicator materials |
//! | `spawn_fixtures` | `On<Add, LightFixture>` | build the lamp mesh + a downward `SpotLight`, insert `Powered` |
//! | `setup_switches` | `On<SceneCollidersReady>` | build the plate + indicator from the brush AABB, insert `SwitchState` |
//! | `toggle_on_interact` | `On<Interacted>` | flip the switch, push its state to every matching fixture, rewrite the prompt |
//! | `sync_fixtures` | `Update` | `Powered` → `SpotLight` intensity + lens material (idempotent mirror) |
//! | `sync_switch_indicators` | `Update` | `SwitchState` → indicator material |
//! | `sync_ambient` | `Update` | any switch on → `AMBIENT_LIT`, else `AMBIENT_DARK` |
//!
//! Three deliberate choices:
//!
//! 1. **The `sync_*` systems are idempotent every-frame mirrors, not
//!    `OnEnter`-style edges** — the `src/inventory` `sync_window_visibility`
//!    lesson: a fixture spawned mid-game has to come up in the right state
//!    with no transition to hook. There are ≤ 9 fixtures, so it's free.
//! 2. **`sync_ambient` keys off *switches*, not fixtures.** The pillar
//!    brackets are always `Powered`, so "any fixture lit" would pin the
//!    ambient bright forever and there'd be no dark to fix.
//! 3. **The prompt is the existing `Interactable::prompt`**, mutated on
//!    toggle — `interact::update_focus` clones it every frame and
//!    `ui::update_prompt` already renders it. No new UI.
//!
//! Wiring uses bevy_trenchbroom's built-in `Target` / `Targetable` entity-IO
//! base classes for the switch→lamp link (`target` / `targetname` strings).
//! 0.13's IO is a data-only skeleton, so `toggle_on_interact` does the
//! dispatch itself — but the FGD properties and editor affordances come free.

use avian3d::prelude::*;
use bevy::prelude::*;
use bevy_trenchbroom::physics::SceneCollidersReady;
use bevy_trenchbroom::prelude::{Target, Targetable};

use crate::classes::{FuncLightSwitch, Interactable, LightFixture};
use crate::config;
use crate::devtools;
use crate::interact::Interacted;

/// On each `LightFixture`: is this lamp currently lit? Mirrored onto its
/// `SpotLight` and lens by `sync_fixtures`.
#[derive(Component)]
pub struct Powered(pub bool);

/// On each `FuncLightSwitch`: is this circuit currently energised?
#[derive(Component)]
pub struct SwitchState(pub bool);

/// The lamp's glowing disc child (material swapped on/off).
#[derive(Component)]
pub struct LampLens;

/// The switch plate's indicator-light child (material swapped on/off).
#[derive(Component)]
pub struct SwitchIndicator;

/// The small real `PointLight` on the switch (colour/intensity swapped on/off).
#[derive(Component)]
pub struct SwitchLight;

/// Shared materials, built once at `Startup` (the `carry::CrateAssets`
/// pattern). Meshes are per-fixture — sized from `config::LAMP_*`.
#[derive(Resource)]
pub struct LampAssets {
    shade: Handle<StandardMaterial>,
    lens_off: Handle<StandardMaterial>,
    lens_on: Handle<StandardMaterial>,
    plate: Handle<StandardMaterial>,
    indicator_off: Handle<StandardMaterial>,
    indicator_on: Handle<StandardMaterial>,
}

pub fn setup_lamp_assets(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let emissive = |color: LinearRgba, base: Color| StandardMaterial {
        base_color: base,
        emissive: color,
        ..default()
    };
    commands.insert_resource(LampAssets {
        shade: materials.add(StandardMaterial {
            base_color: config::LAMP_SHADE_COLOR,
            perceptual_roughness: 0.7,
            ..default()
        }),
        lens_off: materials.add(StandardMaterial {
            base_color: config::LAMP_LENS_OFF_COLOR,
            perceptual_roughness: 0.3,
            ..default()
        }),
        lens_on: materials.add(emissive(config::LAMP_LENS_EMISSIVE, config::LAMP_COLOR)),
        plate: materials.add(StandardMaterial {
            base_color: config::SWITCH_PLATE_COLOR,
            emissive: config::SWITCH_PLATE_EMISSIVE,
            perceptual_roughness: 0.6,
            ..default()
        }),
        indicator_off: materials.add(emissive(config::SWITCH_OFF_EMISSIVE, Color::BLACK)),
        indicator_on: materials.add(emissive(config::SWITCH_ON_EMISSIVE, Color::BLACK)),
    });
}

/// `LightFixture` from the map → a lamp: a cone shade, a glowing lens disc that
/// caps its mouth, and a `SpotLight`, all facing `aim`. `aim "down"` (the
/// default) hangs it from a stem like a ceiling lamp; a compass `aim` bolts it
/// to a wall/pillar on an arm, raked 45° down toward that heading. The shade's
/// local `-Y` (its opening) and the `SpotLight`'s local `-Z` are both rotated
/// onto `aim` — no `angle` field on the class (the `FuncLadder::face_yaw`
/// trap). The connector's near end embeds 0.02 m into the mount surface so its
/// face isn't coplanar with the ceiling/deck/pillar (z-fight + visible gap).
pub fn spawn_fixtures(
    add: On<Add, LightFixture>,
    fixtures: Query<&LightFixture>,
    assets: Res<LampAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut commands: Commands,
) {
    let Ok(fixture) = fixtures.get(add.entity) else {
        return;
    };
    let lit = fixture.start_on.0 || devtools::force_lights_on();

    // bevy_trenchbroom maps TB `(x, y, z)` to Bevy `(-y, z, -x) / scale`, so a
    // TB compass heading rotates: north (+Y) → Bevy −X, south → +X, east (+X)
    // → Bevy −Z, west → +Z. A compass `aim` rakes 45° down toward that.
    let aim = match fixture.aim.as_str() {
        "north" => Vec3::new(-1.0, -1.0, 0.0),
        "south" => Vec3::new(1.0, -1.0, 0.0),
        "east" => Vec3::new(0.0, -1.0, -1.0),
        "west" => Vec3::new(0.0, -1.0, 1.0),
        _ => Vec3::NEG_Y, // "down" and anything unrecognised
    }
    .normalize();
    let raked = aim != Vec3::NEG_Y;

    let shade_h = config::LAMP_SHADE_HEIGHT;
    let shade_rot = Quat::from_rotation_arc(Vec3::NEG_Y, aim);
    let light_rot = Quat::from_rotation_arc(Vec3::NEG_Z, aim);

    // `head` is where the shade's mouth begins, relative to the fixture origin:
    // straight down a stem, or out along a bracket arm. The connector is 0.04 m
    // longer than the throw and shifted 0.02 m back so its near end is buried
    // in the mount surface.
    let embed = 0.02;
    let (connector_mesh, connector_at, head) = if raked {
        let arm = config::LAMP_BRACKET_ARM;
        let t = config::LAMP_STEM_RADIUS * 2.0;
        // Arm runs along the horizontal part of `aim` — always a cardinal, so
        // pick the cuboid's long axis directly rather than rotating it.
        let arm_dir = Vec3::new(aim.x, 0.0, aim.z).normalize();
        let mesh = if arm_dir.x.abs() > 0.5 {
            Cuboid::new(arm + 2.0 * embed, t, t)
        } else {
            Cuboid::new(t, t, arm + 2.0 * embed)
        };
        (meshes.add(mesh), arm_dir * (arm * 0.5 - embed), arm_dir * arm)
    } else {
        let len = config::LAMP_STEM_LENGTH;
        let mesh = Cylinder::new(config::LAMP_STEM_RADIUS, len + 2.0 * embed);
        (meshes.add(mesh), aim * (len * 0.5 - embed), aim * len)
    };

    let shade = meshes.add(ConicalFrustum {
        radius_top: config::LAMP_SHADE_TOP_RADIUS,
        radius_bottom: config::LAMP_SHADE_BOTTOM_RADIUS,
        height: shade_h,
    });
    // Caps the shade's mouth from just below its (solid) bottom cap.
    let lens = meshes.add(Cylinder::new(config::LAMP_SHADE_BOTTOM_RADIUS, 0.015));

    let outer = (fixture.cone_deg.to_radians() * 0.5).min(1.55);

    commands
        .entity(add.entity)
        .insert(Powered(lit))
        .with_children(|lamp| {
            lamp.spawn((
                Mesh3d(connector_mesh),
                MeshMaterial3d(assets.shade.clone()),
                Transform::from_translation(connector_at),
            ));
            lamp.spawn((
                Mesh3d(shade),
                MeshMaterial3d(assets.shade.clone()),
                Transform::from_translation(head + aim * (shade_h * 0.5))
                    .with_rotation(shade_rot),
            ));
            lamp.spawn((
                LampLens,
                Mesh3d(lens),
                MeshMaterial3d(if lit {
                    assets.lens_on.clone()
                } else {
                    assets.lens_off.clone()
                }),
                Transform::from_translation(head + aim * (shade_h + 0.015))
                    .with_rotation(shade_rot),
            ));
            lamp.spawn((
                SpotLight {
                    color: fixture.color,
                    intensity: if lit { fixture.intensity } else { 0.0 },
                    range: fixture.range,
                    outer_angle: outer,
                    inner_angle: outer * 0.6,
                    shadows_enabled: fixture.shadows.0,
                    ..default()
                },
                // Just past the lens so the shade cone never clips the light.
                Transform::from_translation(head + aim * (shade_h + 0.06))
                    .with_rotation(light_rot),
            ));
        });
}

/// `FuncLightSwitch` brush → a wall plate with a coloured indicator. Same
/// `On<SceneCollidersReady>` timing as `door::setup_doors` / `ladder`: the
/// `ColliderAabb` isn't populated until the physics backend has run.
pub fn setup_switches(
    ready: On<SceneCollidersReady>,
    switches: Query<(&FuncLightSwitch, &ColliderAabb), Without<SwitchState>>,
    assets: Res<LampAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut commands: Commands,
    mut interactables: Query<&mut Interactable>,
) {
    for &entity in &ready.collider_entities {
        let Ok((switch, aabb)) = switches.get(entity) else {
            continue;
        };
        let on = switch.start_on.0 || devtools::force_lights_on();
        let size = aabb.size();
        let center = aabb.center();

        // The plate faces the room along whichever horizontal axis the brush
        // is thinnest on (it's bolted flush to a wall). Its size is fixed
        // (`SWITCH_PLATE_SIZE`), not derived from the invisible brush.
        let face_x = size.x <= size.z;
        let (pw, ph) = (config::SWITCH_PLATE_SIZE.x, config::SWITCH_PLATE_SIZE.y);
        let plate = meshes.add(Cuboid::new(
            if face_x { 0.04 } else { pw },
            ph,
            if face_x { pw } else { 0.04 },
        ));
        let s = config::SWITCH_INDICATOR_SIZE;
        let indicator = meshes.add(Cuboid::new(
            if face_x { 0.02 } else { s },
            s,
            if face_x { s } else { 0.02 },
        ));
        let lever = meshes.add(Cuboid::new(
            if face_x { 0.06 } else { 0.05 },
            0.14,
            if face_x { 0.05 } else { 0.06 },
        ));
        // The `func_light_switch` brush entity's own `Transform` is identity
        // (no `angle`) — its geometry and `ColliderAabb` are world-space, like
        // the ladder. So children are positioned at the brush **centre** plus a
        // small push toward the room (`-center` on the facing axis, just proud
        // of the brush face). Anchoring at `plate_off` alone lands them on the
        // map origin.
        let out = -center.normalize_or_zero();
        let face_push = if face_x {
            Vec3::new(out.x.signum() * (size.x * 0.5 + 0.01), 0.0, 0.0)
        } else {
            Vec3::new(0.0, 0.0, out.z.signum() * (size.z * 0.5 + 0.01))
        };
        let plate_at = center + face_push;
        let bump = face_push.normalize_or_zero() * 0.02;

        if let Ok(mut i) = interactables.get_mut(entity) {
            i.prompt = if on {
                config::SWITCH_PROMPT_ON
            } else {
                config::SWITCH_PROMPT_OFF
            }
            .to_string();
        }
        let (light_color, light_intensity) = if on {
            (config::SWITCH_LIGHT_ON_COLOR, config::SWITCH_LIGHT_ON_INTENSITY)
        } else {
            (config::SWITCH_LIGHT_OFF_COLOR, config::SWITCH_LIGHT_OFF_INTENSITY)
        };

        commands
            .entity(entity)
            .insert(SwitchState(on))
            .with_children(|sw| {
                sw.spawn((
                    Mesh3d(plate),
                    MeshMaterial3d(assets.plate.clone()),
                    Transform::from_translation(plate_at),
                ));
                sw.spawn((
                    Mesh3d(lever),
                    MeshMaterial3d(assets.plate.clone()),
                    Transform::from_translation(plate_at + Vec3::Y * -0.08 + bump * 1.5),
                ));
                sw.spawn((
                    SwitchIndicator,
                    Mesh3d(indicator),
                    MeshMaterial3d(if on {
                        assets.indicator_on.clone()
                    } else {
                        assets.indicator_off.clone()
                    }),
                    Transform::from_translation(plate_at + Vec3::Y * 0.15 + bump),
                ));
                sw.spawn((
                    SwitchLight,
                    PointLight {
                        color: light_color,
                        intensity: light_intensity,
                        range: config::SWITCH_LIGHT_RANGE,
                        shadows_enabled: false,
                        ..default()
                    },
                    Transform::from_translation(plate_at + bump * 3.0),
                ));
            });
    }
}

/// E on a switch: flip it, push the new state to every `LightFixture` whose
/// `targetname` matches this switch's `target`, and swap the prompt.
pub fn toggle_on_interact(
    trigger: On<Interacted>,
    mut switches: Query<(&mut SwitchState, &mut Interactable, &Target)>,
    mut fixtures: Query<(&mut Powered, &Targetable)>,
) {
    let Ok((mut state, mut interactable, target)) = switches.get_mut(trigger.entity) else {
        return;
    };
    state.0 = !state.0;
    interactable.prompt = if state.0 {
        config::SWITCH_PROMPT_ON
    } else {
        config::SWITCH_PROMPT_OFF
    }
    .to_string();

    for (mut powered, targetable) in &mut fixtures {
        if targetable.targetname.0 == target.target.0 {
            powered.0 = state.0;
        }
    }
}

pub fn sync_fixtures(
    fixtures: Query<(&Powered, &LightFixture, &Children), Changed<Powered>>,
    assets: Res<LampAssets>,
    mut spots: Query<&mut SpotLight>,
    mut lenses: Query<&mut MeshMaterial3d<StandardMaterial>, With<LampLens>>,
) {
    for (powered, fixture, children) in &fixtures {
        for &child in children {
            if let Ok(mut spot) = spots.get_mut(child) {
                spot.intensity = if powered.0 { fixture.intensity } else { 0.0 };
            }
            if let Ok(mut mat) = lenses.get_mut(child) {
                mat.0 = if powered.0 {
                    assets.lens_on.clone()
                } else {
                    assets.lens_off.clone()
                };
            }
        }
    }
}

pub fn sync_switch_indicators(
    switches: Query<(&SwitchState, &Children), Changed<SwitchState>>,
    assets: Res<LampAssets>,
    mut indicators: Query<&mut MeshMaterial3d<StandardMaterial>, With<SwitchIndicator>>,
    mut lights: Query<&mut PointLight, With<SwitchLight>>,
) {
    for (state, children) in &switches {
        for &child in children {
            if let Ok(mut mat) = indicators.get_mut(child) {
                mat.0 = if state.0 {
                    assets.indicator_on.clone()
                } else {
                    assets.indicator_off.clone()
                };
            }
            if let Ok(mut light) = lights.get_mut(child) {
                (light.color, light.intensity) = if state.0 {
                    (config::SWITCH_LIGHT_ON_COLOR, config::SWITCH_LIGHT_ON_INTENSITY)
                } else {
                    (config::SWITCH_LIGHT_OFF_COLOR, config::SWITCH_LIGHT_OFF_INTENSITY)
                };
            }
        }
    }
}

/// One switch on anywhere lifts the global fill from a near-black void to a
/// dim "lit warehouse" bounce. Keyed on switches, not fixtures — see the
/// module docs.
pub fn sync_ambient(switches: Query<&SwitchState>, mut ambient: ResMut<GlobalAmbientLight>) {
    let target = if switches.iter().any(|s| s.0) {
        config::AMBIENT_LIT
    } else {
        config::AMBIENT_DARK
    };
    if ambient.brightness != target {
        ambient.brightness = target;
    }
}
