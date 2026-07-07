# 007-turret — Game Spec

A small tower defense: place turrets on a desert field, stop the waves marching
toward you, survive all 10 waves.

## Rules

- **Win**: wave 10 is fully resolved (every spawned enemy dead or leaked) with
  Base HP above zero.
- **Lose**: Base HP reaches 0. The base starts at **10 HP**; every enemy that
  leaks past the near edge deducts its `leak_cost` (1 for most, 3 for brutes).
- **Flow**: the game starts in a **build phase** (12 s, `Space` skips) so you
  can place your opening turrets. Each wave spawns its groups on a schedule and
  ends only when the field is clear; the wave reward is granted, then the next
  build phase begins. After victory or defeat, `R` restarts from scratch.

## States

```
             timer or Space            all resolved & wave < 10
  Building ────────────────► WaveActive ────────────────► Building
     ▲                           │  │
     │        R                  │  └─ all resolved & wave == 10 ─► Victory ─R─┐
     └───────────────────────────┼─────────────────────────────────────────────┤
                                 └─ Base HP == 0 ─► Defeat ────────────R───────┘
```

Turrets keep sweeping and may be placed during both `Building` and
`WaveActive`. Everything freezes on `Victory`/`Defeat`.

## Economy

- Start with **100 materials**.
- Kinetic turret costs **20**, laser turret costs **40**. Placement is rejected
  (HUD flashes red) if you can't afford it.
- Each cleared wave pays a reward (see wave table). Total available over a full
  run: 100 + 665 = **765 materials**.

## Turrets

| Turret  | Cost | Damage model                                | Sustained DPS |
|---------|------|---------------------------------------------|---------------|
| Kinetic | 20   | 10 rounds/s × 5 dmg, magazine 30, reload 2.5 s | ~27 (50 burst × 3s/5.5s duty cycle) |
| Laser   | 40   | 120 DPS hitscan, continuous while locked     | 120           |

Kinetic is the cheap early filler and coverage buy; laser is the big
single-purchase commitment (3× the DPS per material, but 40 up front).
The kinetic magazine empties in 3 s of continuous fire, then the turret
reloads for 2.5 s (its cone turns gray). Reload progresses even without a
target.

Shared sensor: 24 range, 8° half-angle cone, ±60° sweep at 65°/s.

## Enemy archetypes

| Archetype | HP  | Speed | Cross time | Leak cost | Scale | Color        |
|-----------|-----|-------|------------|-----------|-------|--------------|
| Grunt     | 100 | 2.0   | 15 s       | 1         | 1.0   | teal-green   |
| Runner    | 50  | 4.0   | 7.5 s      | 1         | 0.7   | yellow-green |
| Brute     | 400 | 1.2   | 25 s       | 3         | 1.6   | dark red     |

All share the cuboid mesh; scale applies to visuals, hitbox radius and spawn
height.

## Wave table

Groups are `archetype×count @start_delay /spawn_interval` (seconds). A wave is
defined purely as data in `waves.rs` (`WAVES`); no per-wave logic.

| Wave | Groups                                                                | Total HP | Reward |
|------|-----------------------------------------------------------------------|----------|--------|
| 1    | Grunt×5 @0 /1.5s                                                      | 500      | 40     |
| 2    | Grunt×5 @0 /1.2s · Grunt×5 @6 /1.2s                                   | 1000     | 50     |
| 3    | Grunt×6 @0 /1.2s · **Runner×4** @4 /0.8s                              | 800      | 55     |
| 4    | Grunt×8 @0 /1.0s · Runner×6 @5 /0.7s                                  | 1100     | 65     |
| 5    | **Brute×1** @0 · Grunt×8 @2 /1.0s                                     | 1200     | 75     |
| 6    | Brute×2 @0 /6s · Runner×8 @3 /0.6s · Grunt×6 @8 /1.0s                 | 1700     | 80     |
| 7    | Grunt×12 @0 /0.8s · Runner×8 @6 /0.5s                                 | 1600     | 90     |
| 8    | Brute×3 @0 /5s · Grunt×10 @3 /0.8s                                    | 2200     | 100    |
| 9    | Runner×10 @0 /0.5s · Brute×2 @4 /6s · Grunt×10 @8 /0.8s               | 2300     | 110    |
| 10   | Brute×2 @0 /4s · Grunt×12 @2 /0.6s · Runner×10 @8 /0.4s · Brute×2 @14 /4s | 3300 | —      |

Runners debut on wave 3, brutes on wave 5. Wave 10 opens and closes with brute
pairs sandwiching a dense grunt/runner rush.

## Controls

| Input   | Action                       |
|---------|------------------------------|
| LMB     | Place kinetic turret (20)    |
| RMB     | Place laser turret (40)      |
| `Space` | Start the wave early         |
| `R`     | Restart (from Victory/Defeat) |

## Tuning

Every number above lives in `config.rs` or `waves.rs` as a constant. This table
is the reference; keep it in sync when balancing.
