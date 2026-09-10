//! All TrenchBroom entity classes for this example, and nothing else — meant
//! to be read as a single reference for what a level author can place.

use bevy::prelude::*;
use bevy_trenchbroom::fgd::IntBool;
use bevy_trenchbroom::prelude::*;

/// Where the player starts. The entity spawned from the map *becomes* the
/// player: `player::spawn_player` observes `On<Add, PlayerSpawn>` and bolts
/// the character controller onto it. The origin is the capsule's vertical
/// center, matching how `Collider::cylinder` is centered on `Transform`.
#[point_class(base(Transform), color(0 255 0), size(-16 -16 -36, 16 16 36))]
pub struct PlayerSpawn;

/// Base class for anything the player's "use" raycast can target.
/// `interact.rs` matches on this to know what to show a prompt for.
#[base_class]
#[derive(Default)]
pub struct Interactable {
    /// Text shown in the centered prompt, e.g. "Open door".
    pub prompt: String,
}

/// A brush door that slides `lip`-short of its own bounds along `angle`
/// when used, waits `wait` seconds, then slides back. Quake `func_door`
/// semantics.
#[solid_class(base(Interactable))]
pub struct FuncDoor {
    /// Yaw in degrees the door travels toward (0 = +X, 90 = +Y, ...).
    pub angle: f32,
    /// Travel speed in meters per second.
    pub speed: f32,
    /// How much of the door stays visible when fully open, in meters.
    pub lip: f32,
    /// Seconds open before auto-closing. Negative means it stays open.
    pub wait: f32,
}
impl Default for FuncDoor {
    fn default() -> Self {
        Self {
            angle: 0.0,
            speed: 2.0,
            lip: 0.2,
            wait: 3.0,
        }
    }
}

/// A ladder volume. The brush's collider is turned into a `Sensor` at
/// runtime (`ladder::setup_ladders`) — the *volume* never blocks movement, it
/// only marks the column of space in which the player is "on the ladder" and
/// supplies the ladder's AABB. The visible rails-and-rungs mesh that
/// `setup_ladders` spawns as a child *does* carry a solid collider, so you
/// can't walk through the ladder itself. Being
/// `Interactable`, aiming at it shows a prompt and RMB/E grabs it, which is how
/// you mount from the top edge (where you're facing away from the rungs and
/// the proximity grab deliberately won't fire).
#[solid_class(base(Interactable))]
pub struct FuncLadder {
    /// Yaw in degrees the climber faces while on the ladder — toward the
    /// platform it's bolted to. Same numbers as a Quake `angle`
    /// (0 = +X, 90 = +Y, ...), but deliberately **not** named `angle`:
    /// bevy_trenchbroom treats a field literally called `angle` as a brush
    /// rotation and flings the geometry across the room. `ladder.rs` turns
    /// this into a facing direction itself.
    pub face_yaw: f32,
}
impl Default for FuncLadder {
    fn default() -> Self {
        Self { face_yaw: 0.0 }
    }
}

/// A carryable metal crate. `carry::spawn_crates` gives it a dynamic body, a
/// cuboid collider and its painted-metal look; `carry.rs` handles the RMB
/// grab/carry/throw. `mass` (kg) is both the crate's real physics mass **and**
/// the grab gate: RMB lifts a crate only at or under `config::CARRY_MAX_MASS`,
/// so a level author makes one an immovable step just by giving it a big
/// number (the crate under platform B is ~800 kg). `size` sets the cube edge —
/// the same base crate is bigger than the loose ones you stack on it. Static
/// brushes (ladders, walls) and kinematic bodies (doors) can't be grabbed at
/// all — wrong `RigidBody` type, checked before mass.
#[point_class(base(Transform, Interactable))]
pub struct PropCrate {
    /// Physical mass in kilograms. Also the lift gate (see the type doc).
    pub mass: f32,
    /// Cube edge length in meters. Defaults to `config::CRATE_SIZE` (0.8 m).
    pub size: f32,
}
impl Default for PropCrate {
    fn default() -> Self {
        Self {
            mass: crate::config::CRATE_MASS,
            size: crate::config::CRATE_SIZE,
        }
    }
}

/// A wall-mounted light switch. Aiming at it shows its prompt; RMB/E flips every
/// [`LightFixture`] whose `targetname` matches this switch's `target`
/// (Quake entity-IO, via bevy_trenchbroom's built-in [`Target`]/[`Targetable`]
/// base classes — `lights.rs` does the dispatch since 0.13's IO is a
/// data-only skeleton). Its brush is `skip`-textured (invisible); `lights.rs`
/// draws the plate + indicator from the brush's AABB, like the ladder.
#[solid_class(base(Interactable, Target))]
pub struct FuncLightSwitch {
    /// Whether this circuit is energised when the map loads.
    pub start_on: IntBool,
}
impl Default for FuncLightSwitch {
    fn default() -> Self {
        Self {
            start_on: IntBool(false),
        }
    }
}

