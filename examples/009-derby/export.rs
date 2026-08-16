//! Writes a car class's procedural meshes and materials out as a standalone
//! glTF binary (`assets/models/derby-<class>.glb` by default) — the
//! artist-editable source of truth `model.rs` loads back at runtime. Every
//! part's shape and rest pose comes straight from the class's `CarSpec`
//! (`config.rs`), so this stays in lockstep with the procedural car for as
//! long as both exist.
//!
//! Not wired into the running game: invoked once per class via
//! `cargo run --example 009-derby -- export-model <truck|buggy> [path]
//! [--force]`, handled in `main.rs` before the `App` is even built (a
//! `Mesh`'s primitive builders don't need one). Re-running without `--force`
//! refuses to overwrite an existing file, so it can never clobber an
//! artist's edits by accident.
//!
//! Node naming is the contract `model.rs` loads against — see
//! `docs/car-model.md`. It's identical across classes: the same node set,
//! just different sizes and rest poses, which is what lets `model.rs`'s
//! parsing stay completely class-agnostic. Sides (`_L`/`_R`) are baked into
//! names only to keep them unique; the loader re-derives which side a part
//! is actually on from its node's own X translation, so a reshaped or moved
//! part still attributes damage correctly.

use std::collections::BTreeMap;
use std::f32::consts::FRAC_PI_2;
use std::io;
use std::path::Path;

use bevy::prelude::*;
use gltf_json as json;
use gltf_json::validation::Checked;

use crate::config::{CarSpec, HEADLIGHT_EMISSIVE};

