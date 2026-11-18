//! Flight for Horizon's port of Iced Fox Studios' Magic Brooms.
use super::{Mob, MobEvent, MobWorld};
use crate::collisions::SceneCollisions;
use horizon_core::geometry::{Aabb, Vec3};
use horizon_world::registry::BlockRegistry;

pub const NAMESPACE: &str = "horizon_magic_brooms";
pub const RACK: &str = "horizon_magic_brooms:broom_rack";
// The rider's feet anchor is below the broom entity origin and slightly behind it.
pub const SEAT: [f64; 3] = [0.0, -0.15, -0.1];

// Keep the saved entities inside the rack's central column, including the
// two offsets used by older versions. The renderer maps them to the slots.
const RACK_MARKERS: [f64; 3] = [0.23, 0.5, 0.77];

pub fn rack_marker(pos: [i32; 3], slot: usize) -> Vec3 {
    let [x, y, z] = pos.map(f64::from);
    Vec3::new(x + 0.5, y + 1.0, z + RACK_MARKERS[slot])
}

pub fn stored_rack_slot(mob: &Mob) -> Option<([i32; 3], usize)> {
    if !mob.no_gravity {
        return None;
    }
    let p = mob.position;
    let pos = [p.x, p.y - 0.1, p.z].map(|v| v.floor() as i32);
    RACK_MARKERS.iter().enumerate().find_map(|(slot, _)| {
        (p.subtract(rack_marker(pos, slot)).length_squared() < 0.01).then_some((pos, slot))
    })
}

