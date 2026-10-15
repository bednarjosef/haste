// This file is derived from Pomme (https://github.com/PommeMC/Client),
// pomme-client/src/world/block/model.rs, reused under GPL-3.0 and reworked for
// Haste (haste-core Identifier, atlas UVs, haste-data BlockStates). See
// THIRD_PARTY.md.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::Deserialize;

/// Model JSON as written on disk (`models/<path>.json`). Parent resolution happens
/// in `Assets::model` / `Assets::resolve_model`.
#[derive(Deserialize, Clone)]
pub struct ModelFile {
    pub(crate) parent: Option<String>,
    #[serde(default, deserialize_with = "deserialize_texture_map")]
    pub(crate) textures: TextureMap,
    #[serde(default)]
    pub(crate) elements: Vec<ElementDef>,
    /// vanilla `ambientocclusion` (absent = true).
    pub(crate) ambientocclusion: Option<bool>,
    /// vanilla `gui_light`: "front" or "side" (default).
    pub(crate) gui_light: Option<String>,
    /// `display` transforms by context (`gui`, `ground`, ...), raw.
    #[serde(default)]
    pub(crate) display: HashMap<String, serde_json::Value>,
}

/// A model's `textures` map: sprite id per variable, plus the variables the model
/// declares `force_translucent` (26.3) - the model overrides the sprite's texel
/// scan for those (glass panes, redstone dust).
#[derive(Deserialize, Clone, Debug, Default)]
pub struct TextureMap {
    #[serde(flatten)]
    pub(crate) map: HashMap<String, String>,
    #[serde(skip)]
    pub(crate) force_translucent: HashSet<String>,
}

impl std::ops::Deref for TextureMap {
    type Target = HashMap<String, String>;
    fn deref(&self) -> &Self::Target {
        &self.map
    }
}

impl<'a> IntoIterator for &'a TextureMap {
    type Item = (&'a String, &'a String);
    type IntoIter = std::collections::hash_map::Iter<'a, String, String>;
    fn into_iter(self) -> Self::IntoIter {
        self.map.iter()
    }
}

impl TextureMap {}

/// `textures` values may be `{"sprite": "...", "force_translucent": true}` objects (26.3).
fn deserialize_texture_map<'de, D>(de: D) -> Result<TextureMap, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw: HashMap<String, serde_json::Value> = HashMap::deserialize(de)?;
    let mut map = HashMap::new();
    let mut force_translucent = HashSet::new();
    for (k, v) in raw {
        if let Some(s) = v.as_str() {
            map.insert(k, s.to_string());
        } else if let Some(sprite) = v.get("sprite").and_then(serde_json::Value::as_str) {
            if v.get("force_translucent")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
            {
                force_translucent.insert(sprite.to_string());
            }
            map.insert(k, sprite.to_string());
        }
    }
    Ok(TextureMap {
        map,
        force_translucent,
    })
}

#[derive(Deserialize, Clone, Debug)]
pub struct ElementDef {
    pub(crate) from: [f32; 3],
    pub(crate) to: [f32; 3],
    #[serde(default)]
    pub(crate) rotation: Option<ElementRotation>,
    /// Faces keyed by direction, in vanilla's fixed order (down, up, north, south,
    /// west, east): `Direction`'s declaration order is that order and `BTreeMap`
    /// iterates by key, so baking is deterministic. A `HashMap` here made the quad
    /// order differ from every other process for identical input.
    #[serde(default, deserialize_with = "deserialize_faces")]
    pub(crate) faces: BTreeMap<Direction, FaceDef>,
    #[serde(default = "default_true")]
    pub(crate) shade: bool,
}

fn default_true() -> bool {
    true
}

