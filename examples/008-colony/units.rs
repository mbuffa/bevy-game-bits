use bevy::prelude::*;
use bevy_ecs_ldtk::prelude::*;

use crate::config::*;
use crate::director::{JobKind, Objective, PawnStatus};
use crate::game::GameAssets;
use crate::history::JobHistory;
use crate::map;
use crate::movement::Wader;
use crate::needs::Needs;
use crate::selection::Selectable;

/// Data marker inserted by bevy_ecs_ldtk for each `Pawn` entity instance
/// placed in the LDtk map.
#[derive(Default, Component)]
pub struct PawnSpawn;

/// Stable unique identifier from the LDtk `Id` field. Nothing reads it yet;
/// it's the hook for save/load and cross-references that must survive
/// entity respawns.
#[derive(Default, Component, Clone, Copy)]
pub struct PawnId(#[allow(dead_code)] pub u32);

/// Display name from the LDtk `PawnName` field.
#[derive(Default, Component, Clone)]
pub struct PawnName(pub String);

/// Skill tier, best to worst. Affects which jobs a pawn prefers, not (yet)
/// how fast it works.
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tier {
    S,
    A,
    B,
    #[default]
    C,
    D,
}

impl Tier {
    /// Higher is better (S = 5 .. D = 1), for assignment scoring.
    pub fn rank(&self) -> u8 {
        match self {
            Tier::S => 5,
            Tier::A => 4,
            Tier::B => 3,
            Tier::C => 2,
            Tier::D => 1,
        }
    }

    pub fn letter(&self) -> &'static str {
        match self {
            Tier::S => "S",
            Tier::A => "A",
            Tier::B => "B",
            Tier::C => "C",
            Tier::D => "D",
        }
    }

    fn parse(text: &str) -> Tier {
        match text {
            "S" => Tier::S,
            "A" => Tier::A,
            "B" => Tier::B,
            "D" => Tier::D,
            _ => Tier::C,
        }
    }
}

/// Per-job-kind skill tiers from the LDtk `HarvestSkill`/`HaulSkill`/
/// `FarmSkill`/`ForestrySkill`/`BuildSkill` fields (fields the map doesn't
/// set get the default C tier).
#[derive(Default, Component, Clone, Copy)]
pub struct Skills {
    pub harvest: Tier,
    pub haul: Tier,
    pub farm: Tier,
    pub forestry: Tier,
    pub build: Tier,
}

fn skills_from_fields(entity_instance: &EntityInstance) -> Skills {
    let tier = |field: &str| {
        Tier::parse(
            entity_instance
                .get_string_field(field)
                .map(|s| s.as_str())
                .unwrap_or("C"),
        )
    };
    Skills {
        harvest: tier("HarvestSkill"),
        haul: tier("HaulSkill"),
        farm: tier("FarmSkill"),
        forestry: tier("ForestrySkill"),
        build: tier("BuildSkill"),
    }
}

fn pawn_id_from_field(entity_instance: &EntityInstance) -> PawnId {
    let id = *entity_instance.get_int_field("Id").unwrap_or(&0);
    PawnId(id.max(0) as u32)
}

fn pawn_name_from_field(entity_instance: &EntityInstance) -> PawnName {
    let name = entity_instance
        .get_string_field("PawnName")
        .cloned()
        .unwrap_or_else(|_| String::from("Pawn"));
    PawnName(name)
}

#[derive(Default, Bundle, LdtkEntity)]
pub struct PawnSpawnBundle {
    marker: PawnSpawn,
    #[grid_coords]
    grid_coords: GridCoords,
    #[with(pawn_id_from_field)]
    id: PawnId,
    #[with(pawn_name_from_field)]
    name: PawnName,
    #[with(skills_from_fields)]
    skills: Skills,
}

/// Which work types this pawn may be assigned (the Allowance panel toggles
/// them). Recreation is always allowed.
#[derive(Component, Clone, Copy)]
pub struct Allowance {
    pub harvest: bool,
    pub haul: bool,
    pub farm: bool,
    pub forestry: bool,
    pub build: bool,
}

