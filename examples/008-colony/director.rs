use std::collections::HashSet;

use bevy::prelude::*;
use bevy_ecs_ldtk::prelude::*;

use crate::config::*;
use crate::construction::{
    self, Bed, Blueprint, BuildCommand, DemolishCommand, Footprint, RoofCommand, ToDemolish,
};
use crate::crops::{self, Crop};
use crate::flora::{BerryBush, HarvestCommand, ToHarvest};
use crate::game::GameAssets;
use crate::history::JobCompleted;
use crate::items::{self, ItemKind, ItemStack};
use crate::map::{self, RoofMap, Stockpile, TerrainMap};
use crate::movement::{MoveOrder, Path, Wader};
use crate::nav::NavGrid;
use crate::needs::{wants_sleep, Needs};
use crate::selection::Selectable;
use crate::shrubs::Shrub;
use crate::trees::{self, CutCommand, ToCut, Tree};
use crate::units::{Allowance, ManualMode, Pawn, Skills};
use crate::zones::{GrowOrder, ZonePlant, ZoneRegion};

/// A unit of work, spawned by the Director and claimed by idle pawns.
#[derive(Component)]
pub struct Job {
    pub kind: JobKind,
}

#[derive(Clone, Copy)]
pub enum JobKind {
    /// Harvest a bush or a crop — whichever plant `target` is.
    Harvest {
        target: Entity,
    },
    Haul {
        stack: Entity,
    },
    /// Consolidate the stockpile: pour this (small) stack into the others.
    Merge {
        stack: Entity,
    },
    /// Recreation: stroll to a random nearby cell.
    Walk {
        goal: GridCoords,
    },
    /// Plant on this empty cell of a Grow-enabled growing zone. The plant
    /// kind is captured at generation so execution needs no zone lookup.
    Farming {
        cell: GridCoords,
        plant: ZonePlant,
    },
    /// Fell a tree marked `ToCut` — forestry, deliberately separate from
    /// the Harvest path. The first timed job: the pawn works next to the
    /// tree for `TREE_CUT_SECS` (see `WorkProgress`) before it falls.
    Cut {
        target: Entity,
    },
    /// Batched wood run for construction: fetch a load from the nearest
    /// wood stack, top up needy blueprints one by one, and let
    /// `deliver_carried` walk any leftover back to the stockpile. At most
    /// one in flight (the Merge model) — the singleton is the reservation.
    Supply,
    /// Raise a fully supplied blueprint, the builder standing next to it
    /// (timed, `BuildableKind::build_secs`).
    Build {
        site: Entity,
    },
    /// Tear down a structure marked `ToDemolish` — construction work in
    /// reverse, sharing the Build skill/allowance. The pawn works next to
    /// its footprint for `DEMOLISH_SECS` before it comes down, salvaging
    /// `DEMOLISH_REFUND_FRACTION` of its wood cost.
    Demolish {
        site: Entity,
    },
    /// Install a roof on this Roof-stamped cell, standing on it (timed,
    /// fast, no materials).
    BuildRoof {
        cell: GridCoords,
    },
    /// Tear the roof off this NoRoof-stamped cell, standing on it (timed).
    RemoveRoof {
        cell: GridCoords,
    },
    /// Personal, pre-assigned like `Walk`: walk to `bed` and rest there,
    /// recovering `Needs::sleep` faster than the ground fallback. Open-ended
    /// (no timer) — `cleanup_jobs` ends it once the pawn wakes or the bed is
    /// gone. The live job entity referencing `bed` *is* the reservation
    /// (`generate_sleep_jobs` skips any bed already targeted), the same
    /// pattern `generate_jobs`' `targeted` set uses for Harvest/Cut/Haul/Build.
    Sleep {
        bed: Entity,
    },
}

impl JobKind {
    /// Real work, as opposed to recreation (or a biological need).
    pub fn is_work(&self) -> bool {
        !matches!(self, JobKind::Walk { .. } | JobKind::Sleep { .. })
    }
}

/// What a pawn wants to be doing. Derived every frame for now (work when
/// there is work, recreation otherwise, sleep overriding both once tired
/// enough); a schedule can replace this later.
#[derive(Default, Component, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Objective {
    #[default]
    Work,
    Recreation,
    Sleep,
}

impl Objective {
    pub fn label(&self) -> &'static str {
        match self {
            Objective::Work => "Work",
            Objective::Recreation => "Recreation",
            Objective::Sleep => "Sleep",
        }
    }
}

/// Xorshift64 for stroll destinations — hand-rolled like the A*, no `rand`
/// dependency needed for a prototype.
#[derive(Resource)]
pub struct WanderRng(u64);

impl Default for WanderRng {
    fn default() -> Self {
        Self(0x9E37_79B9_7F4A_7C15)
    }
}

impl WanderRng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// Uniform-ish integer in `min..=max`. Shared by strollers and trees.
    pub fn range(&mut self, min: i32, max: i32) -> i32 {
        min + (self.next() % (max - min + 1) as u64) as i32
    }

    /// One biased coin flip: true with probability `p` (clamped 0..=1).
    /// The top 53 bits give a uniform float in [0, 1).
    pub fn chance(&mut self, p: f32) -> bool {
        ((self.next() >> 11) as f64 / (1u64 << 53) as f64) < p as f64
    }
}

/// Breather between two stroll legs; removed when it finishes.
#[derive(Component)]
pub struct WanderCooldown(pub Timer);

/// Lower = more urgent; ties broken by pawn-to-target distance.
#[derive(Component)]
pub struct JobPriority(pub u32);

/// The pawn working this job.
#[derive(Component)]
pub struct AssignedTo(pub Entity);

/// No pawn could act on this job right now (unreachable target, nowhere to
/// drop the yield); skipped by assignment until the timer expires, then it
/// re-enters the pool for a retry — the world may have changed.
#[derive(Component)]
pub struct Stuck(pub Timer);

impl Stuck {
    fn new() -> Self {
        Stuck(Timer::from_seconds(STUCK_RETRY_SECS, TimerMode::Once))
    }
}

/// Tick stuck jobs back into the pool.
pub fn tick_stuck(mut commands: Commands, time: Res<Time>, mut stuck: Query<(Entity, &mut Stuck)>) {
    for (job, mut timer) in &mut stuck {
        if timer.0.tick(time.delta()).is_finished() {
            commands.entity(job).remove::<Stuck>();
        }
    }
}

/// The job this pawn is working on.
#[derive(Component)]
pub struct CurrentJob(pub Entity);

