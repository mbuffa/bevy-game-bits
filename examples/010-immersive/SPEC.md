# 010-immersive ("Immersive") — Phased spec: first-person immersive-sim prototype on TrenchBroom-authored Quake maps

**Status:** Phases 1-3, 4a, and 5 done and verified (2026-07-28) on the
hand-authored bootstrap map. **Phase 7 — ladder climbing — done and verified
(2026-09-08)** on a new generated warehouse map that replaced the bootstrap one.
Phase 4b (TrenchBroom authoring) remains and needs a human at the TrenchBroom
GUI — see "Environment notes" at the bottom.

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
Startup:  spawn_map -> SceneRoot("maps/immersive/warehouse.map#Scene")
          spawn_light -> DirectionalLight (see "Known issues" below)
          + GlobalAmbientLight resource (Phase 7)
          bevy_trenchbroom parses brushes -> meshes + GenericMaterials
          SceneHooks per class: convex_collider (default_solid_scene_hooks)
                                 + smooth_by_default_angle
PostUpdate: TrenchBroomPhysicsPlugin adds colliders -> SceneCollidersReady
Observers:  On<Add, PlayerSpawn> -> CharacterController + Collider + camera
            (Phase 2: On<SceneCollidersReady> -> DoorState + RigidBody::Kinematic)
            (Phase 7: On<SceneCollidersReady> -> Ladder + Sensor on the volume
                      + ladder mesh child carrying its own solid Collider)
            (Phase 7: On<Interacted> -> ladder::attach_on_interact — E mounts, or toggles off)
            (Phase 7: On<Start<Jump>> -> ladder::let_go_on_jump — a Space press while climbing detaches)
PreUpdate:  bevy_enhanced_input -> ahoy AccumulatedInput
RunFixedMainLoop:
   BeforeFixedMainLoop: (Phase 7) ladder::stash_input — once per frame:
                        take last_movement into Climbing::wish (ZERO if released),
                        discard jumped
FixedPostUpdate:
   AhoySystems::MoveCharacters:  ahoy KCC: move_and_slide, stair step, coyote
   after MoveCharacters, before PhysicsSystems::First:
                          (Phase 7) ladder::climb — absolute Transform + zero velocity;
                          the top dismount is a walked crest onto the deck, not a teleport
Update:     ahoy camera<->CharacterLook sync
            interact::update_focus -> fire_interact -> Interacted
            door::drive_doors, ui::update_prompt