impl Default for Allowance {
    fn default() -> Self {
        Self {
            harvest: true,
            haul: true,
            farm: true,
            forestry: true,
            build: true,
        }
    }
}

impl Allowance {
    pub fn allows(&self, kind: &JobKind) -> bool {
        match kind {
            JobKind::Harvest { .. } => self.harvest,
            // Merging is hauling work: same allowance, same skill. Supply
            // runs are too — carrying wood to a site is still hauling.
            JobKind::Haul { .. } | JobKind::Merge { .. } | JobKind::Supply => self.haul,
            JobKind::Farming { .. } => self.farm,
            // Cutting trees is forestry, deliberately not harvest work.
            JobKind::Cut { .. } => self.forestry,
            // Raising walls/doors, roofing, and demolition are all
            // construction proper — tearing a structure down shares the
            // Build skill/allowance with putting one up.
            JobKind::Build { .. }
            | JobKind::BuildRoof { .. }
            | JobKind::RemoveRoof { .. }
            | JobKind::Demolish { .. } => self.build,
            // Recreation and sleep are never gated behind a work toggle —
            // strolling and resting are biological/personal, not work.
            JobKind::Walk { .. } | JobKind::Sleep { .. } => true,
        }
    }
}

/// A colonist; movement, actions and AI will attach here.
#[derive(Component)]
pub struct Pawn;

/// Player has taken this pawn off AI control ("drafted"). The Director skips
/// it entirely; the player right-clicks to move it instead. Absent by
/// default — pawns start under AI control.
#[derive(Component)]
pub struct ManualMode;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cut_jobs_follow_the_forestry_allowance() {
        let target = Entity::PLACEHOLDER;
        let mut allowance = Allowance::default();
        assert!(allowance.allows(&JobKind::Cut { target }));
        allowance.forestry = false;
        assert!(!allowance.allows(&JobKind::Cut { target }));
        // Forestry is its own bag: harvest stays unaffected.
        assert!(allowance.allows(&JobKind::Harvest { target }));
    }

    #[test]
    fn construction_jobs_follow_the_build_allowance() {
        let site = Entity::PLACEHOLDER;
        let cell = GridCoords::new(3, 3);
        let mut allowance = Allowance::default();
        assert!(allowance.allows(&JobKind::Build { site }));
        assert!(allowance.allows(&JobKind::BuildRoof { cell }));
        assert!(allowance.allows(&JobKind::RemoveRoof { cell }));
        assert!(allowance.allows(&JobKind::Demolish { site }));
        allowance.build = false;
        assert!(!allowance.allows(&JobKind::Build { site }));
        assert!(!allowance.allows(&JobKind::BuildRoof { cell }));
        assert!(!allowance.allows(&JobKind::RemoveRoof { cell }));
        assert!(!allowance.allows(&JobKind::Demolish { site }));
        // Supply runs are hauling work, not build work.
        assert!(allowance.allows(&JobKind::Supply));
        allowance.haul = false;
        assert!(!allowance.allows(&JobKind::Supply));
    }
}

pub fn spawn_pawn_visual(
    mut commands: Commands,
    assets: Res<GameAssets>,
    spawns: Query<(&GridCoords, &PawnId, &PawnName, &Skills), Added<PawnSpawn>>,
) {
    for (grid, id, name, skills) in &spawns {
        commands.spawn((
            Mesh3d(assets.pawn_mesh.clone()),
            MeshMaterial3d(assets.pawn_material.clone()),
            Transform::from_translation(map::grid_to_world(grid) + Vec3::Y * PAWN_HEIGHT / 2.0),
            Name::new(name.0.clone()),
            *id,
            *skills,
            Pawn,
            PawnStatus::default(),
            Objective::default(),
            Allowance::default(),
            JobHistory::default(),
            Needs::default(),
            Wader {
                base_y: PAWN_HEIGHT / 2.0,
            },
            *grid,
            Selectable,
        ));
    }
}
