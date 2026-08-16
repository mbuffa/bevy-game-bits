//! Skid-mark decals: dark quads stamped just above the ground wherever a
//! wheel's contact patch slips faster than the threshold — drifting,
//! handbrake scrubs and hard launches paint the map.
//!
//! Purely a rendering system. It reads the [`slip_speed`](Wheel::slip_speed)
//! and [`contact_world`](Wheel::contact_world) hooks the controller writes on
//! every [`Wheel`] and never touches forces, so adding or removing it can't
//! change how anything drives.
//!
//! Marks are segment quads connecting successive contact points — continuous
//! strips, not dots — and persist as battle scars, bounded by a ring buffer:
//! once [`SkidMarkConfig::max_marks`] exist, the oldest quad is repositioned
//! instead of a new one being spawned.

use std::collections::HashMap;

use bevy::prelude::*;

use super::wheel::Wheel;
use super::VehicleSet;

/// How skid marks look and when they appear.
#[derive(Resource, Clone, Debug)]
pub struct SkidMarkConfig {
    /// Contact-patch slip speed (m/s) above which a wheel paints rubber.
    /// Grip-limit cornering peaks around 0.3–1.0, while drifting and
    /// handbrake locks sail far past it — so lowering this makes ordinary
    /// hard cornering leave marks too.
    pub slip_threshold: f32,
    /// Ring-buffer cap on stamped quads. Beyond this the oldest are recycled,
    /// so scars accumulate but memory doesn't.
    pub max_marks: usize,
    /// Mark width (m) — a hair narrower than the tire reads best.
    pub width: f32,
    /// Minimum travel between stamps. Shorter wastes quads on dense dots.
    pub min_segment: f32,
    /// A gap larger than this (a teleport, a respawn, an airborne hop)
    /// restarts the strip instead of stamping one long spear across the map.
    pub max_segment: f32,
    /// Lift above the contact point so marks don't z-fight the ground.
    pub y_offset: f32,
    /// Mark color. Alpha below 1 blends; the material is unlit, so marks read
    /// as flat stains regardless of sun angle.
    pub color: Color,
}

impl Default for SkidMarkConfig {
    fn default() -> Self {
        Self {
            slip_threshold: 1.5,
            max_marks: 3000,
            width: 0.25,
            min_segment: 0.15,
            max_segment: 1.5,
            y_offset: 0.02,
            color: Color::srgba(0.05, 0.05, 0.05, 0.55),
        }
    }
}

/// The mark pool and per-wheel strip state. Inserted by [`SkidMarkPlugin`].
#[derive(Resource)]
pub struct SkidMarks {
    /// Ring buffer of stamped quad entities, oldest-first once full.
    marks: Vec<Entity>,
    /// Index of the next entity to recycle when the buffer is full.
    next: usize,
    /// Per-wheel end point of its current strip, removed whenever the wheel
    /// stops skidding so strips restart cleanly.
    last_point: HashMap<Entity, Vec3>,
    mesh: Handle<Mesh>,
    material: Handle<StandardMaterial>,
}

/// Adds skid marks. Independent of the rest of the module beyond needing
/// [`Wheel`]s to read, so it can be dropped in or left out freely.
#[derive(Default)]
pub struct SkidMarkPlugin {
    pub config: SkidMarkConfig,
}

impl Plugin for SkidMarkPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(self.config.clone())
            .add_systems(Startup, setup_skid_marks)
            .add_systems(Update, emit_skid_marks.in_set(VehicleSet::Visuals));
    }
}

/// Builds the shared quad mesh and material. Runs at `Startup`.
pub fn setup_skid_marks(
    mut commands: Commands,
    config: Res<SkidMarkConfig>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // Unit XZ plane facing +Y; each mark's Transform scales it into a segment.
    let mesh = meshes.add(Plane3d::default().mesh().size(1.0, 1.0));
    let material = materials.add(StandardMaterial {
        base_color: config.color,
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        ..default()
    });
    commands.insert_resource(SkidMarks {
        marks: Vec::new(),
        next: 0,
        last_point: HashMap::new(),
        mesh,
        material,
    });
}

/// Stamps a quad for every wheel that travelled far enough while slipping.
pub fn emit_skid_marks(
    mut commands: Commands,
    config: Res<SkidMarkConfig>,
    mut skid: ResMut<SkidMarks>,
    wheels: Query<(Entity, &Wheel)>,
    mut transforms: Query<&mut Transform>,
) {
    for (wheel_entity, wheel) in &wheels {
        let contact = match wheel.contact_world {
            Some(contact) if wheel.slip_speed >= config.slip_threshold => contact,
            // Gripping or airborne: end this wheel's strip.
            _ => {
                skid.last_point.remove(&wheel_entity);
                continue;
            }
        };

        let Some(&last) = skid.last_point.get(&wheel_entity) else {
            // First skidding frame: anchor the strip, stamp from the next
            // point on.
            skid.last_point.insert(wheel_entity, contact);
            continue;
        };

        let delta = contact - last;
        let planar = Vec3::new(delta.x, 0.0, delta.z);
        let length = planar.length();
        if length < config.min_segment {
            // Not enough travel yet — keep accumulating toward `last`.
            continue;
        }
        if length > config.max_segment {
            skid.last_point.insert(wheel_entity, contact);
            continue;
        }

        let index = if skid.marks.len() < config.max_marks {
            skid.marks.len()
        } else {
            skid.next
        };
        let midpoint = (last + contact) / 2.0;
        let transform = Transform {
            // Slightly above the local surface — the ground may have dips and
            // ramps, so the contact's own height matters — and staggered per
            // slot so overlapping marks don't z-fight each other.
            translation: Vec3::new(
                midpoint.x,
                midpoint.y + config.y_offset + (index % 16) as f32 * 0.0005,
                midpoint.z,
            ),
            // Align local +Z with the segment direction: yaw along the planar
            // direction, then pitch to follow the slope, so marks on ramps
            // lie on the surface instead of knifing through it.
            rotation: Quat::from_rotation_y(planar.x.atan2(planar.z))
                * Quat::from_rotation_x(-delta.y.atan2(length)),
            scale: Vec3::new(config.width, 1.0, delta.length()),
        };

        if skid.marks.len() < config.max_marks {
            let mark = commands
                .spawn((
                    Name::new("SkidMark"),
                    Mesh3d(skid.mesh.clone()),
                    MeshMaterial3d(skid.material.clone()),
                    transform,
                ))
                .id();
            skid.marks.push(mark);
        } else {
            if let Ok(mut mark_transform) = transforms.get_mut(skid.marks[skid.next]) {
                *mark_transform = transform;
            }
            skid.next = (skid.next + 1) % config.max_marks;
        }

        skid.last_point.insert(wheel_entity, contact);
    }
}
