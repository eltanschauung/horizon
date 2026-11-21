//! Final presentation values for mob variants.

use crate::mobs::Species;

#[horizon_mod_api_macros::replaceable]
pub(crate) fn index(_species: Species, index: u8) -> u8 {
    index
}

#[horizon_mod_api_macros::replaceable]
pub(crate) fn sound_id(_species: Species, _kind: &str, id: String) -> String {
    id
}
