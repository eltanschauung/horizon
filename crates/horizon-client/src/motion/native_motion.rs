//! Optional native changes around the normal player simulation.
use super::MotionWorld;
use horizon_core::geometry::Vec3;

#[horizon_mod_api_macros::replaceable]
pub(super) fn player_input(_world: &mut MotionWorld<'_, '_, '_>, _jumping: bool) {}

#[horizon_mod_api_macros::replaceable]
pub(super) fn server_travel(_world: &mut MotionWorld<'_, '_, '_>) {}

#[horizon_mod_api_macros::replaceable]
pub(super) fn glide_gravity(_world: &MotionWorld<'_, '_, '_>, gravity: f64) -> f64 {
    gravity
}

#[horizon_mod_api_macros::replaceable]
pub(super) fn glide_velocity(_world: &MotionWorld<'_, '_, '_>, velocity: Vec3) -> Vec3 {
    velocity
}