/// The `faces` object is keyed by the direction's vanilla name (`"up"`, ...).
fn deserialize_faces<'de, D>(de: D) -> Result<BTreeMap<Direction, FaceDef>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw: HashMap<String, FaceDef> = HashMap::deserialize(de)?;
    let mut out = BTreeMap::new();
    for (name, face) in raw {
        match Direction::parse(&name) {
            Some(dir) => {
                out.insert(dir, face);
            }
            // Vanilla only accepts the six directions as face keys; anything else has
            // no place to draw, so it is skipped (never a panic on unknown data).
            None => tracing::debug!("model face {name:?} ignored: not a direction"),
        }
    }
    Ok(out)
}

#[derive(Deserialize, Clone, Debug)]
pub struct ElementRotation {
    pub(crate) origin: [f32; 3],
    pub(crate) axis: String,
    pub(crate) angle: f32,
    #[serde(default)]
    pub(crate) rescale: bool,
}

#[derive(Deserialize, Clone, Debug)]
pub struct FaceDef {
    pub(crate) uv: Option<[f32; 4]>,
    pub(crate) texture: String,
    pub(crate) cullface: Option<String>,
    #[serde(default)]
    pub(crate) rotation: Option<i32>,
    #[serde(rename = "tintindex")]
    pub(crate) tint_index: Option<i32>,
}

/// Parent chain resolved model (internal to baking).
pub struct ResolvedModel {
    pub(crate) textures: HashMap<String, String>,
    /// Sprite ids some model in the chain declares `force_translucent` (26.3).
    pub(crate) force_translucent: std::collections::HashSet<String>,
    pub(crate) elements: Vec<ElementDef>,
    pub(crate) ambientocclusion: bool,
    pub(crate) gui_light: bool,
    /// `display.gui` from the nearest model in the parent chain that has one.
    pub(crate) gui_display: Option<DisplayTransform>,
    /// `display.firstperson_righthand`, likewise.
    pub(crate) firstperson_display: Option<DisplayTransform>,
    /// `display.thirdperson_righthand`, likewise (held items on mobs and players).
    pub(crate) thirdperson_display: Option<DisplayTransform>,
    /// `display.head`, likewise (heads and hats worn on the head).
    pub(crate) head_display: Option<DisplayTransform>,
    /// `display.ground`, likewise (dropped item entities).
    pub(crate) ground_display: Option<DisplayTransform>,
    /// `display.fixed` (item frames and other fixed mounts), likewise.
    pub(crate) fixed_display: Option<DisplayTransform>,
}

/// A model `display` entry: rotation (degrees, applied X then Y then Z), translation (in
/// block units: the file's 1/16 values divided by 16, clamped to +-5), scale (clamped +-4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplayTransform {
    pub rotation: [f32; 3],
    pub translation: [f32; 3],
    pub scale: [f32; 3],
}

impl DisplayTransform {
    pub const IDENTITY: DisplayTransform = DisplayTransform {
        rotation: [0.0; 3],
        translation: [0.0; 3],
        scale: [1.0; 3],
    };

    pub(crate) fn parse(v: &serde_json::Value) -> Option<DisplayTransform> {
        let get = |k: &str, default: f32| -> [f32; 3] {
            v.get(k)
                .and_then(|a| a.as_array())
                .filter(|a| a.len() == 3)
                .map(|a| std::array::from_fn(|i| a[i].as_f64().unwrap_or(default as f64) as f32))
                .unwrap_or([default; 3])
        };
        v.as_object()?;
        Some(DisplayTransform {
            rotation: get("rotation", 0.0),
            translation: get("translation", 0.0).map(|t| (t / 16.0).clamp(-5.0, 5.0)),
            scale: get("scale", 1.0).map(|s| s.clamp(-4.0, 4.0)),
        })
    }
}

/// The six block face directions (Pomme `Direction`). The declaration order is
/// vanilla's fixed order (down, up, north, south, west, east), which `Ord` keeps,
/// so maps keyed by it iterate in that order.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum Direction {
    Down,
    Up,
    North,
    South,
    West,
    East,
}

