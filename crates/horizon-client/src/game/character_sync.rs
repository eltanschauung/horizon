//! Cached character metadata and event-driven selection/roster messages.
use super::Game;
use horizon_client::characters::{
    Catalog, Character,
    sync::{self, Client, Rejection, Request, Server, Update},
};
use std::{
    collections::BTreeMap,
    io::{self, Write},
    sync::Arc,
    time::Instant,
};

pub(super) use sync::{CHANNEL, LEGACY_CHANNEL};
pub(super) struct State {
    pub client: Client,
    server: Server,
    saved: Option<Saved>,
    resolved: BTreeMap<u128, Character>,
    account_names: BTreeMap<String,u128>,
    legacy_requests: BTreeMap<u128, Vec<u8>>,
    started: Instant,
    reported: Option<Rejection>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            client: Client::default(),
            server: Server::default(),
            saved: None,
            resolved: BTreeMap::new(),
            account_names: BTreeMap::new(),
            legacy_requests: BTreeMap::new(),
            started: Instant::now(),
            reported: None,
        }
    }
}
struct Saved {
    entries: BTreeMap<String, String>,
    writable: bool,
    dirty: bool,
}
impl Saved {
    fn load() -> Self {
        let result = std::fs::read("character-selections.json")
            .and_then(|b| serde_json::from_slice(&b).map_err(io::Error::other));
        match result {
            Ok(entries) => Self {
                entries,
                writable: true,
                dirty: false,
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => Self {
                entries: BTreeMap::new(),
                writable: true,
                dirty: false,
            },
            Err(_) => {
                log::error!("Could not read character selections; preserving the existing file");
                Self {
                    entries: BTreeMap::new(),
                    writable: false,
                    dirty: false,
                }
            }
        }
    }
    fn remember(&mut self, uuid: u128, id: &str) {
        let key = format!("{uuid:032x}");
        if self.entries.get(&key).is_none_or(|old| old != id) {
            self.entries.insert(key, id.to_owned());
            self.dirty = true;
        }
    }
    fn flush(&mut self) {
        if !self.dirty || !self.writable {
            return;
        }
        let temporary = format!(".character-selections-{}.tmp", std::process::id());
        let result = (|| -> io::Result<()> {
            let bytes = serde_json::to_vec_pretty(&self.entries)?;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            let result = (|| {
                file.write_all(&bytes)?;
                file.sync_all()?;
                drop(file);
                std::fs::rename(&temporary, "character-selections.json")
            })();
            if result.is_err() {
                let _ = std::fs::remove_file(&temporary);
            }
            result
        })();
        self.dirty = false;
        if result.is_err() {
            // Do not retry disk failures every tick or overwrite an unreadable preference file.
            self.writable = false;
            log::error!("Could not save character selections; existing preferences preserved");
        }
    }
}
impl Game<'_> {
    pub(super) fn character_for(&self, uuid: u128) -> Option<&Character> {
        let index = if self.remote.is_some() {
            self.character_sync.client.peers.get(&uuid)
        } else {
            self.character_sync.server.peers.get(&uuid)
        };
        index
            .and_then(|i| self.pack.character_catalog.get(*i))
            .map(|base| self.character_sync.resolved.get(&uuid).filter(|c| c.id == base.id).unwrap_or(base))
            .or_else(|| {
                (!self.character_sync.client.authoritative()
                    && self.remote.as_ref().is_some_and(|r| r.profile() == uuid))
                .then_some(self.character.as_ref())
                .flatten()
            })
    }
    pub(super) fn character_for_name(&self, name: &str) -> Option<&Character> {
        if let Some(remote) = &self.remote { let uuid=remote.character_uuid(name).or_else(||self.character_sync.account_names.get(name).copied())?;return self.character_for(uuid).or_else(||self.character_sync.resolved.get(&uuid)); }
        if self.lan_acting_name.as_deref() == Some(name) { return self.character_for(self.lan_acting_uuid?); }
        self.lan.as_ref().and_then(|h| h.hub.guests.iter().find(|g|g.name==name)).and_then(|g|self.character_for(g.uuid))
            .or_else(|| (name == self.acting_player_name()).then_some(self.character.as_ref()).flatten())
    }
    // Rewrite identity arguments in server announcements, never signed message bodies or command targets.
    pub(super) fn character_system_text(&self, text:horizon_client::text::Text)->horizon_client::text::Text {
        fn visit(game:&Game<'_>, v:&mut serde_json::Value){
            let identity=v.get("translate").and_then(|v|v.as_str()).is_some_and(|key|key.starts_with("chat.type.advancement.")||key.starts_with("multiplayer.player.")||key.starts_with("death.attack.")||key.starts_with("commands."));
            if identity {if let Some(args)=v.get_mut("with").and_then(|a|a.as_array_mut()){for arg in args {
                let account=arg.as_str().or_else(||arg.get("text").and_then(|t|t.as_str())).map(str::to_owned);
                if let Some(c)=account.as_deref().and_then(|n|game.character_for_name(n)){*arg=c.display_name().component();}else{visit(game,arg);}
            }}}
            if let Some(extra)=v.get_mut("extra").and_then(|a|a.as_array_mut()){for child in extra{visit(game,child);}}
        }
        let mut component=text.component();visit(self,&mut component);horizon_client::scoreboard::comp_text(&component)
    }
    pub(super) fn tick_character_sync(&mut self) {
        let Some(remote) = self
            .remote
            .as_ref()
            .filter(|r| r.joined && r.mod_net && r.reconfiguring.is_none())
        else {
            return;
        };
        let Some(index) = self
            .character
            .as_ref()
            .and_then(|c| self.pack.character_catalog.index(&c.id))
        else {
            return;
        };
        let data = self.character_sync.client.poll(
            remote.profile(),
            index,
            self.pack.character_catalog.digest,
            self.character_sync.started.elapsed(),
        );
        if let Some(data) = data {
            if self.native_net_send_to_server(CHANNEL, &data).is_err() {
                self.character_sync.client.send_failed();
            }
        }
        let rejection = self.character_sync.client.rejection;
        if rejection != self.character_sync.reported {
            self.character_sync.reported = rejection;
            if rejection.is_some() {
                self.system_message(horizon_client::text::Text::literal("Lunatic character selection rejected: install the server's matching character catalog."));
            }
        }
    }
    pub(super) fn character_from_client(&mut self, uuid: u128, bytes: &[u8]) {
        let Ok(request) = Request::decode(bytes) else {
            return;
        };
        if self
            .character_sync
            .server
            .select(uuid, request, &self.pack.character_catalog)
        {
            self.remember_character(uuid);
        }
    }
    pub(super) fn character_from_legacy_client(&mut self, uuid: u128, bytes: &[u8]) {
        if bytes.len() > 256
            || self
                .character_sync
                .legacy_requests
                .get(&uuid)
                .is_some_and(|old| old == bytes)
        {
            return;
        }
        self.character_sync
            .legacy_requests
            .insert(uuid, bytes.to_vec());
        let Some(index) = self.pack.character_catalog.resolve_legacy(bytes) else {
            return;
        };
        if self.character_sync.server.select_legacy(uuid, index) {
            self.remember_character(uuid);
        }
    }
    fn resolve_character_name(&mut self, uuid:u128) {
        let Some(mut c) = self.character_sync.server.peers.get(&uuid).and_then(|i|self.pack.character_catalog.get(*i)).cloned() else{return;};
        let path=std::path::Path::new("character-name-reservations.json");
        let result=(|| -> io::Result<u32>{let mut names=horizon_client::characters::reservations::Reservations::load(path)?;let slot=names.reserve(uuid,&c.id);names.save(path)?;Ok(slot)})();
        match result {Ok(slot)=>{c.name.push_str(&horizon_client::characters::reservations::suffix(slot));self.character_sync.resolved.insert(uuid,c);},Err(e)=>log::error!("Cannot reserve character name: {e}")}
    }
    pub(super) fn character_names_from_server(&mut self, bytes:&[u8]) {
        if let Ok(rows)=serde_json::from_slice::<Vec<(String,String,String)>>(bytes) {
            for (uuid,id,name) in rows.into_iter().take(64) {if let (Ok(uuid),Some(mut c))=(u128::from_str_radix(&uuid,16),self.pack.character_catalog.index(&id).and_then(|i|self.pack.character_catalog.get(i)).cloned()){if name.len()<=256 && name.starts_with(&c.name){c.name=name;if let Some(p)=self.remote.as_ref().and_then(|r|r.tab_players.get(&uuid)){self.character_sync.account_names.insert(p.name.clone(),uuid);}self.character_sync.resolved.insert(uuid,c);}}}
        }
    }
    fn remember_character(&mut self, uuid: u128) {
        self.resolve_character_name(uuid);
        if let Some(character) = self
            .character_sync
            .server
            .peers
            .get(&uuid)
            .and_then(|i| self.pack.character_catalog.get(*i))
        {
            self.character_sync
                .saved
                .get_or_insert_with(Saved::load)
                .remember(uuid, &character.id);
        }
    }
    pub(super) fn flush_character_sync(&mut self) {
        if let Some(saved) = &mut self.character_sync.saved {
            saved.flush();
        }
        for delivery in self
            .character_sync
            .server
            .flush(&self.pack.character_catalog)
        {
            let rows:Vec<_>=self.character_sync.resolved.iter().filter(|(u,_)|self.character_sync.server.peers.contains_key(u)).map(|(u,c)|(format!("{u:032x}"),c.id.clone(),c.name.clone())).collect();
            for chunk in rows.chunks(64) {if let Ok(bytes)=serde_json::to_vec(chunk){let _=self.native_net_send_to_player(delivery.player,"lunatic:character_names_v1",&bytes);}}
            // Legacy JSON rosters may exceed the old channel limit on large servers.
            // They cannot be retried successfully; the compact v2 snapshot always fits.
            if delivery.bytes.len() > horizon_protocol::mod_net::MAX_DATA {
                continue;
            }
            let channel = if delivery.legacy {
                LEGACY_CHANNEL
            } else {
                CHANNEL
            };
            if self
                .native_net_send_to_player(delivery.player, channel, &delivery.bytes)
                .is_err()
            {
                self.character_sync.server.delivery_failed(delivery.player);
            }
        }
    }
    pub(super) fn character_logged_in(&mut self, uuid: u128) {
        let path=std::path::Path::new("character-name-reservations.json");
        if let Err(e)=(|| -> io::Result<()> {let mut r=horizon_client::characters::reservations::Reservations::load(path)?;r.join(uuid);r.save(path)})(){log::error!("Cannot record character join order: {e}");}
        let saved = self.character_sync.saved.get_or_insert_with(Saved::load);
        let restored = saved
            .entries
            .get(&format!("{uuid:032x}"))
            .and_then(|id| self.pack.character_catalog.index(id));
        self.character_sync.server.join(uuid, restored);
        self.resolve_character_name(uuid);
    }
    pub(super) fn character_logged_out(&mut self, uuid: u128) {
        self.character_sync.server.leave(uuid);
        self.character_sync.legacy_requests.remove(&uuid);
    }
    pub(super) fn character_from_server(&mut self, bytes: &[u8]) {
        if let Ok(update) = Update::decode(bytes) {
            self.character_sync
                .client
                .receive(update, &self.pack.character_catalog);
        }
    }
    pub(super) fn reload_character_catalog(&mut self, old: Arc<Catalog>) {
        if old.digest == self.pack.character_catalog.digest {
            return;
        }
        self.character_sync
            .server
            .reload(&old, &self.pack.character_catalog);
        self.character_sync.client.reload();
        self.character_sync.legacy_requests.clear();
        self.character_sync.reported = None;
        self.character_sync.resolved.clear();
        for uuid in self.character_sync.server.peers.keys().copied().collect::<Vec<_>>() {self.resolve_character_name(uuid);}
    }
}
