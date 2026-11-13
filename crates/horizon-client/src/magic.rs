//! Content-defined, permanently unlocked spells. The server owns the ledger;
//! virtual slots never contain ItemStacks or participate in container transactions.
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

pub const CHANNEL: &str = "horizon_magic:spells";
pub const SLOT_COUNT: usize = 144;
pub const PAGE_SIZE: usize = 36;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Page {
    pub icon: String,
    pub title: String,
    pub color: u32,
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Giant,
    Tiny,
    Effect,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Spell {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub icon: String,
    pub color: u32,
    pub kind: Kind,
    pub hunger: i32,
    pub scale: f64,
    pub transition_ticks: u32,
    pub duration_ticks: u32,
    pub ending_ticks: u32,
    /// Transition, active, and returning markers, in that order.
    pub effects: [String; 3],
    #[serde(default)]
    pub start_sound: Option<String>,
    #[serde(default)]
    pub end_sound: Option<String>,
    #[serde(default)]
    pub particle_color: Option<[f32; 3]>,
}
impl Spell {
    pub fn total_ticks(&self) -> u32 {
        self.transition_ticks + self.duration_ticks + self.ending_ticks
    }
    pub fn phase(&self, age: u32) -> Phase {
        if age < self.transition_ticks {
            Phase::Changing
        } else if age < self.transition_ticks + self.duration_ticks {
            Phase::Active
        } else if age < self.total_ticks() {
            Phase::Returning
        } else {
            Phase::Finished
        }
    }
    pub fn scale_at(&self, age: u32) -> f64 {
        let smooth = |t: f64| {
            let t = t.clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        match self.phase(age) {
            Phase::Changing => {
                1.0 + (self.scale - 1.0) * smooth(f64::from(age) / f64::from(self.transition_ticks))
            }
            Phase::Active => self.scale,
            Phase::Returning => {
                self.scale
                    + (1.0 - self.scale)
                        * smooth(
                            f64::from(age - self.transition_ticks - self.duration_ticks)
                                / f64::from(self.ending_ticks),
                        )
            }
            Phase::Finished => 1.0,
        }
    }
    fn valid(&self, namespace: &str) -> bool {
        self.id.starts_with(&format!("{namespace}:"))
            && identifier(&self.id)
            && identifier(&self.icon)
            && self.name.len() <= 80
            && !self.name.is_empty()
            && self.description.len() <= 2048
            && (0..=20).contains(&self.hunger)
            && self.scale.is_finite()
            && (0.125..=8.0).contains(&self.scale)
            && (1..=200).contains(&self.transition_ticks)
            && (1..=1200).contains(&self.duration_ticks)
            && (1..=200).contains(&self.ending_ticks)
            && self
                .effects
                .iter()
                .all(|id| identifier(id) && id.starts_with(&format!("{namespace}:")))
    }
}
fn identifier(id: &str) -> bool {
    id.len() <= 160
        && id.split_once(':').is_some_and(|(ns, path)| {
            !ns.is_empty()
                && !path.is_empty()
                && ns
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_-.".contains(&b))
                && path
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_-./".contains(&b))
        })
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Changing,
    Active,
    Returning,
    Finished,
}
#[derive(Default)]
pub struct Catalog {
    pub page: Option<Page>,
    pub spells: BTreeMap<String, Spell>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Definitions {
    page: Page,
    spells: Vec<Spell>,
}
impl Catalog {
    pub fn load(root: &Path) -> Self {
        let mut catalog = Self::default();
        for namespace in crate::mod_content::namespaces(root) {
            let path = root
                .join("assets")
                .join(&namespace)
                .join("horizon/magic.json");
            let Ok(bytes) = horizon_core::prepared_files::read(&path) else {
                continue;
            };
            let Ok(defs) = serde_json::from_slice::<Definitions>(&bytes) else {
                log::warn!("Invalid magic catalog for {namespace}");
                continue;
            };
            if !identifier(&defs.page.icon) || defs.page.title.len() > 40 {
                continue;
            }
            if catalog.page.is_none() {
                catalog.page = Some(defs.page);
            }
            for spell in defs.spells {
                if catalog.spells.len() < 128 && spell.valid(&namespace) {
                    catalog.spells.entry(spell.id.clone()).or_insert(spell);
                }
            }
        }
        catalog
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Ledger {
    /// Spell inventories are Soulbound to this character, not the account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub character: Option<String>,
    pub revision: u64,
    pub slots: Vec<Option<String>>,
}
impl Default for Ledger {
    fn default() -> Self {
        Self {
            character: None,
            revision: 0,
            slots: vec![None; SLOT_COUNT],
        }
    }
}
impl Ledger {
    pub fn valid(&self) -> bool {
        let mut seen = BTreeSet::new();
        self.character.as_ref().is_none_or(|id| identifier(id))
            && self.slots.len() == SLOT_COUNT
            && self
                .slots
                .iter()
                .flatten()
                .all(|id| identifier(id) && seen.insert(id))
    }
    /// Grant catalog-approved spells to the current Soulbound ledger. Existing
    /// positions survive reconnects; bind_character handles character switches.
    pub fn grant(&mut self, ids: &[String], catalog: &Catalog) -> bool {
        let mut changed = false;
        for id in ids {
            if catalog.spells.contains_key(id)
                && !self.contains(id)
                && let Some(empty) = self.slots.iter_mut().find(|s| s.is_none())
            {
                *empty = Some(id.clone());
                changed = true;
            }
        }
        if changed {
            self.revision = self.revision.wrapping_add(1);
        }
        changed
    }
    /// Reconcile ownership before accepting any queued inventory/cast request.
    /// A legacy unowned ledger keeps current starters in place, removing stale
    /// spells of other characters without resetting the player's arrangement.
    pub fn bind_character(&mut self, character: Option<&str>, starters: &[String], catalog: &Catalog) -> bool {
        if self.character.as_deref() == character && self.character.is_some() { return false; }
        let switched = self.character.is_some();
        for slot in &mut self.slots {
            if switched || slot.as_ref().is_some_and(|id| catalog.spells.contains_key(id) && !starters.contains(id)) { *slot = None; }
        }
        self.character = character.map(str::to_owned);
        self.revision = self.revision.wrapping_add(1);
        true
    }
    pub fn contains(&self, id: &str) -> bool {
        self.slots.iter().flatten().any(|s| s == id)
    }
    pub fn swap(&mut self, from: usize, to: usize, revision: u64) -> bool {
        if revision != self.revision
            || from >= SLOT_COUNT
            || to >= SLOT_COUNT
            || from == to
            || self.slots[from].is_none()
        {
            return false;
        }
        self.slots.swap(from, to);
        self.revision = self.revision.wrapping_add(1);
        true
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Open,
    Swap {
        from: usize,
        to: usize,
        revision: u64,
    },
    Cast {
        id: String,
    },
}
impl Request {
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > 4096 {
            return None;
        }
        let request: Self = serde_json::from_slice(bytes).ok()?;
        match &request {
            Self::Cast { id } if !identifier(id) => None,
            _ => Some(request),
        }
    }
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Update {
    #[serde(default)]
    pub swap_revision: Option<u64>,
    pub ledger: Ledger,
    pub started: bool,
    pub error: Option<String>,
}

/// Client-side prediction over a server-owned ledger. Only one swap is sent at
/// once; further drags display immediately and replay after each acknowledgement.
#[derive(Default)]
pub struct Prediction {
    pub ledger: Ledger,
    confirmed: Ledger,
    pending: std::collections::VecDeque<Swap>,
}
#[derive(Clone)]
struct Swap {
    from: usize,
    to: usize,
    revision: u64,
}
impl Swap {
    fn request(&self) -> Request {
        Request::Swap {
            from: self.from,
            to: self.to,
            revision: self.revision,
        }
    }
}
impl Prediction {
    pub fn pending(&self) -> bool {
        !self.pending.is_empty()
    }
    pub fn swap(&mut self, from: usize, to: usize) -> Option<Request> {
        if self.pending.len() >= 64 {
            return None;
        }
        let revision = self.ledger.revision;
        if !self.ledger.swap(from, to, revision) {
            return None;
        }
        self.pending.push_back(Swap { from, to, revision });
        (self.pending.len() == 1).then(|| self.pending.front().unwrap().request())
    }
    pub fn reset(&mut self) {
        self.pending.clear();
        self.ledger = self.confirmed.clone();
    }
    pub fn receive(&mut self, update: &Update) -> Option<Request> {
        if let Some(front) = self.pending.front() {
            // Unrelated/stale replies must never overwrite predicted positions.
            if update.swap_revision != Some(front.revision) {
                return None;
            }
            let accepted =
                update.error.is_none() && update.ledger.revision == front.revision.wrapping_add(1);
            self.pending.pop_front();
            if !accepted {
                self.pending.clear();
            }
        }
        self.confirmed = update.ledger.clone();
        self.ledger = self.confirmed.clone();
        self.pending.retain_mut(|swap| {
            swap.revision = self.ledger.revision;
            self.ledger.swap(swap.from, swap.to, swap.revision)
        });
        self.pending.front().map(Swap::request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn spell() -> Spell {
        serde_json::from_value(serde_json::json!({"id":"test:giant","name":"Giant","icon":"test:gui/giant","color":0,"kind":"giant","hunger":6,"scale":8.,"transition_ticks":80,"duration_ticks":400,"ending_ticks":80,"effects":["test:growing","test:giant","test:shrinking"]})).unwrap()
    }
    #[test]
    fn prediction_keeps_chained_moves_visible_until_acknowledged() {
        let mut p = Prediction::default();
        let mut l = Ledger::default();
        l.slots[0] = Some("test:giant".into());
        p.receive(&Update {
            ledger: l.clone(),
            started: false,
            error: None,
            swap_revision: None,
        });
        assert!(p.swap(0, 8).is_some());
        assert!(p.ledger.slots[0].is_none());
        assert!(p.swap(8, 35).is_none());
        assert!(p.ledger.slots[8].is_none());
        assert_eq!(p.ledger.slots[35].as_deref(), Some("test:giant"));
        l.swap(0, 8, 0);
        let next = p.receive(&Update {
            ledger: l.clone(),
            started: false,
            error: None,
            swap_revision: Some(0),
        });
        assert!(matches!(
            next,
            Some(Request::Swap {
                from: 8,
                to: 35,
                revision: 1
            })
        ));
        assert_eq!(p.ledger.slots[35].as_deref(), Some("test:giant"));
        // A duplicate acknowledgement cannot flash the icon back to slot 8.
        p.receive(&Update {
            ledger: l.clone(),
            started: false,
            error: None,
            swap_revision: Some(0),
        });
        assert_eq!(p.ledger.slots[35].as_deref(), Some("test:giant"));
        l.swap(8, 35, 1);
        p.receive(&Update {
            ledger: l.clone(),
            started: false,
            error: None,
            swap_revision: Some(1),
        });
        assert!(!p.pending());
        assert_eq!(p.ledger, l);
    }
    #[test]
    fn rejected_prediction_restores_authoritative_slots() {
        let mut p = Prediction::default();
        let mut l = Ledger::default();
        l.slots[0] = Some("test:giant".into());
        p.receive(&Update {
            ledger: l.clone(),
            started: false,
            error: None,
            swap_revision: None,
        });
        p.swap(0, 8);
        p.swap(8, 35);
        p.receive(&Update {
            ledger: l.clone(),
            started: false,
            error: Some("rejected".into()),
            swap_revision: Some(0),
        });
        assert!(!p.pending());
        assert_eq!(p.ledger, l);
        assert!(p.ledger.valid());
    }

    #[test]
    fn sizes_and_stage_boundaries() {
        let s = spell();
        assert_eq!(s.scale_at(0), 1.);
        assert_eq!(s.scale_at(40), 4.5);
        assert_eq!(s.phase(79), Phase::Changing);
        assert_eq!(s.phase(80), Phase::Active);
        assert_eq!(s.scale_at(479), 8.);
        assert_eq!(s.phase(480), Phase::Returning);
        assert_eq!(s.scale_at(520), 4.5);
        assert_eq!(s.scale_at(560), 1.);
        let mut tiny = s;
        tiny.scale = 0.2;
        tiny.transition_ticks = 60;
        tiny.duration_ticks = 600;
        tiny.ending_ticks = 60;
        assert_eq!(tiny.scale_at(60), 0.2);
        assert_eq!(tiny.phase(660), Phase::Returning);
        assert_eq!(tiny.scale_at(720), 1.);
    }
    #[test]
    fn unlock_and_rearrange_never_duplicate_or_delete() {
        let s = spell();
        let mut c = Catalog::default();
        c.spells.insert(s.id.clone(), s);
        let mut l = Ledger::default();
        assert!(l.grant(&["test:giant".into(), "test:unknown".into()], &c));
        assert!(!l.grant(&["test:giant".into()], &c));
        assert!(l.valid());
        assert!(!l.swap(0, 8, 0));
        assert!(!l.swap(0, SLOT_COUNT, l.revision));
        assert!(l.swap(0, 35, l.revision));
        assert!(l.slots[0].is_none());
        assert!(l.contains("test:giant"));
        assert_eq!(l.slots.iter().flatten().count(), 1);
        l.slots[0] = l.slots[35].clone();
        assert!(!l.valid());
    }
    #[test]
    fn requests_are_bounded_and_not_unlock_grants() {
        assert!(Request::decode(br#"{"action":"cast","id":"test:giant","scale":100}"#).is_none());
        assert!(Request::decode(br#"{"action":"grant","id":"test:giant"}"#).is_none());
        assert!(Request::decode(&vec![b' '; 4097]).is_none());
        assert!(Request::decode(br#"{"action":"open"}"#).is_some());
    }
}
