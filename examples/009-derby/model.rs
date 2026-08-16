//! Loads each car class's `assets/models/derby-<class>.glb` (written by
//! `export.rs`) and turns its node graph into a `CarRig` — the
//! mesh/material/rest-transform for every visual part, keyed the way
//! `vehicle.rs` needs to spawn a car, `powerups.rs` needs to repair one, and
//! `damage.rs` needs to size its debris. This is the seam an artist's edits
//! cross: reshape a fender in Blender, re-export over the same file, and
//! every one of those call sites follows without a code change.
//!
//! Deliberately not `SceneRoot` + a spawned scene: Bevy exposes the glTF
//! node graph directly as assets (`Gltf::named_nodes`, `GltfNode`,
//! `GltfMesh`), which lets this keep the game's existing hand-built
//! entity hierarchy (per-car paint, per-entity headlight materials, the
//! repair/detach paths) instead of walking a spawned scene and re-deriving
//! all of that after the fact.
//!
//! Node naming is the contract (`docs/car-model.md`): sides are baked into
//! names only to keep them unique. Which side a part actually damages comes
//! from its own node's X translation (`vehicle::side_index`), and whether a
//! wheel steers comes from its Z translation — so a repositioned part is
//! still attributed correctly, not just wherever its name suggests.
//!
//! Every car class shares this one node contract — `build_car_rig` runs the
//! same parsing over each class's glTF and only proceeds once every one of
//! them has landed, so `resource_added::<CarRigs>` still fires exactly once.

use bevy::gltf::{Gltf, GltfMesh, GltfNode};
use bevy::prelude::*;

use crate::config::CarClass;
use bevy_game_bits::vehicle::side_index;

/// Handles to every class's in-flight (or loaded) car model, indexed by
/// `CarClass as usize`. A plain wrapper, not a marker of readiness —
/// `build_car_rig` reads `Assets<Gltf>` directly to find out when each is
/// ready.
#[derive(Resource)]
pub(crate) struct CarModels([Handle<Gltf>; 2]);

/// One part's mesh, material and rest pose, read straight off a glTF node.
/// `half_extents` comes from the mesh's own bounding box (not a config.rs
/// constant), so a reshaped part gets a matching debris collider for free
/// when it detaches (`damage::detach`).
#[derive(Clone)]
pub struct RigPart {
    pub mesh: Handle<Mesh>,
    pub material: Handle<StandardMaterial>,
    pub transform: Transform,
    pub half_extents: Vec3,
}

/// The whole car, as read from `derby-car.glb`. Replaces the old
/// `CarAssets` as the seam every spawn/repair/detach site reads from — see
/// `vehicle::spawn_car`, `powerups::respawn_missing_parts`,
/// `damage::detach`. Parts that stay procedural (colliders, the immunity
/// bubble) are not here; see `vehicle::CarAssets` for those.
#[derive(Clone)]
pub struct CarRig {
    pub chassis: RigPart,
    /// FL, FR, RL, RR — matches `damage`/`vehicle`'s wheel-mount ordering.
    pub wheels: [RigPart; 4],
    pub stripes: [RigPart; 4],
    /// Indexed by `vehicle::side_index` (0 = −X, 1 = +X).
    pub headlights: [RigPart; 2],
    pub fenders: [RigPart; 2],
    pub windshield: RigPart,
    pub spoiler: RigPart,
    pub spoiler_struts: [RigPart; 2],
    pub ram_bar: RigPart,
    pub shield_struts: [RigPart; 2],
}

impl CarRig {
    /// Distance between front and rear axles — derived from the wheel
    /// nodes' own Z translations (FL vs RL) rather than a `config.rs`
    /// constant, so the steering model can never disagree with whatever an
    /// artist actually put in the `.glb`.
    pub fn wheelbase(&self) -> f32 {
        (self.wheels[0].transform.translation.z - self.wheels[2].transform.translation.z).abs()
    }

    /// Distance between left and right wheels — from the wheel nodes' own X
    /// translations (FL vs FR), same rationale as `wheelbase`.
    pub fn track_width(&self) -> f32 {
        (self.wheels[0].transform.translation.x - self.wheels[1].transform.translation.x).abs()
    }
}

/// One `CarRig` per class, indexed by `CarClass as usize`. `resource_added::
/// <CarRigs>` (`main.rs`) fires exactly once, the tick every class's glTF has
/// landed — `vehicle::spawn_vehicle` is gated on it right after in the
/// schedule, so the whole grid appears the same frame the rigs do.
#[derive(Resource)]
pub struct CarRigs([CarRig; 2]);

