# The derby car models

Each car **class** (`config::CarClass` — `Truck`, `Buggy`) has its own glTF:
`assets/models/derby-car.glb` (Truck) and `assets/models/derby-buggy.glb`
(Buggy). Together they're the source of truth for every car's shape, paint,
and part rest-poses. `model.rs` reads both at startup into one `CarRigs`
resource; nothing about a car's *look* lives in Rust any more (the
colliders, suspension raycasts, and immunity bubble still do — see "What
stays in code" below).

Both files share **the same node contract** below — that's what lets
`model.rs`'s parsing stay completely class-agnostic; only the sizes and
rest poses baked into the nodes differ. Regenerate either one from its
class's `CarSpec` (`config.rs`) with:

```
cargo run --example 009-derby -- export-model <truck|buggy> [path] [--force]
```

This refuses to overwrite an existing file unless `--force` is passed, so
it can't clobber your edits by accident. You'll only need it once per
class, to get a starting point — from there, edit the `.glb` directly
(Blender opens it fine) and the game picks up the change on next launch. No
re-export step.

## The node contract

The game looks up parts **by node name**, flat under the scene root:

```
Chassis
Wheel_FL   Wheel_FR   Wheel_RL   Wheel_RR
Headlight_R   Headlight_L
Fender_R      Fender_L
SpoilerStrut_R   SpoilerStrut_L
ShieldStrut_R    ShieldStrut_L
Windshield
Spoiler
RamBar
```

Every node must have exactly one mesh primitive with a material assigned —
the loader panics (naming the missing node) if one isn't found, rather than
spawning a half-built car.

Each `Wheel_*` node needs **exactly one child node**, holding the mesh for
its spin stripe. That child can be named anything — it isn't looked up by
name (four nodes all named "Stripe" would collide in the file's flat
node-name map), just "the wheel's one child."

## What you can freely change

- **Shape.** Reshape, resize, or add detail to any part's mesh. A resized
  fender's debris (when it's rammed off in-game) is sized to match — the
  game reads the mesh's own bounding box, not a hardcoded number.
- **Material look.** Base color, roughness, metallic — all carry through.
  (Headlight emissive intensity survives via the `KHR_materials_emissive_strength`
  extension; a viewer without it just shows the un-intensified color.)
- **Rest pose.** A part's position and rotation in the file *is* its
  in-game rest pose, wheels included (a wheel's rotation is where its
  suspension travel and steering compose on top of). The wheel nodes'
  positions are also where `model::CarRig::wheelbase()`/`track_width()`
  read the steering model's geometry from — there's no separate
  `config.rs` constant to keep in sync; move the wheels and the Ackermann
  math follows.
- **Which side a part is on.** Sides aren't read from the `_L`/`_R`
  suffix — those exist only because node names must be unique across the
  whole file. Damage attribution and wheel steering are both derived from
  the node's own translation (negative X vs. positive X for sides,
  positive Z vs. negative Z for front/rear wheels). Move
  `Headlight_L` to the other side of the car and it takes damage from
  hits on *that* side, not the side its name implies.

## What you can't change here

- **Chassis paint color** is set per car in code (`vehicle.rs`, one blue,
  three others per `config::DRIVERS`) by cloning the `Chassis` node's
  material and overriding just its base color — so whatever
  roughness/metallic you give it survives, but the color itself is
  overridden per car at spawn time. Fenders and the spoiler wear that same
  per-car paint.
- **Colliders, suspension, and physics** are untouched by this file. The
  chassis collider size, wheel radius, mount ray length, and every tuning
  number live in `config::CarSpec` (one per class) — the *shapes* you edit
  here should stay in proportion with them, but nothing here drives them.
- **The immunity-bubble sphere** is still built in code — it's a gameplay
  effect, not part of the car's look, so there's nothing here for it to
  read.

## Orientation

**+Z is forward** in this rig (the headlights sit at `z ≈ +1.8`, the ram
bar just ahead of them). This is the opposite of the glTF/Blender
convention (−Z forward) — the car will look like it's facing backwards on
import. That's correct; don't rotate it to "fix" that.

## Why not `SceneRoot`

The game doesn't spawn this file as a scene. It reads the glTF node graph
directly (`Gltf::named_nodes`, `GltfNode`, `GltfMesh`) and rebuilds its own
existing entity hierarchy from it — same flat parent/child structure, same
per-car paint, same per-headlight material instances the damage system
dims individually. That's what makes ramming, detaching, and repairing
parts keep working exactly as before; the model just supplies the shapes.