PostUpdate: (Phase 7) ladder::turn_to_ladder — yaw ease onto the rungs, before Propagate
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
  - `IMMERSIVE_AUTOPILOT=1`: `autopilot_drive` walks `config::AUTOPILOT_SCRIPT` — per-leg `movement` / `yaw_rate` / `interact` / `jump` — writing `AccumulatedInput.last_movement` (the same field ahoy's own WASD observer writes), rotating the camera `Transform` directly (ahoy copies that into `CharacterLook`, which steers movement), **tapping the real `KeyCode::KeyE` in `ButtonInput`** for `config::AUTOPILOT_TAP_SECS` on `interact` legs (starting the frame `InteractionFocus` resolves, via a shared `Tap` helper), and **holding the real `KeyCode::Space` down for the whole of any `jump` leg** — so a run of consecutive `jump` legs is one continuous hold, i.e. Space pressed well before the leg that grabs the ladder. It runs in `PreUpdate` between `bevy::input::InputSystems` and `EnhancedInputSystems::Update` so the key writes are sampled the same frame. Pressing the *real* keys rather than triggering `Interacted` / writing `AccumulatedInput.jumped` directly is deliberate: it puts the whole binding → `Press` condition → `Fire<Interact>` → `fire_interact` path (E) and binding → ahoy `Jump` / `Start<Jump>` path (Space) under test — the only way the harness can catch the E action firing every frame instead of once per press, or a jump *pressed before* a grab wrongly knocking the player off a ladder (see Phase 7).
  - `IMMERSIVE_TELEMETRY=1`: `telemetry` logs position/grounded/interaction-focus every `config::TELEMETRY_INTERVAL` (0.25s).
  - `IMMERSIVE_SHOTS=1`: `take_screenshot` spawns a `Screenshot::primary_window()` + `save_to_disk`, saved to `screenshots/010-immersive/`.
  - `IMMERSIVE_GIZMOS` (avian `PhysicsDebugPlugin`) **not implemented** — descoped, lower value than the other three and none of the debugging this session needed it.

**Verified (2026-07-28)**, all three flags on together in one run:
- Telemetry showed the player walking continuously (`z` from `-0.2` to `-6.09` over ~4 real seconds, `grounded=true` on every single sample — no falling) then **stopping exactly at the wall boundary** and holding position for the rest of the run, even though the script's movement input was still nonzero during that window. This is a second, independent proof (beyond Phase 1's resting-height check) that wall collision works: the kinematic controller was physically blocked, not just scripted to stop.
- `IMMERSIVE_SHOTS` produced a real in-app screenshot (`devtools-autopilot.png`) showing the textured floor/wall grid and crosshair from mid-walk — and critically, **this route is immune to the macOS Space-switching problem** that blocked `screencapture -x` during Phase 2 (the terminal ended up full-screened on a different Space than the game window, and Accessibility isn't granted to fix it via AppleScript). In-app screenshots render and save inside the same process, with no dependency on window focus, Spaces, or OS-level capture permissions — this is now the preferred verification method for this example going forward.
- Zero `error!`/`warn!` lines across the run.

## Phase 4b — TrenchBroom-authored map (not started — needs a human)

> **Superseded for the runtime default by Phase 7's generated `warehouse.map`.**
> `test.map` / `test_bsp_source.map` / `test.bsp` are deleted; `config::MAP_PATH`
> is `maps/immersive/warehouse.map`. This phase is now "author a *hand-made*
> level in TrenchBroom" as a distinct exercise from the generator, whenever
> someone wants to sit at the GUI — the steps below still apply, just save to a
> new path and point `MAP_PATH` at it.

**Goal:** a TrenchBroom-built level testing `player_spawn`, `func_door` (set into an actual wall gap), `item_pickup`, a stairwell for `step_size`, and a `func_ladder`.

TrenchBroom is a GUI editor with no meaningful CLI/scripting surface, so this step is for you, not something to automate further from here. Concrete steps:
1. TrenchBroom.app is already installed and the game is already registered (`bevy_game_bits_immersive` — confirmed every run by the `Successfully wrote TrenchBroom game config` log line, and by `~/Library/Application Support/TrenchBroom/games/bevy_game_bits_immersive/` existing with a `.fgd` reflecting `classes.rs`). Just launch TrenchBroom.
2. **New Map** → game `bevy_game_bits_immersive` → format **Quake2 (Valve)**.
3. Build a sealed room, texture with the PNGs in `assets/textures/`, place one `player_spawn`, a `func_door` actually set into a wall gap, one or two `item_pickup`s.
4. Save as `assets/maps/immersive/test.map` (overwriting the bootstrap one — or save elsewhere and update `config::MAP_PATH`).
5. `cargo run --example 010-immersive` — no code changes needed, the loader doesn't care whether the `.map` came from TrenchBroom or a text generator.

## Phase 5 — BSP compile with baked lighting ✅

> Phase 7 renamed the two sources to `warehouse.map` /
> `warehouse_bsp_source.map` (→ `warehouse.bsp`) and the `Makefile` byproduct
> list to match; both sources are now generated by `tools/gen_map.py`. The
> `test.*` filenames below are historical.

**Goal:** the same map, compiled, loads with baked lightmaps + irradiance volumes.

ericw-tools isn't in Homebrew — it has to be built from source (no prebuilt release binary for this macOS/arch either, as of 2026-07-28). Build notes, in case this needs redoing:
- `git clone`'d ericw-tools does **not** fetch its submodules (`3rdparty/{fmt,jsoncpp,nanobench,pareto}`) automatically — `git submodule status` shows a leading `-` on each until you run `git submodule update --init --recursive`. Without this, `cmake` fails immediately with "does not contain a CMakeLists.txt file" for each one.
- `cmake .. -DCMAKE_PREFIX_PATH="$(brew --prefix embree);$(brew --prefix tbb)" -DCMAKE_BUILD_TYPE=Release -DDISABLE_DOCS=ON` — the `DISABLE_DOCS` flag matters: without it, `make` builds `qbsp`/`light`/`vis`/`bsputil` successfully (100%) and then fails on an unrelated Sphinx docs target (missing the `furo` HTML theme in the ambient Python env), making it look like the whole build failed when the tools were actually fine. The binaries land at `build/<tool>/<tool>` (e.g. `build/qbsp/qbsp`), not in a `build/bin/`.
- Built version: **2.0.0-alpha11** (SPEC originally said alpha10 — close enough, no issues).

**Two separate `.map` sources, not one** — this matters:
- `assets/maps/immersive/test.map` stays exactly as Phase 1-4 left it (no `light` entity — a live `PointLight` reintroduces the white-out bug on the dynamic-load path). This is still what `cargo run --example 010-immersive` loads by default.
- `assets/maps/immersive/test_bsp_source.map` is a **separate** generator output: the same room, plus a `light` entity and an `info_player_start` (qbsp's leak-fill occupant — the literal classname matters here, but our own `player_spawn` also has to be present alongside it or nothing spawns a camera and the screen goes solid black instead of white). This compiles to `assets/maps/immersive/test.bsp`. Toggle `config::MAP_PATH` between the two to switch which one the example loads.
- **Critical:** use `-qbism` (Quake 2 BSP38). Plain Q1 BSP strips brushes unless compiled with `-wrbrushesonly`, and `convex_collider()` would find nothing — the player falls forever.

`examples/010-immersive/Makefile` wraps the two commands (`make bsp`, or `make clean-bsp` to remove the `.bsp` + byproducts). ericw-tools isn't on PATH, so override on the command line: `make bsp QBSP=/path/to/qbsp LIGHT=/path/to/light`. Proper `make` dependency tracking — running `make bsp` again with nothing changed is a no-op. Equivalent to running directly:
```
qbsp  -qbism -nosubdivide -nosoftware -path assets -notex \
      assets/maps/immersive/test_bsp_source.map assets/maps/immersive/test.bsp
light -wrnormals -extra4 -lightgrid -path assets \
      -bounce 8 -bouncecolorscale 1 -bouncestyled 1 -dirt 1 -phong 1 \
      assets/maps/immersive/test.bsp
```
qbsp produces `test.log`/`test-light.log`/`test.prt`/`test.content.json`/`test.texinfo.json` as compile byproducts alongside the `.bsp` — the Makefile deletes these automatically after each build; not checked in (only `test.bsp` matters at runtime, and it's already LFS-tracked via `.gitattributes`).

**Verified (2026-07-28):** qbsp reported `1 player-occupiable leaves` (sealed, no leak) and no warnings; `light` completed with a populated lightgrid (`2048 grid nodes`) and no errors. Switched `config::MAP_PATH` to `test.bsp` temporarily and ran with `IMMERSIVE_SHOTS=1` (and separately with the full autopilot+telemetry harness):
- **The Phase 1 point-light white-out does not occur.** Both screenshots (`screenshots/010-immersive/260728-phase5-bsp-baked-wall.png`, close on the door and again on a far wall after autopilot walked into it) show a real lighting gradient across the grid texture — brighter near the light, falling off with distance — not flat ambient-only shading and not a white-out. This confirms the hypothesis from Phase 1's "Known issues": baked BSP lighting genuinely sidesteps the bug, since `LightingWorkflow::MapDynamicBspBaked` doesn't spawn a runtime `PointLight` at all when loading a `.bsp`.
- Movement/collision on BSP-loaded geometry matched the `.map` path almost exactly: same autopilot script produced the same walk-forward-then-stop-at-wall telemetry pattern, `grounded=true` throughout, halting at the same `z≈-6.087` wall boundary — the physics pipeline (convex colliders from `Brushes::Bsp`) behaves identically to the `.map` (`Brushes::Owned`) path.
- `config::MAP_PATH` reverted back to `test.map` afterward — the `.bsp` path is proven working, but isn't the default; switch it manually to demonstrate baked lighting.

Not done: `GlobalAmbientLight::NONE` (Bevy's default ambient wasn't zeroed for this test — the baked gradient was still clearly visible over it, but a cleaner comparison would zero it) and switching the primary `player_spawn` to rely solely on `InfoPlayerStart` (both coexist in `test_bsp_source.map` right now, which works but is slightly redundant).

## Phase 6 — Immersive-sim seasoning (later)

Physical object pickup via ahoy's `pickup` feature (`avian_pickup`); `func_button` wired to a door through the built-in `Target`/`Targetable` base classes; animated lights; a LibreQuake `.bsp` as a stress test.

## Phase 7 — Ladder climbing ✅

**Goal:** a Half-Life / Deus Ex-style ladder: aim at it and press **E** to
lock on, forward climbs and back descends, mouse-look stays free, no
third-person cutscene and no teleport. E is the *only* way on — there's no
automatic "walk into it" grab, so a single code path mounts you square whether
you're at the foot of the ladder or looking down at its head from the deck,
and there's no mirrored "press S facing away to descend" that fires when you
just want to back up near a ladder. Press E again (or Space) to let go.

### The map is now a warehouse, generated

`assets/maps/immersive/test.map` (+ `test_bsp_source.map`, `test.bsp`) are
**deleted**, replaced by `warehouse.map` / `warehouse_bsp_source.map`, both
emitted by **`tools/gen_map.py`** (stdlib Python, outside the Rust build).
`tools/gen_textures.py` (also stdlib, no PIL) emits three neutral-grey
`dev_gray_*` PNGs beside the five original coloured ones.

A tall room, **1280 × 896 × 512 u** interior: two platforms at z 176–192
(4.88 m), one in each far corner, **512 u ≈ 13 m** apart — past any ahoy jump
(speed 12, jump_height 1.8, gravity 29 → ~8.5 m flat). A ladder brush is bolted
to platform A's edge and runs 16 u above the deck so the top rungs are
grabbable. `player_spawn` on the floor, 7 m from the ladder, facing it.

The winding trap that bit the bootstrap map (bevy_trenchbroom's
`(p3-p1)×(p2-p1)` normal) lives in exactly one function in `gen_map.py`
(`_BOX_FACES`, each entry checked against a known face of the old `test.map`),
so a box declared `mins < maxs` comes out solid with outward normals.

### `func_ladder` and the `angle` trap

`FuncLadder` is `#[solid_class(base(Interactable))]` with a `face_yaw: f32`
field — **deliberately not `angle`**. bevy_trenchbroom reads a class field
literally named `angle` (also `angles`, `mangle`) as a brush rotation
(`angle_to_quat` → `Quat::from_rotation_y`), applied about the **world origin**,
not the brush centroid — it flings the geometry across the room (confirmed:
`angle 90` put the ladder's collider ~12 m from where its verts should be, with
`GlobalTransform` carrying a spurious 90° Y and `ColliderAabb` stuck in the
un-rotated local frame). `FuncDoor` has this bug too, latent — its brush is
free-standing and nobody has walked to it. `ladder.rs` turns `face_yaw` into a
facing direction itself, matching `angle_to_quat`'s convention
(`Quat::from_rotation_y(yaw) * -Z`; 0 → −Z, 90 → −X).

### `ladder.rs` — brackets bevy_ahoy, doesn't hook it

`bevy_ahoy`'s `run_kcc` is one private monolithic system with no seam for a new
movement mode. So the ladder *brackets* it, all via public API:

- `stash_input` (`RunFixedMainLoop`, `RunFixedMainLoopSystems::BeforeFixedMainLoop`) —
  `take`s `AccumulatedInput::last_movement` into `Climbing::wish` and *discards*
  `AccumulatedInput::jumped`, so ahoy walks and jumps nowhere. A **missing**
  `last_movement` is a release: ahoy's `apply_movement` only writes it on
  `Fire<Movement>`, and a dead-zoned axis doesn't fire on zero, so `None` → a
  zero `wish` is what makes letting go of W/S stop the climb where you hang (at
  any height, either direction). An earlier version kept the last non-zero value
  on `None` and so carried a released climber to the top.
  **Trap:** this must run once per *frame*, not per fixed step — hence
  `RunFixedMainLoop` rather than the old `FixedPostUpdate` slot. `AccumulatedInput`
  has a one-frame lifetime (ahoy's `clear_accumulated_input` ends it in
  `AfterFixedMainLoop`), so a `take` + default-to-zero here feeds every substep
  the same intent; the same code in `FixedPostUpdate` would read `None` on the
  second substep of a multi-substep frame and stall the climb to zero every
  other step. **Trap:**
  `AccumulatedInput::jumped` is ahoy's jump *buffer* (`Option<Stopwatch>`), not a
  per-step press flag, and reading it here at all — even age-checked — is wrong.
  `apply_jump` observes `Fire<Jump>`, and ahoy's `Jump` action has no condition,
  so it fires *every frame Space is held* and each one resets the stopwatch:
  `jumped.elapsed()` measures time since you *released* Space, not since you
  jumped, and is `0` while the key is down. `clear_accumulated_input` also clones
  it forward across fixed frames, and `handle_jump` returns without clearing it
  whenever it declines to jump. First cut age-checked it against
  `CharacterController::jump_input_buffer` (150 ms) — that fixed a genuinely
  stale buffer but made "run at the ladder, jump, press E" a coin flip on whether
  E landed inside that window. The decision to let go now comes only from
  `let_go_on_jump` on the `Start<Jump>` press *edge*. Same "state vs event"
  family as the `Press` trap below — and this file's second `Fire`-vs-edge bug.
- `let_go_on_jump` (`On<Start<Jump>>`) — a Space *press* while `Climbing` sets
  `Climbing::jump`, which `climb` turns into a push-off next fixed step.
  `Start<A>` is bevy_enhanced_input's `just_pressed` edge: it fires once per
  press and not again until release, so a Space that went down *before* the grab
  (holding jump into a ladder, or a jump made seconds earlier) raises no edge
  while climbing and does not detach.
- `attach_on_interact` (`On<Interacted>`) — the E-grab, the one and only way
  on. Snaps the body onto `Ladder::mount()` — `LADDER_STANDOFF` (0.7 m) out in
  front of the centre line along `-facing`, so you hang on the ladder's front
  face like a climber and the rungs are in view instead of clipped through the
  near plane (the visual is only `LADDER_VISUAL_DEPTH` = 0.10 m deep; centred on
  it you are looking *through* it). `mount()` is the one source of the offset —
  grab, the per-frame stick and the bottom dismount all call it; the **top**
  dismount walks along `line`, measured against the ladder itself. If
  already `Climbing`, E instead removes `Climbing` (a lock/unlock toggle — you
  drop off where you hang). On a mount it also inserts `LadderTurn(yaw)` on the
  camera entity, `yaw` recovered from `facing` via `atan2(-facing.x, -facing.z)`.
  Since the ladder now has two colliders on one logical entity (sensor volume +
  solid visual child, faces coplanar), `interact::update_focus` resolves a ray
  hit to the nearest `Interactable` *ancestor* (`resolve_interactable`, a
  `ChildOf` walk) rather than requiring the hit collider itself to be one — so
  a hit on the solid part still reads as "the ladder", and a future nested
  `SceneRoot` model works the same way. Doors and pickups hit their own entity
  and are unaffected.
  **Trap:** the toggle only works because the `Interact` action carries a
  `Press` condition (`input.rs`). An unconditioned bevy_enhanced_input action is
  `Down`-like, so its `Fire` event triggers *every frame* the key is held — and
  `fire_interact` would then fire `Interacted` every frame, flipping `Climbing`
  on/off/on/off for the duration of the press and leaving you locked on or not
  by parity. `attach_on_interact` is the repo's first non-idempotent
  `Interacted` consumer; `door::open_on_interact` (only acts from `Closed`) and
  `pickup::collect_on_interact` (despawns) never noticed. Regression-checked:
  without `Press` the autopilot's ~0.15 s hold fires `fire_interact` ~21× and
  the player walks straight past the ladder.
- `turn_to_ladder` (`PostUpdate`, before `TransformSystems::Propagate`) — eases
  the camera yaw onto `LadderTurn`'s target (`LADDER_TURN_RATE` exponential,
  ~0.15 s), leaving pitch alone, then removes the marker. **Not `Update`**:
  ahoy's `copy_character_look_to_camera` rewrites the camera rotation from
  `CharacterLook` every `Update` with no public set to order against, so a yaw
  written there gets clobbered; written in `PostUpdate` it survives and next
  frame's `copy_camera_to_character_look` (`RunFixedMainLoop`) folds it back
  into `CharacterLook`.
- `climb` (`FixedPostUpdate`, after `MoveCharacters`, before
  `PhysicsSystems::First`) — the climb height lives in `Climbing`, not the
  `Transform`; `climb` writes an **absolute** position every step and zeros
  `LinearVelocity`, so ahoy's gravity and ground-snap (which still run) are
  overwritten, not fought. Detach on jump (pushed off the rungs), on reaching
  the bottom while descending (step onto the floor), or — **off the top** —
  after a *walked* dismount: `Climbing::crest` records metres walked, and
  `climb` slides the body `LADDER_DISMOUNT_STEP` (1.6 m) forward onto the deck
  under its own control before detaching, feet a short drop above solid ground.
  A bare teleport-and-detach there does **not** hold — ahoy's controller drags
  the released body back toward the mount line and over the deck's edge before
  it grounds (worse the further the release point is from the mount, which the
  0.7 m standoff made unignorable). The absolute per-step assertion is load-
  bearing: a `+=` walk accumulates against that drift and barely moves.

`ColliderAabb` on the ladder brush **is** its true world box here — because
there's no `angle`, the collider entity's `Transform` is identity and local ==
world. All tuning is in `config.rs` (`LADDER_CLIMB_SPEED` 2.2 m/s, snap rate,
`LADDER_STANDOFF` 0.7 m, turn-ease rate, jump/push-off, dismount step, plus the
visual rail/rung sizes). `ui.rs` swaps the centre prompt for
`"W/S to climb · E or Space to let go"` while `Climbing`.

### The ladder visual is decoupled from the volume, and it's what's solid

The `func_ladder` brush is textured **`skip`** — one of bevy_trenchbroom's
default `auto_remove_textures`, so it renders nothing (no mesh child, no PNG
needed) while still producing a full convex collider from its `Brushes` asset
and still firing `SceneCollidersReady`. `setup_ladders` then builds the visible
ladder itself: `ladder_mesh(width, height)` merges two rail cuboids + a run of
rung cuboids (`LADDER_RUNG_SPACING` apart) into **one** `Mesh`, spawned as a
grey `StandardMaterial` child of the brush entity, sized from the brush AABB
and yawed by `face_yaw`. One mesh + one material = a drop-in `SceneRoot` swap
when a real model arrives. `dev_ladder.png` and its generator entry are retired.

The split of duties is: **volume = `Sensor`** (the aim/grab target and the
source of `Ladder`'s AABB, never a wall — `gen_map.py` makes it a deliberately
generous 40 u / 1 m deep grab box, and a solid box that size would be an
invisible wall swallowing the 0.7 m mount standoff); **visual child = solid**.
The child carries a `Collider::cuboid(width, size.y, LADDER_VISUAL_DEPTH)` —
exactly `ladder_mesh`'s bounding box, solid straight through the rung gaps like
any shooter's ladder — so you can't walk through the rungs but the grab volume
around them still doesn't block. Every `climb` state already sits clear of that
0.10 m slab: the mount and bottom dismount are at `mount()` (0.7 m out), the
crest teleport is at `line` with the feet at the slab's *top* face
(`hi = aabb.max.y + PLAYER_HEIGHT/2`), the jump-off pushes along `-facing`.
**Trap:** a `Collider` blocks an ahoy KCC only if avian gave it a
`ColliderOf` — i.e. it has a `RigidBody` somewhere up its hierarchy (here
bevy_trenchbroom's `RigidBody::Static` on the brush). ahoy moves through
avian's `MoveAndSlide`, whose collider set is `(With<ColliderOf>,
Without<Sensor>)`. A `Collider` with no `RigidBody` ancestor is silently
invisible to the controller — and it fails as "the player still walks straight
through" with no log line, exactly like the sensor did.

### Lighting

The warehouse is ~2.5× the bootstrap room in each axis and its dev textures are
darker, so `SUN_ILLUMINANCE` went 3000 → 10 000 and a `GlobalAmbientLight`
(`AMBIENT_BRIGHTNESS` 800) was added — in Bevy 0.18 `AmbientLight` is a
per-camera component and `GlobalAmbientLight` is the resource.

**Verified (2026-09-08)** via the Phase 4a autopilot (synthetic keyboard input
is still blocked on this Mac). `AUTOPILOT_SCRIPT` now presses E on approach,
presses E again on the rungs, drops, re-mounts, and rides to the top;
telemetry:
- during the whole approach `climbing=false` even as the player passes through
  the ladder's old grab volume — the proof the on-contact grab is gone (before
  this change `climbing` flipped true the instant the body entered the volume);
- E fires the frame `focus=Some("Climb ladder")` first appears and `climbing`
  goes true with the origin teleport-snapped onto the mount line —
  `LADDER_STANDOFF` in front of the rungs, `x ≈ -0.42` (was `≈ -1.12` when the
  body sat on the centre line, buried in the visual);
- a second E press one sample later flips `climbing` back to false and the body
  drops the short distance to `grounded=true`, `pos.y ≈ 0.96` — the toggle-off;
- re-mount, then `pos.y` rises monotonically ≈ 1.2 → 5.8 m at ≈ 2.22 m/s
  (`LADDER_CLIMB_SPEED`), `x` held at the mount line by the `smooth_nudge`
  stick; `cam_yaw_pitch` steady at `(90°, 0°)` the whole run — the camera
  faces straight into the rungs (yaw 90° = world −X = `facing`), so
  `turn_to_ladder` had nothing to correct here (the autopilot spawns already
  aimed down the approach) but the lock-on view is confirmed pointed right;
- the mid-climb screenshot (`260908-ladder-lock-view.png`, keyed on
  `Climbing` + `y > 2.5`) shows two rungs across the frame at arm's length
  with the warehouse visible past them — this ladder backs onto the open
  volume *under* deck A (the deck is a slab overhead, not a wall behind), so
  the rungs read as sparse; the point the shot proves is that the camera is
  **out of** the mesh now (before, at `x ≈ -1.12`, it was inside the 0.10 m
  visual and saw nothing). Whether 0.7 m is the right distance, and whether a
  climb wants a slight downward look, are feel calls for a person;
- at the top the crest walk slides `x` −0.42 → −2.7 (`y` pinned at ≈ 6.19),
  then `climbing=false` and the body settles `pos.y ≈ 5.79 m`, `grounded=true`
  ≈ 1 m onto platform A. Landed cleanly on **every** run of a 3× + final
  repeat; the earlier teleport-and-detach dismount fell off the deck edge on
  ~half of them once the 0.7 m standoff moved the release point;
- zero `error!`/`warn!`, no avian `ColliderAabb`/negative-mass panic, no
  `"No colliders produced by brushes"` (the `skip` texture didn't break the
  collider), no `Cannot update KCC`.

The approach screenshot still shows the rails-and-rungs ladder floor to above
the deck (the `skip`-textured volume + procedural `ladder_mesh` from the
earlier visual pass are unchanged).

**Re-verified (2026-09-08) after the E-grab `Press` fix.** The lock/unlock
toggle was a coin flip — the `Interact` action had no condition, so `Fire`
triggered `Interacted` every frame E was held and `attach_on_interact` flipped
`Climbing` on/off for the whole press. Added `Press::default()` to the action
(`input.rs`); reworked the autopilot to hold the real `KeyCode::KeyE` in
`ButtonInput` (`PreUpdate`, before `EnhancedInputSystems::Update`) so the fix is
actually under test — the old direct `Interacted` trigger, guarded to once per
step, could never have caught this.
- Regression check first: a temporary `fire_interact` log, `Press` left out —
  the autopilot's ~0.15 s hold fired `fire_interact` **~21×** and that run's
  parity left the player *not* climbing, walking straight past the ladder
  (`focus` `Some` → `None`, `x` drifting to −3.47). The reported bug, on demand.
- With `Press`: **exactly 3** `fire_interact` calls in the whole run — one per
  interact leg. `climbing` goes true once on the grab and holds (no flap), one
  clean toggle-off to `grounded` `y ≈ 0.96`, re-mount, climb, crest walk, lands
  on platform A. Same sequence as above, now deterministic. Run 3× — identical
  every time (the point, since the complaint was intermittent). Zero
  `error!`/`warn!`, no `Cannot update KCC`.

**Re-verified (2026-09-08) — grab while jumping.** After the age-check fix
above, grabbing a ladder mid-jump still worked only *"roughly 50% of the time"*.
Cause: `AccumulatedInput::jumped`'s stopwatch is reset to `0` every frame Space
is held (`apply_jump` observes `Fire<Jump>`; ahoy's `Jump` has no condition), so
`stash_input`'s age check passed the buffer straight through whenever E landed
within `jump_input_buffer` of releasing Space — a coin flip on E's exact timing.
Fix: `stash_input` now *discards* `jumped` (taken only to keep ahoy quiet), and
a new `let_go_on_jump` observer on `Start<Jump>` — bevy_enhanced_input's
`just_pressed` edge — is the only thing that sets `Climbing::jump`. A Space press
that predates the grab raises no edge while climbing, so holding jump into a
ladder no longer throws you off. `ladder.rs` + one `add_observer` in `main.rs`;
`input.rs` gains a comment on why `Jump` stays unconditioned.
The autopilot's `jump` leg is now a continuous `KeyCode::Space` *hold* for the
leg's whole duration (a run of `jump` legs = one hold). Script: grab #1 (clean),
toggle off, settle at the mount, **jump straight up and — still holding Space —
press E in the air** (grab #2), walk forward.
- Regression check first, on the age-check version: grab #1 climbs normally;
  grab #2 is undone the frame it lands (`climbing` never sampled true) and the
  detached body walks forward through the sensor to the far wall (`x ≈ -5…-11`,
  `y ≈ 0.92`), platform A never reached.
- With the fix: grab #2 goes `climbing=true` at `y ≈ 1.45` and **holds** through
  the whole airborne leg with Space still down, then the FWD leg climbs `y` to
  ≈ 5.8, crest walk, `grounded` on platform A at `y ≈ 5.79` / `x ≈ -2.8`. Grab #1
  + toggle-off unchanged. Run 3× — identical. Zero `error!`/`warn!`, no
  `Cannot update KCC`. `260908-on-platform-a.png` / `260908-ladder-lock-view.png`
  refreshed.

**Verified (2026-09-09) — the ladder is solid.** The visual child gained a
`Collider::cuboid` (see "The ladder visual is decoupled from the volume, and
it's what's solid"); the sensor volume is unchanged. `update_focus` now walks
`ChildOf` to the `Interactable` ancestor. The autopilot script gained a leading
leg — walk straight into the rungs, no E — and grab #1 shrank to a 1 s `STILL`
press since the body is already parked in front of the ladder.
- Regression check first, `Collider::cuboid` line commented out: the opening
  leg walks clean **through** the ladder, under deck A, to the west wall at
  `x ≈ -11.0`, `y ≈ 0.9` — `climbing` false throughout. The sensor-only ladder,
  reproduced.
- With the collider: the opening leg stalls at `x ≈ -0.65` (`≈ 0.05 +
  PLAYER_RADIUS` off the slab face) with `climbing` false every sample — solid.
  Then the rest is unchanged: grab #1 snaps to the mount (`x = -0.418`),
  toggle-off to `grounded y ≈ 0.92`, jump straight up, grab #2 holds
  `climbing=true` through the airborne leg with Space held, climb to `grounded`
  on platform A at `y ≈ 5.79`. Run 3× — identical. Zero `error!`/`warn!`, no
  `Cannot update KCC`, no avian penetration/negative-mass panic. Screenshots
  refreshed (low-contrast grey-on-grey — the ladder material and the wall
  texture are near the same value under the flat warehouse lighting; a feel/art
  call, not a regression).

**Verified (2026-09-09) — the climb stops when you let go.** `stash_input` now
defaults `Climbing::wish` to `Vec2::ZERO` on a missing `last_movement` (a real
release) and runs in `RunFixedMainLoop`/`BeforeFixedMainLoop` (once per frame, so
the take/default is safe). `autopilot_drive` no longer publishes `Some(ZERO)` on
a `STILL` leg — it writes nothing, mirroring a released dead-zoned axis. The
single 5.5 s FWD climb leg was split FWD 1.2 s / STILL 1.5 s / FWD 3.0 s.
- Regression check first, `stash_input` reverted to `if let Some(...)`: during the
  1.5 s `STILL` leg `y` keeps rising (`≈ 4.7 → 5.85`) and crests onto platform A —
  the last wish stuck. Reproduced.
- With the fix, 3× identical: on the `STILL` leg `y` holds flat (run 1
  `y = 3.9927`, run 2 `4.0671`, run 3 `4.1046`) for every sample with
  `climbing=true`; the following FWD leg resumes the climb, crests, and lands
  `grounded` on platform A at `y ≈ 5.79`. The solid-ladder opening leg
  (`x ≈ -0.5…-0.6`, `climbing` false), grab #1, toggle-off, and airborne grab #2
  (`climbing=true` held with Space down) are all unchanged. Zero `error!`/`warn!`,
  no `Cannot update KCC`, no avian panic. Screenshots refreshed.

Not autopilot-checkable (no keyboard), left for a human: pressing E for real,
repeatedly, at a ladder (the thing that surfaced the coin flip); the same for
running at a ladder, jumping, and pressing E once (this fix — one press should
now be enough, every time) and that a *fresh* Space press still lets go promptly
mid-climb while *holding* Space through the grab does not; whether losing
"hold E while walking up" — a press consumed before the ladder focuses no longer
grabs — is felt as a loss; the E-grab from the
*top* edge of platform A; whether `LADDER_STANDOFF` = 0.7 m frames the ladder
right or wants to be nearer/further; whether the ~1.6 m crest walk off the top
reads as "hauling over" or as a shove (`LADDER_DISMOUNT_STEP`); the jump-off
mid-ladder; whether the ~0.15 s yaw ease onto the rungs *feels* right
(`LADDER_TURN_RATE`); whether losing the walk-into-it mount makes going up feel
less fluid; whether 2.2 m/s *feels* right; whether a strafe-jumping player can
clear the 13 m gap; whether the ladder's rung spacing / rail thickness /
colour look right; whether walking into the now-solid rungs head-on and from
either side feels clean (no jitter, no climbing into it); whether the ~0.4 m of
ladder standing proud of deck A reads as a step you walk over rather than a
wall; and whether mounting from contact yanks you back more than the 0.25 m the
standoff asks for. Also: tapping W/S in short bursts up a ladder — motion only
while a key is held, no coast or lurch on release, both directions; releasing W
right at the top (the crest step still auto-completes by design — it should read
as a dismount, not a slide); releasing S at the very bottom (you should hang at
the lowest rung, and pressing S again finishes the step down).

## Environment notes

- TrenchBroom.app is installed on this Mac (`~/Library/Application Support/TrenchBroom` exists) — the game config + FGD write on every `cargo run` (`bevy_trenchbroom::config::writing` info logs confirm success).
- ericw-tools is built from source at `/Users/makkusu/Code/other/ericw-tools`, built at `/Users/makkusu/Code/other/ericw-tools/build/{qbsp,light,vis,bsputil}/<name>` (2.0.0-alpha11). Not on PATH — invoke by full path, or add the relevant `build/*/`  directories to PATH. See Phase 5 for the submodule-init and `-DDISABLE_DOCS=ON` gotchas that came up building it.
- Synthetic input (cliclick/osascript CGEvent) is blocked on this Mac (no Accessibility grant) — see `.claude/skills/verify/SKILL.md`. Phase 4's in-process autopilot is the way around this for future verification.
