//! Server-owned spell inventory and temporary size magic. No real container slots
//! or item stacks are created for spells, even while the pointer is carrying one.
use super::{Dim, Game, Screen};
use horizon_client::{
    effects::EffectInstance,
    hud::{HudBatch, rgba},
    magic::{self, Kind, Ledger, Phase, Request, Spell, Update},
    particles::{ParticleOptions, ParticleType},
};
use horizon_world::nbt::Tag;
use std::collections::{HashMap, VecDeque};

const SAVE_KEY: &str = "horizon:magic";
#[derive(Default)]
pub(super) struct State {
    pub ui: Ui,
    actors: HashMap<u128, Actor>,
    requests: HashMap<u128, VecDeque<Request>>,
    impacts: Vec<Impact>,
}
#[derive(Default)]
pub(super) struct Ui {
    pub open: bool,
    inventory: magic::Prediction,
    ready: bool,
    page: usize,
    drag: Option<(usize, (f32, f32))>,
    error: Option<String>,
}
struct Actor {
    ledger: Ledger,
    writable: bool,
    character: Option<String>,
    active: Option<Cast>,
    effect_spells: std::collections::BTreeMap<String, Spell>,
}
struct Cast {
    spell: Spell,
    age: u32,
    phase: Option<Phase>,
    scale: f64,
    grounded: bool,
    airborne_ticks: u32,
    highest: f64,
    last_impact: u32,
    last_growth_blast: Option<u32>,
}
struct Impact {
    owner: u128,
    dimension: Dim,
    at: [f64; 3],
    conn: Option<u32>,
}
impl Actor {
    fn load(extra: Option<&Tag>) -> Self {
        let (ledger, writable) = match extra {
            None => (Ledger::default(), true),
            Some(Tag::Compound(_)) => match extra.and_then(|tag| tag.get(SAVE_KEY)) {
                None => (Ledger::default(), true),
                Some(Tag::String(raw)) => match raw
                    .to_string()
                    .ok()
                    .and_then(|s| serde_json::from_str::<Ledger>(&s).ok())
                    .filter(Ledger::valid)
                {
                    Some(ledger) => (ledger, true),
                    None => (Ledger::default(), false),
                },
                _ => (Ledger::default(), false),
            },
            _ => (Ledger::default(), false),
        };
        Self {
            ledger,
            writable,
            character: None,
            active: None,
            effect_spells: Default::default(),
        }
    }
}
impl Game<'_> {
    pub(super) fn magic_control_state(&self) -> serde_json::Value {
        let geometry = self.renderer.as_ref().map(|r| {
            let batch = HudBatch::new([r.config.width, r.config.height]);
            let (ox, oy) = self.inventory_origin(&batch);
            serde_json::json!({"origin": [ox, oy], "scale": batch.scale,
                "button": [ox + 104., (batch.height / 2.).floor() - 22.]})
        });
        let physical_hover = self.renderer.as_ref().and_then(|r| {
            let batch = HudBatch::new([r.config.width, r.config.height]);
            self.slot_at(&batch, self.cursor.0, self.cursor.1)
        });
        serde_json::json!({"enabled": self.pack.magic_catalog.page.is_some(),
            "open": self.magic_page_open(), "ready": self.magic.ui.ready,
            "pending_swaps": self.magic.ui.inventory.pending(),
            "slots": self.magic.ui.inventory.ledger.slots, "revision": self.magic.ui.inventory.ledger.revision,
            "error": self.magic.ui.error, "geometry": geometry, "physical_hover_slot": physical_hover})
    }
    /// The temporary Giant reach modifier, also applied to items whose explicit
    /// attack_range component would otherwise bypass entity_interaction_range.
    pub(super) fn magic_reach_bonus(&self) -> f32 {
        let Some(player) = self.player.as_ref() else {
            return 0.0;
        };
        let key = player.attributes.key("minecraft:entity_interaction_range");
        key.as_ref()
            .and_then(|key| player.attributes.lookup(Some(key)))
            .and_then(|handle| player.attributes.instance(&handle).ok())
            .and_then(|instance| {
                instance
                    .modifier("horizon:magic_reach")
                    .map(|m| m.amount() as f32)
            })
            .unwrap_or(0.0)
    }
    pub(super) fn magic_available(&self) -> bool {
        self.pack.magic_catalog.page.is_some()
            && self.screen == Screen::Inventory
            && self.container.is_none()
            && self.craft_size == 2
            && !self.mount_active()
            && self.station.is_none()
            && self.native_screen_height().is_none()
    }
    pub(super) fn magic_page_open(&self) -> bool {
        self.magic_available() && self.magic.ui.open
    }
    pub(super) fn magic_close(&mut self) {
        self.magic.ui = Ui::default();
    }
    pub(super) fn magic_forget(&mut self, player: u128) {
        self.magic.actors.remove(&player);
        self.magic.requests.remove(&player);
        self.magic.impacts.retain(|impact| impact.owner != player);
    }
    pub(super) fn magic_from_client(&mut self, player: u128, bytes: &[u8]) {
        if self.remote.is_some() || self.pack.magic_catalog.page.is_none() {
            return;
        }
        if let Some(request) = Request::decode(bytes) {
            // A bounded queue for each authenticated connection, processed only
            // in its own player tick. Arbitrary UUIDs never come from the payload.
            let queue = self.magic.requests.entry(player).or_default();
            if queue.len() < 4 {
                queue.push_back(request);
            }
        }
    }
    pub(super) fn magic_from_server(&mut self, bytes: &[u8]) {
        if self.remote.is_none() || bytes.len() > 30_000 {
            return;
        }
        if let Ok(update) = serde_json::from_slice::<Update>(bytes)
            && update.ledger.valid()
        {
            self.magic_receive(update);
        }
    }
    fn magic_receive(&mut self, update: Update) {
        let next = self.magic.ui.inventory.receive(&update);
        if let Some(request) = next {
            self.magic_request(request);
        }
        self.magic.ui.ready = true;
        self.magic.ui.error = update.error;
        if update.started {
            self.close_container();
            self.screen = Screen::Playing;
            self.capture(true);
        }
    }
    fn magic_request(&mut self, request: Request) {
        if self.remote.is_some() {
            if let Ok(bytes) = serde_json::to_vec(&request)
                && self
                    .native_net_send_to_server(magic::CHANNEL, &bytes)
                    .is_err()
            {
                self.magic.ui.inventory.reset();
                self.magic.ui.error = Some("This server does not support magic.".into());
            }
        } else {
            let player = self.native_acting_id();
            let queue = self.magic.requests.entry(player).or_default();
            if queue.len() < 4 {
                queue.push_back(request);
            }
        }
    }
    fn magic_update(
        &mut self,
        player: u128,
        actor: &Actor,
        started: bool,
        error: Option<String>,
        swap_revision: Option<u64>,
    ) {
        let update = Update {
            swap_revision,
            ledger: actor.ledger.clone(),
            started,
            error,
        };
        if self.lan_acting.is_none() && self.remote.is_none() && self.options.dedicated.is_none() {
            self.magic_receive(update);
        } else if let Ok(bytes) = serde_json::to_vec(&update) {
            let _ = self.native_net_send_to_player(player, magic::CHANNEL, &bytes);
        }
    }
    fn magic_save(&mut self, ledger: &Ledger) {
        if self.level.player_extra.is_none() {
            self.level.player_extra = Some(Tag::Compound(Default::default()));
        }
        if let Some(Tag::Compound(map)) = &mut self.level.player_extra {
            map.insert(
                SAVE_KEY.into(),
                Tag::String(
                    serde_json::to_string(ledger)
                        .expect("spell ledger serializes")
                        .as_str()
                        .into(),
                ),
            );
        }
    }
    pub(super) fn magic_player_pre(&mut self) {
        if self.remote.is_some() || self.pack.magic_catalog.page.is_none() || self.player.is_none()
        {
            return;
        }
        let player = self.native_acting_id();
        if self.lan_acting.is_some() && !self.character_sync.server.selection_confirmed(player) { return; }
        let mut actor = match self.magic.actors.remove(&player) {
            Some(actor) => actor,
            None => {
                self.magic_reset_body();
                let mut actor = Actor::load(self.level.player_extra.as_ref());
                for spell in self.pack.magic_catalog.spells.values().filter(|s| s.kind == Kind::Effect) {
                    if self.potions.effects.get(&spell.effects[1]).is_some() { actor.effect_spells.insert(spell.id.clone(), spell.clone()); }
                }
                actor
            }
        };
        let character = self.character_for(player).or_else(|| {
            self.lan_acting
                .is_none()
                .then_some(self.character.as_ref())
                .flatten()
        });
        let id = character.map(|c| c.id.clone());
        if actor.character != id {
            let spells = character.map(|c| c.starting_spells.clone()).unwrap_or_default();
            let switched = actor.ledger.character.as_ref().is_some_and(|previous| Some(previous) != id.as_ref())
                || actor.character.as_ref().is_some_and(|previous| Some(previous) != id.as_ref());
            if switched {
                actor.active = None;
                self.magic_reset_body();
                self.magic_clear_spell_effects();
                self.magic.requests.remove(&player);
                self.magic.impacts.retain(|impact| impact.owner != player);
            }
            actor.character = id.clone();
            if actor.writable {
                let bound = actor.ledger.bind_character(id.as_deref(), &spells, &self.pack.magic_catalog);
                let granted = actor.ledger.grant(&spells, &self.pack.magic_catalog);
                if bound || granted {
                    self.magic_prune_unowned_effects(&actor.ledger);
                    self.magic_save(&actor.ledger);
                    self.magic_update(player, &actor, false, None, None);
                }
            }
        }
        let ended = actor.effect_spells.iter().filter(|(_, spell)| self.potions.effects.get(&spell.effects[1]).is_none()).map(|(id, _)| id.clone()).collect::<Vec<_>>();
        for id in ended {
            if let Some(spell) = actor.effect_spells.remove(&id) { self.magic_effect_cue(&spell, false); }
        }
        let requests = self.magic.requests.remove(&player).unwrap_or_default();
        for request in requests {
            let swap_revision = match &request {
                Request::Swap { revision, .. } => Some(*revision),
                _ => None,
            };
            let mut started = false;
            let mut error = None;
            if !actor.writable {
                error = Some("Your saved magic data needs repair; it has been preserved.".into());
            } else {
                match request {
                    Request::Open => {}
                    Request::Swap { from, to, revision } => {
                        if actor.ledger.swap(from, to, revision) {
                            self.magic_save(&actor.ledger);
                        }
                    }
                    Request::Cast { id } => {
                        let spell = self.pack.magic_catalog.spells.get(&id).cloned();
                        if actor.active.is_some() {
                            error = Some("Finish your current spell first.".into());
                        } else if self.spectator()
                            || self
                                .player
                                .as_ref()
                                .is_none_or(|p| p.health.dead_or_dying())
                        {
                            error = Some("You cannot cast a spell now.".into());
                        } else if !actor.ledger.contains(&id) || spell.is_none() {
                            error = Some("That spell has not been unlocked.".into());
                        } else {
                            let spell = spell.unwrap();
                            let body = self.player.as_mut().unwrap();
                            if body.health.food.food < spell.hunger {
                                error = Some(format!("Requires {} hunger points.", spell.hunger));
                            } else {
                                body.health.food.food -= spell.hunger;
                                body.health.food.saturation = body
                                    .health
                                    .food
                                    .saturation
                                    .min(body.health.food.food as f32);
                                if spell.kind == Kind::Effect {
                                    if let Some(id) = super::potions::mob_effect_id(&spell.effects[1]) {
                                        let mut effect = EffectInstance::new(id, spell.duration_ticks as i32, 0);
                                        effect.visible = false;
                                        self.add_player_effect(effect);
                                    }
                                    self.magic_effect_cue(&spell, true);
                                    actor.effect_spells.insert(spell.id.clone(), spell.clone());
                                    started = true;
                                } else {
                                actor.active = Some(Cast {
                                    spell,
                                    age: 0,
                                    phase: None,
                                    scale: 1.0,
                                    grounded: body.ground.on_ground,
                                    airborne_ticks: 0,
                                    highest: body.position.y,
                                    last_impact: 0,
                                    last_growth_blast: None,
                                });
                                self.swing_arm();
                                started = true;
                                }
                            }
                        }
                    }
                }
            }
            self.magic_update(player, &actor, started, error, swap_revision);
        }
        if let Some(mut cast) = actor.active.take() {
            if self.magic_tick_cast(&mut cast) {
                actor.active = Some(cast);
            }
        }
        self.magic.actors.insert(player, actor);
    }
    pub(super) fn magic_player_post(&mut self) {
        if self.remote.is_some() || self.pack.magic_catalog.page.is_none() {
            return;
        }
        let owner = self.native_acting_id();
        let Some(actor) = self.magic.actors.get_mut(&owner) else {
            return;
        };
        let Some(cast) = actor.active.as_mut() else {
            return;
        };
        let Some(p) = self.player.as_ref() else {
            return;
        };
        if cast.spell.kind != Kind::Giant
            || cast.spell.phase(cast.age.saturating_sub(1)) != Phase::Active
        {
            return;
        }
        let grounded = p.ground.on_ground;
        if !grounded {
            if cast.grounded {
                cast.highest = p.position.y;
                cast.airborne_ticks = 0;
            }
            cast.airborne_ticks += 1;
            cast.highest = cast.highest.max(p.position.y);
        }
        // One impact on a real landing; walking down a step or colliding with
        // a wall cannot repeatedly detonate at twenty explosions per second.
        if grounded
            && !cast.grounded
            && cast.airborne_ticks >= 4
            && cast.highest - p.position.y >= 0.35
            && cast.age >= cast.last_impact + 10
        {
            self.magic.impacts.push(Impact {
                owner,
                conn: self.lan_acting,
                dimension: self.dim,
                at: [p.position.x, p.position.y + 0.15, p.position.z],
            });
            cast.last_impact = cast.age;
            cast.airborne_ticks = 0;
        }
        cast.grounded = grounded;
    }
    fn magic_marker(&mut self, spell: &Spell, phase: Phase) {
        self.magic_clear_markers();
        let (index, ticks) = match phase {
            Phase::Changing => (0, spell.transition_ticks),
            Phase::Active => (1, spell.duration_ticks),
            Phase::Returning => (2, spell.ending_ticks),
            Phase::Finished => return,
        };
        if let Some(id) = super::potions::mob_effect_id(&spell.effects[index]) {
            let mut effect = EffectInstance::new(id, ticks as i32, 0);
            effect.visible = false; // The authored casting particles replace potion bubbles.
            self.add_player_effect(effect);
        }
    }
    fn magic_prune_unowned_effects(&mut self, ledger: &Ledger) {
        let all = self.pack.magic_catalog.spells.values().flat_map(|s| s.effects.iter().cloned()).collect::<std::collections::BTreeSet<_>>();
        let owned = self.pack.magic_catalog.spells.values().filter(|s| ledger.contains(&s.id)).flat_map(|s| s.effects.iter().cloned()).collect::<std::collections::BTreeSet<_>>();
        let before = self.potions.effects.list.len();
        self.potions.effects.list.retain(|effect| !all.contains(effect.id) || owned.contains(effect.id));
        if self.potions.effects.list.len() != before { self.apply_effect_status(); }
    }

    fn magic_clear_spell_effects(&mut self) {
        let ids = self.pack.magic_catalog.spells.values().flat_map(|s| s.effects.iter().cloned()).collect::<std::collections::BTreeSet<_>>();
        self.potions.effects.list.retain(|effect| !ids.contains(effect.id));
        self.apply_effect_status();
    }

    fn magic_effect_cue(&mut self, spell: &Spell, starting: bool) {
        if let Some(sound) = if starting { &spell.start_sound } else { &spell.end_sound } {
            self.magic_sound(sound, 0.65, if starting { 1.15 } else { 0.9 });
        }
        let Some(color) = spell.particle_color else { return; };
        let Some(body) = self.player.as_ref() else { return; };
        let at = [body.position.x, body.position.y + f64::from(body.dimensions.eye_height) * 0.7, body.position.z];
        let options = ParticleOptions::dust(color, 0.8);
        // One bounded burst at each boundary, using the normal world/packet pipeline.
        self.lan_send_particles(super::lan_particles::QueuedParticle {
            dimension: self.dim.id(), options, state: None, at,
            spread: [0.35, 0.45, 0.35], speed: [0.02; 3], count: 18, randomization: 0,
        });
    }

    fn magic_clear_markers(&mut self) {
        let catalog = &self.pack.magic_catalog;
        self.potions.effects.list.retain(|effect| {
            !catalog
                .spells
                .values()
                .filter(|s| s.kind != Kind::Effect)
                .any(|s| s.effects.iter().any(|id| id == effect.id))
        });
        self.apply_effect_status();
    }
    fn magic_modifier(&mut self, attribute: &str, key: &str, value: f64, operation: &str) {
        if let Some(p) = self.player.as_mut() {
            let _ = p.set_modifier(attribute, key, value, operation, value != 0.0);
        }
    }
    fn magic_reset_body(&mut self) {
        for (attribute, key) in [
            ("minecraft:scale", "horizon:magic_scale"),
            ("minecraft:entity_interaction_range", "horizon:magic_reach"),
            ("minecraft:block_interaction_range", "horizon:magic_reach"),
            ("minecraft:movement_speed", "horizon:magic_casting"),
            ("minecraft:movement_speed", "horizon:magic_speed"),
            ("minecraft:attack_damage", "horizon:magic_strength"),
            ("minecraft:block_break_speed", "horizon:magic_mining"),
            ("minecraft:jump_strength", "horizon:magic_jump"),
            ("minecraft:fall_damage_multiplier", "horizon:magic_fall"),
        ] {
            self.magic_modifier(attribute, key, 0.0, "add_value");
        }
        if let Some(p) = self.player.as_mut() {
            let _ = p.refresh_dimensions();
        }
        self.magic_clear_markers();
    }
    fn magic_tick_cast(&mut self, cast: &mut Cast) -> bool {
        let phase = cast.spell.phase(cast.age);
        if phase == Phase::Finished {
            self.magic_reset_body();
            self.magic_sound("minecraft:entity.item_frame.remove_item", 0.7, 1.3);
            self.magic_motes(cast, true);
            return false;
        }
        if cast.phase != Some(phase) {
            self.magic_marker(&cast.spell, phase);
            let active = phase == Phase::Active;
            let giant = active && cast.spell.kind == Kind::Giant;
            let tiny = active && cast.spell.kind == Kind::Tiny;
            self.magic_modifier(
                "minecraft:movement_speed",
                "horizon:magic_casting",
                if phase == Phase::Changing && cast.spell.kind == Kind::Giant {
                    -0.9
                } else {
                    0.0
                },
                "add_multiplied_total",
            );
            self.magic_modifier(
                "minecraft:movement_speed",
                "horizon:magic_speed",
                if tiny { 0.5 } else { 0.0 },
                "add_multiplied_total",
            );
            self.magic_modifier(
                "minecraft:attack_damage",
                "horizon:magic_strength",
                if giant { 6.0 } else { 0.0 },
                "add_value",
            );
            self.magic_modifier(
                "minecraft:block_break_speed",
                "horizon:magic_mining",
                if tiny { -0.7 } else { 0.0 },
                "add_multiplied_total",
            );
            self.magic_modifier(
                "minecraft:jump_strength",
                "horizon:magic_jump",
                if tiny { 0.2 } else { 0.0 },
                "add_value",
            );
            self.magic_modifier(
                "minecraft:fall_damage_multiplier",
                "horizon:magic_fall",
                if tiny { -1.0 } else { 0.0 },
                "add_multiplied_total",
            );
            if phase == Phase::Changing {
                self.magic_sound("minecraft:block.enchantment_table.use", 0.8, 0.8);
            } else if giant {
                self.magic_sound("minecraft:entity.ender_dragon.growl", 0.8, 0.9);
            } else if tiny {
                self.magic_sound("minecraft:block.amethyst_cluster.hit", 0.7, 1.6);
            } else {
                self.magic_sound("minecraft:block.beacon.deactivate", 0.5, 1.15);
            }
            cast.phase = Some(phase);
            self.magic_motes(cast, true);
        }
        let scale = cast.spell.scale_at(cast.age);
        let scale_changed = (scale - cast.scale).abs() > 1.0e-8;
        if scale_changed {
            if cast.spell.kind == Kind::Giant && phase == Phase::Changing {
                if let Some(p) = self.player.as_ref() {
                    let ratio = scale / cast.scale;
                    let half = f64::from(p.dimensions.width) * ratio / 2.0 + 0.1;
                    let height = f64::from(p.dimensions.height) * ratio + 0.1;
                    let at = [p.position.x, p.position.y, p.position.z];
                    let min = [at[0] - half, at[1] + 0.01, at[2] - half];
                    let max = [at[0] + half, at[1] + height, at[2] + half];
                    if self.explosion_clearance(min, max, 3.0).unwrap_or(0) > 0
                        && cast
                            .last_growth_blast
                            .is_none_or(|age| cast.age >= age + 10)
                    {
                        let owner = self.native_acting_id();
                        self.magic.impacts.push(Impact {
                            owner,
                            conn: self.lan_acting,
                            dimension: self.dim,
                            at: [at[0], at[1] + height * 0.5, at[2]],
                        });
                        cast.last_growth_blast = Some(cast.age);
                    }
                }
            }
            self.magic_modifier(
                "minecraft:scale",
                "horizon:magic_scale",
                scale - 1.0,
                "add_multiplied_total",
            );
            if let Some(p) = self.player.as_mut() {
                let _ = p.refresh_dimensions();
            }
            cast.scale = scale;
            if cast.spell.kind == Kind::Giant {
                let bonus = self.player.as_ref().map_or(0.0, |p| {
                    let eye = f64::from(p.dimensions.eye_height);
                    (eye - eye / scale).max(0.0)
                });
                for attr in [
                    "minecraft:entity_interaction_range",
                    "minecraft:block_interaction_range",
                ] {
                    self.magic_modifier(attr, "horizon:magic_reach", bonus, "add_value");
                }
            }
        }
        if phase != Phase::Active {
            if cast.age % 4 == 0 {
                self.magic_motes(cast, false);
            }
            if cast.age % 16 == 0 {
                let progress = if phase == Phase::Changing {
                    f64::from(cast.age) / f64::from(cast.spell.transition_ticks)
                } else {
                    1.0 - f64::from(
                        cast.age - cast.spell.transition_ticks - cast.spell.duration_ticks,
                    ) / f64::from(cast.spell.ending_ticks)
                };
                let pitch = if cast.spell.kind == Kind::Giant {
                    1.6 - progress as f32 * 0.9
                } else {
                    0.8 + progress as f32 * 0.9
                };
                self.magic_sound("minecraft:block.amethyst_block.chime", 0.45, pitch);
            }
        }
        cast.age += 1;
        true
    }
    fn magic_sound(&mut self, sound: &str, volume: f32, pitch: f32) {
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let at = [p.position.x, p.position.y, p.position.z];
        self.audio.play(sound, Some(at), volume, pitch);
        self.creatures
            .lan_sounds
            .push(super::creatures::NativeSound {
                dimension: self.dim.id(),
                sound: sound.into(),
                at,
                source: 7,
                volume,
                pitch,
            });
    }
    fn magic_motes(&mut self, cast: &Cast, burst: bool) {
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let mut options = ParticleOptions::simple(ParticleType::Dust);
        options.color = if cast.spell.kind == Kind::Giant {
            [1.0, 0.53, 0.16, 1.0]
        } else {
            [0.4, 0.78, 1.0, 1.0]
        };
        options.scale = if burst { 1.2 } else { 0.75 };
        // Two gently counter-rotating ribbons; sparse packets rather than
        // a per-frame network emitter or a screen-filling potion cloud.
        let radius = f64::from(p.dimensions.width) * 0.65 + 0.15;
        let height = f64::from(p.dimensions.height);
        let angle = f64::from(cast.age) * 0.24;
        let feet = [p.position.x, p.position.y, p.position.z];
        for side in 0..2 {
            let a = angle + f64::from(side) * std::f64::consts::PI;
            let at = [
                feet[0] + radius * a.cos(),
                feet[1] + height * (0.25 + (a * 0.35).sin().abs() * 0.55),
                feet[2] + radius * a.sin(),
            ];
            self.particles.add(&options, at, [0.0, 0.015, 0.0]);
            self.lan_send_particles(super::lan_particles::QueuedParticle {
                dimension: self.dim.id(),
                options: options.clone(),
                state: None,
                at,
                spread: [0.08; 3],
                speed: [0.015; 3],
                count: if burst { 10 } else { 2 },
                randomization: 0,
            });
        }
    }
    /// Run after all actor swaps, with the hub present, so damage, knockback,
    /// sounds and terrain changes reach every player in the impact's dimension.
    pub(super) fn magic_flush_impacts(&mut self) {
        if self.remote.is_some() || self.lan.is_none() {
            return;
        }
        for impact in std::mem::take(&mut self.magic.impacts) {
            let back = self.dim;
            if self.enter_level(None, impact.dimension).is_err() {
                continue;
            }
            let live = self.lan.as_ref().is_some_and(|h| {
                h.hub.guests.iter().any(|g| {
                    g.uuid == impact.owner
                        && g.loaded
                        && !g.dead
                        && g.dimension == impact.dimension.id()
                })
            });
            if live || impact.conn.is_none() {
                let previous_owner = self.combat.blast_owner.take();
                let previous_guest = self.combat.blast_guest;
                self.combat.blast_owner = self
                    .lan
                    .as_ref()
                    .and_then(|h| self.player_attacker(h, impact.conn));
                self.combat.blast_guest = impact.conn;
                let interaction = self.destroy_type("tnt_explosion_drop_decay");
                // Radius 3 gives vanilla's six-block entity-damage reach and
                // resistance-aware, compact craters without destroying bedrock.
                let _ = self.server_explode_excluding(impact.at, 3.0, interaction, impact.owner);
                self.combat.blast_owner = previous_owner;
                self.combat.blast_guest = previous_guest;
            }
            if self.dim != back {
                let _ = self.enter_level(None, back);
            }
        }
    }

    pub(super) fn magic_button_press(&mut self) {
        if self.carried.is_some() {
            return;
        }
        self.magic.ui.open = !self.magic.ui.open;
        self.magic.ui.drag = None;
        self.magic.ui.error = None;
        self.recipe_ui.reset_screen();
        if self.magic.ui.open {
            self.magic.ui.ready = false;
            self.magic_request(Request::Open);
        }
        self.audio
            .play("minecraft:ui.button.click", None, 0.25, 1.0);
    }
    pub(super) fn draw_magic_button(&self, batch: &mut HudBatch, bx: f32, by: f32) {
        let Some(page) = &self.pack.magic_catalog.page else {
            return;
        };
        let atlas = &self.gui.atlas;
        let hover = inside(self.cursor, bx, by, 20.0, 18.0);
        batch.sprite(
            atlas,
            if hover {
                "gui/sprites/recipe_book/button_highlighted"
            } else {
                "gui/sprites/recipe_book/button"
            },
            bx,
            by,
        );
        // Cover the vanilla book glyph while retaining its familiar button rim.
        batch.fill(atlas, bx + 2., by + 1., 16., 16., rgba(139, 139, 139, 255));
        if let Some(rect) = atlas.sprites.get(&page.icon).copied() {
            batch.rect_uv(bx + 2., by + 1., 16., 16., rect, u32::MAX);
        }
        if hover {
            self.draw_tooltip_lines(
                batch,
                &[(page.title.clone(), rgb(page.color))],
                self.cursor.0,
                self.cursor.1,
            );
        }
    }
    fn magic_slot_at(&self, batch: &HudBatch) -> Option<usize> {
        let (ox, oy) = self.inventory_origin(batch);
        (0..magic::PAGE_SIZE)
            .find(|i| {
                let (x, y) = self.slot_position(visual_slot(*i));
                inside(self.cursor, ox + x, oy + y, 16., 16.)
            })
            .map(|i| i + self.magic.ui.page * magic::PAGE_SIZE)
    }
    pub(super) fn magic_press(&mut self, right: bool) {
        if right {
            return;
        }
        let Some(renderer) = self.renderer.as_ref() else {
            return;
        };
        let batch = HudBatch::new([renderer.config.width, renderer.config.height]);
        let (ox, oy) = self.inventory_origin(&batch);
        let pages = self.magic_pages();
        if inside(self.cursor, ox + 122., oy + 62., 16., 12.) {
            self.magic.ui.page = self.magic.ui.page.saturating_sub(1);
            self.magic.ui.drag = None;
            return;
        }
        if inside(self.cursor, ox + 146., oy + 62., 16., 12.) {
            self.magic.ui.page = (self.magic.ui.page + 1).min(pages - 1);
            self.magic.ui.drag = None;
            return;
        }
        if self.magic.ui.ready
            && let Some(slot) = self.magic_slot_at(&batch)
            && self.magic.ui.inventory.ledger.slots[slot].is_some()
        {
            self.magic.ui.drag = Some((slot, self.cursor));
        }
    }
    pub(super) fn magic_release(&mut self, right: bool) {
        if right {
            return;
        }
        let Some((from, start)) = self.magic.ui.drag.take() else {
            return;
        };
        let Some(renderer) = self.renderer.as_ref() else {
            return;
        };
        let batch = HudBatch::new([renderer.config.width, renderer.config.height]);
        let Some(to) = self.magic_slot_at(&batch) else {
            return;
        };
        if from != to {
            if let Some(request) = self.magic.ui.inventory.swap(from, to) {
                self.magic_request(request);
            }
        } else if (self.cursor.0 - start.0).abs() + (self.cursor.1 - start.1).abs() < 5.
            && let Some(id) = self.magic.ui.inventory.ledger.slots[from].clone()
        {
            if !self.magic.ui.inventory.pending() {
                self.magic_request(Request::Cast { id });
            }
        }
    }
    fn magic_pages(&self) -> usize {
        self.magic
            .ui
            .inventory
            .ledger
            .slots
            .iter()
            .rposition(Option::is_some)
            .map_or(1, |i| (i + 1).div_ceil(magic::PAGE_SIZE))
    }
    pub(super) fn draw_magic_page(&mut self, batch: &mut HudBatch, ox: f32, oy: f32) {
        let atlas = &self.gui.atlas;
        // Equipment, offhand, output and crafting are visible but disabled.
        for (x, y, w, h) in [
            (6., 6., 20., 74.),
            (75., 60., 20., 20.),
            (96., 14., 65., 45.),
        ] {
            batch.fill(atlas, ox + x, oy + y, w, h, rgba(30, 24, 40, 185));
        }
        batch.fill(atlas, ox + 96., oy + 5., 65., 10., rgba(198, 198, 198, 255));
        batch.text(
            atlas,
            "Magic",
            ox + 98.,
            oy + 6.,
            rgba(112, 63, 153, 255),
            false,
        );
        batch.text(
            atlas,
            "Locked",
            ox + 104.,
            oy + 30.,
            rgba(170, 160, 180, 255),
            false,
        );
        let hovered = self.magic_slot_at(batch);
        for i in 0..magic::PAGE_SIZE {
            let slot = i + self.magic.ui.page * magic::PAGE_SIZE;
            let (x, y) = self.slot_position(visual_slot(i));
            if hovered == Some(slot) {
                batch.fill(atlas, ox + x, oy + y, 16., 16., rgba(208, 180, 255, 80));
            }
            if self
                .magic
                .ui
                .drag
                .as_ref()
                .is_some_and(|(from, _)| *from == slot)
            {
                continue;
            }
            if let Some(id) = &self.magic.ui.inventory.ledger.slots[slot]
                && let Some(spell) = self.pack.magic_catalog.spells.get(id)
                && let Some(rect) = atlas.sprites.get(&spell.icon).copied()
            {
                batch.rect_uv(ox + x, oy + y, 16., 16., rect, u32::MAX);
            }
        }
        let pages = self.magic_pages();
        if pages > 1 {
            batch.text(atlas, "<", ox + 126., oy + 65., u32::MAX, false);
            batch.text(atlas, ">", ox + 150., oy + 65., u32::MAX, false);
        }
        if !self.magic.ui.ready {
            batch.text(
                atlas,
                "Loading spells...",
                ox + 8.,
                oy + 73.,
                rgba(90, 63, 110, 255),
                false,
            );
        }
        self.draw_recipe_book(batch);
        if let Some((slot, _)) = &self.magic.ui.drag {
            if let Some(spell) = self.magic.ui.inventory.ledger.slots[*slot]
                .as_ref()
                .and_then(|id| self.pack.magic_catalog.spells.get(id))
                && let Some(rect) = self.gui.atlas.sprites.get(&spell.icon).copied()
            {
                batch.rect_uv(
                    self.cursor.0 - 8.,
                    self.cursor.1 - 8.,
                    16.,
                    16.,
                    rect,
                    u32::MAX,
                );
            }
        } else if let Some(spell) = hovered
            .and_then(|slot| self.magic.ui.inventory.ledger.slots[slot].as_ref())
            .and_then(|id| self.pack.magic_catalog.spells.get(id))
        {
            let atlas = &self.gui.atlas;
            let cost = format!("Cost: {}", spell.hunger);
            let cost_width = atlas.font.width(&cost);
            let mut lines = vec![
                (
                    horizon_client::text::Text::literal(&spell.name),
                    rgb(spell.color),
                ),
                (
                    horizon_client::text::Text::literal(&cost),
                    rgba(170, 170, 170, 255),
                ),
            ];
            let width = (batch.width - 24.0).min(250.0).max(80.0);
            for text in atlas.font.split_formatted(
                &horizon_client::text::Text::literal(&spell.description),
                width,
            ) {
                lines.push((text, rgba(170, 170, 170, 255)));
            }
            let (tx, ty, tw, th) = Self::tooltip_geometry(
                atlas,
                &lines,
                self.cursor,
                (batch.width, batch.height),
                [cost_width + 12.0, 0.0],
            );
            self.tooltip_box(batch, None, tx, ty, tw, th);
            Self::tooltip_text(batch, atlas, &lines, tx, ty);
            if let Some(icon) = atlas.sprites.get("gui/sprites/hud/food_full").copied() {
                batch.rect_uv(tx + cost_width + 3.0, ty + 11.0, 9.0, 9.0, icon, u32::MAX);
            }
        }
        if let Some(error) = &self.magic.ui.error {
            self.draw_tooltip_lines(
                batch,
                &[(error.clone(), rgba(255, 110, 110, 255))],
                ox + 8.,
                oy + 166.,
            );
        }
    }
}
fn visual_slot(index: usize) -> usize {
    if index < 27 { index + 9 } else { index - 27 }
}
fn inside(point: (f32, f32), x: f32, y: f32, w: f32, h: f32) -> bool {
    point.0 >= x && point.1 >= y && point.0 < x + w && point.1 < y + h
}
fn rgb(color: u32) -> u32 {
    rgba((color >> 16) as u8, (color >> 8) as u8, color as u8, 255)
}
