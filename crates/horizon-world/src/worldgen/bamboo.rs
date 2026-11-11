//! Bamboo stalks, podzol patches and ordered cap overwrites.
use super::{
    DecodeError, Object,
    block_feature::FeatureBlockBindings,
    feature::FeatureWorld,
    number_text,
    placement::{PlacementContext, PlacementQueryError},
    required,
};
use crate::{heightmap::HeightmapType, registry::BlockStateId};
use horizon_core::random::{InvalidBound, RandomSource};
use std::{collections::BTreeSet, sync::Arc};
#[derive(Clone, Debug)]
pub struct BambooFeature {
    probability: f32,
    patch: Option<BambooPatch>,
    air: [BlockStateId; 3],
    bamboo: BlockStateId,
    trunk: BlockStateId,
    caps: [BlockStateId; 3],
    podzol: BlockStateId,
    replaceable: Arc<BTreeSet<BlockStateId>>,
}
/// Optional content-defined replacement; vanilla bamboo has no patch selector.
#[derive(Clone, Debug)]
struct BambooPatch {
    trunk: BlockStateId,
    caps: [BlockStateId; 3],
    size: i32,
    chance: f64,
}

fn patch_hash(seed: u64, x: i64, z: i64) -> u64 {
    let mut n = seed
        ^ (x as u64).wrapping_mul(0x9e3779b97f4a7c15)
        ^ (z as u64).wrapping_mul(0xd1b54a32d192ed03);
    n = (n ^ (n >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    n = (n ^ (n >> 27)).wrapping_mul(0x94d049bb133111eb);
    n ^ (n >> 31)
}

/// Jittered cells give coherent, irregular patches independent of chunk order,
/// stalk height and the feature RNG stream. Seed varies their layout per world.
fn bushy_patch(seed: i64, x: i32, z: i32, size: i32, chance: f64) -> bool {
    let cell_x = i64::from(x.div_euclid(size));
    let cell_z = i64::from(z.div_euclid(size));
    let unit = |h: u64| (h >> 11) as f64 / ((1u64 << 53) as f64);
    let mut closest = f64::INFINITY;
    let mut chosen = 0;
    for cx in cell_x - 1..=cell_x + 1 {
        for cz in cell_z - 1..=cell_z + 1 {
            let hash = patch_hash(seed as u64 ^ 0x6275736879, cx, cz);
            let px = (cx as f64 + 0.25 + 0.5 * unit(hash)) * f64::from(size);
            let pz = (cz as f64 + 0.25 + 0.5 * unit(hash.rotate_left(29))) * f64::from(size);
            let distance = (f64::from(x) - px).powi(2) + (f64::from(z) - pz).powi(2);
            if distance < closest {
                closest = distance;
                chosen = patch_hash(hash ^ 0x7061746368, cx, cz);
            }
        }
    }
    unit(chosen) < chance
}

#[derive(Debug)]
pub enum BambooError<E> {
    World(E),
    Query(PlacementQueryError),
    Random(InvalidBound),
    WorkLimit,
}
impl<E: std::fmt::Display> std::fmt::Display for BambooError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::World(e) => e.fmt(f),
            Self::Query(e) => e.fmt(f),
            Self::Random(e) => e.fmt(f),
            Self::WorkLimit => f.write_str("bamboo work limit exceeded"),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for BambooError<E> {}
impl BambooFeature {
    pub fn from_json(json: &str, bindings: &FeatureBlockBindings<'_>) -> Result<Self, DecodeError> {
        let object: Object = serde_json::from_str(json)?;
        let probability = number_text(required(&object, "probability")?)?
            .parse::<f64>()
            .map_err(|_| DecodeError::Field("bamboo probability"))?
            as f32;
        if !(0.0..=1.0).contains(&probability) || probability.is_sign_negative() {
            return Err(DecodeError::Field("bamboo probability"));
        }
        let state = |leaves, stage| {
            bindings.codec.decode(&format!(r#"{{"id":"bamboo","properties":{{"age":"1","leaves":"{leaves}","stage":"{stage}"}}}}"#),false).map_err(|e|DecodeError::Registry(e.to_string()))
        };
        let get = |name| {
            bindings
                .registry()
                .default_state(name)
                .ok_or_else(|| DecodeError::Registry(format!("unknown bamboo block {name}")))
        };
        let patch = object
            .get("patch")
            .map(|raw| -> Result<BambooPatch, DecodeError> {
                let patch: Object = serde_json::from_str(raw.get())?;
                let block: String = serde_json::from_str(required(&patch, "block")?.get())?;
                let size: i32 = serde_json::from_str(required(&patch, "size")?.get())?;
                let chance: f64 = serde_json::from_str(required(&patch, "chance")?.get())?;
                if !(4..=256).contains(&size) || !(0.0..=1.0).contains(&chance) {
                    return Err(DecodeError::Field("bamboo patch size/chance"));
                }
                let state = |leaves: &str, stage: i32| {
                    bindings
                        .codec
                        .decode(
                            &serde_json::json!({"id": block,
                    "properties": {"age":"1", "leaves":leaves, "stage":stage.to_string()}})
                            .to_string(),
                            false,
                        )
                        .map_err(|e| DecodeError::Registry(e.to_string()))
                };
                Ok(BambooPatch {
                    trunk: state("none", 0)?,
                    caps: [state("large", 1)?, state("large", 0)?, state("small", 0)?],
                    size,
                    chance,
                })
            })
            .transpose()?;
        Ok(Self {
            probability,
            patch,
            air: bindings.air,
            bamboo: get("minecraft:bamboo")?,
            trunk: state("none", 0)?,
            caps: [state("large", 1)?, state("large", 0)?, state("small", 0)?],
            podzol: get("minecraft:podzol")?,
            replaceable: bindings
                .tags
                .get("minecraft:beneath_bamboo_podzol_replaceable")
                .cloned()
                .unwrap_or_default(),
        })
    }
    pub fn place<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        random: &mut impl RandomSource,
        origin: [i32; 3],
    ) -> Result<bool, BambooError<W::Error>> {
        if !self
            .air
            .contains(&world.block(origin).map_err(BambooError::World)?)
        {
            return Ok(false);
        }
        // Native success records the air-origin attempt, even if survival fails.
        if !world
            .block_survives(self.bamboo, origin)
            .map_err(BambooError::Query)?
        {
            return Ok(true);
        }
        let mut trunk = self.trunk;
        let mut caps = self.caps;
        if let Some(patch) = &self.patch {
            let seed = world.world_seed().map_err(BambooError::Query)?;
            if bushy_patch(seed, origin[0], origin[2], patch.size, patch.chance) {
                trunk = patch.trunk;
                caps = patch.caps;
            }
        }
        let height = random.next_int_bound(12).map_err(BambooError::Random)? + 5;
        if random.next_float() < self.probability {
            let radius = random.next_int_bound(4).map_err(BambooError::Random)? + 1;
            let mut x = origin[0].wrapping_sub(radius);
            let max_x = origin[0].wrapping_add(radius);
            let mut remaining = 1_048_576;
            while x <= max_x {
                let mut z = origin[2].wrapping_sub(radius);
                let max_z = origin[2].wrapping_add(radius);
                while z <= max_z {
                    if remaining == 0 {
                        return Err(BambooError::WorkLimit);
                    }
                    remaining -= 1;
                    let dx = x.wrapping_sub(origin[0]);
                    let dz = z.wrapping_sub(origin[2]);
                    if dx.wrapping_mul(dx).wrapping_add(dz.wrapping_mul(dz)) <= radius * radius {
                        let y = world
                            .height(HeightmapType::WorldSurface, x, z)
                            .map_err(BambooError::Query)?
                            .wrapping_sub(1);
                        let p = [x, y, z];
                        if self
                            .replaceable
                            .contains(&world.block(p).map_err(BambooError::World)?)
                        {
                            world
                                .set_block(p, self.podzol, 2)
                                .map_err(BambooError::World)?;
                        }
                    }
                    z = z.wrapping_add(1);
                }
                // Also bound overflow loops when the inner range is empty.
                if remaining == 0 {
                    return Err(BambooError::WorkLimit);
                }
                remaining -= 1;
                x = x.wrapping_add(1);
            }
        }
        let mut p = origin;
        for _ in 0..height {
            if !self
                .air
                .contains(&world.block(p).map_err(BambooError::World)?)
            {
                break;
            }
            world.set_block(p, trunk, 2).map_err(BambooError::World)?;
            p[1] = p[1].wrapping_add(1);
        }
        if p[1].wrapping_sub(origin[1]) >= 3 {
            // The first cap occupies the stopping position, including an obstacle.
            for state in caps {
                world.set_block(p, state, 2).map_err(BambooError::World)?;
                p[1] = p[1].wrapping_sub(1);
            }
        }
        Ok(true)
    }
}

#[cfg(test)]
mod patch_tests {
    use super::super::anchor::WorldGenerationContext;
    use super::*;
    use horizon_core::random::LegacyRandom;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct World {
        blocks: BTreeMap<[i32; 3], BlockStateId>,
        seeded: bool,
    }
    impl FeatureWorld for World {
        type Error = std::convert::Infallible;
        fn block(&mut self, p: [i32; 3]) -> Result<BlockStateId, Self::Error> {
            Ok(self.blocks.get(&p).copied().unwrap_or(BlockStateId(0)))
        }
        fn set_block(
            &mut self,
            p: [i32; 3],
            state: BlockStateId,
            _: i32,
        ) -> Result<bool, Self::Error> {
            self.blocks.insert(p, state);
            Ok(true)
        }
    }
    impl PlacementContext for World {
        fn generation(&self) -> WorldGenerationContext {
            WorldGenerationContext::new(0, 64, 32)
        }
        fn min_y(&mut self) -> Result<i32, PlacementQueryError> {
            Ok(0)
        }
        fn height(&mut self, _: HeightmapType, _: i32, _: i32) -> Result<i32, PlacementQueryError> {
            unreachable!()
        }
        fn block_survives(
            &mut self,
            _: BlockStateId,
            _: [i32; 3],
        ) -> Result<bool, PlacementQueryError> {
            Ok(true)
        }
        fn world_seed(&mut self) -> Result<i64, PlacementQueryError> {
            assert!(
                self.seeded,
                "vanilla feature must not add a world-seed query"
            );
            Ok(123)
        }
    }
    #[test]
    fn patched_stalk_preserves_height_caps_and_random_stream() {
        let vanilla = BambooFeature {
            probability: 0.0,
            patch: None,
            air: [BlockStateId(0); 3],
            bamboo: BlockStateId(1),
            trunk: BlockStateId(2),
            caps: [BlockStateId(3), BlockStateId(4), BlockStateId(5)],
            podzol: BlockStateId(6),
            replaceable: Arc::new(BTreeSet::new()),
        };
        for chance in [0.0, 1.0] {
            let mut patched = vanilla.clone();
            patched.patch = Some(BambooPatch {
                trunk: BlockStateId(22),
                caps: [BlockStateId(23), BlockStateId(24), BlockStateId(25)],
                size: 16,
                chance,
            });
            let mut a = World::default();
            let mut b = World {
                seeded: true,
                ..World::default()
            };
            let mut ra = LegacyRandom::new(17);
            let mut rb = ra.clone();
            vanilla.place(&mut a, &mut ra, [-16, 1, 15]).unwrap();
            patched.place(&mut b, &mut rb, [-16, 1, 15]).unwrap();
            assert_eq!(a.blocks.len(), b.blocks.len());
            for (p, state) in &a.blocks {
                assert_eq!(b.blocks[p].0, state.0 + if chance == 1.0 { 20 } else { 0 });
            }
            assert_eq!(ra.next_long(), rb.next_long());
        }
    }
    #[test]
    fn patches_are_clustered_seeded_and_cover_both_coordinate_signs() {
        let mut selected = 0;
        let mut same = 0;
        let mut different_seed = 0;
        for x in -96..96 {
            for z in -96..96 {
                let value = bushy_patch(123, x, z, 16, 0.35);
                selected += usize::from(value);
                same += usize::from(value == bushy_patch(123, x + 1, z, 16, 0.35));
                different_seed += usize::from(value != bushy_patch(456, x, z, 16, 0.35));
                assert!(!bushy_patch(123, x, z, 16, 0.0));
                assert!(bushy_patch(123, x, z, 16, 1.0));
            }
        }
        let count = 192 * 192;
        assert!(selected > count / 5 && selected < count / 2, "{selected}");
        assert!(same > count * 9 / 10, "{same}");
        assert!(different_seed > count / 5);
    }
}
