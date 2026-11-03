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
only, preserving the account name and UUID. It does not upload skins to account services. Matching fork servers synchronize
server-approved character IDs to other clients.

Steve is only the fallback for a missing or invalid skin and is never inserted
into the character roster. With no enabled character catalog, the screen explains
that characters are unavailable and keeps play disabled.

Characters may set `character_chat: true` to display the local player's ordinary
chat as `Character Name : message`, and use that name on the player's own visible
nametag. Optional `color` accepts Minecraft named colors or `#RRGGBB`, and colors
the name only. These are presentation changes: profile identity, scoreboard keys,
message signing, commands and private messages are unchanged.

Selections synchronize over `lunatic:characters_v2` on Horizon mod networking.
The authenticated connection supplies the UUID. Both sides cache one indexed
catalog and its SHA-256 digest per loaded resource generation, shared by cloned
resource bundles. A selection is 39 bytes (sequence, digest and catalog index).
The server validates it and piggybacks an acknowledgement on a revisioned roster
update. It sends one snapshot on join and coalesces changed selections/removals
into one delta per tick. Encoded snapshots are shared until the roster changes;
normal chat reads the cached UUID map and sends no character messages.

After acknowledgement, synchronization is silent until a selection, roster or
catalog changes. A matching restored selection needs no client request. Failed
handshakes/resyncs have at most four attempts with exponential backoff; explicit
rejections stop retries. Revision gaps request a fresh snapshot. Catalog mismatch
stays rejected until the client loads a different catalog. Texture-only reloads
retain the same digest and need no resynchronization. The roster is capped at
1,024 players so its binary snapshot fits the 30,000-byte mod channel.

The server saves changed IDs only, coalescing writes once per tick, and restores
them on login. An unreadable/invalid preference file is preserved, never replaced
with defaults. Disconnects remove online appearances without deleting saved
preferences. Older v1 clients still receive cached JSON snapshots when something
changes; their identical polling requests generate no replies or disk writes.
New clients and servers should be updated together. Agent `status` exposes safe
`server.character_sync` counters for traffic regression tests.

Player models, overhead names, player-list names/skins and ordinary chat resolve
through that shared UUID map. Accounts, scoreboard keys and signed message bodies
remain unchanged. Chat messages retain a sender UUID and skin snapshot internally;
the ChatHeads compatibility bundle uses those rather than parsing character names.

ChatHeads 0.1.0 parses only `<profile>` chat lines. Our compatible bundle is built
with `python3 tools/patch_chatheads.py ORIGINAL.hnmod OUTPUT.hnmod`, then installed
as ChatHeads and included in launcher Build. The patch registers character names
and skin locations with ChatHeads' layout, recognizes their `Name : message`
prefix, and draws the character skin's face. Profile chat remains supported.
The source checks deliberately reject changed/already patched bundles so a newer
upstream ChatHeads requires reviewing the patch rather than silently losing it.


Run the multiplayer regression with a built game and your prepared resources:

```sh
python3 tools/character_sync_e2e.py --game /path/to/horizon-client \
  --ctl /path/to/horizon-ctl --resources /path/to/prepared/26.3 \
  --mod /path/to/lunatic.hmod
```

It runs a private offline dedicated fixture and two headless clients, all in
scratch folders, and checks the shared roster/colors, late joins, chat,
overhead names, disconnects, changed selections, reconnects and persisted UUID
choices. It also verifies that eight idle seconds and a burst of chat produce
zero character messages, and an unchanged reconnect does not rewrite preferences.
The production server uses online authentication.
