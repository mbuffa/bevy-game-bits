# 009-derby ("Derby") — Phased spec: arena driving prototype (Destruction-Derby-inspired)

Status: **Phases 1–3 done and verified — 2026-07-27.** Rest settle (zero
jitter, 0.2° tilt), steady full-lock circle at 6.4 m radius / 7.2 m/s
(≈0.83 g, no oscillation, no rollover), μ-limited braking ~9.3 m/s² dead
straight, wall hits land drivable, S↔reverse threshold behaves. Verified
via a temporary scripted autopilot + telemetry (since removed).
Post-verification: arena grown 30 → 50 m radius and drive switched RWD →
AWD (same total engine force, launch now engine-limited ~8 m/s² instead of
rear-traction-limited ~5 m/s²) so acceleration is actually enjoyable —
top speed is reachable inside the arena. Phase 4 (Space handbrake for
drifting, with skid-decal hooks), Phase 5 (skid-mark decals), Phase 6
(pushable obstacles + three fellow cars behind a Player/DriveInput input
split), Phase 7 (destructible parts, impact damage, limp mode,
steam/smoke, vehicle-state HUD), Phase 8 (AI drivers), Phase 9 (arena
remodel: edge spawns, pillar, ramps, sculpted potholes; dimming lights +
impact spark bursts), Phase 10 (front ram bar + soft prop impacts), and
Phase 11 (breakable crates + power-ups) added same day. Phase 12 (AI
vision-cone targeting with commitment, closing-geometry threat
prediction, nose-first `Brace`/`Evade` defense, forward obstacle
whiskers, self-preservation `Recover`) added 2026-07-28. Phase 13 (car
model extraction to `derby-car.glb`) added 2026-07-29. Phase 14 (match
structure: countdown, knock-outs, results, restart) added 2026-08-02.
Phase 15 (arena grown 50 → 65 m radius with its layout, engine + drag
retuned for a ~3 s launch), Phase 16 (screenshake, deterministic
near-miss slow-motion), Phase 18 (car classes: the Buggy — smaller,
quicker, grippier, more fragile; per-car `CarSpec` replaces flat vehicle
constants), and Phase 19 (landing physics: a high-speed suspension damper
cap so ramp landings settle instead of bottoming out, plus a per-car
push-direction Δv discount so a clean landing costs no health, and a
cosmetic dust-puff/shake landing thump) added 2026-08-03.

## Context

A rapid prototype in the spirit of Destruction Derby: a circular arena with
wall "cylinder faces", a vehicle made of a box chassis and cylinder wheels, a
third-person chase camera slightly up and behind, WASD driving, collisions
with the arena walls, and a HUD showing velocity and acceleration.
Destructible parts come later.

The first pass (built with Opus) got the structure right but the driving
wrong: the tire model applied unclamped velocity-proportional grip forces
that overshot within a physics tick (jitter, hypersensitive inputs), the
near-equal front/rear grip made the car understeer into uselessness, and a
too-eager upright-assist torque fought normal cornering roll. This spec
covers the rewrite of the physics core into a **semi-sim raycast car**:
slip-based tire forces, load-dependent grip capped by a friction circle, so
drift and weight transfer emerge from the model instead of being scripted.

Decisions already made:

- **Keep the scaffold** (`arena.rs`, `camera.rs`, `main.rs` wiring, reset
  system, HUD, `config.rs` tuning-surface layout). Only the physics core of
  `vehicle.rs` is rewritten.
- **Semi-sim, keyboard-drivable**: slip-velocity tire model with a friction
  circle (`|F| <= mu * N` per wheel, `N` = that wheel's live suspension
  load), not full wheel angular dynamics. Each wheel computes forces from
  its *own* contact-point velocity, so inner/outer wheel speed differences
  in a turn are captured at the force level for free.
- **Stability via the one-step impulse clamp**: no tire force may exceed
  what would zero its slip within one fixed tick — this is what makes an
  explicit 64 Hz integrator of stiff tire forces stable.
- **AWD** (originally RWD; switched so the launch is engine-limited rather
  than rear-traction-limited) with slightly lower rear grip
  (`MU_REAR < MU_FRONT`) so the rear still saturates first in corners.
- **Ackermann steering**: in a turn the inner front wheel steers more than
  the outer, both derived from a shared turn radius.
- Rollover prevention by construction (low center of mass + lateral forces
  applied above the contact patch), not by fighting torque: the old upright
  assist survives only as a `flip_rescue` that ignores anything below ~57°
  of tilt.

## Architecture

```
FixedUpdate: vehicle_controller                    (before Avian's step)
  input -> smoothed throttle/steer (accel-limited lock, Ackermann per wheel)
  per wheel:
    raycast along -chassis_up  ->  suspension spring/damper force (load N)
    slip velocities at contact ->  lateral + longitudinal tire forces
                                   capped by friction circle mu*N
                                   capped by one-step impulse clamp
    apply: suspension+longitudinal at contact, lateral raised toward CoM
  chassis: quadratic aero drag; flip_rescue torque if tilt > ~57°
Update: wheel visuals (travel, steer angle, spin), chase camera, HUD, reset
```

All tuning lives in `config.rs`. The chassis is one `RigidBody::Dynamic`
with a collider slightly smaller than the visual box so the ray suspension
alone carries the car during normal driving; the collider exists for wall
hits, landings, and (later) car-vs-car contact.

---

## Phase 1 — Scaffold ✅

Circular floor + 24-segment wall ring (`arena.rs`), box chassis + four
cylinder wheel children (`vehicle.rs`), chase camera with yaw-only follow
(`camera.rs`), speed/throttle/steer HUD (`ui.rs`), R reset, physics debug
gizmos. Done in the first pass and kept.

## Phase 2 — Physics core rewrite

**Goal**: the car settles level without jitter, accelerates smoothly to a
top speed set by drag, turns with authority (mild oversteer at the limit),
and survives wall hits without oscillation.

- Suspension: raycast along `-chassis_up` (ramp-ready), hits gated by
  `normal · chassis_up >= GROUND_NORMAL_MIN`; spring−damper force clamped to
  `[0, SUSPENSION_MAX_FORCE]`, applied along `chassis_up` at the contact.
- Tires: `v_lat`/`v_long` slip at each wheel's own contact velocity;
  lateral = `-right * v_lat * TIRE_LAT_STIFFNESS`; longitudinal =
  drive/brake/reverse + rolling resistance; whole vector scaled into the
  friction circle `mu * N`; lateral and braking components additionally
  clamped to the one-step-zeroing impulse.
- Drive: `ENGINE_FORCE` split across all four wheels; quadratic aero drag
  at the chassis sets top speed (~20 m/s); S brakes above
  `REVERSE_THRESHOLD` forward speed, reverses below it (W symmetric).
- Steering: lock limited by lateral acceleration
  (`atan(STEER_ACCEL_LIMIT * WHEELBASE / v²)`), Ackermann split per front
  wheel, exponential input easing.
- Chassis: collider shrunk to `CHASSIS_COLLIDER_SIZE`, `CenterOfMass`
  lowered, spawn height fixed so the car settles instead of drop-bouncing.
- `apply_upright_assist` → `flip_rescue`: threshold 1.0 rad, gentle gains;
  R reset stays the escape hatch.
- **Verify**: rest settle (3 s hands-off: level, zero jitter, speed <
  0.05 m/s); straight line (smooth accel to ~20 m/s, S brakes straight,
  reverses from rest); steady full-lock circle at throttle (~8–15 m radius,
  slight rear slip, no snap-spin, no rollover); slalom at ~15 m/s (body
  roll recovers, no assist interference); head-on wall hit (bounces, lands
  on wheels, immediately drivable); flip rescue rights near-rollovers.

## Phase 3 — Feedback: HUD acceleration + wheel spin

**Goal**: the spec's "velocity and acceleration" readout, and wheels that
visibly roll at their own speeds.

- HUD: signed forward acceleration (m/s², exponentially smoothed to be
  readable) alongside speed in m/s and km/h; keep throttle/steer.
