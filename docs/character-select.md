# Character Select

The title's Character Select button replaces Realms. Singleplayer, Multiplayer,
and Friends require a confirmed character. Multiplayer still respects account
restrictions. Browsing or cancelling does not confirm a choice. The instance
stores the chosen identifier in `character.json`, independently of account and
world data. A missing or disabled character requires a new selection.

Enabled content mods provide `assets/<namespace>/horizon/characters.json`:

```json
{
  "characters": [
    {
      "id": "lunatic:reisen_udongein_inaba",
      "name": "Reisen Udongein Inaba",
      "texture": "lunatic:entity/character/reisen_udongein_inaba",
      "model": "slim"
    }
  ]
}
```

IDs and texture names belong to that namespace. Skins live under
`assets/<namespace>/textures/entity/character/`, as 64×64 or legacy 64×32 PNGs.
Legacy skins are expanded in memory using the same layout and transparency
handling as downloaded skins. Original mod assets remain in the mod, separately
from the engine's MIT source.

The optional `model` is `slim` or `wide` (the default for older catalogs).
The preview renders the matching prepared player model with idle arm movement,
mouse-following head rotation, lighting, and depth. The confirmed skin also
covers the local player's model and first-person arm. Selection changes appearance
only, preserving the account name and UUID. It does not upload skins to account
services or distribute them to other players on remote servers.

Steve is only the fallback for a missing or invalid skin and is never inserted
into the character roster. With no enabled character catalog, the screen explains
that characters are unavailable and keeps play disabled.
