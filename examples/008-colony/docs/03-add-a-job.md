# Add a new job kind and/or work type

Two separable layers:

- **A job kind** (`director::JobKind` variant): a new unit of work pawns can
  perform. Worked example: `Cut` (the first timed job).
- **A work type** (allowance column + skill): only needed when the new job
  isn't already covered by an existing skill — `Merge` and `Supply` reuse
  `haul`; `BuildRoof`/`RemoveRoof` reuse `build`. Worked example: `forestry`.

Jobs are entities: `Job { kind }` + `JobPriority(u32)` (lower = more urgent),
optionally `AssignedTo(Entity)`, `Stuck(Timer)`, `WorkProgress(Timer)`. The
pawn points back with `CurrentJob(Entity)`.

## Checklist: new job kind

1. Priority const in `config.rs` (see the commented block around
   `HARVEST_JOB_PRIORITY`; real work is 10, finishing work 15, housekeeping
   20, recreation 100).
2. `JobKind` variant in `director.rs`. Carry what execution needs (target
   `Entity` and/or `GridCoords`); capture data at generation time if the
   source can change meanwhile (as `Farming` captures its `ZonePlant`).
3. Add the variant, run `cargo check`, and follow the compiler through the
   exhaustive matches:
   - `generate_jobs`: the `targeted` set (does your variant reserve a target
     entity?) — plus your **spawning logic** (see below).
   - `cleanup_jobs`: the `done` arm — when is the job finished or void?
   - `assign_jobs`: `(target cell, skill)` for distance/skill scoring.
   - `execute_jobs`: the state-machine arm — walk, act, or mark `Stuck`.
   - `update_pawn_status`: the human-readable activity strings.
   - `units::Allowance::allows`: which allowance gates it.
   - `debug_ui::update_jobs_tab`: the Jobs-tab line.
4. Generation logic in `generate_jobs`: spawn one job per eligible source,
   skipping sources already in the `targeted`/pending set (the compiler can't
   write this part). Copy the pattern that matches your shape:
   - **Per designated entity** (Harvest/Cut/Demolish): a `ToX` marker
     component on the target generates the job; the Cancel button removes the
     marker and `cleanup_jobs` voids the job. Add your `ToX` insertion path
     (guide 05). `Demolish` is the footprint-based variant of this shape:
     `ToDemolish` sits on a built structure (any `Footprint`-carrying entity,
     not just a single-cell target), so its execution arm copies Build's
     footprint helpers (`footprint_stand_spot`, `adjacent_to_footprint`)
     rather than Cut's single-cell `nearest_stand_spot`.
   - **Per map-state gap** (BuildRoof/RemoveRoof, Farming): derive jobs by
     scanning a resource/zone; no stored designations to desync.
   - **Singleton** (Merge, Supply): at most one alive; the job itself is the
     reservation.
   - **Pre-assigned personal** (Walk): spawned in `generate_walk_jobs` with
     `AssignedTo` already set; never enters the open pool.
5. Verify in-game: backtick → Jobs tab shows the pool live.

## Execution-arm patterns (`execute_jobs`)

The system early-outs while the pawn still has `Path`/`MoveOrder`, so each
arm only handles "arrived (or not yet dispatched)":

- **Walk to the work spot**: `nearest_stand_spot(&nav, from, target, include_target)`
  picks the reachable 4-neighbor with the shortest path. Pass
  `include_target = false` for work done *beside* the target (Harvest, Cut,
  Build — the target cell is or becomes blocked) and `true` for work done on
  a possibly-standable cell (roofing). Insert `MoveOrder { goal }` and let
  `movement` take over.
- **Unreachable / undroppable**: insert `Stuck::new()` on the **job** and
  remove `AssignedTo` (+ `WorkProgress` if timed), remove the pawn's
  `CurrentJob`. `tick_stuck` returns it to the pool after `STUCK_RETRY_SECS`.
- **Timed work**: on first arrival insert
  `WorkProgress(Timer::from_seconds(...))` on the **job entity**; on later
  frames tick it and act when finished. It lives on the job (not the pawn)
  but is removed on every release (`enforce_allowances`, stuck paths) so a
  new pawn restarts from zero — that's intentional.