pub fn rack_facing<'a>(
    registry: &'a BlockRegistry,
    world: &crate::world::World,
    [x, y, z]: [i32; 3],
) -> Option<&'a str> {
    let state = registry.state(world.block(x, y, z))?;
    (state.name == RACK).then(|| {
        state
            .properties
            .get("facing")
            .map_or("north", String::as_str)
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Magic,
    Fire,
    Ice,
    Lightning,
    Ender,
    Grove,
    Tide,
}

#[derive(Clone, Copy)]
pub struct Variant {
    pub family: Family,
    pub upgraded: bool,
}

impl Variant {
    pub fn of(id: &str) -> Option<Self> {
        let name = id.strip_prefix("horizon_magic_brooms:broom_")?;
        let (name, upgraded) = name
            .strip_suffix("_upgraded")
            .map_or((name, false), |s| (s, true));
        let family = match name {
            "magic" => Family::Magic,
            "fire" => Family::Fire,
            "ice" => Family::Ice,
            "lightning" => Family::Lightning,
            "ender" => Family::Ender,
            "grove" => Family::Grove,
            "tide" => Family::Tide,
            _ => return None,
        };
        Some(Self { family, upgraded })
    }

    pub fn cooldown(self) -> i32 {
        match (self.family, self.upgraded) {
            (Family::Magic, _) => 0,
            (Family::Ender, false) => 130,
            (Family::Ender, true) => 90,
            (Family::Grove, false) => 240,
            (Family::Grove, true) => 190,
            (Family::Tide, false) => 180,
            (Family::Tide, true) => 140,
            _ => 200,
        }
    }

    fn cruise(self) -> f64 {
        let (speed, thrust) = match (self.family, self.upgraded) {
            (Family::Ender, false) => (5.0, 0.022),
            (Family::Ender, true) => (7.0, 0.030),
            (Family::Grove, false) => (3.0, 0.007),
            (Family::Grove, true) => (5.0, 0.011),
            (Family::Tide, false) => (4.0, 0.009),
            (Family::Tide, true) => (6.0, 0.014),
            (_, false) => (4.0, 0.008),
            (_, true) => (6.0, 0.012),
        };
        (0.22 + speed * 0.029 + thrust * 5.2) * 1.08
    }
}

pub(super) fn tick(
    mob: &mut Mob,
    w: &MobWorld,
    events: &mut Vec<MobEvent>,
    variant: Variant,
) -> bool {
    if !mob.alive() || mob.removed {
        return false;
    }
    mob.age = mob.age.saturating_add(1);
    mob.extra.broom_dash_cooldown = mob.extra.broom_dash_cooldown.saturating_sub(1);
    mob.previous = mob.position;
    mob.previous_yaw = mob.yaw;
    mob.previous_body_yaw = mob.body_yaw;

    if let Some((pos, _)) = stored_rack_slot(mob) {
        if !w.world.loaded(pos[0], pos[2]) || rack_facing(w.registry, w.world, pos).is_some() {
            return true;
        }
        let stack = mob
            .extra
            .main_hand_stack()
            .cloned()
            .unwrap_or_else(|| crate::items::ItemStack::new(&mob.kind.id, 1));
        events.push(MobEvent::ThrowItem {
            mob: mob.id,
            stack,
            at: [mob.position.x, mob.position.y, mob.position.z],
            motion: [0.0, 0.12, 0.0],
            pickup_delay: 10,
        });
        mob.removed = true;
        return false;
    }
    mob.no_gravity = false;
    if let Some(rider) = mob.rider {
        let yaw = f64::from(rider.yaw).to_radians();
        let forward = Vec3::new(-yaw.sin(), 0.0, yaw.cos());
        let left = Vec3::new(yaw.cos(), 0.0, yaw.sin());
        let wish = forward
            .scale(f64::from(rider.forward))
            .add_vector(left.scale(f64::from(rider.strafe) * 0.72));
        let strength = wish.length().min(1.0);
        let current = mob.velocity;
        let water = variant.family == Family::Tide && w.fluids(mob.bounds(), mob.eye_y()).in_water;
        let mut speed = variant.cruise();
        if water {
            speed *= if variant.upgraded { 1.32 } else { 1.22 };
        }
        if rider.forward < 0.0 {
            speed *= 0.58;
        }
        if rider.sprint && variant.family == Family::Ender {
            speed += if variant.upgraded { 0.099 } else { 0.066 };
        }

        let mut horizontal = Vec3::new(current.x, 0.0, current.z);
        if strength > 0.0001 {
            let target = wish.normalize().scale(speed * strength);
            let response = if variant.upgraded { 0.1035 } else { 0.0855 };
            horizontal = horizontal.add_vector(target.subtract(horizontal).scale(response));
        } else {
            horizontal = horizontal.scale(if mob.on_ground { 0.72 } else { 0.982 });
        }
        let lift = (horizontal.length() / speed).clamp(0.0, 1.15);
        let mut y = current.y * (if water { 0.88 } else { 0.965 })
            + if water {
                0.006 + 0.012 * lift
            } else {
                -0.024 + 0.021 * lift
            };
        if rider.jumping {
            y += if variant.upgraded { 0.054 } else { 0.045 };
        }
        if rider.sprint {
            y -= if variant.upgraded { 0.063 } else { 0.0522 };
        }
        y -= f64::from(rider.pitch).to_radians().sin().clamp(-0.45, 0.55)
            * f64::from(rider.forward.max(0.0))
            * 0.0288;
        y = y.clamp(
            if water { -0.24 } else { -0.48 },
            if variant.upgraded { 0.31 } else { 0.26 },
        );
        if mob.on_ground && rider.jumping {
            y = y.max(if variant.upgraded { 0.1305 } else { 0.1125 });
        }
        mob.velocity = Vec3::new(horizontal.x, y, horizontal.z);
        if variant.family == Family::Ender
            && rider.jumping
            && !mob.extra.broom_was_ascending
            && !mob.on_ground
            && mob.extra.broom_dash_cooldown == 0
        {
            let dash_direction = if strength > 0.0001 {
                wish.normalize()
            } else {
                forward
            };
            let thrust = if variant.upgraded { 0.855 } else { 0.648 };
            let lift = if variant.upgraded { 0.198 } else { 0.162 };
            mob.velocity = mob
                .velocity
                .add_vector(dash_direction.scale(thrust))
                .add_vector(Vec3::new(0.0, lift, 0.0));
            mob.extra.broom_dash_cooldown = if variant.upgraded { 30 } else { 44 };
        }
        mob.extra.broom_was_ascending = rider.jumping;
        let turn = (rider.yaw - mob.yaw + 180.0).rem_euclid(360.0) - 180.0;
        mob.yaw += turn.clamp(-14.0, 14.0);
        mob.body_yaw = mob.yaw;
        mob.head_yaw = mob.yaw;
        mob.head_pitch += (rider.pitch * 0.22 - y as f32 * 34.0 - mob.head_pitch) * 0.2;
        // Sweep clearance for the occupant as well as the broom's own collision box.
        let bounds = mob.bounds();
        let hi = bounds.max_position();
        let clearance = Aabb::new(bounds.min_position(), Vec3::new(hi.x, hi.y + 1.2, hi.z));
        let scene = SceneCollisions {
            world: w.world,
            binding: w.collision,
            subject: None,
        };
        let safe = scene
            .resolve_in_border_with(
                clearance,
                mob.velocity,
                0.0,
                mob.on_ground,
                w.border.map(|b| (b, [mob.position.x, mob.position.z])),
                &[],
            )
            .unwrap_or(Vec3::ZERO);
        // The broom's own movement still owns ground detection.
        mob.velocity.x = safe.x;
        mob.velocity.z = safe.z;
        if mob.velocity.y > 0.0 {
            mob.velocity.y = safe.y;
        }
    } else {
        mob.extra.broom_was_ascending = false;
        mob.velocity.x *= 0.65;
        mob.velocity.y = (mob.velocity.y - 0.05).max(-0.55);
        mob.velocity.z *= 0.65;
    }
    mob.fall_distance = 0.0;
    mob.do_move(w, events);
    mob.fall_distance = 0.0;
    if mob.rider.is_some() && !mob.on_ground && mob.age % 3 == 0 {
        let kind = match variant.family {
            Family::Magic => "minecraft:enchant",
            Family::Fire => "minecraft:flame",
            Family::Ice => "minecraft:snowflake",
            Family::Lightning => "minecraft:electric_spark",
            Family::Ender => "minecraft:dragon_breath",
            Family::Grove => "minecraft:happy_villager",
            Family::Tide => "minecraft:splash",
        };
        let yaw = f64::from(mob.yaw).to_radians();
        let p = mob.position;
        events.push(MobEvent::LevelParticles {
            kind,
            at: [p.x + yaw.sin() * 0.85, p.y + 0.3, p.z - yaw.cos() * 0.85],
            spread: [0.08; 3],
            speed: 0.018,
            count: if variant.upgraded { 4 } else { 2 },
        });
    }
    mob.alive()
}
