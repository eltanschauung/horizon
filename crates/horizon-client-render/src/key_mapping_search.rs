/// Match the localized name of a mapping in the Controls search.
#[horizon_mod_api_macros::replaceable]
pub(crate) fn matches(name: &str, _bound_key: &str, query: &str) -> bool {
    query.is_empty() || name.to_lowercase().contains(query)
}