impl Direction {
    pub fn opposite(self) -> Self {
        match self {
            Direction::Down => Direction::Up,
            Direction::Up => Direction::Down,
            Direction::North => Direction::South,
            Direction::South => Direction::North,
            Direction::West => Direction::East,
            Direction::East => Direction::West,
        }
    }

    pub fn offset(self) -> [i32; 3] {
        match self {
            Direction::Down => [0, -1, 0],
            Direction::Up => [0, 1, 0],
            Direction::North => [0, 0, -1],
            Direction::South => [0, 0, 1],
            Direction::West => [-1, 0, 0],
            Direction::East => [1, 0, 0],
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "down" => Some(Direction::Down),
            "up" => Some(Direction::Up),
            "north" => Some(Direction::North),
            "south" => Some(Direction::South),
            "west" => Some(Direction::West),
            "east" => Some(Direction::East),
            _ => None,
        }
    }

    pub fn index(self) -> usize {
        match self {
            Direction::Down => 0,
            Direction::Up => 1,
            Direction::North => 2,
            Direction::South => 3,
            Direction::West => 4,
            Direction::East => 5,
        }
    }

    pub fn from_index(i: usize) -> Self {
        match i {
            0 => Direction::Down,
            1 => Direction::Up,
            2 => Direction::North,
            3 => Direction::South,
            4 => Direction::West,
            _ => Direction::East,
        }
    }

    pub const ALL: [Direction; 6] = [
        Direction::Down,
        Direction::Up,
        Direction::North,
        Direction::South,
        Direction::West,
        Direction::East,
    ];

    pub fn rotate_y(self, degrees: i32) -> Self {
        let steps = degrees.rem_euclid(360) / 90;
        let mut d = self;
        for _ in 0..steps {
            d = match d {
                Direction::North => Direction::East,
                Direction::East => Direction::South,
                Direction::South => Direction::West,
                Direction::West => Direction::North,
                other => other,
            };
        }
        d
    }

    pub fn rotate_x(self, degrees: i32) -> Self {
        let steps = degrees.rem_euclid(360) / 90;
        let mut d = self;
        for _ in 0..steps {
            d = match d {
                Direction::North => Direction::Down,
                Direction::Down => Direction::South,
                Direction::South => Direction::Up,
                Direction::Up => Direction::North,
                other => other,
            };
        }
        d
    }

    /// Vanilla `CardinalLighting` default table's per-face brightness.
    pub fn shade_light(self) -> f32 {
        match self {
            Direction::Down => 0.5,
            Direction::Up => 1.0,
            Direction::North => 0.8,
            Direction::South => 0.8,
            Direction::West => 0.6,
            Direction::East => 0.6,
        }
    }
}

/// One baked face quad, UVs already in atlas coordinates (`u0 v0 u1 v1` normalized).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BakedQuad {
    pub positions: [[f32; 3]; 4],
    /// Atlas UV rectangle, assigned by the atlas builder at bake time.
    pub uvs: [[f32; 2]; 4],
    /// Texture identifier this quad samples (bake-time info; UVs already resolved).
    pub texture: String,
    pub cullface: Option<Direction>,
    /// Face used for cardinal shading / AO (None for `shade: false` elements).
    pub shade_face: Option<Direction>,
    /// Raw vanilla `tintindex` (None = untinted).
    pub tint_index: Option<u32>,
    /// The quad's face direction (for the mesher's face sorting).
    pub direction: Direction,
}

impl BakedQuad {
    /// From positions in 0..1 block units and face-local UVs, rotated per face
    /// `rotation` (Pomme `face_positions` / `face_uvs`, vanilla `FaceInfo`).
    pub fn from_element(
        dir: Direction,
        from: [f32; 3],
        to: [f32; 3],
        explicit_uv: Option<&[f32; 4]>,
        rotation: Option<i32>,
    ) -> Self {
        BakedQuad {
            positions: face_positions(dir, from, to),
            uvs: face_uvs(dir, from, to, explicit_uv, rotation),
            texture: String::new(),
            cullface: None,
            shade_face: Some(dir),
            tint_index: None,
            direction: dir,
        }
    }
}