- Wheel visuals: each wheel accumulates spin from its own ground-plane
  forward speed (`ω = v_long / WHEEL_RADIUS`) — inner and outer wheels
  visibly rotate at different rates in a turn; steering wheels show their
  Ackermann angle.
- **Verify**: drive a tight circle and watch inner/outer front wheels spin
  at visibly different rates; HUD acceleration goes positive under W,
  negative under braking, ~0 at top speed.

## Phase 4 — Handbrake & drifting

**Goal**: Space locks the rear axle so the car can be thrown into drifts.

- While Space is held (`Vehicle::handbrake`), the rear wheels run "locked":
  friction-circle μ scaled by `HANDBRAKE_MU_SCALE`, lateral stiffness by
  `HANDBRAKE_LAT_STIFFNESS_SCALE`, engine force dropped (the front two keep
  pulling — AWD becomes FWD, dragging the car through the drift), and a
  locked-wheel drag (`HANDBRAKE_DRAG_FORCE`, impulse-clamped) scrubs speed.
- The speed-based steering-lock cap is bypassed while the handbrake is held:
  drift entry and counter-steer need the full `MAX_STEER_ANGLE`.
- HUD shows a `HANDBRAKE` line while active.
- **Decal hooks** for the future skid-mark system: every fixed step each
  `Wheel` records `slip_speed` (lateral slip magnitude at the contact, m/s)
  and `contact_world` (ray hit point, `None` airborne). The decal system
  will read wheels with `slip_speed` above a threshold and stamp marks at
  `contact_world`.
- **Verify**: at speed, W + full steer + Space swings the tail out (slip
  angle well past the grip-limited regime, yaw rate above the ~1.1 rad/s
  grip ceiling); releasing Space with counter-steer catches the drift
  instead of spinning; Space alone in a straight line scrubs speed straight.

## Phase 5 — Skid-mark decals

**Goal**: drifting paints the map — dark tire marks persist on the floor.

- New `skidmarks.rs`, rendering-only: reads each `Wheel`'s
  `slip_speed`/`contact_world` hooks and stamps unlit translucent segment
  quads (`SKID_WIDTH` × distance) connecting successive contact points into
  continuous strips while `slip_speed >= SKID_SLIP_THRESHOLD`. Strips
  restart on grip/airborne; gaps over `SKID_MAX_SEGMENT` (R reset, hops)
  don't stamp. Quads sit `SKID_Y_OFFSET` above the floor, stagger-lifted
  per buffer slot against z-fighting.
- `slip_speed` semantics: lateral slip for a rolling tire, **full planar
  velocity for a handbrake-locked one** — so straight-line handbrake scrubs
  mark too.
- Marks persist (derby battle scars) but are bounded: a `SKID_MAX_MARKS`
  ring buffer recycles the oldest quad once full.
- **Verify**: a handbrake drift arc leaves curved strips, a straight
  handbrake scrub leaves straight parallel rear-wheel strips, plain
  acceleration/cruising leaves nothing, marks persist after driving away.

## Phase 6 — Arena props & fellow cars

**Goal**: the arena stops being an empty bowl — pushable obstacles and
three more cars to ram.

- **Input decoupling** (groundwork for AI): `Player` marker on the
  keyboard-driven chassis; per-chassis `DriveInput { drive, steer,
  handbrake }` intent component; `read_player_input` (FixedUpdate, chained
  before `vehicle_controller`) samples the keyboard into the Player's
  input. The controller reads each chassis's own `DriveInput` — an AI
  later just writes another car's. Camera/HUD/R-reset filter on `Player`.
- **Three fellow cars**: full physics twins spawned via the shared
  `spawn_car` helper (distinct paint, `FELLOW_CAR_SPAWNS` table) with zero
  input — they sit on live suspension, rock and slide when rammed, and
  their wheels leave skid marks when shoved.
- **Obstacles** (`obstacles.rs`, layout tables in-module, tunables in
  config): 4 light crates (50 kg — fly when rammed) clustered downrange of
  spawn, 3 heavy blocks (500 kg — barely budge), 4 balls (40 kg — roll).
  All dynamic rigid bodies with angular damping so nothing spins forever.
- **Verify**: all four cars settle without jitter; ramming the crate
  cluster at speed scatters it; the downrange car rocks/slides when hit
  and settles; blocks stop the car more than the car moves them; R resets
  only the player.

## Phase 7 — Destructible parts, damage & engine smoke ✅

**Goal**: the original spec's end goal — ramming has consequences. Every
car (player and fellows alike) carries destructible parts with health
1.0 → 0.0, damaged by collision impulses.

