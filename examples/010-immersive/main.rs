//! Immersive-sim prototype: roam a TrenchBroom-authored Quake map with a
//! first-person kinematic controller (bevy_ahoy). Levels are authored in
//! TrenchBroom (game "bevy_game_bits_immersive") and loaded as `.map` files
//! via bevy_trenchbroom; see SPEC.md for the phased build-out and why Q1/Q2
//! BSP was chosen over Quake 3.
//!
//! Controls: WASD move, mouse look, Space jump, Ctrl crouch, Escape to release
//! the cursor (click to re-grab). Aim at a ladder and press E to lock on — W/S
//! climb up/down, E or Space lets go. Aim at a crate and press RMB to carry
//! it; RMB again to place it, or hold RMB to charge a throw (slider under the
//! crosshair) and release to fling it. Tab opens the grid pack
//! (`src/inventory/`); number keys 1–9/0 pick the active quickbar slot (an
//! empty or already-active slot frees your hands), and it's drawn in your
//! hands. LMB uses the active item — a lockpick on a locked door (one use), a
//! crowbar on a wooden crate. E opens/closes a hinged door.
//!
//! The loop (Phases 10–16): the warehouse loads lit. Stack crates to reach
//! platform B, where a wooden crate and a crowbar sit. Carry the crate to the
//! deck edge and throw it off — it shatters and drops a lockpick. Pick both up
//! (Tab shows the pack), make the lockpick active (key `1`), LMB the east-wall
//! door to pick the lock — the lockpick is spent — then E to open it, and walk
//! the corridor → the bay → out through the large opening to the yard. The
//! platform-B wall switch still toggles the whole ceiling bank.

mod breakable;
mod carry;
mod classes;
mod config;
mod devtools;
mod door;
mod input;
mod interact;
mod items;
mod ladder;
mod lights;
mod pickup;
mod player;
mod trenchbroom;
mod ui;
mod use_item;
mod viewmodel;

use avian3d::prelude::*;
use bevy::prelude::*;
use bevy_ahoy::prelude::*;
use bevy_enhanced_input::prelude::*;
use bevy_game_bits::inventory::prelude::*;
use bevy_trenchbroom::physics::TrenchBroomPhysicsPlugin;
use bevy_trenchbroom::prelude::*;
use bevy_trenchbroom_avian::AvianPhysicsBackend;