/// Marks a pawn as actually resting in the `Bed` entity it's pointing at
/// (as opposed to still walking there) — inserted by `execute_jobs`' `Sleep`
/// arm on arrival, removed by `cleanup_jobs`/the release systems alongside
/// `CurrentJob`. `needs::decay_needs` reads this to pick the faster bed
/// recovery rate over the ground fallback (both the ground fallback and bed
/// rest otherwise look identical to `sync_sleeping_pose`, which triggers on
/// simply being stationary while `Objective::Sleep`).
#[derive(Component)]
pub struct InBed(#[allow(dead_code)] pub Entity);

/// Timed work: progress toward completing a job while its pawn stands at
/// the work spot, ticked in `execute_jobs` (so pausing freezes it). Lives
/// on the JOB entity but is removed whenever the job is released back to
/// the pool — a new pawn starts from zero. Cut jobs are the first users;
/// timed harvesting can reuse it later.
#[derive(Component)]
pub struct WorkProgress(pub Timer);

/// Human-readable activity ("idle", "harvesting", ...), derived every frame;
/// the single source for the job panel and the selection info panel.
#[derive(Default, Component)]
pub struct PawnStatus(pub String);

/// What this pawn carries. Delivery to the stockpile is driven entirely by
/// this component (`deliver_carried`), so a haul survives its job entity.
#[derive(Component)]
pub struct Carrying {
    pub kind: ItemKind,
    pub amount: u32,
}

/// The little berry cube riding on a carrying pawn; points back at it.
#[derive(Component)]
pub struct CarryVisual(Entity);

/// Pick each pawn's objective: `Sleep` overrides everything once the pawn is
/// tired enough (hysteresis via `needs::wants_sleep`, so it doesn't flicker
/// right at the threshold); otherwise `Work` while it is working (or work is
/// available), `Recreation` otherwise. Drafted pawns are left alone — the
/// player is controlling them directly.
///
/// Sleep is decided from `Needs::sleep` alone today (a flat day-calibrated
/// decay already lands "tired" around dusk). A night-bias hook — e.g. also
/// requiring `!matches!(clock.phase(), DayPhase::Day)` — would slot in here
/// if pawns ever nap during the day at low-but-not-critical sleep.
#[allow(clippy::type_complexity)]
pub fn choose_objectives(
    mut pawns: Query<
        (
            &mut Objective,
            Option<&CurrentJob>,
            Has<Carrying>,
            &Allowance,
            &Needs,
            Has<ManualMode>,
        ),
        With<Pawn>,
    >,
    jobs: Query<&Job>,
    free_jobs: Query<&Job, (Without<AssignedTo>, Without<Stuck>)>,
) {
    let free_kinds: Vec<JobKind> = free_jobs
        .iter()
        .map(|job| job.kind)
        .filter(JobKind::is_work)
        .collect();
    for (mut objective, current, carrying, allowance, needs, manual) in &mut pawns {
        if manual {
            continue;
        }
        let wanted = if wants_sleep(needs.sleep, *objective == Objective::Sleep) {
            Objective::Sleep
        } else {
            let working = carrying
                || current
                    .and_then(|job| jobs.get(job.0).ok())
                    .is_some_and(|job| job.kind.is_work());
            // Only work this pawn is allowed to do counts as available.
            let work_available = free_kinds.iter().any(|kind| allowance.allows(kind));
            if working || work_available {
                Objective::Work
            } else {
                Objective::Recreation
            }
        };
        if *objective != wanted {
            *objective = wanted;
        }
    }
}

/// How many more items of `kind` the stockpile can absorb: free cells take a
/// whole fresh stack, started same-kind stacks their remaining space; cells
/// holding another kind take nothing. `stacks` is every stack on the map as
/// `(kind, cell, amount)` — non-stockpiled ones are simply never matched.
///
/// This gates haul-job generation: hauling toward a full stockpile is what
/// caused the pickup/drop-at-feet livelock (a pawn forever re-hauling the
/// stack it just dropped).
fn stockpile_capacity(
    kind: ItemKind,
    cells: &HashSet<GridCoords>,
    stacks: &[(ItemKind, GridCoords, u32)],
) -> u32 {
    cells
        .iter()
        .map(
            |cell| match stacks.iter().find(|(_, grid, _)| grid == cell) {
                None => STACK_CAP,
                Some((stack_kind, _, amount)) if *stack_kind == kind => {
                    STACK_CAP.saturating_sub(*amount)
                }
                Some(_) => 0,
            },
        )
        .sum()
}

/// One harvest job per plant (bush or crop) queued `ToHarvest` (full-grown
/// or player-ordered), one cut job per tree marked `ToCut`, one haul job
/// per item stack lying outside the stockpile (only while the stockpile can
/// still absorb that kind), one farming job per empty plantable cell of a
/// Grow-enabled growing zone; none for targets that already have a job.
#[allow(clippy::too_many_arguments)]
pub fn generate_jobs(
    mut commands: Commands,
    to_harvest: Query<Entity, With<ToHarvest>>,
    to_cut: Query<Entity, With<ToCut>>,
    to_demolish: Query<Entity, With<ToDemolish>>,
    all_stacks: Query<(Entity, &ItemStack, &GridCoords)>,
    stockpile: Res<Stockpile>,
    jobs: Query<&Job>,
    growing_zones: Query<(&ZoneRegion, &GrowOrder, &ZonePlant)>,
    crop_cells: Query<&GridCoords, With<Crop>>,
    tree_cells: Query<&GridCoords, With<Tree>>,
    shrub_cells: Query<&GridCoords, With<Shrub>>,
    blueprints: Query<(Entity, &Blueprint)>,
    roofs: Res<RoofMap>,
    terrain: Res<TerrainMap>,
    water: Res<map::WaterMap>,
) {
    let targeted: HashSet<Entity> = jobs
        .iter()
        .filter_map(|job| match job.kind {
            JobKind::Harvest { target } | JobKind::Cut { target } => Some(target),
            JobKind::Haul { stack } | JobKind::Merge { stack } => Some(stack),
            JobKind::Build { site } | JobKind::Demolish { site } => Some(site),
            JobKind::Walk { .. }
            | JobKind::Farming { .. }
            | JobKind::Supply
            | JobKind::BuildRoof { .. }
            | JobKind::RemoveRoof { .. }
            | JobKind::Sleep { .. } => None,
        })
        .collect();
    for target in &to_harvest {
        if !targeted.contains(&target) {
            info!("director: new harvest job for {target:?}");
            commands.spawn((
                Job {
                    kind: JobKind::Harvest { target },
                },
                JobPriority(HARVEST_JOB_PRIORITY),
                Name::new("Harvest job"),
            ));
        }
    }
    for target in &to_cut {
        if !targeted.contains(&target) {
            info!("director: new cut job for {target:?}");
            commands.spawn((
                Job {
                    kind: JobKind::Cut { target },
                },
                JobPriority(CUT_JOB_PRIORITY),
                Name::new("Cut job"),
            ));
        }
    }
    for site in &to_demolish {
        if !targeted.contains(&site) {
            info!("director: new demolish job for {site:?}");
            commands.spawn((
                Job {
                    kind: JobKind::Demolish { site },
                },
                JobPriority(DEMOLISH_JOB_PRIORITY),
                Name::new("Demolish job"),
            ));
        }
    }
    let stack_list: Vec<(ItemKind, GridCoords, u32)> = all_stacks
        .iter()
        .map(|(_, stack, grid)| (stack.kind, *grid, stack.amount))
        .collect();
    for (stack, item_stack, grid) in &all_stacks {
        if !targeted.contains(&stack)
            && !stockpile.cells.contains(grid)
            && stockpile_capacity(item_stack.kind, &stockpile.cells, &stack_list) > 0
        {
            info!("director: new haul job for stack {stack:?}");
            commands.spawn((
                Job {
                    kind: JobKind::Haul { stack },
                },
                JobPriority(HAUL_JOB_PRIORITY),
                Name::new("Haul job"),
            ));
        }
    }

    // Consolidation: at most one merge job at a time (avoids two pawns
    // playing ping-pong with the same pair of stacks), for the smallest
    // stockpiled stack the other same-kind stockpiled stacks can fully
    // absorb (different kinds never merge).
    let has_merge_job = jobs
        .iter()
        .any(|job| matches!(job.kind, JobKind::Merge { .. }));
    if !has_merge_job {
        let inside: Vec<(Entity, &ItemStack)> = all_stacks
            .iter()
            .filter(|(_, _, grid)| stockpile.cells.contains(grid))
            .map(|(entity, stack, _)| (entity, stack))
            .collect();
        let candidate = inside
            .iter()
            .filter(|(entity, _)| !targeted.contains(entity))
            .min_by_key(|(_, stack)| stack.amount)
            .filter(|(entity, stack)| {
                let others_space: u32 = inside
                    .iter()
                    .filter(|(other, other_stack)| {
                        other != entity && other_stack.kind == stack.kind
                    })
                    .map(|(_, other_stack)| other_stack.space())
                    .sum();
                others_space >= stack.amount
            });
        if let Some((stack, _)) = candidate {
            info!("director: new merge job for stack {stack:?}");
            commands.spawn((
                Job {
                    kind: JobKind::Merge { stack: *stack },
                },
                JobPriority(MERGE_JOB_PRIORITY),
                Name::new("Merge job"),
            ));
        }
    }

    // Construction supply: at most one batched wood run at a time (the
    // Merge model — the singleton is the reservation; `delivered` only
    // moves on physical drop-off, so an aborted trip self-heals). Only
    // while some blueprint still needs wood and wood exists to fetch.
    let has_supply_job = jobs.iter().any(|job| matches!(job.kind, JobKind::Supply));
    if !has_supply_job {
        let total_need: u32 = blueprints
            .iter()
            .map(|(_, blueprint)| blueprint.needed())
            .sum();
        let wood_exists = all_stacks
            .iter()
            .any(|(_, stack, _)| stack.kind == ItemKind::Wood);
        if total_need > 0 && wood_exists {
            info!("director: new supply job ({total_need} wood needed)");
            commands.spawn((
                Job {
                    kind: JobKind::Supply,
                },
                JobPriority(SUPPLY_JOB_PRIORITY),
                Name::new("Supply job"),
            ));
        }
    }

    // Building: one job per fully supplied blueprint.
    for (site, blueprint) in &blueprints {
        if blueprint.is_supplied() && !targeted.contains(&site) {
            info!("director: new build job for {site:?}");
            commands.spawn((
                Job {
                    kind: JobKind::Build { site },
                },
                JobPriority(BUILD_JOB_PRIORITY),
                Name::new("Build job"),
            ));
        }
    }

    // Roofing: derived straight from the RoofMap — one build job per cell
    // whose Roof stamp isn't satisfied yet, one removal job per roofed
    // NoRoof cell; no stored designations to keep in sync.
    let pending_roof: HashSet<GridCoords> = jobs
        .iter()
        .filter_map(|job| match job.kind {
            JobKind::BuildRoof { cell } | JobKind::RemoveRoof { cell } => Some(cell),
            _ => None,
        })
        .collect();
    for y in 0..MAP_HEIGHT {
        for x in 0..MAP_WIDTH {
            let cell = GridCoords::new(x, y);
            if pending_roof.contains(&cell) {
                continue;
            }
            let policy = roofs.policy(cell);
            let roofed = roofs.is_roofed(cell);
            if construction::wants_roof(
                policy,
                roofed,
                terrain.get(cell).is_some(),
                water.has_bed(cell),
            ) {
                commands.spawn((
                    Job {
                        kind: JobKind::BuildRoof { cell },
                    },
                    JobPriority(ROOF_JOB_PRIORITY),
                    Name::new("Roof job"),
                ));
            } else if construction::wants_roof_removed(policy, roofed) {
                commands.spawn((
                    Job {
                        kind: JobKind::RemoveRoof { cell },
                    },
                    JobPriority(ROOF_JOB_PRIORITY),
                    Name::new("Unroof job"),
                ));
            }
        }
    }

    // Farming: one job per empty plantable cell of every Grow-enabled
    // growing zone — cells the zone's plant kind accepts (trees keep their
    // spacing grid), skipping cells that already hold a crop or a tree or
    // already have a pending job.
    let planted: HashSet<GridCoords> = crop_cells
        .iter()
        .chain(&tree_cells)
        .chain(&shrub_cells)
        .copied()
        .collect();
    let pending_farm: HashSet<GridCoords> = jobs
        .iter()
        .filter_map(|job| match job.kind {
            JobKind::Farming { cell, .. } => Some(cell),
            _ => None,
        })
        .collect();
    for (region, grow, plant) in &growing_zones {
        if !grow.enabled {
            continue;
        }
        for cell in region.cells() {
            if plant.accepts_cell(cell) && !planted.contains(&cell) && !pending_farm.contains(&cell)
            {
                commands.spawn((
                    Job {
                        kind: JobKind::Farming {
                            cell,
                            plant: *plant,
                        },
                    },
                    JobPriority(FARM_JOB_PRIORITY),
                    Name::new("Farming job"),
                ));
            }
        }
    }
}

/// Pawns off work go for a walk: a personal Walk job to a random reachable
/// cell nearby, spawned pre-assigned (it never enters the open job pool).
#[allow(clippy::type_complexity)]
pub fn generate_walk_jobs(
    mut commands: Commands,
    nav: Res<NavGrid>,
    mut rng: ResMut<WanderRng>,
    strollers: Query<
        (Entity, &GridCoords, &Objective),
        (
            With<Pawn>,
            Without<CurrentJob>,
            Without<Path>,
            Without<MoveOrder>,
            Without<Carrying>,
            Without<WanderCooldown>,
            Without<ManualMode>,
        ),
    >,
) {
    for (pawn, pawn_grid, objective) in &strollers {
        if *objective != Objective::Recreation {
            continue;
        }
        let goal = (0..10)
            .map(|_| {
                GridCoords::new(
                    pawn_grid.x + rng.range(-WANDER_RADIUS, WANDER_RADIUS),
                    pawn_grid.y + rng.range(-WANDER_RADIUS, WANDER_RADIUS),
                )
            })
            .find(|cell| {
                cell != pawn_grid
                    && nav.is_walkable(*cell)
                    && nav.find_path(*pawn_grid, *cell).is_some()
            });
        // No luck this frame: re-roll next frame.
        if let Some(goal) = goal {
            let job = commands
                .spawn((
                    Job {
                        kind: JobKind::Walk { goal },
                    },
                    JobPriority(WALK_JOB_PRIORITY),
                    AssignedTo(pawn),
                    Name::new("Walk job"),
                ))
                .id();
            commands.entity(pawn).insert(CurrentJob(job));
        }
    }
}

/// Tired pawns claim a bed: a personal Sleep job to the nearest free,
/// reachable `Bed`, spawned pre-assigned like `Walk` (never enters the open
/// pool). The reservation is the live job entity itself — a bed already
/// targeted by another live `Sleep` job is skipped, mirroring `generate_jobs`'
/// `targeted`-set idiom. No reachable free bed this frame → nothing spawned;
/// the pawn just stays put and recovers on the ground
/// (`release_sleeping_pawns` already stopped it, `needs::decay_needs`
/// recovers it there at the slower rate).
#[allow(clippy::type_complexity)]
pub fn generate_sleep_jobs(
    mut commands: Commands,
    nav: Res<NavGrid>,
    sleepers: Query<
        (Entity, &GridCoords, &Objective),
        (
            With<Pawn>,
            Without<CurrentJob>,
            Without<Path>,
            Without<MoveOrder>,
            Without<ManualMode>,
        ),
    >,
    jobs: Query<&Job>,
    beds: Query<(Entity, &Footprint), With<Bed>>,
) {
    let reserved: HashSet<Entity> = jobs
        .iter()
        .filter_map(|job| match job.kind {
            JobKind::Sleep { bed } => Some(bed),
            _ => None,
        })
        .collect();
    for (pawn, pawn_grid, objective) in &sleepers {
        if *objective != Objective::Sleep {
            continue;
        }
        let best = beds
            .iter()
            .filter(|(bed, _)| !reserved.contains(bed))
            .filter_map(|(bed, footprint)| {
                footprint_rest_spot(&nav, *pawn_grid, &footprint.cells).map(|cell| (bed, cell))
            })
            .min_by_key(|(_, cell)| {
                let (dx, dy) = (cell.x - pawn_grid.x, cell.y - pawn_grid.y);
                dx * dx + dy * dy
            });
        if let Some((bed, _)) = best {
            info!("director: new sleep job for pawn {pawn:?} at bed {bed:?}");
            let job = commands
                .spawn((
                    Job {
                        kind: JobKind::Sleep { bed },
                    },
                    JobPriority(SLEEP_JOB_PRIORITY),
                    AssignedTo(pawn),
                    Name::new("Sleep job"),
                ))
                .id();
            commands.entity(pawn).insert(CurrentJob(job));
        }
    }
}

/// Tick the between-strolls breather and clear it when done.
pub fn tick_wander_cooldowns(
    mut commands: Commands,
    time: Res<Time>,
    mut cooldowns: Query<(Entity, &mut WanderCooldown)>,
) {
    for (pawn, mut cooldown) in &mut cooldowns {
        if cooldown.0.tick(time.delta()).is_finished() {
            commands.entity(pawn).remove::<WanderCooldown>();
        }
    }
}

/// Drop jobs whose target is gone or no longer harvestable, releasing the
/// assigned pawn; also clear pawns whose job entity has disappeared.
#[allow(clippy::too_many_arguments)]
pub fn cleanup_jobs(
    mut commands: Commands,
    jobs: Query<(Entity, &Job, Option<&AssignedTo>)>,
    harvestables: Query<(), With<ToHarvest>>,
    cuttables: Query<(), With<ToCut>>,
    stacks: Query<(&ItemStack, &GridCoords)>,
    stockpile: Res<Stockpile>,
    workers: Query<(Entity, &CurrentJob), With<Pawn>>,
    objectives: Query<&Objective, With<Pawn>>,
    growing_zones: Query<(&ZoneRegion, &GrowOrder, &ZonePlant)>,
    crop_cells: Query<&GridCoords, With<Crop>>,
    tree_cells: Query<&GridCoords, With<Tree>>,
    shrub_cells: Query<&GridCoords, With<Shrub>>,
    blueprints: Query<&Blueprint>,
    carriers: Query<&Carrying, With<Pawn>>,
    roofs: Res<RoofMap>,
    // Bundled into one tuple param (system-param arity limit).
    ground: (
        Res<TerrainMap>,
        Res<map::WaterMap>,
        Query<(), With<Bed>>,
        Query<(), With<ToDemolish>>,
    ),
) {
    let (terrain, water, beds, demolishables) = ground;
    for (job_entity, job, assigned) in &jobs {
        let done = match job.kind {
            // A stroll is void as soon as its pawn has better things to do.
            JobKind::Walk { .. } => assigned
                .map(|AssignedTo(pawn)| {
                    objectives
                        .get(*pawn)
                        .map(|objective| *objective != Objective::Recreation)
                        .unwrap_or(true)
                })
                .unwrap_or(true),
            JobKind::Harvest { target } => harvestables.get(target).is_err(),
            // Done once felled (despawned); void if the designation was
            // canceled — this is how the Cancel button aborts a cut.
            JobKind::Cut { target } => cuttables.get(target).is_err(),
            // Done once something stands there; void if the cell left a
            // Grow-enabled zone (deleted, shrunk, or Grow toggled off) or
            // the zone switched to another plant kind meanwhile.
            JobKind::Farming { cell, plant } => {
                crop_cells
                    .iter()
                    .chain(&tree_cells)
                    .chain(&shrub_cells)
                    .any(|grid| *grid == cell)
                    || !growing_zones.iter().any(|(region, grow, zone_plant)| {
                        grow.enabled && *zone_plant == plant && region.contains(cell)
                    })
            }
            // Gone = picked up; in the stockpile = nothing left to do.
            JobKind::Haul { stack } => stacks
                .get(stack)
                .map(|(_, grid)| stockpile.cells.contains(grid))
                .unwrap_or(true),
            // Done or void once every blueprint is supplied — or there is
            // simply no wood left anywhere to fetch. Wood in the assigned
            // pawn's hands counts: the moment the last ground stack is
            // picked up (by the supplier itself, or a concurrent haul), the
            // run is still very much alive — voiding it here would strand
            // the load and respawn the job every frame.
            JobKind::Supply => {
                let carrier_has_wood = assigned.is_some_and(|AssignedTo(pawn)| {
                    carriers
                        .get(*pawn)
                        .is_ok_and(|carrying| carrying.kind == ItemKind::Wood)
                });
                blueprints.iter().all(|blueprint| blueprint.is_supplied())
                    || (!carrier_has_wood
                        && !stacks.iter().any(|(stack, _)| stack.kind == ItemKind::Wood))
            }
            // Gone = built or canceled.
            JobKind::Build { site } => blueprints.get(site).is_err(),
            // Done once demolished (despawned); void if the designation was
            // canceled — the Demolish mirror of the Cut arm above.
            JobKind::Demolish { site } => demolishables.get(site).is_err(),
            // The derived designations void themselves: once the cell is
            // roofed (done) or its stamp/terrain changed (void), the want
            // is gone either way.
            JobKind::BuildRoof { cell } => !construction::wants_roof(
                roofs.policy(cell),
                roofs.is_roofed(cell),
                terrain.get(cell).is_some(),
                water.has_bed(cell),
            ),
            JobKind::RemoveRoof { cell } => {
                !construction::wants_roof_removed(roofs.policy(cell), roofs.is_roofed(cell))
            }
            // Gone = picked up; void if the other same-kind stockpiled
            // stacks can no longer fully absorb it (their space shrank
            // meanwhile).
            JobKind::Merge { stack } => match stacks.get(stack) {
                Err(_) => true,
                Ok((merge_stack, merge_grid)) => {
                    !stockpile.cells.contains(merge_grid)
                        || stacks
                            .iter()
                            .filter(|(other, grid)| {
                                stockpile.cells.contains(grid)
                                    && *grid != merge_grid
                                    && other.kind == merge_stack.kind
                            })
                            .map(|(other, _)| other.space())
                            .sum::<u32>()
                            < merge_stack.amount
                }
            },
            // Void once the pawn wakes (rested past the threshold) or the
            // bed it was walking to/resting in is gone (demolished mid-nap).
            JobKind::Sleep { bed } => {
                beds.get(bed).is_err()
                    || assigned
                        .map(|AssignedTo(pawn)| {
                            objectives
                                .get(*pawn)
                                .map(|objective| *objective != Objective::Sleep)
                                .unwrap_or(true)
                        })
                        .unwrap_or(true)
            }
        };
        if done {
            info!("director: job {job_entity:?} finished or void");
            commands.entity(job_entity).despawn();
            if let Some(AssignedTo(pawn)) = assigned {
                // Releasing also cancels the walking: a normally completed
                // job leaves no Path anyway, and a canceled/voided one must
                // not have the pawn finish a pointless trip. `InBed` is a
                // harmless no-op remove for every job kind but Sleep.
                commands
                    .entity(*pawn)
                    .remove::<(CurrentJob, Path, MoveOrder, InBed)>();
            }
        }
    }
    for (pawn, current) in &workers {
        if jobs.get(current.0).is_err() {
            commands.entity(pawn).remove::<CurrentJob>();
        }
    }
}

/// Release assigned jobs whose pawn no longer allows that work type: the
/// job returns to the pool (its target is still valid), the pawn stops.
pub fn enforce_allowances(
    mut commands: Commands,
    jobs: Query<(Entity, &Job, &AssignedTo)>,
    allowances: Query<&Allowance, With<Pawn>>,
) {
    for (job_entity, job, AssignedTo(pawn)) in &jobs {
        let allowed = allowances
            .get(*pawn)
            .map(|allowance| allowance.allows(&job.kind))
            .unwrap_or(false);
        if !allowed {
            info!("director: pawn {pawn:?} no longer allowed job {job_entity:?}, releasing");
            // Timed work starts over for whoever picks the job up next.
            commands
                .entity(job_entity)
                .remove::<(AssignedTo, WorkProgress)>();
            commands
                .entity(*pawn)
                .remove::<(CurrentJob, Path, MoveOrder)>();
        }
    }
}

/// Drafting a pawn drops whatever it was doing at once: the job returns to
/// the pool (a Walk job, once unassigned, is voided by `cleanup_jobs` next
/// frame) and any in-progress move is cancelled. Mirrors `enforce_allowances`.
pub fn release_manual_pawns(
    mut commands: Commands,
    jobs: Query<(Entity, &AssignedTo)>,
    manual: Query<(), (With<Pawn>, With<ManualMode>)>,
) {
    for (job_entity, AssignedTo(pawn)) in &jobs {
        if manual.get(*pawn).is_ok() {
            info!("director: pawn {pawn:?} drafted, releasing job {job_entity:?}");
            commands
                .entity(job_entity)
                .remove::<(AssignedTo, WorkProgress)>();
            // `InBed` alongside the rest: without it, a pawn drafted mid-nap
            // would keep the marker forever — `AssignedTo` is gone from the
            // job the moment this runs, so `cleanup_jobs` treats it as
            // unassigned and despawns it next frame, but that path has no
            // pawn reference left to clean `InBed` up with; strip it here
            // instead, before that reference is gone.
            commands
                .entity(*pawn)
                .remove::<(CurrentJob, Path, MoveOrder, InBed)>();
        }
    }
}

/// A pawn that has fallen asleep drops whatever *other* work it was doing at
/// once, same as drafting: the job returns to the pool (a Walk job is voided
/// by `cleanup_jobs` next frame since it's no longer on `Recreation`) and any
/// in-progress move is cancelled, so the pawn stops right where it fell
/// tired. Mirrors `release_manual_pawns`.
///
/// Exempts `JobKind::Sleep` itself: `generate_sleep_jobs` spawns that job
/// pre-assigned to a newly-tired pawn in this same frame's earlier systems,
/// and this system running unconditionally would immediately undo that,
/// stripping the very job meant to walk the pawn to a bed. A pawn's *old*
/// work job still drops the frame it goes `Sleep` either way — `generate_
/// sleep_jobs`'s idle-only query simply skips it that frame (it still holds
/// the old `CurrentJob`), picking it up one frame later once this system has
/// cleared that job — the same one-frame tolerance the rest of the pipeline
/// already has.
pub fn release_sleeping_pawns(
    mut commands: Commands,
    jobs: Query<(Entity, &Job, &AssignedTo)>,
    sleeping: Query<&Objective, With<Pawn>>,
) {
    for (job_entity, job, AssignedTo(pawn)) in &jobs {
        if matches!(job.kind, JobKind::Sleep { .. }) {
            continue;
        }
        if sleeping
            .get(*pawn)
            .is_ok_and(|objective| *objective == Objective::Sleep)
        {
            info!("director: pawn {pawn:?} fell asleep, releasing job {job_entity:?}");
            commands
                .entity(job_entity)
                .remove::<(AssignedTo, WorkProgress)>();
            commands
                .entity(*pawn)
                .remove::<(CurrentJob, Path, MoveOrder)>();
        }
    }
}

/// Idle pawns claim the best free job: lowest priority number first, then
/// the closest target.
#[allow(clippy::type_complexity)]
pub fn assign_jobs(
    mut commands: Commands,
    idle_pawns: Query<
        (Entity, &GridCoords, &Skills, &Objective, &Allowance),
        (
            With<Pawn>,
            Without<CurrentJob>,
            Without<Path>,
            Without<MoveOrder>,
            Without<Carrying>,
            Without<ManualMode>,
        ),
    >,
    free_jobs: Query<(Entity, &Job, &JobPriority), (Without<AssignedTo>, Without<Stuck>)>,
    plants: Query<&GridCoords, Or<(With<BerryBush>, With<Crop>)>>,
    tree_targets: Query<&GridCoords, With<Tree>>,
    stacks: Query<(&ItemStack, &GridCoords)>,
    sites: Query<&GridCoords, With<Blueprint>>,
    demolish_sites: Query<&GridCoords, With<ToDemolish>>,
) {
    let mut claimed = HashSet::new();
    for (pawn, pawn_grid, skills, objective, allowance) in &idle_pawns {
        // The pool only holds work jobs (Walk jobs spawn pre-assigned), so
        // only pawns on the Work objective shop in it.
        if *objective != Objective::Work {
            continue;
        }
        let best = free_jobs
            .iter()
            .filter(|(job_entity, ..)| !claimed.contains(job_entity))
            .filter_map(|(job_entity, job, priority)| {
                if !allowance.allows(&job.kind) {
                    return None;
                }
                let (target, skill) = match job.kind {
                    JobKind::Harvest { target } => (*plants.get(target).ok()?, skills.harvest),
                    JobKind::Cut { target } => (*tree_targets.get(target).ok()?, skills.forestry),
                    JobKind::Haul { stack } | JobKind::Merge { stack } => {
                        (*stacks.get(stack).ok()?.1, skills.haul)
                    }
                    JobKind::Farming { cell, .. } => (cell, skills.farm),
                    // A supply run starts at the nearest wood stack.
                    JobKind::Supply => {
                        let nearest = stacks
                            .iter()
                            .filter(|(stack, _)| stack.kind == ItemKind::Wood)
                            .map(|(_, grid)| *grid)
                            .min_by_key(|grid| {
                                let (dx, dy) = (grid.x - pawn_grid.x, grid.y - pawn_grid.y);
                                dx * dx + dy * dy
                            })?;
                        (nearest, skills.haul)
                    }
                    JobKind::Build { site } => (*sites.get(site).ok()?, skills.build),
                    JobKind::Demolish { site } => (*demolish_sites.get(site).ok()?, skills.build),
                    JobKind::BuildRoof { cell } | JobKind::RemoveRoof { cell } => {
                        (cell, skills.build)
                    }
                    JobKind::Walk { .. } | JobKind::Sleep { .. } => return None,
                };
                let (dx, dy) = (target.x - pawn_grid.x, target.y - pawn_grid.y);
                Some((priority.0, skill.rank(), dx * dx + dy * dy, job_entity))
            })
            // Lowest priority number, then this pawn's best skill tier,
            // then the closest target.
            .min_by_key(|(priority, rank, distance_sq, _)| {
                (*priority, std::cmp::Reverse(*rank), *distance_sq)
            });
        if let Some((.., job_entity)) = best {
            info!("director: job {job_entity:?} assigned to pawn {pawn:?}");
            claimed.insert(job_entity);
            commands.entity(pawn).insert(CurrentJob(job_entity));
            commands.entity(job_entity).insert(AssignedTo(pawn));
        }
    }
}

/// Derive each pawn's status line from its job/movement components.
#[allow(clippy::type_complexity)]
pub fn update_pawn_status(
    mut pawns: Query<
        (
            &mut PawnStatus,
            Option<&CurrentJob>,
            Option<&Carrying>,
            Has<Path>,
            Has<MoveOrder>,
            Has<ManualMode>,
            &Objective,
        ),
        With<Pawn>,
    >,
    jobs: Query<&Job>,
) {
    for (mut status, current, carrying, has_path, has_order, manual, objective) in &mut pawns {
        let moving = has_path || has_order;
        if manual {
            let text = if moving { "manual (moving)" } else { "manual" }.to_string();
            if status.0 != text {
                status.0 = text;
            }
            continue;
        }
        let on_supply_run = current
            .and_then(|job| jobs.get(job.0).ok())
            .is_some_and(|job| matches!(job.kind, JobKind::Supply));
        let text = match (carrying, current) {
            (Some(carried), _) if on_supply_run => {
                format!("delivering materials ({})", carried.amount)
            }
            (Some(carried), _) => {
                format!("hauling {} ({})", carried.kind.label(), carried.amount)
            }
            (None, Some(job)) => match jobs.get(job.0).map(|job| job.kind) {
                Ok(JobKind::Harvest { .. }) if moving => "walking to harvest".to_string(),
                Ok(JobKind::Harvest { .. }) => "harvesting".to_string(),
                Ok(JobKind::Cut { .. }) if moving => "walking to cut".to_string(),
                Ok(JobKind::Cut { .. }) => "cutting a tree".to_string(),
                Ok(JobKind::Haul { .. }) => "fetching stack".to_string(),
                Ok(JobKind::Merge { .. }) => "merging stacks".to_string(),
                Ok(JobKind::Walk { .. }) => "going for a walk".to_string(),
                Ok(JobKind::Farming { .. }) if moving => "walking to plant".to_string(),
                Ok(JobKind::Farming { .. }) => "planting".to_string(),
                Ok(JobKind::Supply) => "fetching materials".to_string(),
                Ok(JobKind::Build { .. }) if moving => "walking to build".to_string(),
                Ok(JobKind::Build { .. }) => "building".to_string(),
                Ok(JobKind::Demolish { .. }) if moving => "walking to demolish".to_string(),
                Ok(JobKind::Demolish { .. }) => "demolishing".to_string(),
                Ok(JobKind::BuildRoof { .. }) if moving => "walking to roof".to_string(),
                Ok(JobKind::BuildRoof { .. }) => "roofing".to_string(),
                Ok(JobKind::RemoveRoof { .. }) if moving => "walking to unroof".to_string(),
                Ok(JobKind::RemoveRoof { .. }) => "removing a roof".to_string(),
                Ok(JobKind::Sleep { .. }) if moving => "walking to bed".to_string(),
                Ok(JobKind::Sleep { .. }) => "sleeping in bed".to_string(),
                Err(_) => "idle".to_string(),
            },
            (None, None) if moving => "moving".to_string(),
            // Ground-sleep fallback: tired, no bed job (none reachable), just
            // resting in place where the pawn stopped.
            _ if *objective == Objective::Sleep => "sleeping".to_string(),
            _ => "idle".to_string(),
        };
        if status.0 != text {
            status.0 = text;
        }
    }
}

/// Best cell for a pawn at `from` to stand while working on `target`, chosen
/// as the reachable walkable candidate with the shortest path. `include_target`
/// allows standing directly on the target (roofs: a walkable target, e.g. the
/// lone interior of a 1x1 room whose 4 neighbors are all walls, can only be
/// worked by standing on it); Harvest/Cut/Build pass `false` because they
/// must stand *beside* the target, never on it.
fn nearest_stand_spot(
    nav: &NavGrid,
    from: GridCoords,
    target: GridCoords,
    include_target: bool,
) -> Option<GridCoords> {
    let offsets: &[(i32, i32)] = if include_target {
        &[(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)]
    } else {
        &[(1, 0), (-1, 0), (0, 1), (0, -1)]
    };
    offsets
        .iter()
        .map(|(dx, dy)| GridCoords::new(target.x + dx, target.y + dy))
        .filter(|cell| nav.is_walkable(*cell))
        .filter_map(|cell| nav.find_path(from, cell).map(|path| (path.len(), cell)))
        .min_by_key(|(length, _)| *length)
        .map(|(_, cell)| cell)
}

/// Like `nearest_stand_spot`, generalized to a multi-cell footprint (a 2x2
/// solar panel, or any single-cell building's one-cell footprint): the
/// nearest walkable cell orthogonally adjacent to any footprint cell but
/// not itself part of it — the builder always works from outside, same as
/// a single-cell site (`include_target = false` there).
fn footprint_stand_spot(
    nav: &NavGrid,
    from: GridCoords,
    footprint: &[GridCoords],
) -> Option<GridCoords> {
    let footprint_set: HashSet<GridCoords> = footprint.iter().copied().collect();
    // A `Vec` with a linear dedup, not a `HashSet`: footprints are tiny (1-4
    // cells), and keeping insertion order makes tie-breaking deterministic
    // (matches `nearest_stand_spot`'s fixed-offset order for a one-cell
    // footprint) instead of depending on hasher-randomized iteration order.
    let mut candidates: Vec<GridCoords> = Vec::new();
    for cell in footprint {
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let neighbor = GridCoords::new(cell.x + dx, cell.y + dy);
            if !footprint_set.contains(&neighbor) && !candidates.contains(&neighbor) {
                candidates.push(neighbor);
            }
        }
    }
    candidates
        .into_iter()
        .filter(|cell| nav.is_walkable(*cell))
        .filter_map(|cell| nav.find_path(from, cell).map(|path| (path.len(), cell)))
        .min_by_key(|(length, _)| *length)
        .map(|(_, cell)| cell)
}

