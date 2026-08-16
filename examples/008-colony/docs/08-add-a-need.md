# Add a pawn need

Worked example: `needs.rs` — the `Sleep` need. Read this before adding another
0-100 drive (Food, Recreation, ...) or before touching how Sleep decays,
recovers, or drives behavior.

The model, end to end (all three phases shipped — stat/gauge, sleeping
behavior, beds):

```
needs::decay_needs (sim-gated) --> Needs.sleep (0..100, per pawn)
        ^  drains awake/moving,        |
        |  refills while resting       v
        |  (ground or InBed rate)  ui::sync_needs_panel --> Sleep gauge
        |                          (selection HUD, pawn-only row)
        |
director::choose_objectives --> Objective::Sleep
  (needs::wants_sleep hysteresis: falls asleep at SLEEP_TIRED_THRESHOLD,
  wakes at SLEEP_RESTED_THRESHOLD; overrides Work/Recreation)
        |
        v
director::release_sleeping_pawns (drops the OTHER job in flight, not Sleep)
director::generate_sleep_jobs (nearest free reachable Bed, or nothing —
  ground fallback) --> JobKind::Sleep { bed } (personal, pre-assigned)
        |
        v
director::execute_jobs' Sleep arm (walk to the bed's footprint, then hold —
  inserts InBed on arrival) --> director::cleanup_jobs ends it on waking
        |
        v
director::sync_sleeping_pose (tips the pawn's Transform onto its side —
  only once actually stationary, whether on the ground or in a bed)
```

`Needs` is a plain component on every `Pawn` (attached in
`units::spawn_pawn_visual`). `needs::decay_needs` is the only writer of
`Needs.sleep` — it drains at a flat `SLEEP_DECAY_PER_SEC` while awake **or
still moving** (walking to a bed doesn't recover you — only actually resting
does, so a distant bed is a real cost), or refills while resting
(`Objective::Sleep`, stationary, not drafted): at `SLEEP_RECOVER_BED_PER_SEC`
if `director::InBed` marks the pawn as resting in a built bed, the slower
`SLEEP_RECOVER_GROUND_PER_SEC` otherwise (the ground fallback, or simply no
bed exists to walk to). Clamped to `[0, 100]` either way. It lives in the
sim-gated growth/weather block in `game.rs`
(`.run_if(in_state(SimState::Running))`), so pausing freezes it like
everything else in that block.

`needs::wants_sleep(sleep, was_sleeping) -> bool` is the pure hysteresis
`director::choose_objectives` calls to decide the `Sleep` transition: once
asleep, a pawn keeps sleeping until `sleep >= SLEEP_RESTED_THRESHOLD`; once
awake, it doesn't fall asleep until `sleep <= SLEEP_TIRED_THRESHOLD`. Without
this band, a value sitting exactly on one threshold would flicker the
objective every frame. `Sleep` takes priority over `Work`/`Recreation`
unconditionally — a pawn mid-job that becomes tired enough drops everything.
`director::release_sleeping_pawns` (mirrors `release_manual_pawns`) is what
actually returns that *other* in-flight job to the pool the same frame — it
deliberately exempts `JobKind::Sleep` itself, since `generate_sleep_jobs` may
have just spawned one this same frame and releasing it immediately would
undo that (see its doc comment for the one-frame-lag reasoning). With no
reachable free bed, the pawn simply stays put where it fell tired and
`decay_needs` recovers it there at the ground rate — beds
(`construction::BuildableKind::Bed`) just make that faster.
`director::sync_sleeping_pose` is a purely cosmetic, ungated follower that
tips a *resting* pawn's `Transform.rotation` onto its side and restores it the
moment it's moving again — see its pitfalls below before adding another
system that touches pawn rotation or translation.

## Checklist: adding a new need

1. A field on `needs::Needs` (e.g. `pub food: f32`), defaulted via
   `Needs::default()` alongside `sleep`.
