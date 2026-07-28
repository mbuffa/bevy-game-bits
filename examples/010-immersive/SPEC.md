# 010-immersive ("Immersive") — Phased spec: first-person immersive-sim prototype on TrenchBroom-authored Quake maps

**Status:** Phases 1-3 and 4a done and verified (2026-07-28), all on a hand-authored bootstrap map. Phase 4b (TrenchBroom authoring) and Phase 5 (BSP compile) need a human at TrenchBroom / ericw-tools respectively — see the "Environment notes" section at the bottom for exactly what's installed on this machine.

## Context

The goal is a small FPS/RPG (immersive-sim) prototype: roam a level authored in TrenchBroom, loaded via `bevy_trenchbroom`, with a first-person kinematic controller (`bevy_ahoy`) and the seeds of immersive-sim interaction (a door, a pickup, a "use" key).

Two decisions made before any code was written:
- **Quake 3 BSP is not supported anywhere in the Bevy/Rust ecosystem** (the only maintained BSP crate, `qbsp`, covers BSP29/BSP2/GoldSrc/Quake2/Qbism, not IBSP 46). We target Quake 1/2 via `bevy_trenchbroom` instead — same editor (TrenchBroom), different compile target.
- **`bevy_fps_controller` is stuck on Bevy 0.17.** We use `bevy_ahoy 0.1` instead, which matches our Bevy 0.18 + avian3d 0.6 stack exactly and is by the same author as Foxtrot, the reference project for this stack.

### Decisions already made
- Map format: Quake 2 (Valve220), `bevy_trenchbroom`'s default and most capable target.
- Iterate on `.map` loaded directly; a `.bsp` compile step (ericw-tools) comes in Phase 5 for baked lightmaps.
- `bevy_trenchbroom` 0.13 + `bevy_trenchbroom_avian` 0.13.0-rc.1 (the only published avian backend that targets avian `^0.6.0-rc.1`, matching our `avian3d = "0.6"`).
- Module shape follows 009-derby: flat modules, no plugin struct, everything wired inline in `main()`, tuning centralized in `config.rs`.
- The Phase 1 bootstrap map (`assets/maps/immersive/test.map`) is **hand-authored** (plain text, no TrenchBroom involved yet) so the loader/controller code could be verified before any GUI level design. TrenchBroom-authored replacement is Phase 4b.

## Architecture

```
Startup:  spawn_map -> SceneRoot("maps/immersive/test.map#Scene")
          spawn_light -> DirectionalLight (see "Known issues" below)
          bevy_trenchbroom parses brushes -> meshes + GenericMaterials
          SceneHooks per class: convex_collider (default_solid_scene_hooks)
                                 + smooth_by_default_angle
PostUpdate: TrenchBroomPhysicsPlugin adds colliders -> SceneCollidersReady
Observers:  On<Add, PlayerSpawn> -> CharacterController + Collider + camera
            (Phase 2: On<Add, FuncDoor> -> DoorState + RigidBody::Kinematic)
PreUpdate:  bevy_enhanced_input -> ahoy AccumulatedInput
FixedPostUpdate (AhoySystems::MoveCharacters, before PhysicsSystems::First):
            ahoy KCC: move_and_slide, stair step, coyote, crouch
Update:     ahoy camera<->CharacterLook sync
            (Phase 2: interact::update_focus -> fire_interact -> Interacted)
            (Phase 2: door::drive_doors, ui::update_prompt)
```

All tuning constants (map path, TB scale, player capsule, sun illuminance, ...)
live in `config.rs`.

## Known issues (found during Phase 1, unresolved)

