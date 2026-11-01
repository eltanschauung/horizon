# Content biome extensions

The fork supports additional namespaced biomes in content mods (`.hmod`) and
world data packs. This is a generic engine foundation; it is not a finished
Biomes O' Plenty port.

## Registration and composition

A content mod supplies `data/<namespace>/worldgen/biome/<path>.json` using the
host's 26.3 biome format. The definition joins `worldgen/biomes.json`, its
features/carvers resolve against the assembled registries, and its identifier
can appear in biome tags and biome modifiers. Vanilla local IDs stay fixed;
content IDs append in identifier order. World saves persist names, not these
local IDs. Network IDs are independently sorted in protocol identifier order.
The server includes custom biome definitions even when a client acknowledges
the vanilla core pack. Custom blocks still require matching content on peers.

Both `worldgen/feature/` (26.3) and `worldgen/configured_feature/` (older content)
feed the configured-feature bundle. `worldgen/carver/` is supported too. A
custom type inside a JSON file still needs a native implementation; registering
a file does not translate Java algorithms.

Content mods own distinct namespaces. Biome placement declarations are merged
by declaration identifier, rather than by mod load order. They extend the host's
candidate table and do not write noise settings, density functions, dimension
generators, or Tectonic configuration. Data-pack replacements retain vanilla's
explicit pack-priority semantics.

## Additive multi-noise climate points

Place a declaration at
`data/<namespace>/worldgen/biome_placement/<name>.json`:

```json
{
  "preset": "minecraft:overworld",
  "biomes": [{
    "biome": "example:woodland",
    "parameters": {
      "temperature": [-0.15, 0.2],
      "humidity": [0.3, 1.0],
      "continentalness": [-0.11, 1.0],
      "erosion": [-1.0, 1.0],
      "depth": 0.0,
      "weirdness": [-1.0, 1.0],
      "offset": 0.0
    }
  }]
}
```

The six intervals and offset use vanilla's climate codec. These are normalized
host climate coordinates, not degrees Celsius, millimeters of rainfall, or
physical elevation. The engine validates biome/preset references and publishes
the assembled table only after all declarations validate. The table is bounded
to 65,536 entries. Fixed, checkerboard, and End sources are unchanged.

Nearest-climate matching uses the existing active router samplers, including
Tectonic's temperature/vegetation scale and multiplier changes. Additions win
an equal-fitness match explicitly: otherwise vanilla's broad zero-distance
boxes and previous-leaf search cache could hide every new biome. Existing
vanilla-only resolution and its tie behavior remain unchanged when no additions
are installed. An offset affects distance fitness; it is not a selection weight.
This API does not yet implement probabilistic weights, physical environmental
restrictions, seasonal climate, or planetary climate integration.

Surface material composition and custom native feature algorithms are separate
work. Merely adding a biome does not replace the host's material rule. A full
biome pack must provide and validate its surface and decoration integration.

## Existing worlds and removal

Enabling a biome pack changes future generation; existing saved chunks keep their
named palettes. Runtime IDs can change as the installed set changes, and loading
remaps by name. Removing a mod is not lossless: the current loader maps missing
biome names to plains and unknown block states to air. Re-saving such chunks can
make that substitution permanent. Keep the original content installed, or test
an explicit migration on a backup before removing it from a real world.

## Verification

Unit and integration coverage includes custom biome metadata/local IDs,
world-pack changes, tags, modifiers applied to newly declared biomes, modern
feature bundles, network definitions with the known-core-pack optimization,
reference validation, deterministic table assembly, and scalar/bulk biome
resolution, including zero-distance host ties. These tests do not certify a
complete BOP port, full mod-removal migration, or full chunk-generation parity.