impl CarRigs {
    pub fn get(&self, class: CarClass) -> &CarRig {
        &self.0[class as usize]
    }
}

/// Kicks off the async load for every class. Runs at `Startup`;
/// `build_car_rig` polls for completion every `Update` tick until every rig
/// is built.
pub fn start_loading_car_model(mut commands: Commands, asset_server: Res<AssetServer>) {
    let handles = CarClass::ALL.map(|class| asset_server.load(class.asset_path()));
    commands.insert_resource(CarModels(handles));
}

/// Builds `CarRigs` the first `Update` tick every class's glTF (and
/// everything it references — nodes, meshes, materials — all part of the
/// same load) is ready. A missing or malformed node panics with the node's
/// name rather than spawning a half-built car — a bad model should fail
/// loudly.
pub fn build_car_rig(
    mut commands: Commands,
    models: Option<Res<CarModels>>,
    gltf_assets: Res<Assets<Gltf>>,
    gltf_nodes: Res<Assets<GltfNode>>,
    gltf_meshes: Res<Assets<GltfMesh>>,
    meshes: Res<Assets<Mesh>>,
) {
    let Some(models) = models else { return };

    let mut rigs: Vec<CarRig> = Vec::with_capacity(models.0.len());
    for (class, handle) in CarClass::ALL.iter().zip(models.0.iter()) {
        let Some(gltf) = gltf_assets.get(handle) else {
            return; // At least one class is still loading.
        };
        rigs.push(build_one_rig(gltf, &gltf_nodes, &gltf_meshes, &meshes, class.asset_path()));
    }

    info!("all {} car classes loaded", rigs.len());
    commands.insert_resource(CarRigs(rigs.try_into().unwrap_or_else(|_| unreachable!("exactly CarClass::ALL.len() pushed"))));
}

fn build_one_rig(
    gltf: &Gltf,
    gltf_nodes: &Assets<GltfNode>,
    gltf_meshes: &Assets<GltfMesh>,
    meshes: &Assets<Mesh>,
    model_path: &str,
) -> CarRig {
    const WHEEL_NAMES: [&str; 4] = ["Wheel_FL", "Wheel_FR", "Wheel_RL", "Wheel_RR"];
    let mut wheels: Vec<RigPart> = Vec::with_capacity(4);
    let mut stripes: Vec<RigPart> = Vec::with_capacity(4);
    for name in WHEEL_NAMES {
        let wheel_node = named_node(gltf, gltf_nodes, model_path, name);
        wheels.push(node_part(wheel_node, gltf_meshes, meshes, model_path, name));
        stripes.push(stripe_part(wheel_node, gltf_nodes, gltf_meshes, meshes, model_path, name));
    }

    let rig = CarRig {
        chassis: named_part(gltf, gltf_nodes, gltf_meshes, meshes, model_path, "Chassis"),
        wheels: wheels.try_into().unwrap_or_else(|_| unreachable!("exactly 4 pushed")),
        stripes: stripes.try_into().unwrap_or_else(|_| unreachable!("exactly 4 pushed")),
        headlights: sided_parts(gltf, gltf_nodes, gltf_meshes, meshes, model_path, ["Headlight_R", "Headlight_L"]),
        fenders: sided_parts(gltf, gltf_nodes, gltf_meshes, meshes, model_path, ["Fender_R", "Fender_L"]),
        windshield: named_part(gltf, gltf_nodes, gltf_meshes, meshes, model_path, "Windshield"),
        spoiler: named_part(gltf, gltf_nodes, gltf_meshes, meshes, model_path, "Spoiler"),
        spoiler_struts: sided_parts(gltf, gltf_nodes, gltf_meshes, meshes, model_path, ["SpoilerStrut_R", "SpoilerStrut_L"]),
        ram_bar: named_part(gltf, gltf_nodes, gltf_meshes, meshes, model_path, "RamBar"),
        shield_struts: sided_parts(gltf, gltf_nodes, gltf_meshes, meshes, model_path, ["ShieldStrut_R", "ShieldStrut_L"]),
    };
    info!(
        "{model_path} loaded: chassis + 4 wheels + {} other parts",
        2 + 2 + 1 + 1 + 2 + 1 + 2 // headlights, fenders, windshield, spoiler, spoiler struts, ram bar, shield struts
    );
    rig
}

