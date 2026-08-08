// Water ripples (`map::WaterMaterial`): fragment-only override of the
// standard PBR pipeline. The surface quads stay flat; this shader perturbs
// the lighting normal with a few sine waves traveling along the river's
// flow direction, so sun/moon glints shimmer and drift as if the surface
// had small waves. Everything is computed from WORLD position, so the
// pattern runs seamlessly across the per-cell quads — no mesh merging
// needed. Base color, translucency and the rest of StandardMaterial pass
// through untouched.
//
// Known nit: animated normals under TAA can shimmer slightly (same class
// of artifact as the documented hanabi ghosting); the noise-like layering
// below blends into the temporal filter rather than smearing.

#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::alpha_discard,
}

#ifdef PREPASS_PIPELINE
#import bevy_pbr::{
    prepass_io::{VertexOutput, FragmentOutput},
    pbr_deferred_functions::deferred_output,
}
#else
#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
}
#endif

// xy = flow direction (world XZ, normalized — WATER_FLOW_DIRECTION),
// z = sim time in seconds (frozen while the game is paused, unlike
// globals.time — that's why time comes in through the uniform),
// w = live wind strength (weather::Wind.current, ~1.5 calm to 6 heavy).
@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> params: vec4<f32>;
// x = freeze level 0..1 (snow::FreezeLevel): 0 open water, 1 solid ice.
// Flattens the waves, whitens the color toward ICE_TINT and walks the
// alpha/roughness toward an opaque matte sheet. yzw spare.
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var<uniform> ice: vec4<f32>;

const TAU: f32 = 6.28318530718;
// Shader-side tuning; the CPU-side water constants live in config.rs.
// Three wave trains with incommensurate wavelengths/speeds (the
// gust_multiplier trick) so the pattern never visibly loops. Wavelengths
// in world units (a tile is 1.0), speeds in world units per second.
const WAVE_LENGTHS: vec3<f32> = vec3<f32>(2.3, 4.1, 1.3);
const WAVE_SPEEDS: vec3<f32> = vec3<f32>(0.8, 0.5, 1.2);
// Per-train slope contribution (unitless surface gradient at chop = 1).
const WAVE_STEEPNESS: vec3<f32> = vec3<f32>(0.7, 0.45, 0.35);
// Chop (overall slope multiplier) = base + per-wind * wind strength: calm
// water is near-glassy, the Heavy Wind devtool visibly roughens it.
const CHOP_BASE: f32 = 0.06;
const CHOP_PER_WIND: f32 = 0.05;
const CHOP_MAX: f32 = 0.4;
// Fully frozen surface look: a pale blue-white sheet, nearly opaque and
// matte (ICE_ROUGHNESS overrides the glossy WATER_ROUGHNESS as it firms).
const ICE_TINT: vec3<f32> = vec3<f32>(0.85, 0.91, 0.96);
const ICE_ALPHA: f32 = 0.95;
const ICE_ROUGHNESS: f32 = 0.7;

// Surface gradient (dh/dx, dh/dz) of the three traveling waves at world
// point `p`: the primary runs straight down the flow, the other two angle
// off either side so crests aren't parallel bars.
fn wave_gradient(p: vec2<f32>, t: f32) -> vec2<f32> {
    let flow = params.xy;
    let perp = vec2<f32>(-flow.y, flow.x);
    let d1 = flow;
    let d2 = normalize(flow + 0.6 * perp);
    let d3 = normalize(flow - 0.8 * perp);

    var gradient = vec2<f32>(0.0);
    let k = TAU / WAVE_LENGTHS;
    gradient += WAVE_STEEPNESS.x * cos(k.x * (dot(p, d1) - WAVE_SPEEDS.x * t)) * d1;
    gradient += WAVE_STEEPNESS.y * cos(k.y * (dot(p, d2) - WAVE_SPEEDS.y * t)) * d2;
    gradient += WAVE_STEEPNESS.z * cos(k.z * (dot(p, d3) - WAVE_SPEEDS.z * t)) * d3;
    return gradient;
}

@fragment
fn fragment(
    in: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);

    // Tilt the lighting normal by the wave slope. The quad itself is flat
    // and horizontal, so the perturbed normal is built directly in world
    // space (up plus the negated gradient — the normal of a heightfield).
    // Freezing stills the water: the wave slope, the tint and the
    // translucency all lerp toward a matte ice sheet with the freeze level.
    let freeze = clamp(ice.x, 0.0, 1.0);
    let chop = min(CHOP_BASE + CHOP_PER_WIND * params.w, CHOP_MAX) * (1.0 - freeze);
    let gradient = chop * wave_gradient(in.world_position.xz, params.z);
    pbr_input.N = normalize(vec3<f32>(-gradient.x, 1.0, -gradient.y));

    // The base color/alpha carries the per-cell depth tint and shore alpha
    // fade baked into the surface mesh's vertex colors (see
    // `map::water_vertex_color`); at partial freeze that per-cell variation
    // would otherwise show through the ice as a grid or, worse, a shallow-
    // vs-deep patchwork (shallow shore cells start far more transparent, so
    // mixing color and alpha on the same curve makes them flash to a stark
    // opaque white well before the deep interior catches up). Alpha
    // homogenizes fast — opacity reads as "one solid sheet" almost as soon
    // as freezing starts — while color whitens on a slower curve, so young
    // ice keeps a translucent blue cast instead of instantly bleaching, and
    // only commits to the flat ICE_TINT sheet near full freeze. Both start
    // past a dead zone (not at freeze = 0): the shallow shore fade already
    // sits at a very low base alpha, so even a whisper of residual freeze
    // (a Winter->Spring thaw in progress, not yet fully back to 0) would
    // otherwise nudge alpha up and read as the shallow diagonal going
    // noticeably darker/more opaque with no accompanying whitening to
    // explain it.
    let alpha_mix = smoothstep(0.1, 0.4, freeze);
    let color_mix = smoothstep(0.15, 0.9, freeze);
    let color = pbr_input.material.base_color;
    pbr_input.material.base_color = vec4<f32>(
        mix(color.rgb, ICE_TINT, color_mix),
        mix(color.a, ICE_ALPHA, alpha_mix),
    );
    pbr_input.material.perceptual_roughness =
        mix(pbr_input.material.perceptual_roughness, ICE_ROUGHNESS, color_mix);

    pbr_input.material.base_color =
        alpha_discard(pbr_input.material, pbr_input.material.base_color);

#ifdef PREPASS_PIPELINE
    // Alpha-blended water never renders in the prepass; required boilerplate.
    let out = deferred_output(in, pbr_input);
#else
    var out: FragmentOutput;
    out.color = apply_pbr_lighting(pbr_input);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
#endif
    return out;
}
