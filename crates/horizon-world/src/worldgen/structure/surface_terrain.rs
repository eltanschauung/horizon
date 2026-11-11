//! Bounded, immutable surface plans, shared by every chunk referencing a start.
use super::{
    BoundingBox,
    jigsaw::Projection,
    start::{Piece, StartContext, StructureStart},
};
use crate::{
    heightmap::{HeightmapState, HeightmapType},
    registry::BlockStateId,
    worldgen::terrain::{TerrainBlockStates, TerrainChunk, TerrainError},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub enabled: bool,
    pub max_slope: f64,
    pub max_excavation: i32,
    pub max_fill: i32,
    pub radius_scale: f64,
    pub min_radius: i32,
    pub max_radius: i32,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            max_slope: 1.0,
            max_excavation: 8,
            max_fill: 8,
            radius_scale: 0.5,
            min_radius: 4,
            max_radius: 24,
        }
    }
}
impl Config {
    pub fn valid(&self) -> bool {
        self.max_slope.is_finite()
            && (0.0..=2.0).contains(&self.max_slope)
            && (0..=32).contains(&self.max_excavation)
            && (0..=32).contains(&self.max_fill)
            && self.radius_scale.is_finite()
            && (0.0..=2.0).contains(&self.radius_scale)
            && self.min_radius >= 1
            && self.min_radius <= self.max_radius
            && self.max_radius <= 32
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Patch {
    pub min: [i32; 2],
    pub max: [i32; 2],
    pub road: bool,
    #[serde(with = "exact_float")]
    pub floor: f64,
    #[serde(with = "exact_pair")]
    pub grade: [f64; 2],
    #[serde(with = "exact_float")]
    pub radius: f64,
}
impl Patch {
    fn target(&self, x: i32, z: i32) -> f64 {
        self.floor
            + self.grade[0] * f64::from(x - self.min[0])
            + self.grade[1] * f64::from(z - self.min[1])
    }
    fn weight(&self, x: i32, z: i32) -> f64 {
        let dx = (self.min[0] - x).max(x - self.max[0]).max(0) as f64;
        let dz = (self.min[1] - z).max(z - self.max[1]).max(0) as f64;
        let t = (1.0 - dx.hypot(dz) / self.radius).clamp(0.0, 1.0);
        t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Plan {
    pub site: [i32; 2],
    pub config: Config,
    pub patches: Vec<Patch>,
}
fn fixed_legacy_floor(pieces: &[Piece]) -> Option<i32> {
    pieces
        .iter()
        .filter_map(|p| match p {
            Piece::Legacy(p) => p.fixed_surface_floor(),
            Piece::Pool(_) => None,
        })
        .min()
}
impl Plan {
    /// Sample noise-only terrain; never ask neighbouring generated chunks.
    pub fn evaluate(
        start: &StructureStart,
        context: &mut impl StartContext,
        config: &Config,
        floors: &[Option<i32>],
    ) -> Option<(Self, f64)> {
        let mut patches = Vec::new();
        let mut score = 0.0;
        let sea = context.sea_level();
        let legacy_floor = fixed_legacy_floor(&start.pieces);
        'pieces: for (index, piece) in start.pieces.iter().enumerate() {
            let b = piece.bounding_box();
            let xs = [b.min[0], (b.min[0] + b.max[0]) / 2, b.max[0]];
            let zs = [b.min[2], (b.min[2] + b.max[2]) / 2, b.max[2]];
            let mut heights = Vec::with_capacity(9);
            for x in xs {
                for z in zs {
                    let ground = context.base_height(x, z, HeightmapType::OceanFloorWg);
                    let surface = context.base_height(x, z, HeightmapType::WorldSurfaceWg);
                    // No reclaiming sea/lakes or draining rivers for a surface building.
                    if ground < sea || ground != surface {
                        // Preserve bridges and waterfront pieces. Penalize these
                        // candidates, but never reclaim their fluid footprint.
                        score += 1000.0;
                        continue 'pieces;
                    }
                    heights.push(ground);
                }
            }
            let mut sorted = heights.clone();
            sorted.sort_unstable();
            let median = sorted[4];
            let (floor, road) = match piece {
                Piece::Pool(p) => (
                    floors[index].unwrap_or(b.min[1] + p.ground_level_delta),
                    p.element.projection() == Projection::TerrainMatching,
                ),
                Piece::Legacy(_) => (legacy_floor.unwrap_or(median), false),
            };
            // Upper storeys need no independent ground platform.
            if start.pieces.iter().any(|other| {
                let lower = other.bounding_box();
                lower.min[1] < b.min[1]
                    && lower.min[0] <= b.min[0]
                    && lower.max[0] >= b.max[0]
                    && lower.min[2] <= b.min[2]
                    && lower.max[2] >= b.max[2]
            }) {
                continue;
            }
            let mut grade = [0.0; 2];
            let mut floor = f64::from(floor);
            if road {
                // Fit a gentle plane to each terrain-matching road, rather than
                // flattening all village roads to the town centre's elevation.
                grade = [
                    f64::from(heights[7] - heights[1]) / f64::from((b.max[0] - b.min[0]).max(1)),
                    f64::from(heights[5] - heights[3]) / f64::from((b.max[2] - b.min[2]).max(1)),
                ];
                let length = grade[0].hypot(grade[1]);
                if length > config.max_slope {
                    for g in &mut grade {
                        *g *= config.max_slope / length;
                    }
                }
                floor = f64::from(median)
                    - grade[0] * f64::from(xs[1] - xs[0])
                    - grade[1] * f64::from(zs[1] - zs[0]);
            }
            let size = (b.max[0] - b.min[0] + 1).max(b.max[2] - b.min[2] + 1);
            let patch = Patch {
                min: [b.min[0], b.min[2]],
                max: [b.max[0], b.max[2]],
                road,
                floor,
                grade,
                radius: (f64::from(size) * config.radius_scale)
                    .clamp(config.min_radius as f64, config.max_radius as f64),
            };
            for (i, h) in heights.iter().enumerate() {
                let delta = patch.target(xs[i / 3], zs[i % 3]) - f64::from(*h);
                if delta > config.max_fill as f64 || -delta > config.max_excavation as f64 {
                    return None;
                }
                score += delta * delta;
            }
            // Grade over the support footprint, not every two-block bump
            // inside a lamp post or other tiny village decoration.
            let span = f64::from((size - 1).max(config.min_radius * 2));
            if !road && f64::from(sorted[8] - sorted[0]) / span > config.max_slope {
                return None;
            }
            patches.push(patch);
        }
        if patches.is_empty() {
            return None;
        }
        Some((
            Self {
                site: [start.chunk.x, start.chunk.z],
                config: config.clone(),
                patches,
            },
            score,
        ))
    }
    pub fn bounds(&self) -> BoundingBox {
        let mut b = BoundingBox::new([i32::MAX; 3], [i32::MIN; 3]);
        for p in &self.patches {
            let r = p.radius.ceil() as i32;
            b.encapsulate(&BoundingBox::new(
                [
                    p.min[0] - r,
                    p.floor.floor() as i32 - self.config.max_excavation - 2,
                    p.min[1] - r,
                ],
                [
                    p.max[0] + r,
                    p.floor.ceil() as i32 + self.config.max_fill + 2,
                    p.max[1] + r,
                ],
            ));
        }
        b
    }
}

/// Runs between aquifer noise filling and surface rules. Writes only new chunks;
/// no material guessing, cave filling, fluid replacement or neighbour mutation.
pub fn apply<B: TerrainBlockStates>(
    chunk: &mut TerrainChunk<'_, B>,
    starts: &[std::sync::Arc<StructureStart>],
    stone: BlockStateId,
    air: BlockStateId,
    sea: i32,
    mut outside_water: impl FnMut(i32, i32) -> Option<i32>,
) -> Result<(), TerrainError> {
    if !starts.iter().any(|s| s.surface.is_some()) {
        return Ok(());
    }
    // Snapshot fluids before any terrain writes; traversal order cannot affect
    // shore protection. Neighbour queries are needed only at chunk edges.
    let water: [[Option<i32>; 16]; 16] = std::array::from_fn(|z| {
        std::array::from_fn(|x| {
            let surface = chunk
                .heightmap(HeightmapType::WorldSurfaceWg)?
                .first_available(x as i32, z as i32)
                .ok()?;
            let floor = chunk
                .heightmap(HeightmapType::OceanFloorWg)?
                .first_available(x as i32, z as i32)
                .ok()?;
            (surface > floor).then_some(surface)
        })
    });
    for z in 0..16 {
        for x in 0..16 {
            let wx = chunk.position().x * 16 + x;
            let wz = chunk.position().z * 16 + z;
            let Some(original) = chunk
                .heightmap(HeightmapType::OceanFloorWg)
                .and_then(|m| m.first_available(x, z).ok())
            else {
                continue;
            };
            if original < sea
                || chunk
                    .heightmap(HeightmapType::WorldSurfaceWg)
                    .and_then(|m| m.first_available(x, z).ok())
                    != Some(original)
            {
                continue;
            }
            let mut total = 0.0;
            let mut weighted = 0.0;
            let mut influence: f64 = 0.0;
            let mut fill = 32;
            let mut cut = 32;
            let mut inside: Option<(bool, f64)> = None;
            for plan in starts.iter().filter_map(|s| s.surface.as_ref()) {
                for patch in &plan.patches {
                    let w = patch.weight(wx, wz);
                    if w == 0.0 {
                        continue;
                    }
                    if w == 1.0 {
                        let candidate = (patch.road, patch.target(wx, wz));
                        if inside.is_none_or(|old| {
                            !candidate.0 && old.0 || candidate.0 == old.0 && candidate.1 < old.1
                        }) {
                            inside = Some(candidate);
                        }
                    }
                    // Infinite confidence at footprint edges keeps a nearby
                    // patch's feather from introducing a step at this edge.
                    let confidence = w / (1.0 - w).max(1e-9);
                    weighted += confidence * patch.target(wx, wz);
                    total += confidence;
                    influence = influence.max(w);
                    fill = fill.min(plan.config.max_fill);
                    cut = cut.min(plan.config.max_excavation);
                }
            }
            if total == 0.0 {
                continue;
            }
            let target = inside
                .map(|(_, floor)| floor)
                .unwrap_or(
                    f64::from(original) + (weighted / total - f64::from(original)) * influence,
                )
                .round() as i32;
            let mut target = target.clamp(original - cut, original + fill).max(sea);
            if target < original {
                for dz in -1..=1 {
                    for dx in -1..=1 {
                        if dx == 0 && dz == 0 {
                            continue;
                        }
                        let (nx, nz) = (x + dx, z + dz);
                        let level = if (0..16).contains(&nx) && (0..16).contains(&nz) {
                            water[nz as usize][nx as usize]
                        } else {
                            outside_water(wx + dx, wz + dz)
                        };
                        if let Some(level) = level {
                            target = target.max(level.min(original));
                        }
                    }
                }
            }
            // Retain thin cave roofs and fluid pockets. Only reshape a continuous
            // near-surface solid column, anchored below the excavation limit.
            let anchor = original.min(target) - 2;
            if anchor < chunk.min_y() || target >= chunk.min_y() + chunk.height() {
                continue;
            }
            if (anchor..original).any(|y| {
                let s = chunk.block_properties(chunk.block(wx, y, wz));
                s.is_air() || s.has_fluid()
            }) {
                continue;
            }
            for y in original.min(target)..original.max(target) {
                chunk.set_block_worldgen(wx, y, wz, if target > original { stone } else { air })?;
            }
        }
    }
    Ok(())
}

mod exact_float {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(f: &f64, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(f.to_bits())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
        Ok(f64::from_bits(u64::deserialize(d)?))
    }
}
mod exact_pair {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    pub fn serialize<S: Serializer>(f: &[f64; 2], s: S) -> Result<S::Ok, S::Error> {
        f.map(f64::to_bits).serialize(s)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[f64; 2], D::Error> {
        Ok(<[u64; 2]>::deserialize(d)?.map(f64::from_bits))
    }
}

/// Settle connected rigid building components together, independently of village
/// roads. Upper storeys retain their offsets; the town is never one platform.
pub fn settle(
    start: &mut StructureStart,
    mut height: impl FnMut(i32, i32) -> i32,
) -> Vec<Option<i32>> {
    let mut floors = vec![None; start.pieces.len()];
    let indices: Vec<usize> = start
        .pieces
        .iter()
        .enumerate()
        .filter_map(|(i, p)| {
            matches!(p,Piece::Pool(p) if p.element.projection()==Projection::Rigid).then_some(i)
        })
        .collect();
    let mut remaining: std::collections::BTreeSet<usize> = indices.into_iter().collect();
    while let Some(first) = remaining.pop_first() {
        let mut group = vec![first];
        let mut at = 0;
        while at < group.len() {
            let a = start.pieces[group[at]].bounding_box();
            let connected: Vec<usize> = remaining
                .iter()
                .copied()
                .filter(|&i| {
                    let b = start.pieces[i].bounding_box();
                    a.min[0] <= b.max[0] + 1
                        && b.min[0] <= a.max[0] + 1
                        && a.min[2] <= b.max[2] + 1
                        && b.min[2] <= a.max[2] + 1
                })
                .collect();
            for i in connected {
                remaining.remove(&i);
                group.push(i);
            }
            at += 1;
        }
        let mut box_ = start.pieces[first].bounding_box();
        // The first generated piece anchors this building. A cellar must
        // remain below that anchor rather than becoming the ground floor.
        let Piece::Pool(anchor) = &start.pieces[first] else {
            unreachable!()
        };
        let floor = anchor.bounding_box.min[1] + anchor.ground_level_delta;
        for &i in &group {
            box_.encapsulate(&start.pieces[i].bounding_box());
        }
        let mut heights = Vec::new();
        for x in [box_.min[0], (box_.min[0] + box_.max[0]) / 2, box_.max[0]] {
            for z in [box_.min[2], (box_.min[2] + box_.max[2]) / 2, box_.max[2]] {
                heights.push(height(x, z));
            }
        }
        heights.sort_unstable();
        let delta = heights[4] - floor;
        for i in group {
            floors[i] = Some(heights[4]);
            if let Piece::Pool(p) = &mut start.pieces[i] {
                p.position[1] += delta;
                p.bounding_box = p.bounding_box.moved([0, delta, 0]);
                for j in &mut p.junctions {
                    j.source_ground_y += delta;
                }
            }
        }
    }
    floors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{nbt::Tag, registry::BlockRegistry};
    use horizon_core::position::ChunkPos;
    use std::sync::Arc;
    #[derive(Clone, Copy)]
    struct State(u32);
    impl HeightmapState for State {
        fn is_air(self) -> bool {
            self.0 < 2
        }
        fn is_air_block(self) -> bool {
            self.0 < 2
        }
        fn has_fluid(self) -> bool {
            self.0 == 3
        }
        fn blocks_motion_in_heightmap(self) -> bool {
            self.0 == 2
        }
        fn blocks_motion_in_heightmap_no_leaves(self) -> bool {
            self.0 == 2
        }
    }
    struct Properties;
    impl TerrainBlockStates for Properties {
        type State = State;
        fn heightmap_state(&self, id: BlockStateId) -> State {
            State(id.0)
        }
    }
    fn registry() -> BlockRegistry {
        let names = [
            "minecraft:air",
            "minecraft:void_air",
            "minecraft:stone",
            "minecraft:water",
        ];
        BlockRegistry::from_nbt(&Tag::compound([
            ("DataVersion", Tag::Int(crate::DATA_VERSION)),
            (
                "states",
                Tag::List(
                    names
                        .iter()
                        .map(|s| Tag::compound([("id", Tag::String((*s).into()))]))
                        .collect(),
                ),
            ),
            (
                "defaults",
                Tag::Compound(
                    names
                        .iter()
                        .enumerate()
                        .map(|(i, s)| ((*s).into(), Tag::Int(i as i32)))
                        .collect(),
                ),
            ),
        ]))
        .unwrap()
    }
    fn plan(floor: f64) -> Arc<StructureStart> {
        let p = Plan {
            site: [0, 0],
            config: Config::default(),
            patches: vec![Patch {
                min: [10, 2],
                max: [22, 13],
                road: false,
                floor,
                grade: [0.0; 2],
                radius: 8.0,
            }],
        };
        Arc::new(StructureStart {
            surface: Some(p),
            structure: 0,
            name: "test:surface".into(),
            chunk: ChunkPos::new(0, 0),
            pieces: vec![],
            bounding_box: BoundingBox::new([0; 3], [32; 3]),
        })
    }
    fn chunk<'a>(r: &'a BlockRegistry, pos: ChunkPos, top: i32) -> TerrainChunk<'a, Properties> {
        let mut c = TerrainChunk::new(pos, 0, 128, r, Properties).unwrap();
        for z in 0..16 {
            for x in 0..16 {
                for y in 0..top {
                    c.set_block_raw(x, y, z, BlockStateId(2)).unwrap();
                }
            }
        }
        c.prime_heightmaps(&[HeightmapType::OceanFloorWg, HeightmapType::WorldSurfaceWg])
            .unwrap();
        c
    }
    #[test]
    fn surface_terrain_bounded_and_footprint_scaled() {
        let p = plan(74.0);
        let patch = &p.surface.as_ref().unwrap().patches[0];
        assert_eq!(patch.weight(15, 8), 1.0);
        assert_eq!(patch.weight(31, 8), 0.0);
        assert!((0.0..1.0).contains(&patch.weight(26, 8)));
        assert_eq!(patch.weight(30, 8), 0.0);
        assert!(Config::default().valid());
        assert!(
            !Config {
                max_radius: 33,
                ..Config::default()
            }
            .valid()
        );
    }
    #[test]
    fn surface_terrain_preserves_caves_water_and_distant_terrain() {
        let r = registry();
        let mut c = chunk(&r, ChunkPos::new(0, 0), 70);
        // Deep caves and thin roofs must remain untouched.
        c.set_block_raw(12, 30, 5, BlockStateId(0)).unwrap();
        c.set_block_raw(13, 68, 5, BlockStateId(0)).unwrap();
        c.set_block_raw(14, 70, 5, BlockStateId(3)).unwrap();
        c.prime_heightmaps(&[HeightmapType::OceanFloorWg, HeightmapType::WorldSurfaceWg])
            .unwrap();
        apply(
            &mut c,
            &[plan(74.0)],
            BlockStateId(2),
            BlockStateId(0),
            62,
            |_, _| None,
        )
        .unwrap();
        assert_eq!(c.block(12, 30, 5), BlockStateId(0));
        assert_eq!(c.block(12, 73, 5), BlockStateId(2));
        assert_eq!(c.block(13, 70, 5), BlockStateId(0));
        assert_eq!(c.block(14, 70, 5), BlockStateId(3));
        assert_eq!(c.block(0, 70, 5), BlockStateId(0));
        let mut c = chunk(&r, ChunkPos::new(0, 0), 70);
        apply(
            &mut c,
            &[plan(100.0)],
            BlockStateId(2),
            BlockStateId(0),
            62,
            |_, _| None,
        )
        .unwrap();
        assert_eq!(c.block(12, 77, 5), BlockStateId(2));
        assert_eq!(c.block(12, 78, 5), BlockStateId(0));
    }
    #[test]
    fn surface_terrain_nearby_feathers_do_not_step_at_foundation_edge() {
        let r = registry();
        let mut c = chunk(&r, ChunkPos::new(0, 0), 70);
        let mut start = plan(74.0);
        let patches = &mut Arc::get_mut(&mut start)
            .unwrap()
            .surface
            .as_mut()
            .unwrap()
            .patches;
        patches[0].min = [8, 0];
        patches[0].max = [12, 15];
        patches[0].radius = 12.0;
        let mut other = patches[0].clone();
        other.min = [20, 0];
        other.max = [24, 15];
        other.floor = 68.0;
        patches.push(other);
        apply(
            &mut c,
            &[start],
            BlockStateId(2),
            BlockStateId(0),
            62,
            |_, _| None,
        )
        .unwrap();
        let heights = c.heightmap(HeightmapType::WorldSurfaceWg).unwrap();
        assert_eq!(heights.first_available(12, 8).unwrap(), 74);
        assert_eq!(heights.first_available(13, 8).unwrap(), 74);
    }
    #[test]
    fn surface_terrain_shores_protected_across_chunk_boundary() {
        let r = registry();
        let mut c = chunk(&r, ChunkPos::new(0, 0), 70);
        c.set_block_raw(14, 70, 5, BlockStateId(3)).unwrap();
        c.prime_heightmaps(&[HeightmapType::OceanFloorWg, HeightmapType::WorldSurfaceWg])
            .unwrap();
        apply(
            &mut c,
            &[plan(66.0)],
            BlockStateId(2),
            BlockStateId(0),
            62,
            |x, _| if x == 16 { Some(71) } else { None },
        )
        .unwrap();
        assert_eq!(c.block(13, 69, 5), BlockStateId(2));
        assert_eq!(c.block(14, 70, 5), BlockStateId(3));
        assert_eq!(c.block(15, 69, 8), BlockStateId(2));
        assert_eq!(c.block(10, 69, 8), BlockStateId(0));
    }
    #[test]
    fn surface_terrain_chunk_order_and_saved_plan_are_stable() {
        let r = registry();
        let plans = [plan(74.0)];
        let encoded = serde_json::to_string(plans[0].surface.as_ref().unwrap()).unwrap();
        let decoded: Plan = serde_json::from_str(&encoded).unwrap();
        assert_eq!(encoded, serde_json::to_string(&decoded).unwrap());
        let mut results = Vec::new();
        for order in [[0, 1], [1, 0]] {
            let mut snapshots = std::collections::BTreeMap::new();
            for x in order {
                let mut c = chunk(&r, ChunkPos::new(x, 0), 70);
                apply(
                    &mut c,
                    &plans,
                    BlockStateId(2),
                    BlockStateId(0),
                    62,
                    |_, _| None,
                )
                .unwrap();
                snapshots.insert(
                    x,
                    (0..16)
                        .map(|local_x| {
                            c.heightmap(HeightmapType::WorldSurfaceWg)
                                .unwrap()
                                .first_available(local_x, 8)
                                .unwrap()
                        })
                        .collect::<Vec<_>>(),
                );
            }
            results.push(snapshots);
        }
        assert_eq!(results[0], results[1]);
        assert_eq!(results[0][&0][15], 74);
        assert_eq!(results[0][&1][0], 74);
    }
}

#[cfg(test)]
mod settlement_tests {
    use super::super::{
        Rotation, StructureTemplate,
        jigsaw::{PoolElement, PoolPiece},
    };
    use super::*;
    use horizon_core::position::ChunkPos;
    use std::sync::Arc;
    fn piece(x: i32, y: i32, road: bool) -> Piece {
        Piece::Pool(PoolPiece {
            element: Arc::new(PoolElement::Single {
                template: Arc::new(StructureTemplate::empty()),
                processors: Default::default(),
                processors_source: serde_json::Value::Null,
                legacy: false,
                projection: if road {
                    Projection::TerrainMatching
                } else {
                    Projection::Rigid
                },
                waterlogging: None,
                enhanced_terrain: None,
                ground_level: 1,
            }),
            position: [x, y, 0],
            ground_level_delta: 1,
            rotation: Rotation::None,
            bounding_box: BoundingBox::new([x, y, 0], [x + 7, y + 5, 7]),
            junctions: vec![],
            waterlogging: false,
        })
    }
    #[test]
    fn surface_terrain_settles_buildings_without_flattening_village() {
        let mut start = StructureStart {
            surface: None,
            structure: 0,
            name: "test:village".into(),
            chunk: ChunkPos::new(0, 0),
            bounding_box: BoundingBox::new([0; 3], [32; 3]),
            pieces: vec![
                piece(0, 64, false),
                piece(0, 70, false),
                piece(24, 64, false),
                piece(10, 64, true),
                piece(0, 58, false),
            ],
        };
        let floors = settle(&mut start, |x, _| if x < 16 { 80 } else { 72 });
        assert_eq!(floors, vec![Some(80), Some(80), Some(72), None, Some(80)]);
        assert_eq!(start.pieces[0].bounding_box().min[1], 79);
        assert_eq!(start.pieces[1].bounding_box().min[1], 85);
        assert_eq!(start.pieces[2].bounding_box().min[1], 71);
        assert_eq!(start.pieces[3].bounding_box().min[1], 64);
        assert_eq!(start.pieces[4].bounding_box().min[1], 73);
    }
    #[test]
    fn surface_terrain_fixed_legacy_floor_preserves_mansion_storeys() {
        use super::super::legacy::{TemplateHandler, TemplatePiece};
        use super::super::{Mirror, processor::ProcessorList};
        let template = |y, handler| {
            Piece::Legacy(Box::new(TemplatePiece::new(
                "minecraft:wmp",
                "test:room",
                Arc::new(StructureTemplate::empty()),
                [0, y, 0],
                Rotation::None,
                Mirror::None,
                [0; 3],
                Arc::new(ProcessorList::default()),
                true,
                vec![],
                handler,
            )))
        };
        let pieces = vec![
            template(70, TemplateHandler::Mansion),
            template(78, TemplateHandler::Mansion),
        ];
        // Room floors share one terrain platform even where sampled natural
        // terrain would put the second room's local median much higher.
        assert_eq!(fixed_legacy_floor(&pieces).unwrap_or(94), 71);
        let projected = vec![template(90, TemplateHandler::Igloo)];
        assert_eq!(fixed_legacy_floor(&projected).unwrap_or(94), 94);
    }
    #[test]
    fn surface_terrain_preserves_exact_saved_slopes() {
        let p = Plan {
            site: [1, -1],
            config: Config::default(),
            patches: vec![Patch {
                min: [1, 2],
                max: [30, 40],
                road: true,
                floor: 73.17142857142856,
                grade: [1.0 / 22.0, -1.0 / 7.0],
                radius: 7.5,
            }],
        };
        let text = serde_json::to_string(&p).unwrap();
        let restored: Plan = serde_json::from_str(&text).unwrap();
        assert_eq!(
            p.patches[0].floor.to_bits(),
            restored.patches[0].floor.to_bits()
        );
        assert_eq!(
            p.patches[0].grade.map(f64::to_bits),
            restored.patches[0].grade.map(f64::to_bits)
        );
    }
}
