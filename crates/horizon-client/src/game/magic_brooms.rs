//! Items, workstations and rider interactions for the Magic Brooms content pack.
use super::{Game, Hit};
use horizon_assets::item_model::DisplayContext;
use horizon_client::{
    collisions::SceneCollisions,
    effects::EffectInstance,
    items::ItemStack,
    mobs::{
        Damage, DamageKind, Mob,
        magic_brooms::{Family, NAMESPACE, Variant},
    },
    particles::{ParticleOptions, ParticleType},
};
use horizon_core::{
    geometry::{Aabb, Vec3},
    position::BlockPos,
};
use std::sync::Arc;

const FAMILY_NAMES: [&str; 6] = ["fire", "ice", "lightning", "ender", "grove", "tide"];
const CATALYSTS: [&str; 6] = [
    "minecraft:blaze_powder",
    "minecraft:packed_ice",
    "minecraft:redstone",
    "minecraft:chorus_fruit",
    "minecraft:vine",
    "minecraft:prismarine",
];
const UPGRADES: [&str; 6] = [
    "minecraft:magma_cream",
    "minecraft:blue_ice",
    "minecraft:lightning_rod",
    "minecraft:ender_eye",
    "minecraft:emerald",
    "minecraft:heart_of_the_sea",
];

fn family_name(family: Family) -> &'static str {
    match family {
        Family::Magic => "magic",
        Family::Fire => "fire",
        Family::Ice => "ice",
        Family::Lightning => "lightning",
        Family::Ender => "ender",
        Family::Grove => "grove",
        Family::Tide => "tide",
    }
}