/// A warehouse lamp — a ceiling-hung lamp (stem + cone shade + downward
/// `SpotLight`) by default, or a raked pillar/wall bracket when `aim` names a
/// compass direction. `lights::spawn_fixtures` builds it; a [`FuncLightSwitch`]
/// toggles it by `targetname`. Deliberately **no** `angle`-family field (the
/// `FuncLadder::face_yaw` trap) — `aim` is a plain string the code interprets.
#[point_class(base(Transform, Targetable))]
pub struct LightFixture {
    /// `SpotLight::intensity` in lumens.
    pub intensity: f32,
    /// `SpotLight::range` in metres.
    pub range: f32,
    /// Full cone angle in degrees (split into inner/outer for a soft edge).
    pub cone_deg: f32,
    /// Lamp tint.
    pub color: Color,
    /// Whether this lamp casts real-time shadows (keep the count tiny).
    pub shadows: IntBool,
    /// Whether this lamp is lit when the map loads.
    pub start_on: IntBool,
    /// Which way the lamp points: `"down"` (default — a ceiling-hung lamp) or
    /// a compass direction (`"north"`/`"south"`/`"east"`/`"west"` — a bracket
    /// bolted to a pillar/wall and raked 45° down toward that heading, in
    /// TrenchBroom's +Y-north axes).
    pub aim: String,
}
impl Default for LightFixture {
    fn default() -> Self {
        Self {
            intensity: crate::config::LAMP_INTENSITY,
            range: crate::config::LAMP_RANGE,
            cone_deg: crate::config::LAMP_CONE_DEG,
            color: crate::config::LAMP_COLOR,
            shadows: IntBool(crate::config::LAMP_SHADOWS),
            start_on: IntBool(false),
            aim: "down".to_string(),
        }
    }
}

/// A collectable. `pickup::spawn_visuals` looks `item` up in
/// [`crate::items::CATALOGUE`] for a procedural model + colour (an unknown key
/// falls back to the placeholder emissive cube), and adds a `Sensor` grab
/// volume. Swap the model for a glTF `SceneRoot` later.
#[point_class(base(Transform, Interactable))]
#[derive(Default)]
pub struct ItemPickup {
    /// Catalogue key, e.g. "lockpick" / "crowbar".
    pub item: String,
}

/// A hinged door — a "regular" swinging door, as opposed to the Quake
/// [`FuncDoor`] slider. **A `point_class`, and its origin is the HINGE, not the
/// leaf centre.** `door::spawn_swing_doors` builds the leaf as a child offset
/// `+width/2` along local +X and swings the door by rotating *this entity's*
/// `Transform` about +Y — which pivots about the hinge only because a point
/// entity has a real `Transform`. A `solid_class` would have an identity
/// `Transform` with its geometry baked in world space (the `FuncLightSwitch` /
/// `FuncLadder` case), and rotating that pivots the door about the world
/// origin. See SPEC.md Phase 10.
#[point_class(base(Transform, Interactable))]
pub struct PropDoor {
    /// Leaf width in metres (hinge to latch).
    pub width: f32,
    /// Leaf height in metres.
    pub height: f32,
    /// Leaf thickness in metres.
    pub thickness: f32,
    /// Yaw in degrees the closed leaf points toward (hinge → latch), the Quake
    /// way (0 = +X, 90 = +Y, ...). Deliberately **not** `angle` — the
    /// [`FuncLadder::face_yaw`] trap. `door.rs` turns it into a rotation.
    pub face_yaw: f32,
    /// Degrees the leaf swings open; the sign picks the direction.
    pub swing: f32,
    /// Swing speed in degrees per second.
    pub speed: f32,
    /// Mechanically locked: RMB/E refuses to open it, and the lock plate glows red.
    /// A lockpick clears it (Phase 16). A level author sets `"locked" "1"`.
    pub locked: IntBool,
}
impl Default for PropDoor {
    fn default() -> Self {
        Self {
            width: crate::config::DOOR_WIDTH,
            height: crate::config::DOOR_HEIGHT,
            thickness: crate::config::DOOR_THICKNESS,
            face_yaw: 0.0,
            swing: crate::config::DOOR_SWING_DEG,
            speed: crate::config::DOOR_SPEED_DEG,
            locked: IntBool(false),
        }
    }
}

/// A breakable wooden crate that holds one item. `breakable::spawn_wood_crates`
/// gives it a `Dynamic` body light enough that `carry.rs` lifts and throws it
/// unchanged — dropping it off a deck onto the concrete is the intended way to
/// open it. When its `health` reaches zero it shatters into debris and spawns
/// `contains` as an [`ItemPickup`]. See SPEC.md Phase 10.
#[point_class(base(Transform, Interactable))]
pub struct PropWoodCrate {
    /// Cube edge length in metres.
    pub size: f32,
    /// Physical mass in kilograms. Must stay at or under
    /// `config::CARRY_MAX_MASS` to remain liftable.
    pub mass: f32,
    /// Starting health, in the same units as `config::BREAK_*` /
    /// `config::CROWBAR_DAMAGE`.
    pub health: f32,
    /// Catalogue key of the item spawned when it breaks, e.g. "lockpick".
    pub contains: String,
}
impl Default for PropWoodCrate {
    fn default() -> Self {
        Self {
            size: crate::config::WOOD_CRATE_SIZE,
            mass: crate::config::WOOD_CRATE_MASS,
            health: crate::config::WOOD_CRATE_HEALTH,
            contains: String::new(),
        }
    }
}
