//! Gensokyo Legacy's Miasma contact effect. Particle rendering is client-side.
use super::{Game, potions::mob_can_be_affected};
use horizon_client::{
    effects::{EffectInstance, MIASMA},
    world::World,
};
use horizon_core::geometry::Aabb;
use horizon_world::registry::BlockRegistry;

fn exposure(world: &World, registry: &BlockRegistry, bounds: Aabb) -> Option<(i32, i32)> {
    let lo = bounds.min_position();
    let hi = bounds.max_position();
    let mut effect = None;
    for x in (lo.x + 1e-7).floor() as i32..=(hi.x - 1e-7).floor() as i32 {
        for y in (lo.y + 1e-7).floor() as i32..=(hi.y - 1e-7).floor() as i32 {
            for z in (lo.z + 1e-7).floor() as i32..=(hi.z - 1e-7).floor() as i32 {
                let Some(state) = registry.state(world.block(x, y, z)) else {
                    continue;
                };
                let next = match state.name.as_str() {
                    "horizon_gensokyo_legacy:demonic_miasma_mushroom" => (0, 200),
                    "horizon_gensokyo_legacy:miasma_air" => {
                        let strong = state
                            .properties
                            .get("intensity")
                            .and_then(|value| value.parse::<u8>().ok())
                            .is_some_and(|intensity| intensity > 4);
                        (i32::from(strong), 30)
                    }
                    _ => continue,
                };
                effect = effect.max(Some(next));
            }
        }
    }
    effect
}

impl Game<'_> {
    pub(super) fn apply_miasma_on_hurt(&mut self) {
        use horizon_core::random::RandomSource;
        let effects = &self.potions.effects;
        if let Some(amp) = effects.amplifier(MIASMA)
            && ["slowness", "weakness", "poison"]
                .iter()
                .all(|id| effects.amplifier(id).is_none())
            && let Some((effect, amp)) =
                horizon_client::effects::miasma_on_hurt(amp, self.world.random.next_double())
        {
            self.add_player_effect(EffectInstance::new(effect, 100, amp));
        }
    }

    pub(super) fn tick_miasma_exposure(&mut self) {
        if horizon_client::brewing::effect(MIASMA).is_none() {
            return;
        }
        let pause_refresh = self.world.game_time % 20 == 0;
        for player in self.fight_players().into_iter().filter(|p| p.alive) {
            let Some((amp, duration)) =
                exposure(&self.world, &self.pack.assets.registry, player.bounds)
            else {
                continue;
            };
            self.as_fight_player(player.who, |game| {
                if game.spectator()
                    || (pause_refresh && game.potions.effects.amplifier(MIASMA).is_some())
                {
                    return;
                }
                let mut effect = EffectInstance::new(MIASMA, duration, amp);
                effect.ambient = true;
                effect.visible = false;
                game.add_player_effect(effect);
            });
        }
        let tags = &self.res.stations().enchantments.entity_tags;
        for mob in self.mobs.iter_mut().filter(|m| m.alive()) {
            if let Some((amp, duration)) =
                exposure(&self.world, &self.pack.assets.registry, mob.bounds())
                && (!pause_refresh || !mob.has_effect(MIASMA))
                && mob_can_be_affected(tags, &mob.kind.id, MIASMA)
            {
                mob.add_effect_with(MIASMA, duration, amp, true, false);
            }
        }
    }
}
