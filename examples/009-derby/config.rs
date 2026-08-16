//! Tunable constants for the derby prototype. Kept in one place so the
//! feel (suspension stiffness, grip, camera lag, ...) can be iterated on
//! without hunting through the systems that use them.
//!
//! Everything that makes one car *class* handle, look, or break differently
//! from another lives in `CarSpec` (see "Car classes" below) instead of as a
//! bare constant — mass, engine force, tire grip, geometry, the lot. Values
//! that are shared *feel* rather than a class trait (damage thresholds, part
//! weights, AI gains) stay plain constants below, read by both classes alike.
//!
//! What's *not* here any more: the driving model's own constants. Suspension
//! frequency, tire slip clamps, handbrake scales, the drag exponent, steer
//! smoothing, flip rescue, skid thresholds and the impulse-to-Δv pipeline all
//! moved to `bevy_game_bits::vehicle`'s `VehicleTuning` / `ImpactTuning` /
//! `SkidMarkConfig` / `DebrisConfig`, whose defaults *are* the numbers this
//! file used to hold. `main.rs` adds the plugins with those defaults; anything
//! here that needs one reads it back off the resource.

use std::ops::Deref;

use bevy::prelude::*;
use bevy_game_bits::vehicle::VehicleSpec;

// --- Arena ---------------------------------------------------------------

/// Radius of the circular floor and the wall ring, in meters. A full-width
/// run reaches top speed in well under 40 m even for the Buggy (see
/// `CarSpec::top_speed`/`DRAG_EXPONENT`), so this leaves plenty of room to
/// spare beyond that.
pub const ARENA_RADIUS: f32 = 65.0;
/// How many flat panels make up the wall ring — the "cylinder faces" look.
/// Scaled with the radius to keep the chord width (panel size) constant.
pub const WALL_SEGMENTS: usize = 52;
/// Wall panel height off the floor.
pub const WALL_HEIGHT: f32 = 3.0;
/// Wall panel thickness (along the radial direction).
pub const WALL_THICKNESS: f32 = 1.0;
/// How far each panel overlaps its neighbor's chord width, so faceted walls
/// don't leave a gap at the seams a car could clip through.
pub const WALL_OVERLAP: f32 = 1.1;
/// Wall bounce — the derby feel. 0 = no bounce, 1 = perfectly elastic.
pub const WALL_RESTITUTION: f32 = 0.25;
/// Floor friction; low restitution so the car doesn't bobble on landing.
pub const FLOOR_FRICTION: f32 = 0.9;
pub const FLOOR_RESTITUTION: f32 = 0.05;

// --- Arena features ----------------------------------------------------------

/// Distance between floor mesh rings (m). Also roughly the radial vertex
/// spacing potholes are sculpted from — smaller = smoother bowls, more tris.
pub const FLOOR_MESH_RING_STEP: f32 = 0.5;
/// Sectors around the floor disc. Sized so the tangential vertex spacing at
/// the pothole radii (~2π·33/320 ≈ 0.65 m) resolves a 2 m pothole bowl.
pub const FLOOR_MESH_SECTORS: usize = 320;
/// Pothole bowl radius (m) and center depth (m). The bowl is a cosine dip —
/// rim-smooth, so tires roll in and out instead of hitting a lip.
pub const POTHOLE_RADIUS: f32 = 2.0;
pub const POTHOLE_DEPTH: f32 = 0.15;
/// Center pillar: a static concrete cylinder at the arena origin.
pub const PILLAR_RADIUS: f32 = 3.0;
pub const PILLAR_HEIGHT: f32 = 6.0;
/// Ramp wedge: rises `RAMP_HEIGHT` over `RAMP_LENGTH` (~11° — launch at
/// 20 m/s gives around 0.8 s of air), `RAMP_WIDTH` across.
pub const RAMP_LENGTH: f32 = 6.0;
pub const RAMP_WIDTH: f32 = 4.0;
pub const RAMP_HEIGHT: f32 = 1.2;
/// Radius of the pinwheel ring the four ramps sit on.
pub const RAMP_RING_RADIUS: f32 = 25.0;
/// Radius of the starting grid: all four cars spawn on this ring, evenly
/// spaced, facing the center — the classic derby start.
pub const SPAWN_RING_RADIUS: f32 = 52.0;

// --- Obstacles --------------------------------------------------------------

/// Light wooden crate: flies satisfyingly when rammed at speed.
pub const CRATE_SIZE: f32 = 1.2;
pub const CRATE_MASS: f32 = 50.0;
/// Heavy concrete block: a real hazard — at 500 kg it barely budges.
pub const BLOCK_SIZE: f32 = 2.5;
pub const BLOCK_MASS: f32 = 500.0;
/// Balls roll away when clipped. Mass stays at 40 kg deliberately: it is
/// what lets a *punted* ball still hurt (see `PROP_SOFTNESS_*` below) —
/// making the ball light enough to be harmless at rest would make it
/// harmless always, since impulse physics can't otherwise tell "parked"
/// from "punted" apart.
pub const BALL_RADIUS: f32 = 0.8;
pub const BALL_MASS: f32 = 40.0;
pub const OBSTACLE_FRICTION: f32 = 0.6;
/// Bleeds off tumbling so scattered props settle instead of spinning
/// forever.
pub const OBSTACLE_ANGULAR_DAMPING: f32 = 0.3;
pub const BOX_RESTITUTION: f32 = 0.1;
/// Near-dead bounce (was 0.3): combined with the chassis's 0.1 under
/// `CoefficientCombine::Min`, the pair uses 0.05 — no more springing off
/// the nose when shoved.
pub const BALL_RESTITUTION: f32 = 0.05;
/// Slicker than the other props (`OBSTACLE_FRICTION` 0.6): a ball that
/// grips the nose shoves the car sideways, a slick one squirts off it.
pub const BALL_FRICTION: f32 = 0.35;
/// A punted ball coasts to a stop in a few seconds instead of crossing the
/// arena, so it stays an obstacle rather than leaving the field.
pub const BALL_LINEAR_DAMPING: f32 = 0.25;
/// Lower than the box props' `OBSTACLE_ANGULAR_DAMPING` so a rolling ball
/// keeps visibly rolling.
pub const BALL_ANGULAR_DAMPING: f32 = 0.15;

