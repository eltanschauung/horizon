//! Registry-bound native feature extension point. Content algorithms belong to mods.
use super::{
    DecodeError,
    block_feature::FeatureBlockBindings,
    feature::{FeatureError, FeatureWorld},
    feature_graph::FeatureExecutionError,
    placement::{PlacementContext, PlacementQueryError},
};
use crate::registry::BlockStateId;
use horizon_core::random::RandomSource;
use std::sync::Arc;

pub trait NativeWorld {
    fn block(&mut self, at: [i32; 3]) -> Result<BlockStateId, PlacementQueryError>;
    fn set(
        &mut self,
        at: [i32; 3],
        state: BlockStateId,
        flags: i32,
    ) -> Result<bool, PlacementQueryError>;
    fn min_y(&mut self) -> Result<i32, PlacementQueryError>;
    fn max_y(&mut self) -> Result<i32, PlacementQueryError>;
}
pub trait NativeRandom {
    fn int(&mut self, bound: i32) -> Result<i32, PlacementQueryError>;
    fn float(&mut self) -> f32;
    fn double(&mut self) -> f64;
    fn boolean(&mut self) -> bool;
}
struct RandomBridge<'a, R>(&'a mut R);
impl<R: RandomSource> NativeRandom for RandomBridge<'_, R> {
    fn int(&mut self, bound: i32) -> Result<i32, PlacementQueryError> {
        self.0
            .next_int_bound(bound)
            .map_err(|e| PlacementQueryError::Failed(e.to_string()))
    }
    fn float(&mut self) -> f32 {
        self.0.next_float()
    }
    fn double(&mut self) -> f64 {
        self.0.next_double()
    }
    fn boolean(&mut self) -> bool {
        self.0.next_boolean()
    }
}
/// Implementations bind states once when registries load, then execute against live terrain.
pub trait NativeAlgorithm: std::fmt::Debug + Send + Sync {
    fn place(
        &self,
        world: &mut dyn NativeWorld,
        random: &mut dyn NativeRandom,
        origin: [i32; 3],
    ) -> Result<bool, PlacementQueryError>;
}
#[derive(Clone, Debug)]
pub struct NativeFeature(pub Arc<dyn NativeAlgorithm>);
/// Native bundles inject their own decoder here; a plain game has no extension algorithms.
pub fn decode(
    kind: &str,
    json: &str,
    bindings: &FeatureBlockBindings<'_>,
) -> Option<Result<NativeFeature, DecodeError>> {
    let _ = (kind, json, bindings);
    None
}
struct WorldBridge<'a, W: FeatureWorld> {
    world: &'a mut W,
    error: Option<W::Error>,
    left: usize,
}
impl<W: FeatureWorld> WorldBridge<'_, W> {
    fn budget(&mut self) -> Result<(), PlacementQueryError> {
        if self.left == 0 {
            return Err(PlacementQueryError::WorkLimit);
        }
        self.left -= 1;
        Ok(())
    }
}
impl<W: FeatureWorld + PlacementContext> NativeWorld for WorldBridge<'_, W> {
    fn block(&mut self, at: [i32; 3]) -> Result<BlockStateId, PlacementQueryError> {
        self.budget()?;
        self.world.block(at).map_err(|e| {
            if self.error.is_none() {
                self.error = Some(e);
            }
            PlacementQueryError::Unavailable
        })
    }
    fn set(
        &mut self,
        at: [i32; 3],
        state: BlockStateId,
        flags: i32,
    ) -> Result<bool, PlacementQueryError> {
        self.budget()?;
        if !self.world.ensure_can_write(at)? || self.world.outside_build_height(at[1])? {
            return Ok(false);
        }
        self.world.set_block(at, state, flags).map_err(|e| {
            if self.error.is_none() {
                self.error = Some(e);
            }
            PlacementQueryError::Unavailable
        })
    }
    fn min_y(&mut self) -> Result<i32, PlacementQueryError> {
        self.world.min_y()
    }
    fn max_y(&mut self) -> Result<i32, PlacementQueryError> {
        self.world.max_y()
    }
}
impl NativeFeature {
    pub fn place<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        random: &mut impl RandomSource,
        origin: [i32; 3],
    ) -> Result<bool, FeatureExecutionError<W::Error>> {
        let mut bridge = WorldBridge {
            world,
            error: None,
            left: 1_000_000,
        };
        let result = self.0.place(&mut bridge, &mut RandomBridge(random), origin);
        if let Some(error) = bridge.error {
            return Err(FeatureExecutionError::Basic(FeatureError::World(error)));
        }
        result.map_err(FeatureExecutionError::Query)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worldgen::anchor::WorldGenerationContext;
    #[derive(Debug)]
    struct Write;
    impl NativeAlgorithm for Write {
        fn place(
            &self,
            w: &mut dyn NativeWorld,
            _: &mut dyn NativeRandom,
            p: [i32; 3],
        ) -> Result<bool, PlacementQueryError> {
            w.set(p, BlockStateId(1), 2)
        }
    }
    struct World {
        fail: bool,
        writes: usize,
        height: i32,
    }
    impl FeatureWorld for World {
        type Error = &'static str;
        fn block(&mut self, _: [i32; 3]) -> Result<BlockStateId, Self::Error> {
            Ok(BlockStateId(0))
        }
        fn set_block(&mut self, _: [i32; 3], _: BlockStateId, _: i32) -> Result<bool, Self::Error> {
            if self.fail {
                return Err("write failed");
            }
            self.writes += 1;
            Ok(true)
        }
    }
    impl PlacementContext for World {
        fn max_y(&mut self) -> Result<i32, PlacementQueryError> {
            Ok(-64 + self.height - 1)
        }
        fn generation(&self) -> WorldGenerationContext {
            WorldGenerationContext::new(-64, self.height, 63)
        }
        fn min_y(&mut self) -> Result<i32, PlacementQueryError> {
            Ok(-64)
        }
        fn height(
            &mut self,
            _: crate::heightmap::HeightmapType,
            _: i32,
            _: i32,
        ) -> Result<i32, PlacementQueryError> {
            Ok(64)
        }
    }
    #[test]
    fn native_features_preserve_world_errors() {
        let f = NativeFeature(Arc::new(Write));
        let mut w = World {
            fail: true,
            writes: 0,
            height: 384,
        };
        let error = f
            .place(
                &mut w,
                &mut horizon_core::random::LegacyRandom::new(0),
                [0, 64, 0],
            )
            .unwrap_err();
        assert!(matches!(
            error,
            FeatureExecutionError::Basic(FeatureError::World("write failed"))
        ));
        assert_eq!(w.writes, 0);
    }
    #[test]
    fn native_features_respect_the_world_build_range() {
        let f = NativeFeature(Arc::new(Write));
        let mut w = World {
            fail: false,
            writes: 0,
            height: 384,
        };
        let mut r = horizon_core::random::LegacyRandom::new(0);
        assert!(!f.place(&mut w, &mut r, [0, 320, 0]).unwrap());
        assert!(!f.place(&mut w, &mut r, [0, -65, 0]).unwrap());
        assert!(f.place(&mut w, &mut r, [0, -64, 0]).unwrap());
        assert!(f.place(&mut w, &mut r, [0, 319, 0]).unwrap());
        assert_eq!(w.writes, 2);
        w.height = 704; // Tectonic increased-height Overworld.
        assert!(f.place(&mut w, &mut r, [0, 639, 0]).unwrap());
        assert!(!f.place(&mut w, &mut r, [0, 640, 0]).unwrap());
        assert_eq!(w.writes, 3);
    }
}