/// Exports `spec`'s car to `path`. Refuses to overwrite an existing file
/// unless `force` is set.
pub fn export_car_model(spec: &CarSpec, path: &Path, force: bool) -> io::Result<()> {
    if path.exists() && !force {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} already exists — pass --force to overwrite", path.display()),
        ));
    }

    let mut root = json::Root {
        asset: json::Asset {
            generator: Some("009-derby export-model".to_string()),
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bin = Vec::new();
    let buffer_index = root.push(json::Buffer {
        byte_length: 0u64.into(),
        name: None,
        uri: None,
        extensions: None,
        extras: Default::default(),
    });

    let materials = push_materials(&mut root);
    let mut scene_nodes = Vec::new();

    // --- Chassis -----------------------------------------------------
    let chassis_mesh_data: Mesh = Cuboid::new(spec.chassis_size.x, spec.chassis_size.y, spec.chassis_size.z).into();
    let chassis_mesh = push_mesh(&mut root, &mut bin, buffer_index, &chassis_mesh_data, materials.chassis_paint, "Chassis");
    scene_nodes.push(push_node(&mut root, "Chassis", chassis_mesh, Vec3::ZERO, Quat::IDENTITY, None));

    // --- Wheels + spin stripes -----------------------------------------
    // Baked exactly as `vehicle.rs::spawn_car` builds them: the cylinder's
    // axis is Y, rotated onto chassis-local X (left/right) by
    // `wheel_axis_align`; the stripe sits in the wheel's own (pre-rotation)
    // frame so it spins as one rigid unit with the wheel.
    let wheel_axis_align = Quat::from_rotation_z(FRAC_PI_2);
    let wheel_mesh_data: Mesh = Cylinder::new(spec.wheel_radius, spec.wheel_width).into();
    let wheel_mesh = push_mesh(&mut root, &mut bin, buffer_index, &wheel_mesh_data, materials.wheel, "WheelMesh");
    let stripe_mesh_data: Mesh = Cuboid::new(0.06, spec.wheel_width + 0.02, spec.wheel_radius * 0.85).into();
    let stripe_mesh = push_mesh(&mut root, &mut bin, buffer_index, &stripe_mesh_data, materials.stripe, "StripeMesh");
    const WHEEL_NAMES: [&str; 4] = ["Wheel_FL", "Wheel_FR", "Wheel_RL", "Wheel_RR"];
    for (&name, &mount) in WHEEL_NAMES.iter().zip(spec.wheel_mounts.iter()) {
        let stripe_node = push_node(
            &mut root,
            "Stripe",
            stripe_mesh,
            Vec3::new(0.0, 0.0, spec.wheel_radius * 0.5),
            Quat::IDENTITY,
            None,
        );
        let wheel_node = push_node(&mut root, name, wheel_mesh, mount, wheel_axis_align, Some(vec![stripe_node]));
        scene_nodes.push(wheel_node);
    }

    // --- Sided destructible parts ---------------------------------------
    // (name, sign) pairs: sign -1.0 = −X = driver's right ("_R"), +1.0 = +X
    // = driver's left ("_L") — matches `damage::side_index`'s convention.
    let headlight_mesh_data: Mesh = Cuboid::new(spec.headlight_size.x, spec.headlight_size.y, spec.headlight_size.z).into();
    let headlight_mesh = push_mesh(&mut root, &mut bin, buffer_index, &headlight_mesh_data, materials.headlight, "HeadlightMesh");
    for (name, sign) in [("Headlight_R", -1.0), ("Headlight_L", 1.0)] {
        let offset = spec.headlight_offset.with_x(spec.headlight_offset.x * sign);
        scene_nodes.push(push_node(&mut root, name, headlight_mesh, offset, Quat::IDENTITY, None));
    }

    let fender_mesh_data: Mesh = Cuboid::new(spec.fender_size.x, spec.fender_size.y, spec.fender_size.z).into();
    let fender_mesh = push_mesh(&mut root, &mut bin, buffer_index, &fender_mesh_data, materials.chassis_paint, "FenderMesh");
    for (name, sign) in [("Fender_R", -1.0), ("Fender_L", 1.0)] {
        let offset = spec.fender_offset.with_x(spec.fender_offset.x * sign);
        scene_nodes.push(push_node(&mut root, name, fender_mesh, offset, Quat::IDENTITY, None));
    }

    let spoiler_strut_mesh_data: Mesh =
        Cuboid::new(spec.spoiler_strut_size.x, spec.spoiler_strut_size.y, spec.spoiler_strut_size.z).into();
    let spoiler_strut_mesh = push_mesh(&mut root, &mut bin, buffer_index, &spoiler_strut_mesh_data, materials.trim, "SpoilerStrutMesh");
    for (name, sign) in [("SpoilerStrut_R", -1.0), ("SpoilerStrut_L", 1.0)] {
        let offset = spec.spoiler_strut_offset.with_x(spec.spoiler_strut_offset.x * sign);
        scene_nodes.push(push_node(&mut root, name, spoiler_strut_mesh, offset, Quat::IDENTITY, None));
    }

    let shield_strut_mesh_data: Mesh =
        Cuboid::new(spec.shield_strut_size.x, spec.shield_strut_size.y, spec.shield_strut_size.z).into();
    let shield_strut_mesh = push_mesh(&mut root, &mut bin, buffer_index, &shield_strut_mesh_data, materials.trim, "ShieldStrutMesh");
    for (name, sign) in [("ShieldStrut_R", -1.0), ("ShieldStrut_L", 1.0)] {
        let offset = spec.shield_strut_offset.with_x(spec.shield_strut_offset.x * sign);
        scene_nodes.push(push_node(&mut root, name, shield_strut_mesh, offset, Quat::IDENTITY, None));
    }

    // --- Unsided parts ---------------------------------------------------
    let windshield_mesh_data: Mesh = Cuboid::new(spec.windshield_size.x, spec.windshield_size.y, spec.windshield_size.z).into();
    let windshield_mesh = push_mesh(&mut root, &mut bin, buffer_index, &windshield_mesh_data, materials.glass, "Windshield");
    scene_nodes.push(push_node(
        &mut root,
        "Windshield",
        windshield_mesh,
        spec.windshield_offset,
        Quat::from_rotation_x(spec.windshield_pitch),
        None,
    ));

    let spoiler_mesh_data: Mesh = Cuboid::new(spec.spoiler_size.x, spec.spoiler_size.y, spec.spoiler_size.z).into();
    let spoiler_mesh = push_mesh(&mut root, &mut bin, buffer_index, &spoiler_mesh_data, materials.chassis_paint, "Spoiler");
    scene_nodes.push(push_node(&mut root, "Spoiler", spoiler_mesh, spec.spoiler_offset, Quat::IDENTITY, None));

    let shield_mesh_data: Mesh = Cuboid::new(spec.shield_size.x, spec.shield_size.y, spec.shield_size.z).into();
    let shield_mesh = push_mesh(&mut root, &mut bin, buffer_index, &shield_mesh_data, materials.trim, "RamBar");
    scene_nodes.push(push_node(&mut root, "RamBar", shield_mesh, spec.shield_offset, Quat::IDENTITY, None));

    let scene = root.push(json::Scene {
        extensions: None,
        extras: Default::default(),
        name: Some("DerbyCar".to_string()),
        nodes: scene_nodes,
    });
    root.scene = Some(scene);
    root.buffers[0].byte_length = (bin.len() as u64).into();

    let json_bytes = root.to_vec().expect("gltf-json serialization");
    let node_count = root.nodes.len();
    let mesh_count = root.meshes.len();
    let material_count = root.materials.len();
    let glb = build_glb(json_bytes, bin);

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, &glb)?;
    println!(
        "wrote {} ({node_count} nodes, {mesh_count} meshes, {material_count} materials, {} bytes)",
        path.display(),
        glb.len()
    );
    Ok(())
}

