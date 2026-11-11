//! Exercise surface starts against locally prepared resources; no worlds saved.
use horizon_core::position::ChunkPos;
use horizon_world::worldgen::{
    generator::{GeneratorData, OverworldGenerator},
    overrides::WorldgenOverrides,
};
use std::{collections::BTreeMap, fs, path::PathBuf};
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let root = PathBuf::from(&args[1]);
    let registry = Box::leak(Box::new(
        horizon_world::level::data::load_registry(&root).unwrap(),
    ));
    let mut ov = WorldgenOverrides::default();
    if let Some(file) = args.get(2) {
        let bundles: BTreeMap<String, BTreeMap<String, String>> =
            serde_json::from_str(&fs::read_to_string(file).unwrap()).unwrap();
        for (bundle, entries) in bundles {
            for (id, json) in entries {
                ov.set(&bundle, &id, json);
            }
        }
    }
    let disabled = std::env::var_os("HORIZON_SURFACE_FIT_DISABLED").is_some();
    if disabled {
        let source: BTreeMap<String, String> = serde_json::from_str(
            &fs::read_to_string(root.join("worldgen/structures.json")).unwrap(),
        )
        .unwrap();
        for (id, text) in source {
            let mut v: serde_json::Value = serde_json::from_str(&text).unwrap();
            v["horizon:surface_placement"] = serde_json::json!({"enabled":false});
            ov.set("structures", &id, v.to_string());
        }
    }
    if let Ok(biome) = std::env::var("HORIZON_SURFACE_TEST_BIOME") {
        let presets: BTreeMap<String, String> = serde_json::from_str(
            &fs::read_to_string(root.join("worldgen/world_presets.json")).unwrap(),
        )
        .unwrap();
        let mut preset: serde_json::Value =
            serde_json::from_str(&presets["minecraft:normal"]).unwrap();
        preset["dimensions"]["minecraft:overworld"]["generator"]["biome_source"] =
            serde_json::json!({"type":"minecraft:fixed","biome":biome});
        ov.set("world_presets", "minecraft:normal", preset.to_string());
    }
    let data = GeneratorData::load_with(&root.join("worldgen"), registry, &ov).unwrap();
    let seeds = std::env::var("HORIZON_SURFACE_SEEDS")
        .unwrap_or("1234,-9182,42".into())
        .split(',')
        .map(|s| s.parse::<i64>().unwrap())
        .collect::<Vec<_>>();
    for seed in seeds {
        let start = std::time::Instant::now();
        let generator = OverworldGenerator::for_dimension(
            data.clone(),
            seed,
            "minecraft:normal",
            "minecraft:overworld",
        )
        .unwrap();
        let mut worker = generator.worker();
        let mut saved_check = None;
        let mut check_at = None;
        let mut fitted = Vec::new();
        let mut total = 0;
        let mut types = BTreeMap::<String, usize>::new();
        for z in -80..=80 {
            for x in -80..=80 {
                let pos = ChunkPos::new(x, z);
                for s in worker.structure_starts(pos) {
                    total += 1;
                    *types.entry(s.name.clone()).or_default() += 1;
                    if let Some(p) = &s.surface {
                        let wanted = std::env::var("HORIZON_SURFACE_TEST_STRUCTURE").ok();
                        if wanted.as_ref().is_some_and(|id| *id == s.name)
                            || wanted.is_none()
                                && (check_at.is_none() || s.name.contains("village"))
                        {
                            saved_check = Some(s.clone());
                            let patch = &p.patches[0];
                            check_at = Some(ChunkPos::new(patch.min[0] >> 4, patch.min[1] >> 4));
                        }
                        assert_eq!(s.chunk, pos);
                        let persisted = serde_json::to_string(p).unwrap();
                        let loaded: horizon_world::worldgen::structure::surface_terrain::Plan =
                            serde_json::from_str(&persisted).unwrap();
                        assert_eq!(persisted, serde_json::to_string(&loaded).unwrap());
                        fitted.push(serde_json::json!({"id":s.name,"owner":[x,z],"site":p.site,"patches":p.patches.len()}));
                    }
                }
            }
        }
        if let Ok(expected) = std::env::var("HORIZON_SURFACE_TEST_STRUCTURE") {
            if !disabled {
                assert!(
                    fitted.iter().any(|v| v["id"] == expected),
                    "expected fitted structure {expected}"
                );
            }
        }
        let mut generated_hashes = Vec::new();
        if std::env::var_os("HORIZON_SURFACE_CHECK_CHUNKS").is_some() {
            if let Some(saved) = saved_check {
                use horizon_world::nbt::Tag;
                let before = saved.save().expect("save selected start");
                let Tag::Compound(map) = &before else {
                    panic!("start compound")
                };
                let Some(Tag::List(children)) = map.get(&"Children".into()) else {
                    panic!("saved children")
                };
                let g = OverworldGenerator::for_dimension(
                    data.clone(),
                    seed,
                    "minecraft:normal",
                    "minecraft:overworld",
                )
                .unwrap();
                g.set_old_world(std::sync::Arc::new(SavedWorld {
                    owner: saved.chunk,
                    start: horizon_world::worldgen::blending::SavedStart {
                        name: saved.name.clone(),
                        bounds: Some([
                            saved.bounding_box.min[0],
                            saved.bounding_box.min[1],
                            saved.bounding_box.min[2],
                            saved.bounding_box.max[0],
                            saved.bounding_box.max[1],
                            saved.bounding_box.max[2],
                        ]),
                        pieces: children.clone(),
                    },
                }));
                let restored = g
                    .worker()
                    .structure_starts(saved.chunk)
                    .into_iter()
                    .find(|s| s.name == saved.name)
                    .expect("restore saved start");
                assert_eq!(
                    before,
                    restored.save().expect("save restored start"),
                    "saved start changed on reload"
                );
            }
            if let Some(at) = check_at {
                let positions = [at, ChunkPos::new(at.x + 1, at.z)];
                let mut snapshots = Vec::new();
                for reversed in [false, true] {
                    let g = OverworldGenerator::for_dimension(
                        data.clone(),
                        seed,
                        "minecraft:normal",
                        "minecraft:overworld",
                    )
                    .unwrap();
                    let mut w = g.worker();
                    let mut hashes = BTreeMap::new();
                    for index in if reversed { [1, 0] } else { [0, 1] } {
                        let p = positions[index];
                        let c = w.generate_proto(p).unwrap();
                        let mut hash = 0xcbf29ce484222325u64;
                        for y in c.terrain.min_y()..c.terrain.min_y() + c.terrain.height() {
                            for z in 0..16 {
                                for x in 0..16 {
                                    hash = (hash ^ u64::from(c.block(x, y, z).0))
                                        .wrapping_mul(0x100000001b3);
                                }
                            }
                        }
                        hashes.insert(index, hash);
                    }
                    snapshots.push(hashes);
                }
                assert_eq!(
                    snapshots[0], snapshots[1],
                    "cross-chunk generation order changed terrain"
                );
                generated_hashes = snapshots.remove(0).into_values().collect();
            }
        }
        println!(
            "{}",
            serde_json::json!({"seed":seed,"starts":total,"types":types,"fitted":fitted,"chunk_hashes":generated_hashes,"seconds":start.elapsed().as_secs_f64()})
        );
    }
}

struct SavedWorld {
    owner: ChunkPos,
    start: horizon_world::worldgen::blending::SavedStart,
}
impl horizon_world::worldgen::blending::OldWorld for SavedWorld {
    fn is_old_chunk_around(&self, c: ChunkPos, r: i32) -> bool {
        (c.x - self.owner.x).abs() <= r && (c.z - self.owner.z).abs() <= r
    }
    fn blending(
        &self,
        _: i32,
        _: i32,
    ) -> Option<std::sync::Arc<horizon_world::worldgen::blending::BlendingData>> {
        None
    }
    fn old_biomes(
        &self,
        _: i32,
        _: i32,
    ) -> Option<
        std::sync::Arc<
            horizon_world::worldgen::biome_chunk::BiomeChunk<
                horizon_world::worldgen::biome::BiomeId,
            >,
        >,
    > {
        None
    }
    fn saved_starts(&self, x: i32, z: i32) -> Vec<horizon_world::worldgen::blending::SavedStart> {
        if self.owner == ChunkPos::new(x, z) {
            vec![self.start.clone()]
        } else {
            vec![]
        }
    }
}
