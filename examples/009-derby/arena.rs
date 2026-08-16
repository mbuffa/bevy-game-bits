//! The circular derby arena: a sculpted floor disc (flat except for the
//! pothole bowls dented into it), a ring of flat wall panels approximating a
//! cylinder (its "faces", as the user put it), a concrete pillar at the
//! center, and four launch ramps in a pinwheel around it. The floor is one
//! trimesh collider built from the same mesh that is rendered, so the
//! raycast suspension feels every dip the eye sees.

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use avian3d::prelude::*;
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

use crate::config::*;

/// Pothole centers (x, z). Mid-field, clear of the pillar, the ramp ring's
/// four footprints (on the axes at radius 25), the obstacle layouts, and
/// the spawn ring at radius 52. The last two sit in the outer band, clear
/// of both the ramps and the four diagonal spawn slots.
const POTHOLES: [(f32, f32); 8] = [
    (-15.6, 7.8),
    (11.7, 15.6),
    (18.2, -10.4),
    (-10.4, -18.2),
    (-33.8, -23.4),
    (31.2, -31.2),
    (36.0, -14.0),
    (-38.0, 15.0),
];

pub fn setup_arena(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let floor_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.35, 0.35, 0.38),
        perceptual_roughness: 0.95,
        ..default()
    });
    let floor_mesh = build_floor_mesh();
    let floor_collider =
        Collider::trimesh_from_mesh(&floor_mesh).expect("floor mesh should yield a trimesh");
    commands.spawn((
        Name::new("Floor"),
        RigidBody::Static,
        floor_collider,
        Friction::new(FLOOR_FRICTION),
        Restitution::new(FLOOR_RESTITUTION),
        Mesh3d(meshes.add(floor_mesh)),
        MeshMaterial3d(floor_material),
        Transform::IDENTITY,
    ));

    let wall_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.65, 0.15, 0.15),
        perceptual_roughness: 0.7,
        ..default()
    });
    // Chord width of one segment, padded by WALL_OVERLAP so neighboring
    // panels overlap slightly instead of leaving a seam gap the car could
    // clip through.
    let segment_angle = TAU / WALL_SEGMENTS as f32;
    let chord_width = 2.0 * ARENA_RADIUS * (segment_angle / 2.0).sin() * WALL_OVERLAP;
    let wall_mesh = meshes.add(Cuboid::new(chord_width, WALL_HEIGHT, WALL_THICKNESS));
    let wall_collider = Collider::cuboid(chord_width, WALL_HEIGHT, WALL_THICKNESS);

    for i in 0..WALL_SEGMENTS {
        let angle = i as f32 * segment_angle;
        let (sin, cos) = angle.sin_cos();
        // Rotating +Z by `angle` around Y gives (sin, 0, cos) — the same
        // radial direction the panel is positioned along, so the cuboid's
        // local Z (its thickness axis) ends up pointing radially and its
        // local X (its width axis) ends up tangential to the ring.
        let position = Vec3::new(ARENA_RADIUS * sin, WALL_HEIGHT / 2.0, ARENA_RADIUS * cos);
        commands.spawn((
            Name::new("WallPanel"),
            RigidBody::Static,
            wall_collider.clone(),
            Friction::new(FLOOR_FRICTION),
            Restitution::new(WALL_RESTITUTION),
            Mesh3d(wall_mesh.clone()),
            MeshMaterial3d(wall_material.clone()),
            Transform::from_translation(position).with_rotation(Quat::from_rotation_y(angle)),
        ));
    }

    // Center pillar: the arena's anchor hazard. Head-on hits register
    // through the same contact readback as walls.
    let pillar_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.5, 0.5, 0.52),
        perceptual_roughness: 0.9,
        ..default()
    });
    commands.spawn((
        Name::new("Pillar"),
        RigidBody::Static,
        Collider::cylinder(PILLAR_RADIUS, PILLAR_HEIGHT),
        Friction::new(FLOOR_FRICTION),
        Restitution::new(WALL_RESTITUTION),
        Mesh3d(meshes.add(Cylinder::new(PILLAR_RADIUS, PILLAR_HEIGHT))),
        MeshMaterial3d(pillar_material),
        Transform::from_xyz(0.0, PILLAR_HEIGHT / 2.0, 0.0),
    ));

    // Four ramps in a pinwheel: each sits on the ramp ring facing
    // tangentially (same handedness), so a car hitting one at speed is
    // thrown on a flying arc around the pillar. The tall back face doubles
    // as a barrier when approached from the wrong side — like a real stunt
    // ramp. The wedge mesh is an extruded triangle whose slope runs along
    // local +X (lip at −X, crest at +X), extruded across local Z.
    let ramp_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.75, 0.55, 0.15),
        perceptual_roughness: 0.8,
        ..default()
    });
    let half_length = RAMP_LENGTH / 2.0;
    let half_width = RAMP_WIDTH / 2.0;
    let ramp_profile = Triangle2d::new(
        Vec2::new(-half_length, 0.0),
        Vec2::new(half_length, 0.0),
        Vec2::new(half_length, RAMP_HEIGHT),
    );
    let ramp_mesh = meshes.add(Extrusion::new(ramp_profile, RAMP_WIDTH));
    let ramp_collider = Collider::convex_hull(vec![
        Vec3::new(-half_length, 0.0, -half_width),
        Vec3::new(-half_length, 0.0, half_width),
        Vec3::new(half_length, 0.0, -half_width),
        Vec3::new(half_length, 0.0, half_width),
        Vec3::new(half_length, RAMP_HEIGHT, -half_width),
        Vec3::new(half_length, RAMP_HEIGHT, half_width),
    ])
    .expect("ramp wedge is a valid convex hull");

    for i in 0..4 {
        let angle = i as f32 * FRAC_PI_2;
        let (sin, cos) = angle.sin_cos();
        let position = Vec3::new(RAMP_RING_RADIUS * sin, 0.0, RAMP_RING_RADIUS * cos);
        // `from_rotation_y(angle)` maps local +X (the drive-up direction)
        // to the ring tangent (cos, 0, −sin) at this spot.
        commands.spawn((
            Name::new("Ramp"),
            RigidBody::Static,
            ramp_collider.clone(),
            Friction::new(FLOOR_FRICTION),
            Restitution::new(FLOOR_RESTITUTION),
            Mesh3d(ramp_mesh.clone()),
            MeshMaterial3d(ramp_material.clone()),
            Transform::from_translation(position).with_rotation(Quat::from_rotation_y(angle)),
        ));
    }
}

