//! Verification harness for running this example on a machine where
//! synthetic keyboard/mouse input is blocked (no macOS Accessibility grant —
//! see `.claude/skills/verify/SKILL.md`). `bevy_ahoy`'s `AccumulatedInput`
//! fields are all `pub`, and its own input observers only *write* them, so
//! driving the same fields from here composes cleanly with real input still
//! attached — this isn't a replacement input path, just another writer. It
//! mirrors the real path's fidelity too: a zero-movement leg writes *nothing*
//! to `last_movement` (leaving it `None`), exactly as a released dead-zoned
//! axis does, so `ladder::stash_input` can tell "let go" from "still holding".
//! The
//! `interact` and `jump` legs go one step further and press the real
//! `KeyCode::KeyE` / `KeyCode::Space` in `ButtonInput`, so the whole binding →
//! `Press` -> `fire_interact` path (E) and binding -> ahoy `Jump` /
//! `Start<Jump>` path (Space) are under test — those *are* the real input paths
//! end to end, which is how this harness can catch an `Interact` action that
//! fires every frame instead of once per press, or a jump pressed *before* a
//! grab wrongly knocking the player off a ladder.
//!
//! All of this is opt-in via environment variables, checked once at
//! startup in `main.rs`, so a normal `cargo run` behaves exactly as before:
//! - `IMMERSIVE_AUTOPILOT=1` / `=crates` / `=walk` — walk a scripted route automatically.
//! - `IMMERSIVE_TELEMETRY=1` — log position/speed/grounded/focus/lights periodically.
//! - `IMMERSIVE_SHOTS=1` — take in-app screenshots at scripted checkpoints.
//! - `IMMERSIVE_LIGHTS=on` — every fixture/switch comes up energised.
//! - `IMMERSIVE_LIGHTS=toggle` — fire `Interacted` at the wall switch once,
//!   ~2.5 s in (the switch is only reachable by a hand-built crate stack).
//! - `IMMERSIVE_AUDIO=log` — echo every `PlaySfx` request (clip, volume, pitch).

use avian3d::prelude::LinearVelocity;
use bevy::prelude::*;
use bevy::render::view::window::screenshot::{save_to_disk, Screenshot};
use bevy::ui::RelativeCursorPosition;
use bevy_ahoy::input::AccumulatedInput;
use bevy_ahoy::prelude::*;
use bevy_ahoy::CharacterLook;
use bevy_game_bits::inventory::prelude::{
    ActiveSlot, InventoryConfig, InventoryGrid, InventoryWindow, Quickbar, QuickbarParts,
};
use bevy_game_bits::inventory::{AddItem, InventoryItem};

use crate::breakable::Breakable;
use crate::carry::{Carried, Carrying, ThrowCharge};
use crate::classes::{ItemPickup, PropCrate, PropWoodCrate};
use crate::config::{self, AutopilotStep};
use crate::door::DoorSwing;
use crate::interact::{Interacted, InteractionFocus};
use crate::ladder::Climbing;
use crate::pickup::PlayerPack;

pub fn env_flag(name: &str) -> bool {
    std::env::var(name).map(|v| v == "1").unwrap_or(false)
}

/// `IMMERSIVE_LIGHTS=on` — every `LightFixture` / `FuncLightSwitch` comes up
/// energised regardless of its `start_on`, so the lit warehouse can be
/// iterated on without first building a crate stack to reach the switch.
pub fn force_lights_on() -> bool {
    std::env::var("IMMERSIVE_LIGHTS")
        .map(|v| v == "on")
        .unwrap_or(false)
}

/// `IMMERSIVE_LIGHTS=toggle` — fire `Interacted` straight at the wall switch
/// once, a couple of seconds in. The switch sits where only a hand-built
/// crate stack reaches, which no autopilot can do; the "use" raycast that
/// finds it is the same `SpatialQuery::cast_ray` the ladder already proves,
/// so the harness skips straight to the event and exercises the part that's
/// new: `lights::toggle_on_interact` → `Powered` → the `sync_*` mirrors →
/// `GlobalAmbientLight`. Read with `IMMERSIVE_TELEMETRY=1`: `lights=4/12`
/// (the 4 always-on pillar brackets) must step to `lights=12/12 switches_on=1`
/// exactly once and stay, and the 4 brackets are unaffected.
pub fn auto_toggle_switch(
    switches: Query<Entity, With<crate::lights::SwitchState>>,
    mut commands: Commands,
    mut frame: Local<u32>,
    mut done: Local<bool>,
) {
    *frame += 1;
    if *done || *frame < 150 {
        return;
    }
    if let Ok(switch) = switches.single() {
        info!("devtools: auto-toggling light switch {switch}");
        commands.trigger(crate::interact::Interacted { entity: switch });
        *done = true;
    }
}

