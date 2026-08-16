# Bevy Game Bits

Welcome!

This repository contains my experiments with the Bevy Engine. My goal is to learn how to use the engine by building simple games, after building precise game mechanics.

## Running examples

You'll need to install Rust. The default toolchain should work.

You can then run any example with this command:
```sh
cargo run --example 004-infinite-runner
```

## Reusable bits

A few mechanics have graduated out of the examples into the library, so they
can be dropped into another project:

- `bevy_game_bits::jump` — a configurable parabolic jump (from `000-jump`).
- `bevy_game_bits::vehicle` — a raycast-suspension car for Avian 3D:
  spring/damper suspension, a slip-based tire model with a friction circle,
  Ackermann steering, flip rescue, plus optional skid marks and a
  collision-damage pipeline. Extracted from `009-derby`;
  `012-vehicle` is the minimal example of using it.

## Track

I try to follow this path, allowing myself to work on secondary topics from time to time.

First column below lists topics that are essential for the next milestone marked by a `+` (plus) sign.
The second one contains random topics that I simply wanted to work on after a shower thought!

```
|  Jump
 | Load GLTF
 | Input Pad
+  Infinite Runner
|  Wireframe Graphics
|  Projectiles (Fixed Circular Trajectory, Straight Line...)
|  Projectiles (Ricochets, Homing Missile, Grenades...)
|  Turret
|  Game Feel (Screen Shake, Hit Feedback)
|  Vehicle Physics (Raycast Suspension, Tire Model)
|  Procedural Spawns
|  Rounds
|  Character Progression
+  Survivor
```