- **Parts** (`Damage` component per chassis; per-side arrays index 0 = −X
  = driver's right, 1 = +X = left): internal engine; two emissive
  headlight cubes (dead → dark material swap); an angled translucent
  windshield (≤ 0.5 → cracked material, 0 → shatters/despawns); two
  fender rails along the top side edges and a rear spoiler blade on strut
  stubs — both **detach at 0**: the visual child is replaced by a loose
  `RigidBody::Dynamic` debris body inheriting the chassis velocity at its
  mount plus an up-and-outward fling. Fenders and spoiler wear the car's
  paint so arena debris is attributable. Wheels stay unbreakable.
- **Impact model** (`damage.rs`): each fixed step, Avian's `Collisions`
  contact graph is read back; per manifold, the total normal impulse is
  converted to Δv (`impulse / CHASSIS_MASS`), thresholded
  (`IMPACT_MIN_DELTA_V`), and attributed by the impulse-weighted contact
  point in chassis-local space — top → windshield; front → engine 0.6 /
  nearer light 0.25 / windshield 0.15; rear → spoiler 0.8 / engine 0.1
  (reversing into people protects your engine — the classic derby
  tactic); sides → that side's fender. Car-vs-car pairs damage both,
  each through its own frame. One `info!` line per registered impact.
- **Limp mode**: engine/reverse force scales with engine health down to
  an `ENGINE_MIN_POWER` (0.3) floor — wounded, never stranded.
- **Steam & smoke** (`effects.rs`, first bevy_hanabi use in this
  example): two shared `EffectAsset`s (white steam puffs / big black
  smoke), every car gets both emitter children over the hood, a per-frame
  system flips `EffectSpawner.active` from engine health (steam in
  (0.2, 0.5], smoke ≤ 0.2). World-space simulation, so limping cars drag
  a trail.
- **Vehicle-state HUD**: top-right panel, one line per part,
  green→yellow→red by health, `SHATTERED`/`GONE`/`DETACHED` at zero.
  Player only; R reset restores pose, **not** parts (a derby car stays
  wrecked).
- **Verified** (scripted autopilot + telemetry, since removed): crate hit
  Δv 3.0 barely scratches; nose-to-nose fellow-car ram Δv 7.5 damages
  both cars with correct per-car zone attribution; wall head-on Δv 18.5
  wrecks the engine; limp mode measurable (13.6 m/s healthy launch vs
  ~3.4 m/s at power 0.30); steam and smoke bands toggle correctly and
  read white vs black; forced part deaths on all four cars: dead lights,
  shattered windshields, fenders flung to both sides, spoilers off,
  strut stubs left, fellow cars smoking — zero panics.

## Phase 8 — AI drivers ✅

**Goal**: the fellow cars fight back. Each AI writes its own `DriveInput`
every fixed step — the same intent component the keyboard fills — so the
AI drives the exact physics the player does, and everything downstream
(damage, limp mode, steam/smoke, skid marks) applies unchanged.

- **Free-for-all targeting**: each AI hunts the *nearest* other car,
  player or fellow — the derby spirit, and it exercises car-vs-car damage
  on every pair instead of gang-piling the player.
- **`ai.rs`**: `AiDriver` component (on non-player cars) holding a
  two-state machine. `Hunt`: full throttle, steer by bearing to the
  target in chassis-local space (`AI_STEER_GAIN` per radian, clamped);
  handbrake-swing when the target is far off the nose at speed
  (`AI_HANDBRAKE_BEARING`, `AI_HANDBRAKE_MIN_SPEED`) — the car whips
  around instead of arcing wide, painting skid marks. Predictive wall
  avoidance overrides the chase: the position `AI_WALL_LOOKAHEAD` seconds
  ahead along planar velocity leaving `ARENA_RADIUS − AI_WALL_MARGIN`
  switches the steering target to the arena center. `Retreat`: commanded
  drive with planar speed under `AI_STUCK_SPEED` for `AI_STUCK_TIME`
  (wall, block, locked bumpers) → timed reverse (`AI_RETREAT_TIME`)
  steering away from the hunt bearing, then hunt again — repeated cycles
  work a car free. One `info!` line per state transition.
- **Wiring**: `ai_drivers` chained in FixedUpdate between
  `read_player_input` and `vehicle_controller`; tunables in the config's
  "AI drivers" section.
- **Verified** (pty log + temp 1 Hz telemetry, since removed; player
  parked as bait): AI cars cross the arena at up to 13.4 m/s, converge,
  and ram — 70 impacts in ~90 s with front/rear/side zones on both cars
  of each pair (Δv up to 13.5); 79 stuck→retreat cycles all resolved,
  no permanent wall-camping (max radius 32.5 m of 44 available); the
  parked player got mobbed to 7% engine; wrecked AI cars limp and drag
  black smoke; spirograph skid arcs across the floor; zero panics.
  Emergent pacing note: after converging, the field settles into a
  low-speed scrum of shove–retreat–ram cycles (Δv 2–6) — thematically
  right for a derby; `DAMAGE_PER_DELTA_V` and `AI_RETREAT_TIME` are the
  knobs if rounds should last longer.

## Phase 9 — Arena remodel & damage juice ✅

**Goal**: the flat bowl becomes a derby course — edge spawns, a center
pillar, launch ramps, real potholes — and the damage system gets juice:
progressively dimming headlights and spark bursts at every impact.

- **Sculpted floor** (`arena.rs`): the flat cylinder is replaced by a
  generated polar-grid disc mesh (center fan + rings, `FLOOR_MESH_*`),
  flat except inside the six `POTHOLES` circles where vertices dip by a
  rim-smooth cosine bowl (`POTHOLE_RADIUS` 2 m, `POTHOLE_DEPTH` 0.15 m).
  The *same mesh* feeds `Collider::trimesh_from_mesh`, so physics and
  visuals can't disagree; vertex colors darken the bowls in proportion
  to depth so the dips read on the uniform floor material. The raycast
  suspension handles them natively.
- **Center pillar**: static concrete cylinder at the origin
  (`PILLAR_RADIUS` 3, `PILLAR_HEIGHT` 6); hits register through the
  existing contact readback.
- **Pinwheel ramps**: four wedge prisms (`RAMP_*`, ~11° incline) on the
  radius-19 ring, facing tangentially with the same handedness — hit one
  at speed and fly an arc around the pillar; the tall back face is a
  barrier from the wrong side, like a real stunt ramp. Mesh via Bevy's
  `Extrusion<Triangle2d>`, collider via `Collider::convex_hull`.
- **Edge spawns**: all four cars start on the `SPAWN_RING_RADIUS` 40
  ring, 90° apart at the diagonal angles (clear of the on-axis ramps),
  facing the center; R resets the player to its grid slot
  (`spawn_transform`). Obstacle layouts re-scattered clear of the new
  furniture.
- **Dimming headlights** (`damage.rs`): each headlight owns a material
  instance; on damage change its emissive scales by health² and the lens
  color grays out — dark at 0 (the old binary dead-material swap and
  `PartMaterials::light_dead` are gone).
- **Impact spark bursts** (`damage.rs` → `effects.rs`): every registered
  impact writes an `ImpactBurst` message (one per manifold — a car-vs-car
  hit is one shower, not two); a transient one-shot hanabi emitter spawns
  at the contact point — 10 sparks for hits removing under
  `BURST_HEAVY_DAMAGE` (5%) part health, 36 for harder ones — hot
  white-orange, gravity-pulled, gone in under a second, reaped by timer.
- **Physics debug gizmos removed**: the trimesh floor made
  `PhysicsDebugPlugin` draw every triangle; the feel is settled, so the
  plugin is gone per its own comment.
- **Skid marks on slopes**: marks now stamp at the contact's actual
  height and pitch along the segment, so strips lie on ramps and pothole
  walls instead of floating or knifing through.
- **Verified** (scripted waypoint autopilot + telemetry, since removed):
  pothole crossings dip the chassis (y 0.80 → 0.73/0.75 in telemetry at
  two different potholes); a ramp run climbs (y 1.27) and goes airborne;
  a pillar ram registers Δv 14–17 and wrecks the engine; the parked
  lights comparison shows one bright and one near-dead headlight on the
  same car; burst tiers both fire (light/heavy in the log, sparks
  visible in screenshots); a live AI run produced 41 impacts in 40 s
  with an AI car launching off a ramp mid-chase, zero panics.

## Phase 10 — Front ram bar & soft prop impacts ✅

**Goal**: ramming was all downside — a frontal hit put 0.6 of the damage
into your own engine, so there was no reward for playing aggressive. And
every prop hit like a wall regardless of whether it was standing still,
because a contact's impulse is Galilean-invariant: a car at 20 m/s into a
parked 40 kg ball produces the same impulse as a 20 m/s ball into a parked
car, so shoving a ball threw a heavy spark shower and dented the engine.

- **Ram bar** (`damage.rs`, `vehicle.rs`, `config.rs`): a new destructible
  `CarPart::Shield` — a steel bar bolted proud of the nose
  (`SHIELD_OFFSET`, wider than the chassis) with two permanent strut
  stubs, on every car. A front-zone hit now: (1) is absorbed by
  `SHIELD_ABSORB * shield_health` before reaching the engine/lights/glass
  (0.75 at full health), (2) wears the bar by `SHIELD_WEAR` times the
  *raw* impact (absorbing it is what destroys it — four to six good rams
  tear it off), and (3) deals the *other* car `1.0 + RAM_DAMAGE_BONUS *
  shield_health` damage if the hit landed on their front — up to 1.6× at
  full bar health. A dented bar (≤ 50%) swaps to a scraped-steel material;
  at 0 it tears off via the existing `detach()` path (cartwheels forward
  off the nose, per the fling formula) leaving the strut stubs behind.
  Zone detection (`Zone::of`) now checks front/rear before the roof, so a
  nose-on hit that lands high on the front face still routes through the
  bar instead of silently becoming a roof hit.
- **Soft prop impacts** (`damage.rs`, `obstacles.rs`): a new `SoftProp`
  marker (on the pushable balls and on any part's own torn-off debris)
  makes `apply_impact_damage` scale a contact's Δv by the prop's own
  speed *before* the damage threshold — `PROP_SOFTNESS_MIN` (0.15) at
  rest, ramping to full strength by `PROP_SOFTNESS_FULL_SPEED` (10 m/s).
  Impulse magnitude alone can't distinguish "parked" from "punted" (it's
  the same physics either way), so this is the only fix that works. Balls
  also lost their springback: `BALL_RESTITUTION` down to 0.05 with a
  `CoefficientCombine::Min` rule (avian priority Max > Multiply > Min >
  GeometricMean > Average, so it overrides the chassis's default
  `Average`), plus slicker friction and real linear/angular damping so a
  punted ball rolls and settles instead of ricocheting or coasting
  forever.
- **HUD**: a sixth damage-panel row, `ram bar`, alongside the existing
  five.
- **Verified** (live 4-car AI free-for-all, ~75 s, no player input needed
  — the AI hunts autonomously; `caffeinate`-backed pty log, screenshots):
  77 impacts logged, zero panics/errors. Two prop hits recorded at `soft
  0.82` / `0.80` (a moving-but-not-full-speed ball), confirming the
  softness scale is live; the two-observed-hit sample is a consequence of
  AI-only driving rarely leaving a ball exactly at rest or at exactly
  10 m/s, not a gap in the mechanism (the `IMPACT_MIN_DELTA_V` threshold
  is what makes a truly parked hit invisible — no log line at all — which
  matches a full-throttle test against a stationary ball producing zero
  new `impact:` lines). 15 separate `bar 0.00` (torn-off) lines over the
  run. Traced one ram-bonus pair directly: `car 163v0 side Δv 1.8 ram
  ×1.53` alongside `car 207v0 front Δv 1.8` at the same instant — car 207
  rammed nose-first with `bar 0.87`, and `1.0 + 0.6×0.87 ≈ 1.52` matches
  the logged `×1.53` exactly. HUD screenshot confirms the new `ram bar`
  row renders and colors correctly (a sustained-combat player car showed
  `ram bar 6%` in red).

## Phase 11 — Breakable crates & power-ups ✅

**Goal**: the four wooden crates were cosmetically identical to the heavier
concrete blocks — hit-and-fly, nothing more. Make them matter: hit one hard
enough and it shatters, dropping a floating power-up. Any car (player or AI)
that drives through collects it. Three kinds so far, designed so a fourth
is a one-line addition: a partial repair, a full repair to the worst part,
and temporary damage immunity.

- **Breaking** (`powerups.rs::break_crates`, `FixedUpdate` after
  `apply_impact_damage`): walks each `LootCrate`'s own edges in avian's
  contact graph (`Collisions::collisions_with`), converting impulse through
  the *crate's own* 50 kg mass — a light box ends up at nearly the puncher's
  speed, so `CRATE_BREAK_DELTA_V` (8.0) reads directly as "hit at ~29 km/h".
  Walking crates → graph rather than pairs → crates makes double-breaking
  impossible by construction (one visit per crate per tick). A broken crate
  despawns, bursts into 8 octant shards (`shatter`, `damage::detach`'s fling
  recipe aimed radially) tagged `SoftProp` so a car can't wreck itself on
  its own wreckage, throws an extra heavy `ImpactBurst`, and drops a pickup
  at its own center (not the contact point — a crate crushed against a wall
  has contacts inside the panel).
- **Pickups** (`powerups.rs`): a bobbing, spinning cube colored by kind,
  collected by a plain nearest-car-within-radius distance check — not a
  `Sensor` collider, since avian's `SpatialQueryFilter` has no sensor
  exemption and the suspension raycast (`vehicle_controller`) would ride a
  car up onto one. `PowerUpKind::TABLE` is a weighted drop table (5:3:2 —
  patch : full repair : immunity); `DropRng` is a hand-rolled xorshift64
  resource, following 008-colony's `WanderRng` precedent rather than adding
  a `rand` dependency neither bevy nor bevy_hanabi re-exports.
- **Part-respawn machinery** (`vehicle.rs`, `powerups.rs`): the hard part —
  a repair that lifts a destroyed part's health off zero must rebuild the
  visual `sync_car_parts` despawned/detached, or health and model desync.
  `CarAssets` (mesh/material handles) is promoted from a startup-local
  struct to a `Resource` so it survives past spawn time; a new `CarPaint`
  component holds each car's paint handle explicitly (not read back off the
  chassis's own material, which only coincidentally matches today).
  `respawn_missing_parts` rebuilds whichever of windshield/fender/spoiler/
  ram bar is both healthy and absent, guarded by the chassis's actual
  `Children` so a repair to a merely-dented part can't bolt a second copy
  on. `collect_powerups.before(sync_car_parts)` is load-bearing, not
  cosmetic: it forces the command flush that makes new children — and the
  `Damage` change tick — visible to `sync_car_parts` the same frame, so a
  torn-off ram bar's crush pose and a dead headlight's emissive both
  correct instantly on repair.
- **Immunity** (`damage.rs`): `Immunity(Timer)` + a permanent hidden
  `ImmunityBubble` child, toggled the same way engine steam/smoke is.
  `apply_impact_damage` snapshots `Has<Immunity>` into each hit before
  mutating anything, then skips only the immune car's own damage
  application — the other car's iteration (already reading its ram bonus
  from the pre-mutation snapshot) is untouched, so immunity blocks incoming
  damage without softening what the immune car deals out, and its own ram
  bar takes no wear from hits it shrugs off.
