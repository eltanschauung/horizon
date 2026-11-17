use super::*;
use horizon_core::density::{ConstantSampler, DensitySampler};
use std::sync::Arc;

#[derive(Debug)]
struct ThinStrip;
impl DensitySampler for ThinStrip {
    fn sample_value(&self, _: &mut SamplerContext, p: [i32; 3]) -> Result<f32, SampleError> {
        Ok(if p[0].rem_euclid(512) < 8 { 0. } else { 0.3 })
    }
}
fn fixture(special: &str, depth: f32) -> (MultiNoiseBiomeSource, ClimateSampler) {
    let mut entries = Vec::new();
    for (name, t, d) in [
        ("minecraft:plains", [-1., 1.], [-1., 1.]),
        ("test:forest", [-1., -0.1], [0., 0.]),
        (
            special,
            [-0.1, 0.1],
            if special == "test:underground" {
                [0.2, 0.9]
            } else {
                [0., 0.]
            },
        ),
        ("test:forest", [0.1, 1.], [0., 0.]),
    ] {
        let mut v = serde_json::json!({"biome":name,"coordinates":[t,[-1,1],[-1,1],[-1,1],d,[-1,1]],"offset":0});
        // The quantized codec stores integer climate coordinates.
        v["coordinates"] = serde_json::json!(
            [t, [-1., 1.], [-1., 1.], [-1., 1.], d, [-1., 1.]]
                .map(|range| range.map(|f| super::super::climate::quantize(f)))
        );
        if name != "minecraft:plains" {
            for (k, value) in [
                ("horizon:group", serde_json::json!("test:region")),
                ("horizon:weight", serde_json::json!(1)),
                ("horizon:region_order", serde_json::json!(1)),
                ("horizon:region_zooms", serde_json::json!(3)),
                ("horizon:vanilla_weight", serde_json::json!(0)),
                ("horizon:region_min_quarts", serde_json::json!(192)),
            ] {
                v[k] = value;
            }
        }
        entries.push(v);
    }
    let source =
        MultiNoiseBiomeSource::from_quantized_json(&serde_json::to_string(&entries).unwrap())
            .unwrap();
    let fields: [Arc<dyn DensitySampler>; 6] = std::array::from_fn(|i| {
        if i == 0 {
            Arc::new(ThinStrip) as _
        } else {
            Arc::new(ConstantSampler(if i == 4 { depth } else { 0. })) as _
        }
    });
    (source, ClimateSampler::new(fields).with_region_seed(51))
}
#[test]
fn thin_biome_strips_merge_across_scalar_and_prepared_worker_queries() {
    let (source, climate) = fixture("test:tiny_redwood", 0.);
    let mut scalar = source.resolver(&climate);
    let mut bulk = source.resolver(&climate);
    let mut context = SamplerContext::cached();
    let volume = bulk
        .prepare_volume(&mut context, [-4, 16, -4], [8, 2, 8])
        .unwrap();
    for x in -4..4 {
        for z in -4..4 {
            let q = [x, 16, z];
            assert_eq!(scalar.sample(&mut context, q).unwrap(), "test:forest");
            assert_eq!(bulk.sample_prepared(&volume, q).unwrap(), "test:forest");
        }
    }
    let mut reversed = source.resolver(&climate);
    for x in (-1000..1000).step_by(19).rev() {
        assert_eq!(
            scalar.sample(&mut context, [x, 16, -x]).unwrap(),
            reversed.sample(&mut context, [x, 16, -x]).unwrap()
        );
    }
}
#[test]
fn rivers_remain_local_and_underground_biomes_never_expand_into_surface_provinces() {
    let (source, climate) = fixture("minecraft:river", 0.);
    let mut resolver = source.resolver(&climate);
    let mut context = SamplerContext::cached();
    assert_eq!(
        resolver.sample(&mut context, [0, 16, 0]).unwrap(),
        "minecraft:river"
    );
    assert_eq!(
        resolver.sample(&mut context, [3, 16, 0]).unwrap(),
        "test:forest"
    );
    let (source, climate) = fixture("test:underground", 0.5);
    assert_eq!(
        source
            .resolver(&climate)
            .sample(&mut context, [0, 16, 0])
            .unwrap(),
        "test:underground"
    );
    let (mut source, climate) = fixture("test:lake", 0.);
    source.configure_landforms(&[("minecraft:is_ocean".into(), vec!["test:lake".into()])].into());
    assert_eq!(
        source
            .resolver(&climate)
            .sample(&mut context, [0, 16, 0])
            .unwrap(),
        "test:lake"
    );
}

#[test]
fn disabling_all_regional_packs_retains_the_original_vanilla_resolver() {
    let (source, climate) = fixture("test:tiny_redwood", 0.);
    let mut entries: Vec<_> = source.parameters.entries().iter().map(|(p,name)|
        serde_json::json!({"coordinates":p.coordinates.map(|c|[c.min,c.max]),"offset":p.offset,"biome":name})
    ).collect();
    for e in &mut entries {
        if e["biome"] != "minecraft:plains" {
            for (k, v) in [
                ("horizon:group", serde_json::json!("test:disabled")),
                ("horizon:weight", serde_json::json!(0)),
                ("horizon:region_order", serde_json::json!(1)),
                ("horizon:region_zooms", serde_json::json!(3)),
                ("horizon:vanilla_weight", serde_json::json!(1)),
                ("horizon:region_min_quarts", serde_json::json!(192)),
            ] {
                e[k] = v;
            }
        }
    }
    let source =
        MultiNoiseBiomeSource::from_quantized_json(&serde_json::to_string(&entries).unwrap())
            .unwrap();
    let mut resolver = source.resolver(&climate);
    assert!(resolver.surface.is_none());
    assert_eq!(
        resolver
            .sample(&mut SamplerContext::cached(), [0, 16, 0])
            .unwrap(),
        "minecraft:plains"
    );
}
