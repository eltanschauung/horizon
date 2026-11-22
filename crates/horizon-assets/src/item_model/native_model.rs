use super::ComponentsView;

#[horizon_mod_api_macros::replaceable]
pub(super) fn foil(_components: &ComponentsView<'_>) -> Option<bool> {
    None
}