// Prop impact softness (a parked ball is furniture, a punted one is a
// projectile) is `vehicle::ImpactTuning::prop_softness_min` /
// `prop_softness_full_speed`; the balls are tagged `vehicle::SoftProp` in
// `obstacles.rs`.

// --- Car classes -------------------------------------------------------------
//
// Every number that makes one class of car *drive*, *look*, or *break*
// differently from another lives here. `vehicle::spawn_car` inserts the
// `CarClass` component on the chassis; every system that used to read one of
// these as a bare global now reads `class.spec()` (or the `CarClass`
// component directly) off the car it's processing instead.
//
// Wheelbase and track width are deliberately *not* fields here: they're
// derived once at spawn from the loaded model's own wheel-node positions
// (`model::CarRig::wheelbase`/`track_width`) and cached on `Vehicle`. Tabling
// them here too would give the steering model and the artist-edited `.glb` two
// sources of truth that could silently disagree; deriving from the rig means
// there's exactly one.

#[derive(Component, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum CarClass {
    #[default]
    Truck,
    Buggy,
}

impl CarClass {
    pub const ALL: [CarClass; 2] = [CarClass::Truck, CarClass::Buggy];

    pub fn spec(self) -> &'static CarSpec {
        &CAR_SPECS[self as usize]
    }

    /// Cycles through `ALL` — what the starting-grid `C` key advances.
    pub fn next(self) -> Self {
        match self {
            CarClass::Truck => CarClass::Buggy,
            CarClass::Buggy => CarClass::Truck,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            CarClass::Truck => "Truck",
            CarClass::Buggy => "Buggy",
        }
    }

    /// `models/derby-<class>.glb`, relative to `assets/` — `model::CarRigs`
    /// loads this at startup; `export.rs`'s CLI defaults its output path to
    /// the same spot.
    pub fn asset_path(self) -> &'static str {
        match self {
            CarClass::Truck => "models/derby-car.glb",
            CarClass::Buggy => "models/derby-buggy.glb",
        }
    }
}

/// One car class's complete spec: the physics half (`VehicleSpec`, shared
/// with the library's driving model) plus the visual-geometry half the
/// exporter bakes into that class's `.glb`. `Copy` and entirely
/// `const`-constructible, so `CAR_SPECS` is a plain array with no startup
/// cost.
///
/// `Deref`s to its `physics`, so `spec.mass` / `spec.engine_force` /
/// `spec.top_speed` read through unchanged everywhere.
#[derive(Clone, Copy)]
pub struct CarSpec {
    /// Everything the driving model reads: mass, geometry, grip, engine
    /// forces, steering limits, fragility. Inserted on the chassis at spawn
    /// (`vehicle::spawn_car`), which is how `vehicle_controller` sees it.
    pub physics: VehicleSpec,

    // Visual geometry (`export.rs`'s only remaining reader — the running
    // game reads shapes and rest poses back from the `.glb`; see
    // `docs/car-model.md`).
    pub headlight_size: Vec3,
    pub headlight_offset: Vec3,
    pub windshield_size: Vec3,
    pub windshield_offset: Vec3,
    pub windshield_pitch: f32,
    pub fender_size: Vec3,
    pub fender_offset: Vec3,
    pub spoiler_size: Vec3,
    pub spoiler_offset: Vec3,
    pub spoiler_strut_size: Vec3,
    pub spoiler_strut_offset: Vec3,
    pub shield_size: Vec3,
    pub shield_offset: Vec3,
    pub shield_strut_size: Vec3,
    pub shield_strut_offset: Vec3,
    /// How far back into the nose a fully-crushed bar is driven (m).
    pub shield_crush_depth: f32,
}

impl Deref for CarSpec {
    type Target = VehicleSpec;

    fn deref(&self) -> &Self::Target {
        &self.physics
    }
}

