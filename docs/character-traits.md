# Character traits

Content catalogs can provide `race`, `size` and `starting_items` alongside the
appearance fields in `assets/<namespace>/horizon/characters.json`:

```json
{
  "id": "lunatic:cirno",
  "name": "Cirno",
  "texture": "lunatic:entity/character/cirno",
  "model": "slim",
  "race": "fairy,ice_fairy,youkai",
  "size": 0.7,
  "starting_items": [
    {"item": "minecraft:cookie", "count": 16},
    {"choices": ["minecraft:poppy", "minecraft:dandelion"],
     "min_count": 1, "max_count": 4, "rolls": 3}
  ]
}
```

The generic catalog preserves these fields for native mods and includes them in
catalog validation. `race` defaults to an empty string; `Character::has_race`
matches complete comma-separated tags, ignoring surrounding spaces and ASCII
case. `size` defaults to `1.0` and is clamped to `0.7`–`1.2`; the menu preview
shows the relative size. `starting_items` defaults to an empty list. Each entry
supports a fixed item/count or a choices pool, optional min/max counts and rolls
(default one).

Lunatic implements gameplay through its native bundle: a transient
`minecraft:scale` modifier uses the engine’s existing pose dimensions, collisions,
models, eyes and targeting. Its fairy movement uses the ordinary Slow Falling
status flag and movement/fall-damage pipeline, retaining potion effects as an
independent source. Its server grants kits from the approved character catalog
and persists awards and pending inventory remainders in player NBT.

The engine does not grant items or race abilities without an enabled native mod.
Matching content catalogs are required on the server and client. See Lunatic’s
README for kit behavior and the available abilities.
