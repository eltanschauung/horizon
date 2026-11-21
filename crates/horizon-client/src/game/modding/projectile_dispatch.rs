//! Server host access for a replacement of a mod-owned projectile's default hit.

use horizon_client::projectiles::{Hit, Projectile};

#[horizon_mod_api_macros::replaceable]
pub(super) fn handle_custom_impact(
    _host: &mut dyn mod_api::ServerHost,
    _projectile: &Projectile,
    _hit: Hit,
) -> bool {
    false
}
