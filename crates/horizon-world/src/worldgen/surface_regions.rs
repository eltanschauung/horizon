//! Shared ecological provinces for opt-in regional biome packs. A province
//! chooses a supported biome for each climate/landform family, across providers.
//! Votes come from the original resolver, never from already smoothed neighbours.
use super::climate::TargetPoint;
use horizon_core::density::SamplerContext;
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Habitat {
    Temperate,
    Arid,
    Tropical,
    Cold,
    Wetland,
    Highland,
    Preserve,
}
impl Habitat {
    pub(super) const COUNT: usize = 6;
    pub(super) fn vanilla(name: &str) -> Self {
        match name {
            "minecraft:desert"
            | "minecraft:badlands"
            | "minecraft:eroded_badlands"
            | "minecraft:wooded_badlands"
            | "minecraft:savanna"
            | "minecraft:savanna_plateau"
            | "minecraft:windswept_savanna" => Self::Arid,
            "minecraft:jungle" | "minecraft:bamboo_jungle" | "minecraft:sparse_jungle" => {
                Self::Tropical
            }
            "minecraft:snowy_plains"
            | "minecraft:ice_spikes"
            | "minecraft:snowy_taiga"
            | "minecraft:taiga"
            | "minecraft:old_growth_spruce_taiga"
            | "minecraft:old_growth_pine_taiga" => Self::Cold,
            "minecraft:swamp" | "minecraft:mangrove_swamp" => Self::Wetland,
            "minecraft:meadow"
            | "minecraft:cherry_grove"
            | "minecraft:grove"
            | "minecraft:windswept_hills"
            | "minecraft:windswept_forest"
            | "minecraft:windswept_gravelly_hills" => Self::Highland,
            "minecraft:river"
            | "minecraft:frozen_river"
            | "minecraft:beach"
            | "minecraft:snowy_beach"
            | "minecraft:stony_shore"
            | "minecraft:ocean"
            | "minecraft:deep_ocean"
            | "minecraft:cold_ocean"
            | "minecraft:deep_cold_ocean"
            | "minecraft:frozen_ocean"
            | "minecraft:deep_frozen_ocean"
            | "minecraft:lukewarm_ocean"
            | "minecraft:deep_lukewarm_ocean"
            | "minecraft:warm_ocean"
            | "minecraft:lush_caves"
            | "minecraft:dripstone_caves"
            | "minecraft:deep_dark"
            | "minecraft:snowy_slopes"
            | "minecraft:jagged_peaks"
            | "minecraft:frozen_peaks"
            | "minecraft:stony_peaks"
            | "minecraft:mushroom_fields" => Self::Preserve,
            _ => Self::Temperate,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Choice<'a> {
    pub biome: &'a str,
    pub temperature: i64,
    pub humidity: i64,
}
#[derive(Clone, Copy, Debug)]
pub(super) struct Province<'a> {
    pub choices: [Option<Choice<'a>>; Habitat::COUNT],
    habitats: [u8; Habitat::COUNT],
}
impl<'a> Province<'a> {
    pub(super) fn census(
        seed: i64,
        cell: [i32; 2],
        samples: &[(Habitat, &'a str, TargetPoint)],
    ) -> Self {
        let mut votes = BTreeMap::<(Habitat, &str), (u8, i64, i64)>::new();
        let mut habitats = [0; Habitat::COUNT];
        for &(habitat, biome, target) in samples {
            if habitat == Habitat::Preserve {
                continue;
            }
            habitats[habitat as usize] += 1;
            let v = votes.entry((habitat, biome)).or_default();
            v.0 += 1;
            v.1 += target.temperature;
            v.2 += target.humidity;
        }
        let mut choices = [None; Habitat::COUNT];
        let mut ranks = [(0, 0); Habitat::COUNT];
        for ((habitat, biome), (count, t, h)) in votes {
            // A single isolated sample is not enough to paint a whole province.
            if count < 2 {
                continue;
            }
            let index = habitat as usize;
            let salt = biome.bytes().fold(0xcbf29ce484222325u64, |v, b| {
                (v ^ u64::from(b)).wrapping_mul(0x100000001b3)
            });
            let rank = (count, hash(seed, salt, cell));
            if rank > ranks[index] {
                ranks[index] = rank;
                choices[index] = Some(Choice {
                    biome,
                    temperature: t / i64::from(count),
                    humidity: h / i64::from(count),
                });
            }
        }
        Self { choices, habitats }
    }
    pub(super) fn choose(&self, habitat: Habitat, target: TargetPoint) -> Option<&'a str> {
        if habitat == Habitat::Preserve {
            return None;
        }
        let compatible = |choice: Choice<'a>| {
            ((choice.temperature - target.temperature).abs() <= 4000
                && (choice.humidity - target.humidity).abs() <= 6000)
                .then_some(choice.biome)
        };
        // Absorb small climate-family blips too, when there is a clear local
        // consensus. Wetlands and uplands retain their terrain/water niches;
        // arid, tropical and cold interiors are never substituted for each other.
        let dominant = self
            .habitats
            .iter()
            .enumerate()
            .max_by_key(|(_, n)| **n)
            .unwrap()
            .0;
        let land: u8 = self.habitats.iter().sum();
        let temperate = Habitat::Temperate as usize;
        let ordinary = |h: usize| h != Habitat::Highland as usize && h != Habitat::Wetland as usize;
        if self.habitats[dominant] >= 3
            && self.habitats[dominant] * 2 > land
            && ordinary(dominant)
            && ordinary(habitat as usize)
            && (dominant == temperate || habitat == Habitat::Temperate)
            && let Some(choice) = self.choices[dominant]
            && let Some(biome) = compatible(choice)
        {
            return Some(biome);
        }
        let choice = self.choices[habitat as usize]?;
        // A nearby province may span a sharp genuine climate boundary. It must
        // not project snow into a desert or a rainforest into an arid interior.
        compatible(choice)
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Site {
    pub cell: [i32; 2],
    pub position: [i32; 2],
    distance: i64,
}
#[derive(Debug)]
pub(super) struct SurfaceRegions<'a> {
    pub seed: i64,
    /// Quarts: 768-block provinces with the current packs' 192-quart setting.
    pub span: i32,
    pub cache: HashMap<[i32; 2], Province<'a>>,
    pub context: SamplerContext,
}
impl<'a> SurfaceRegions<'a> {
    pub(super) fn new(seed: i64, minimum: u32) -> Self {
        Self {
            seed,
            span: minimum.clamp(96, 192) as i32,
            cache: HashMap::new(),
            context: SamplerContext::cached(),
        }
    }
    pub(super) fn sites(&self, x: i32, z: i32) -> [Site; 9] {
        // A slow bounded warp makes the province borders curved. Jitter is
        // bounded, so sites cannot crowd together into tiny Voronoi cells.
        let warped = [
            i64::from(x) + self.warp(x, z, 0),
            i64::from(z) + self.warp(x, z, 1),
        ];
        let cell = warped.map(|v| v.div_euclid(i64::from(self.span)) as i32);
        let mut sites = std::array::from_fn(|i| {
            let c = [
                cell[0].wrapping_add(i as i32 % 3 - 1),
                cell[1].wrapping_add(i as i32 / 3 - 1),
            ];
            let position = std::array::from_fn(|axis| {
                let jitter = (hash(self.seed, 0x50524f56 + axis as u64, c) % 10001) as i64 - 5000;
                (i64::from(c[axis]) * i64::from(self.span)
                    + i64::from(self.span) / 2
                    + jitter * i64::from(self.span) / 25000) as i32
            });
            let d = std::array::from_fn::<_, 2, _>(|i| {
                i64::from(position[i].wrapping_sub(warped[i] as i32))
            });
            Site {
                cell: c,
                position,
                distance: d[0] * d[0] + d[1] * d[1],
            }
        });
        sites.sort_unstable_by_key(|s| (s.distance, s.cell));
        sites
    }
    fn warp(&self, x: i32, z: i32, salt: u64) -> i64 {
        let scale = i64::from(self.span) * 2;
        let (gx, gz) = (
            i64::from(x).div_euclid(scale),
            i64::from(z).div_euclid(scale),
        );
        let smooth = |v: i64| {
            let f = v.rem_euclid(scale) as f64 / scale as f64;
            f * f * (3. - 2. * f)
        };
        let (fx, fz) = (smooth(i64::from(x)), smooth(i64::from(z)));
        let at = |dx, dz| {
            hash(
                self.seed,
                0x45434f00 + salt,
                [(gx + dx) as i32, (gz + dz) as i32],
            ) as f64
                / u64::MAX as f64
        };
        let a = at(0, 0) * (1. - fx) + at(1, 0) * fx;
        let b = at(0, 1) * (1. - fx) + at(1, 1) * fx;
        (((a * (1. - fz) + b * fz) * 2. - 1.) * f64::from(self.span) / 10.).round() as i64
    }
    pub(super) fn insert(&mut self, site: Site, province: Province<'a>) {
        if self.cache.len() >= 256 {
            self.cache.clear();
        }
        self.cache.insert(site.cell, province);
    }
}
fn hash(seed: i64, salt: u64, cell: [i32; 2]) -> u64 {
    fn mix(mut v: u64) -> u64 {
        v = (v ^ (v >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        v = (v ^ (v >> 27)).wrapping_mul(0x94d049bb133111eb);
        v ^ (v >> 31)
    }
    mix(seed as u64 ^ mix(salt) ^ mix(cell[0] as u64) ^ mix(cell[1] as u64 ^ 0x9e3779b97f4a7c15))
}

#[cfg(test)]
mod tests;
