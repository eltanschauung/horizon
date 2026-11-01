//! The `template` and `fossil` configured features.
use super::{
    BoundingBox, Mirror, Rotation, StructureResources, StructureTemplate,
    processor::{PlaceSettings, ProcessorList, place_in_world},
};
use crate::{
    heightmap::HeightmapType,
    worldgen::{
        DecodeError,
        block_feature::FeatureBlockBindings,
        feature::FeatureWorld,
        placement::{PlacementContext, PlacementQueryError},
        weighted::WeightedList,
    },
};
use horizon_core::random::RandomSource;
use serde_json::{Value, value::RawValue};
use std::sync::Arc;

fn resources(bindings: &FeatureBlockBindings<'_>) -> Result<Arc<StructureResources>, DecodeError> {
    bindings
        .structures
        .clone()
        .ok_or(DecodeError::Field("structure template bindings"))
}
fn template(
    resources: &StructureResources,
    id: &str,
) -> Result<Arc<StructureTemplate>, DecodeError> {
    resources
        .template(id)
        .ok_or_else(|| DecodeError::Registry(format!("missing structure template {id}")))
}

#[derive(Clone, Debug)]
pub struct TemplateFeature {
    templates: WeightedList<(Arc<StructureTemplate>, Vec<Rotation>)>,
    processors: ProcessorList,
    resources: Arc<StructureResources>,
    offset: [i32; 3],
    clearance: Option<Arc<crate::worldgen::tree_blocks::TreeBlocks>>,
}
impl TemplateFeature {
    pub fn decode(json: &str, bindings: &FeatureBlockBindings<'_>) -> Result<Self, DecodeError> {
        let resources = resources(bindings)?;
        let value: Value = serde_json::from_str(json)?;
        let raw: Box<RawValue> = serde_json::from_str(&value["templates"].to_string())?;
        let require_environment = value["require_environment"].as_bool() == Some(true);
        if require_environment && value.get("processors").is_some() {
            return Err(DecodeError::Field(
                "guarded templates cannot relocate through processors",
            ));
        }
        let templates = WeightedList::decode(&raw, |entry| {
            let entry: Value = serde_json::from_str(entry.get())?;
            let id = entry["id"]
                .as_str()
                .ok_or(DecodeError::Field("template id"))?;
            let rotations = match entry.get("rotations").and_then(Value::as_array) {
                Some(list) => list
                    .iter()
                    .map(|r| {
                        r.as_str()
                            .and_then(Rotation::from_name)
                            .ok_or(DecodeError::Field("rotation"))
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                None => Rotation::ALL.to_vec(),
            };
            let template = template(&resources, id)?;
            if require_environment && template.environment.as_ref().is_none_or(Vec::is_empty) {
                return Err(DecodeError::Field(
                    "missing template environment requirements",
                ));
            }
            Ok((template, rotations))
        })?;
        let processors = match value.get("processors") {
            Some(p) => ProcessorList::decode(&p.to_string(), &resources, bindings)?,
            None => ProcessorList::default(),
        };
        Ok(Self {
            templates,
            processors,
            resources,
            offset: match value.get("offset") {
                Some(v) => serde_json::from_value::<[i32; 3]>(v.clone())?,
                None => [0; 3],
            },
            clearance: if value["clearance"].as_bool() == Some(true) {
                Some(bindings.tree_blocks()?)
            } else {
                None
            },
        })
    }
    pub fn place<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        random: &mut impl RandomSource,
        origin: [i32; 3],
    ) -> Result<bool, PlacementQueryError> {
        let invalid = |_| PlacementQueryError::Failed("template weights".into());
        let (template, rotations) = self.templates.sample(random).map_err(invalid)?;
        let rotation = rotations[random
            .next_int_bound(rotations.len() as i32)
            .map_err(invalid)? as usize];
        // rotation.rotate(axis.getNegative()) * (size along axis / 2), for X then Z.
        let mut pos = std::array::from_fn(|i| origin[i].wrapping_add(self.offset[i]));
        for (axis, negative) in [
            (0, horizon_core::geometry::Direction::West),
            (2, horizon_core::geometry::Direction::North),
        ] {
            let step = rotation.rotate_direction(negative).normal();
            let length = template.size[axis] / 2;
            for i in 0..3 {
                pos[i] += step[i] * length;
            }
        }
        // Validate the complete environment before writing a single block.
        if let Some(requirements) = &template.environment {
            for requirement in requirements {
                let shifted =
                    StructureTemplate::transform(requirement.pos, Mirror::None, rotation, [0; 3]);
                let p = std::array::from_fn(|i| pos[i].wrapping_add(shifted[i]));
                if !world.ensure_can_write(p)? {
                    return Ok(false);
                }
                let old = world.block_state(p)?;
                let matches = if rotation == Rotation::None {
                    requirement.states.contains(&old)
                } else {
                    requirement
                        .states
                        .iter()
                        .any(|state| self.resources.transforms.rotate(*state, rotation) == old)
                };
                if !matches {
                    return Ok(false);
                }
            }
        }
        if let Some(blocks) = &self.clearance {
            if let Some(palette) = template.palettes().first() {
                for block in palette.blocks() {
                    let shifted =
                        StructureTemplate::transform(block.pos, Mirror::None, rotation, [0; 3]);
                    let p = std::array::from_fn(|i| pos[i] + shifted[i]);
                    if p[1] >= origin[1] {
                        let old = world.block_state(p)?;
                        if !blocks.valid_tree_pos(old)
                            && !blocks.has(
                                old,
                                crate::worldgen::tree_blocks::WATER_FLUID
                                    | crate::worldgen::tree_blocks::REPLACEABLE,
                            )
                        {
                            return Ok(false);
                        }
                    }
                }
            }
        }
        let mut settings = PlaceSettings::new(rotation).with_processors(&self.processors);
        settings.random = Some(random);
        place_in_world(
            world,
            &self.resources,
            template,
            pos,
            pos,
            &mut settings,
            None,
            3,
        )
    }
}

#[derive(Clone, Debug)]
pub struct FossilFeature {
    fossils: Vec<(Arc<StructureTemplate>, Arc<StructureTemplate>)>,
    fossil_processors: ProcessorList,
    overlay_processors: ProcessorList,
    max_empty_corners: i32,
    resources: Arc<StructureResources>,
}
impl FossilFeature {
    pub fn decode(json: &str, bindings: &FeatureBlockBindings<'_>) -> Result<Self, DecodeError> {
        let resources = resources(bindings)?;
        let value: Value = serde_json::from_str(json)?;
        let list = |key: &'static str| -> Result<Vec<Arc<StructureTemplate>>, DecodeError> {
            value[key]
                .as_array()
                .ok_or(DecodeError::Field(key))?
                .iter()
                .map(|id| template(&resources, id.as_str().unwrap_or_default()))
                .collect()
        };
        let (base, overlay) = (list("fossil_structures")?, list("overlay_structures")?);
        if base.is_empty() || base.len() != overlay.len() {
            return Err(DecodeError::Field("fossil structure lists"));
        }
        Ok(Self {
            fossils: base.into_iter().zip(overlay).collect(),
            fossil_processors: ProcessorList::decode(
                &value["fossil_processors"].to_string(),
                &resources,
                bindings,
            )?,
            overlay_processors: ProcessorList::decode(
                &value["overlay_processors"].to_string(),
                &resources,
                bindings,
            )?,
            max_empty_corners: value["max_empty_corners_allowed"].as_i64().unwrap_or(0) as i32,
            resources,
        })
    }
    pub fn place<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        random: &mut impl RandomSource,
        origin: [i32; 3],
    ) -> Result<bool, PlacementQueryError> {
        let invalid = |_| PlacementQueryError::Failed("fossil bounds".into());
        let rotation = Rotation::random(random);
        let index = random
            .next_int_bound(self.fossils.len() as i32)
            .map_err(invalid)? as usize;
        let (base, overlay) = &self.fossils[index];
        let (cx, cz) = (origin[0] >> 4, origin[2] >> 4);
        let bounds = BoundingBox::new(
            [cx * 16 - 16, world.min_y()?, cz * 16 - 16],
            [cx * 16 + 15 + 16, world.max_y()?, cz * 16 + 15 + 16],
        );
        let size = base.size(rotation);
        let low = [origin[0] - size[0] / 2, 0, origin[2] - size[2] / 2];
        let mut lowest = origin[1];
        for x in 0..size[0] {
            for z in 0..size[2] {
                lowest = lowest.min(world.height(
                    HeightmapType::OceanFloorWg,
                    low[0] + x,
                    low[2] + z,
                )?);
            }
        }
        let y =
            (lowest - 15 - random.next_int_bound(10).map_err(invalid)?).max(world.min_y()? + 10);
        let target = StructureTemplate::zero_position_with_transform(
            [low[0], y, low[2]],
            Mirror::None,
            rotation,
            base.size[0],
            base.size[2],
        );
        let mut empty = 0;
        for corner in base
            .bounding_box(target, rotation, [0; 3], Mirror::None)
            .corners()
        {
            let state = world.block_state(corner)?;
            let name = &self.resources.registry.state(state).unwrap().name;
            if matches!(
                name.as_str(),
                "minecraft:air"
                    | "minecraft:cave_air"
                    | "minecraft:void_air"
                    | "minecraft:lava"
                    | "minecraft:water"
            ) {
                empty += 1;
            }
        }
        if empty > self.max_empty_corners {
            return Ok(false);
        }
        for (template, processors) in [
            (base, &self.fossil_processors),
            (overlay, &self.overlay_processors),
        ] {
            let mut settings = PlaceSettings::new(rotation).with_processors(processors);
            settings.bounding_box = Some(bounds);
            settings.random = Some(&mut *random);
            place_in_world(
                world,
                &self.resources,
                template,
                target,
                target,
                &mut settings,
                None,
                260,
            )?;
        }
        Ok(true)
    }
}