/// Indexed by `CarClass as usize` — `CarClass::spec()` is the one accessor;
/// nothing else should index this directly.
pub const CAR_SPECS: [CarSpec; 2] = [
    // --- Truck: the original car, unchanged in feel. ---
    CarSpec {
        physics: VehicleSpec {
            chassis_size: Vec3::new(1.8, 0.6, 3.6),
            collider_size: Vec3::new(1.7, 0.5, 3.4),
            mass: 350.0,
            com_offset: Vec3::new(0.0, -0.25, 0.0),
            spawn_height: 0.9,
            wheel_radius: 0.35,
            wheel_width: 0.3,
            // Order FL/FR/RL/RR — the numbers `export.rs` bakes into the
            // `.glb`'s node poses. Nothing at runtime reads these directly
            // (`vehicle.rs` reads mounts back off the loaded rig); they exist
            // so the exporter has a per-class source.
            wheel_mounts: [
                Vec3::new(-0.9, -0.1, 1.3),
                Vec3::new(0.9, -0.1, 1.3),
                Vec3::new(-0.9, -0.1, -1.3),
                Vec3::new(0.9, -0.1, -1.3),
            ],
            suspension_rest: 0.40,
            mu_front: 1.10,
            mu_rear: 0.95,
            engine_force: 3_200.0,
            brake_force: 1_200.0,
            reverse_force: 1_500.0,
            top_speed: 21.0,
            max_steer_angle: 0.55,
            steer_accel_limit: 13.0,
            damage_scale: 1.0,
        },
        headlight_size: Vec3::new(0.3, 0.18, 0.1),
        headlight_offset: Vec3::new(0.55, 0.1, 1.8),
        windshield_size: Vec3::new(1.5, 0.5, 0.05),
        windshield_offset: Vec3::new(0.0, 0.5, 0.6),
        windshield_pitch: -0.6,
        fender_size: Vec3::new(0.15, 0.12, 2.8),
        fender_offset: Vec3::new(0.9, 0.35, 0.0),
        spoiler_size: Vec3::new(1.6, 0.06, 0.35),
        spoiler_offset: Vec3::new(0.0, 0.55, -1.7),
        spoiler_strut_size: Vec3::new(0.08, 0.25, 0.08),
        spoiler_strut_offset: Vec3::new(0.5, 0.42, -1.7),
        shield_size: Vec3::new(1.9, 0.22, 0.18),
        shield_offset: Vec3::new(0.0, -0.12, 1.85),
        shield_strut_size: Vec3::new(0.1, 0.14, 0.2),
        shield_strut_offset: Vec3::new(0.6, -0.12, 1.72),
        shield_crush_depth: 0.12,
    },
    // --- Buggy: smaller, quicker, grippier, far more fragile. Body
    // dimensions are the Truck's scaled ×0.83 (width/height) / ×0.81
    // (length) — genuinely smaller, not just lighter — see SPEC.md Phase 18
    // for the full rationale (launch/top-speed/cornering/fragility deltas).
    CarSpec {
        physics: VehicleSpec {
            chassis_size: Vec3::new(1.5, 0.5, 2.9),
            collider_size: Vec3::new(1.4, 0.42, 2.75),
            mass: 230.0,
            com_offset: Vec3::new(0.0, -0.21, 0.0),
            spawn_height: 0.78,
            wheel_radius: 0.30,
            wheel_width: 0.26,
            wheel_mounts: [
                Vec3::new(-0.75, -0.08, 1.05),
                Vec3::new(0.75, -0.08, 1.05),
                Vec3::new(-0.75, -0.08, -1.05),
                Vec3::new(0.75, -0.08, -1.05),
            ],
            suspension_rest: 0.32,
            // Small, sticky tires: this (not raw engine force) is what buys
            // the quicker launch and the tighter cornering — see the
            // friction-circle math in SPEC.md Phase 18.
            mu_front: 1.35,
            mu_rear: 1.20,
            engine_force: 2_700.0,
            brake_force: 900.0,
            reverse_force: 1_200.0,
            top_speed: 25.0,
            max_steer_angle: 0.62,
            steer_accel_limit: 16.0,
            // On top of the ~1.5x Δv a 230 kg car already takes for free from
            // `Δv = impulse / mass` vs. the Truck's 350 kg — a glass cannon,
            // not a wrecking ball.
            damage_scale: 1.15,
        },
        headlight_size: Vec3::new(0.25, 0.15, 0.08),
        headlight_offset: Vec3::new(0.46, 0.08, 1.45),
        windshield_size: Vec3::new(1.25, 0.42, 0.04),
        windshield_offset: Vec3::new(0.0, 0.42, 0.48),
        windshield_pitch: -0.6,
        fender_size: Vec3::new(0.13, 0.10, 2.26),
        fender_offset: Vec3::new(0.75, 0.29, 0.0),
        spoiler_size: Vec3::new(1.33, 0.05, 0.28),
        spoiler_offset: Vec3::new(0.0, 0.46, -1.37),
        spoiler_strut_size: Vec3::new(0.07, 0.21, 0.06),
        spoiler_strut_offset: Vec3::new(0.42, 0.35, -1.37),
        shield_size: Vec3::new(1.58, 0.18, 0.15),
        shield_offset: Vec3::new(0.0, -0.10, 1.49),
        shield_strut_size: Vec3::new(0.08, 0.12, 0.16),
        shield_strut_offset: Vec3::new(0.5, -0.10, 1.39),
        shield_crush_depth: 0.10,
    },
];

// --- Damage -----------------------------------------------------------------
//
// The impulse-to-Δv half of this (the landing discount, the Δv threshold,
// damage per Δv, the front/top zone boundaries, prop softness, debris mass and
// fling) now lives in `vehicle::ImpactTuning` / `vehicle::DebrisConfig`, whose
// defaults are the numbers that used to be here. What's left is derby's own:
// which parts exist, what protects them, and when a car is out.

/// Limp mode: engine/reverse force scales from 1.0 at full health down to
/// this floor at 0 — a wrecked engine still crawls back into the fight.
pub const ENGINE_MIN_POWER: f32 = 0.3;
/// Engine health at or below which white steam pours from the hood.
pub const STEAM_THRESHOLD: f32 = 0.5;
/// Engine health at or below which the steam turns to black smoke.
pub const SMOKE_THRESHOLD: f32 = 0.2;
/// Windshield health at or below which the glass shows cracks (material
/// swap); at 0 it shatters (despawns).
pub const WINDSHIELD_CRACK_THRESHOLD: f32 = 0.5;
/// Fraction of a frontal hit the ram bar eats, scaled by its remaining
/// health: at 0.75 a pristine bar leaves only a quarter of the impact for
/// the engine/lights/glass behind it, fading as the bar crumples.
pub const SHIELD_ABSORB: f32 = 0.75;
/// Bar health lost per unit of *raw* front-zone damage (pre-absorption —
/// eating the hit is the bar's job). Four to six good rams tear it off.
pub const SHIELD_WEAR: f32 = 0.8;
/// Extra damage a front-zone hit deals to the *other* car, scaled by the
/// rammer's remaining bar health: up to 1.6x with a pristine bar, 1.0x with
/// none. This is the reward for lining a rival up and hitting nose-first.
pub const RAM_DAMAGE_BONUS: f32 = 0.6;
/// Bar health at or below which it swaps to the scraped/rusted material
/// (mirrors `WINDSHIELD_CRACK_THRESHOLD`).
pub const SHIELD_DENT_THRESHOLD: f32 = 0.5;
/// Weight of `engine` in `Damage::condition()`'s overall health score: the
/// engine is the only part that changes what the car can *do* — it drives
/// `engine_power()`'s limp-mode floor — so it dominates the aggregate.
pub const CONDITION_ENGINE_WEIGHT: f32 = 0.5;
/// Weight of `shield` (the ram bar) in `Damage::condition()`: the only part
/// that changes what a hit *costs* (`SHIELD_ABSORB`) and *deals*
/// (`RAM_DAMAGE_BONUS`). Everything else — lights, glass, fenders,
/// spoiler — is scored but cosmetic, and shares the remaining `1.0 -
/// CONDITION_ENGINE_WEIGHT - CONDITION_SHIELD_WEIGHT`.
pub const CONDITION_SHIELD_WEIGHT: f32 = 0.3;
/// `Damage::condition()` at or below which a car is wrecked — cut out of
/// the fight and left as an inert, smoking obstacle (see `game.rs`). Low
/// rather than zero: `engine_power()`'s limp-mode floor means `condition()`
/// alone never quite reaches 0 from engine damage, so a literal 0.0
/// threshold would make some cars unkillable by attrition.
pub const WRECK_CONDITION: f32 = 0.10;

