//! Event-driven character synchronization. Ordered, reliable mod payloads carry a
//! snapshot on join and coalesced deltas afterward. No chat operation calls this.
use super::Catalog;
use horizon_protocol::{
    Error, Result,
    buf::{Reader, Writer},
};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::Duration,
};

pub const CHANNEL: &str = "lunatic:characters_v2";
pub const LEGACY_CHANNEL: &str = "lunatic:characters_v1";
pub const MAX_PEERS: usize = 1024;
const ATTEMPTS: u8 = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub sequence: u32,
    pub catalog: [u8; 32],
    pub character: u16,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Select(Selection),
    Resync,
}
impl Request {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            Self::Select(s) => {
                w.u8(0);
                w.u32(s.sequence);
                w.bytes(&s.catalog);
                w.u16(s.character);
            }
            Self::Resync => w.u8(1),
        }
        w.data
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes);
        let request = match r.u8()? {
            0 => Self::Select(Selection {
                sequence: r.u32()?,
                catalog: r.bytes(32)?.try_into().unwrap(),
                character: r.u16()?,
            }),
            1 => Self::Resync,
            _ => return Err(Error::new("invalid character request")),
        };
        if r.remaining() != 0 {
            return Err(Error::new("trailing character request data"));
        }
        Ok(request)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejection {
    Catalog,
    Character,
    Sequence,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ack {
    pub sequence: u32,
    pub result: std::result::Result<u16, Rejection>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Snapshot([u8; 32]),
    Delta,
    Ack,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Update {
    pub kind: Kind,
    pub revision: u64,
    pub changes: BTreeMap<u128, Option<u16>>,
    pub ack: Option<Ack>,
}
impl Update {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(match self.kind {
            Kind::Snapshot(_) => 0,
            Kind::Delta => 1,
            Kind::Ack => 2,
        });
        w.i64(self.revision as i64);
        if let Kind::Snapshot(digest) = self.kind {
            w.bytes(&digest);
        }
        w.u16(self.changes.len() as u16);
        for (uuid, index) in &self.changes {
            w.uuid(*uuid);
            w.u16(index.unwrap_or(u16::MAX));
        }
        write_ack(&mut w, self.ack);
        w.data
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes);
        let kind = r.u8()?;
        let revision = r.i64()? as u64;
        let kind = match kind {
            0 => Kind::Snapshot(r.bytes(32)?.try_into().unwrap()),
            1 => Kind::Delta,
            2 => Kind::Ack,
            _ => return Err(Error::new("invalid character update")),
        };
        let count = usize::from(r.u16()?);
        if count > MAX_PEERS {
            return Err(Error::new("too many character entries"));
        }
        let mut changes = BTreeMap::new();
        for _ in 0..count {
            let uuid = r.uuid()?;
            let index = r.u16()?;
            if changes
                .insert(uuid, (index != u16::MAX).then_some(index))
                .is_some()
            {
                return Err(Error::new("duplicate character entry"));
            }
        }
        let ack = if r.bool()? {
            let sequence = r.u32()?;
            let result = match r.u8()? {
                0 => Ok(r.u16()?),
                1 => Err(Rejection::Catalog),
                2 => Err(Rejection::Character),
                3 => Err(Rejection::Sequence),
                _ => return Err(Error::new("invalid character acknowledgement")),
            };
            Some(Ack { sequence, result })
        } else {
            None
        };
        if r.remaining() != 0
            || matches!(kind, Kind::Ack) && (!changes.is_empty() || ack.is_none())
            || matches!(kind, Kind::Snapshot(_)) && changes.values().any(Option::is_none)
        {
            return Err(Error::new("invalid character update body"));
        }
        Ok(Self {
            kind,
            revision,
            changes,
            ack,
        })
    }
}