/// Unlike `footprint_stand_spot` (which always works from *outside* a
/// footprint — a builder must never stand on the floor it's raising), a
/// sleeper rests *inside* the bed's footprint: the nearest walkable cell of
/// `footprint` itself, reachable by path. Nothing turns a bed's cells solid
/// the way `sync_construction_collision` does to an in-progress blueprint, so
/// standing on one is always safe.
fn footprint_rest_spot(
    nav: &NavGrid,
    from: GridCoords,
    footprint: &[GridCoords],
) -> Option<GridCoords> {
    footprint
        .iter()
        .copied()
        .filter(|cell| nav.is_walkable(*cell))
        .filter_map(|cell| nav.find_path(from, cell).map(|path| (path.len(), cell)))
        .min_by_key(|(length, _)| *length)
        .map(|(_, cell)| cell)
}

/// May a pawn standing at `pawn` build on `footprint`? Only from a cell
/// orthogonally adjacent to it, never from inside — a pawn "adjacent" only
/// because it stands on one footprint cell while another cell of the same
/// footprint happens to be its neighbor (the 2x2 solar panel's case) must not
/// count, or the pawn would be building the very floor it's standing on,
/// which turns solid under it once construction starts
/// (`construction::sync_construction_collision`) and traps it.
fn adjacent_to_footprint(pawn: GridCoords, footprint: &[GridCoords]) -> bool {
    !footprint.contains(&pawn)
        && footprint
            .iter()
            .any(|cell| (pawn.x - cell.x).abs() + (pawn.y - cell.y).abs() == 1)
}

