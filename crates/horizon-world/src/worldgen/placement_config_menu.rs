//! In-game controls for biome placement TOML files.
use std::{cell::RefCell, io::Write, path::Path};
const BOP_FIELDS: [(&str, &str, &str, i64); 6] = [
    (
        "overworld",
        "bop_primary_overworld_region_weight",
        "Overworld primary",
        10,
    ),
    (
        "overworld",
        "bop_secondary_overworld_region_weight",
        "Overworld secondary",
        8,
    ),
    (
        "overworld",
        "bop_overworld_rare_region_weight",
        "Overworld rare",
        2,
    ),
    (
        "overworld",
        "bop_overworld_extra_rare_region_weight",
        "Overworld extra rare",
        1,
    ),
    ("nether", "bop_nether_region_weight", "Nether common", 13),
    ("nether", "bop_nether_rare_region_weight", "Nether rare", 2),
];
const GENSOKYO_FIELDS: [(&str, &str, &str, i64); 2] = [
    ("overworld", "region_weight", "Overworld weight", 4),
    ("overworld", "biome_scale", "Biome Scale", 10),
];

#[derive(Clone, Copy)]
pub enum PlacementMenu {
    BiomesOPlenty,
    GensokyoLegacy,
}

impl PlacementMenu {
    pub fn for_mod(id: &str) -> Option<Self> {
        match id {
            "biomesoplenty" => Some(Self::BiomesOPlenty),
            "horizon_gensokyo_legacy" => Some(Self::GensokyoLegacy),
            _ => None,
        }
    }

    fn fields(self) -> &'static [(&'static str, &'static str, &'static str, i64)] {
        match self {
            Self::BiomesOPlenty => &BOP_FIELDS,
            Self::GensokyoLegacy => &GENSOKYO_FIELDS,
        }
    }

    fn file(self) -> &'static str {
        match self {
            Self::BiomesOPlenty => "biomesoplenty/generation.toml",
            Self::GensokyoLegacy => "horizon_gensokyo_legacy/generation.toml",
        }
    }
}

fn default_value(key: &str, value: i64) -> toml::Value {
    if key == "biome_scale" {
        toml::Value::Float(value as f64 / 10.0)
    } else {
        toml::Value::Integer(value)
    }
}

thread_local! { static ERROR: RefCell<Option<String>> = const { RefCell::new(None) }; }
fn read(root: &Path, menu: PlacementMenu) -> Result<toml::Value, String> {
    let path = root.join(menu.file());
    let mut doc: toml::Value = match std::fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text).map_err(|e| e.to_string())?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            toml::Value::Table(Default::default())
        }
        Err(e) => return Err(e.to_string()),
    };
    for (section, key, _, default) in menu.fields().iter().copied() {
        let table = doc
            .as_table_mut()
            .ok_or("Expected a TOML table")?
            .entry(section)
            .or_insert_with(|| toml::Value::Table(Default::default()))
            .as_table_mut()
            .ok_or("Invalid config section")?;
        let value = table.entry(key).or_insert(default_value(key, default));
        if key == "biome_scale" {
            if super::placement_config::parse_scale(value).is_none() {
                return Err(format!("{section}.{key} must be a number from 1.0 to 2.0"));
            }
            continue;
        }
        if !value
            .as_integer()
            .is_some_and(|n| (0..=i32::MAX as i64).contains(&n))
        {
            return Err(format!(
                "{section}.{key} must be an integer from 0 to {}",
                i32::MAX
            ));
        }
    }
    Ok(doc)
}
pub fn buttons(root: &Path, menu: PlacementMenu) -> Vec<String> {
    let (hint, reset) = match menu {
        PlacementMenu::BiomesOPlenty => (
            "Zero disables; extra rare uses quarter-weight units",
            "Reset rarity weights to defaults",
        ),
        PlacementMenu::GensokyoLegacy => (
            "Weight 0 disables Gensokyo biome generation",
            "Reset generation settings to defaults",
        ),
    };
    let mut rows = vec![
        "Restart game after edits; new chunks only".into(),
        hint.into(),
        reset.into(),
    ];
    match read(root, menu) {
        Ok(doc) => {
            for (section, key, label, _) in menu.fields().iter().copied() {
                let value = if key == "biome_scale" {
                    let scale = super::placement_config::parse_scale(&doc[section][key]).unwrap();
                    let formatted = format!("{scale:.2}");
                    if formatted.ends_with('0') {
                        formatted[..formatted.len() - 1].to_owned()
                    } else {
                        formatted
                    }
                } else {
                    doc[section][key].as_integer().unwrap().to_string()
                };
                rows.push(format!("{label}: {value} (-)"));
                rows.push(format!("{label}: {value} (+)"));
            }
        }
        Err(e) => rows.push(format!("Cannot read configuration: {e}")),
    }
    ERROR.with(|e| {
        if let Some(e) = e.borrow().as_ref() {
            rows.push(format!("Save failed: {e}"));
        }
    });
    rows
}
fn change(root: &Path, menu: PlacementMenu, button: usize) -> Result<(), String> {
    if button < 2 || button >= 3 + menu.fields().len() * 2 {
        return Ok(());
    }
    let mut doc = read(root, menu)?; // Refuse malformed input rather than overwriting it.
    if button == 2 {
        for (section, key, _, value) in menu.fields().iter().copied() {
            doc[section][key] = default_value(key, value);
        }
    } else {
        let (section, key, _, _) = menu.fields()[(button - 3) / 2];
        let delta = if (button - 3) % 2 == 0 { -1 } else { 1 };
        let value = &mut doc[section][key];
        *value = if key == "biome_scale" {
            let scale = super::placement_config::parse_scale(value).unwrap();
            let next = ((scale + delta as f64 / 10.0) * 100.0).round() / 100.0;
            toml::Value::Float(next.clamp(1.0, 2.0))
        } else {
            toml::Value::Integer((value.as_integer().unwrap() + delta).clamp(0, i32::MAX as i64))
        };
    }
    let path = root.join(menu.file());
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let temp = path.with_extension(format!("toml.menu-{}", std::process::id()));
    let result = (|| -> Result<(), String> {
        let text = toml::to_string_pretty(&doc).map_err(|e| e.to_string())?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        file.write_all(text.as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        std::fs::rename(&temp, &path).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    result
}
pub fn click(root: &Path, menu: PlacementMenu, button: usize) {
    ERROR.with(|e| *e.borrow_mut() = change(root, menu, button).err());
}