/// Builds the floor disc as a polar grid: a center fan plus concentric
/// rings of quads, flat at y = 0 except inside the pothole circles, where
/// vertices dip by a cosine bowl (full `POTHOLE_DEPTH` at the center,
/// blending smoothly to zero at the rim). The same mesh feeds the trimesh
/// collider, so physics and visuals can't disagree.
fn build_floor_mesh() -> Mesh {
    let rings = (ARENA_RADIUS / FLOOR_MESH_RING_STEP).ceil() as usize;
    let sectors = FLOOR_MESH_SECTORS;

    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(1 + rings * sectors);
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(positions.capacity());
    let mut colors: Vec<[f32; 4]> = Vec::with_capacity(positions.capacity());
    let mut vertex = |x: f32, z: f32| {
        let y = floor_height(x, z);
        positions.push([x, y, z]);
        uvs.push([
            (x / ARENA_RADIUS + 1.0) / 2.0,
            (z / ARENA_RADIUS + 1.0) / 2.0,
        ]);
        // Darken pothole interiors in proportion to their depth so the dips
        // read on the uniform floor material (smooth-shaded bowls are
        // nearly invisible from a shallow camera angle otherwise).
        let shade = 1.0 - 0.65 * (-y / POTHOLE_DEPTH).clamp(0.0, 1.0);
        colors.push([shade, shade, shade, 1.0]);
    };

    vertex(0.0, 0.0);
    for ring in 1..=rings {
        let radius = ring as f32 / rings as f32 * ARENA_RADIUS;
        for sector in 0..sectors {
            let theta = sector as f32 / sectors as f32 * TAU;
            vertex(radius * theta.sin(), radius * theta.cos());
        }
    }
    // Vertex index for (ring ≥ 1, sector) with wrap-around.
    let at = |ring: usize, sector: usize| (1 + (ring - 1) * sectors + sector % sectors) as u32;

    let mut indices: Vec<u32> = Vec::with_capacity(3 * sectors * (2 * rings - 1));
    for sector in 0..sectors {
        // Center fan; winding chosen so face normals point +Y.
        indices.extend([0, at(1, sector), at(1, sector + 1)]);
    }
    for ring in 1..rings {
        for sector in 0..sectors {
            let (a, b) = (at(ring, sector), at(ring + 1, sector));
            let (c, d) = (at(ring + 1, sector + 1), at(ring, sector + 1));
            indices.extend([a, b, c, a, c, d]);
        }
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
    .with_inserted_indices(Indices::U32(indices));
    mesh.compute_smooth_normals();
    mesh
}

/// Floor height at (x, z): 0 outside every pothole, dipping inside by a
/// cosine bowl. Bowls are far enough apart that at most one applies, but
/// summing keeps overlaps well-defined anyway.
fn floor_height(x: f32, z: f32) -> f32 {
    let mut y = 0.0;
    for &(px, pz) in POTHOLES.iter() {
        let distance = (Vec2::new(x, z) - Vec2::new(px, pz)).length();
        if distance < POTHOLE_RADIUS {
            y -= POTHOLE_DEPTH * 0.5 * (1.0 + (PI * distance / POTHOLE_RADIUS).cos());
        }
    }
    y
}