/// Shared material template handles, referenced by every part that wears
/// that look. `model.rs` clones `chassis_paint` per car (overriding just its
/// base color) and `headlight` per headlight (each dims independently); the
/// rest are shared as-is.
struct MaterialSet {
    chassis_paint: json::Index<json::Material>,
    wheel: json::Index<json::Material>,
    stripe: json::Index<json::Material>,
    glass: json::Index<json::Material>,
    trim: json::Index<json::Material>,
    headlight: json::Index<json::Material>,
}

/// Mirrors `vehicle.rs::spawn_vehicle`'s material block. `chassis_paint`
/// exports as the player's blue — `model.rs` overrides the color per car.
fn push_materials(root: &mut json::Root) -> MaterialSet {
    MaterialSet {
        chassis_paint: push_material(root, "ChassisPaint", Color::srgb(0.15, 0.4, 0.75), 0.5, 0.0, false, None),
        wheel: push_material(root, "Wheel", Color::srgb(0.08, 0.08, 0.08), 0.9, 0.0, false, None),
        stripe: push_material(root, "Stripe", Color::srgb(0.9, 0.75, 0.1), 0.7, 0.0, false, None),
        glass: push_material(root, "Glass", Color::srgba(0.55, 0.75, 0.9, 0.35), 0.1, 0.0, true, None),
        trim: push_material(root, "Trim", Color::srgb(0.12, 0.12, 0.12), 0.8, 0.0, false, None),
        headlight: push_material(root, "Headlight", Color::srgb(1.0, 0.95, 0.8), 0.5, 0.0, false, Some(HEADLIGHT_EMISSIVE)),
    }
}