- **Acting**: either do it inline (Farming plants directly) or write a
  message consumed by the owning module (`HarvestCommand`, `CutCommand`,
  `BuildCommand`, `RoofCommand`) — prefer a message when completion mutates
  maps/visuals owned elsewhere, and keep its reader ungated (see
  architecture guide, pause-gating).
- **Recording history**: at the exact same point you decide the job is
  genuinely done (not voided/canceled), also `completed.write(JobCompleted {
  pawn, label })` (`history::JobCompleted`, bundled into `execute_jobs`'s
  `construction_state` tuple param). `history::record_history` appends it to
  the pawn's `JobHistory`, shown in the selection HUD's History panel (guide
  05). This must live in `execute_jobs`, not `cleanup_jobs` — see the
  Pitfalls entry below. Skip the write entirely for jobs that would just add
  noise: recreation (`Walk`) and the whole haul family (`Haul`, `Merge`,
  `Supply` — they fire constantly during normal play). Not writing is the
  entire mechanism; there's no separate filter list to keep in sync.
- **Producing items**: pre-check `items::find_drop_cell` before working
  (Harvest/Cut do) so pawns don't work for a yield with nowhere to land;
  drop via `items::pour_yield`.
- **Carrying**: if the pawn picks things up, insert `Carrying` and remove
  `CurrentJob` — `deliver_carried` owns the delivery leg entirely, keyed on
  the component so it survives the job entity.

**Release invariant**: every path that detaches a pawn from a job must
remove `CurrentJob` (and `Path` + `MoveOrder` if the pawn should stop —
`cleanup_jobs`, `enforce_allowances`, and `release_manual_pawns` (drafting a
pawn into `units::ManualMode`) show the full removal). A missed removal
wedges the pawn as permanently "busy".

## Checklist: new work type (allowance + skill)

Worked example: `forestry`. Only for jobs no existing skill covers.

1. Field on `units::Skills` and on `units::Allowance` (+ its `Default`,
   which is all-true).
2. Map your `JobKind` to it in `Allowance::allows`.
3. Skill source: field read in `units::skills_from_fields`
   (`tier("ForestrySkill")`), and — **manually, in the LDtk editor** — the
   matching String field on the Pawn entity definition in
   `assets/maps/colony.ldtk`. An undeclared field does not error: it silently
   defaults to tier C. (`FarmSkill`/`ForestrySkill` are currently in exactly
   that state — code reads them, the map never defines them.)
4. Assignment scoring: use `skills.x` in the `assign_jobs` arm (ties on
   priority break by best skill tier, then distance).
5. UI, all in `ui.rs`: entry in `ToggleWork::ALL` (the allowance grid derives
   its header and per-pawn checkbox columns from this one list) + variant
   arms in `toggle_allowance` and `sync_allowance_buttons`; extend the skill
   line in `update_panel` if it should show on the pawn info panel.

## Pitfalls

- **`choose_objectives` is automatic** — it counts any free `is_work()` job
  the pawn's allowance permits. But check `JobKind::is_work()` if your job is
  recreation-like (only `Walk` returns false today).
- **Don't gate generation on pawn availability.** Jobs exist independently;
  unclaimable ones just sit in the pool (visible in the debug window). The
  exceptions are pre-assigned personal jobs.
- **Void, don't complete, on world change.** `cleanup_jobs` is the single
  place that decides a job is over; execution arms should bail (remove
  `CurrentJob`, `continue`) when their target query fails and let cleanup
  despawn the job. Both `Cut` and `Build` show the pattern.
- **`cleanup_jobs`'s despawn fires for cancels too, not just completions** —
  it's "finished or void" in one branch. Don't write `JobCompleted` there;
  write it only at the execution-arm site where the work actually finishes
  (see "Recording history" above), or a canceled order gets logged as done.
- **Deferred commands race generation.** `generate_jobs` sees last frame's
  world; guard your act-step against duplicates the way Farming's `taken`
  check does if two paths could produce on the same cell/target.
- **Priorities are deliberately flat** (10 for real work) so skill tiers
  break ties. Slot new work into the existing bands rather than inventing
  finer-grained numbers.
