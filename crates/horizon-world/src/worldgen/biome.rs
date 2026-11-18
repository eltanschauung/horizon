//! Multi-noise biome resolution over an imported immutable parameter table.
//! Vanilla biome JSON codecs and built-in preset builders remain separate work.
use super::surface_regions::{Habitat, Province, SurfaceRegions};
use super::{
    DecodeError,
    climate::{
        ClimateIndex, ClimateSampler, ClimateSearch, ClimateVolume, IndexError, Parameter,
        ParameterPoint,
    },
    density::identifier,
};
use horizon_core::density::{SampleError, SamplerContext};
use std::collections::BTreeMap;

/// Identity within an immutable biome registry snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BiomeId(pub u32);

#[derive(Debug)]
pub enum BiomeError {
    Decode(DecodeError),
    Index(IndexError),
    Sample(SampleError),
    OutsideVolume,
}
impl std::fmt::Display for BiomeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Decode(e) => e.fmt(f),
            Self::Index(e) => e.fmt(f),
            Self::Sample(e) => e.fmt(f),
            Self::OutsideVolume => f.write_str("biome query is outside the prepared quart volume"),
        }
    }
}
impl std::error::Error for BiomeError {}
impl From<DecodeError> for BiomeError {
    fn from(e: DecodeError) -> Self {
        Self::Decode(e)
    }
}
impl From<IndexError> for BiomeError {
    fn from(e: IndexError) -> Self {
        Self::Index(e)
    }
}
impl From<SampleError> for BiomeError {
    fn from(e: SampleError) -> Self {
        Self::Sample(e)
    }
}

#[derive(Debug, Clone)]
struct RegionOptions {
    weights: Vec<u32>,
    zooms: u8,
    minimum: u32,
    scales: Vec<f64>,
}

