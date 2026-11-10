//! Additive biome placement in an existing multi-noise preset. Contributions
//! change the candidate pool, never the generator, router or sampling scale.
use super::{DecodeError, climate::decode_parameter_point, density::identifier};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// Assemble namespaced declarations in key order, independently of pack/mod
/// load order. A declaration uses vanilla's six climate intervals and offset.
/// All references are checked before publishing the immutable preset snapshot.
pub fn extend_presets(
    presets: &mut BTreeMap<String, String>,
    declarations: &BTreeMap<String, String>,
    biomes: &BTreeSet<String>,
) -> Result<(), DecodeError> {
    let mut scales = BTreeMap::<String, u64>::new();
    for text in declarations.values() {
        let doc: Value = serde_json::from_str(text)?;
        let divisor = doc["weight_divisor"].as_u64().unwrap_or(1);
        if divisor == 0 || divisor > 16 || !divisor.is_power_of_two() {
            return Err(DecodeError::Field("region weight divisor"));
        }
        let preset = doc["preset"].as_str().unwrap_or_default().to_owned();
        scales.entry(preset).and_modify(|v| *v = (*v).max(divisor)).or_insert(divisor);
    }
    let mut assembled = presets.clone();
    for (name, text) in declarations {
        identifier(name)?;
        let doc: Value = serde_json::from_str(text)?;
        let preset = identifier(
            doc["preset"]
                .as_str()
                .ok_or(DecodeError::Field("placement preset"))?,
        )?;
        let original = assembled
            .get(&preset)
            .ok_or(DecodeError::Field("unknown placement preset"))?;
        let mut table: Vec<Value> = serde_json::from_str(original)?;
        let entries = doc["biomes"]
            .as_array()
            .ok_or(DecodeError::Field("placement biomes"))?;
        if entries.len() > 65536 || table.len().saturating_add(entries.len()) > 65536 {
            return Err(DecodeError::Field("biome placement table size"));
        }
        for entry in entries {
            let biome = identifier(
                entry["biome"]
                    .as_str()
                    .ok_or(DecodeError::Field("placement biome"))?,
            )?;
            if !biomes.contains(&biome) {
                return Err(DecodeError::Registry(format!(
                    "placement {name}: unknown biome {biome}"
                )));
            }
            let point = decode_parameter_point(&entry["parameters"].to_string())?;
            let mut candidate = json!({
                "biome": biome,
                "coordinates": point.coordinates.map(|p| [p.min, p.max]),
                "offset": point.offset,
                "horizon:additive": true
            });
            if let Some(group) = doc["group"].as_str() {
                identifier(group)?;
                let weight = doc["weight"].as_f64().unwrap_or(1.0);
                if !weight.is_finite()
                    || weight < 0.0
                    || (weight == 0.0 && doc["region_order"].is_null())
                {
                    return Err(DecodeError::Field("biome group weight"));
                }
                let selector = doc["selector"].as_str().unwrap_or("weirdness");
                let axis = [
                    "temperature",
                    "humidity",
                    "continentalness",
                    "erosion",
                    "depth",
                    "weirdness",
                ]
                .iter()
                .position(|name| *name == selector)
                .ok_or(DecodeError::Field("biome pool selector"))?;
                candidate["horizon:selector_axis"] = json!(axis);
                candidate["horizon:group"] = json!(group);
                let scale = scales[&preset];
                let divisor = doc["weight_divisor"].as_u64().unwrap_or(1);
                candidate["horizon:weight"] = json!(weight * scale as f64 / divisor as f64);
                if !doc["region_order"].is_null() {
                    for key in ["region_order", "region_zooms", "vanilla_weight"] {
                        candidate[format!("horizon:{key}")] = if key == "vanilla_weight" {
                            json!(doc[key].as_u64().ok_or(DecodeError::Field("vanilla region weight"))?
                                .checked_mul(scale).ok_or(DecodeError::Field("region weight overflow"))?)
                        } else { doc[key].clone() };
                    }
                }
            }
            table.push(candidate);
        }
        assembled.insert(preset, serde_json::to_string(&table)?);
    }
    *presets = assembled;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worldgen::{biome::MultiNoiseBiomeSource, climate::TargetPoint};
    fn contribution(biome: &str, temperature: f32) -> String {
        json!({"preset":"minecraft:overworld", "biomes":[{
            "biome":biome, "parameters":{
                "temperature":temperature,"humidity":[-1,1],"continentalness":[-1,1],
                "erosion":[-1,1],"depth":[-1,1],"weirdness":[-1,1],"offset":0
            }
        }]})
        .to_string()
    }
    fn base() -> BTreeMap<String, String> {
        [("minecraft:overworld".into(), json!([{
            "biome":"minecraft:plains", "coordinates":[[0,0],[-10000,10000],[-10000,10000],[-10000,10000],[-10000,10000],[-10000,10000]], "offset":0
        }]).to_string())].into()
    }
    #[test]
    fn quarter_weight_region_preserves_other_ratios_and_zero_disables_it() {
        for extra in [0, 1] {
            let names = ["minecraft:plains", "example:rare", "example:extra"];
            let mut files = BTreeMap::new();
            for (i, weight, divisor) in [(1, 2, 1), (2, extra, 4)] {
                let mut d: Value = serde_json::from_str(&contribution(names[i], 0.)).unwrap();
                for (key, value) in [("group", json!(names[i])), ("weight", json!(weight)),
                    ("weight_divisor", json!(divisor)), ("region_order", json!(i)),
                    ("region_zooms", json!(3)), ("vanilla_weight", json!(10))] { d[key] = value; }
                files.insert(names[i].into(), d.to_string());
            }
            let mut presets = base();
            extend_presets(&mut presets, &files, &names.map(String::from).into()).unwrap();
            let entries: Vec<Value> = serde_json::from_str(&presets["minecraft:overworld"]).unwrap();
            let rare = entries.iter().find(|e| e["horizon:group"] == "example:rare").unwrap();
            let extra_entry = entries.iter().find(|e| e["horizon:group"] == "example:extra").unwrap();
            assert_eq!(rare["horizon:weight"], json!(8.0));
            assert_eq!(extra_entry["horizon:weight"], json!(extra as f64));
            assert_eq!(rare["horizon:vanilla_weight"], json!(40));
            MultiNoiseBiomeSource::from_quantized_json(&presets["minecraft:overworld"]).unwrap();
        }
    }
    #[test]
    fn java_region_choices_match_scalar_and_prepared_biomes() {
        use crate::worldgen::climate::ClimateSampler;
        use horizon_core::density::{ConstantSampler, DensitySampler, SamplerContext};
        use std::sync::Arc;
        let names = [
            "minecraft:plains",
            "example:primary",
            "example:secondary",
            "example:rare",
        ];
        let mut files = BTreeMap::new();
        let mut presets = base();
        for (i, w) in [(1, 10), (2, 8), (3, 2)] {
            let mut d: Value = serde_json::from_str(&contribution(names[i], 0.)).unwrap();
            d["group"] = json!(names[i]);
            d["weight"] = json!(w);
            d["region_order"] = json!(i);
            d["region_zooms"] = json!(3);
            d["vanilla_weight"] = json!(10);
            files.insert(names[i].into(), d.to_string());
        }
        extend_presets(&mut presets, &files, &names.map(String::from).into()).unwrap();
        let source =
            MultiNoiseBiomeSource::from_quantized_json(&presets["minecraft:overworld"]).unwrap();
        for seed in [0, 1, -1] {
            let climate = ClimateSampler::new(std::array::from_fn(|_| {
                Arc::new(ConstantSampler(0.)) as Arc<dyn DensitySampler>
            }))
            .with_region_seed(seed);
            let mut resolver = source.resolver(&climate);
            let mut context = SamplerContext::cached();
            let volume = resolver
                .prepare_volume(&mut context, [-16, 0, -16], [32, 1, 32])
                .unwrap();
            for line in include_str!("../../tests/fixtures/region_weights.csv")
                .lines()
                .rev()
            {
                let n: Vec<i64> = line.split(',').map(|s| s.parse().unwrap()).collect();
                if n[0] != seed || n[1] != 0 || n[2] < -16 || n[2] >= 16 || n[3] < -16 || n[3] >= 16
                {
                    continue;
                }
                let q = [n[2] as i32, 0, n[3] as i32];
                assert_eq!(
                    resolver.sample(&mut context, q).unwrap(),
                    names[n[4] as usize],
                    "{line}"
                );
                assert_eq!(
                    resolver.sample_prepared(&volume, q).unwrap(),
                    names[n[4] as usize],
                    "{line}"
                );
            }
        }
    }
    #[test]
    fn java_regions_include_vanilla_and_allow_zero_weight() {
        use crate::worldgen::climate::ClimateSampler;
        use horizon_core::density::{ConstantSampler, DensitySampler, SamplerContext};
        use std::sync::Arc;
        let mut presets = base();
        let mut doc: Value = serde_json::from_str(&contribution("example:cold", 0.)).unwrap();
        doc["group"] = json!("example:primary");
        doc["weight"] = json!(0);
        doc["region_order"] = json!(1);
        doc["region_zooms"] = json!(3);
        doc["vanilla_weight"] = json!(10);
        extend_presets(
            &mut presets,
            &[("example:primary".into(), doc.to_string())].into(),
            &["minecraft:plains".into(), "example:cold".into()].into(),
        )
        .unwrap();
        let source =
            MultiNoiseBiomeSource::from_quantized_json(&presets["minecraft:overworld"]).unwrap();
        let climate = ClimateSampler::new(std::array::from_fn(|_| {
            Arc::new(ConstantSampler(0.)) as Arc<dyn DensitySampler>
        }))
        .with_region_seed(123);
        let mut resolver = source.resolver(&climate);
        let mut context = SamplerContext::cached();
        let volume = resolver
            .prepare_volume(&mut context, [-8, 0, -8], [16, 1, 16])
            .unwrap();
        for x in -8..8 {
            for z in -8..8 {
                assert_eq!(
                    resolver.sample(&mut context, [x, 0, z]).unwrap(),
                    "minecraft:plains"
                );
                assert_eq!(
                    resolver.sample_prepared(&volume, [x, 0, z]).unwrap(),
                    "minecraft:plains"
                );
            }
        }
    }
    #[test]
    fn weighted_groups_use_host_selector_and_are_query_order_independent() {
        use crate::worldgen::climate::ClimateSampler;
        use horizon_core::density::{ConstantSampler, DensitySampler, SamplerContext};
        use std::sync::Arc;
        let mut presets = base();
        let mut declarations = BTreeMap::new();
        for (id, biome) in [("a", "example:cold"), ("b", "example:warm")] {
            let mut doc: Value = serde_json::from_str(&contribution(biome, 0.)).unwrap();
            doc["group"] = json!(format!("example:{id}"));
            doc["weight"] = json!(1);
            doc["selector"] = json!("humidity");
            declarations.insert(format!("example:{id}"), doc.to_string());
        }
        extend_presets(
            &mut presets,
            &declarations,
            &[
                "minecraft:plains".into(),
                "example:cold".into(),
                "example:warm".into(),
            ]
            .into(),
        )
        .unwrap();
        let source =
            MultiNoiseBiomeSource::from_quantized_json(&presets["minecraft:overworld"]).unwrap();
        for (h, expected) in [(-0.8, "example:cold"), (0.8, "example:warm")] {
            let fields: [Arc<dyn DensitySampler>; 6] = std::array::from_fn(|i| {
                Arc::new(ConstantSampler(if i == 1 { h } else { 0. })) as Arc<dyn DensitySampler>
            });
            let climate = ClimateSampler::new(fields);
            let mut resolver = source.resolver(&climate);
            let mut context = SamplerContext::cached();
            let volume = resolver
                .prepare_volume(&mut context, [-2, 0, -2], [4, 1, 4])
                .unwrap();
            for x in [-1, 0, 1, 0, -1] {
                assert_eq!(resolver.sample(&mut context, [x, 0, 0]).unwrap(), expected);
                assert_eq!(
                    resolver.sample_prepared(&volume, [x, 0, 0]).unwrap(),
                    expected
                );
            }
        }
    }
    #[test]
    fn contributions_are_additive_deterministic_and_use_host_climate() {
        let mut presets = base();
        let biomes = ["minecraft:plains", "example:frost", "other:tropical"]
            .map(String::from)
            .into();
        let files: BTreeMap<_, _> = [
            (
                "other:placement".into(),
                contribution("other:tropical", 0.8),
            ),
            (
                "example:placement".into(),
                contribution("example:frost", -0.8),
            ),
        ]
        .into();
        extend_presets(&mut presets, &files, &biomes).unwrap();
        let first = presets.clone();
        let mut repeated = base();
        extend_presets(&mut repeated, &files, &biomes).unwrap();
        assert_eq!(first, repeated);
        let source =
            MultiNoiseBiomeSource::from_quantized_json(&presets["minecraft:overworld"]).unwrap();
        let mut resolver = source.parameters().searcher();
        for (t, expected) in [
            (-0.8, "example:frost"),
            (0., "minecraft:plains"),
            (0.8, "other:tropical"),
        ] {
            assert_eq!(
                resolver
                    .find(TargetPoint::from_values([t, 0., 0., 0., 0., 0.]))
                    .unwrap(),
                expected
            );
        }
        let mut empty = base();
        let original = empty.clone();
        extend_presets(&mut empty, &BTreeMap::new(), &biomes).unwrap();
        assert_eq!(empty, original);
    }
    #[test]
    fn additions_win_matching_vanilla_boxes_for_scalar_and_bulk_queries() {
        use crate::worldgen::climate::ClimateSampler;
        use horizon_core::density::{ConstantSampler, DensitySampler, SamplerContext};
        use std::sync::Arc;
        let mut presets = base();
        // Vanilla boxes commonly already have zero distance. Appending a
        // second ordinary entry would leave the new biome hidden by ties.
        let mut table: Vec<Value> = serde_json::from_str(&presets["minecraft:overworld"]).unwrap();
        table[0]["coordinates"][0] = json!([-10000, 10000]);
        presets.insert(
            "minecraft:overworld".into(),
            serde_json::to_string(&table).unwrap(),
        );
        extend_presets(
            &mut presets,
            &[("example:frost".into(), contribution("example:frost", -0.8))].into(),
            &["minecraft:plains".into(), "example:frost".into()].into(),
        )
        .unwrap();
        let source =
            MultiNoiseBiomeSource::from_quantized_json(&presets["minecraft:overworld"]).unwrap();
        for (temperature, expected) in [(-0.8, "example:frost"), (0.0, "minecraft:plains")] {
            let fields: [Arc<dyn DensitySampler>; 6] = std::array::from_fn(|i| {
                Arc::new(ConstantSampler(if i == 0 { temperature } else { 0. }))
                    as Arc<dyn DensitySampler>
            });
            let climate = ClimateSampler::new(fields);
            let mut context = SamplerContext::cached();
            let mut resolver = source.resolver(&climate);
            let volume = resolver
                .prepare_volume(&mut context, [-4, 0, -4], [8, 1, 8])
                .unwrap();
            for x in -4..4 {
                assert_eq!(resolver.sample(&mut context, [x, 0, 0]).unwrap(), expected);
                assert_eq!(
                    resolver.sample_prepared(&volume, [x, 0, 0]).unwrap(),
                    expected
                );
            }
        }
    }

    #[test]
    fn overlapping_additions_do_not_depend_on_worker_query_order() {
        use crate::worldgen::climate::ClimateSampler;
        use horizon_core::density::{
            ConstantSampler, DensitySampler, GradientSampler, SamplerContext, TilingMode,
        };
        use horizon_core::geometry::Axis;
        use std::sync::Arc;
        let mut presets = base();
        let files = [
            ("a:placement".into(), contribution("a:forest", 0.)),
            ("b:placement".into(), contribution("b:forest", 0.)),
        ]
        .into();
        extend_presets(
            &mut presets,
            &files,
            &[
                "a:forest".into(),
                "b:forest".into(),
                "minecraft:plains".into(),
            ]
            .into(),
        )
        .unwrap();
        let source =
            MultiNoiseBiomeSource::from_quantized_json(&presets["minecraft:overworld"]).unwrap();
        let fields: [Arc<dyn DensitySampler>; 6] = std::array::from_fn(|i| {
            if i == 0 {
                Arc::new(GradientSampler::new(
                    Axis::X,
                    TilingMode::ClampToEdge,
                    -16,
                    16,
                    -1.,
                    1.,
                )) as Arc<dyn DensitySampler>
            } else {
                Arc::new(ConstantSampler(0.)) as Arc<dyn DensitySampler>
            }
        });
        let climate = ClimateSampler::new(fields);
        let mut forward = source.resolver(&climate);
        let mut backward = source.resolver(&climate);
        let mut context = SamplerContext::cached();
        let expected: Vec<_> = (-4..=4)
            .map(|x| forward.sample(&mut context, [x, 0, 0]).unwrap().to_string())
            .collect();
        for x in (-4..=4).rev() {
            assert_eq!(
                backward.sample(&mut context, [x, 0, 0]).unwrap(),
                expected[(x + 4) as usize]
            );
        }
    }

    #[test]
    fn bad_references_fail_without_partially_changing_presets() {
        let mut presets = base();
        let original = presets.clone();
        let files = [("example:test".into(), contribution("example:missing", 0.))].into();
        assert!(extend_presets(&mut presets, &files, &BTreeSet::new()).is_err());
        assert_eq!(presets, original);
        let invalid =
            contribution("example:test", 0.).replace("minecraft:overworld", "missing:preset");
        assert!(
            extend_presets(
                &mut presets,
                &[("example:test".into(), invalid)].into(),
                &["example:test".into()].into()
            )
            .is_err()
        );
        assert_eq!(presets, original);
    }
}