- **Crate respawn**: `CrateOrigin` tracks each crate's hand-placed spawn
  slot; `break_crates` announces a `CrateBroken { origin }` message;
  `obstacles::respawn_crates` (no new entities — a `Local<Vec<(Vec3,
  Timer)>>`, the same pattern the HUD's accel smoothing already uses) ticks
  a `CRATE_RESPAWN_DELAY` (20 s) timer per broken slot and rebuilds the
  crate there via the shared `spawn_crate` helper.
- **HUD**: a seventh damage-panel row, an immunity countdown (blank/
  transparent when not immune, cyan with seconds remaining otherwise); the
  match arms now yield a `Color` per row instead of a health fed through
  `health_color` once at the end.
- **Verified** (live 4-car AI free-for-all, `caffeinate`-backed pty log +
  screenshots, no player input needed — the AI drives through the crate
  cluster and each other's pickups on its own): across four separate runs
  (~90–100 s each), crates broke 3–4 times per run at Δv 8.1–27.6 (all
  cleanly over the 8.0 threshold — no false triggers from shoving or
  resting), each producing exactly one drop and one spark burst; the same
  fixed-seed `DropRng`, combined with the sim's otherwise-deterministic
  physics/AI, reproduced an identical break sequence and drop pattern
  across the first two runs — an independent Python replay of the
  xorshift64 sequence against `PowerUpKind::TABLE`'s weights confirmed the
  roll logic itself is correct (patch/part/immunity all reachable; the
  observed early runs just hadn't drawn a patch yet). Every collection
  line paired with a health change in the next `impact:` line for that
  car — a `repair part` visibly reset a torn-off ram bar to ~1.00. The key
  correctness case landed directly in the log: `car 107v0 collected
  immunity` at T, then a `front Δv 2.4 m/s — IMMUNE` line for the same
  car 4.1 s later (within the 5 s window) with damage fully skipped; a
  later hit on the same car well past the 5 s mark applied full normal
  damage, confirming `tick_immunity` correctly expires the component
  rather than leaving it stuck. Zero panics or errors across all runs
  (~300 combined impacts). HUD screenshot confirms all seven rows: six
  full health rows plus a correctly blank/transparent seventh (immunity)
  row when no `Immunity` is present.

## Phase 12 — AI perception & tactics ✅

**Goal**: the Phase 8 AI hunted the nearest car blindly — no terrain
awareness (its wall-avoidance override literally steered at the center
pillar), no defense, and no sense of its own health. Give the fellow cars
a vision cone with real commitment, closing-geometry threat prediction,
a nose-first defense that exploits the ram bar's own damage math, forward
obstacle sensing, and a self-preservation break-off.

- **`ai.rs` rewrite — five states**: `Hunt` (lead-pursuit at the
  committed target, so contact lands nose-first), `Brace` (rotate to meet
  an incoming threat nose-first while the ram bar and health can afford
  it), `Evade` (steer away, brake if head-on, when it can't rotate in
  time), `Recover` (break off to a power-up or open space once badly
  damaged), `Retreat` (unchanged from Phase 8 — stuck detection, timed
  reverse). Priority highest to lowest: `Retreat` > `Brace`/`Evade` >
  `Recover` > `Hunt`. Perception is never cached in `AiDriver` — only
  the committed `target`, timers, and a per-car xorshift64 stream are;
  everything else (threats, obstacles, the wall breach) is recomputed
  from a fresh per-tick `CarView` snapshot of every car.
- **Cone targeting + commitment**: a 120° forward cone scored by nose
  alignment + closeness + how wounded the candidate is (immune targets
  penalized — hits do nothing to them and they still deal full damage
  back), falling back to a uniform random pick among cars within
  `AI_TARGET_DROP_RANGE` when the cone is empty. A committed target holds
  for `AI_RETARGET_TIME` (jittered) instead of recomputing "nearest"
  every tick.
- **Threat prediction**: per-tick closing-geometry analysis against every
  other car (time to closest approach × miss distance × closing speed,
  multiplicatively — all three have to hold for a threat to count),
  Schmitt-triggered entry/exit so it doesn't flicker. `Brace` is chosen
  over `Evade` when the achievable yaw rate (derived from
  `vehicle_controller`'s own speed-limited steering lock) can rotate the
  nose onto the threat's lead point in time and the ram bar/health can
  still afford the trade.
- **Static obstacle awareness**: three horizontal whiskers
  (`SpatialQuery::cast_ray_predicate`) from the nose, filtered to static
  scenery and anything too heavy to shove (`AI_AVOID_MIN_MASS`, between
  the 50 kg crate and the 500 kg block — so the AI still drives through
  loot crates) and to non-drivable surfaces (`hit.normal.y <
  GROUND_NORMAL_MIN`, which reads a ramp's sloped face as clear while
  still avoiding its back/side faces, so ramp jumps survive). Hard
  override (steer + brake) inside the braking distance, a bounded soft
  steering nudge otherwise. The predictive wall escape no longer steers
  toward `Vec3::ZERO` — the Phase 8 bug that aimed the "safe" direction
  straight at the r=3 center pillar — but along a blend of the inward
  radial and the wall tangent, so cars peel off the wall instead of
  grinding it or orbiting it.
- **Self-preservation**: below `AI_RECOVER_CONDITION` (a weighted health
  aggregate — `Damage::condition()`, weighted toward the engine and ram
  bar, the two parts that change what the car can do and what a hit
  costs), the AI breaks off to the nearest power-up in range or an
  inverse-square repulsion point away from the field, for up to
  `AI_RECOVER_TIME` with a cooldown before it can re-enter, so a
  permanently-wrecked car doesn't hide for the rest of the round.
- **Arbitration**: hard constraints (a measured obstacle hit, then a
  predicted wall breach) override steering outright; everything else
  blends into one `goal_bearing` — full weighted-vector blending across
  every influence was rejected because bearings are angles, and averaging
  two escape angles either side of an obstacle averages to "drive
  straight into it." Throttle lifts off to turn tighter (shares the tire
  friction circle with lateral force) and brakes only into a genuinely
  head-on threat (killing the Δv it would deal); the handbrake swing is
  now a pure function of the final steer bearing, so every state gets it
  for free instead of only `Hunt`.
- **`vehicle_controller`** now honors `DriveInput::drive`'s magnitude
  (`input.drive.min(1.0)` / `.max(-1.0)`) instead of quantizing every
  command to ±1.0 — needed for the AI's partial-throttle turns; the
  keyboard still always writes exactly ±1.0, so player feel is unchanged.
- **A genuine bug caught during verification, not just designed around**:
  the first pass eagerly dropped a target the instant it read
  `> AI_TARGET_DROP_RANGE` away, then let the random fallback re-pick
  from the same out-of-range pool — which immediately failed the same
  check next tick. Every car spawns at least 56 m from every other car
  (`SPAWN_RING_RADIUS` 40, 90° apart), so this fired on frame one of
  every run, and recurred any time all rivals were briefly out of range
  at once (observed: two cars wedged at the wall while a third orbited
  just past 45 m of both) — an every-tick "(random)" retarget storm, 1302
  re-picks in one 100 s run before the fix. Fixed by only letting
  "too far" force a drop when a *nearer* alternative actually exists
  (`anyone_in_range`); confirmed by the same run afterward settling to
  the intended ~1 retarget per 2.7 s per car, 89% resolved in-cone.
- **Verified** (pty log, `caffeinate`-backed, player parked as bait,
  ~110 s, same free-for-all harness as Phase 8): zero panics/errors.
  51 impacts, 82% landing in the front zone (up sharply from Phase 8's
  unweighted mix of front/rear/side) and 29% carrying an actual ram
  bonus (`ram ×1.x`, i.e. landed while the attacker's bar was still
  standing) — the direct signature of lead pursuit and `Brace` working.
  Both defensive states fired within the first 15 s (`-> brace` at t+5s,
  `-> evade` at t+11s, `sev` 0.36–0.37 at entry, matching the tuned
  `AI_THREAT_ENTER`); `-> recover` fired twice, each on a car whose
  condition had genuinely dropped under `AI_RECOVER_CONDITION`.
  `stuck, retreating` dropped to 28 over 110 s versus Phase 8's 79 over
  ~90 s (roughly a 70% drop in rate) — obstacle avoidance removing most
  of the pillar/block ramming that used to force a retreat. Radius
  telemetry showed no wall-camping (heaviest bucket was 0–20 m, near the
  chase's natural center; only a thin tail near the 44 m wall-margin, no
  parked cluster at either the wall or the pillar). Obstacle whiskers
  fired 31 times with real hit distances logged. Determinism holds
  path-for-path within a run (fixed per-car RNG streams); screenshot
  capture failed this session (blank frames, display asleep despite
  `caffeinate` — a known environment flakiness) so this pass is
  log-verified only, no visual confirmation.

## Phase 13 — Car model extraction ✅

**Goal**: hand the car's look to an artist without touching Rust. Every
visual part (chassis, wheels + spin stripes, headlights, fenders,
windshield, spoiler + struts, ram bar + struts) now comes from
`assets/models/derby-car.glb` instead of procedural primitives; colliders,
suspension raycasts, and the immunity bubble stay in code (see
`docs/car-model.md`, the artist-facing contract).

- **Exporter** (`export.rs`, `cargo run --example 009-derby --
  export-model [path] [--force]`): hand-builds a GLB — no `App`, `Mesh`'s
  primitive builders run standalone — from the same `config.rs`
  `*_SIZE`/`*_OFFSET` constants the old procedural spawn used. First
  direct use of `gltf-json` in this repo (already resolved transitively
  via `bevy_gltf`); ships `KHR_materials_emissive_strength` so the
  headlights' HDR emissive survives the round trip. Refuses to overwrite
  an existing file without `--force`.
- **Loader** (`model.rs`): reads the glTF node graph directly
  (`Gltf::named_nodes`, `GltfNode`, `GltfMesh`) into a `CarRig` resource
  rather than spawning a `SceneRoot` — keeps the game's existing flat
  entity hierarchy, per-car paint, and per-headlight material instances
  instead of re-deriving them from a spawned scene. `CarRig` replaces
  `CarAssets` as the seam `vehicle::spawn_car`, `powerups::
  respawn_missing_parts`, and `damage::detach` all read from; `CarAssets`
  now only holds the immunity bubble. A part's debris collider is sized
  from its *mesh's own bounding box*, not a constant, so a reshaped fender
  gets a matching debris body for free.
- **Node contract**: names exist only to stay unique in the file's flat
  `named_nodes` map — which side a part is actually on (damage
  attribution) and whether a wheel steers are both re-derived from the
  node's own translation, not parsed from its name. A wheel's one child
  node is its spin stripe, found positionally rather than by name (four
  nodes all named "Stripe" would collide).
- **Async spawn**: the glTF loads asynchronously, so `spawn_vehicle` moved
  from `Startup` to `Update`, gated `run_if(resource_added::<CarRig>)`
  right after `build_car_rig` (itself gated `not(resource_exists)` so it
  stops polling once the rig is built — an early version of this omission
  silently rebuilt the whole rig every frame forever). Every system that
  assumed a `Player` car already existed (`read_player_input`,
  `follow_camera`, both HUD updates) switched from `Single` to
  `Option<Single>`; `sync_car_parts`/`collect_powerups` (which read
  `Res<CarRig>`/`Res<PartMaterials>` unconditionally — `Res<T>` validates
  before a system body even runs) are gated `run_if(resource_exists)`
  instead, since there's nothing for either to do before the first car
  spawns anyway.
- **Verified**: exporter output inspected byte-for-byte (20 nodes/10
  meshes/6 materials, POSITION min/max present, sRGB→linear base colors,
  wheel node transforms matching `WHEEL_MOUNTS`/`wheel_axis_align`
  exactly, buffer byte length matching the BIN chunk); two full launches
  with pty logging show one `derby-car.glb loaded` line each (not a
  per-frame flood), zero panics, and normal play — AI hunt/brace/evade,
  front-zone ram-bar wear, headlight/windshield/fender damage — all
  reading correctly against the model-driven cars. Screenshot capture
  came back blank (known environment flakiness, same as Phase 12), so
  this pass is log- and byte-level-verified, not visually confirmed.

## Phase 14 — Match structure ✅

**Goal**: everything through Phase 13 was a sandbox — cars never died,
nothing ended, and there was no way to tell four identically-shaped cars
apart except by paint. Turn it into a game: driver identity, a knock-out
rule, and a round loop with a countdown, a winner, and a restart.

- **`game.rs` (new)** — the whole match layer. `MatchState { Loading,
  Countdown, Fighting, Over }` (Bevy `States`). `Loading` waits on the
  async car model; `Countdown` spawns a fresh grid and holds every
  `DriveInput` at zero for `COUNTDOWN_SECS` while `vehicle_controller`
  still runs (so suspension settles the cars before "GO!"); `Fighting` is
  the derby itself; `Over` shows the results until `R` starts a new
  `Countdown`.
- **`Driver`** (name + paint color) is inserted on every chassis at spawn
  from a new `config::DRIVERS` table (slot 0 is always the player),
  replacing the old `vehicle::FELLOW_CAR_COLORS` const and the
  hardcoded player paint. Nameplates and the results panel read identity
  from this component alone — neither branches on `vehicle::Player`.
- **Knock-outs**: `game::check_wrecks` (`FixedUpdate`, after
  `damage::apply_impact_damage`) watches `Damage::condition()` — the same
  weighted health the nameplate bar shows — and knocks out anyone at or
  below `WRECK_CONDITION` (0.10). A KO inserts `Wrecked { place, at }`
  (place counted down from how many cars were still alive, so
  simultaneous KOs still resolve to distinct standings), zeroes
  `DriveInput`, and strips `AiDriver` — the car keeps its `RigidBody` and
  suspension (still shovable, still smokes — `effects::
  update_engine_emitters` forces smoke on any `Wrecked` car regardless of
  its own engine health) but never drives again.
  `read_player_input`/`ai_drivers` both additionally filter
  `Without<Wrecked>` on top of the `MatchState::Fighting` schedule gate,
  since `Fighting` doesn't end the instant *one* car (possibly the
  player) goes down. `alive <= 1` moves the match to `Over`.
- **Camera hand-off**: `CameraTarget` (not `Player`) is what
  `camera::follow_camera` tracks now. `game::hand_off_camera` moves it
  off a freshly-wrecked target onto any survivor, ordered ahead of
  `follow_camera` so the swap lands the same frame — a dead player still
  gets to watch the rest of the fight instead of staring at their own
  hulk.
- **Restart is the same code path as the first match**: `OnEnter(
  Countdown)` runs `game::start_match` (despawns every `Vehicle`, every
  piece of match debris — a new `damage::MatchDebris` marker on
  torn-off parts, *not* the permanent arena balls, which are `SoftProp`
  too but must survive a restart — and every uncollected `PowerUp`) then
  chains straight into the existing `vehicle::spawn_vehicle`. First
  match and every `R`-triggered rematch are one path, not two.
- **UI (`ui.rs`)**: floating nameplates (`spawn_nameplates`/
  `sync_nameplates`) — name in the driver's paint color over a health bar
  tinted by the existing `health_color` ramp — projected with
  `camera.world_to_viewport`, the same pattern as 008-colony's
  `sync_pawn_labels`; a wrecked car's plate greys out and appends its
  final place (`NAME · P3`). A centered "3, 2, 1, GO!" readout during
  `Countdown`. A results panel built fresh `OnEnter(Over)` straight from
  the world (`Driver` + `Option<Wrecked>`, sorted by `Wrecked::place`) —
  survivor first, then everyone else in the order they went down — and
  despawned `OnExit(Over)` rather than kept and re-synced.
- **`K`** (new, `Fighting` only): instantly wrecks one live AI car —
  a dev/verification tool, not a gameplay feature, so an end-of-match
  session (results panel, camera hand-off, restart) can be exercised in
  seconds instead of grinding out a whole derby.
- **`main.rs`**: the Update system list grew past the system-tuple arity
  Rust's `IntoSystemConfigs` impls support for a single flat tuple (the
  same limit `008-colony` hit), so it's nested into sub-tuples grouped by
  concern (match flow / vehicle & damage sync / powerups & world /
  UI) rather than flattened further.
- **Verified**: `cargo check` clean. Launched under `caffeinate` +
  `script` with pty logging; screenshots confirm nameplates (name +
  color-matched health bar) tracking multiple simultaneous cars, the
  top-left speed HUD and top-right damage panel unaffected, ram-bar
  "TORN OFF" and engine smoke rendering correctly alongside the new UI.
  A completely hands-off AI-only session (no player input reached the
  window — synthetic keyboard events are still blocked by the
  unresolved Accessibility-permission gap noted in the `verify` skill)
  produced two real knock-outs from natural play: `KO: car 177v0
  wrecked, place 4` and `KO: car 223v0 wrecked, place 3`, both logged at
  the moment `condition()` crossed 0.10. Confirmed from the log, not just
  code review, that the wrecked car never again appears in its own `ai:
  car …` decision lines afterward (only as a target/impact partner for
  the cars still fighting) — `AiDriver` removal and the `Without<
  Wrecked>` gate hold up under real play, not just at a glance.

## Phase 15 — Arena & acceleration ✅

**Goal**: playing Phase 14's match layer surfaced two feel problems: the
50 m-radius arena was cramped for four cars plus a pillar, four ramps and
eleven props, and the launch was soft — quadratic aero drag made the last
third of the speed range crawl (0 → 95% of top speed took ~4.6 s).

- **Arena grown 50 → 65 m radius.** Object *sizes* stay put (pillar,
  ramps, crates/blocks/balls, pothole bowls) since they're car-scale and
  `ai.rs`'s avoidance tuning is written against them; only *positions and
  ring radii* scale by 1.3× — `WALL_SEGMENTS` (40→52) and
  `FLOOR_MESH_SECTORS` (256→320) grow with it to hold the wall's chord
  width and the floor mesh's tangential vertex spacing (so pothole bowls
  still resolve) constant. `RAMP_RING_RADIUS` 19→25, `SPAWN_RING_RADIUS`
  40→52. `arena.rs::POTHOLES` and `obstacles.rs::CRATES`/`BLOCKS`/`BALLS`
  scaled the same 1.3× plus a handful of new entries (2 potholes, 2
  crates, 1 block, 2 balls) so prop density doesn't thin out across 1.7×
  the floor area. The AI's arena-keyed ranges (`AI_VISION_RANGE`,
  `AI_TARGET_DROP_RANGE`, `AI_RECOVER_POWERUP_RANGE`, `AI_REFUGE_DISTANCE`)
  scale too, or the cars go blind in the new floor.