/// Builds one glTF material. `emissive` (if any) is Bevy's HDR
/// `LinearRgba`, whose components can exceed 1.0 (`HEADLIGHT_EMISSIVE` peaks
/// at 2.5) — glTF's plain `emissiveFactor` is clamped to [0, 1], so the
/// color is normalized to its peak channel and the peak itself is carried
/// separately via `KHR_materials_emissive_strength`, an optional extension
/// (viewers that don't support it still show the un-intensified glow).
fn push_material(
    root: &mut json::Root,
    name: &str,
    color: Color,
    roughness: f32,
    metallic: f32,
    blend: bool,
    emissive: Option<LinearRgba>,
) -> json::Index<json::Material> {
    let base = color.to_linear();
    let mut material = json::Material {
        name: Some(name.to_string()),
        pbr_metallic_roughness: json::material::PbrMetallicRoughness {
            base_color_factor: json::material::PbrBaseColorFactor([base.red, base.green, base.blue, base.alpha]),
            metallic_factor: json::material::StrengthFactor(metallic),
            roughness_factor: json::material::StrengthFactor(roughness),
            ..Default::default()
        },
        alpha_mode: Checked::Valid(if blend {
            json::material::AlphaMode::Blend
        } else {
            json::material::AlphaMode::Opaque
        }),
        double_sided: blend,
        ..Default::default()
    };
    if let Some(emissive) = emissive {
        let peak = emissive.red.max(emissive.green).max(emissive.blue).max(1.0);
        material.emissive_factor =
            json::material::EmissiveFactor([emissive.red / peak, emissive.green / peak, emissive.blue / peak]);
        material.extensions = Some(json::extensions::material::Material {
            emissive_strength: Some(json::extensions::material::EmissiveStrength {
                emissive_strength: json::extensions::material::EmissiveStrengthFactor(peak),
            }),
            ..Default::default()
        });
        if !root.extensions_used.iter().any(|ext| ext == "KHR_materials_emissive_strength") {
            root.extensions_used.push("KHR_materials_emissive_strength".to_string());
        }
    }
    root.push(material)
}

/// Encodes one `Mesh`'s POSITION/NORMAL/index data into the shared binary
/// buffer and registers the accessors, buffer views and glTF mesh for it.
/// No UVs: nothing here samples a texture, so they'd only be dead weight.
fn push_mesh(
    root: &mut json::Root,
    bin: &mut Vec<u8>,
    buffer_index: json::Index<json::Buffer>,
    mesh: &Mesh,
    material: json::Index<json::Material>,
    name: &str,
) -> json::Index<json::Mesh> {
    let positions = mesh
        .attribute(Mesh::ATTRIBUTE_POSITION)
        .and_then(|a| a.as_float3())
        .unwrap_or_else(|| panic!("{name}: mesh has no POSITION attribute"));
    let normals = mesh
        .attribute(Mesh::ATTRIBUTE_NORMAL)
        .and_then(|a| a.as_float3())
        .unwrap_or_else(|| panic!("{name}: mesh has no NORMAL attribute"));
    let indices: Vec<u32> = mesh
        .indices()
        .unwrap_or_else(|| panic!("{name}: mesh has no indices"))
        .iter()
        .map(|i| i as u32)
        .collect();

    let (min, max) = position_bounds(positions);

    let pos_offset = push_f32x3(bin, positions);
    let pos_view = push_buffer_view(root, buffer_index, pos_offset, positions.len() * 12, json::buffer::Target::ArrayBuffer);
    let pos_accessor = push_accessor(
        root,
        pos_view,
        positions.len(),
        json::accessor::ComponentType::F32,
        json::accessor::Type::Vec3,
        Some(serde_json::json!(min)),
        Some(serde_json::json!(max)),
    );

    let norm_offset = push_f32x3(bin, normals);
    let norm_view = push_buffer_view(root, buffer_index, norm_offset, normals.len() * 12, json::buffer::Target::ArrayBuffer);
    let norm_accessor = push_accessor(root, norm_view, normals.len(), json::accessor::ComponentType::F32, json::accessor::Type::Vec3, None, None);

    let idx_offset = push_u32(bin, &indices);
    let idx_view = push_buffer_view(root, buffer_index, idx_offset, indices.len() * 4, json::buffer::Target::ElementArrayBuffer);
    let idx_accessor = push_accessor(root, idx_view, indices.len(), json::accessor::ComponentType::U32, json::accessor::Type::Scalar, None, None);

    let mut attributes = BTreeMap::new();
    attributes.insert(Checked::Valid(json::mesh::Semantic::Positions), pos_accessor);
    attributes.insert(Checked::Valid(json::mesh::Semantic::Normals), norm_accessor);

    root.push(json::Mesh {
        extensions: None,
        extras: Default::default(),
        name: Some(name.to_string()),
        primitives: vec![json::mesh::Primitive {
            attributes,
            extensions: None,
            extras: Default::default(),
            indices: Some(idx_accessor),
            material: Some(material),
            mode: Checked::Valid(json::mesh::Mode::Triangles),
            targets: None,
        }],
        weights: None,
    })
}

