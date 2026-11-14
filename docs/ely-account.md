# Ely.by accounts

Settings → Account → Ely.by accepts an email/username, password and optional six-digit
2FA code. Signing in selects that account. The normal player switcher can switch
between Microsoft, Ely.by and existing portable/offline players; Sign out revokes
only this launcher's Ely session and removes it locally, even if Ely is unavailable.

No OAuth application is registered, so this uses Ely's official direct launcher
[authentication protocol](https://docs.ely.by/en/minecraft-auth.html). Passwords
and 2FA codes stay in memory, are cleared after success or leaving the page, and
are never saved. Only refreshable session/client tokens enter the existing OS
keychain/encrypted fallback store, under `ely:<uuid>`. Public account metadata is
separate from Microsoft metadata. Existing Microsoft ownership/download checks
remain in place: Ely authenticates a player, not a Minecraft license.

The native game reads a fresh owner-only session file with `user_type: "ely"` and
joins through the fixed Ely session service. It never sends Ely tokens to Microsoft
attributes, certificates, profile or Realms services. Public Ely skins use the
normal skin pipeline with narrowly allowed texture origins. No authlib injector
or Java authentication agent is required for Horizon's Rust game.

Online-mode Horizon servers accepting portable accounts (`[server] allow_portable = true` in `server.toml`)
also verify Ely joins through Ely's HTTPS `hasJoined` service after Microsoft.
They retain Microsoft precedence and accept only Ely UUIDv5 identities, disjoint
from Microsoft UUIDv4 and portable UUIDv8. Display names never link accounts or
transfer existing progress. Servers without this opt-in continue accepting only
Microsoft authentication. Clients and servers need this change; older pinned
game builds do not understand Ely sessions. Java instances retain Microsoft auth.

CLI selection uses `account select ely:<uuid>` or `play --account ely:<uuid>`;
`account --json` includes a separate Ely list without tokens.