fn write_ack(w: &mut Writer, ack: Option<Ack>) {
    w.bool(ack.is_some());
    if let Some(ack) = ack {
        w.u32(ack.sequence);
        match ack.result {
            Ok(index) => {
                w.u8(0);
                w.u16(index);
            }
            Err(reason) => w.u8(match reason {
                Rejection::Catalog => 1,
                Rejection::Character => 2,
                Rejection::Sequence => 3,
            }),
        }
    }
}
// Reuse the encoded roster body when a recipient needs a private acknowledgement.
// Only the final optional-ack field differs; no roster parsing or cloning is needed.
fn with_ack(base: &Arc<[u8]>, ack: Option<Ack>) -> Arc<[u8]> {
    if ack.is_none() {
        return base.clone();
    }
    debug_assert_eq!(base.last(), Some(&0));
    let mut w = Writer::new();
    w.bytes(&base[..base.len() - 1]);
    write_ack(&mut w, ack);
    w.data.into()
}

#[derive(Clone, Default, Serialize)]
pub struct Stats {
    pub sent: u64,
    pub received: u64,
    pub snapshots: u64,
    pub deltas: u64,
    pub acknowledgements: u64,
    pub retries: u64,
}
struct Pending {
    bytes: Vec<u8>,
    sent: Option<Duration>,
    attempts: u8,
}
impl Pending {
    fn new(request: Request) -> Self {
        Self {
            bytes: request.encode(),
            sent: None,
            attempts: 0,
        }
    }
    fn poll(&mut self, now: Duration) -> Option<Vec<u8>> {
        if self.attempts >= ATTEMPTS {
            return None;
        }
        if self.sent.is_some_and(|last| {
            now.saturating_sub(last) < Duration::from_secs(2u64 << self.attempts.saturating_sub(1))
        }) {
            return None;
        }
        self.sent = Some(now);
        self.attempts += 1;
        Some(self.bytes.clone())
    }
}
#[derive(Default)]
pub struct Client {
    pub peers: BTreeMap<u128, u16>,
    pub stats: Stats,
    revision: u64,
    snapshot: bool,
    authoritative: bool,
    wanted: Option<(u16, [u8; 32])>,
    sequence: u32,
    pending: Option<Pending>,
    resync: Option<Pending>,
    pub rejection: Option<Rejection>,
    blocked_catalog: Option<[u8; 32]>,
}
impl Client {
    pub fn reload(&mut self) {
        // Resource reload is not a new authenticated connection: keep sequence monotonic.
        let sequence = self.sequence;
        let stats = std::mem::take(&mut self.stats);
        *self = Self::default();
        self.sequence = sequence;
        self.stats = stats;
        self.resync = Some(Pending::new(Request::Resync));
    }
    pub fn authoritative(&self) -> bool {
        self.authoritative
    }
    pub fn poll(
        &mut self,
        own: u128,
        wanted: u16,
        digest: [u8; 32],
        now: Duration,
    ) -> Option<Vec<u8>> {
        if self.blocked_catalog == Some(digest) {
            return None;
        }
        if self.wanted != Some((wanted, digest)) {
            self.wanted = Some((wanted, digest));
            self.sequence = self.sequence.wrapping_add(1);
            self.pending = Some(Pending::new(Request::Select(Selection {
                sequence: self.sequence,
                catalog: digest,
                character: wanted,
            })));
            self.rejection = None;
        }
        // A restored server-approved choice in the initial snapshot needs no request.
        if self.snapshot && self.peers.get(&own) == Some(&wanted) {
            self.pending = None;
        }
        if self.rejection == Some(Rejection::Catalog) {
            return None;
        }
        let pending = self.resync.as_mut().or(self.pending.as_mut())?;
        let retry = pending.attempts > 0;
        let data = pending.poll(now)?;
        self.stats.sent += 1;
        self.stats.retries += u64::from(retry);
        self.authoritative = true;
        Some(data)
    }
    pub fn send_failed(&mut self) {
        if let Some(pending) = self.resync.as_mut().or(self.pending.as_mut()) {
            self.stats.sent = self.stats.sent.saturating_sub(1);
            self.stats.retries = self
                .stats
                .retries
                .saturating_sub(u64::from(pending.attempts > 1));
            pending.attempts = pending.attempts.saturating_sub(1);
            pending.sent = None;
        }
    }
    pub fn receive(&mut self, update: Update, catalog: &Catalog) -> bool {
        self.stats.received += 1;
        if let Kind::Snapshot(digest) = update.kind {
            self.authoritative = true;
            if digest != catalog.digest {
                self.peers.clear();
                self.pending = None;
                self.resync = None;
                self.rejection = Some(Rejection::Catalog);
                self.blocked_catalog = Some(catalog.digest);
                return false;
            }
            if self.blocked_catalog.take().is_some() {
                self.rejection = None;
                self.wanted = None;
            }
        }
        if update
            .changes
            .values()
            .flatten()
            .any(|i| catalog.get(*i).is_none())
        {
            return false;
        }
        if update.revision < self.revision
            || matches!(update.kind, Kind::Delta)
                && self.snapshot
                && update.revision == self.revision
        {
            return false;
        }
        match update.kind {
            Kind::Snapshot(_) => {
                self.stats.snapshots += 1;
                self.peers = update
                    .changes
                    .into_iter()
                    .filter_map(|(u, i)| Some((u, i?)))
                    .collect();
                self.snapshot = true;
                self.resync = None;
                self.revision = update.revision;
            }
            Kind::Delta => {
                if !self.snapshot || update.revision != self.revision + 1 {
                    self.resync
                        .get_or_insert_with(|| Pending::new(Request::Resync));
                    return false;
                }
                self.stats.deltas += 1;
                for (uuid, index) in update.changes {
                    if let Some(i) = index {
                        self.peers.insert(uuid, i);
                    } else {
                        self.peers.remove(&uuid);
                    }
                }
                self.revision = update.revision;
            }
            Kind::Ack => {
                if self.snapshot && update.revision > self.revision {
                    self.resync
                        .get_or_insert_with(|| Pending::new(Request::Resync));
                }
            }
        }
        if let Some(ack) = update.ack {
            self.stats.acknowledgements += 1;
            if ack.sequence == self.sequence {
                self.pending = None;
                if let Err(reason) = ack.result {
                    self.rejection = Some(reason);
                    if reason == Rejection::Catalog {
                        self.blocked_catalog = Some(catalog.digest);
                    }
                }
            }
        }
        true
    }
}

