# Fork commands

`/worldspawn` teleports the executing player to the world's configured Overworld
spawn, centered on its block and facing the saved spawn rotation. It ignores
personal beds/respawn anchors and works from other dimensions. It uses the normal
teleport machinery, including destination chunk loading and client synchronization.

Requires operator permission level 2 or higher (the same as `/tp`). It takes no
arguments. A console or command block needs a player source, for example
`execute as GojoWon run worldspawn`.

Registered in the fork's game command trees on dedicated and integrated servers;
vanilla-only parity trees remain unchanged. The dedicated server must run a build
containing the command; installing a client alone does not add server commands.

The regression is `tools/character_sync_e2e.py --worldspawn` with its normal game,
control executable, prepared-resource and Lunatic archive arguments. It checks
operator permission, denial for non-operators, the world spawn rather than a bed,
block centering/rotation, and returning from the Nether on a real dedicated server.
