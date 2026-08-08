// Wind sway for wheat (`weather::WindSwayMaterial`): vertex-only override of
// the standard PBR pipeline. Vertices are pushed along the wind, weighted by
// height squared, so stalk bases stay pinned to the ground while the tips
// bend — the standard vegetation trick. No fragment fn here: the
// StandardMaterial PBR fragment runs unchanged, so lighting is untouched
// (normals are not re-bent either; invisible at this stalk size).

#import bevy_pbr::{
    mesh_functions,
    forward_io::{Vertex, VertexOutput},
    view_transformations::position_world_to_clip,
}

// xy = normalized wind direction (XZ plane), z = current strength,
// w = sim time in seconds (frozen while the game is paused, unlike
// globals.time — that's why time comes in through the uniform).
@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> wind: vec4<f32>;

const TAU: f32 = 6.28318530718;
// Shader-side tuning; the CPU-side wind constants live in config.rs.
// Tip offset in world units per strength unit.
const SWAY_PER_STRENGTH: f32 = 0.03;
// World units between ripple crests traveling across the field.
const WAVE_LENGTH: f32 = 6.0;
// How fast the ripple travels, scaled further by wind strength.
const WAVE_SPEED: f32 = 1.4;
// Keep in sync with config::CROP_HEIGHT (a fully grown stalk's world height).
const CROP_HEIGHT: f32 = 0.5;

// Cheap 2D hash -> phase in [0, TAU): keeps neighboring tiles out of step.
fn phase_hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2(12.9898, 78.233))) * 43758.5453) * TAU;
}

@vertex
fn vertex(vertex: Vertex) -> VertexOutput {
    var out: VertexOutput;

    let world_from_local = mesh_functions::get_world_from_local(vertex.instance_index);
    var world_position = mesh_functions::mesh_position_local_to_world(
        world_from_local,
        vec4<f32>(vertex.position, 1.0),
    );

    // Bend weight from WORLD height: growth squashes local Y through the
    // transform scale, so a young crop's tip sits lower and sways
    // proportionally less, and the y=0 base never moves.
    let scale_y = length(world_from_local[1].xyz);
    let height = vertex.position.y * scale_y;
    let weight = pow(clamp(height / CROP_HEIGHT, 0.0, 1.0), 2.0);

    // A wave traveling along the wind direction plus a per-tile phase, so
    // the field ripples instead of rocking in lockstep.
    let tile = world_from_local[3].xz;
    let travel = dot(world_position.xz, wind.xy) * (TAU / WAVE_LENGTH);
    let ripple = 0.65 + 0.35 * sin(wind.w * WAVE_SPEED * wind.z - travel + phase_hash(tile));

    let offset = wind.xy * (wind.z * SWAY_PER_STRENGTH) * weight * ripple;
    world_position.x += offset.x;
    world_position.z += offset.y;

    out.world_position = world_position;
    out.position = position_world_to_clip(world_position.xyz);
#ifdef VERTEX_NORMALS
    out.world_normal = mesh_functions::mesh_normal_local_to_world(
        vertex.normal,
        vertex.instance_index,
    );
#endif
#ifdef VERTEX_UVS_A
    out.uv = vertex.uv;
#endif
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    out.instance_index = vertex.instance_index;
#endif
#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(
        vertex.instance_index, world_from_local[3]);
#endif
    return out;
}