/// Working pawns walk to their job's target and act on arrival.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn execute_jobs(
    mut commands: Commands,
    assets: Res<GameAssets>,
    nav: Res<NavGrid>,
    time: Res<Time>,
    mut harvests: MessageWriter<HarvestCommand>,
    mut cuts: MessageWriter<CutCommand>,
    workers: Query<
        (Entity, &GridCoords, &CurrentJob, Has<Path>, Has<MoveOrder>),
        (With<Pawn>, Without<ManualMode>),
    >,
    jobs: Query<&Job>,
    plants: Query<
        (&GridCoords, Has<ToHarvest>, Has<BerryBush>),
        Or<(With<BerryBush>, With<Crop>, With<Shrub>)>,
    >,
    cut_targets: Query<(&GridCoords, Has<ToCut>), With<Tree>>,
    mut work_progress: Query<&mut WorkProgress>,
    mut stacks: Query<(Entity, &mut ItemStack, &GridCoords)>,
    crop_cells: Query<&GridCoords, With<Crop>>,
    occupied: Query<&GridCoords, With<Selectable>>,
    // Construction state plus the history writer, bundled into one tuple
    // param to stay inside the system-param arity limit.
    construction_state: (
        MessageWriter<BuildCommand>,
        MessageWriter<RoofCommand>,
        Query<(&mut Blueprint, &GridCoords, &Footprint)>,
        Query<&mut Carrying>,
        Query<&GridCoords, With<Pawn>>,
        MessageWriter<JobCompleted>,
        Query<&Footprint, With<Bed>>,
        MessageWriter<DemolishCommand>,
        Query<(&GridCoords, &Footprint), With<ToDemolish>>,
        Res<construction::ConstructionMap>,
    ),
) {
    let (
        mut builds,
        mut roof_cmds,
        mut blueprints,
        mut carriers,
        pawn_cells,
        mut completed,
        beds,
        mut demolitions,
        demolish_sites,
        construction,
    ) = construction_state;
    for (pawn, pawn_grid, current, has_path, has_order) in &workers {
        let Ok(job) = jobs.get(current.0) else {
            commands.entity(pawn).remove::<CurrentJob>();
            continue;
        };
        if has_path || has_order {
            continue; // still on the way
        }

        match job.kind {
            JobKind::Harvest { target } => {
                let Ok((target_grid, true, is_bush)) = plants.get(target) else {
                    commands.entity(pawn).remove::<CurrentJob>();
                    continue;
                };
                let kind = if is_bush {
                    ItemKind::Berries
                } else {
                    ItemKind::Wheat
                };
                let stack_list: Vec<(ItemKind, GridCoords, u32)> = stacks
                    .iter()
                    .map(|(_, stack, grid)| (stack.kind, *grid, stack.amount))
                    .collect();
                let occupied_cells: HashSet<GridCoords> = occupied.iter().copied().collect();
                // The yield spills up to YIELD_DROP_RADIUS tiles out; only a
                // truly walled-in plant blocks (retried via `Stuck`).
                if items::find_drop_cell(
                    kind,
                    *target_grid,
                    YIELD_DROP_RADIUS,
                    &stack_list,
                    &occupied_cells,
                    |cell| nav.is_walkable(cell),
                )
                .is_none()
                {
                    warn!("director: nowhere to drop the yield of {target:?}, marking job stuck");
                    commands
                        .entity(current.0)
                        .insert(Stuck::new())
                        .remove::<AssignedTo>();
                    commands.entity(pawn).remove::<CurrentJob>();
                    continue;
                }
                let adjacent =
                    (pawn_grid.x - target_grid.x).abs() + (pawn_grid.y - target_grid.y).abs() == 1;
                if adjacent {
                    info!("director: pawn {pawn:?} harvests {target:?} from {pawn_grid:?}");
                    harvests.write(HarvestCommand {
                        target,
                        drop_at: Some(*pawn_grid),
                    });
                    completed.write(JobCompleted {
                        pawn,
                        label: format!("Harvested {}", kind.label()),
                    });
                    commands.entity(pawn).remove::<CurrentJob>();
                    continue;
                }
                // Walk to the reachable 4-neighbor of the plant with the
                // shortest path.
                match nearest_stand_spot(&nav, *pawn_grid, *target_grid, false) {
                    Some(goal) => {
                        commands.entity(pawn).insert(MoveOrder { goal });
                    }
                    None => {
                        warn!("director: job {:?} unreachable, marking stuck", current.0);
                        commands
                            .entity(current.0)
                            .insert(Stuck::new())
                            .remove::<AssignedTo>();
                        commands.entity(pawn).remove::<CurrentJob>();
                    }
                }
            }
            JobKind::Cut { target } => {
                let Ok((target_grid, true)) = cut_targets.get(target) else {
                    // Canceled or already felled; `cleanup_jobs` voids the job.
                    commands.entity(pawn).remove::<CurrentJob>();
                    continue;
                };
                let stack_list: Vec<(ItemKind, GridCoords, u32)> = stacks
                    .iter()
                    .map(|(_, stack, grid)| (stack.kind, *grid, stack.amount))
                    .collect();
                let occupied_cells: HashSet<GridCoords> = occupied.iter().copied().collect();
                if items::find_drop_cell(
                    ItemKind::Wood,
                    *target_grid,
                    YIELD_DROP_RADIUS,
                    &stack_list,
                    &occupied_cells,
                    |cell| nav.is_walkable(cell),
                )
                .is_none()
                {
                    warn!("director: nowhere to drop the wood of {target:?}, marking job stuck");
                    commands
                        .entity(current.0)
                        .insert(Stuck::new())
                        .remove::<(AssignedTo, WorkProgress)>();
                    commands.entity(pawn).remove::<CurrentJob>();
                    continue;
                }
                let adjacent =
                    (pawn_grid.x - target_grid.x).abs() + (pawn_grid.y - target_grid.y).abs() == 1;
                if adjacent {
                    // Timed work: the first frame at the tree arms the timer,
                    // then it ticks here every frame the pawn keeps standing
                    // (pause gates this whole system, freezing the cut).
                    if let Ok(mut progress) = work_progress.get_mut(current.0) {
                        if progress.0.tick(time.delta()).is_finished() {
                            info!("director: pawn {pawn:?} fells {target:?} from {pawn_grid:?}");
                            cuts.write(CutCommand {
                                target,
                                drop_at: *pawn_grid,
                            });
                            completed.write(JobCompleted {
                                pawn,
                                label: "Chopped a tree".to_string(),
                            });
                            commands.entity(pawn).remove::<CurrentJob>();
                        }
                    } else {
                        commands
                            .entity(current.0)
                            .insert(WorkProgress(Timer::from_seconds(
                                TREE_CUT_SECS,
                                TimerMode::Once,
                            )));
                    }
                    continue;
                }
                // Walk to the reachable 4-neighbor of the tree with the
                // shortest path (same approach as Harvest).
                match nearest_stand_spot(&nav, *pawn_grid, *target_grid, false) {
                    Some(goal) => {
                        commands.entity(pawn).insert(MoveOrder { goal });
                    }
                    None => {
                        warn!("director: job {:?} unreachable, marking stuck", current.0);
                        commands
                            .entity(current.0)
                            .insert(Stuck::new())
                            .remove::<AssignedTo>();
                        commands.entity(pawn).remove::<CurrentJob>();
                    }
                }
            }
            JobKind::Haul { stack } | JobKind::Merge { stack } => {
                let Ok((_, item_stack, stack_grid)) = stacks.get(stack) else {
                    commands.entity(pawn).remove::<CurrentJob>();
                    continue;
                };
                if pawn_grid == stack_grid {
                    // Pick the whole stack up; `deliver_carried` takes over.
                    info!(
                        "director: pawn {pawn:?} picks up stack {stack:?} ({})",
                        item_stack.amount
                    );
                    // No JobCompleted here: Haul/Merge fire constantly during
                    // normal play and would drown out the more meaningful
                    // history entries (harvests, builds, ...).
                    commands.entity(stack).despawn();
                    commands
                        .entity(pawn)
                        .insert(Carrying {
                            kind: item_stack.kind,
                            amount: item_stack.amount,
                        })
                        .remove::<CurrentJob>();
                    continue;
                }
                if nav.find_path(*pawn_grid, *stack_grid).is_some() {
                    commands
                        .entity(pawn)
                        .insert(MoveOrder { goal: *stack_grid });
                } else {
                    warn!("director: job {:?} unreachable, marking stuck", current.0);
                    commands
                        .entity(current.0)
                        .insert(Stuck::new())
                        .remove::<AssignedTo>();
                    commands.entity(pawn).remove::<CurrentJob>();
                }
            }
            JobKind::Walk { goal } => {
                if *pawn_grid == goal {
                    // Stroll over: breathe a moment, then roll a new one.
                    commands.entity(current.0).despawn();
                    commands
                        .entity(pawn)
                        .remove::<CurrentJob>()
                        .insert(WanderCooldown(Timer::from_seconds(
                            WANDER_PAUSE_SECS,
                            TimerMode::Once,
                        )));
                } else if nav.find_path(*pawn_grid, goal).is_some() {
                    commands.entity(pawn).insert(MoveOrder { goal });
                } else {
                    // Destination turned invalid: no Stuck spam, the next
                    // generation pass just rolls another one.
                    commands.entity(current.0).despawn();
                    commands.entity(pawn).remove::<CurrentJob>();
                }
            }
            JobKind::Sleep { bed } => {
                let Ok(footprint) = beds.get(bed) else {
                    // Bed demolished mid-walk/mid-nap; `cleanup_jobs` handles
                    // the primary despawn — this is just a same-frame guard.
                    commands.entity(pawn).remove::<(CurrentJob, InBed)>();
                    continue;
                };
                if footprint.cells.contains(pawn_grid) {
                    // Resting: open-ended, no timer. `cleanup_jobs` ends this
                    // job once the pawn wakes or the bed disappears.
                    commands.entity(pawn).insert(InBed(bed));
                } else if let Some(goal) = footprint_rest_spot(&nav, *pawn_grid, &footprint.cells) {
                    commands.entity(pawn).insert(MoveOrder { goal });
                } else {
                    // No reachable cell right now: drop and rest on the
                    // ground where the pawn stands — no Stuck spam, same
                    // tolerance as Walk's unreachable-goal case.
                    commands.entity(current.0).despawn();
                    commands.entity(pawn).remove::<CurrentJob>();
                }
            }
            JobKind::Farming { cell, plant } => {
                if *pawn_grid == cell {
                    // Guard against something already standing here (e.g. the
                    // zone flickered Grow off/on and generate_jobs raced a
                    // stale job) — `cleanup_jobs` would void it next frame
                    // anyway, but don't double-plant in the meantime.
                    let taken = crop_cells.iter().any(|grid| *grid == cell)
                        || cut_targets.iter().any(|(grid, _)| *grid == cell)
                        // Widened plants query: bushes and shrubs block
                        // deliberate planting too.
                        || plants.iter().any(|(grid, ..)| *grid == cell);
                    if !taken {
                        info!(
                            "director: pawn {pawn:?} plants {} at {cell:?}",
                            plant.label()
                        );
                        match plant {
                            ZonePlant::Wheat => crops::spawn_crop(&mut commands, &assets, cell),
                            ZonePlant::Trees => trees::plant_sapling(&mut commands, &assets, cell),
                        }
                        completed.write(JobCompleted {
                            pawn,
                            label: format!("Planted {}", plant.label()),
                        });
                    }
                    commands.entity(pawn).remove::<CurrentJob>();
                } else if nav.find_path(*pawn_grid, cell).is_some() {
                    commands.entity(pawn).insert(MoveOrder { goal: cell });
                } else {
                    warn!("director: job {:?} unreachable, marking stuck", current.0);
                    commands
                        .entity(current.0)
                        .insert(Stuck::new())
                        .remove::<AssignedTo>();
                    commands.entity(pawn).remove::<CurrentJob>();
                }
            }
            JobKind::Supply => {
                let distance_to = |grid: GridCoords| {
                    let (dx, dy) = (grid.x - pawn_grid.x, grid.y - pawn_grid.y);
                    dx * dx + dy * dy
                };
                // The run is stateless per frame: carrying wood = delivery
                // leg, empty-handed = fetch leg. An interrupted run needs no
                // unwinding — `deliver_carried` reclaims any orphaned load.
                if let Ok(mut carrying) = carriers.get_mut(pawn) {
                    let Some(goal) = blueprints
                        .iter()
                        .filter(|(blueprint, _, _)| blueprint.needed() > 0)
                        .map(|(_, grid, _)| *grid)
                        .min_by_key(|grid| distance_to(*grid))
                    else {
                        // Every site is supplied: the run is over; the
                        // leftover load walks home via `deliver_carried`. No
                        // JobCompleted: Supply is haul-family work and would
                        // flood the history like Haul/Merge does.
                        commands.entity(current.0).despawn();
                        commands.entity(pawn).remove::<CurrentJob>();
                        continue;
                    };
                    if *pawn_grid == goal {
                        if let Some((mut blueprint, _, _)) =
                            blueprints.iter_mut().find(|(_, grid, _)| **grid == goal)
                        {
                            let poured = blueprint.needed().min(carrying.amount);
                            blueprint.delivered += poured;
                            carrying.amount -= poured;
                            info!("director: pawn {pawn:?} supplies {poured} wood at {goal:?}");
                        }
                        if carrying.amount == 0 {
                            // Next frame decides: fetch more or wrap up.
                            commands.entity(pawn).remove::<Carrying>();
                        }
                        continue;
                    }
                    // Nearest needy site, but only reachable ones get a trip.
                    let reachable = blueprints
                        .iter()
                        .filter(|(blueprint, _, _)| blueprint.needed() > 0)
                        .map(|(_, grid, _)| *grid)
                        .filter(|grid| nav.find_path(*pawn_grid, *grid).is_some())
                        .min_by_key(|grid| distance_to(*grid));
                    match reachable {
                        Some(goal) => {
                            commands.entity(pawn).insert(MoveOrder { goal });
                        }
                        None => {
                            warn!("director: job {:?} unreachable, marking stuck", current.0);
                            commands
                                .entity(current.0)
                                .insert(Stuck::new())
                                .remove::<AssignedTo>();
                            commands.entity(pawn).remove::<CurrentJob>();
                        }
                    }
                } else {
                    let total_need: u32 = blueprints
                        .iter()
                        .map(|(blueprint, _, _)| blueprint.needed())
                        .sum();
                    if total_need == 0 {
                        commands.entity(current.0).despawn();
                        commands.entity(pawn).remove::<CurrentJob>();
                        continue;
                    }
                    let nearest = stacks
                        .iter()
                        .filter(|(_, stack, _)| stack.kind == ItemKind::Wood)
                        .map(|(entity, _, grid)| (entity, *grid))
                        .min_by_key(|(_, grid)| distance_to(*grid));
                    match nearest {
                        // No wood left anywhere: `cleanup_jobs` voids the
                        // job; park it meanwhile.
                        None => {
                            commands
                                .entity(current.0)
                                .insert(Stuck::new())
                                .remove::<AssignedTo>();
                            commands.entity(pawn).remove::<CurrentJob>();
                        }
                        Some((stack_entity, stack_grid)) if *pawn_grid == stack_grid => {
                            // Partial pickup: take only what the sites still
                            // need (capped per trip), leave the rest.
                            if let Ok((_, mut stack, _)) = stacks.get_mut(stack_entity) {
                                let take = construction::supply_load(total_need, stack.amount);
                                stack.amount -= take;
                                if stack.amount == 0 {
                                    commands.entity(stack_entity).despawn();
                                }
                                if take > 0 {
                                    info!("director: pawn {pawn:?} loads {take} wood for supply");
                                    commands.entity(pawn).insert(Carrying {
                                        kind: ItemKind::Wood,
                                        amount: take,
                                    });
                                }
                            }
                        }
                        Some((_, stack_grid))
                            if nav.find_path(*pawn_grid, stack_grid).is_some() =>
                        {
                            commands.entity(pawn).insert(MoveOrder { goal: stack_grid });
                        }
                        Some(_) => {
                            // Nearest stack walled off: any reachable one.
                            let reachable = stacks
                                .iter()
                                .filter(|(_, stack, _)| stack.kind == ItemKind::Wood)
                                .map(|(_, _, grid)| *grid)
                                .filter(|grid| nav.find_path(*pawn_grid, *grid).is_some())
                                .min_by_key(|grid| distance_to(*grid));
                            match reachable {
                                Some(goal) => {
                                    commands.entity(pawn).insert(MoveOrder { goal });
                                }
                                None => {
                                    warn!(
                                        "director: job {:?} unreachable, marking stuck",
                                        current.0
                                    );
                                    commands
                                        .entity(current.0)
                                        .insert(Stuck::new())
                                        .remove::<AssignedTo>();
                                    commands.entity(pawn).remove::<CurrentJob>();
                                }
                            }
                        }
                    }
                }
            }
            JobKind::Build { site } => {
                let Ok((blueprint, _site_grid, footprint)) = blueprints.get(site) else {
                    // Canceled or already built; `cleanup_jobs` voids the job.
                    commands.entity(pawn).remove::<CurrentJob>();
                    continue;
                };
                let adjacent = adjacent_to_footprint(*pawn_grid, &footprint.cells);
                if adjacent {
                    if let Ok(mut progress) = work_progress.get_mut(current.0) {
                        if progress.0.tick(time.delta()).is_finished() {
                            // A wall (turbine/solar-panel base) must not
                            // entomb whatever stands on its footprint; if
                            // occupied, retry shortly via Stuck. Doors stay
                            // walkable, so they skip the check.
                            let blocked = blueprint.kind.blocks_movement()
                                && footprint.cells.iter().any(|cell| {
                                    pawn_cells.iter().any(|grid| grid == cell)
                                        || stacks.iter().any(|(_, _, grid)| grid == cell)
                                        || plants.iter().any(|(grid, ..)| grid == cell)
                                        || cut_targets.iter().any(|(grid, _)| grid == cell)
                                });
                            if blocked {
                                warn!("director: build site {site:?} occupied, retrying shortly");
                                commands.entity(current.0).insert(Stuck::new()).remove::<(
                                    AssignedTo,
                                    WorkProgress,
                                )>(
                                );
                                commands.entity(pawn).remove::<CurrentJob>();
                            } else {
                                info!("director: pawn {pawn:?} finishes building {site:?}");
                                builds.write(BuildCommand { site });
                                completed.write(JobCompleted {
                                    pawn,
                                    label: format!("Built {}", blueprint.kind.label()),
                                });
                                commands.entity(pawn).remove::<CurrentJob>();
                            }
                        }
                    } else {
                        commands
                            .entity(current.0)
                            .insert(WorkProgress(Timer::from_seconds(
                                blueprint.kind.build_secs(),
                                TimerMode::Once,
                            )));
                    }
                    continue;
                }
                // Walk to the reachable neighbor of the footprint with the
                // shortest path (same approach as Cut, generalized over
                // every footprint cell rather than a single site).
                match footprint_stand_spot(&nav, *pawn_grid, &footprint.cells) {
                    Some(goal) => {
                        commands.entity(pawn).insert(MoveOrder { goal });
                    }
                    None => {
                        warn!("director: job {:?} unreachable, marking stuck", current.0);
                        commands
                            .entity(current.0)
                            .insert(Stuck::new())
                            .remove::<(AssignedTo, WorkProgress)>();
                        commands.entity(pawn).remove::<CurrentJob>();
                    }
                }
            }
            JobKind::Demolish { site } => {
                let Ok((site_grid, footprint)) = demolish_sites.get(site) else {
                    // Canceled or already demolished; `cleanup_jobs` voids
                    // the job.
                    commands.entity(pawn).remove::<CurrentJob>();
                    continue;
                };
                // Pre-check a drop cell for the salvaged wood exists before
                // committing the work, same as Cut/Harvest.
                let stack_list: Vec<(ItemKind, GridCoords, u32)> = stacks
                    .iter()
                    .map(|(_, stack, grid)| (stack.kind, *grid, stack.amount))
                    .collect();
                let occupied_cells: HashSet<GridCoords> = occupied.iter().copied().collect();
                if items::find_drop_cell(
                    ItemKind::Wood,
                    *site_grid,
                    YIELD_DROP_RADIUS,
                    &stack_list,
                    &occupied_cells,
                    |cell| nav.is_walkable(cell),
                )
                .is_none()
                {
                    warn!("director: nowhere to drop the salvage of {site:?}, marking job stuck");
                    commands
                        .entity(current.0)
                        .insert(Stuck::new())
                        .remove::<(AssignedTo, WorkProgress)>();
                    commands.entity(pawn).remove::<CurrentJob>();
                    continue;
                }
                let adjacent = adjacent_to_footprint(*pawn_grid, &footprint.cells);
                if adjacent {
                    if let Ok(mut progress) = work_progress.get_mut(current.0) {
                        if progress.0.tick(time.delta()).is_finished() {
                            let label = construction
                                .get(*site_grid)
                                .map(|kind| kind.label())
                                .unwrap_or("structure");
                            info!("director: pawn {pawn:?} demolishes {site:?} from {pawn_grid:?}");
                            demolitions.write(DemolishCommand {
                                site,
                                drop_at: *pawn_grid,
                            });
                            completed.write(JobCompleted {
                                pawn,
                                label: format!("Demolished a {label}"),
                            });
                            commands.entity(pawn).remove::<CurrentJob>();
                        }
                    } else {
                        commands
                            .entity(current.0)
                            .insert(WorkProgress(Timer::from_seconds(
                                DEMOLISH_SECS,
                                TimerMode::Once,
                            )));
                    }
                    continue;
                }
                // Walk to the reachable neighbor of the footprint with the
                // shortest path (same approach as Build/Cut).
                match footprint_stand_spot(&nav, *pawn_grid, &footprint.cells) {
                    Some(goal) => {
                        commands.entity(pawn).insert(MoveOrder { goal });
                    }
                    None => {
                        warn!("director: job {:?} unreachable, marking stuck", current.0);
                        commands
                            .entity(current.0)
                            .insert(Stuck::new())
                            .remove::<(AssignedTo, WorkProgress)>();
                        commands.entity(pawn).remove::<CurrentJob>();
                    }
                }
            }
            JobKind::BuildRoof { cell } | JobKind::RemoveRoof { cell } => {
                let install = matches!(job.kind, JobKind::BuildRoof { .. });
                // Roofs may sit on a blocked cell (a wall/door) as well as
                // walkable ground, so work happens standing on it OR beside
                // it — unlike Harvest/Cut/Build, which never stand on the
                // target.
                let in_range = *pawn_grid == cell
                    || (pawn_grid.x - cell.x).abs() + (pawn_grid.y - cell.y).abs() == 1;
                if in_range {
                    if let Ok(mut progress) = work_progress.get_mut(current.0) {
                        if progress.0.tick(time.delta()).is_finished() {
                            info!(
                                "director: pawn {pawn:?} {} the roof at {cell:?}",
                                if install { "installs" } else { "removes" }
                            );
                            roof_cmds.write(RoofCommand { cell, install });
                            completed.write(JobCompleted {
                                pawn,
                                label: if install {
                                    "Installed a roof".to_string()
                                } else {
                                    "Removed a roof".to_string()
                                },
                            });
                            commands.entity(pawn).remove::<CurrentJob>();
                        }
                    } else {
                        let secs = if install {
                            ROOF_BUILD_SECS
                        } else {
                            ROOF_REMOVE_SECS
                        };
                        commands
                            .entity(current.0)
                            .insert(WorkProgress(Timer::from_seconds(secs, TimerMode::Once)));
                    }
                } else if let Some(goal) = nearest_stand_spot(&nav, *pawn_grid, cell, true) {
                    commands.entity(pawn).insert(MoveOrder { goal });
                } else {
                    warn!("director: job {:?} unreachable, marking stuck", current.0);
                    commands
                        .entity(current.0)
                        .insert(Stuck::new())
                        .remove::<(AssignedTo, WorkProgress)>();
                    commands.entity(pawn).remove::<CurrentJob>();
                }
            }
        }
    }
}

