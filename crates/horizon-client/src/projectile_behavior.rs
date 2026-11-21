//! Replacement boundaries for mod-owned projectile motion, appearance and persistence.

use crate::items::ItemStack;
use crate::projectiles::{Hit, Projectile, ProjectileWorld};
use horizon_core::nbt::Tag;

#[horizon_mod_api_macros::replaceable]
pub fn custom_tick(
    _shot: &mut Projectile,
    _world: &dyn ProjectileWorld,
    _impact: bool,
) -> Option<(bool, Hit)> {
    None
}

#[horizon_mod_api_macros::replaceable]
pub fn custom_drawn_item(_shot: &Projectile, vanilla: Option<ItemStack>) -> Option<ItemStack> {
    vanilla
}

#[horizon_mod_api_macros::replaceable]
pub fn save_custom_projectile(_shot: &Projectile, _tag: &mut Tag) {}

#[horizon_mod_api_macros::replaceable]
pub fn load_custom_projectile(_shot: &mut Projectile, _tag: &Tag) {}