- **Quicker launch.** `ENGINE_FORCE` 2800 → 3200 N — parked just under the
  rear axle's static-load friction cap, so a standing launch is
  grip-limited rather than leaving grip on the table. Quadratic aero drag
  (`AERO_DRAG`) replaced with a quartic curve expressed as an explicit
  `TOP_SPEED` (21 m/s) and `DRAG_EXPONENT` (4): drag stays out of the way
  through the mid-range and only walls off hard right at `TOP_SPEED`,
  instead of fighting the engine from the get-go. Net: 0 → 95% of top
  speed in ~3 s over ~36 m, up from ~4.6 s.
- **Verified**: `cargo check` clean. A fresh AI-only session (`verify`
  skill, `caffeinate`/`script` pty logging) showed AI cruise speeds up to
  19.0 m/s in the per-car telemetry log (`ai: car … v 19.0 …`), consistent
  with the new 21 m/s ceiling. Screenshots confirm the wider floor reads
  smoothly (no visible wall-chord seams from the higher `WALL_SEGMENTS`)
  and skid marks/props render correctly at the new scale.

## Phase 16 — Juice ✅

**Goal**: impacts didn't *land* — a wall hit hard enough to wreck the
engine produced sparks and a HUD number, but no felt weight. Add
presentation-only feedback: camera screenshake on impact, and a
cinematic slow-motion beat for a genuine near miss.