// --- Impact bursts -----------------------------------------------------------

/// Per-hit part damage at or above which an impact throws the heavy spark
/// burst instead of the light one (0.05 = the hit removed ≥ 5% of a part).
pub const BURST_HEAVY_DAMAGE: f32 = 0.05;
/// Particles per burst tier.
pub const BURST_LIGHT_COUNT: f32 = 10.0;
pub const BURST_HEAVY_COUNT: f32 = 36.0;
/// Spark particle lifetime range (s) — short and hot.
pub const BURST_LIFETIME_MIN: f32 = 0.3;
pub const BURST_LIFETIME_MAX: f32 = 0.7;
/// How long a burst entity lives before it's reaped (spawn happens on the
/// first frame; this just needs to outlast the longest particle).
pub const BURST_ENTITY_AGE: f32 = 1.5;
pub const BURST_PARTICLE_CAPACITY: u32 = 64;

// --- Landing thump (dust puff + shake, no damage) ---------------------------

// The lower end of this ramp is `VehicleTuning::landing_thump_min_speed` —
// below it the controller doesn't emit a `WheelLanding` at all, so there's
// nothing here to scale. `juice.rs` reads it off the resource.
//
/// Closing speed (m/s) at which a landing's screenshake contribution
/// saturates to full — mirrors `SHAKE_FULL_DELTA_V`'s role for impacts.
pub const LANDING_THUMP_FULL_SPEED: f32 = 8.0;
/// Trauma added per wheel that lands hard, before the same distance falloff
/// `shake_camera` applies to impacts. About a third of `SHAKE_PER_IMPACT`
/// deliberately: up to four wheels can land the same tick, and trauma is
/// capped at 1.0 regardless.
pub const SHAKE_PER_LANDING: f32 = 0.22;
/// Dust particles per landing puff.
pub const LANDING_DUST_COUNT: f32 = 14.0;
/// Dust particle lifetime range (s) — lingers longer than a spark, since dust
/// settles instead of burning out.
pub const LANDING_DUST_LIFETIME_MIN: f32 = 0.5;
pub const LANDING_DUST_LIFETIME_MAX: f32 = 1.1;
pub const LANDING_DUST_PARTICLE_CAPACITY: u32 = 32;

// --- Car parts (visual geometry, chassis-local) -----------------------------
//
// Per-class sizes/offsets now live on `CarSpec` (`export.rs` is their only
// reader — it bakes them into each class's `.glb` node poses; the running
// game reads shape and rest poses back from that file, see
// `docs/car-model.md`). What's left here is genuinely shared: the same bulb
// look and the same crumple angle regardless of which car it's bolted to.

/// Headlight emissive at full health; dimmed by health² as the light takes
/// damage, reaching zero (dead dark) at 0.
pub const HEADLIGHT_EMISSIVE: LinearRgba = LinearRgba::new(2.5, 2.2, 1.4, 1.0);
/// Roll (radians, ~20°) of a fully-crushed ram bar, so a nearly-dead bar
/// visibly hangs off one mount before it tears away.
pub const SHIELD_SAG_ANGLE: f32 = 0.35;

// --- Engine steam / smoke ----------------------------------------------------

/// Where on the chassis the steam/smoke emitters sit (the hood).
pub const ENGINE_EMITTER_OFFSET: Vec3 = Vec3::new(0.0, 0.35, 1.2);
pub const STEAM_SPAWN_RATE: f32 = 40.0;
pub const STEAM_PARTICLE_LIFETIME: f32 = 1.2;
pub const STEAM_PARTICLE_CAPACITY: u32 = 512;
pub const SMOKE_SPAWN_RATE: f32 = 60.0;
pub const SMOKE_PARTICLE_LIFETIME: f32 = 2.2;
pub const SMOKE_PARTICLE_CAPACITY: u32 = 1024;

// --- AI drivers --------------------------------------------------------------