/// Walk carried items to the stockpile and deposit them, splitting across
/// cells when a stack tops out at the cap. Only same-kind stacks are topped
/// up — a cell already holding a different kind is treated as occupied.
#[allow(clippy::type_complexity)]
pub fn deliver_carried(
    mut commands: Commands,
    nav: Res<NavGrid>,
    stockpile: Res<Stockpile>,
    assets: Res<GameAssets>,
    // `Without<CurrentJob>` keeps mid-Supply pawns out of here: their load
    // belongs to the build sites until the run wraps up (or is released,
    // which drops CurrentJob and hands the leftover to this system).
    mut carriers: Query<
        (Entity, &GridCoords, &mut Carrying),
        (
            With<Pawn>,
            Without<Path>,
            Without<MoveOrder>,
            Without<CurrentJob>,
            // A drafted pawn holds onto whatever it's carrying instead of
            // auto-walking to the stockpile, which would fight the player's
            // clicks; delivery resumes once it's un-drafted.
            Without<ManualMode>,
        ),
    >,
    mut stacks: Query<(&mut ItemStack, &GridCoords)>,
    occupied: Query<&GridCoords, With<Selectable>>,
) {
    for (pawn, pawn_grid, mut carrying) in &mut carriers {
        let kind = carrying.kind;
        // Best deposit cell: top up started same-kind stacks before opening
        // a fresh one, then by distance. Never "deposit where I happen to
        // stand" — that would pour a just-picked-up merge stack straight back.
        let target = stockpile
            .cells
            .iter()
            .filter_map(|cell| match stacks.iter().find(|(_, grid)| *grid == cell) {
                Some((stack, _)) if stack.kind == kind && stack.space() > 0 => {
                    Some((*cell, false)) // started, has space
                }
                Some(_) => None,             // full, or holds a different kind
                None => Some((*cell, true)), // fresh cell
            })
            .min_by_key(|(cell, fresh)| {
                let (dx, dy) = (cell.x - pawn_grid.x, cell.y - pawn_grid.y);
                (*fresh, dx * dx + dy * dy)
            });

        // With the stockpile unable to take this kind, fall back to the
        // nearest usable ground cell (spilling outward) instead of blindly
        // dumping at the feet — which could pile two stacks on one cell.
        let destination = target.map(|(cell, _)| cell).or_else(|| {
            let stack_list: Vec<(ItemKind, GridCoords, u32)> = stacks
                .iter()
                .map(|(stack, grid)| (stack.kind, *grid, stack.amount))
                .collect();
            let occupied_cells: HashSet<GridCoords> = occupied.iter().copied().collect();
            items::find_drop_cell(
                kind,
                *pawn_grid,
                YIELD_DROP_RADIUS,
                &stack_list,
                &occupied_cells,
                |cell| nav.is_walkable(cell),
            )
        });

        match destination {
            Some(cell) if cell == *pawn_grid => {
                if !stockpile.cells.contains(pawn_grid) {
                    warn!(
                        "director: no stockpile space, dropping {} {} at {pawn_grid:?}",
                        carrying.amount,
                        kind.label()
                    );
                }
                if let Some((mut stack, _)) = stacks
                    .iter_mut()
                    .find(|(stack, grid)| *grid == pawn_grid && stack.kind == kind)
                {
                    let added = stack.space().min(carrying.amount);
                    stack.amount += added;
                    carrying.amount -= added;
                } else {
                    info!(
                        "director: pawn {pawn:?} deposits {} {} at {pawn_grid:?}",
                        carrying.amount,
                        kind.label()
                    );
                    items::spawn_item_stack(
                        &mut commands,
                        &assets,
                        kind,
                        *pawn_grid,
                        carrying.amount,
                    );
                    carrying.amount = 0;
                }
                if carrying.amount == 0 {
                    commands.entity(pawn).remove::<Carrying>();
                }
            }
            Some(cell) if nav.find_path(*pawn_grid, cell).is_some() => {
                commands.entity(pawn).insert(MoveOrder { goal: cell });
            }
            // Nowhere at all to put it down (walled in): keep holding the
            // load and retry next frame; capacity-gated haul generation
            // means no job churn happens meanwhile.
            _ => {}
        }
    }
}

