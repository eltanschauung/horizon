//! Freeze generation-mod selections per world, separately from menu defaults.
use crate::{biome::Biomes, mod_list::ModList};
use horizon_world::worldgen::overrides::WorldgenOverrides;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};
const FILE: &str = "horizon-worldgen-mods.json";
#[derive(Serialize, Deserialize)]
struct Selection {
    version: u8,
    mods: BTreeMap<String, bool>,
    required_biomes: Vec<String>,
}
fn current() -> Selection {
    let mods = ModList::load(&crate::logging::game_home());
    let mut flags: BTreeMap<String, bool> =
        mods.enabled.iter().map(|id| (id.clone(), true)).collect();
    flags.extend(mods.disabled.iter().map(|id| (id.clone(), false)));
    Selection {
        version: 1,
        mods: flags,
        required_biomes: Vec::new(),
    }
}
fn read(world: &Path) -> Result<Selection, String> {
    let p = world.join(FILE);
    match std::fs::read(&p) {
        Ok(bytes) => {
            let s: Selection =
                serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", p.display()))?;
            if s.version != 1 {
                return Err("unsupported generation-mod selection version".into());
            }
            Ok(s)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(current()),
        Err(e) => Err(e.to_string()),
    }
}
/// Called before opening any saved chunks. Missing content must never become air.
pub fn capture(world: &Path, biomes: &Biomes) -> Result<(), String> {
    let mut selection = read(world)?;
    for id in &selection.required_biomes {
        if !biomes.contains(id) {
            return Err(format!(
                "This world requires biome {id}. Enable its content mod in Options > Mods before opening it."
            ));
        }
    }
    if !world.join(FILE).exists() {
        selection.required_biomes = biomes
            .list
            .iter()
            .filter(|b| !b.name.starts_with("minecraft:"))
            .map(|b| b.name.clone())
            .collect();
        std::fs::create_dir_all(world).map_err(|e| e.to_string())?;
        let text = serde_json::to_vec_pretty(&selection).map_err(|e| e.to_string())?;
        std::fs::write(world.join(FILE), text).map_err(|e| e.to_string())?;
    }
    Ok(())
}
pub fn apply(world: Option<&Path>, ov: &mut WorldgenOverrides) -> Result<(), String> {
    let selection = match world {
        Some(p) => read(p)?,
        None => current(),
    };
    ov.set_mods(selection.mods);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_flags_override_menu_defaults_and_invalid_versions_fail() {
        let dir = std::env::temp_dir().join(format!(
            "horizon-generation-selection-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE);
        std::fs::write(
            &path,
            r#"{"version":1,"mods":{"tectonic_horizon":false},"required_biomes":[]}"#,
        )
        .unwrap();
        let mut ov = WorldgenOverrides::default();
        apply(Some(&dir), &mut ov).unwrap();
        assert!(!ov.mod_enabled("tectonic_horizon"));
        assert!(ov.mod_enabled("other"));
        std::fs::write(&path, r#"{"version":2,"mods":{},"required_biomes":[]}"#).unwrap();
        assert!(apply(Some(&dir), &mut ov).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
