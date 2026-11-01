//! Headless content/worldgen smoke check. Uses only the supplied scratch directory.
//! worldgen_smoke PREPARED SCRATCH MOD_OR_DASH TECTONIC_BOOL [FIXED_BIOME]
use horizon_client::{
    assets::Assets,
    biome::Biomes,
    mod_content::{self, ContentMod},
    streaming::{ChunkGenerator, Proto},
};
use horizon_world::worldgen::{
    generator::{GeneratorData, OverworldGenerator},
    overrides::WorldgenOverrides,
};
use std::{path::Path, sync::Arc};
fn main() -> horizon_client::Result<()> {
    let a: Vec<_> = std::env::args().skip(1).collect();
    if a.get(3).is_some_and(|s| s == "matrix") {
        for (content, tectonic) in [
            (false, false),
            (false, true),
            (true, false),
            (true, true),
            (false, false),
        ] {
            let mut input = a.clone();
            input[2] = if content { a[2].clone() } else { "-".into() };
            input[3] = tectonic.to_string();
            println!("COMBINATION content={content} tectonic={tectonic}");
            check(&input)?;
        }
        return Ok(());
    }
    check(&a)
}
fn check(a: &[String]) -> horizon_client::Result<()> {
    let base = Path::new(&a[0]);
    let scratch = Path::new(&a[1]);
    std::fs::create_dir_all(scratch)?;
    let mut mods = Vec::new();
    if a[2] != "-" {
        let p = Path::new(&a[2]);
        let package = horizon_mods::Package::read(p)?;
        mods.push(ContentMod {
            id: package.manifest.id.clone(),
            content: horizon_mods::Content::read(p, &package.manifest)?,
        });
    }
    let started = std::time::Instant::now();
    let root = mod_content::resource_root(base, &mods, &scratch.join("cache"))?;
    let assets = Assets::load(&root)?;
    let biomes = Biomes::load(&root)?;
    let mut ov = WorldgenOverrides::default();
    ov.set_mods([("tectonic_horizon".into(), a[3] == "true")].into());
    ov.activate();
    if let Some(id) = a.get(4) {
        let dimension = serde_json::json!({"type":"minecraft:overworld","generator":{"type":"minecraft:noise","settings":"minecraft:overworld","biome_source":{"type":"minecraft:fixed","biome":id}}});
        ov.add_file("smoke", "data/minecraft/dimension/overworld.json", || {
            Some(dimension.to_string())
        });
    }
    let data = GeneratorData::load_with(&root.join("worldgen"), assets.registry, &ov)?;
    println!(
        "load {:.2}s, {} biomes, {} states",
        started.elapsed().as_secs_f64(),
        biomes.list.len(),
        assets.registry.len()
    );
    for w in data.warnings() {
        println!("warning: {w}");
    }
    for dimension in [
        "minecraft:overworld",
        "minecraft:the_nether",
        "minecraft:the_end",
    ] {
        let g = OverworldGenerator::for_dimension(
            data.clone(),
            8448363262698333288,
            "minecraft:normal",
            dimension,
        )?;
        let unsupported = g.unsupported_features();
        assert!(unsupported.is_empty(), "{dimension}: {unsupported:?}");
        let mut worker = g.worker();
        let points: Vec<_> = (0..1024)
            .map(|i| [(i % 32 - 16) * 512, 16, (i / 32 - 16) * 512])
            .collect();
        let samples = worker.noise_biomes(&points);
        let bop = samples
            .iter()
            .flatten()
            .filter(|b| b.starts_with("biomesoplenty:"))
            .count();
        println!(
            "{dimension}: BOP {bop}/1024, unsupported {}",
            unsupported.len()
        );
        if !mods.is_empty() && a.get(4).is_none() {
            assert!(bop > 0, "BOP must be reachable in {dimension}");
        }
    }
    let terrain = horizon_client::vanillagen::VanillaTerrain::for_world_with(
        &root.join("worldgen"),
        &ov,
        assets.registry,
        &biomes,
        8448363262698333288,
        "minecraft:normal",
        "minecraft:overworld",
        true,
    )?;
    let mut worker = terrain.worker();
    let mut region: [Proto; 9] =
        std::array::from_fn(|i| terrain.proto(&mut worker, i as i32 % 3 - 1, i as i32 / 3 - 1));
    terrain.decorate(&mut worker, &mut region);
    let [_, _, _, _, center, ..] = region;
    let chunk = terrain.finish(center);
    let save = horizon_client::save::SaveDir::new(
        scratch.join("roundtrip"),
        horizon_client::chunk::LevelShape::OVERWORLD,
    );
    let chunk = Arc::new(chunk);
    save.save(&[chunk.clone()], assets.registry, &biomes)?;
    let loaded = save
        .load(0, 0, assets.registry, &biomes)
        .ok_or("chunk did not reopen")?;
    let mut custom = 0;
    let mut hash = 0xcbf29ce484222325u64;
    for y in chunk.min_y()..chunk.max_y() {
        for z in 0..16 {
            for x in 0..16 {
                let b = chunk.get(x, y, z);
                assert_eq!(b, loaded.get(x, y, z));
                hash = (hash ^ b as u64).wrapping_mul(0x100000001b3);
                custom += usize::from(
                    assets
                        .registry
                        .state(horizon_world::registry::BlockStateId(b))
                        .is_some_and(|s| s.name.starts_with("biomesoplenty:")),
                );
            }
        }
    }
    for y in (-64..320).step_by(4) {
        assert_eq!(
            biomes.get(chunk.biome(0, y, 0)).name,
            biomes.get(loaded.biome(0, y, 0)).name
        );
    }
    println!(
        "PASS blocks={hash:016x} custom_blocks={custom} total_seconds={:.2}",
        started.elapsed().as_secs_f64()
    );
    Ok(())
}
