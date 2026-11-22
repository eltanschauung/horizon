//! Optional terrain changes used by native world-generation mods.
use super::generator::{GeneratorData, GeneratorError, ProtoChunk};
use super::overrides::{LevelHeight, WorldgenOverrides};
use std::collections::BTreeMap;
use std::sync::Arc;

#[horizon_mod_api_macros::replaceable]
pub(super) fn registry_defaults(
    _overrides: &WorldgenOverrides,
    _bundle: &str,
    _base: &mut BTreeMap<String, String>,
) {
}

/// Rewrite the effective registry after content packs and world datapacks have been applied.
#[horizon_mod_api_macros::replaceable]
pub(super) fn registry_effective(
    _overrides: &WorldgenOverrides,
    _bundle: &str,
    _base: &mut BTreeMap<String, String>,
) {}

#[horizon_mod_api_macros::replaceable]
pub(super) fn level_height(
    _overrides: &WorldgenOverrides,
    _level: &str,
) -> Option<LevelHeight> {
    None
}

#[horizon_mod_api_macros::replaceable]
pub(super) fn temperature_start(start: i32) -> i32 {
    start
}

#[horizon_mod_api_macros::replaceable]
pub(super) fn structure_start_y(y: i32, _width: i32, _height: i32, _depth: i32) -> i32 {
    y
}

#[horizon_mod_api_macros::replaceable]
pub(super) fn decorate(
    _data: &Arc<GeneratorData>,
    _seed: i64,
    _region: &mut [ProtoChunk; 9],
) -> Result<(), GeneratorError> {
    Ok(())
}
