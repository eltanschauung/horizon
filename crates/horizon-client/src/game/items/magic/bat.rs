//! Server-owned bat forms. Only their presentation is replicated separately
//! from the regular player abilities, health, attributes and status effect.
use super::{Game, Spell, Tag};
use horizon_client::{
    effects::EffectInstance,
    mesher::Vertex,
    mobs::{Mob, bat, render},
    pose::Dimensions,
};
use horizon_core::{geometry::Vec3, random::RandomSource};
use serde::{Deserialize, Serialize};
use std::{cell::RefCell, collections::BTreeMap};

use horizon_protocol::lunatic_channels::BAT_FORMS as CHANNEL;
const SAVE_KEY: &str = "horizon:bat_form";
const BAT: &str = "minecraft:bat";
const SCALE: f32 = 1.3;
const CAPACITY: i32 = 10;
const HEALTH_MODIFIER: &str = "horizon:bat_form_health";

#[derive(Clone, Deserialize, Serialize)]
pub(super) struct BatForm {
    pub(super) character: Option<String>,
    tint: [u8; 3],
    effect: String,
    withheld_health: f32,
    withheld_food: i32,
    food_capacity: i32,
    withheld_saturation: f32,
    may_fly: bool,
    flying: bool,
    creative: bool,
}

#[derive(Default)]
pub(super) struct BatVisuals {
    forms: BTreeMap<u128, [u8; 3]>,
    viewers: Vec<(u32, u128)>,
    model: RefCell<Option<Mob>>,
}

impl BatForm {
    pub(super) fn load(extra: Option<&Tag>) -> Option<Self> {
        let Tag::String(raw) = extra?.get(SAVE_KEY)? else {
            return None;
        };
        let form: Self = serde_json::from_str(&raw.to_string().ok()?).ok()?;
        (form.withheld_health.is_finite()
            && (0.0..=1024.0).contains(&form.withheld_health)
            && (1..=100).contains(&form.food_capacity)
            && (0..=100).contains(&form.withheld_food)
            && (0.0..=100.0).contains(&form.withheld_saturation))
        .then_some(form)
    }
}

