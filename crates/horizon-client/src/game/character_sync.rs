//! Server-approved character IDs, carried over Horizon's existing mod channel.
use super::Game;
use horizon_client::characters::{self, Character};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

pub(super) const CHANNEL: &str = "lunatic:characters_v1";
#[derive(Default)]
pub(super) struct State {
    pub peers: BTreeMap<u128, Character>,
    saved: Option<BTreeMap<String, String>>,
    last_sent: Option<Instant>,
    requests: BTreeMap<u128, Instant>,
}

impl Game<'_> {
    pub(super) fn character_for(&self, uuid: u128) -> Option<&Character> {
        self.character_sync.peers.get(&uuid).or_else(|| {
            // Keep singleplayer/vanilla compatibility until an authoritative snapshot arrives.
            (self.character_sync.last_sent.is_none()
                && self.remote.as_ref().is_some_and(|r| r.profile() == uuid))
            .then_some(self.character.as_ref())
            .flatten()
        })
    }
    pub(super) fn character_for_name(&self, name: &str) -> Option<&Character> {
        let uuid = self.remote.as_ref()?.character_uuid(name)?;
        self.character_for(uuid)
    }
    pub(super) fn tick_character_sync(&mut self) {
        let Some(id) = self.character.as_ref().map(|c| c.id.clone()) else {
            return;
        };
        if self
            .character_sync
            .last_sent
            .is_some_and(|t| t.elapsed() < Duration::from_secs(2))
        {
            return;
        }
        let data = serde_json::to_vec(&characters::SelectionRequest {
            character: id,
            catalog: characters::catalog_digest(&characters::roster(&self.options.resources)),
        })
        .unwrap();
        if self.native_net_send_to_server(CHANNEL, &data).is_ok() {
            self.character_sync.last_sent = Some(Instant::now());
        }
    }
    pub(super) fn character_from_client(&mut self, uuid: u128, bytes: &[u8]) {
        if bytes.len() > 256 {
            return;
        }
        if self
            .character_sync
            .requests
            .get(&uuid)
            .is_some_and(|t| t.elapsed() < Duration::from_secs(1))
        {
            return;
        }
        self.character_sync.requests.insert(uuid, Instant::now());
        let roster = characters::roster(&self.options.resources);
        let Some(character) = characters::resolve_selection(bytes, &roster) else {
            log::warn!("Rejected unknown character or mismatched catalog for player {uuid:032x}");
            self.native_tell(uuid, horizon_client::text::Text::literal("Lunatic character selection rejected: install the server's matching character catalog."));
            return;
        };
        let id = character.id.clone();
        let saved = self.character_sync.saved.get_or_insert_with(|| {
            std::fs::read("character-selections.json")
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default()
        });
        let changed = saved.get(&format!("{uuid:032x}")) != Some(&id)
            || !self.character_sync.peers.contains_key(&uuid);
        saved.insert(format!("{uuid:032x}"), id);
        self.character_sync.peers.insert(uuid, character);
        if changed {
            if let Ok(bytes) = serde_json::to_vec_pretty(saved) {
                if let Err(e) = std::fs::write(".character-selections.tmp", bytes).and_then(|_| {
                    std::fs::rename(".character-selections.tmp", "character-selections.json")
                }) {
                    log::error!("Could not save server character selections: {e}");
                }
            }
        }
        if changed {
            log::info!(
                "Character selection {uuid:032x}: {}",
                self.character_sync.peers[&uuid].id
            );
        }
        self.publish_characters(if changed { None } else { Some(uuid) });
    }
    fn publish_characters(&mut self, recipient: Option<u128>) {
        let players = self.native_server_players();
        let snapshot: BTreeMap<String, String> = players
            .iter()
            .filter_map(|p| {
                self.character_sync
                    .peers
                    .get(&p.uuid)
                    .map(|c| (format!("{:032x}", p.uuid), c.id.clone()))
            })
            .collect();
        let Ok(data) = serde_json::to_vec(&snapshot) else {
            return;
        };
        for player in players {
            if recipient.is_none_or(|u| u == player.uuid) {
                let _ = self.native_net_send_to_player(player.uuid, CHANNEL, &data);
            }
        }
    }
    pub(super) fn character_logged_in(&mut self, uuid: u128) {
        let saved = self.character_sync.saved.get_or_insert_with(|| {
            std::fs::read("character-selections.json")
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default()
        });
        if let Some(id) = saved.get(&format!("{uuid:032x}")) {
            if let Some(c) = characters::roster(&self.options.resources)
                .into_iter()
                .find(|c| &c.id == id)
            {
                self.character_sync.peers.insert(uuid, c);
            }
        }
        self.publish_characters(None);
    }
    pub(super) fn character_logged_out(&mut self, uuid: u128) {
        self.character_sync.peers.remove(&uuid);
        self.character_sync.requests.remove(&uuid);
        self.publish_characters(None);
    }

    pub(super) fn character_from_server(&mut self, bytes: &[u8]) {
        let Ok(snapshot) = serde_json::from_slice::<BTreeMap<String, String>>(bytes) else {
            return;
        };
        if snapshot.len() > 1024 {
            return;
        }
        let roster = characters::roster(&self.options.resources);
        let mut peers = BTreeMap::new();
        for (uuid, id) in snapshot {
            let Ok(uuid) = u128::from_str_radix(&uuid, 16) else {
                return;
            };
            if let Some(c) = roster.iter().find(|c| c.id == id) {
                peers.insert(uuid, c.clone());
            }
        }
        self.character_sync.peers = peers;
    }
}