/// Steer command per radian of bearing to the steering target — at 2.0 the
/// AI is at full lock beyond ~28° off the nose, tracking smoothly inside.
pub const AI_STEER_GAIN: f32 = 2.0;
/// Bearing (radians, ~100°) beyond which a moving AI yanks the handbrake to
/// swing toward a target behind it instead of driving a wide arc. Applied
/// as a pure function of the final steer bearing, so every state — not
/// just `Hunt` — gets the swing for free.
pub const AI_HANDBRAKE_BEARING: f32 = 1.75;
/// Minimum planar speed (m/s) for the handbrake swing — below it the yank
/// does nothing but stall the chase.
pub const AI_HANDBRAKE_MIN_SPEED: f32 = 6.0;
/// How far ahead (seconds along current planar velocity) the AI projects
/// itself for wall avoidance.
pub const AI_WALL_LOOKAHEAD: f32 = 1.0;
/// Predicted positions closer than this (m) to the wall trigger avoidance.
pub const AI_WALL_MARGIN: f32 = 6.0;
/// Planar speed (m/s) below which a full-throttle AI counts as stuck.
pub const AI_STUCK_SPEED: f32 = 0.6;
/// Seconds of continuous stuck-ness before the AI gives up and reverses.
pub const AI_STUCK_TIME: f32 = 1.5;
/// How long (seconds) a retreat lasts before hunting again. If still stuck,
/// the cycle repeats — alternating shoves work a car free of most corners.
pub const AI_RETREAT_TIME: f32 = 1.2;

// --- AI drivers: vision & targeting ------------------------------------------

/// Half-angle (radians, ~60°) of the forward vision cone used to pick a
/// hunt target — a 120° cone total.
pub const AI_VISION_HALF_ANGLE: f32 = 1.05;
/// Range (m) of the vision cone: 0.7x the arena radius, about 2.7 s of
/// travel at cruise speed — a chase worth committing to. Beyond it a
/// random fallback pick is no worse.
pub const AI_VISION_RANGE: f32 = 45.0;
/// Weight of nose alignment in target scoring — dominant, since a target
/// you're already pointed at is one you can ram nose-first, and nose-first
/// is where the damage (and the ram bonus) is.
pub const AI_SCORE_ALIGNMENT: f32 = 0.40;
/// Weight of proximity in target scoring — preserves the old "nearest"
/// behaviour as the main secondary term.
pub const AI_SCORE_CLOSENESS: f32 = 0.35;
/// Weight of the target's own damage in target scoring: wounded cars get
/// finished off. Enough to redirect the pack onto a weak target without
/// making every fight a dogpile.
pub const AI_SCORE_WEAKNESS: f32 = 0.25;
/// Score penalty applied to an immune target — larger than any positive
/// term above, since attacking an immune car is strictly negative EV
/// (hits do nothing to them, but they still deal full damage plus their
/// own ram bonus back). Not a hard ban: still a last resort if it's the
/// only thing in the cone.
pub const AI_SCORE_IMMUNE_PENALTY: f32 = 0.50;
/// Seconds a hunt target stays committed before re-evaluating. Long
/// enough to actually close the distance (about 40 m at cruise speed —
/// most of the arena) instead of dithering between two equidistant
/// rivals every tick.
pub const AI_RETARGET_TIME: f32 = 3.0;
/// +/- fraction of `AI_RETARGET_TIME` the retarget timer is jittered by,
/// so cars that spawn (and so first pick) on the same tick don't stay in
/// lockstep re-deciding together forever.
pub const AI_RETARGET_JITTER: f32 = 0.35;
/// A committed target beyond this range (m) — just under the arena
/// radius — is dropped early: it's run too far to be worth crossing the
/// whole floor for.
pub const AI_TARGET_DROP_RANGE: f32 = 60.0;
/// Cap (seconds) on how far ahead of a target's current velocity the AI
/// leads its aim point. This is what turns rams into *front-zone* rams —
/// the whole point of the ram bar. Higher over-leads into empty floor
/// since the target doesn't drive straight either.
pub const AI_LEAD_TIME_MAX: f32 = 1.0;
/// Minimum target speed (m/s) used in the lead-time calculation, so a
/// stationary target doesn't produce a divide-by-near-zero lead.
pub const AI_LEAD_MIN_SPEED: f32 = 4.0;
/// Seed mixed with each fellow car's spawn index for its private xorshift
/// stream (`AiDriver::new`). Any nonzero value; xorshift is a fixed point
/// at 0.
pub const AI_RNG_SEED: u64 = 0x9E37_79B9_7F4A_7C15;

// --- AI drivers: threat assessment -------------------------------------------

/// Seconds ahead a threat's closing geometry is projected. About one
/// evasive manoeuvre (~1 s at derby speeds) plus margin.
pub const AI_THREAT_HORIZON: f32 = 1.5;
/// Miss distance (m) inside which a projected close approach counts as a
/// threat at all. Two 1.8x3.6 m chassis boxes whose centers pass within
/// ~2.7 m are already touching in the worst orientation; 3.0 adds a small
/// margin without flagging cars 5 m away.
pub const AI_THREAT_RADIUS: f32 = 3.0;
/// Relative speed (m/s) below which closing geometry isn't even
/// evaluated. At this closing speed a hit costs about
/// `(3.0 - IMPACT_MIN_DELTA_V) * DAMAGE_PER_DELTA_V` = 0.09 of a part — a
/// scrape, not worth breaking off a chase for.
pub const AI_THREAT_MIN_CLOSING: f32 = 3.0;
/// Relative speed (m/s) at which threat severity saturates to 1.0. At this
/// closing speed a hit costs ~0.63 of a part (~1.0 with a healthy
/// attacker's ram bonus) — genuinely car-wrecking.
pub const AI_THREAT_FULL_CLOSING: f32 = 12.0;
/// Severity above which `Hunt` breaks into `Brace`/`Evade`. Tuned so a
/// head-on threat at 12 m/s closing, 0.5 m off-center, ~0.75 s out
/// (severity ~0.42) triggers, while the same geometry 1.2 s out (~0.17)
/// doesn't — the AI reacts roughly 0.7-0.9 s before contact.
pub const AI_THREAT_ENTER: f32 = 0.35;
/// Severity below which `Brace`/`Evade` relaxes back to `Hunt`. Well under
/// `AI_THREAT_ENTER` (a Schmitt trigger) so severity oscillating near the
/// boundary doesn't flicker the state every tick.
pub const AI_THREAT_EXIT: f32 = 0.15;

