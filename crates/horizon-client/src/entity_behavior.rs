//! Replacement boundaries for mods that add behavior to existing entity types.

use crate::mobs::{Mob, MobWorld};

#[horizon_mod_api_macros::replaceable]
pub fn before_mob_tick(_mob: &mut Mob, _world: &MobWorld<'_>) {}

#[horizon_mod_api_macros::replaceable]
pub fn silence_mob(_mob: &Mob) -> bool {
    false
}

#[horizon_mod_api_macros::replaceable]
pub fn suppress_chicken_eggs(_mob: &Mob) -> bool {
    false
}

#[horizon_mod_api_macros::replaceable]
pub fn protect_dropped_item(_item: &str) -> bool {
    false
}