pub struct Delivery {
    pub player: u128,
    pub legacy: bool,
    pub bytes: Arc<[u8]>,
}
#[derive(Default)]
pub struct Server {
    pub peers: BTreeMap<u128, u16>,
    online: BTreeSet<u128>,
    confirmed: BTreeSet<u128>,
    legacy: BTreeSet<u128>,
    full: BTreeSet<u128>,
    dirty: BTreeMap<u128, Option<u16>>,
    acknowledgements: BTreeMap<u128, Ack>,
    requests: BTreeMap<u128, (Selection, Ack)>,
    revision: u64,
    snapshot_cache: Option<Arc<[u8]>>,
    legacy_cache: Option<Arc<[u8]>>,
}
impl Server {
    pub fn join(&mut self, uuid: u128, restored: Option<u16>) {
        if self.online.len() >= MAX_PEERS && !self.online.contains(&uuid) {
            return;
        }
        self.online.insert(uuid);
        self.full.insert(uuid);
        if let Some(index) = restored {
            // Keep the preference available to the server, but do not advertise
            // it as this connection's choice before the client confirms it.
            self.peers.insert(uuid, index);
        }
    }
    /// A valid selection received on this connection, rather than a restored
    /// preference. Retransmits and unchanged selections still confirm identity.
    pub fn selection_confirmed(&self, uuid: u128) -> bool {
        self.online.contains(&uuid) && self.confirmed.contains(&uuid)
    }

