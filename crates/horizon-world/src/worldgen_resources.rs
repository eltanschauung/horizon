//! Resource transformation points after lookup and feature holder expansion.

#[horizon_mod_api_macros::replaceable]
pub(crate) fn expanded_feature(resource: String) -> String {
    resource
}

#[horizon_mod_api_macros::replaceable]
pub(crate) fn biome_resource(resource: &str) -> &str {
    resource
}