- **Screenshake (`juice.rs`)**: `CameraShake { trauma }`, the standard
  trauma model (shake amplitude is `trauma²`, so small hits barely
  register and hits stack convincingly). Fed by the existing
  `damage::ImpactBurst` message — extended with a `delta_v` field so
  shake intensity and part damage agree on how hard a hit was — read by a
  second, independent `MessageReader` alongside `effects::
  spawn_impact_bursts`'s own. Applied as a **rotation-only** offset
  post-multiplied onto the camera transform after `camera::follow_camera`
  runs (`.after` ordering): three out-of-phase sines of elapsed
  **real** time (`SHAKE_FREQS`) drive pitch/yaw/roll, decayed on
  `Time<Real>` rather than the (possibly slowed) game clock so the shake
  stays crisp through slow-motion. Because `follow_camera` recomputes its
  rotation from scratch via `look_at` every frame, the shake can never
  feed back into the eased chase pose the way a positional kick would.
- **Near-miss slow-motion (`juice.rs`)**: a deterministic detector, not a
  vibe check. For the car the camera is watching (`game::CameraTarget`)
  against every other live car, each `FixedUpdate` tick projects closing
  geometry exactly the way `ai.rs::assess_threat` already does for its own
  `Brace`/`Evade` decisions (severity from predicted miss distance ×
  urgency × closing-speed fraction). An **approach record** opens once
  severity crosses `NEAR_MISS_ARM` and tracks peak severity, peak closing
  speed, minimum actual centre distance, and whether avian's contact
  graph (`Collisions::contains`, an exact test) ever saw the pair
  actually touch. The record resolves the tick the pair starts
  separating; it counts as an avoided near miss — and triggers
  slow-motion, subject to a cooldown — only if peak severity, minimum
  distance and peak closing speed all cleared their thresholds *and* no
  contact was ever registered. Every resolution is logged
  (`near-miss: … -> SLOWMO`/`no trigger`) regardless of outcome, so the
  thresholds can be tuned from the log rather than by feel.