fn main() {
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "Immersive".into(),
            ..default()
        }),
        ..default()
    }))
    .add_plugins(PhysicsPlugins::default())
    .add_plugins(EnhancedInputPlugin)
    .add_plugins(AhoyPlugins::default())
    .add_plugins((
        TrenchBroomPlugins(trenchbroom::config()),
        TrenchBroomPhysicsPlugin::new(AvianPhysicsBackend),
    ))
    // The player's pack (Phase 14) and its hotbar (Phase 15). `headless()` —
    // `spawn_pack` builds the one board with a non-default spec and keeps its
    // entity in `pickup::PlayerPack`.
    .add_plugins((InventoryPlugin::headless(), QuickbarPlugin))
    .add_input_context::<input::PlayerInput>()
    // Seeds the lit state (Phase 12 — the warehouse now loads with the main
    // lights on). `lights::sync_ambient` owns it from the first frame: it
    // stays `AMBIENT_LIT` while any switch is on, and drops to `AMBIENT_DARK`
    // if the platform-B switch is flipped off.
    .insert_resource(GlobalAmbientLight {
        brightness: config::AMBIENT_LIT,
        ..default()
    })
    .init_resource::<interact::InteractionFocus>()
    .init_resource::<pickup::PackFullFlash>()
    .init_resource::<player::CursorReleased>()
    .add_observer(player::spawn_player)
    .add_observer(door::setup_doors)
    .add_observer(door::open_on_interact)
    .add_observer(door::spawn_swing_doors)
    .add_observer(door::toggle_swing_on_interact)
    .add_observer(interact::fire_interact)
    .add_observer(pickup::spawn_visuals)
    .add_observer(pickup::collect_on_interact)
    .add_observer(pickup::tag_item_kind)
    .add_observer(viewmodel::spawn_viewmodel)
    .add_observer(use_item::fire_use)
    .add_observer(breakable::spawn_wood_crates)
    .add_observer(ladder::setup_ladders)
    .add_observer(ladder::attach_on_interact)
    .add_observer(ladder::let_go_on_jump)
    .add_observer(carry::spawn_crates)
    .add_observer(carry::start_grab_or_charge)
    .add_observer(carry::release_prop)
    .add_observer(lights::spawn_fixtures)
    .add_observer(lights::setup_switches)
    .add_observer(lights::toggle_on_interact)
    .add_systems(
        Startup,
        (
            spawn_map,
            spawn_pack,
            carry::setup_crate_assets,
            lights::setup_lamp_assets,
            door::setup_door_assets,
            breakable::setup_breakable_assets,
            ui::setup_hud,
        ),
    )
    .add_systems(
        Update,
        (
            player::player_cursor_input,
            player::sync_cursor_mode,
            interact::update_focus,
            carry::advance_charge,
            door::drive_doors,
            door::swing_doors,
            door::sync_lock_plates,
            (breakable::damage_on_impact, breakable::shatter).chain(),
            breakable::despawn_after,
            lights::sync_fixtures,
            lights::sync_switch_indicators,
            lights::sync_ambient,
            ui::update_prompt,
            ui::update_notice,
            ui::sync_hud_visibility,
            ui::update_charge_bar,
            (viewmodel::sync_viewmodel, viewmodel::bob_viewmodel).chain(),
        ),
    )
    // The yaw ease after an E-grab runs here, not in Update, so ahoy's
    // Update-schedule camera sync can't clobber it. See `ladder.rs`.
    .add_systems(
        PostUpdate,
        (ladder::turn_to_ladder, carry::hold_prop).before(TransformSystems::Propagate),
    )
    // Ladder climbing brackets bevy_ahoy's controller: read intent before it
    // runs, overwrite the result after. See `ladder.rs`.
    //
    // `stash_input` runs once per *frame* (not per fixed step): it `take`s
    // `AccumulatedInput::last_movement` and treats a missing value as
    // "released". `AccumulatedInput` has a one-frame lifetime — ahoy clears it
    // in `AfterFixedMainLoop` — so taking it here feeds every substep the same
    // intent, exactly as ahoy reads it. In `FixedPostUpdate` a second substep
    // would see `None` and stall the climb.
    .add_systems(
        RunFixedMainLoop,
        ladder::stash_input.in_set(RunFixedMainLoopSystems::BeforeFixedMainLoop),
    )
    .add_systems(
        FixedPostUpdate,
        ladder::climb
            .after(AhoySystems::MoveCharacters)
            .before(PhysicsSystems::First),
    );

    if std::env::var("IMMERSIVE_LIGHTS").as_deref() == Ok("toggle") {
        app.add_systems(Update, devtools::auto_toggle_switch);
    }

    if let Some(mode) = devtools::props_mode() {
        // Phase 10/13/16: fire `Interacted` at the hinged door / zero a
        // breakable's health / warp to the corridor door and pick its lock,
        // on a frame timer, so the door swing, crate shatter and LMB-use path
        // can be read in `IMMERSIVE_TELEMETRY` without a keyboard.
        let pick = mode == "pick";
        app.insert_resource(devtools::PropsMode(mode))
            .add_systems(Update, devtools::exercise_props);
        if pick {
            app.add_systems(
                PreUpdate,
                devtools::press_use_key
                    .after(bevy::input::InputSystems)
                    .before(EnhancedInputSystems::Update),
            );
        }
    }

    if let Some(mode) = devtools::inventory_mode() {
        // Phases 14–16: keyboard-free checks for the grid pack / quickbar /
        // consume path. `.before(InventorySet::Window)` so the `drag` mode's
        // hand-written `RelativeCursorPosition` (set after PreUpdate's
        // `ui_focus_system`) survives into that frame's inventory systems.
        app.insert_resource(devtools::InventoryMode(mode)).add_systems(
            Update,
            devtools::exercise_inventory.before(InventorySet::Window),
        );
    }

    if let Some(script) = devtools::autopilot_script() {
        // In PreUpdate, after bevy samples the keyboard/mouse and before
        // bevy_enhanced_input reads it, so the autopilot's held `KeyE` /
        // `MouseButton::Right` are seen the same frame. See
        // `devtools::autopilot_drive`. `IMMERSIVE_AUTOPILOT=1` runs the ladder
        // walk, `=crates` the crate grab/place/throw walk.
        app.insert_resource(script)
            .init_resource::<devtools::AutopilotAim>()
            .add_systems(
                PreUpdate,
                devtools::autopilot_drive
                    .after(bevy::input::InputSystems)
                    .before(EnhancedInputSystems::Update),
            )
            .add_systems(
                PostUpdate,
                devtools::autopilot_look.before(TransformSystems::Propagate),
            );
    }
    if devtools::env_flag("IMMERSIVE_TELEMETRY") {
        app.add_systems(Update, devtools::telemetry);
    }
    if devtools::env_flag("IMMERSIVE_SHOTS") {
        app.add_systems(Update, devtools::take_screenshot);
    }

    trenchbroom::register_classes(&mut app);

    app.run();
}

fn spawn_map(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.spawn(SceneRoot(
        asset_server.load(format!("{}#Scene", config::MAP_PATH)),
    ));
}

/// Builds the player's pack (a `src/inventory/` board, closed, centred) and
/// hangs a quickbar strip off it. The board entity goes in
/// `pickup::PlayerPack` — every other item/quickbar API takes it.
fn spawn_pack(mut commands: Commands) {
    let board = spawn_inventory(
        &mut commands,
        InventoryBoardSpec {
            config: InventoryConfig {
                cols: config::INVENTORY_COLS,
                rows: config::INVENTORY_ROWS,
                cell_px: config::INVENTORY_CELL_PX,
                title: Some("PACK".into()),
                ..default()
            },
            layout: InventoryLayout {
                horizontal: JustifyContent::Center,
                vertical: AlignItems::Center,
                ..default()
            },
            window: InventoryWindow { open: false },
            ..default()
        },
    );
    commands.entity(board).insert((
        Quickbar::new(config::QUICKBAR_SLOTS),
        ActiveSlot::default(),
        QuickbarStyle::default(),
    ));
    commands.insert_resource(pickup::PlayerPack(board));
}