**Real-time `PointLight` in this scene blows the whole frame out to solid
white, at any intensity.** Isolated via bisection:
- `light` intensity 300,000 lm and 40,000 lm both render pure white — intensity-independent, ruling out simple overexposure.
- No point light at all renders correctly (dim, from Bevy's default ambient only).
- Swapping the exact same light position/setup to a `DirectionalLight` renders correctly, fully lit, with shadows.
- Camera, meshes, and window diagnostics (logged via a temporary debug system) all looked completely normal in every case — active camera at the expected position, 3 meshes present, window a normal 2560×1440 physical size.

Root cause not found (candidate: a clustered-forward+ interaction specific to this tiny sealed room, or an avian/bevy_ahoy/bevy_trenchbroom interaction — not isolated further). **Workaround:** Phase 1 spawns a plain `DirectionalLight` directly in `main.rs` (`spawn_light`) instead of using a Quake `light` point entity. The bootstrap map has no `light` entity as a result. Revisit before Phase 5 (baked BSP lighting, which doesn't use a runtime `PointLight` at all and may sidestep this) or Phase 6 (if dynamic point lights are wanted for gameplay, e.g. flashlights/explosions).

Two other real bugs were found and fixed while building the bootstrap map (both were bugs in the hand-authored `.map` text, not in the libraries):
1. A single 6-plane brush is a **solid filled box** in Quake's brush model (intersection of half-spaces), not a hollow room — the first draft spawned the player embedded in solid rock. Fixed by building the room from 6 separate slab brushes (floor/ceiling/4 walls) bounding an open interior.
2. `bevy_trenchbroom` computes a face's plane normal as `(p3-p1) × (p2-p1)` (`brush.rs::BrushPlane::from_triangle`) — the *opposite* winding from the textbook `(p2-p1) × (p3-p1)` convention. Every hand-written face had this backwards, which produced negative/inverted collider volumes and crashed avian3d (`ColliderAabb` min/max assertion, then a negative-mass assertion) depending on brush shape. Fixed in the map generator by swapping the last two points of every face before emitting.

## Phase 1 — Map loads and you can walk on it ✅

**Goal:** `cargo run --example 010-immersive` opens a window showing a room, camera renders it correctly, and the player is caught by floor collision (doesn't fall through).

Done:
- `Cargo.toml`: `bevy_trenchbroom = "0.13"` (`bsp`, `physics-integration` features) + `bevy_trenchbroom_avian = "0.13.0-rc.1"` + `bevy_ahoy = "0.1"` (`default-features = false`) + `bevy_enhanced_input = "0.24"` (pinned, ahoy requires exactly `^0.24`). `cargo tree -i avian3d` / `-i bevy_enhanced_input` confirmed single resolved versions.
- `main.rs`, `config.rs`, `trenchbroom.rs`, `classes.rs` (`PlayerSpawn` only), `player.rs`, `input.rs`.
- 5 placeholder "dev texture" PNGs generated with PIL (checker/grid patterns, distinct colors per surface type) in `assets/textures/` — not sourced externally, self-generated to avoid any licensing/attribution question.
- `assets/maps/immersive/test.map`: hand-authored hollow 512×512×256-unit room (see "Known issues" for the two bugs found building it), one `player_spawn`.
- Cursor grab on click / release on Escape — this repo's first `CursorGrabMode` usage.

**Verified (2026-07-28):**
- `cargo check --example 010-immersive` clean, zero warnings.
- Runtime log clean: no `error!`/`warn!` from any crate across multiple full runs.
- Screenshot (`screenshots/010-immersive/260728-phase1-room.png`) shows a properly textured, lit room from the player's first-person perspective — dev-grid floor and walls, correct perspective, real shadows from the directional light.
- Player physically fell and settled: camera Y stabilized at **1.7365–1.7375 m** consistently across five independent runs (different light configs, different debug builds) — matches the expected resting eye height (`CharacterController::default().standing_view_height = 1.7`) and proves gravity + floor collision both work (spawned at 1.22 m, settled to a physically sensible resting height, never fell further).
- Window: physical size 2560×1440 @ 2x scale (normal); exactly one active camera; exactly 3 mesh entities (one per distinct texture: `dev_floor_a`, `dev_grid_128`, `dev_wall_a`).

Not yet verified: live WASD/mouse-look input (synthetic input is blocked on this Mac — no Accessibility grant — so this needs either a human driving it, or the Phase 4 in-process autopilot harness).

## Phase 2 — The door ✅

**Goal:** E on a `func_door` slides it open, waits, closes; the player can walk through the opening and cannot walk through the closed door.

Done:
- `Interactable` base class carrying a `prompt: String`; `FuncDoor` solid class (`angle`/`speed`/`lip`/`wait`, Quake semantics).
- `interact.rs`: `SpatialQuery::cast_ray` from the camera (same pattern as `009-derby/vehicle.rs:405-530`), `InteractionFocus` resource, `Interacted` event fired on `Fire<input::Interact>` (E key).
- `door.rs`: `setup_doors` observes `SceneCollidersReady` (not `On<Add, FuncDoor>` — the `ColliderAabb` doesn't exist yet at that point) and computes travel distance from the door's own collider AABB projected onto `angle`'s direction via the standard box support-function formula (`2·(hx·|dx| + hy·|dy| + hz·|dz|)`), minus `lip`. Explicitly inserts `RigidBody::Kinematic` — `insert_static_collider`'s `insert_if_new(RigidBody::Static)` won't overwrite, but doesn't give us kinematic either, so the door needs its own override. State machine: Closed → Opening → Open (holds for `wait` seconds) → Closing → Closed.
- Test door added to the bootstrap map: free-standing (not set into a wall gap — that needs the Phase 4 TrenchBroom-authored map), `angle 90 speed 2 lip 0.2 wait 3 prompt "Open door"`.

**Verified (2026-07-28):** synthetic key input is blocked on this Mac (same Accessibility restriction noted in the verify skill), so E couldn't be pressed live. Instead, a temporary debug system fired the `Interacted` event directly and logged door position every 60 frames; removed after verification. Log evidence from that run:
- `setup_doors` computed `travel=0.216m` from the door's actual collider AABB (a 16-TB-unit=0.406 m thick brush minus 0.2 m lip) — confirms the AABB-projection formula runs correctly on real scene data, not just in isolation.
- Door reached `open_pos` within one frame interval of the trigger (travel/speed ≈ 0.11s, well under a 60-frame/0.5s sampling window).
- Door held at `open_pos` across 5 consecutive 60-frame samples (~3s), matching `wait = 3`.
- Door returned to **exactly** `closed_pos = (0, 0, 0)` by the next sample — no drift, confirming `move_towards`'s exact-snap-on-arrival logic.

Not yet verified live: prompt visibility falloff at the 2.5 m boundary, and the closed door physically blocking the player (both need either a human driving the controls, or the Phase 4 autopilot harness — this phase's manual `Interacted` trigger bypassed the raycast/prompt path entirely by design, to isolate the door state machine from the input path).

## Phase 3 — The pickup ✅

**Goal:** E on an `item_pickup` despawns it and adds its `item` string to an on-screen inventory.

Done:
- `ItemPickup` point class (`base(Transform, Interactable)`, `item: String` property).
- `pickup.rs`: `spawn_visuals` (`On<Add, ItemPickup>`) adds a small emissive cube mesh + `Sensor` collider (no solid collision — pickups shouldn't block movement, but the sensor collider still lets `interact.rs`'s raycast find them); `collect_on_interact` (`On<Interacted>`) pushes `item` onto the `Inventory` resource and despawns the entity.
- `ui.rs`: top-left inventory line, updated only `if inventory.is_changed()`.
- Two pickups added to the bootstrap map (`keycard_blue`, `battery`).

**Verified (2026-07-28):** same synthetic-input constraint as Phase 2 — a temporary debug system fired `Interacted` at both pickup entities directly (removed after verification). Log evidence:
```
DEBUG: auto-collecting pickup 164v0
DEBUG: auto-collecting pickup 163v0
DEBUG: inventory = ["battery", "keycard_blue"], remaining pickup entities = 0
```
Both items collected, `Inventory` populated with both distinct values in trigger order, both entities fully despawned (query count went to 0) — matches the acceptance criteria exactly.

## Phase 4a — Dev tooling & verification harness ✅

**Goal:** the example proves itself without a human at the keyboard.

Done:
- `devtools.rs` behind env vars, gated once at startup in `main.rs` (a normal `cargo run` behaves exactly as before):
  - `IMMERSIVE_AUTOPILOT=1`: `autopilot_drive` walks `config::AUTOPILOT_SCRIPT` (forward → turn → forward → turn → hold), writing `AccumulatedInput.last_movement` (the same field ahoy's own WASD observer writes) and rotating the camera `Transform` directly (ahoy copies that into `CharacterLook`, which steers movement).
  - `IMMERSIVE_TELEMETRY=1`: `telemetry` logs position/grounded/interaction-focus every `config::TELEMETRY_INTERVAL` (0.25s).
  - `IMMERSIVE_SHOTS=1`: `take_screenshot` spawns a `Screenshot::primary_window()` + `save_to_disk`, saved to `screenshots/010-immersive/`.
  - `IMMERSIVE_GIZMOS` (avian `PhysicsDebugPlugin`) **not implemented** — descoped, lower value than the other three and none of the debugging this session needed it.

**Verified (2026-07-28)**, all three flags on together in one run:
- Telemetry showed the player walking continuously (`z` from `-0.2` to `-6.09` over ~4 real seconds, `grounded=true` on every single sample — no falling) then **stopping exactly at the wall boundary** and holding position for the rest of the run, even though the script's movement input was still nonzero during that window. This is a second, independent proof (beyond Phase 1's resting-height check) that wall collision works: the kinematic controller was physically blocked, not just scripted to stop.
- `IMMERSIVE_SHOTS` produced a real in-app screenshot (`devtools-autopilot.png`) showing the textured floor/wall grid and crosshair from mid-walk — and critically, **this route is immune to the macOS Space-switching problem** that blocked `screencapture -x` during Phase 2 (the terminal ended up full-screened on a different Space than the game window, and Accessibility isn't granted to fix it via AppleScript). In-app screenshots render and save inside the same process, with no dependency on window focus, Spaces, or OS-level capture permissions — this is now the preferred verification method for this example going forward.
- Zero `error!`/`warn!` lines across the run.

## Phase 4b — TrenchBroom-authored map (not started — needs a human)

**Goal:** replace the hand-authored bootstrap `test.map` with one built in TrenchBroom, testing `player_spawn`, `func_door` (set into an actual wall gap this time — the bootstrap door is free-standing), `item_pickup`, and a stairwell for `step_size`.

TrenchBroom is a GUI editor with no meaningful CLI/scripting surface, so this step is for you, not something to automate further from here. Concrete steps:
1. TrenchBroom.app is already installed and the game is already registered (`bevy_game_bits_immersive` — confirmed every run by the `Successfully wrote TrenchBroom game config` log line, and by `~/Library/Application Support/TrenchBroom/games/bevy_game_bits_immersive/` existing with a `.fgd` reflecting `classes.rs`). Just launch TrenchBroom.
2. **New Map** → game `bevy_game_bits_immersive` → format **Quake2 (Valve)**.
3. Build a sealed room, texture with the PNGs in `assets/textures/`, place one `player_spawn`, a `func_door` actually set into a wall gap, one or two `item_pickup`s.
4. Save as `assets/maps/immersive/test.map` (overwriting the bootstrap one — or save elsewhere and update `config::MAP_PATH`).
5. `cargo run --example 010-immersive` — no code changes needed, the loader doesn't care whether the `.map` came from TrenchBroom or a text generator.

## Phase 5 — BSP compile with baked lighting

**Goal:** the same map, compiled, loads with baked lightmaps + irradiance volumes.

- Install ericw-tools (not on PATH as of Phase 1 — `brew install ericw-tools` or the 2.0.0-alpha10 release).
- Switch `player_spawn` → `InfoPlayerStart` so qbsp's leak detection works; `GlobalAmbientLight::NONE`.
- **Critical:** use `-qbism` (Quake 2 BSP38). Plain Q1 BSP strips brushes unless compiled with `-wrbrushesonly`, and `convex_collider()` would find nothing — the player falls forever.
```
qbsp  -qbism -nosubdivide -nosoftware -path assets -notex assets/maps/immersive/test.map
light -wrnormals -extra4 -lightgrid -path assets \
      -bounce 8 -bouncecolorscale 1 -bouncestyled 1 -dirt 1 -phong 1 \
      assets/maps/immersive/test.bsp
```
- Revisit the Phase-1 point-light white-out here — baked BSP lights don't spawn a runtime `PointLight` at all under `LightingWorkflow::MapDynamicBspBaked` when loading a `.bsp`, so this may be moot for baked content even if never root-caused.
- LFS-track `.bsp` (`.gitattributes` already has the rule).

## Phase 6 — Immersive-sim seasoning (later)

Physical object pickup via ahoy's `pickup` feature (`avian_pickup`); `func_button` wired to a door through the built-in `Target`/`Targetable` base classes; animated lights; a LibreQuake `.bsp` as a stress test.

## Environment notes

- TrenchBroom.app is installed on this Mac (`~/Library/Application Support/TrenchBroom` exists) — the game config + FGD write on every `cargo run` (`bevy_trenchbroom::config::writing` info logs confirm success).
- ericw-tools is **not** installed — only needed starting Phase 5.
- Synthetic input (cliclick/osascript CGEvent) is blocked on this Mac (no Accessibility grant) — see `.claude/skills/verify/SKILL.md`. Phase 4's in-process autopilot is the way around this for future verification.
