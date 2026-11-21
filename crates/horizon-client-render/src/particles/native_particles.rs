//! Optional native particle appearance, motion, and factory overrides.
use super::{Particle, ParticleOptions};

#[horizon_mod_api_macros::replaceable]
pub(super) fn tick(_particle: &mut Particle) -> bool {
    false
}

#[horizon_mod_api_macros::replaceable]
pub(super) fn light(_particle: &Particle, _sky: u32, _block: u32) -> Option<(u32, u32)> {
    None
}

#[horizon_mod_api_macros::replaceable]
pub(super) fn make(_options: &ParticleOptions, _pos: [f64; 3], _velocity: [f64; 3]) -> Option<Particle> {
    None
}