// --- AI drivers: defence -------------------------------------------------

/// Ram bar health above which the AI is willing to `Brace` (turn to meet a
/// threat nose-first) instead of evading. Below this the bar barely
/// absorbs or rewards a hit any more, so it isn't armor.
pub const AI_BRACE_MIN_SHIELD: f32 = 0.25;
/// Overall `Damage::condition()` above which the AI is willing to `Brace`.
/// A nearly-wrecked car shouldn't trade blows even nose-first.
pub const AI_BRACE_MIN_CONDITION: f32 = 0.25;
/// Assumed achievable yaw rate (rad/s) when deciding whether a car can
/// rotate to meet a threat in time. Derived from the bicycle-model yaw
/// `v * tan(steer_lock) / wheelbase` using `vehicle_controller`'s
/// speed-limited lock: ~1.6 rad/s at 8 m/s, ~0.87 at 15 m/s for the Truck
/// (the Buggy's shorter wheelbase and wider lock both raise its own
/// achievable rate). 1.2 is a deliberately conservative mid-range figure —
/// shared across classes rather than tuned per one, since it's already
/// conservative for the nimbler Buggy too.
pub const AI_BRACE_YAW_RATE: f32 = 1.2;
/// Bearing (radians) to a threat's lead point inside which the car counts
/// as "aimed" and commits full throttle for the counter-ram, rather than
/// still turning into it.
pub const AI_BRACE_ALIGNED: f32 = 0.30;
/// How far (m) an `Evade` aim point is placed off to the side, perpendicular
/// to the threat's relative velocity (the direction that grows miss
/// distance fastest). Far enough for a stable bearing, near enough not to
/// read as driving sideways forever.
pub const AI_EVADE_DISTANCE: f32 = 8.0;
/// Bearing (radians, ~35°) inside which an evading car's threat counts as
/// genuinely head-on, and braking (killing the Δv a head-on hit would
/// deal) beats accelerating away (which a side threat needs the grip
/// for).
pub const AI_EVADE_BRAKE_BEARING: f32 = 0.6;

// --- AI drivers: obstacle & wall avoidance ------------------------------

/// How far ahead (seconds, scaled by speed) the AI casts its forward
/// obstacle whiskers. Clearing the r=3 m center pillar at cruise speed
/// needs about 0.77 s / 10 m of lateral manoeuvre; 1.4 s gives comfortable
/// margin beyond that.
pub const AI_OBSTACLE_LOOKAHEAD: f32 = 1.4;
/// Floor (m) on whisker range, so a crawling car still sees one car-length
/// ahead.
pub const AI_OBSTACLE_MIN_RANGE: f32 = 5.0;
/// Ceiling (m) on whisker range, so a car at top speed isn't reacting to
/// scenery on the far side of the arena.
pub const AI_OBSTACLE_MAX_RANGE: f32 = 20.0;
/// Whisker hit distance (seconds of travel at current speed) below which
/// obstacle avoidance overrides steering *and* brakes, rather than just
/// nudging the racing line.
pub const AI_OBSTACLE_BRAKE_TIME: f32 = 0.8;
/// Maximum soft steering bias (radians) applied while an obstacle is
/// merely in range but not yet close enough for the hard override — decays
/// to 0 as the whisker's hit distance approaches full range.
pub const AI_OBSTACLE_STEER: f32 = 0.8;
/// Steering bearing (radians) commanded by the hard obstacle override —
/// well past full lock, i.e. "turn as hard as possible".
pub const AI_OBSTACLE_HARD_BEARING: f32 = 1.0;
/// Whisker distance difference (m) below which left/right clearance counts
/// as a tie, and the AI picks the side its current goal is on rather than
/// the marginally more-open one.
pub const AI_OBSTACLE_TIE: f32 = 1.0;
/// Minimum `Mass` (kg) a dynamic body needs to count as an obstacle worth
/// avoiding, rather than something to shove through. Between the heaviest
/// pushable prop (the 50 kg crate) and the lightest true obstacle (the
/// 500 kg block) — "avoid what you can't shove". Bodies with no `Mass` at
/// all (all static scenery in this arena) are always avoided.
pub const AI_AVOID_MIN_MASS: f32 = 200.0;
/// How far (0..1, as a fraction mixed with the wall-tangent direction) a
/// wall-avoidance escape peels inward off the tangent. Guarantees a net
/// inward drift — the car spirals off the wall instead of orbiting it —
/// without demanding a full reversal of momentum, and (unlike aiming at
/// the arena center) never points straight at the center pillar.
pub const AI_WALL_TURN_IN: f32 = 0.6;

// --- AI drivers: self-preservation ---------------------------------------

/// `Damage::condition()` at or below which a `Hunt`ing car breaks off into
/// `Recover`. Roughly "engine under half and the ram bar gone" with the
/// `CONDITION_*_WEIGHT`s above — the point where limp mode is real and
/// there's no armor left.
pub const AI_RECOVER_CONDITION: f32 = 0.35;
/// `Recover` exits once condition rises this far above
/// `AI_RECOVER_CONDITION` (i.e. at 0.45). A `REPAIR_PATCH_AMOUNT` pickup
/// adds ~0.2 to every part health, so one successful pickup reliably
/// clears the exit threshold — the state has a visible payoff.
pub const AI_RECOVER_HYSTERESIS: f32 = 0.10;
/// Maximum seconds a `Recover` lasts before giving up and rejoining the
/// fight regardless — long enough to cross a third of the arena to a
/// pickup, short enough that a hopeless case doesn't hide forever.
pub const AI_RECOVER_TIME: f32 = 8.0;
/// Seconds after a `Recover` times out before the same car is allowed to
/// enter `Recover` again. Without this, a car whose health can never
/// improve re-enters on the very next tick and never fights again.
pub const AI_RECOVER_COOLDOWN: f32 = 6.0;
/// A power-up within this range (m) is worth detouring to while
/// recovering, rather than just running for open space.
pub const AI_RECOVER_POWERUP_RANGE: f32 = 39.0;
/// Distance (m) a `Recover` with no power-up in range runs from the
/// nearest threat, away along the inverse-square-weighted repulsion from
/// every other car (so the nearest car dominates the escape direction).
pub const AI_REFUGE_DISTANCE: f32 = 20.0;