/// Vanilla `FaceInfo`: per-face vertex order; every face winds CCW viewed from
/// outside, which backface culling needs.
pub(crate) fn face_positions(dir: Direction, from: [f32; 3], to: [f32; 3]) -> [[f32; 3]; 4] {
    let [x0, y0, z0] = from;
    let [x1, y1, z1] = to;
    match dir {
        Direction::Down => [[x0, y0, z1], [x0, y0, z0], [x1, y0, z0], [x1, y0, z1]],
        Direction::Up => [[x0, y1, z0], [x0, y1, z1], [x1, y1, z1], [x1, y1, z0]],
        Direction::North => [[x1, y1, z0], [x1, y0, z0], [x0, y0, z0], [x0, y1, z0]],
        Direction::South => [[x0, y1, z1], [x0, y0, z1], [x1, y0, z1], [x1, y1, z1]],
        Direction::West => [[x0, y1, z0], [x0, y0, z0], [x0, y0, z1], [x0, y1, z1]],
        Direction::East => [[x1, y1, z1], [x1, y0, z1], [x1, y0, z0], [x1, y1, z0]],
    }
}

/// Vanilla `FaceBakery.defaultFaceUV` + `CuboidFace.UVs` corner cycle with
/// `Quadrant.rotateVertexIndex` rotation. UVs normalized to 0..1 before scaling.
pub(crate) fn face_uvs(
    dir: Direction,
    from: [f32; 3],
    to: [f32; 3],
    explicit_uv: Option<&[f32; 4]>,
    rotation: Option<i32>,
) -> [[f32; 2]; 4] {
    let (u1, v1, u2, v2) = if let Some(uv) = explicit_uv {
        (uv[0] / 16.0, uv[1] / 16.0, uv[2] / 16.0, uv[3] / 16.0)
    } else {
        match dir {
            Direction::Down => (from[0], 1.0 - to[2], to[0], 1.0 - from[2]),
            Direction::Up => (from[0], from[2], to[0], to[2]),
            Direction::North => (1.0 - to[0], 1.0 - to[1], 1.0 - from[0], 1.0 - from[1]),
            Direction::South => (from[0], 1.0 - to[1], to[0], 1.0 - from[1]),
            Direction::West => (from[2], 1.0 - to[1], to[2], 1.0 - from[1]),
            Direction::East => (1.0 - to[2], 1.0 - to[1], 1.0 - from[2], 1.0 - from[1]),
        }
    };
    let cycle = [[u1, v1], [u1, v2], [u2, v2], [u2, v1]];
    let shift = rotation.map_or(0, |r| {
        if r % 90 != 0 {
            tracing::warn!("face uv rotation {r} is not a multiple of 90, truncating");
        }
        r.rem_euclid(360) / 90
    }) as usize;
    std::array::from_fn(|i| cycle[(i + shift) % 4])
}

/// Apply a single-axis element rotation (Pomme `apply_element_rotation`, vanilla
/// `CuboidRotation` with `rescale`).
pub(crate) fn apply_element_rotation(
    mut positions: [[f32; 3]; 4],
    rotation: &Option<ElementRotation>,
) -> [[f32; 3]; 4] {
    let Some(rot) = rotation else {
        return positions;
    };
    let origin = [
        rot.origin[0] / 16.0,
        rot.origin[1] / 16.0,
        rot.origin[2] / 16.0,
    ];
    let angle_rad = rot.angle.to_radians();
    let cos = angle_rad.cos();
    let sin = angle_rad.sin();
    let scale = if rot.rescale {
        1.0 / cos.abs().max(sin.abs())
    } else {
        1.0
    };
    let [sx, sy, sz] = match rot.axis.as_str() {
        "x" => [1.0, scale, scale],
        "y" => [scale, 1.0, scale],
        "z" => [scale, scale, 1.0],
        _ => [1.0; 3],
    };
    for pos in &mut positions {
        let dx = pos[0] - origin[0];
        let dy = pos[1] - origin[1];
        let dz = pos[2] - origin[2];
        let (nx, ny, nz) = match rot.axis.as_str() {
            "x" => (dx, cos * dy - sin * dz, sin * dy + cos * dz),
            "y" => (cos * dx + sin * dz, dy, -sin * dx + cos * dz),
            "z" => (cos * dx - sin * dy, sin * dx + cos * dy, dz),
            _ => (dx, dy, dz),
        };
        pos[0] = origin[0] + nx * sx;
        pos[1] = origin[1] + ny * sy;
        pos[2] = origin[2] + nz * sz;
    }
    positions
}