#[cfg(test)]
mod environment_tests {
    use super::*;
    use crate::{
        nbt::Tag,
        registry::{BlockRegistry, BlockStateId},
        worldgen::{anchor::WorldGenerationContext, structure::StateTransforms},
    };
    use std::collections::BTreeMap;
    fn fixture() -> TemplateFeature {
        let names = [
            "minecraft:air",
            "minecraft:stone",
            "minecraft:water",
            "minecraft:barrier",
            "minecraft:cave_air",
            "minecraft:void_air",
        ];
        let state = |name: &str| Tag::compound([("id", Tag::String(name.into()))]);
        let registry = Box::leak(Box::new(
            BlockRegistry::from_nbt(&Tag::compound([
                ("DataVersion", Tag::Int(horizon_core::version::DATA_VERSION)),
                (
                    "states",
                    Tag::List(names.iter().map(|n| state(n)).collect()),
                ),
                (
                    "defaults",
                    Tag::compound(
                        names
                            .iter()
                            .enumerate()
                            .map(|(i, n)| (*n, Tag::Int(i as i32))),
                    ),
                ),
            ]))
            .unwrap(),
        ));
        let transforms=StateTransforms::from_json(r#"{"rotate":[[0,0,0],[1,1,1],[2,2,2],[3,3,3],[4,4,4],[5,5,5]],"mirror":[[0,0],[1,1],[2,2],[3,3],[4,4],[5,5]],"full_block":[false,true,false,true,false,false],"loot_containers":[],"block_entities":{}}"#,6).unwrap();
        let ints = |p: [i32; 3]| Tag::List(p.into_iter().map(Tag::Int).collect());
        let tag = Tag::compound([
            ("size", ints([1, 1, 1])),
            ("palette", Tag::List(vec![state("minecraft:stone")])),
            (
                "blocks",
                Tag::List(vec![Tag::compound([
                    ("pos", ints([0, 0, 0])),
                    ("state", Tag::Int(0)),
                ])]),
            ),
            (
                "horizon:allowed_ground",
                Tag::List(vec![state("minecraft:stone")]),
            ),
            (
                "horizon:environment",
                Tag::List(vec![
                    Tag::compound([("pos", ints([0, -1, 0])), ("ground", Tag::Byte(1))]),
                    Tag::compound([
                        ("pos", ints([0, 0, 0])),
                        ("state", state("minecraft:air")),
                        ("air", Tag::Byte(1)),
                    ]),
                    Tag::compound([
                        ("pos", ints([0, 1, 0])),
                        ("state", state("minecraft:air")),
                        ("air", Tag::Byte(1)),
                    ]),
                ]),
            ),
        ]);
        let template = Arc::new(StructureTemplate::from_nbt(&tag, registry, &transforms));
        let resources = Arc::new(StructureResources::new(
            registry,
            transforms,
            std::path::Path::new("/nonexistent"),
            BTreeMap::new(),
            &BTreeMap::new(),
        ));
        TemplateFeature {
            templates: WeightedList::new(vec![((template, vec![Rotation::None]), 1)]).unwrap(),
            processors: ProcessorList::default(),
            resources,
            offset: [0; 3],
            clearance: None,
        }
    }
    struct World {
        origin: [i32; 3],
        ground: BlockStateId,
        ceiling: bool,
        air: BlockStateId,
        writes: Vec<[i32; 3]>,
    }
    impl FeatureWorld for World {
        type Error = ();
        fn block(&mut self, p: [i32; 3]) -> Result<BlockStateId, ()> {
            Ok(if p[1] < self.origin[1] {
                self.ground
            } else if self.ceiling && p[1] > self.origin[1] {
                BlockStateId(1)
            } else {
                self.air
            })
        }
        fn set_block(&mut self, p: [i32; 3], _: BlockStateId, _: i32) -> Result<bool, ()> {
            self.writes.push(p);
            Ok(true)
        }
    }
    impl PlacementContext for World {
        fn generation(&self) -> WorldGenerationContext {
            WorldGenerationContext::new(-64, 384, 63)
        }
        fn min_y(&mut self) -> Result<i32, PlacementQueryError> {
            Ok(-64)
        }
        fn height(&mut self, _: HeightmapType, _: i32, _: i32) -> Result<i32, PlacementQueryError> {
            panic!("an anchored feature must not project itself onto the ocean surface")
        }
        fn block_state(&mut self, p: [i32; 3]) -> Result<BlockStateId, PlacementQueryError> {
            Ok(self.block(p).unwrap())
        }
        fn fluid_state(
            &mut self,
            _: [i32; 3],
        ) -> Result<crate::fluid::FluidState, PlacementQueryError> {
            Ok(crate::fluid::FluidState::EMPTY)
        }
    }
    #[test]
    fn guarded_templates_require_solid_substrate_and_space_before_any_writes() {
        let feature = fixture();
        for air in [0, 4, 5] {
            for (y, ground, ceiling, expected) in [
                (-30, 1, false, true),
                (68, 1, false, true),
                (63, 2, false, false),
                (63, 0, false, false),
                (-30, 1, true, false),
            ] {
                let mut world = World {
                    origin: [142, y, -864],
                    ground: BlockStateId(ground),
                    ceiling,
                    air: BlockStateId(air),
                    writes: vec![],
                };
                let origin = world.origin;
                let result = feature
                    .place(
                        &mut world,
                        &mut horizon_core::random::LegacyRandom::new(42),
                        origin,
                    )
                    .unwrap();
                assert_eq!(result, expected);
                assert_eq!(world.writes, if expected { vec![origin] } else { vec![] });
            }
        }
    }
}
