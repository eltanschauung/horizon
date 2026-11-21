//! Screen layout for a registered menu that opens without a backing block.

#[horizon_mod_api_macros::replaceable]
pub(super) fn blockless_menu_screen(_menu: &str) -> Option<mod_api::Screen> {
    None
}