fn named_node<'a>(gltf: &Gltf, gltf_nodes: &'a Assets<GltfNode>, model_path: &str, name: &str) -> &'a GltfNode {
    let handle = gltf
        .named_nodes
        .get(name)
        .unwrap_or_else(|| panic!("{model_path}: missing required node {name:?} — see docs/car-model.md"));
    gltf_nodes
        .get(handle)
        .unwrap_or_else(|| panic!("{model_path}: node {name:?} not loaded"))
}

fn named_part(
    gltf: &Gltf,
    gltf_nodes: &Assets<GltfNode>,
    gltf_meshes: &Assets<GltfMesh>,
    meshes: &Assets<Mesh>,
    model_path: &str,
    name: &str,
) -> RigPart {
    node_part(named_node(gltf, gltf_nodes, model_path, name), gltf_meshes, meshes, model_path, name)
}

/// A pair of nodes named for uniqueness only (`_R`/`_L`) but placed into
/// `[side_index(x) == 0, side_index(x) == 1]` slots by their own geometry —
/// so an artist swapping which side a part sits on re-sides its damage too.
fn sided_parts(
    gltf: &Gltf,
    gltf_nodes: &Assets<GltfNode>,
    gltf_meshes: &Assets<GltfMesh>,
    meshes: &Assets<Mesh>,
    model_path: &str,
    names: [&str; 2],
) -> [RigPart; 2] {
    let mut slots: [Option<RigPart>; 2] = [None, None];
    for name in names {
        let part = named_part(gltf, gltf_nodes, gltf_meshes, meshes, model_path, name);
        let side = side_index(part.transform.translation.x);
        if slots[side].is_some() {
            panic!("{model_path}: {names:?} both landed on the same side (x sign) — spread them across ±X");
        }
        slots[side] = Some(part);
    }
    let [a, b] = slots;
    [
        a.unwrap_or_else(|| panic!("{model_path}: {names:?} — no node on the −X side")),
        b.unwrap_or_else(|| panic!("{model_path}: {names:?} — no node on the +X side")),
    ]
}

/// A wheel's spin stripe: its one child node, not looked up by name — the
/// stripe can be named anything, since `named_nodes` is a flat map keyed
/// across the whole file and four "Stripe" nodes would collide.
fn stripe_part(
    wheel_node: &GltfNode,
    gltf_nodes: &Assets<GltfNode>,
    gltf_meshes: &Assets<GltfMesh>,
    meshes: &Assets<Mesh>,
    model_path: &str,
    wheel_name: &str,
) -> RigPart {
    let child_handle = wheel_node
        .children
        .first()
        .unwrap_or_else(|| panic!("{model_path}: {wheel_name:?} needs one child node for its spin stripe"));
    let child = gltf_nodes
        .get(child_handle)
        .unwrap_or_else(|| panic!("{model_path}: {wheel_name:?}'s stripe child not loaded"));
    node_part(child, gltf_meshes, meshes, model_path, wheel_name)
}

fn node_part(node: &GltfNode, gltf_meshes: &Assets<GltfMesh>, meshes: &Assets<Mesh>, model_path: &str, context: &str) -> RigPart {
    let mesh_handle = node
        .mesh
        .as_ref()
        .unwrap_or_else(|| panic!("{model_path}: node for {context:?} has no mesh"));
    let gltf_mesh = gltf_meshes
        .get(mesh_handle)
        .unwrap_or_else(|| panic!("{model_path}: {context:?}'s mesh not loaded"));
    let primitive = gltf_mesh
        .primitives
        .first()
        .unwrap_or_else(|| panic!("{model_path}: {context:?}'s mesh has no primitives"));
    let mesh_asset = meshes
        .get(&primitive.mesh)
        .unwrap_or_else(|| panic!("{model_path}: {context:?}'s mesh asset not loaded"));
    let material = primitive
        .material
        .clone()
        .unwrap_or_else(|| panic!("{model_path}: {context:?}'s primitive has no material"));
    RigPart {
        mesh: primitive.mesh.clone(),
        material,
        transform: node.transform,
        half_extents: mesh_half_extents(mesh_asset),
    }
}

/// Half the mesh's own POSITION bounding box — used to size a detached
/// part's debris collider (`damage::detach`) from whatever shape the artist
/// actually gave it.
fn mesh_half_extents(mesh: &Mesh) -> Vec3 {
    let positions = mesh
        .attribute(Mesh::ATTRIBUTE_POSITION)
        .and_then(|a| a.as_float3())
        .expect("mesh has no POSITION attribute");
    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(f32::MIN);
    for &[x, y, z] in positions {
        let p = Vec3::new(x, y, z);
        min = min.min(p);
        max = max.max(p);
    }
    (max - min) / 2.0
}
