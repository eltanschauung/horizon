/// Final compatibility after resolving both enchantments' exclusive sets.
#[horizon_mod_api_macros::replaceable]
pub(crate) fn compatible(_a: &str, _b: &str, compatible: bool) -> bool {
    compatible
}