    fn set(&mut self, uuid: u128, index: u16) -> bool {
        let changed = self.peers.insert(uuid, index) != Some(index);
        let first_confirmation = self.confirmed.insert(uuid);
        if changed || first_confirmation {
            self.dirty.insert(uuid, Some(index));
        }
        changed
    }
    pub fn leave(&mut self, uuid: u128) {
        self.online.remove(&uuid);
        let confirmed = self.confirmed.remove(&uuid);
        self.legacy.remove(&uuid);
        self.full.remove(&uuid);
        self.acknowledgements.remove(&uuid);
        self.requests.remove(&uuid);
        if self.peers.remove(&uuid).is_some() && confirmed {
            self.dirty.insert(uuid, None);
        }
    }
    pub fn select(&mut self, uuid: u128, request: Request, catalog: &Catalog) -> bool {
        if !self.online.contains(&uuid) {
            return false;
        }
        let Request::Select(selection) = request else {
            self.full.insert(uuid);
            return false;
        };
        self.legacy.remove(&uuid);
        if let Some((previous, ack)) = self.requests.get(&uuid) {
            if previous == &selection {
                self.acknowledgements.insert(uuid, *ack);
                return false;
            }
            if selection.sequence <= previous.sequence {
                self.acknowledgements.insert(
                    uuid,
                    Ack {
                        sequence: selection.sequence,
                        result: Err(Rejection::Sequence),
                    },
                );
                return false;
            }
        }
        let result = if selection.catalog != catalog.digest {
            Err(Rejection::Catalog)
        } else if catalog.get(selection.character).is_none() {
            Err(Rejection::Character)
        } else {
            Ok(selection.character)
        };
        let ack = Ack {
            sequence: selection.sequence,
            result,
        };
        self.acknowledgements.insert(uuid, ack);
        self.requests.insert(uuid, (selection, ack));
        result.is_ok_and(|index| self.set(uuid, index))
    }
    pub fn select_legacy(&mut self, uuid: u128, index: u16) -> bool {
        if !self.online.contains(&uuid) {
            return false;
        }
        if self.legacy.insert(uuid) {
            self.full.insert(uuid);
        }
        self.set(uuid, index)
    }
    pub fn delivery_failed(&mut self, uuid: u128) {
        if self.online.contains(&uuid) {
            self.full.insert(uuid);
            if let Some((_, ack)) = self.requests.get(&uuid) {
                self.acknowledgements.insert(uuid, *ack);
            }
        }
    }
    pub fn reload(&mut self, old: &Catalog, new: &Catalog) {
        self.peers = self
            .peers
            .iter()
            .filter_map(|(uuid, index)| Some((*uuid, new.index(&old.get(*index)?.id)?)))
            .collect();
        self.full.extend(&self.online);
        self.dirty.clear();
        self.requests.clear();
        self.acknowledgements.clear();
        self.revision += 1;
        self.snapshot_cache = None;
        self.legacy_cache = None;
    }
    pub fn flush(&mut self, catalog: &Catalog) -> Vec<Delivery> {
        if self.dirty.is_empty() && self.full.is_empty() && self.acknowledgements.is_empty() {
            return Vec::new();
        }
        let changed = !self.dirty.is_empty();
        if changed {
            self.revision += 1;
            self.snapshot_cache = None;
            self.legacy_cache = None;
        }
        let mut deliveries = Vec::new();
        let delta_bytes: Option<Arc<[u8]>> =
            (changed && self.dirty.len() <= MAX_PEERS).then(|| {
                Update {
                    kind: Kind::Delta,
                    revision: self.revision,
                    changes: self.dirty.clone(),
                    ack: None,
                }
                .encode()
                .into()
            });
        for uuid in &self.online {
            let ack = self.acknowledgements.remove(uuid);
            let full = self.full.contains(uuid) || self.dirty.len() > MAX_PEERS;
            if self.legacy.contains(uuid) {
                if changed || full {
                    let bytes = self.legacy_cache.get_or_insert_with(|| {
                        let roster: BTreeMap<String, &str> = self
                            .peers
                            .iter()
                            .filter_map(|(u, i)| {
                                if !self.confirmed.contains(u) {
                                    return None;
                                }
                                Some((format!("{u:032x}"), catalog.get(*i)?.id.as_str()))
                            })
                            .collect();
                        serde_json::to_vec(&roster).unwrap().into()
                    });
                    deliveries.push(Delivery {
                        player: *uuid,
                        legacy: true,
                        bytes: bytes.clone(),
                    });
                }
            } else if full {
                let base = self.snapshot_cache.get_or_insert_with(|| {
                    Update {
                        kind: Kind::Snapshot(catalog.digest),
                        revision: self.revision,
                        changes: self
                            .peers
                            .iter()
                            .filter(|(uuid, _)| self.confirmed.contains(*uuid))
                            .map(|(uuid, index)| (*uuid, Some(*index)))
                            .collect(),
                        ack: None,
                    }
                    .encode()
                    .into()
                });
                let bytes = with_ack(base, ack);
                deliveries.push(Delivery {
                    player: *uuid,
                    legacy: false,
                    bytes,
                });
            } else if changed {
                let bytes = with_ack(delta_bytes.as_ref().expect("bounded delta"), ack);
                deliveries.push(Delivery {
                    player: *uuid,
                    legacy: false,
                    bytes,
                });
            } else if ack.is_some() {
                deliveries.push(Delivery {
                    player: *uuid,
                    legacy: false,
                    bytes: Update {
                        kind: Kind::Ack,
                        revision: self.revision,
                        changes: BTreeMap::new(),
                        ack,
                    }
                    .encode()
                    .into(),
                });
            }
        }
        self.full.clear();
        self.dirty.clear();
        deliveries
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn catalog() -> Catalog {
        Catalog::new(["reimu", "marisa"].into_iter().map(|id| serde_json::from_value(serde_json::json!({
            "id": format!("lunatic:{id}"), "name": id, "texture": format!("lunatic:entity/character/{id}")
        })).unwrap()).collect())
    }
    fn select(c: &Catalog, sequence: u32, character: u16) -> Request {
        Request::Select(Selection {
            sequence,
            character,
            catalog: c.digest,
        })
    }
    fn receive(client: &mut Client, messages: &[Delivery], uuid: u128, c: &Catalog) {
        for m in messages.iter().filter(|m| m.player == uuid) {
            assert!(client.receive(Update::decode(&m.bytes).unwrap(), c));
        }
    }
    #[test]
    fn reconnect_waits_for_current_selection_and_keeps_reserved_character_suffix() {
        let c = Catalog::new(
            [("satori", "Komeiji Satori"), ("cirno", "Cirno")]
                .into_iter()
                .map(|(id, name)| {
                    serde_json::from_value(serde_json::json!({
                        "id": format!("lunatic:{id}"), "name": name,
                        "texture": format!("lunatic:entity/character/{id}")
                    }))
                    .unwrap()
                })
                .collect(),
        );
        let satori = c.index("lunatic:satori").unwrap();
        let cirno = c.index("lunatic:cirno").unwrap();
        let mut names = crate::characters::reservations::Reservations::default();
        assert_eq!(names.reserve(1, "lunatic:cirno"), 0);
        let slot = names.reserve(2, "lunatic:cirno");
        let mut server = Server::default();
        server.join(2, Some(satori));
        assert!(!server.selection_confirmed(2));
        assert!(server.select(2, select(&c, 1, cirno), &c));
        assert!(server.selection_confirmed(2));
        assert_eq!(
            format!("Cirno{}", crate::characters::reservations::suffix(slot)),
            "Cirno (B)"
        );
        server.leave(2);
        assert!(!server.selection_confirmed(2));
        // An old saved character or roster sync cannot announce the new session.
        server.join(2, Some(satori));
        assert_eq!(server.peers.get(&2), Some(&satori));
        assert!(!server.selection_confirmed(2));
        assert!(!server.select(2, Request::Resync, &c));
        assert!(!server.selection_confirmed(2));
        let mut invalid = select(&c, 1, cirno);
        let Request::Select(selection) = &mut invalid else {
            unreachable!()
        };
        selection.catalog = [0; 32];
        assert!(!server.select(2, invalid, &c));
        assert!(!server.selection_confirmed(2));
        assert!(server.select(2, select(&c, 2, cirno), &c));
        assert!(server.selection_confirmed(2));
        let character = c.get(server.peers[&2]).unwrap();
        let name = format!(
            "{}{}",
            character.name,
            crate::characters::reservations::suffix(names.reserve(2, &character.id))
        );
        assert_eq!(name, "Cirno (B)");
    }

    #[test]
    fn unchanged_selection_confirms_reconnect_without_changing_preference() {
        let c = catalog();
        let mut server = Server::default();
        server.join(1, Some(0));
        let mut client = Client::default();
        receive(&mut client, &server.flush(&c), 1, &c);
        assert!(!client.peers.contains_key(&1));
        assert!(!server.selection_confirmed(1));
        // The existing client's snapshot shortcut must not suppress this request.
        let request = client.poll(1, 0, c.digest, Duration::ZERO).unwrap();
        assert!(!server.select(1, Request::decode(&request).unwrap(), &c));
        assert!(server.selection_confirmed(1));
        assert!(!server.select(1, select(&c, 1, 0), &c));
        assert!(server.selection_confirmed(1));
        receive(&mut client, &server.flush(&c), 1, &c);
        assert_eq!(client.peers.get(&1), Some(&0));
        assert!(
            client
                .poll(1, 0, c.digest, Duration::from_secs(60))
                .is_none()
        );
        assert_eq!(client.stats.sent, 1);
        server.leave(1);
        server.join(1, Some(0));
        assert!(!server.selection_confirmed(1));
    }

    #[test]
    fn legacy_selection_confirmation_is_scoped_to_the_connection() {
        let c = catalog();
        let mut server = Server::default();
        server.join(1, Some(0));
        assert!(!server.selection_confirmed(1));
        assert!(!server.select_legacy(1, 0));
        assert!(server.selection_confirmed(1));
        server.join(2, Some(1));
        let messages = server.flush(&c);
        let legacy = messages.iter().find(|message| message.player == 1).unwrap();
        let roster: serde_json::Value = serde_json::from_slice(&legacy.bytes).unwrap();
        assert!(roster.get(format!("{:032x}", 2).as_str()).is_none());
        server.leave(1);
        server.join(1, Some(0));
        assert!(!server.selection_confirmed(1));
    }

    #[test]
    fn handshake_then_silent_idle_and_cached_late_join_delta() {
        let c = catalog();
        let mut server = Server::default();
        let mut alice = Client::default();
        server.join(1, None);
        receive(&mut alice, &server.flush(&c), 1, &c);
        let request = alice.poll(1, 0, c.digest, Duration::ZERO).unwrap();
        assert_eq!(request.len(), 39);
        assert!(server.select(1, Request::decode(&request).unwrap(), &c));
        let update = server.flush(&c);
        assert_eq!(update.len(), 1);
        assert_eq!(update[0].bytes.len(), 37);
        receive(&mut alice, &update, 1, &c);
        for second in 0..120 {
            assert!(
                alice
                    .poll(1, 0, c.digest, Duration::from_secs(second))
                    .is_none()
            );
            assert!(server.flush(&c).is_empty());
        }
        assert_eq!(alice.stats.sent, 1);
        assert_eq!(alice.stats.retries, 0);
        server.join(2, Some(1));
        let mut bob = Client::default();
        let initial = server.flush(&c);
        assert_eq!(initial.len(), 1);
        assert_eq!(initial[0].player, 2);
        receive(&mut bob, &initial, 2, &c);
        assert!(!bob.peers.contains_key(&2));
        let choice = bob.poll(2, 1, c.digest, Duration::ZERO).unwrap();
        assert!(!server.select(2, Request::decode(&choice).unwrap(), &c));
        let messages = server.flush(&c);
        assert_eq!(messages.len(), 2);
        assert_eq!(
            Update::decode(&messages[0].bytes).unwrap().kind,
            Kind::Delta
        );
        receive(&mut alice, &messages, 1, &c);
        receive(&mut bob, &messages, 2, &c);
        assert_eq!(alice.peers, bob.peers);
        assert!(bob.poll(2, 1, c.digest, Duration::ZERO).is_none());
        server.leave(2);
        receive(&mut alice, &server.flush(&c), 1, &c);
        assert_eq!(alice.peers, BTreeMap::from([(1, 0)]));
    }
    #[test]
    fn duplicate_rejected_and_stale_selections_do_not_rebroadcast() {
        let c = catalog();
        let mut s = Server::default();
        s.join(1, None);
        s.join(2, None);
        s.flush(&c);
        assert!(s.select(1, select(&c, 2, 0), &c));
        s.flush(&c);
        assert!(!s.select(1, select(&c, 2, 0), &c));
        let ack = s.flush(&c);
        assert_eq!(ack.len(), 1);
        assert_eq!(ack[0].player, 1);
        assert_eq!(Update::decode(&ack[0].bytes).unwrap().kind, Kind::Ack);
        for request in [select(&c, 1, 1), select(&c, 2, 1), select(&c, 3, u16::MAX)] {
            assert!(!s.select(1, request, &c));
            let ack = s.flush(&c);
            assert_eq!(ack.len(), 1);
            assert!(
                Update::decode(&ack[0].bytes)
                    .unwrap()
                    .ack
                    .unwrap()
                    .result
                    .is_err()
            );
        }
        assert_eq!(s.peers.get(&1), Some(&0));
    }
    #[test]
    fn bounded_retry_backpressure_and_revision_gap_recovery() {
        let c = catalog();
        let mut client = Client::default();
        assert!(client.poll(1, 0, c.digest, Duration::ZERO).is_some());
        client.send_failed();
        assert_eq!(client.stats.sent, 0);
        assert!(client.poll(1, 0, c.digest, Duration::ZERO).is_some());
        for second in [2, 6, 14] {
            assert!(
                client
                    .poll(1, 0, c.digest, Duration::from_secs(second))
                    .is_some()
            );
        }
        assert!(
            client
                .poll(1, 0, c.digest, Duration::from_secs(100))
                .is_none()
        );
        assert_eq!(client.stats.sent, 4);
        assert_eq!(client.stats.retries, 3);
        let snapshot = Update {
            kind: Kind::Snapshot(c.digest),
            revision: 1,
            changes: BTreeMap::from([(1, Some(0))]),
            ack: None,
        };
        assert!(client.receive(snapshot, &c));
        assert!(!client.receive(
            Update {
                kind: Kind::Delta,
                revision: 3,
                changes: BTreeMap::new(),
                ack: None
            },
            &c
        ));
        let request = client
            .poll(1, 0, c.digest, Duration::from_secs(101))
            .unwrap();
        assert_eq!(Request::decode(&request).unwrap(), Request::Resync);
        assert!(client.receive(
            Update {
                kind: Kind::Snapshot(c.digest),
                revision: 3,
                changes: BTreeMap::from([(1, Some(0))]),
                ack: None
            },
            &c
        ));
        assert!(
            client
                .poll(1, 0, c.digest, Duration::from_secs(200))
                .is_none()
        );
    }
    #[test]
    fn catalog_rejection_is_terminal_until_catalog_changes() {
        let c = catalog();
        let mut client = Client::default();
        let mut server = Server::default();
        server.join(1, None);
        let request =
            Request::decode(&client.poll(1, 0, c.digest, Duration::ZERO).unwrap()).unwrap();
        let mut wrong = catalog();
        wrong.digest = [3; 32];
        server.select(1, request, &wrong);
        for message in server.flush(&wrong) {
            client.receive(Update::decode(&message.bytes).unwrap(), &c);
        }
        assert_eq!(client.rejection, Some(Rejection::Catalog));
        for second in 1..60 {
            assert!(
                client
                    .poll(1, 0, c.digest, Duration::from_secs(second))
                    .is_none()
            );
        }
        assert!(
            client
                .poll(1, 1, c.digest, Duration::from_secs(60))
                .is_none()
        );
        assert!(
            client
                .poll(1, 1, wrong.digest, Duration::from_secs(61))
                .is_some()
        );
    }
    #[test]
    fn reload_reindexes_by_id_and_texture_only_reload_is_stable() {
        let old = catalog();
        let new = Catalog::new(old.characters.iter().rev().cloned().collect());
        let mut server = Server::default();
        server.join(1, Some(0));
        server.select(1, select(&old, 1, 0), &old);
        server.flush(&old);
        server.reload(&old, &new);
        assert_eq!(server.peers.get(&1), Some(&1));
        let update = server.flush(&new);
        assert_eq!(update.len(), 1);
        assert_eq!(
            Update::decode(&update[0].bytes).unwrap().kind,
            Kind::Snapshot(new.digest)
        );
        assert_eq!(Catalog::new(old.characters.clone()).digest, old.digest);
    }
    #[test]
    fn client_reload_preserves_connection_sequence_and_recovers_rejection() {
        let c = catalog();
        let mut server = Server::default();
        let mut client = Client::default();
        server.join(1, None);
        let request =
            Request::decode(&client.poll(1, 0, c.digest, Duration::ZERO).unwrap()).unwrap();
        server.select(1, request, &c);
        receive(&mut client, &server.flush(&c), 1, &c);
        client.reload();
        let request =
            Request::decode(&client.poll(1, 1, c.digest, Duration::from_secs(1)).unwrap()).unwrap();
        assert_eq!(request, Request::Resync);
        server.select(1, request, &c);
        receive(&mut client, &server.flush(&c), 1, &c);
        let request =
            Request::decode(&client.poll(1, 1, c.digest, Duration::from_secs(1)).unwrap()).unwrap();
        assert!(server.select(1, request, &c));
        receive(&mut client, &server.flush(&c), 1, &c);
        assert_eq!(client.peers.get(&1), Some(&1));
        client.receive(
            Update {
                kind: Kind::Snapshot([3; 32]),
                revision: 9,
                changes: BTreeMap::new(),
                ack: None,
            },
            &c,
        );
        assert!(
            client
                .poll(1, 1, c.digest, Duration::from_secs(2))
                .is_none()
        );
        client.receive(
            Update {
                kind: Kind::Snapshot(c.digest),
                revision: 10,
                changes: BTreeMap::new(),
                ack: None,
            },
            &c,
        );
        assert!(
            client
                .poll(1, 1, c.digest, Duration::from_secs(3))
                .is_some()
        );
    }
    #[test]
    fn maximum_snapshot_fits_channel_and_broadcast_shares_encoding() {
        let c = catalog();
        let mut server = Server::default();
        for uuid in 0..MAX_PEERS as u128 {
            server.join(uuid, Some(0));
            server.select(uuid, select(&c, 1, 0), &c);
        }
        server.flush(&c);
        for uuid in 0..MAX_PEERS as u128 {
            server.select(uuid, Request::Resync, &c);
        }
        let messages = server.flush(&c);
        assert_eq!(messages.len(), MAX_PEERS);
        assert!(messages[0].bytes.len() < horizon_protocol::mod_net::MAX_DATA);
        assert!(Arc::ptr_eq(&messages[0].bytes, &messages[1].bytes));
        assert_eq!(
            Update::decode(&messages[0].bytes).unwrap().changes.len(),
            MAX_PEERS
        );
        let mut extra = messages[0].bytes.to_vec();
        extra.push(0);
        assert!(Update::decode(&extra).is_err());
        assert!(Request::decode(&[0, 0]).is_err());
        assert!(Request::decode(&[1, 0]).is_err());
    }
    #[test]
    fn legacy_polling_no_longer_generates_replies_or_writes() {
        let c = catalog();
        let mut server = Server::default();
        server.join(1, None);
        server.flush(&c);
        assert!(server.select_legacy(1, 0));
        let messages = server.flush(&c);
        assert_eq!(messages.len(), 1);
        assert!(messages[0].legacy);
        for _ in 0..10 {
            assert!(!server.select_legacy(1, 0));
            assert!(server.flush(&c).is_empty());
        }
        server.delivery_failed(1);
        assert_eq!(server.flush(&c).len(), 1);
    }
}