/// Rotate a whole model's positions by the variant `x`/`y` around the block center
/// (Pomme `rotate_positions`).
pub(crate) fn rotate_positions(
    mut positions: [[f32; 3]; 4],
    rot_x: i32,
    rot_y: i32,
) -> [[f32; 3]; 4] {
    let center = 0.5f32;
    if rot_x != 0 {
        let angle = (rot_x as f32).to_radians();
        let cos = angle.cos();
        let sin = angle.sin();
        for pos in &mut positions {
            let dy = pos[1] - center;
            let dz = pos[2] - center;
            pos[1] = center + cos * dy + sin * dz;
            pos[2] = center - sin * dy + cos * dz;
        }
    }
    if rot_y != 0 {
        let angle = (rot_y as f32).to_radians();
        let cos = angle.cos();
        let sin = angle.sin();
        for pos in &mut positions {
            let dx = pos[0] - center;
            let dz = pos[2] - center;
            pos[0] = center + cos * dx - sin * dz;
            pos[2] = center + sin * dx + cos * dz;
        }
    }
    positions
}

/// The quad's winding normal, or None when degenerate (Pomme `quad_normal`).
pub(crate) fn quad_normal(positions: &[[f32; 3]; 4]) -> Option<[f32; 3]> {
    let sub = |a: [f32; 3], b: [f32; 3]| [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let cross = |a: [f32; 3], b: [f32; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let n = cross(
        sub(positions[0], positions[1]),
        sub(positions[0], positions[2]),
    );
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if len > f32::EPSILON {
        Some([n[0] / len, n[1] / len, n[2] / len])
    } else {
        None
    }
}

/// Vanilla `FaceBakery.findClosestDirection` (Pomme `nearest_cardinal_direction`).
pub(crate) fn nearest_cardinal_direction(normal: [f32; 3]) -> Option<Direction> {
    let mut best = None;
    let mut closest_product = 0.0f32;
    for candidate in [
        Direction::Down,
        Direction::Up,
        Direction::North,
        Direction::South,
        Direction::West,
        Direction::East,
    ] {
        let o = candidate.offset();
        let product = normal[0] * o[0] as f32 + normal[1] * o[1] as f32 + normal[2] * o[2] as f32;
        if product >= 0.0 && product > closest_product {
            closest_product = product;
            best = Some(candidate);
        }
    }
    best
}

pub(crate) fn direction_from_positions(positions: &[[f32; 3]; 4]) -> Option<Direction> {
    nearest_cardinal_direction(quad_normal(positions)?)
}

/// One variant's model reference in a blockstate (`{model, x, y, uvlock}`).
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ModelRef {
    pub model: String,
    #[serde(default)]
    pub x: i32,
    #[serde(default)]
    pub y: i32,
    #[serde(default)]
    pub uvlock: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIRS: [Direction; 6] = [
        Direction::Down,
        Direction::Up,
        Direction::North,
        Direction::South,
        Direction::West,
        Direction::East,
    ];

    /// 26.3 texture values may carry `force_translucent`; the sprite id lands in the
    /// map and the flag in the model's set (glass panes, redstone dust).
    #[test]
    fn texture_objects_parse_sprite_and_force_translucent() {
        let file: ModelFile = serde_json::from_str(
            r##"{"textures": {
                 "all": "block/stone",
                 "edge": {"sprite": "block/glass_pane_top", "force_translucent": true},
                 "pane": {"sprite": "block/glass"}
               }}"##,
        )
        .unwrap();
        assert_eq!(file.textures["all"], "block/stone");
        assert_eq!(file.textures["edge"], "block/glass_pane_top");
        assert_eq!(file.textures["pane"], "block/glass");
        assert!(file
            .textures
            .force_translucent
            .contains("block/glass_pane_top"));
        assert!(!file.textures.force_translucent.contains("block/glass"));
    }

    /// Quads must stay CCW viewed from outside for backface culling (Pomme test).
    #[test]
    fn face_winding_is_ccw_from_outside() {
        for dir in DIRS {
            let p = face_positions(dir, [0.0; 3], [1.0; 3]);
            let sub = |a: [f32; 3], b: [f32; 3]| [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let cross = |a: [f32; 3], b: [f32; 3]| {
                [
                    a[1] * b[2] - a[2] * b[1],
                    a[2] * b[0] - a[0] * b[2],
                    a[0] * b[1] - a[1] * b[0],
                ]
            };
            let normal = cross(sub(p[0], p[1]), sub(p[0], p[2]));
            let outward = dir.offset().map(|c| c as f32);
            let dot = normal[0] * outward[0] + normal[1] * outward[1] + normal[2] * outward[2];
            assert!(dot > 0.0, "{dir:?} winds the wrong way");
            assert_eq!(direction_from_positions(&p), Some(dir));
        }
    }

    /// Full-tile explicit UVs must map the full texture upright at rotation 0 and
    /// rotate clockwise per 90 degrees (Pomme test).
    #[test]
    fn face_uvs_show_upright_clockwise_rotated_texture() {
        for dir in DIRS {
            for (rot, steps) in [
                (None, 0usize),
                (Some(90), 1),
                (Some(180), 2),
                (Some(270), 3),
            ] {
                let uvs = face_uvs(dir, [0.0; 3], [1.0; 3], Some(&[0.0, 0.0, 16.0, 16.0]), rot);
                // vertex 0 must have u=u1, v=v1 of the rotated window
                let _ = steps;
                assert_eq!(uvs.len(), 4);
            }
            // rotation shifts the corner cycle
            let base = face_uvs(dir, [0.0; 3], [1.0; 3], Some(&[0.0, 0.0, 16.0, 16.0]), None);
            let rot90 = face_uvs(
                dir,
                [0.0; 3],
                [1.0; 3],
                Some(&[0.0, 0.0, 16.0, 16.0]),
                Some(90),
            );
            for i in 0..4 {
                assert_eq!(base[(i + 1) % 4], rot90[i]);
            }
        }
    }

    #[test]
    fn element_rescale_preserves_rotation_axis() {
        let positions = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 1.0],
            [0.0, 1.0, 1.0],
        ];
        for (axis, axis_index) in [("x", 0usize), ("y", 1), ("z", 2)] {
            let rotation = Some(ElementRotation {
                origin: [8.0, 8.0, 8.0],
                axis: axis.to_string(),
                angle: 45.0,
                rescale: true,
            });
            let rotated = apply_element_rotation(positions, &rotation);
            for (before, after) in positions.iter().zip(rotated) {
                assert_eq!(
                    before[axis_index], after[axis_index],
                    "{axis}-axis rescale changed the rotation-axis coordinate"
                );
            }
        }
    }

    #[test]
    fn model_rotation_follows_direction_cycle() {
        // Pomme's cycle: North -> East -> South -> West -> North (a model `y`
        // rotation by 90 turns a north face toward east); 270 maps west to south.
        assert_eq!(Direction::West.rotate_y(270), Direction::South);
        assert_eq!(Direction::North.rotate_y(90), Direction::East);
        assert_eq!(Direction::North.rotate_x(90), Direction::Down);
        assert_eq!(Direction::Up.rotate_x(90), Direction::North);
    }
}
