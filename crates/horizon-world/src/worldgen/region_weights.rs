//! Seeded, weighted region cells, refined by fuzzy then majority zooms.
//! The integer selection/zoom contract is checked against TerraBlender 26.3's Java classes.
use std::collections::HashMap;
fn step(seed: i64, salt: i64) -> i64 {
    seed.wrapping_mul(
        seed.wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407),
    )
    .wrapping_add(salt)
}
fn layer_seed(world: i64, salt: i64) -> i64 {
    let mut mixed = salt;
    for _ in 0..3 {
        mixed = step(mixed, salt);
    }
    let mut out = world;
    for _ in 0..3 {
        out = step(out, mixed);
    }
    out
}
struct Random {
    seed: i64,
    value: i64,
}
impl Random {
    fn new(seed: i64, x: i32, z: i32) -> Self {
        let mut value = seed;
        for v in [x, z, x, z] {
            value = step(value, v as i64);
        }
        Self { seed, value }
    }
    fn next(&mut self, bound: i32) -> usize {
        let out = (self.value >> 24).rem_euclid(bound as i64) as usize;
        self.value = step(self.value, self.seed);
        out
    }
}
#[derive(Debug)]
pub struct RegionWeights {
    tiered: Option<(i64, u32)>,
    weights: Vec<u32>,
    total: i32,
    seeds: Vec<i64>,
    cache: Vec<HashMap<[i32; 2], usize>>,
}
impl RegionWeights {
    pub fn new(seed: i64, weights: Vec<u32>, zooms: u8) -> Result<Self, &'static str> {
        let total: u64 = weights.iter().map(|&w| u64::from(w)).sum();
        if total == 0 || total > i32::MAX as u64 || zooms > 6 {
            return Err("region weight total or zoom count");
        }
        let salts = [1, 2000, 2001, 2002, 2003]
            .into_iter()
            .chain((0..zooms).map(|i| 1001 + i as i64));
        let seeds: Vec<_> = salts.map(|s| layer_seed(seed, s)).collect();
        let cache = (0..seeds.len()).map(|_| HashMap::new()).collect();
        Ok(Self {
            tiered: None,
            weights,
            total: total as i32,
            seeds,
            cache,
        })
    }
    /// Opt-in, nested region cells. Size does not change selection probabilities.
    pub fn with_min_quarts(mut self, seed: i64, minimum: u32) -> Self {
        if minimum > 0 { self.tiered = Some((seed, minimum)); }
        self
    }
    pub fn sample(&mut self, x: i32, z: i32) -> usize {
        if let Some((seed, minimum)) = self.tiered {
            return self.tiered_sample(seed, minimum, x, z);
        }
        self.at(self.seeds.len() - 1, x, z)
    }
    fn tiered_sample(&self, seed: i64, minimum: u32, x: i32, z: i32) -> usize {
        let span = i64::from(minimum) * 2;
        let warp = |salt| {
            let (gx, gz) = (i64::from(x).div_euclid(span), i64::from(z).div_euclid(span));
            let (fx, fz) = (i64::from(x).rem_euclid(span) as f64 / span as f64,
                            i64::from(z).rem_euclid(span) as f64 / span as f64);
            let smooth = |v: f64| v*v*(3.0-2.0*v);
            let (fx, fz) = (smooth(fx), smooth(fz));
            let at = |dx, dz| coordinate_hash(seed, salt, gx+dx, gz+dz) as f64 / u64::MAX as f64;
            let a = at(0,0)*(1.0-fx)+at(1,0)*fx;
            let b = at(0,1)*(1.0-fx)+at(1,1)*fx;
            (((a*(1.0-fz)+b*fz)*2.0-1.0) * f64::from(minimum)/8.0).round() as i64
        };
        // A shared bounded warp softens the tier-cell boundaries.
        let (x,z) = (i64::from(x)+warp(0x57415250), i64::from(z)+warp(0x57415251));
        let mut remaining = self.total as u64;
        for (index, &weight) in self.weights.iter().enumerate() {
            if weight == 0 { continue; }
            if remaining == u64::from(weight) { return index; }
            let shift = self.weights.len().saturating_sub(index+2).min(8);
            let size = match shift {
                0 => i64::from(minimum) * 2,
                1 => i64::from(minimum) * 8 / 3,
                _ => i64::from(minimum) << shift,
            };
            let hash = coordinate_hash(seed, index as u64 + 0x54494552, x.div_euclid(size), z.div_euclid(size));
            let roll = ((u128::from(hash)*u128::from(remaining)) >> 64) as u64;
            if roll < u64::from(weight) { return index; }
            remaining -= u64::from(weight);
        }
        unreachable!("positive validated region total")
    }
    fn at(&mut self, level: usize, x: i32, z: i32) -> usize {
        if let Some(&value) = self.cache[level].get(&[x, z]) {
            return value;
        }
        let out = if level == 0 {
            let mut roll = Random::new(self.seeds[0], x, z).next(self.total);
            let mut found = 0;
            for (i, &weight) in self.weights.iter().enumerate() {
                if roll < weight as usize {
                    found = i;
                    break;
                }
                roll -= weight as usize;
            }
            found
        } else {
            let a = self.at(level - 1, x >> 1, z >> 1);
            let mut r = Random::new(self.seeds[level], x >> 1 << 1, z >> 1 << 1);
            if x & 1 == 0 && z & 1 == 0 {
                a
            } else {
                let c = self.at(level - 1, x >> 1, z.wrapping_add(1) >> 1);
                let vertical = [a, c][r.next(2)];
                if x & 1 == 0 {
                    vertical
                } else {
                    let b = self.at(level - 1, x.wrapping_add(1) >> 1, z >> 1);
                    let horizontal = [a, b][r.next(2)];
                    if z & 1 == 0 {
                        horizontal
                    } else {
                        let d = self.at(level - 1, x.wrapping_add(1) >> 1, z.wrapping_add(1) >> 1);
                        if level == 1 {
                            [a, b, c, d][r.next(4)]
                        } else {
                            majority([a, b, c, d]).unwrap_or_else(|| [a, b, c, d][r.next(4)])
                        }
                    }
                }
            }
        };
        // Cache policy does not affect a cell's coordinate-seeded answer.
        if self.cache[level].len() >= 1024 {
            self.cache[level].clear();
        }
        self.cache[level].insert([x, z], out);
        out
    }
}
fn coordinate_hash(seed: i64, salt: u64, x: i64, z: i64) -> u64 {
    fn mix(mut v: u64) -> u64 {
        v = (v ^ (v >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        v = (v ^ (v >> 27)).wrapping_mul(0x94d049bb133111eb);
        v ^ (v >> 31)
    }
    mix(seed as u64 ^ mix(salt) ^ mix(x as u64 ^ 0x9e3779b97f4a7c15)
        ^ mix(z as u64 ^ 0xd1b54a32d192ed03))
}
fn majority([a, b, c, d]: [usize; 4]) -> Option<usize> {
    if b == c && c == d {
        Some(b)
    } else if (a == b && (a == c || a == d || c != d))
        || (a == c && (a == d || b != d))
        || (a == d && b != c)
    {
        Some(a)
    } else if (b == c && a != d) || (b == d && a != c) {
        Some(b)
    } else if c == d && a != b {
        Some(c)
    } else {
        None
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tiered_cells_have_broad_cores_and_are_query_order_independent() {
        let mut regions = RegionWeights::new(123, vec![40,12,8,4,1], 3).unwrap().with_min_quarts(123,192);
        let mut expected = Vec::new();
        for z in -12..12 { for x in -12..12 {
            let (x,z) = (x*1536+192,z*1536+192);
            let center = regions.sample(x,z);
            for dx in [-48,0,48] { for dz in [-48,0,48] {
                assert_eq!(regions.sample(x+dx,z+dz),center);
            }}
            expected.push((x,z,center));
        }}
        for (x,z,value) in expected.into_iter().rev() { assert_eq!(regions.sample(x,z),value); }
    }
    #[test]
    fn tiered_weight_shares_match_weights_and_zero_stays_disabled() {
        for weights in [vec![40,12,8,4,1],vec![40,12,8,0,1]] {
            let total = weights.iter().sum::<u32>() as f64;
            let mut count = vec![0;weights.len()];
            let mut regions = RegionWeights::new(456,weights.clone(),3).unwrap().with_min_quarts(456,192);
            for z in -128..128 { for x in -128..128 {
                count[regions.sample(x*1024,z*1024)] += 1;
            }}
            for (i,&weight) in weights.iter().enumerate() {
                if weight == 0 { assert_eq!(count[i],0); }
                assert!((count[i] as f64/65536.0-f64::from(weight)/total).abs()<0.015);
            }
        }
    }
    #[test]
    fn java_layer_oracle_vectors() {
        let mut previous = None;
        let mut regions = None;
        let settings = [
            vec![10, 10, 8, 2],
            vec![10, 13, 2],
            vec![10, 0, 0, 0],
            vec![10, 1, 0, 50],
            vec![1, i32::MAX as u32 - 1, 0, 0],
        ];
        for line in include_str!("../../tests/fixtures/region_weights.csv").lines() {
            let n: Vec<i64> = line.split(',').map(|v| v.parse().unwrap()).collect();
            let key = (n[0], n[1]);
            if previous != Some(key) {
                regions = Some(
                    RegionWeights::new(
                        n[0],
                        settings[n[1] as usize].clone(),
                        if n[1] == 1 { 2 } else { 3 },
                    )
                    .unwrap(),
                );
                previous = Some(key);
            }
            assert_eq!(
                regions.as_mut().unwrap().sample(n[2] as i32, n[3] as i32),
                n[4] as usize,
                "{line}"
            );
        }
    }
    #[test]
    fn zero_weight_regions_never_occur_and_query_order_is_irrelevant() {
        let mut a = RegionWeights::new(-42, vec![10, 0, 0, 0], 3).unwrap();
        for x in -128..128 {
            assert_eq!(a.sample(x, x * 7), 0);
        }
        let mut a = RegionWeights::new(42, vec![10, 10, 8, 2], 3).unwrap();
        let samples: Vec<_> = (-200..200).map(|x| (x, a.sample(x, x * 13))).collect();
        let mut b = RegionWeights::new(42, vec![10, 10, 8, 2], 3).unwrap();
        for (x, want) in samples.into_iter().rev() {
            assert_eq!(b.sample(x, x * 13), want);
        }
    }
}