/// `IMMERSIVE_PROPS` (Phase 10): exercise the hinged door and the breakable
/// crate without a keyboard, the same way `IMMERSIVE_LIGHTS=toggle` exercises
/// the switch. `=door` fires `Interacted` at every `prop_door` twice (open ~2.5
/// s in, close ~7 s in) — read the swing in `IMMERSIVE_TELEMETRY`'s
/// `doors(open=...)`. `=break` zeroes every wooden crate's health once, ~2.5 s
/// in, so `shatter` runs and the telemetry `wood_crates` count drops to 0 as
/// `pickups` rises — the shattered crate's contained item appearing on the
/// floor. `=locked` fires `Interacted` at every door once and expects the
/// prompt to read "Locked" and the door not to move. There is only one door
/// now (the locked one), so `=door` and `=locked` behave the same. `=pick`
/// (Phase 16) warps to the door, hands the player an active lockpick, holds the
/// real `MouseButton::Left` (`press_use_key`) to pick the lock, fires
/// `Interacted` to open it, then walks into the corridor for a lighting shot —
/// telemetry `doors(... locked=1)` → `locked=0` → `open=1/1`.
pub fn props_mode() -> Option<String> {
    std::env::var("IMMERSIVE_PROPS").ok()
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn exercise_props(
    mode: Res<PropsMode>,
    doors: Query<Entity, With<DoorSwing>>,
    mut breakables: Query<&mut Breakable>,
    // `CharacterLook` is `#[require]`d onto the *player* (the relationship
    // target of `CharacterControllerCamera`), not the camera entity.
    mut player: Query<(&mut Transform, &mut CharacterLook), With<CharacterController>>,
    // ...and the camera transform must be pinned too: ahoy's
    // `copy_camera_to_character_look` re-derives `CharacterLook` from the
    // camera every frame and would otherwise clobber the look write before
    // `copy_character_look_to_camera` applies it (an unordered-system race).
    mut camera: Query<
        &mut Transform,
        (With<CharacterControllerCameraOf>, Without<CharacterController>),
    >,
    pack: Option<Res<PlayerPack>>,
    mut add_item: MessageWriter<AddItem>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut commands: Commands,
    mut frame: Local<u32>,
) {
    *frame += 1;
    let at_door = matches!(mode.0.as_str(), "door" | "locked" | "pick");

    // Warp + hold the view down the east extension so the screenshots actually
    // frame what this devtool exercises. Bevy: the east-wall doorway is at
    // z ≈ -16.5, centred on x ≈ 0; -Z (yaw 0) faces it (and the bay / yard
    // beyond). `pick` walks it in stages so each junction is captured clean:
    //   90–235   2 m in front of the door
    //   236–299  a few metres into the (now-open) corridor
    //   300–420  into the bay, facing the large yard opening
    if at_door && (90..420).contains(&*frame) {
        let z = match (mode.0.as_str(), *frame) {
            ("pick", f) if f >= 300 => -40.0,
            ("pick", f) if f >= 236 => -22.0,
            _ => -14.2,
        };
        let facing = Quat::from_euler(EulerRot::YXZ, 0.0, -0.10, 0.0);
        if let Ok((mut t, mut look)) = player.single_mut() {
            t.translation = Vec3::new(0.0, 1.0, z);
            (look.yaw, look.pitch) = (0.0, -0.10);
        }
        if let Ok(mut cam_t) = camera.single_mut() {
            cam_t.rotation = facing;
        }
        if *frame == 90 {
            info!("devtools: warped + holding player at the corridor door");
        } else if *frame == 300 && mode.0 == "pick" {
            info!("devtools: warped player into the bay for the far-junction shot");
        } else if *frame == 236 && mode.0 == "pick" {
            info!("devtools: warped player into the corridor for the light shot");
        }
    }

    // `pick` (Phase 16): add a lockpick to the pack (it auto-fills quickbar
    // slot 0), then press the real `Digit1` to make it active — so the whole
    // pickup → grid → quickbar → `Use` path is exercised, not just a resource
    // poke. `press_use_key` (PreUpdate) then LMBs the lock, then E opens it.
    if mode.0 == "pick" {
        if *frame == 95 {
            if let (Some(pack), Some(def)) = (&pack, crate::items::lookup("lockpick")) {
                add_item.write(AddItem {
                    board: ***pack,
                    item: InventoryItem::new(def.name, def.description, def.cells, def.color),
                    origin: None,
                });
                info!("devtools: added a lockpick to the pack");
            }
        }
        if *frame == 108 {
            keys.press(KeyCode::Digit1);
        }
        if *frame == 114 {
            keys.release(KeyCode::Digit1);
            info!("devtools: selected quickbar slot 1");
        }
    }

    match (mode.0.as_str(), *frame) {
        ("door", 150) | ("door", 420) | ("locked", 150) | ("pick", 210) => {
            for door in &doors {
                info!("devtools: firing Interacted at door {door}");
                commands.trigger(Interacted { entity: door });
            }
        }
        ("break", 150) => {
            for mut breakable in &mut breakables {
                info!("devtools: zeroing a breakable's health");
                breakable.health = -1.0;
            }
        }
        _ => {}
    }
}

/// `IMMERSIVE_PROPS=pick`: hold the real `MouseButton::Left` for a window
/// (frames 130–150) so the whole `Use` binding → `Press` → `Fire<Use>` →
/// `use_item::fire_use` path is under test — the same "press the real key, not
/// the action" principle as `autopilot_drive`. `PreUpdate`, between
/// `bevy::input::InputSystems` and `EnhancedInputSystems::Update`.
pub fn press_use_key(mut mouse: ResMut<ButtonInput<MouseButton>>, mut frame: Local<u32>) {
    *frame += 1;
    if (130..150).contains(&*frame) {
        mouse.press(MouseButton::Left);
    } else {
        mouse.release(MouseButton::Left);
    }
}

/// Holds `IMMERSIVE_PROPS`'s value between the startup check and `exercise_props`.
#[derive(Resource)]
pub struct PropsMode(pub String);

/// `IMMERSIVE_INVENTORY` (Phases 14–16): keyboard-free checks for the grid pack
/// and the quickbar, since synthetic OS input is blocked on this Mac.
/// - `open` — presses the real `Tab`, so the whole
///   `player::player_cursor_input` → `InventoryWindow` → `sync_cursor_mode`
///   path runs; screenshots the board over the 3D scene, and telemetry shows
///   `pack(open=true)` with the player's look frozen.
/// - `use` — adds a lockpick to the pack (auto-fills quickbar slot 1), selects
///   it (real `Digit1`), warps to the locked door and holds real LMB: proves
///   the **consume** path — `pack(items=1 → 0)`, `doors(locked=1 → 0)`.
/// - `drag` — opens the pack and hand-drives a drag of the crowbar tile onto
///   quickbar slot 4 (writing `RelativeCursorPosition` after
///   `ui_focus_system`), so `quickbar::quickbar_drop` is exercised in-app.
pub fn inventory_mode() -> Option<String> {
    std::env::var("IMMERSIVE_INVENTORY").ok()
}

/// Holds `IMMERSIVE_INVENTORY`'s value.
#[derive(Resource)]
pub struct InventoryMode(pub String);

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn exercise_inventory(
    mode: Res<InventoryMode>,
    pack: Option<Res<PlayerPack>>,
    mut add_item: MessageWriter<AddItem>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut player: Query<(&mut Transform, &mut CharacterLook), With<CharacterController>>,
    mut camera: Query<
        &mut Transform,
        (With<CharacterControllerCameraOf>, Without<CharacterController>),
    >,
    boards: Query<(&InventoryConfig, &Quickbar, &QuickbarParts)>,
    items: Query<(Entity, &InventoryItem, &bevy_game_bits::inventory::InventorySlot)>,
    mut rels: Query<&mut RelativeCursorPosition>,
    mut frame: Local<u32>,
) {
    *frame += 1;
    let Some(pack) = pack.as_ref() else {
        return;
    };
    let board = ***pack;

    // Stock the pack up-front for every mode.
    if *frame == 60 {
        for key in ["lockpick", "crowbar"] {
            if let Some(def) = crate::items::lookup(key) {
                add_item.write(AddItem {
                    board,
                    item: InventoryItem::new(def.name, def.description, def.cells, def.color),
                    origin: None,
                });
            }
        }
        info!("devtools: stocked the pack with a lockpick and a crowbar");
    }

    match mode.0.as_str() {
        "open" => {
            if *frame == 100 || *frame == 260 {
                keys.press(KeyCode::Tab);
            } else {
                keys.release(KeyCode::Tab);
            }
        }
        "use" => {
            if *frame == 110 {
                keys.press(KeyCode::Digit1);
            } else if *frame == 116 {
                keys.release(KeyCode::Digit1);
            }
            // Warp + hold at the locked door (same geometry as exercise_props).
            if (90..320).contains(&*frame) {
                if let Ok((mut t, mut look)) = player.single_mut() {
                    t.translation = Vec3::new(0.0, 1.0, -14.2);
                    (look.yaw, look.pitch) = (0.0, -0.10);
                }
                if let Ok(mut cam_t) = camera.single_mut() {
                    cam_t.rotation = Quat::from_euler(EulerRot::YXZ, 0.0, -0.10, 0.0);
                }
            }
            // Hold LMB to pick the lock.
            if (150..175).contains(&*frame) {
                mouse.press(MouseButton::Left);
            } else {
                mouse.release(MouseButton::Left);
            }
        }
        "drag" => {
            // Open the pack, then drive a synthetic drag of the crowbar tile
            // (grid origin (0,0)) onto quickbar slot 4.
            if *frame == 100 {
                keys.press(KeyCode::Tab);
            } else {
                keys.release(KeyCode::Tab);
            }
            let Ok((config, _, parts)) = boards.get(board) else {
                return;
            };
            let crowbar = items
                .iter()
                .find(|(_, item, _)| item.name == "Crowbar")
                .map(|(_, _, slot)| slot.0);

            let write_rel = |rels: &mut Query<&mut RelativeCursorPosition>,
                             entity: Entity,
                             over: bool,
                             normalized: Vec2| {
                if let Ok(mut rel) = rels.get_mut(entity) {
                    rel.cursor_over = over;
                    rel.normalized = Some(normalized);
                }
            };

            if let Some(origin) = crowbar {
                let cell_center = (origin.as_vec2() + Vec2::splat(0.5)) * config.cell_px;
                let board_norm = cell_center / config.board_size() - Vec2::splat(0.5);
                let slot_norm = Vec2::new((4.0 + 0.5) / config::QUICKBAR_SLOTS as f32 - 0.5, 0.0);

                match *frame {
                    150 => {
                        write_rel(&mut rels, board, true, board_norm);
                        mouse.press(MouseButton::Left);
                    }
                    151..=158 => {
                        write_rel(&mut rels, board, false, Vec2::splat(5.0));
                        write_rel(&mut rels, parts.root, true, slot_norm);
                    }
                    159 => {
                        write_rel(&mut rels, board, false, Vec2::splat(5.0));
                        write_rel(&mut rels, parts.root, true, slot_norm);
                        mouse.release(MouseButton::Left);
                        info!("devtools: released a drag of the crowbar onto quickbar slot 4");
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

/// Which scripted run `IMMERSIVE_AUTOPILOT` selects: `=1` the ladder
/// regression walk (`config::AUTOPILOT_SCRIPT`), `=crates` the crate
/// grab/place/throw/weight-gate walk (`config::AUTOPILOT_SCRIPT_CRATES`),
/// `=walk` a plain back-and-forth on the open floor (`config::AUTOPILOT_SCRIPT_WALK`,
/// for the footstep cadence).
#[derive(Resource, Clone, Copy)]
pub struct AutopilotScript(pub &'static [AutopilotStep]);

/// The absolute view orientation (radians) the current leg wants. Written by
/// `autopilot_drive` in `PreUpdate`, applied to the camera by `autopilot_look`
/// in `PostUpdate` — after ahoy's `copy_camera_to_character_look` (which runs
/// in `RunFixedMainLoop`, *before* `Update`) would otherwise clobber anything
/// written into `CharacterLook`, and after a crate bump has nudged the body.
/// Forcing it late and absolutely is the only stable way. Same "write the
/// camera late" trick as `ladder::turn_to_ladder`.
#[derive(Resource, Default)]
pub struct AutopilotAim {
    /// Yaw to add to the camera's current facing this frame (`yaw_rate * dt`).
    pub yaw_delta: f32,
    /// Absolute pitch (radians) to hold the view at.
    pub pitch: f32,
}

pub fn autopilot_script() -> Option<AutopilotScript> {
    match std::env::var("IMMERSIVE_AUTOPILOT").ok().as_deref() {
        Some("1") => Some(AutopilotScript(config::AUTOPILOT_SCRIPT)),
        Some("crates") => Some(AutopilotScript(config::AUTOPILOT_SCRIPT_CRATES)),
        Some("walk") => Some(AutopilotScript(config::AUTOPILOT_SCRIPT_WALK)),
        _ => None,
    }
}

/// One human-length key press per script leg. Once `start` is true on a leg,
/// holds `key` down in `ButtonInput` for `config::AUTOPILOT_TAP_SECS`, then
/// releases — so the next leg's press is a fresh released→pressed edge for a
/// `Press` condition to fire on. Idempotent `press`/`release` calls, so driving
/// it every frame is fine.
#[derive(Default)]
pub struct Tap {
    for_step: Option<usize>,
    held: f32,
}

impl Tap {
    fn drive(
        &mut self,
        keys: &mut ButtonInput<KeyCode>,
        key: KeyCode,
        step: usize,
        start: bool,
        dt: f32,
    ) {
        if self.for_step == Some(step) {
            if self.held < config::AUTOPILOT_TAP_SECS {
                self.held += dt;
                keys.press(key);
            } else {
                keys.release(key);
            }
        } else if start {
            self.for_step = Some(step);
            self.held = 0.0;
            keys.press(key);
        } else {
            keys.release(key);
        }
    }
}

/// Drives the player through the selected `AutopilotScript` on a loop: writes
/// `AccumulatedInput.last_movement` (same field ahoy's own WASD observer
/// writes), hands the view orientation to `autopilot_look` via `AutopilotAim`,
/// taps the real `KeyCode::KeyE` on `interact` legs, and *holds* the real
/// `KeyCode::Space` / `MouseButton::Right` for the whole of any `jump` / `grab`
/// leg (so a run of such legs is one continuous hold — Space pressed well
/// before a ladder grab, RMB held to charge a throw). Runs in `PreUpdate`,
/// after `bevy::input::InputSystems` and before `EnhancedInputSystems::Update`,
/// so the key/mouse writes are seen by bevy_enhanced_input the same frame.
pub fn autopilot_drive(
    script: Res<AutopilotScript>,
    mut inputs: Query<&mut AccumulatedInput, With<CharacterController>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut aim: ResMut<AutopilotAim>,
    focus: Res<InteractionFocus>,
    time: Res<Time>,
    mut step_idx: Local<usize>,
    mut step_elapsed: Local<f32>,
    mut use_tap: Local<Tap>,
) {
    let steps = script.0;
    if steps.is_empty() || inputs.is_empty() {
        // Don't start (or advance) the script until the player exists — the
        // scene loads asynchronously, and letting the clock run during the load
        // would make which leg is "current" when the player appears depend on
        // disk speed, which the `jump` legs (fired at a leg's first frame)
        // can't tolerate.
        return;
    }

    // Walk the script once and then hold on the last leg (every script ends on
    // a STILL leg). Not a loop — a second pass would start from a world the
    // first pass rearranged (a thrown crate underfoot), making the telemetry
    // impossible to read.
    *step_elapsed += time.delta_secs();
    let mut step = &steps[(*step_idx).min(steps.len() - 1)];
    if *step_elapsed >= step.duration && *step_idx < steps.len() - 1 {
        *step_elapsed = 0.0;
        *step_idx += 1;
        step = &steps[*step_idx];
    }
    let AutopilotStep {
        movement,
        yaw_rate,
        interact,
        jump,
        grab,
        pitch_deg,
        ..
    } = *step;
    let dt = time.delta_secs();

    for mut input in &mut inputs {
        // A real release writes *nothing*: ahoy's `Movement` observer only
        // fires on a non-zero, dead-zoned vector, so `last_movement` stays
        // `None` for the frame. Publishing `Some(ZERO)` here would hide the very
        // bug the stop-mid-climb leg exists to catch (`ladder::stash_input`
        // reads `None` as "let go").
        if movement != Vec2::ZERO {
            input.last_movement = Some(movement);
        }
    }
    // Hand the view orientation to `autopilot_look` (PostUpdate): it keeps the
    // camera's own yaw, adds this leg's `yaw_rate` and forces the pitch. Both
    // scripts here leave `yaw_rate` 0 and rely on the spawn facing, so in
    // practice this only ever sets the pitch — needed because a floor crate is
    // below an eye-level ray. Writing `CharacterLook` from here is pointless:
    // ahoy's `copy_camera_to_character_look` overwrites it from the camera in
    // `RunFixedMainLoop`, before `Update` applies it and before the KCC runs.
    aim.yaw_delta = yaw_rate.to_radians() * dt;
    aim.pitch = pitch_deg.to_radians();

    // Press the real keys, not the actions: this puts the whole binding →
    // `Press` → `fire_interact` (E) and binding → ahoy `Jump` / `Start<Jump>`
    // (Space) paths under test. Drop `Press` from `Interact` and the E hold
    // toggles the ladder every frame; write `AccumulatedInput.jumped` directly
    // instead of via Space and you'd miss that the ladder must key off the
    // press *edge*, not the buffer.
    //
    // E is a tap fired the frame the raycast focuses something (so the grab
    // lands the instant the ladder is in range). Space is *held* for the whole
    // of a `jump` leg — so a run of `jump` legs ending on the grab leg means
    // Space went down seconds before the grab and is still down as it lands:
    // the "run at the ladder, jump, then press E" gesture, and the case
    // `let_go_on_jump` must NOT treat as a request to let go (the press
    // predates the climb, so it raises no new edge while climbing).
    use_tap.drive(
        &mut keys,
        KeyCode::KeyE,
        *step_idx,
        interact && focus.0.is_some(),
        dt,
    );
    if jump {
        keys.press(KeyCode::Space);
    } else {
        keys.release(KeyCode::Space);
    }

    // RMB is *held* for the whole of a `grab` leg, like Space — so a leg's
    // duration is the throw charge, and a run of `grab` legs is one continuous
    // press (grab, then keep holding to charge). Exercises the real
    // binding → `Start<Grab>` / `Complete<Grab>` path in `carry.rs`.
    if grab {
        mouse.press(MouseButton::Right);
    } else {
        mouse.release(MouseButton::Right);
    }
}

/// Apply the current leg's pitch to the camera, after ahoy's own camera sync.
/// `PostUpdate`, before transform propagation — the `ladder::turn_to_ladder`
/// slot and reasoning.
pub fn autopilot_look(
    aim: Res<AutopilotAim>,
    mut cameras: Query<&mut Transform, With<CharacterControllerCameraOf>>,
) {
    for mut transform in &mut cameras {
        let (yaw, _, _) = transform.rotation.to_euler(EulerRot::YXZ);
        transform.rotation = Quat::from_euler(EulerRot::YXZ, yaw + aim.yaw_delta, aim.pitch, 0.0);
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn telemetry(
    player: Query<
        (
            &Transform,
            &CharacterControllerState,
            Option<&Climbing>,
            Option<&Carrying>,
            Option<&ThrowCharge>,
            Option<&crate::footsteps::Footsteps>,
        ),
        With<CharacterController>,
    >,
    cameras: Query<&Transform, With<CharacterControllerCameraOf>>,
    crates: Query<(&Transform, Option<&LinearVelocity>, &PropCrate), Without<Carried>>,
    fixtures: Query<&crate::lights::Powered>,
    switches: Query<&crate::lights::SwitchState>,
    doors: Query<&DoorSwing>,
    wood_crates: Query<&Breakable, With<PropWoodCrate>>,
    pickups: Query<(), With<ItemPickup>>,
    packs: Query<(&Quickbar, &ActiveSlot, &InventoryGrid, &InventoryWindow)>,
    focus: Res<InteractionFocus>,
    time: Res<Time>,
    mut since_last: Local<f32>,
) {
    *since_last += time.delta_secs();
    if *since_last < config::TELEMETRY_INTERVAL {
        return;
    }
    *since_last = 0.0;

    let cam = cameras.single().ok().map(|t| {
        let (y, p, _) = t.rotation.to_euler(EulerRot::YXZ);
        (
            (t.translation.x, t.translation.y, t.translation.z),
            y.to_degrees(),
            p.to_degrees(),
        )
    });
    // *Liftable* crates only (mass <= CARRY_MAX_MASS) — the carry subjects.
    // Skips the pre-placed heavy crates (400 kg autopilot, 800 kg base), so
    // `max_y`/`moving` mean something: a walked-into crate stays at 0 moving,
    // a thrown one shows as 1, a stack lifts `max_y`.
    let mut count = 0;
    let mut max_y = f32::MIN;
    let mut moving = 0;
    for (transform, vel, prop) in &crates {
        if prop.mass > config::CARRY_MAX_MASS {
            continue;
        }
        count += 1;
        max_y = max_y.max(transform.translation.y);
        if vel.is_some_and(|v| v.0.length() > 0.15) {
            moving += 1;
        }
    }
    let lamps_on = fixtures.iter().filter(|p| p.0).count();
    let lamps_total = fixtures.iter().count();
    let switches_on = switches.iter().filter(|s| s.0).count();
    let doors_total = doors.iter().count();
    let doors_open = doors.iter().filter(|d| d.is_open()).count();
    let doors_locked = doors.iter().filter(|d| d.locked).count();
    let wood_crates = wood_crates.iter().count();
    let pickups = pickups.iter().count();
    let pack = packs
        .iter()
        .next()
        .map(|(quickbar, active, grid, window)| {
            let filled = quickbar.slots.iter().filter(|s| s.is_some()).count();
            format!(
                "pack(items={} free={} open={} active={:?})",
                filled,
                grid.free_cells(),
                window.open,
                active.0
            )
        })
        .unwrap_or_default();
    for (transform, state, climbing, carrying, charge, footsteps) in &player {
        info!(
            "telemetry: pos={:?} grounded={} climbing={} carrying={} charge={:.2} \
             steps={} phase={:.2} \
             crates(n={} max_y={:.2} moving={}) lights={}/{} switches_on={} \
             doors(open={}/{} locked={}) wood_crates={} pickups={} {pack} focus={:?} cam_yaw_pitch={:?}",
            transform.translation,
            state.grounded.is_some(),
            climbing.is_some(),
            carrying.is_some(),
            charge.map_or(0.0, |c| c.0),
            footsteps.map_or(0, |f| f.steps),
            footsteps.map_or(0.0, |f| f.phase),
            count,
            if count > 0 { max_y } else { 0.0 },
            moving,
            lamps_on,
            lamps_total,
            switches_on,
            doors_open,
            doors_total,
            doors_locked,
            wood_crates,
            pickups,
            focus.0.as_ref().map(|(_, prompt)| prompt.as_str()),
            cam,
        );
    }
}

/// `IMMERSIVE_AUDIO=log` — echo every [`PlaySfx`](bevy_game_bits::audio::PlaySfx)
/// request. Movement audio can't be verified any other way on a machine that
/// can neither synthesise input nor capture sound: the log of what *would*
/// play, with its volume and pitch, is the artefact.
pub fn audio_log() -> bool {
    std::env::var("IMMERSIVE_AUDIO").as_deref() == Ok("log")
}

pub fn log_sfx(
    mut requests: MessageReader<bevy_game_bits::audio::PlaySfx>,
    asset_server: Res<AssetServer>,
) {
    for req in requests.read() {
        let clip = asset_server
            .get_path(req.clip.id())
            .map(|p| p.to_string())
            .unwrap_or_else(|| format!("{:?}", req.clip.id()));
        info!("sfx: {clip} vol={:.2} speed={:.2}", req.volume, req.speed);
    }
}

/// Which one-shot screenshots have already fired. One bundled `Local` so the
/// system stays under the parameter cap.
#[derive(Default)]
pub struct Shots {
    lock_view: bool,
    on_platform: bool,
    carrying: bool,
    platform_b: bool,
    lights_dark: bool,
    lights_lit: bool,
    door: bool,
    door_unlocked: bool,
    corridor: bool,
    bay: bool,
    pack_open: bool,
    viewmodel: bool,
    quickbar_drag: bool,
}

/// State- and frame-gated screenshots, each fired once. The ladder-standoff
/// and on-platform shots are keyed on state (their timing drifts with FPS);
/// the corridor/bay/inventory shots are frame-gated to the matching devtool
/// run.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn take_screenshot(
    mut commands: Commands,
    player: Query<
        (
            &Transform,
            &CharacterControllerState,
            Option<&Climbing>,
            Option<&Carrying>,
        ),
        With<CharacterController>,
    >,
    switches: Query<&crate::lights::SwitchState>,
    windows: Query<&InventoryWindow>,
    mut frame: Local<u32>,
    mut done: Local<Shots>,
) {
    *frame += 1;
    if *frame == 120 {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(
                "screenshots/010-immersive/devtools-autopilot.png",
            ));
    }
    // Phase 9: the dark room (a fixed early frame) and the lit room (the first
    // frame any switch reads on).
    if !done.lights_dark && *frame == 90 {
        done.lights_dark = true;
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(
                "screenshots/010-immersive/260909-warehouse-dark.png",
            ));
    }
    if !done.lights_lit && switches.iter().any(|s| s.0) {
        done.lights_lit = true;
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(
                "screenshots/010-immersive/260909-warehouse-lit.png",
            ));
    }
    // `IMMERSIVE_INVENTORY`: the grid pack open over the 3D scene.
    if !done.pack_open && *frame > 60 && windows.iter().any(|w| w.open) {
        done.pack_open = true;
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk("screenshots/010-immersive/260910-pack-open.png"));
    }
    // `IMMERSIVE_INVENTORY=drag`: after the synthetic drag onto slot 4 lands.
    if !done.quickbar_drag && *frame == 175 && windows.iter().any(|w| w.open) {
        done.quickbar_drag = true;
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(
                "screenshots/010-immersive/260910-quickbar-drag.png",
            ));
    }
    // `IMMERSIVE_INVENTORY=use`: the lockpick in hand (selected ~frame 116,
    // spent ~frame 151).
    if !done.viewmodel && *frame == 140 {
        done.viewmodel = true;
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk("screenshots/010-immersive/260910-viewmodel.png"));
    }
    let Ok((transform, state, climbing, carrying)) = player.single() else {
        return;
    };
    // In front of the east-wall corridor door (the `IMMERSIVE_PROPS` warp
    // parks the player here) — the shot for the door's fit in its frame.
    let at_corridor_door = transform.translation.x.abs() < 2.0
        && (-15.5..-12.0).contains(&transform.translation.z);
    if !done.door && *frame > 140 && at_corridor_door {
        done.door = true;
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(
                "screenshots/010-immersive/260909-corridor-door.png",
            ));
    }
    // `IMMERSIVE_PROPS=pick`: after the LMB window (frames 130–150) and before
    // the E-open (frame 210) — the door still shut but its lock plate green.
    if !done.door_unlocked && *frame == 190 && at_corridor_door {
        done.door_unlocked = true;
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(
                "screenshots/010-immersive/260910-door-unlocked.png",
            ));
    }
    // `IMMERSIVE_PROPS=pick`: warped a few metres into the corridor (z ≈ -22)
    // after the door opened — the wall-bracket lighting + a clean ceiling all
    // the way to the bay.
    if !done.corridor
        && *frame == 280
        && transform.translation.x.abs() < 2.0
        && (-25.0..-19.0).contains(&transform.translation.z)
    {
        done.corridor = true;
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk("screenshots/010-immersive/260910-corridor.png"));
    }
    // `IMMERSIVE_PROPS=pick`: warped into the bay (z ≈ -40) facing the large
    // yard opening — both new junctions (corridor→bay, bay→yard) in one frame.
    if !done.bay
        && *frame == 380
        && transform.translation.x.abs() < 2.0
        && (-45.0..-35.0).contains(&transform.translation.z)
    {
        done.bay = true;
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk("screenshots/010-immersive/260910-bay.png"));
    }
    if !done.lock_view && climbing.is_some() && transform.translation.y > 2.5 {
        done.lock_view = true;
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(
                "screenshots/010-immersive/260908-ladder-lock-view.png",
            ));
    }
    if !done.on_platform && state.grounded.is_some() && transform.translation.y > 3.0 {
        done.on_platform = true;
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(
                "screenshots/010-immersive/260908-on-platform-a.png",
            ));
    }
    // Phase 8: the ghost crate at arm's length + charge slider.
    if !done.carrying && carrying.is_some() {
        done.carrying = true;
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(
                "screenshots/010-immersive/260909-carrying-crate.png",
            ));
    }
    // Phase 8: standing on platform B (deck top 4.88 m) after a crate stack.
    if !done.platform_b && state.grounded.is_some() && transform.translation.y > 4.5 {
        done.platform_b = true;
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(
                "screenshots/010-immersive/260909-on-platform-b.png",
            ));
    }
}