// --- AI drivers: throttle & telemetry -------------------------------------

/// Throttle commanded while lifting off to turn tighter — `Hunt` past
/// `AI_TURN_LIFT_BEARING`, `Brace` before it's aimed. Shares the tire's
/// friction circle between drive and lateral force with
/// `vehicle_controller`, so easing off genuinely tightens the turn. Set to
/// 0.0 to make this a pure lift-off (no controller change needed); a
/// nonzero value needs `vehicle_controller` to honor `DriveInput::drive`'s
/// magnitude rather than quantizing it to +-1.0.
pub const AI_THROTTLE_TURN: f32 = 0.55;
/// Steer bearing (radians) beyond which `Hunt` lifts off the throttle to
/// turn tighter instead of driving a wide arc at full power.
pub const AI_TURN_LIFT_BEARING: f32 = 0.5;
/// Interval (seconds) between the AI's per-car telemetry log lines.
pub const AI_TELEMETRY_INTERVAL: f32 = 1.0;

// --- HUD --------------------------------------------------------------------

/// Exponential smoothing rate for the HUD acceleration readout — raw
/// per-frame velocity deltas are too noisy to read.
pub const HUD_ACCEL_SMOOTH_RATE: f32 = 10.0;

// --- Camera ----------------------------------------------------------------

/// Distance behind the car (along its yaw-only forward) the camera targets.
pub const CAMERA_BACK: f32 = 7.0;
/// Height above the car the camera targets.
pub const CAMERA_UP: f32 = 3.0;
/// How quickly the camera eases toward its target transform each second.
pub const CAMERA_LERP_RATE: f32 = 5.0;

// --- Breakable crates ---------------------------------------------------

/// Crate Δv (its own impulse / `CRATE_MASS`, m/s) at or above which it
/// bursts — reads as "hit at 8 m/s (29 km/h)", a deliberate run at ~40%
/// of top speed. Sustained pushing can't reach it: full engine force on a
/// crate is Δv ≈ 0.9 per tick, resting under gravity is Δv ≈ 0.15.
pub const CRATE_BREAK_DELTA_V: f32 = 8.0;
/// One shard per octant of the box — `shatter` derives the octant signs
/// from the low three bits of the shard index, so this must stay 8.
pub const CRATE_SHARD_COUNT: usize = 8;
/// Shard cube edge (m), deliberately under an octant's 0.6 m spacing so
/// the pieces read as splinters with daylight between them.
pub const CRATE_SHARD_SIZE: f32 = 0.45;
/// 8 × 4 kg = 32 kg against the crate's 50: the debris field is lighter
/// than the thing that made it, so it reads as scenery, not a wall.
pub const CRATE_SHARD_MASS: f32 = 4.0;
/// Extra outward speed (m/s) each shard gets on top of the crate's own
/// velocity at that corner. Mirrors `DETACH_FLING_SPEED`, a touch lower
/// since eight pieces spreading read faster than one part flying off.
pub const CRATE_SHARD_FLING_SPEED: f32 = 2.5;
/// Shards are reaped after this (s) so a long session doesn't accumulate
/// rigid bodies in the contact graph forever.
pub const CRATE_SHARD_LIFETIME: f32 = 8.0;
/// How long (s) a broken crate takes to reappear at its original spot.
pub const CRATE_RESPAWN_DELAY: f32 = 20.0;

// --- Power-up pickups --------------------------------------------------------

/// Pickup cube edge (m): big enough to spot from the chase camera, small
/// enough not to hide the car behind it.
pub const POWERUP_SIZE: f32 = 0.5;
/// Height (m) of the bob's midpoint above the floor — above the ram bar
/// (top −0.01) and around windshield height, so a car drives through it
/// rather than under it.
pub const POWERUP_FLOAT_HEIGHT: f32 = 1.1;
/// Bob amplitude (m) and rate (rad/s): alive, not jittery.
pub const POWERUP_BOB_AMPLITUDE: f32 = 0.15;
pub const POWERUP_BOB_RATE: f32 = 2.5;
/// Yaw spin (rad/s): one turn every ~4 s, slow enough to read the shape.
pub const POWERUP_SPIN_RATE: f32 = 1.6;
/// Collection radius (m) from the chassis center. The chassis half-length
/// is 1.8, so head-on this triggers ~0.4 m ahead of the nose; a car
/// passing alongside at more than 2.2 m misses it.
pub const POWERUP_PICKUP_RADIUS: f32 = 2.2;
/// A drop nobody wants expires after this (s), so the arena doesn't
/// litter and breaking a crate stays a decision with a clock on it.
pub const POWERUP_LIFETIME: f32 = 25.0;
/// Emissive multiplier on the pickup's per-kind color.
pub const POWERUP_EMISSIVE: f32 = 3.0;
/// A drop is pulled at least this far (m) inside `ARENA_RADIUS`: a crate
/// smashed flat against a wall panel must not leave its loot inside it.
pub const POWERUP_WALL_MARGIN: f32 = 2.5;

// --- Power-up effects --------------------------------------------------------

/// "Patch job": added to every part health, clamped at 1.0.
pub const REPAIR_PATCH_AMOUNT: f32 = 0.2;
/// Temporary damage immunity (s) — a couple of exchanges at derby pace.
pub const IMMUNITY_DURATION: f32 = 5.0;
/// Radius (m) of the translucent immunity bubble around the chassis —
/// outside the 1.8 × 0.6 × 3.6 body (half-diagonal ≈ 2.05).
pub const IMMUNITY_BUBBLE_RADIUS: f32 = 2.3;