impl Game<'_> {
    fn broom_space(&self, p: Vec3, height: f64) -> bool {
        let (x, y, z) = (p.x, p.y, p.z);
        let bounds = Aabb::new(
            Vec3::new(x - 0.45, y, z - 0.45),
            Vec3::new(x + 0.45, y + height, z + 0.45),
        );
        let scene = SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: None,
        };
        self.world
            .chunks
            .get(&((x.floor() as i32) >> 4, (z.floor() as i32) >> 4))
            .is_some()
            && scene.cells_without_collision(bounds)
    }

    fn place_broom_at(&mut self, mut stack: ItemStack, position: Vec3, rack: bool) -> bool {
        let at = [position.x, position.y, position.z];
        let Some(id) = self.spawn_mob(&stack.item, at, true) else {
            return false;
        };
        let Some(broom) = self.mobs.iter_mut().find(|mob| mob.id == id) else {
            return false;
        };
        broom.yaw = self.yaw;
        broom.head_yaw = self.yaw;
        broom.body_yaw = self.yaw;
        broom.previous_yaw = self.yaw;
        broom.previous_body_yaw = self.yaw;
        broom.no_ai = true;
        broom.no_gravity = rack;
        stack.count = 1;
        broom.extra.set_main_hand_stack(Some(stack));
        self.consume_held();
        self.swing_arm();
        true
    }

    pub(super) fn use_magic_broom(&mut self, stack: &ItemStack) -> bool {
        if Variant::of(&stack.item).is_none() || self.riding.is_some() {
            return false;
        }
        let Some(player) = self.player.as_ref() else {
            return false;
        };
        let yaw = f64::from(self.yaw).to_radians();
        let position =
            player
                .position
                .add_vector(Vec3::new(-yaw.sin() * 1.4, 0.2, yaw.cos() * 1.4));
        if !self.broom_space(position, 0.45) {
            self.use_failed = true;
            return true;
        }
        self.place_broom_at(stack.clone(), position, false)
    }

    fn recover_magic_broom(&mut self, index: usize) {
        let broom = &mut self.mobs[index];
        let stack = broom
            .extra
            .main_hand_stack()
            .cloned()
            .unwrap_or_else(|| ItemStack::new(&broom.kind.id, 1));
        let at = [broom.position.x, broom.position.y, broom.position.z];
        broom.removed = true;
        if let Some(left) = self.give(stack) {
            self.spawn_item(left, at, [0.0, 0.15, 0.0], 10);
        }
        self.swing_arm();
    }

    pub(super) fn interact_magic_broom(&mut self, index: usize) -> bool {
        let broom = &self.mobs[index];
        if Variant::of(&broom.kind.id).is_none() {
            return false;
        }
        if !broom.alive()
            || broom.removed
            || broom.rider.is_some()
            || self.ridden_by_guest(broom.id)
        {
            return true;
        }
        let sneaking = self.keys.held(super::Mapping::Sneak);
        if sneaking || broom.no_gravity {
            self.recover_magic_broom(index);
        } else if self.riding.is_none()
            && self.broom_space(broom.position, 1.7)
            && self.native_may_board(broom.id)
        {
            let id = self.mobs[index].id;
            self.mount.reset_jump();
            self.start_riding(id);
            self.position_rider();
            self.swing_arm();
        }
        true
    }

    fn set_broom_stage(&mut self, hit: &Hit, stage: &str) -> bool {
        let registry = &self.pack.assets.registry;
        let [x, y, z] = hit.pos;
        let Some(mut state) = registry.state(self.world.block(x, y, z)).cloned() else {
            return false;
        };
        state.properties.insert("stage".into(), stage.into());
        let Some(next) = registry.id(&state) else {
            return false;
        };
        self.with_level(|b, level, ticks| {
            b.set_block(level, ticks, BlockPos::new(x, y, z), next, 3)
        })
    }

    pub(super) fn use_broom_station(&mut self, hit: &Hit) -> bool {
        let [x, y, z] = hit.pos;
        let Some(state) = self.pack.assets.registry.state(self.world.block(x, y, z)) else {
            return false;
        };
        let Some(station) = state.name.strip_prefix("horizon_magic_brooms:") else {
            return false;
        };
        let held = self.held().cloned();
        let item = held.as_ref().map_or("", |s| s.item.as_ref());
        if station == "broom_rack" {
            let position = Vec3::new(
                f64::from(x) + 0.5,
                f64::from(y) + 1.0,
                f64::from(z)
                    + if hit.point[2] - f64::from(z) < 0.5 {
                        0.23
                    } else {
                        0.77
                    },
            );
            let occupied = self.mobs.iter().position(|mob| {
                mob.no_gravity
                    && !mob.removed
                    && Variant::of(&mob.kind.id).is_some()
                    && mob.position.subtract(position).length() < 0.1
            });
            if let Some(index) = occupied {
                if item.is_empty() {
                    self.recover_magic_broom(index);
                }
            } else if let Some(stack) = held
                && Variant::of(&stack.item).is_some()
            {
                self.place_broom_at(stack, position, true);
            }
            return true;
        }
        let stage = state
            .properties
            .get("stage")
            .map_or("empty", String::as_str);
        let catalyst = CATALYSTS
            .iter()
            .position(|id| *id == item)
            .map(|i| FAMILY_NAMES[i]);
        let (next, result) = match (station, stage, item) {
            ("scroll_table", "empty", "minecraft:paper") => ("paper", None),
            ("scroll_table", "paper", _) if catalyst.is_some() => (catalyst.unwrap(), None),
            ("scroll_table", family, "") if FAMILY_NAMES.contains(&family) => {
                ("empty", Some(format!("{NAMESPACE}:{family}_scroll")))
            }
            ("broom_workbench", "empty", wood)
                if ["_log", "_stem", "_wood", "_hyphae", "_planks"]
                    .iter()
                    .any(|suffix| wood.ends_with(suffix))
                    || wood == "minecraft:stick" =>
            {
                ("handle", None)
            }
            ("broom_workbench", "handle", "minecraft:hay_block") => ("bristles", None),
            ("broom_workbench", "bristles", "minecraft:diamond") => ("magic_upgraded", None),
            ("broom_workbench", "bristles", _) if catalyst.is_some() => (catalyst.unwrap(), None),
            ("broom_workbench", "bristles", "") => {
                ("empty", Some(format!("{NAMESPACE}:broom_magic")))
            }
            ("broom_workbench", family, "") if family != "empty" && family != "handle" => {
                ("empty", Some(format!("{NAMESPACE}:broom_{family}")))
            }
            ("broom_workbench", family, _)
                if FAMILY_NAMES
                    .iter()
                    .position(|v| *v == family)
                    .is_some_and(|i| UPGRADES[i] == item) =>
            {
                let next = format!("{family}_upgraded");
                if self.set_broom_stage(hit, &next) {
                    self.consume_held();
                    self.swing_arm();
                }
                return true;
            }
            ("broom_workbench" | "scroll_table", _, _) => return true,
            _ => return false,
        };
        if self.set_broom_stage(hit, next) {
            if let Some(result) = result {
                let stack = ItemStack::new(&result, 1);
                if let Some(left) = self.give(stack) {
                    self.spawn_item(
                        left,
                        [f64::from(x) + 0.5, f64::from(y) + 1.2, f64::from(z) + 0.5],
                        [0.0, 0.15, 0.0],
                        10,
                    );
                }
            } else {
                self.consume_held();
            }
            self.swing_arm();
        }
        true
    }

    pub(super) fn magic_broom_passives(&mut self, index: usize) {
        let broom = &self.mobs[index];
        let Some(variant) = Variant::of(&broom.kind.id) else {
            return;
        };
        let tick = self.world.game_time;
        if tick % 20 == 0 {
            if variant.upgraded && !broom.on_ground {
                self.add_player_effect(
                    EffectInstance::new("night_vision", 45, 0).with_flags(false, false),
                );
            }
            if variant.family == Family::Tide {
                self.add_player_effect(
                    EffectInstance::new("water_breathing", 45, 0).with_flags(false, false),
                );
            }
        }
        if variant.family == Family::Grove && tick % (if variant.upgraded { 80 } else { 100 }) == 0
        {
            self.add_player_effect(
                EffectInstance::new(
                    "regeneration",
                    if variant.upgraded { 45 } else { 35 },
                    i32::from(variant.upgraded),
                )
                .with_flags(false, false),
            );
        }
    }

    pub(super) fn use_broom_scroll(&mut self, stack: &ItemStack) -> bool {
        let Some(index) = self.mount_index() else {
            return false;
        };
        let Some(variant) = Variant::of(&self.mobs[index].kind.id) else {
            return false;
        };
        if variant.family == Family::Magic
            || stack.item.as_ref() != format!("{NAMESPACE}:{}_scroll", family_name(variant.family))
        {
            return false;
        }
        let at = self.mobs[index].position;
        let id = self.mobs[index].id;
        if variant.family == Family::Ender {
            let yaw = f64::from(self.yaw).to_radians();
            let pitch = f64::from(self.pitch).to_radians();
            let direction = Vec3::new(
                -yaw.sin() * pitch.cos(),
                -pitch.sin(),
                yaw.cos() * pitch.cos(),
            );
            let max = if variant.upgraded { 36 } else { 24 };
            let target = (4..=max)
                .rev()
                .map(|step| at.add_vector(direction.scale(f64::from(step) * 0.5)))
                .find(|&p| self.broom_space(p, 1.7));
            let Some(target) = target else {
                self.use_failed = true;
                return true;
            };
            self.mobs[index].position = target;
            self.mobs[index].previous = target;
            self.mobs[index].velocity = direction
                .scale(if variant.upgraded { 0.38 } else { 0.24 })
                .add_vector(Vec3::new(
                    0.0,
                    if variant.upgraded { 0.08 } else { 0.04 },
                    0.0,
                ));
            self.position_rider();
        } else {
            if variant.family == Family::Lightning {
                let rays = if variant.upgraded { 7 } else { 3 };
                for ray in 0..rays {
                    let angle = std::f64::consts::TAU * f64::from(ray) / f64::from(rays);
                    let point = [at.x + angle.cos() * 4.6, at.y, at.z + angle.sin() * 4.6];
                    let pos = point.map(|v| v.floor() as i32);
                    if self.lightning_strike_allowed(self.lan_acting_uuid, pos, "magic_broom") {
                        self.spawn_bolt(point, false);
                        if let Some(bolt) = self.bolts.last_mut() {
                            bolt.cause = Some(self.lan_acting);
                        }
                    }
                }
            }
            if variant.family == Family::Tide {
                self.add_player_effect(
                    EffectInstance::new(
                        "water_breathing",
                        if variant.upgraded { 520 } else { 360 },
                        0,
                    )
                    .with_flags(false, false),
                );
            }
            let (radius, particle) = match variant.family {
                Family::Fire => (
                    if variant.upgraded { 60.0 } else { 30.0 },
                    ParticleType::Flame,
                ),
                Family::Ice => (
                    if variant.upgraded { 20.0 } else { 15.0 },
                    ParticleType::Snowflake,
                ),
                Family::Lightning => (6.2, ParticleType::ElectricSpark),
                Family::Grove => (
                    if variant.upgraded { 9.0 } else { 6.0 },
                    ParticleType::HappyVillager,
                ),
                Family::Tide => (
                    if variant.upgraded { 14.0 } else { 10.0 },
                    ParticleType::Splash,
                ),
                _ => unreachable!(),
            };
            if variant.family == Family::Grove {
                self.add_player_effect(EffectInstance::new(
                    "regeneration",
                    100,
                    i32::from(variant.upgraded),
                ));
            }
            let yaw = f64::from(self.yaw.to_radians());
            let forward = Vec3::new(-yaw.sin(), 0.0, yaw.cos());
            for mob in &mut self.mobs {
                if !mob.alive() || mob.id == id || Variant::of(&mob.kind.id).is_some() {
                    continue;
                }
                let delta = mob.position.subtract(at);
                let affected = if variant.family == Family::Tide {
                    let reach = if variant.upgraded { 22.0 } else { 16.0 };
                    let along = delta.x * forward.x + delta.z * forward.z;
                    let cross = delta.x * forward.z - delta.z * forward.x;
                    (0.0..=reach).contains(&along) && cross.abs() <= 4.0 && delta.y.abs() <= 3.0
                } else {
                    delta.length_squared() <= radius * radius
                };
                if !affected {
                    continue;
                }
                match variant.family {
                    Family::Fire => {
                        mob.fire_ticks =
                            mob.fire_ticks.max(if variant.upgraded { 160 } else { 100 });
                        mob.damage(
                            if variant.upgraded { 6.0 } else { 4.0 },
                            Damage::of(DamageKind::Magic),
                        );
                    }
                    Family::Ice => {
                        mob.damage(
                            if variant.upgraded { 3.0 } else { 2.0 },
                            Damage::of(DamageKind::Magic),
                        );
                        mob.add_effect("slowness", 100, if variant.upgraded { 2 } else { 1 });
                    }
                    Family::Grove if mob.hostile() => {
                        mob.add_effect("slowness", 60, 0);
                    }
                    Family::Grove => mob.heal(if variant.upgraded { 4.0 } else { 2.0 }),
                    Family::Tide => {
                        mob.velocity = mob
                            .velocity
                            .add_vector(forward.scale(0.65))
                            .add_vector(Vec3::new(0.0, 0.25, 0.0));
                        mob.damage(
                            if variant.upgraded { 5.0 } else { 3.0 },
                            Damage::of(DamageKind::Magic),
                        );
                    }
                    _ => {}
                }
            }
            for i in 0..20 {
                let angle = f64::from(i) * std::f64::consts::TAU / 20.0;
                let p = [
                    at.x + angle.cos() * radius.min(8.0),
                    at.y + 0.3,
                    at.z + angle.sin() * radius.min(8.0),
                ];
                self.creatures
                    .lan_particles
                    .push(super::lan_particles::QueuedParticle {
                        dimension: self.dim.id(),
                        options: ParticleOptions::simple(particle),
                        state: None,
                        at: p,
                        spread: [0.15; 3],
                        speed: [0.01; 3],
                        count: 4,
                        randomization: 0,
                    });
            }
        }
        self.consume_held();
        self.add_cooldown(Arc::clone(&stack.item), variant.cooldown());
        self.swing_arm();
        true
    }

    pub(super) fn draw_magic_broom(
        &self,
        mob: &Mob,
        camera: glam::DVec3,
        alpha: f64,
        packed: u32,
        out: &mut Vec<horizon_client::mesher::Vertex>,
    ) -> bool {
        if Variant::of(&mob.kind.id).is_none() {
            return false;
        }
        let stack = ItemStack::new(&mob.kind.id, 1);
        let model = self.resolve_item(&stack, DisplayContext::Fixed, None, mob.id as i32);
        let p = mob
            .previous
            .add_vector(mob.position.subtract(mob.previous).scale(alpha));
        let relative = (glam::DVec3::new(p.x, p.y, p.z) - camera).as_vec3();
        let yaw = mob.yaw.to_radians();
        let lateral = mob.velocity.x * f64::from(yaw.cos()) + mob.velocity.z * f64::from(yaw.sin());
        let pose = glam::Mat4::from_translation(relative)
            * glam::Mat4::from_rotation_y(-(mob.yaw + 180.0).to_radians())
            * glam::Mat4::from_rotation_x(mob.head_pitch.clamp(-19.0, 19.0).to_radians())
            * glam::Mat4::from_rotation_z((lateral as f32 * 1.05).clamp(-0.36, 0.36));
        self.submit_item_world(&model, pose, packed, out);
        true
    }
}