#[derive(Debug)]
pub struct MultiNoiseBiomeSource {
    parameters: ClimateIndex<String>,
    /// Additive declarations win equal fitness explicitly, rather than being
    /// silently shadowed by vanilla's climate boxes or previous-leaf cache.
    additions: Option<ClimateIndex<String>>,
    baseline: Option<ClimateIndex<String>>,
    groups: Vec<(f64, ClimateIndex<String>)>,
    group_guards: Vec<bool>,
    selector_axis: usize,
    regions: Option<RegionOptions>,
    landforms: BTreeMap<String, Habitat>,
}
impl MultiNoiseBiomeSource {
    pub fn new(entries: Vec<(ParameterPoint, String)>) -> Result<Self, BiomeError> {
        let entries: Vec<(ParameterPoint, String)> = entries
            .into_iter()
            .map(|(point, name)| Ok((point, identifier(&name)?)))
            .collect::<Result<_, DecodeError>>()?;
        // Surface declarations include depth 0 or 1. A biome present only in
        // underground depth intervals must never become a surface province.
        let mut landforms = BTreeMap::new();
        for (point, name) in &entries {
            let depth = point.coordinates[4];
            let role = if depth.distance(0) == 0 || depth.distance(10000) == 0 {
                Habitat::Temperate
            } else {
                Habitat::Preserve
            };
            landforms
                .entry(name.clone())
                .and_modify(|v| {
                    if role == Habitat::Temperate {
                        *v = role;
                    }
                })
                .or_insert(role);
        }
        landforms.retain(|_, role| *role == Habitat::Preserve);
        Ok(Self {
            parameters: ClimateIndex::new(entries)?,
            additions: None,
            baseline: None,
            groups: Vec::new(),
            group_guards: Vec::new(),
            selector_axis: 5,
            regions: None,
            landforms,
        })
    }
    /// Horizon's quantized import format, not the vanilla biome definition
    /// codec: [{"coordinates":[[min,max]; six],"offset":long,"biome":id}].
    /// Built-in numeric tables stay private and are authenticated by the runner.
    pub fn from_quantized_json(json: &str) -> Result<Self, BiomeError> {
        let input: serde_json::Value = serde_json::from_str(json).map_err(DecodeError::from)?;
        let mut entries = Vec::new();
        let mut additions = Vec::new();
        let mut baseline = Vec::new();
        let mut selector_axis = None;
        let mut region_options = None;
        let mut scales = std::collections::BTreeMap::new();
        let mut orders = std::collections::BTreeMap::new();
        let mut guards = std::collections::BTreeMap::new();
        let mut groups =
            std::collections::BTreeMap::<String, (f64, Vec<(ParameterPoint, String)>)>::new();
        for entry in input
            .as_array()
            .ok_or(DecodeError::Field("biome parameter list"))?
        {
            let raw = entry["coordinates"]
                .as_array()
                .filter(|v| v.len() == 6)
                .ok_or(DecodeError::Field("climate coordinates"))?;
            let mut coordinates = [Parameter::new(0, 0); 6];
            for (coordinate, raw) in coordinates.iter_mut().zip(raw) {
                let raw = raw
                    .as_array()
                    .filter(|v| v.len() == 2)
                    .ok_or(DecodeError::Field("climate interval"))?;
                *coordinate = Parameter::new(
                    raw[0]
                        .as_i64()
                        .ok_or(DecodeError::Field("climate minimum"))?,
                    raw[1]
                        .as_i64()
                        .ok_or(DecodeError::Field("climate maximum"))?,
                );
            }
            let offset = entry["offset"]
                .as_i64()
                .ok_or(DecodeError::Field("climate offset"))?;
            let biome = entry["biome"]
                .as_str()
                .ok_or(DecodeError::Field("biome identifier"))?;
            let point = ParameterPoint {
                coordinates,
                offset,
            };
            if let Some(group) = entry["horizon:group"].as_str() {
                let axis = entry["horizon:selector_axis"].as_u64().unwrap_or(5) as usize;
                if axis > 5 || selector_axis.is_some_and(|a| a != axis) {
                    return Err(DecodeError::Field("biome pool selector mismatch").into());
                }
                selector_axis = Some(axis);
                let weight = entry["horizon:weight"].as_f64().unwrap_or(1.0);
                let region = entry["horizon:region_order"].as_u64();
                if let Some(order) = region {
                    let zooms = entry["horizon:region_zooms"]
                        .as_u64()
                        .filter(|v| *v <= 6)
                        .ok_or(DecodeError::Field("region zooms"))?
                        as u8;
                    let vanilla = entry["horizon:vanilla_weight"]
                        .as_u64()
                        .filter(|v| *v <= i32::MAX as u64)
                        .ok_or(DecodeError::Field("vanilla region weight"))?
                        as u32;
                    let scale = entry["horizon:region_scale"].as_f64().unwrap_or(1.0);
                    if !(1.0..=2.0).contains(&scale)
                        || scales
                            .insert(group.to_owned(), scale)
                            .is_some_and(|old| old != scale)
                    {
                        return Err(DecodeError::Field("inconsistent biome region scale").into());
                    }
                    let minimum = entry["horizon:region_min_quarts"].as_u64().unwrap_or(0);
                    if minimum > 4096 {
                        return Err(DecodeError::Field("region minimum size").into());
                    }
                    let minimum = minimum as u32;
                    if order == 0
                        || order > 65535
                        || region_options.is_some_and(|v| v != (vanilla, zooms, minimum))
                    {
                        return Err(DecodeError::Field("inconsistent region declarations").into());
                    }
                    region_options = Some((vanilla, zooms, minimum));
                    if let Some(previous) = orders.insert(group.to_owned(), order) {
                        if previous != order {
                            return Err(DecodeError::Field("inconsistent region order").into());
                        }
                    }
                    if weight.fract() != 0.0 || weight > i32::MAX as f64 {
                        return Err(DecodeError::Field("integer region weight").into());
                    }
                }
                if !weight.is_finite() || weight < 0.0 || (weight == 0.0 && region.is_none()) {
                    return Err(DecodeError::Field("biome group weight").into());
                }
                let guard = entry["horizon:climate_guard"].as_bool().unwrap_or(false);
                if guards
                    .insert(group.to_owned(), guard)
                    .is_some_and(|previous| previous != guard)
                {
                    return Err(DecodeError::Field("inconsistent climate guard").into());
                }
                let group = groups
                    .entry(group.to_owned())
                    .or_insert_with(|| (weight, Vec::new()));
                if group.0 != weight {
                    return Err(DecodeError::Field("inconsistent region weight").into());
                }
                group.1.push((point, identifier(biome)?));
            } else if entry["horizon:additive"].as_bool() == Some(true) {
                additions.push((point, identifier(biome)?));
            } else {
                baseline.push((point, identifier(biome)?));
            }
            entries.push((point, biome.to_owned()));
        }
        let mut source = Self::new(entries)?;
        if !additions.is_empty() {
            source.additions = Some(ClimateIndex::new(additions)?);
        }
        if (!source.additions.is_none() || !groups.is_empty()) && !baseline.is_empty() {
            source.baseline = Some(ClimateIndex::new(baseline)?);
        }
        source.selector_axis = selector_axis.unwrap_or(5);
        let mut groups: Vec<_> = groups.into_iter().collect();
        if let Some((vanilla, zooms, minimum)) = region_options {
            if orders.len() != groups.len()
                || orders
                    .values()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    != orders.len()
            {
                return Err(DecodeError::Field("mixed or duplicate region orders").into());
            }
            groups.sort_by_key(|(id, _)| orders[id]);
            let weights: Vec<_> = std::iter::once(vanilla)
                .chain(groups.iter().map(|(_, (w, _))| *w as u32))
                .collect();
            super::region_weights::RegionWeights::new(0, weights.clone(), zooms)
                .map_err(DecodeError::Field)?;
            let scales = std::iter::once(1.0)
                .chain(groups.iter().map(|(id, _)| scales[id]))
                .collect();
            source.regions = Some(RegionOptions {
                weights,
                zooms,
                minimum,
                scales,
            });
        }
        for (name, (weight, points)) in groups {
            source.group_guards.push(guards[&name]);
            source.groups.push((weight, ClimateIndex::new(points)?));
        }
        Ok(source)
    }
    pub fn parameters(&self) -> &ClimateIndex<String> {
        &self.parameters
    }
    /// Respect the registry's geography, including modded lakes, shores and
    /// mountains. Ordinary forests/fields share one temperate habitat.
    pub(super) fn configure_landforms(&mut self, tags: &BTreeMap<String, Vec<String>>) {
        for (tag, habitat) in [
            ("minecraft:is_mountain", Habitat::Highland),
            ("c:is_mountain", Habitat::Highland),
            ("minecraft:is_ocean", Habitat::Preserve),
            ("minecraft:is_beach", Habitat::Preserve),
            ("minecraft:is_river", Habitat::Preserve),
            ("c:is_ocean", Habitat::Preserve),
            ("c:is_beach", Habitat::Preserve),
            ("c:is_river", Habitat::Preserve),
            ("c:is_underground", Habitat::Preserve),
        ] {
            for biome in tags.get(tag).into_iter().flatten() {
                self.landforms.insert(biome.clone(), habitat);
            }
        }
    }
    pub fn resolver<'a>(&'a self, climate: &'a ClimateSampler) -> BiomeResolver<'a> {
        BiomeResolver {
            surface: self
                .regions
                .as_ref()
                .filter(|region| {
                    region.minimum > 0 && region.weights.iter().skip(1).any(|&w| w > 0)
                })
                .map(|region| SurfaceRegions::new(climate.region_seed, region.minimum)),
            landforms: &self.landforms,
            climate,
            search: self
                .baseline
                .as_ref()
                .unwrap_or(&self.parameters)
                .searcher(),
            additions: self.additions.as_ref().map(ClimateIndex::searcher),
            regions: self.regions.as_ref().map(|region| {
                super::region_weights::RegionWeights::new(
                    climate.region_seed,
                    region.weights.clone(),
                    region.zooms,
                )
                .expect("validated region weights")
                .with_min_quarts(climate.region_seed, region.minimum)
                .with_scales(&region.scales)
            }),
            selector_axis: self.selector_axis,
            group_guards: self.group_guards.clone(),
            groups: self
                .groups
                .iter()
                .map(|(weight, index)| (*weight, index.searcher()))
                .collect(),
        }
    }
}
#[derive(Debug)]
pub struct BiomeResolver<'a> {
    surface: Option<SurfaceRegions<'a>>,
    landforms: &'a BTreeMap<String, Habitat>,
    regions: Option<super::region_weights::RegionWeights>,
    climate: &'a ClimateSampler,
    search: ClimateSearch<'a, String>,
    additions: Option<ClimateSearch<'a, String>>,
    groups: Vec<(f64, ClimateSearch<'a, String>)>,
    group_guards: Vec<bool>,
    selector_axis: usize,
}
impl<'a> BiomeResolver<'a> {
    pub fn sample(
        &mut self,
        context: &mut SamplerContext,
        quart: [i32; 3],
    ) -> Result<&'a str, BiomeError> {
        self.resolve(self.climate.sample(context, quart)?, quart)
    }
    pub fn prepare_volume(
        &self,
        context: &mut SamplerContext,
        minimum: [i32; 3],
        extents: [i32; 3],
    ) -> Result<ClimateVolume, BiomeError> {
        Ok(self.climate.sample_volume(context, minimum, extents)?)
    }
    /// Query order shares the same previous-leaf state as scalar resolution.
    pub fn sample_prepared(
        &mut self,
        volume: &ClimateVolume,
        quart: [i32; 3],
    ) -> Result<&'a str, BiomeError> {
        let target = volume.target(quart).ok_or(BiomeError::OutsideVolume)?;
        self.resolve(target, quart)
    }
    fn resolve(
        &mut self,
        target: super::climate::TargetPoint,
        quart: [i32; 3],
    ) -> Result<&'a str, BiomeError> {
        let stable = self.surface.is_some();
        let raw = self.resolve_original(target, quart, stable)?;
        if !stable {
            return Ok(raw);
        }
        let (habitat, raw) = self.surface_habitat(target, raw)?;
        if habitat == Habitat::Preserve {
            return Ok(raw);
        }
        let sites = self.surface.as_ref().unwrap().sites(quart[0], quart[2]);
        for site in sites {
            let province = if let Some(&p) = self.surface.as_ref().unwrap().cache.get(&site.cell) {
                p
            } else {
                let span = self.surface.as_ref().unwrap().span;
                let mut samples = Vec::with_capacity(9);
                for dz in [-1, 0, 1] {
                    for dx in [-1, 0, 1] {
                        let q = [
                            site.position[0].wrapping_add(dx * span / 3),
                            16,
                            site.position[1].wrapping_add(dz * span / 3),
                        ];
                        let mut t = self
                            .climate
                            .sample(&mut self.surface.as_mut().unwrap().context, q)?;
                        // Survey the surface palette, not a cave at a fixed world Y.
                        t.depth = 0;
                        let name = self.resolve_original(t, q, true)?;
                        let (habitat, name) = self.surface_habitat(t, name)?;
                        samples.push((habitat, name, t));
                    }
                }
                let regions = self.surface.as_mut().unwrap();
                let province = Province::census(regions.seed, site.cell, &samples);
                regions.insert(site, province);
                province
            };
            if let Some(biome) = province.choose(habitat, target) {
                return Ok(biome);
            }
        }
        // Small islands or exceptional climate/height niches may have no
        // supported nearby province of their habitat; retain their real biome.
        Ok(raw)
    }
    /// A regional pool can select a land biome even where the host climate
    /// selects a river (a broad land entry can tie a narrow river entry).
    /// Honor the host's river at the actual depth before assigning a province
    /// or counting votes. Explicit coasts, waters and caves keep their rules.
    fn surface_habitat(
        &mut self,
        target: super::climate::TargetPoint,
        biome: &'a str,
    ) -> Result<(Habitat, &'a str), BiomeError> {
        let local = Habitat::vanilla(biome);
        let tagged = self.landforms.get(biome).copied();
        if local == Habitat::Preserve || tagged == Some(Habitat::Preserve) {
            return Ok((Habitat::Preserve, biome));
        }
        let baseline = self.search.find_entry_stable(target)?.1.as_str();
        if matches!(baseline, "minecraft:river" | "minecraft:frozen_river") {
            return Ok((Habitat::Preserve, baseline));
        }
        // Placement climate identifies a mod biome's ecology, even when its
        // displayed temperature differs (for example, a lush desert).
        let habitat = tagged.unwrap_or_else(|| {
            if biome.starts_with("minecraft:") {
                local
            } else {
                Habitat::vanilla(baseline)
            }
        });
        Ok((habitat, biome))
    }
    fn resolve_original(
        &mut self,
        target: super::climate::TargetPoint,
        quart: [i32; 3],
        stable: bool,
    ) -> Result<&'a str, BiomeError> {
        if let Some(regions) = &mut self.regions {
            let index = regions.sample(quart[0], quart[2]);
            if index == 0 {
                return Ok(if stable {
                    self.search.find_entry_stable(target)?
                } else {
                    self.search.find_entry(target)?
                }
                .1
                .as_str());
            }
            let search = &mut self.groups[index - 1].1;
            let (candidate, biome) = if stable {
                search.find_entry_stable(target)?
            } else {
                search.find_entry(target)?
            };
            if self.group_guards[index - 1] {
                let (baseline, vanilla) = if stable {
                    self.search.find_entry_stable(target)?
                } else {
                    self.search.find_entry(target)?
                };
                if candidate.fitness(target) > baseline.fitness(target) {
                    return Ok(vanilla.as_str());
                }
            }
            return Ok(biome.as_str());
        }
        let (point, name) = self.search.find_entry(target)?;
        if !self.groups.is_empty() {
            let total: f64 = self.groups.iter().map(|(weight, _)| weight).sum();
            // Reuse the host's coherent climate field. No coordinate hash or
            // independent biome-size noise is introduced by a content pack.
            let selector =
                (target.parameters()[self.selector_axis] as f64 / 5000.0).tanh() * 0.5 + 0.5;
            let mut remaining = selector * total;
            let last = self.groups.len() - 1;
            for (i, (weight, search)) in self.groups.iter_mut().enumerate() {
                if remaining < *weight || i == last {
                    let (candidate, biome) = search.find_entry_stable(target)?;
                    if candidate.fitness(target) <= point.fitness(target) {
                        return Ok(biome.as_str());
                    }
                    break;
                }
                remaining -= *weight;
            }
        }
        if let Some(additions) = &mut self.additions {
            let (added_point, added_name) = additions.find_entry_stable(target)?;
            if added_point.fitness(target) <= point.fitness(target) {
                return Ok(added_name.as_str());
            }
        }
        Ok(name.as_str())
    }
}

#[cfg(test)]
mod coherence_tests;