// --- Match -------------------------------------------------------------------

/// Driver name, paint (r, g, b), and default class per starting-grid slot,
/// matched to `vehicle::SPAWN_ANGLES` — slot 0 is the player. The single
/// place `vehicle::spawn_vehicle` (paint + `game::Driver`) and the results
/// panel read identity from. Slot 0's class is only the *seed* for
/// `game::PlayerClass` — pressing `C` on the grid overrides it; every AI
/// slot's class is fixed. One AI slot (Vera) rides the Buggy so a mixed
/// field falls out of this table with no other plumbing.
pub const DRIVERS: [(&str, (f32, f32, f32), CarClass); 4] = [
    ("You", (0.15, 0.4, 0.75), CarClass::Truck),
    ("Rusty", (0.8, 0.2, 0.2), CarClass::Truck),
    ("Vera", (0.2, 0.65, 0.3), CarClass::Buggy),
    ("Duke", (0.4, 0.4, 0.45), CarClass::Truck),
];
/// Countdown length (s) before a match's cars unlock — long enough to read
/// "3, 2, 1, GO!" without dragging.
pub const COUNTDOWN_SECS: f32 = 3.0;

// --- Nameplates ----------------------------------------------------------

/// Nameplate width (px) — wide enough for the longest `DRIVERS` name plus
/// a `(Buggy)` class tag and the " · P4" wreck suffix without wrapping.
pub const NAMEPLATE_WIDTH: f32 = 150.0;
/// Height (m) above the chassis origin the nameplate anchors to — clears
/// the roofline (chassis half-height 0.3) plus headroom to read as
/// floating above the car, not clipped into it.
pub const NAMEPLATE_HEIGHT_OFFSET: f32 = 1.6;
/// Health-bar width (px) inside the nameplate.
pub const NAMEPLATE_BAR_WIDTH: f32 = 80.0;
/// Health-bar height (px).
pub const NAMEPLATE_BAR_HEIGHT: f32 = 5.0;

// --- Screenshake --------------------------------------------------------

/// Trauma added per impact, before the Δv/distance falloffs below, clamped
/// to 1.0 total. Trauma², not trauma, drives the shake angle — see
/// `juice::shake_camera` — so this is tuned generously; the square already
/// makes small hits barely register.
pub const SHAKE_PER_IMPACT: f32 = 0.6;
/// Impact Δv (m/s) at or above which a hit contributes full trauma; scales
/// linearly from `IMPACT_MIN_DELTA_V` (zero contribution) up to this.
pub const SHAKE_FULL_DELTA_V: f32 = 12.0;
/// Distance (m) from the camera at which an impact's contribution has
/// faded to zero — a fight on the far side of the arena rumbles faintly
/// instead of not shaking at all.
pub const SHAKE_RANGE: f32 = 40.0;
/// Trauma decay per second, applied on real (unscaled) time so the shake
/// stays crisp through slow-motion instead of smearing out.
pub const SHAKE_DECAY: f32 = 2.0;
/// Rotational shake amplitude (radians) at trauma = 1.0.
pub const SHAKE_MAX_ANGLE: f32 = 0.05;
/// Frequencies (Hz) of the three out-of-phase sines summed for the shake's
/// pitch/yaw/roll — deterministic and RNG-free, the same layered-sine trick
/// `008-colony` uses for wind.
pub const SHAKE_FREQS: [f32; 3] = [11.0, 17.0, 23.0];

// --- Slow motion ----------------------------------------------------------

/// Near-miss projection horizon (s), mirroring `AI_THREAT_HORIZON` — about
/// one evasive manoeuvre's worth of look-ahead.
pub const NEAR_MISS_HORIZON: f32 = 1.5;
/// Miss distance (m) inside which a projected closest approach starts an
/// approach record at all — mirrors `AI_THREAT_RADIUS`.
pub const NEAR_MISS_RADIUS: f32 = 3.0;
/// Relative closing speed (m/s) at which severity saturates to 1.0 —
/// mirrors `AI_THREAT_FULL_CLOSING`.
pub const NEAR_MISS_FULL_CLOSING: f32 = 12.0;
/// Severity above which an approach record opens for a pair.
pub const NEAR_MISS_ARM: f32 = 0.35;
/// An avoided approach only fires slow-motion if its *peak* severity while
/// closing reached this — a near miss has to have been a real threat, not
/// just brushed the arming threshold on the way past.
pub const NEAR_MISS_SEVERITY: f32 = 0.55;
/// Minimum centre-to-centre distance (m) the approach must have reached —
/// against a 1.8 × 3.6 m chassis, this is close enough to read as "just
/// missed" rather than "was never really near".
pub const NEAR_MISS_DISTANCE: f32 = 4.0;
/// Minimum peak closing speed (m/s) for an avoided approach to count — a
/// slow graze isn't a near miss worth a cinematic beat.
pub const NEAR_MISS_MIN_CLOSING: f32 = 8.0;
/// Seconds after one slow-motion trigger before the watched car can trigger
/// another, so a melee doesn't stutter in and out of it.
pub const NEAR_MISS_COOLDOWN: f32 = 5.0;
/// Relative time scale during slow-motion (of real-time speed).
pub const SLOWMO_SCALE: f32 = 0.35;
/// How long (real seconds) slow-motion holds before easing back to normal.
pub const SLOWMO_DURATION: f32 = 0.8;
/// Ease in/out time (real seconds) at each end of the hold, so the time
/// scale doesn't snap.
pub const SLOWMO_EASE: f32 = 0.15;
/// `CAMERA_BACK` is multiplied by this during slow-motion — pulling the
/// chase camera in is what reads as *cinematic* rather than merely laggy.
pub const SLOWMO_CAMERA_PULL: f32 = 0.75;