impl Game<'_> {
    pub(in crate::game) fn bat_dimensions(&self) -> Option<Dimensions> {
        let kind = self.res.mob_types().get(BAT)?;
        Some(Dimensions {
            width: kind.width as f32 * SCALE,
            height: kind.height as f32 * SCALE,
            eye_height: kind.eye_height as f32 * SCALE,
        })
    }

    pub(in crate::game) fn magic_is_bat(&self) -> bool {
        self.player
            .as_ref()
            .is_some_and(|p| p.form_dimensions.is_some())
    }

    fn bat_save(&mut self, form: Option<&BatForm>) {
        if let Some(form) = form {
            let extra = self
                .level
                .player_extra
                .get_or_insert_with(|| Tag::Compound(Default::default()));
            if let Tag::Compound(map) = extra {
                map.insert(
                    SAVE_KEY.into(),
                    Tag::String(
                        serde_json::to_string(form)
                            .expect("bat form serializes")
                            .as_str()
                            .into(),
                    ),
                );
            }
        } else if let Some(Tag::Compound(map)) = &mut self.level.player_extra {
            map.remove(&SAVE_KEY.into());
        }
    }

    pub(super) fn bat_start(
        &mut self,
        player: u128,
        character: Option<String>,
        spell: &Spell,
    ) -> Option<BatForm> {
        let tint = spell.bat_tint?;
        self.bat_dimensions()?;
        let effect = super::super::potions::mob_effect_id(&spell.effects[1])?;
        let min = spell.min_duration_ticks?;
        let duration = min as i32
            + self
                .world
                .random
                .next_int_bound((spell.duration_ticks - min + 1) as i32)
                .unwrap_or(0);
        let body = self.player.as_mut()?;
        // Hunger is charged before saving excess above the temporary cap.
        let withheld_health = (body.health.health() - CAPACITY as f32).max(0.0);
        body.health.food.food -= spell.hunger;
        let food = &mut body.health.food;
        food.saturation = food.saturation.min(food.food as f32);
        let form = BatForm {
            character,
            tint,
            effect: effect.into(),
            withheld_health,
            withheld_food: (food.food - CAPACITY).max(0),
            food_capacity: food.max_food,
            withheld_saturation: (food.saturation - CAPACITY as f32).max(0.0),
            may_fly: body.may_fly,
            flying: body.flying,
            creative: self.creative,
        };
        let mut marker = EffectInstance::new(effect, duration, 0);
        marker.visible = false;
        self.add_player_effect(marker);
        self.bat_save(Some(&form));
        self.bat_apply_body(player);
        self.magic_sound("minecraft:entity.bat.takeoff", 0.6, 1.0);
        self.bat_scatter();
        Some(form)
    }

    pub(super) fn bat_effect_active(&self, form: &BatForm) -> bool {
        self.potions.effects.get(&form.effect).is_some()
    }

    pub(super) fn bat_apply_body(&mut self, player: u128) {
        let dimensions = self.bat_dimensions();
        let Some(body) = self.player.as_mut() else {
            return;
        };
        let new_body = body.form_dimensions.is_none() || !body.may_fly;
        body.form_dimensions = dimensions;
        if new_body {
            let _ = body.refresh_dimensions();
            body.fall_flying = false;
            body.fall_distance = 0.0;
        }
        if body.attribute("minecraft:max_health").ok() != Some(f64::from(CAPACITY)) {
            let _ = body.set_modifier_unclamped(
                "minecraft:max_health",
                HEALTH_MODIFIER,
                0.0,
                "add_value",
                false,
            );
            let normal = body.attribute("minecraft:max_health").unwrap_or(20.0);
            let _ = body.set_modifier(
                "minecraft:max_health",
                HEALTH_MODIFIER,
                f64::from(CAPACITY) / normal - 1.0,
                "add_multiplied_total",
                true,
            );
        }
        let food = &mut body.health.food;
        food.max_food = CAPACITY;
        food.food = food.food.min(CAPACITY);
        food.saturation = food.saturation.min(food.food as f32);
        if new_body {
            self.using = None;
            self.mining = None;
            self.bat_flight(player, true, true);
        }
    }

    pub(super) fn bat_finish(&mut self, player: u128, form: BatForm) {
        self.bat_save(None);
        self.potions
            .effects
            .list
            .retain(|effect| effect.id != form.effect);
        self.apply_effect_status();
        if let Some(body) = &mut self.player {
            body.form_dimensions = None;
            let _ = body.refresh_dimensions();
            let _ = body.set_modifier(
                "minecraft:max_health",
                HEALTH_MODIFIER,
                0.0,
                "add_value",
                false,
            );
            // Preserve damage and exhaustion in bat form, including the cost
            // of cancelling. Restore only what the temporary cap withheld.
            if !body.health.dead_or_dying() {
                let health = body.health.health() + form.withheld_health;
                let _ = body.set_health(health, false);
                let food = &mut body.health.food;
                food.max_food = form.food_capacity;
                food.food = (food.food + form.withheld_food).min(food.max_food);
                food.saturation =
                    (food.saturation + form.withheld_saturation).min(food.food as f32);
            }
            body.fall_distance = 0.0;
        }
        let may_fly = self.creative || self.spectator() || (!form.creative && form.may_fly);
        self.bat_flight(player, may_fly, form.flying && may_fly);
    }

    pub(in crate::game) fn magic_discard_bat_save(&mut self) {
        if let Some(form) = BatForm::load(self.level.player_extra.as_ref()) {
            self.potions
                .effects
                .list
                .retain(|effect| effect.id != form.effect);
            self.bat_save(None);
        }
    }

    fn bat_flight(&mut self, _player: u128, allow: bool, flying: bool) {
        if let Some(body) = &mut self.player {
            body.may_fly = allow;
            body.flying = flying || body.spectator;
        }
        if let Some(abilities) = self.acting_abilities.as_mut() {
            abilities.may_fly = allow;
            abilities.flying = flying || self.player.as_ref().is_some_and(|body| body.spectator);
        }
    }

    pub(super) fn bat_ambient_sound(&mut self) {
        if self.world.random.next_int_bound(160).ok() == Some(0) {
            let pitch = 0.95 + self.world.random.next_float() * 0.2;
            self.magic_sound("minecraft:entity.bat.ambient", 0.25, pitch);
        }
    }

    fn bat_scatter(&mut self) {
        let Some(body) = &self.player else {
            return;
        };
        let at = [body.position.x, body.position.y, body.position.z];
        let count = 6 + self.world.random.next_int_bound(5).unwrap_or(0);
        let direction = f64::from(self.world.random.next_float()) * std::f64::consts::TAU;
        for i in 0..count {
            let angle = direction + std::f64::consts::TAU * f64::from(i) / f64::from(count);
            if let Some(id) = self.spawn_mob(BAT, at, false, crate::game::creatures::SpawnSite::Plain)
                && let Some(mob) = self.mobs.iter_mut().find(|m| m.id == id)
            {
                bat::set_resting(mob, false);
                mob.velocity = Vec3::new(0.6 * angle.cos(), 0.35, 0.6 * angle.sin());
            }
        }
    }

    pub(in crate::game) fn magic_sync_bats(&mut self, host: &super::super::lan::LanHost) {
        let forms = self
            .magic
            .actors
            .iter()
            .filter_map(|(&uuid, actor)| Some((uuid, actor.bat.as_ref()?.tint)))
            .collect::<BTreeMap<_, _>>();
        let guests = host.hub.guests.iter().filter(|g| {
            g.placed && horizon_client::raw_payloads::negotiated(g.sender.payload_transport(), CHANNEL)
        }).collect::<Vec<_>>();
        let viewers = guests.iter().map(|g| (g.conn, g.uuid)).collect::<Vec<_>>();
        if self.magic.bat_visuals.forms == forms && self.magic.bat_visuals.viewers == viewers {
            return;
        }
        let data = forms
            .iter()
            .flat_map(|(uuid, tint)| uuid.to_be_bytes().into_iter().chain(*tint))
            .collect::<Vec<_>>();
        let mut sent = true;
        for guest in guests {
            sent &= self.send_lunatic_payload(guest.sender.clone(), CHANNEL, &data).is_ok();
        }
        if sent {
            self.magic.bat_visuals.forms = forms;
            self.magic.bat_visuals.viewers = viewers;
        }
    }

    pub(in crate::game) fn magic_receive_bats(&mut self, data: &[u8]) {
        if self.remote.is_none() || !data.len().is_multiple_of(19) {
            return;
        }
        self.magic.bat_visuals.forms = data
            .chunks_exact(19)
            .map(|form| {
                (
                    u128::from_be_bytes(form[..16].try_into().expect("bat UUID")),
                    form[16..].try_into().expect("bat tint"),
                )
            })
            .collect();
        let dimensions = self.bat_dimensions();
        for (id, _) in self.remote_player_names() {
            let form = self
                .remote_player_uuid(id)
                .and_then(|uuid| self.magic_bat_tint(uuid));
            if let Some(mob) = self.mob_index.get_mut(&mut self.mobs, id) {
                mob.extra.form_dimensions = form.and(dimensions);
            }
        }
        self.magic_refresh_remote_bat();
    }

    pub(in crate::game) fn magic_refresh_remote_bat(&mut self) {
        let Some(remote) = &self.remote else {
            return;
        };
        let active = self.magic.bat_visuals.forms.contains_key(&remote.profile());
        let dimensions = active.then(|| self.bat_dimensions()).flatten();
        if let Some(body) = self.player.as_mut()
            && body.form_dimensions.is_some() != dimensions.is_some()
        {
            body.form_dimensions = dimensions;
            let _ = body.refresh_dimensions();
            body.health.food.max_food = if active { CAPACITY } else { 20 };
            self.using = None;
        }
    }

    pub(in crate::game) fn magic_bat_tint(&self, uuid: u128) -> Option<[u8; 3]> {
        if self.remote.is_some() {
            self.magic.bat_visuals.forms.get(&uuid).copied()
        } else {
            self.magic
                .actors
                .get(&uuid)?
                .bat
                .as_ref()
                .map(|form| form.tint)
        }
    }

    pub(in crate::game) fn magic_draw_bat(
        &self,
        avatar: &Mob,
        tint: [u8; 3],
        camera: glam::DVec3,
        alpha: f32,
        light: u32,
        out: &mut Vec<Vertex>,
    ) {
        let mut model = self.magic.bat_visuals.model.borrow_mut();
        if model.is_none() {
            let Some(kind) = self.res.mob_types().get(BAT) else {
                return;
            };
            let mut mob = Mob::new(0, kind.clone(), Vec3::ZERO, 0.0);
            bat::set_resting(&mut mob, false);
            bat::setup_animation_states(&mut mob);
            mob.extra.model_scale = SCALE;
            *model = Some(mob);
        }
        let mob = model.as_mut().expect("bat model initialized");
        mob.age = avatar.age;
        mob.position = avatar.position;
        mob.previous = avatar.previous;
        mob.body_yaw = avatar.body_yaw;
        mob.previous_body_yaw = avatar.previous_body_yaw;
        mob.head_yaw = avatar.head_yaw;
        mob.previous_head_yaw = avatar.previous_head_yaw;
        mob.head_pitch = avatar.head_pitch;
        mob.previous_head_pitch = avatar.previous_head_pitch;
        mob.hurt_time = avatar.hurt_time;
        mob.death_time = avatar.death_time;
        let lookup = |key: &str| {
            self.gui
                .atlas
                .sprites
                .get(key)
                .map(|r| [r.u0, r.v0, r.u1, r.v1])
        };
        let first = out.len();
        render::render_with(
            mob,
            alpha,
            camera,
            &lookup,
            light,
            self.res.mob_data(),
            &[],
            out,
        );
        let color = u32::from_le_bytes([tint[0], tint[1], tint[2], 255]);
        for vertex in &mut out[first..] {
            vertex.color = horizon_client::hud_layout::multiply(vertex.color, color);
        }
    }
}
