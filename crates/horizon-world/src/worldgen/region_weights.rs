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
            weights,
            total: total as i32,
            seeds,
            cache,
        })
    }
    pub fn sample(&mut self, x: i32, z: i32) -> usize {
        self.at(self.seeds.len() - 1, x, z)
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