- **The slow-motion effect** itself is a `Time<Virtual>` relative-speed
  dip (to `SLOWMO_SCALE`) with a short ease in/out, ticked on
  `Time<Real>`. This is the one lever that does everything at once:
  avian's fixed-timestep physics and bevy_hanabi's particle simulation
  (`Time<EffectSimulation>`) both derive from `Time<Virtual>`, so the sim
  and its smoke/sparks slow together with no extra wiring, while the
  fixed timestep itself — and so determinism — is untouched.
  `camera::follow_camera` reads the current relative speed directly (no
  new coupling to `juice`'s trigger logic) and pulls `CAMERA_BACK` in
  toward the car during the dip, which is what reads as *cinematic*
  rather than merely laggy. `juice::reset_slow_motion`
  (`OnExit(MatchState::Fighting)`) forces normal speed back so a KO that
  fired slow-motion can never carry it into the results screen.
- **Verified**: `cargo check` clean. An AI-only session logged real
  collisions correctly *suppressing* the trigger — e.g. a head-on hit at
  peak severity 0.85, minimum distance 2.7 m, closing 18.5 m/s resolved
  as `touched true -> no trigger`, confirming the exact-contact check
  overrides the geometric projection exactly as designed. A crate break
  (`Δv 16.7`, `Δv 8.7`) and several front-on car impacts up to
  `Δv 19.7 m/s` fed the screenshake path with realistic intensities.
  The positive trigger path was confirmed with the `NEAR_MISS_*`
  thresholds temporarily loosened (reverted after) to force the case in
  a short session rather than wait out the known attrition-stalemate
  tail (Phase 17's Balance note): a swerve resolved `touched false ->
  SLOWMO`, and the logged `relative_speed` walked the full envelope
  correctly — easing 1.0 → 0.35 over `SLOWMO_EASE`, holding at exactly
  `SLOWMO_SCALE` (0.35) for the hold, easing back 0.35 → 1.0, then
  `done, relative_speed -> 1.0`. A second near miss that resolved while
  the effect was still active was correctly held to `no trigger` by the
  cooldown, confirming the two triggers can't stack or retrigger the
  ease mid-flight.

## Phase 18 — Car classes: the Buggy ✅

**Goal**: every car was the same car — one 350 kg chassis, one engine, one
tire spec — so "heavy truck" read as *the* game rather than *a* choice. Add
a second class, the **Buggy**: smaller, quicker off the line, faster flat
out, tighter cornering, and noticeably more fragile.

- **`config::CarSpec`**: every number that makes a class *drive*, *look*,
  or *break* differently — mass, chassis/collider size, wheel geometry,
  engine/brake/reverse force, top speed, tire μ, steer lock/limit,
  `damage_scale`, and every visual-geometry size/offset export.rs bakes
  into the model — moved off bare globals into one struct, tabled per class
  in `CAR_SPECS: [CarSpec; 2]` (`CarClass::spec()`). What stayed global:
  feel shared by both classes regardless of size (tire slip clamps,
  handbrake scales, the drag exponent, steer smoothing, skid thresholds,
  every damage/shield threshold).
- **Suspension and tire stiffness are *derived*, not tabled twice**:
  `CarSpec::spring_rate`/`damping_rate`/`max_spring_force`/
  `tire_lat_stiffness` compute from `mass`/`mu_front` against three shared
  design-target constants (`SUSPENSION_FREQ_HZ` 2.0, `SUSPENSION_DAMPING_RATIO`
  0.68, `SUSPENSION_MAX_G` 7.0, `TIRE_SATURATION_SLIP` 0.31) — chosen to
  reproduce the Truck's old flat 14 000/1 500/6 000/3 000 N figures to
  within 2%, so a third class gets proportionally correct springs/tires for
  free instead of a fourth place to copy-paste numbers wrong.
- **Wheelbase and track width are derived from the model, not config**:
  `model::CarRig::wheelbase()`/`track_width()` read the wheel nodes' own
  translations (cached on `Vehicle` at spawn); the old flat `WHEELBASE`/
  `TRACK_WIDTH` constants are gone, so the steering model can never
  disagree with whatever an artist put in the `.glb`.
- **Two `.glb`s, one node contract**: `export.rs` is parameterized over
  `&CarSpec`; `model::CarRigs` loads and holds one `CarRig` per class
  (`resource_added::<CarRigs>` fires once both have landed). The Buggy's
  body (`assets/models/derby-buggy.glb`) is the Truck's proportions scaled
  ×0.83 (width/height) / ×0.81 (length) — a genuinely smaller shape, not a
  scale transform — keeping `docs/car-model.md`'s artist-editable seam
  identical across classes.
- **Fragility is mostly free, physically**: `damage::apply_impact_damage`
  now computes `Δv = impulse / mass` and its resulting `amount` **per car
  side** of a contact (each reading its own `CarSpec`), instead of once per
  manifold off a shared constant. The same collision hands the 230 kg
  Buggy ~1.52× the Δv it hands a 350 kg Truck for free; `damage_scale`
  (1.15 for the Buggy) stacks on top for ~1.75× overall. Emergent
  consequence worth naming: since impulse scales with the pair's *reduced*
  mass, a Truck takes *less* damage ramming a Buggy than ramming another
  Truck — the Buggy is a harasser, not a wrecking ball (the gap a future
  Truck rework would widen further).
- **A mixed field, and a stand-in for car selection**: `config::DRIVERS`
  gained a class per grid slot — Vera rides the Buggy, the rest stay
  Trucks — so a mixed field falls out of the existing spawn loop with no
  new plumbing. Until a real pre-match selection screen exists,
  `game::PlayerClass` (seeded from `DRIVERS[0]`) plus `game::
  cycle_player_class` (`C`, only during `Countdown`) lets the player cycle
  their own class: it despawns and respawns just the player's grid slot via
  a new `vehicle::spawn_grid_car(slot, class)` — the same per-slot spawn
  path `spawn_vehicle`'s loop now calls too — and resets the countdown
  timer so there's time to look at the new car. Nameplates, the countdown
  hint ("C — Truck"/"C — Buggy"), and the results panel all show the class
  alongside the driver name.
- **Verified**: `cargo check` clean. `export-model buggy` wrote
  `derby-buggy.glb` (20 nodes, 10 meshes, 6 materials); `export-model truck`
  and a no-argument call both correctly refused (existing-file guard and
  missing-class-argument error respectively). A live AI-only session
  confirmed both classes load (`models/derby-car.glb loaded`, `models/
  derby-buggy.glb loaded`, `all 2 car classes loaded`) and the mixed grid
  spawns and drives correctly (nameplate reads `You (Truck)` in a
  screenshot). `impact:` log lines for the same car-vs-car contact showed
  the predicted per-mass Δv split almost exactly — e.g. one collision
  logged `Δv 8.0` on one side against `Δv 5.3` on the other, an 8.0/5.3 =
  1.51× ratio against the Truck/Buggy mass ratio's predicted 350/230 =
  1.52× — confirming per-car Δv is reading each side's own `CarSpec::mass`
  rather than a shared constant.

## Phase 19 — Landing physics ✅

**Goal**: a clean ramp landing on the wheels read as a crash — the ram bar
would wear down and the engine would take real damage from nothing more
than gravity. A landing is the suspension's job; it should just animate the
chassis (a squat and rebound on the springs), not tear off parts.

- **Why it happened**: `SUSPENSION_MAX_G` (7×) capped the *combined*
  spring+damper force, so any descent whose damper term alone exceeded that
  cap bottomed the springs out — 6.3 m/s on four wheels, only 4.1 m/s on
  two. The pinwheel ramps launch a car nose-up, so it lands tail-first
  after rotating in the air, at ~6.2 m/s — past the two-wheel figure. Once
  the springs ran out of travel, the chassis collider itself absorbed the
  landing as one hard normal impulse, and `apply_impact_damage` read that
  impulse exactly like a car-vs-car hit: `Δv = impulse / mass`, no notion
  that "down" is different from "into a rival."
- **Suspension: the damper gets its own, much higher cap.** The spring
  stores energy (can fling the car) and keeps its tight 7 g clamp,
  unchanged. The damper only dissipates — zero the instant a wheel stops
  closing, and still the plain linear term (unclamped in practice) on
  rebound — so a higher cap on it can only ever soak a landing faster,
  never launch anything. It also gains a velocity-progressive term,
  `damping_rate * closing * (1 + closing / SUSPENSION_DAMPER_KNEE)`,
  capped at `CarSpec::max_damper_force()` (`SUSPENSION_DAMPER_MAX_G` 14×,
  derived from mass exactly like `max_spring_force`). Effect at ride
  speeds (~0.3-1.0 m/s): 12-40% more force, unnoticeable. Effect at a
  ramp's ~6.2 m/s: the cap roughly doubles (6 → 12 kN for the Truck),
  which is what lets the landing fit inside suspension travel instead of
  bottoming out. Absorbable descent rose from 6.3 → 9.3 m/s (four wheels)
  and 4.1 → 6.6 m/s (two) for the Truck; the Buggy scales the same way off
  its own (lighter) mass.
- **Damage: a contact is discounted by how much it pushes a car along its
  own up axis** — the suspension's job, not the ram bar's — computed
  per-car-side from `manifold.normal` before the Δv threshold, exactly the
  shape `prop_softness` already uses for punted-vs-parked balls:
  `landing_softness = 1 - LANDING_SOFTNESS * max(0, push_dir · chassis_up)`
  (`LANDING_SOFTNESS` 0.75). A wheels-down landing at Δv 6.2 drops to 1.6 —
  under `IMPACT_MIN_DELTA_V`, so no damage, no sparks, no shake, no log
  line. `max(0.0)` keeps this honest: a car landing on its *roof*, or lying
  *under* another car, is pushed the other way and gets none of the
  discount — still a real hit. Side-swipes, head-on wall hits, and every
  car-vs-car ram are pushed roughly perpendicular to up and are likewise
  unaffected.
- **A landing gets its own cosmetic feedback instead**: each wheel whose
  touchdown closing speed clears `LANDING_THUMP_MIN_SPEED` (2.5 m/s) fires
  a new `vehicle::WheelLanding` message — a dust puff at the contact point
  (`effects::spawn_landing_dust`, a grey-tan one-shot hanabi effect
  alongside the existing spark bursts) and a small screenshake contribution
  (`juice::shake_camera`, `SHAKE_PER_LANDING` 0.22 — about a third of an
  impact's, since up to four wheels can fire the same tick). Both follow
  the exact `ImpactBurst` message-in-`FixedUpdate`/read-in-`Update` pattern
  already in use. Deliberately no health cost of any kind.
- **Not changed**: ride height, cornering feel, and pothole response (the
  progressive damper term is only 10-15% at those speeds); `Zone::of` still
  has no separate "hit from underneath" case — a landing hard enough to
  survive the discount is a genuine slam, and letting the ram bar (the
  lowest part on the nose) take it is still defensible; crate-breaking
  (`powerups::break_crates`) computes its own Δv independently and is
  untouched.
- **Verified**: `cargo check` clean. An AI-only session with a temporary
  debug log on the touchdown site (reverted before finishing) recorded 37
  hard wheel landings, up to **9.3 m/s** closing speed on the Truck — past
  the predicted four-wheel absorption ceiling — cross-referenced
  automatically against every `impact:` line within 50 ms: 36 of 37
  produced no damage line at all (the discount put them under
  `IMPACT_MIN_DELTA_V`, so no health lost, no sparks, no log line, exactly
  as designed). The one exception coincided, to the millisecond, with a
  separate car actively ramming the landing car mid-pileup at the ramp's
  base (`land 0.62`, a genuinely mixed vertical+lateral hit) — a real hit,
  correctly only partially discounted, not a plain landing slipping
  through. Car-vs-car `impact:` lines throughout showed `land` at 0.95-1.00
  (negligible discount, as expected for roughly horizontal pushes) and
  Δv/damage figures consistent with Phase 18's baseline. No panics or
  errors in either run. Did not obtain a mid-air/dust screenshot this
  pass — the AI's chases stayed centered on car-vs-car fights more than
  ramp runs during the capture window, and log-level evidence was
  conclusive enough on its own; a screenshot is easy to grab in a future
  session if wanted.

## Phase 20 — Future (not this pass)

- **Pre-match car-selection screen**, replacing the `C`-to-cycle stand-in
  Phase 18 added: a proper UI before `Countdown` starts, presumably
  showing each class's stats (top speed, mass, fragility) rather than
  just a name.
- **Truck rework**: make the Truck interesting in its own right rather
  than "the default" now that the Buggy exists — more damage dealt on a
  ram, or more rigidness (a smaller `damage_scale`, currently 1.0 and
  untouched, or slower part wear).
- **Wheel angular dynamics**: per-wheel spin state, slip-ratio-based
  longitudinal forces, wheelspin/lockup, an open/locked differential —
  the full-sim path deliberately deferred; the friction circle delivers
  most of the feel without it.
- **Anti-roll bar** term if wall-hit landings ever look floppy.
- **Damage polish**: crumple offsets on the chassis mesh, part-specific
  debuffs (dead lights at night?), a wrecked-engine fire state.
- **Balance**: two cars with spent ram bars and near-zero engines can
  grind each other's cosmetic health down asymptotically rather than
  crossing `WRECK_CONDITION` — observed as a multi-hour stalemate in
  Phase 14 verification. Worth a tuning pass (lower the threshold, or
  give `Damage::condition()` a slow passive decay below some health
  floor) if it turns out to be common rather than a tail case.