2. A decay/recovery clause for it in `needs::decay_needs` (or a sibling
   system if its trigger conditions differ enough to be worth separating —
   Sleep's Phase 2 awake/asleep branch is the template for that).
3. Config consts for its start value and rate(s), mirroring the `SLEEP_*`
   block in `config.rs`.
4. A gauge row in the info panel. Today each need's row is hand-spawned in
   `ui::setup` and hand-wired in its own `sync_*` system (`NeedsSection`/
   `SleepGaugeFill`/`SleepGaugeValue` + `sync_needs_panel`), because Sleep is
   the only one. Once a second need lands, prefer generalizing: spawn one row
   per entry of `Needs::gauges()` (already returns `(label, value)` pairs for
   exactly this) and drive them from one generic sync system instead of
   copy-pasting a second hard-coded row + sync fn.
5. If the need should drive behavior (like Sleep's planned `Objective::Sleep`),
   see [03-add-a-job.md](03-add-a-job.md) for the objective/job pattern —
   `director::Objective` and `director::choose_objectives` are where a need
   crosses over from "a number that drains" to "a reason a pawn stops
   working."

## The gauge widget

There's no reusable progress-bar widget elsewhere in the codebase (growth %,
work progress, and battery charge are all rendered as text or a 3D material
swap instead). The needs gauge is a plain nested-`Node` bar: a fixed-size
track `Node` (`GAUGE_WIDTH`/`GAUGE_HEIGHT`, `GAUGE_TRACK_COLOR`) containing one
fill child whose `width: Val::Percent(value)` and color (`gauge_fill_color`,
`ui.rs` — lerps `GAUGE_FILL_LOW_COLOR` to `GAUGE_FILL_HIGH_COLOR`) are written
by the sync system every frame the value changes. Reuse this shape for any new
gauge rather than inventing another bar.

## Beds

`construction::BuildableKind::Bed(BedOrientation)` (see
[02-add-a-building.md](02-add-a-building.md)'s multi-cell section) is a
walkable 2-tile building, sized so a lying pawn (`PAWN_HEIGHT`) fits.
`director::JobKind::Sleep { bed }` (see [03-add-a-job.md](03-add-a-job.md)'s
personal/pre-assigned pattern, modeled on `Walk`) is what actually walks a
tired pawn there: `director::generate_sleep_jobs` picks the nearest bed not
already targeted by another live `Sleep` job (the live job entity *is* the
reservation — same `targeted`-set idiom `generate_jobs` uses for
Harvest/Cut/Haul/Build, no separate "occupied" component needed) and
reachable via `director::footprint_rest_spot` (unlike `footprint_stand_spot`,
which always stays *outside* a footprint, this one picks a cell *inside* it
— a sleeper rests on the bed, not beside it). `execute_jobs`' `Sleep` arm
walks to that cell, then inserts `director::InBed` on arrival and just holds
— open-ended, no timer; `cleanup_jobs` ends the job once the pawn wakes or
the bed is gone.

## Pitfalls

- **Gate decay/recovery on `SimState::Running`.** It belongs in the same
  sim-gated block as growth/weather (`game.rs`) — an ungated need would keep
  draining while the player is paused mid-order, unlike everything else in the
  world.
- **Guard the panel's per-frame writes.** `sync_needs_panel` only touches
  `Node.width`/`BackgroundColor`/`Text` when the wanted value actually differs
  from the current one (the repo-wide UI convention — see
  [05-add-ui.md](05-add-ui.md)'s pitfalls).
- **The panel is selection-scoped, not per-pawn-always-visible.** It reuses
  the `SelectedEntity` + pawn-only-row pattern (`sync_manual_checkbox` is the
  template), not the always-on Allowance-panel pattern — there's no plan to
  show every pawn's needs at once.
- **`Sleep` overrides everything, including drafted pawns' job state — but not
  their objective.** `choose_objectives` skips `ManualMode` pawns entirely (a
  drafted pawn's `Objective` is simply left as whatever it last was), so a
  tired draftee never flips to `Sleep` and never auto-recovers. Don't remove
  that skip without also deciding what "a drafted pawn falls asleep" should
  mean — right now it deliberately can't happen.
- **The pawn mesh is a tall rectangle, not a cube, specifically so this is
  visible.** `PAWN_WIDTH`/`PAWN_HEIGHT` (`config.rs`) feed `game.rs`'s
  `pawn_mesh`; rotating a symmetric cube 90 degrees is indistinguishable from
  not rotating it at all. If the mesh ever becomes a real character model,
  rotation keeps working (unlike a scale-based flatten, which was tried first
  and rejected — it would squash a model's body rather than lay it down).
- **"Resting" means `Objective::Sleep` *and stationary* (no `Path`/
  `MoveOrder`), not `Objective::Sleep` alone.** Beds mean a tired pawn can be
  mid-walk to one; both `needs::decay_needs` (which rate to apply) and
  `director::sync_sleeping_pose` (whether to tip the pawn over) gate on this
  narrower condition. Gating on `Objective::Sleep` alone would tip a pawn over
  — or recover it — while it's still visibly walking.
- **`sync_sleeping_pose` only touches `translation.y` while the pawn isn't
  moving**, to keep the tipped-over box's belly flush with the ground
  (rotating 90 degrees around Z swaps its vertical half-extent from
  `PAWN_HEIGHT / 2` to `PAWN_WIDTH / 2`, so resting at the standing height
  would float it) — and backs off entirely the instant a `Path`/`MoveOrder`
  exists, deferring fully to `movement::plan_paths`/`move_along_path` (which
  own that field while walking, including the Wader dip through water).
  Touching it unconditionally would fight those systems the moment a Sleep
  job's walk-to-bed leg crosses water.
