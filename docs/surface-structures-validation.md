# Surface fitting validation

Validated with locally prepared 26.3 resources. No saved worlds were opened or
modified. The probe generates terrain in memory. Tectonic inputs come from the
0.3.5 settings adapter, including its typed beardifier node.

The census checks 161 × 161 seeded owner chunks for each seed and mode. Original
placement uses the same inputs with `horizon:surface_placement.enabled` disabled;
structure-set spacing, weights and salts are identical. Small count differences
are expected when a nearby candidate satisfies a biome or terrain constraint.

| Mode | Seed | All starts | Villages | Fitted starts |
| --- | --- | --- | --- | --- |
| vanilla | 1234 | 224 | 7 | 17 |
| vanilla | -9182 | 267 | 3 | 6 |
| vanilla | 42 | 212 | 2 | 7 |
| vanilla-original | 1234 | 222 | 6 | 0 |
| vanilla-original | -9182 | 267 | 3 | 0 |
| vanilla-original | 42 | 213 | 2 | 0 |
| tectonic | 1234 | 207 | 10 | 17 |
| tectonic | -9182 | 243 | 6 | 28 |
| tectonic | 42 | 232 | 3 | 3 |
| tectonic-original | 1234 | 206 | 9 | 0 |
| tectonic-original | -9182 | 237 | 5 | 0 |
| tectonic-original | 42 | 233 | 4 | 0 |

Seed 42 additionally checks normal and fixed-desert worlds with and without
Tectonic. All four cases assert identical complete chunk block hashes when two
neighbouring chunks are generated in reverse order using fresh generators. They
also assert exact complete structure-start NBT equality after loading the saved
start through `OldWorld`; the desert cases require a fitted desert pyramid.

Horizon library tests: 411 passed, two existing ignored. Tectonic tests: 14 passed.
Focused tests cover independent building settlement with preserved upper storeys,
exact persisted floating-point slopes, cavity/fluid preservation, bounded fill,
unchanged distant columns, fluid protection across chunk edges and continuous
blending at foundation edges.

Reproduce with `surface_terrain_probe` (the horizon-world example), passing the
prepared resource directory and optionally an overrides JSON. Set
`HORIZON_SURFACE_CHECK_CHUNKS=1` to check generation order and saved starts.
`HORIZON_SURFACE_SEEDS` selects comma-separated seeds; default is 1234,-9182,42.
For the desert case use `HORIZON_SURFACE_TEST_BIOME=minecraft:desert` and
`HORIZON_SURFACE_TEST_STRUCTURE=minecraft:desert_pyramid`. The Tectonic example
`export_surface_overrides` writes its registry overrides from a supplied local
worldgen resource directory.

This is deterministic generation and regression validation, not a visual survey
of every possible terrain/layout combination. Tightened limits can reject sites;
fluids and thin cave roofs take precedence over terrain fitting. Existing chunks
are not retroactively reshaped.