/// Show/hide the little item cube riding on carrying pawns, colored by
/// what's being carried.
#[allow(clippy::type_complexity)]
pub fn sync_carry_visual(
    mut commands: Commands,
    assets: Res<GameAssets>,
    started: Query<(Entity, &Carrying), (With<Pawn>, Added<Carrying>)>,
    mut stopped: RemovedComponents<Carrying>,
    visuals: Query<(Entity, &CarryVisual)>,
) {
    for (pawn, carrying) in &started {
        let material = match carrying.kind {
            ItemKind::Berries => assets.berry_stack_material.clone(),
            ItemKind::Wheat => assets.wheat_stack_material.clone(),
            ItemKind::Wood => assets.wood_stack_material.clone(),
        };
        commands.entity(pawn).with_children(|parent| {
            parent.spawn((
                Mesh3d(assets.item_stack_mesh.clone()),
                MeshMaterial3d(material),
                Transform::from_translation(Vec3::Y * (PAWN_HEIGHT + ITEM_STACK_HEIGHT) / 2.0),
                CarryVisual(pawn),
            ));
        });
    }
    for pawn in stopped.read() {
        for (visual, carry) in &visuals {
            if carry.0 == pawn {
                commands.entity(visual).despawn();
            }
        }
    }
}

/// Tip a *resting* pawn onto its side so it reads as "lying down" at a
/// glance; upright again the moment it's up and about. The pawn mesh is a
/// tall rectangle (`PAWN_WIDTH` x `PAWN_HEIGHT` x `PAWN_WIDTH`, `game.rs`'s
/// `pawn_mesh`), not a cube — unlike a cube, rotating it is actually visible,
/// and unlike scaling it, this stays correct once the mesh is a real
/// character model instead of a placeholder box (a scale hack would flatten
/// a model's body).
///
/// "Resting" is `Objective::Sleep` **and stationary** (no `Path`/`MoveOrder`)
/// — not `Objective::Sleep` alone: since beds exist, a tired pawn can be
/// mid-walk to one, and tipping it over while it's still moving would look
/// wrong (and fight the very translation writes below). Covers both the
/// ground-fallback nap and resting in a bed identically; `needs::decay_needs`
/// is what tells those two apart (via `InBed`) for the recovery *rate*, this
/// system only cares whether the pawn is holding still.
///
/// Ungated (a purely cosmetic follower). While resting it also nudges
/// `translation.y` to keep the tipped-over box's belly flush with the
/// ground: rotating 90 degrees around Z swaps the box's vertical half-extent
/// from `PAWN_HEIGHT / 2` to `PAWN_WIDTH / 2`, so resting at the same height
/// as standing would leave a gap underneath. It only ever touches
/// `translation.y` while the pawn has no `Path`/`MoveOrder` — the moment one
/// exists, `movement::plan_paths`/`move_along_path` own that field
/// exclusively (including the Wader dip through water), so this system backs
/// off entirely rather than fighting them over the same frame.
pub fn sync_sleeping_pose(
    mut pawns: Query<
        (
            &Objective,
            &mut Transform,
            &Wader,
            Has<Path>,
            Has<MoveOrder>,
        ),
        With<Pawn>,
    >,
) {
    for (objective, mut transform, wader, has_path, has_order) in &mut pawns {
        let moving = has_path || has_order;
        let resting = *objective == Objective::Sleep && !moving;
        let wanted_rotation = if resting {
            Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)
        } else {
            Quat::IDENTITY
        };
        if transform.rotation != wanted_rotation {
            transform.rotation = wanted_rotation;
        }
        if !moving {
            let wanted_y = if resting {
                wader.base_y - (PAWN_HEIGHT - PAWN_WIDTH) / 2.0
            } else {
                wader.base_y
            };
            if transform.translation.y != wanted_y {
                transform.translation.y = wanted_y;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cells(list: &[(i32, i32)]) -> HashSet<GridCoords> {
        list.iter().map(|(x, y)| GridCoords::new(*x, *y)).collect()
    }

    #[test]
    fn rng_chance_respects_probability_edges() {
        let mut rng = WanderRng::default();
        for _ in 0..1000 {
            assert!(!rng.chance(0.0));
            assert!(rng.chance(1.0));
        }
        // A middling probability lands somewhere in the middle.
        let hits = (0..10_000).filter(|_| rng.chance(0.5)).count();
        assert!((4_000..6_000).contains(&hits), "hits {hits}");
    }

    #[test]
    fn empty_stockpile_takes_full_stacks_per_cell() {
        let cells = cells(&[(1, 1), (1, 2)]);
        assert_eq!(
            stockpile_capacity(ItemKind::Berries, &cells, &[]),
            2 * STACK_CAP
        );
    }

    #[test]
    fn full_stockpile_has_no_capacity() {
        let cells = cells(&[(1, 1)]);
        let stacks = [(ItemKind::Berries, GridCoords::new(1, 1), STACK_CAP)];
        assert_eq!(stockpile_capacity(ItemKind::Berries, &cells, &stacks), 0);
    }

    #[test]
    fn sync_sleeping_pose_tips_sleeping_pawns_and_stands_the_rest_upright() {
        use bevy::ecs::system::RunSystemOnce;

        let mut world = World::new();
        let sleeping = world
            .spawn((
                Pawn,
                Objective::Sleep,
                Transform::from_xyz(0.0, PAWN_HEIGHT / 2.0, 0.0),
                Wader {
                    base_y: PAWN_HEIGHT / 2.0,
                },
            ))
            .id();
        let awake = world
            .spawn((
                Pawn,
                Objective::Work,
                Transform::from_xyz(0.0, PAWN_HEIGHT / 2.0, 0.0),
                Wader {
                    base_y: PAWN_HEIGHT / 2.0,
                },
            ))
            .id();

        world.run_system_once(sync_sleeping_pose).unwrap();

        let sleeping_transform = world.get::<Transform>(sleeping).unwrap();
        assert_eq!(
            sleeping_transform.rotation,
            Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)
        );
        assert_eq!(
            sleeping_transform.translation.y,
            PAWN_HEIGHT / 2.0 - (PAWN_HEIGHT - PAWN_WIDTH) / 2.0
        );

        let awake_transform = world.get::<Transform>(awake).unwrap();
        assert_eq!(awake_transform.rotation, Quat::IDENTITY);
        assert_eq!(awake_transform.translation.y, PAWN_HEIGHT / 2.0);
    }

    #[test]
    fn sync_sleeping_pose_leaves_a_tired_pawn_upright_while_it_walks_to_bed() {
        use bevy::ecs::system::RunSystemOnce;

        // A pawn on the way to a bed is `Objective::Sleep` but still moving
        // (`MoveOrder`/`Path` — `MoveOrder` is enough to trip the `moving`
        // gate) — it must stay upright, and its translation.y untouched
        // (movement/Wader own that field while it's moving), until it
        // actually arrives and stops.
        let mut world = World::new();
        let walking = world
            .spawn((
                Pawn,
                Objective::Sleep,
                Transform::from_xyz(0.0, 0.71, 0.0),
                Wader {
                    base_y: PAWN_HEIGHT / 2.0,
                },
                MoveOrder {
                    goal: GridCoords::new(0, 0),
                },
            ))
            .id();

        world.run_system_once(sync_sleeping_pose).unwrap();

        let transform = world.get::<Transform>(walking).unwrap();
        assert_eq!(transform.rotation, Quat::IDENTITY);
        // Untouched — not snapped back to `wader.base_y` either, since a
        // wading pawn's dip is a legitimate reason for it to differ.
        assert_eq!(transform.translation.y, 0.71);
    }

    #[test]
    fn started_same_kind_stack_counts_its_space() {
        let cells = cells(&[(1, 1)]);
        let stacks = [(ItemKind::Berries, GridCoords::new(1, 1), STACK_CAP - 30)];
        assert_eq!(stockpile_capacity(ItemKind::Berries, &cells, &stacks), 30);
    }

    #[test]
    fn other_kind_stack_blocks_its_cell() {
        // A cell full of wheat takes no berries, even half-empty.
        let cells = cells(&[(1, 1)]);
        let stacks = [(ItemKind::Wheat, GridCoords::new(1, 1), 10)];
        assert_eq!(stockpile_capacity(ItemKind::Berries, &cells, &stacks), 0);
        assert_eq!(
            stockpile_capacity(ItemKind::Wheat, &cells, &stacks),
            STACK_CAP - 10
        );
    }

    fn at(x: i32, y: i32) -> GridCoords {
        GridCoords::new(x, y)
    }

    #[test]
    fn nearest_stand_spot_picks_a_free_neighbor() {
        let nav = NavGrid::default();
        // All 4 neighbors free: any of them is fine, but it must be one of
        // them, never the target itself (include_target: false).
        let spot = nearest_stand_spot(&nav, at(0, 0), at(5, 5), false).unwrap();
        let dist = (spot.x - 5).abs() + (spot.y - 5).abs();
        assert_eq!(dist, 1);
    }

    #[test]
    fn nearest_stand_spot_can_include_the_target_itself() {
        let mut nav = NavGrid::default();
        // Wall off all 4 neighbors of the target — only the target cell
        // itself (walkable) remains a valid stand spot, e.g. the lone
        // interior of a 1x1 room.
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            nav.set_walkable(at(5 + dx, 5 + dy), false);
        }
        assert_eq!(
            nearest_stand_spot(&nav, at(5, 5), at(5, 5), true),
            Some(at(5, 5))
        );
        // Without include_target, the same fully-walled target is
        // unreachable.
        assert_eq!(nearest_stand_spot(&nav, at(5, 5), at(5, 5), false), None);
    }

    #[test]
    fn nearest_stand_spot_none_when_every_candidate_is_blocked() {
        let mut nav = NavGrid::default();
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            nav.set_walkable(at(5 + dx, 5 + dy), false);
        }
        nav.set_walkable(at(5, 5), false);
        assert_eq!(nearest_stand_spot(&nav, at(0, 0), at(5, 5), true), None);
    }

    #[test]
    fn footprint_stand_spot_matches_nearest_stand_spot_for_one_cell() {
        // A single-cell footprint is exactly `nearest_stand_spot` with
        // `include_target: false` — the builder never stands on the site.
        let nav = NavGrid::default();
        let footprint = vec![at(5, 5)];
        let spot = footprint_stand_spot(&nav, at(0, 0), &footprint).unwrap();
        assert_eq!(
            nearest_stand_spot(&nav, at(0, 0), at(5, 5), false),
            Some(spot)
        );
    }

    #[test]
    fn footprint_stand_spot_stays_outside_a_2x2_block() {
        let nav = NavGrid::default();
        // The solar panel's 2x2 footprint anchored at (5, 5).
        let footprint = vec![at(5, 5), at(6, 5), at(5, 6), at(6, 6)];
        let spot = footprint_stand_spot(&nav, at(0, 0), &footprint).unwrap();
        assert!(!footprint.contains(&spot));
        // It must be orthogonally adjacent to at least one footprint cell.
        assert!(footprint
            .iter()
            .any(|cell| (spot.x - cell.x).abs() + (spot.y - cell.y).abs() == 1));
    }

    #[test]
    fn footprint_stand_spot_none_when_fully_walled_in() {
        let mut nav = NavGrid::default();
        let footprint = vec![at(5, 5), at(6, 5), at(5, 6), at(6, 6)];
        // Wall off every cell orthogonally adjacent to the 2x2 block.
        for (dx, dy) in [
            (-1, 0),
            (-1, 1),
            (0, -1),
            (1, -1),
            (2, 0),
            (2, 1),
            (0, 2),
            (1, 2),
        ] {
            nav.set_walkable(at(5 + dx, 5 + dy), false);
        }
        assert_eq!(footprint_stand_spot(&nav, at(0, 0), &footprint), None);
    }

    #[test]
    fn adjacent_to_footprint_accepts_a_beside_pawn() {
        let footprint = vec![at(5, 5)];
        assert!(adjacent_to_footprint(at(6, 5), &footprint));
        assert!(adjacent_to_footprint(at(4, 5), &footprint));
        assert!(adjacent_to_footprint(at(5, 6), &footprint));
        assert!(adjacent_to_footprint(at(5, 4), &footprint));
    }

    #[test]
    fn adjacent_to_footprint_rejects_standing_on_or_off_the_footprint() {
        let footprint = vec![at(5, 5)];
        // On the target cell itself: not adjacent (must stand beside).
        assert!(!adjacent_to_footprint(at(5, 5), &footprint));
        // Diagonal: not orthogonally adjacent.
        assert!(!adjacent_to_footprint(at(6, 6), &footprint));
        // Far away.
        assert!(!adjacent_to_footprint(at(0, 0), &footprint));
    }

    #[test]
    fn adjacent_to_footprint_excludes_every_footprint_cell_of_a_multi_cell_block() {
        // The 2x2 solar panel: standing on (5,5) is orthogonally adjacent to
        // footprint cell (6,5), but must NOT count as a valid build stance —
        // the pawn would be building the floor under its own feet.
        let footprint = vec![at(5, 5), at(6, 5), at(5, 6), at(6, 6)];
        for cell in &footprint {
            assert!(!adjacent_to_footprint(*cell, &footprint));
        }
        // A cell genuinely outside the block, adjacent to one side, is fine.
        assert!(adjacent_to_footprint(at(4, 5), &footprint));
        assert!(adjacent_to_footprint(at(7, 5), &footprint));
    }
}