fn push_buffer_view(
    root: &mut json::Root,
    buffer_index: json::Index<json::Buffer>,
    byte_offset: usize,
    byte_length: usize,
    target: json::buffer::Target,
) -> json::Index<json::buffer::View> {
    root.push(json::buffer::View {
        buffer: buffer_index,
        byte_length: byte_length.into(),
        byte_offset: Some(byte_offset.into()),
        byte_stride: None,
        name: None,
        target: Some(Checked::Valid(target)),
        extensions: None,
        extras: Default::default(),
    })
}

#[allow(clippy::too_many_arguments)]
fn push_accessor(
    root: &mut json::Root,
    view: json::Index<json::buffer::View>,
    count: usize,
    component_type: json::accessor::ComponentType,
    type_: json::accessor::Type,
    min: Option<serde_json::Value>,
    max: Option<serde_json::Value>,
) -> json::Index<json::Accessor> {
    root.push(json::Accessor {
        buffer_view: Some(view),
        byte_offset: None,
        count: (count as u64).into(),
        component_type: Checked::Valid(json::accessor::GenericComponentType(component_type)),
        extensions: None,
        extras: Default::default(),
        type_: Checked::Valid(type_),
        min,
        max,
        name: None,
        normalized: false,
        sparse: None,
    })
}

fn push_node(
    root: &mut json::Root,
    name: &str,
    mesh: json::Index<json::Mesh>,
    translation: Vec3,
    rotation: Quat,
    children: Option<Vec<json::Index<json::Node>>>,
) -> json::Index<json::Node> {
    root.push(json::Node {
        camera: None,
        children,
        extensions: None,
        extras: Default::default(),
        matrix: None,
        mesh: Some(mesh),
        name: Some(name.to_string()),
        rotation: Some(json::scene::UnitQuaternion([rotation.x, rotation.y, rotation.z, rotation.w])),
        scale: None,
        translation: Some([translation.x, translation.y, translation.z]),
        skin: None,
        weights: None,
    })
}

fn position_bounds(positions: &[[f32; 3]]) -> ([f32; 3], [f32; 3]) {
    let mut min = [f32::MAX; 3];
    let mut max = [f32::MIN; 3];
    for p in positions {
        for i in 0..3 {
            min[i] = min[i].min(p[i]);
            max[i] = max[i].max(p[i]);
        }
    }
    (min, max)
}

fn push_f32x3(bin: &mut Vec<u8>, data: &[[f32; 3]]) -> usize {
    let offset = bin.len();
    for v in data {
        for c in v {
            bin.extend_from_slice(&c.to_le_bytes());
        }
    }
    offset
}

fn push_u32(bin: &mut Vec<u8>, data: &[u32]) -> usize {
    let offset = bin.len();
    for i in data {
        bin.extend_from_slice(&i.to_le_bytes());
    }
    offset
}

/// Packs a JSON chunk and a BIN chunk into one `.glb` container (spec:
/// 12-byte header, then length-prefixed chunks — JSON space-padded, BIN
/// zero-padded — to 4-byte boundaries).
fn build_glb(mut json_bytes: Vec<u8>, mut bin_bytes: Vec<u8>) -> Vec<u8> {
    while json_bytes.len() % 4 != 0 {
        json_bytes.push(b' ');
    }
    while bin_bytes.len() % 4 != 0 {
        bin_bytes.push(0);
    }

    let total_len = 12 + 8 + json_bytes.len() + 8 + bin_bytes.len();
    let mut out = Vec::with_capacity(total_len);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total_len as u32).to_le_bytes());
    out.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json_bytes);
    out.extend_from_slice(&(bin_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(&[b'B', b'I', b'N', 0]);
    out.extend_from_slice(&bin_bytes);
    out
}
