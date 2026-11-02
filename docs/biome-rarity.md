# Biome region weights

The BOP 0.2.1 package declares Java-style weighted Overworld and Nether regions.
The initial cell lottery includes vanilla (weight 10), then the BOP regions in
Java registration order. A fuzzy zoom and the normal majority zooms form coherent
regions; each region resolves the host's climate target against its own climate
table. Region choice does not use the distribution of temperature or weirdness.
The world seed, signed coordinates, integer overflow and random draw order match
the Java layer oracle. Tectonic's climate and density samplers are unchanged.

With BOP enabled, Horizon creates `config/biomesoplenty/generation.toml` in the
game home. The five BOP keys, defaults and integer bounds match Java BOP:

```toml
[overworld]
bop_primary_overworld_region_weight = 10
bop_secondary_overworld_region_weight = 8
bop_overworld_rare_region_weight = 2

[nether]
bop_nether_region_weight = 13
bop_nether_rare_region_weight = 2
```

Weights are relative, not percentages. Zero disables that region; a biome may
still occur if another enabled region contains it. All five weights may be zero:
vanilla regions remain available. Each weight is in 0..2147483647; as in Java,
a dimension's total (including vanilla's 10) must fit a signed 32-bit integer.
Missing keys use defaults. Invalid values fail loading with the file/key named.
Existing configuration files are never overwritten.

Restart the game after editing. Changes affect newly generated chunks; saved
chunks keep their existing biomes. This pass exposes rarity weights only.
Region zoom counts remain at Java's defaults (Overworld 3, Nether 2); End
replacement selection and its fixed weights are unchanged by this pass.

For content authors, a biome-placement declaration can opt in with `region_order`
(unique positive integer), `region_zooms` (0..6), and `vanilla_weight`. All groups
in a preset must consistently declare this mode. `weight` may then be zero.
A `configuration` object names `file` and dotted TOML `key`; the file must stay
under the declaration namespace inside the supplied configuration root. Settings
are read before constructing the immutable generator tables. Legacy declarations
without region metadata retain their previous selector behavior.

Verification uses 7,680 numeric samples from the original TerraBlender 26.3 Java
layers, plus scalar/prepared biome queries, configuration overrides and zero-weight
regions. Run `tools/test_region_weights_parity.py` with a local TerraBlender source
checkout and the private Minecraft oracle classpath to regenerate the comparison.
