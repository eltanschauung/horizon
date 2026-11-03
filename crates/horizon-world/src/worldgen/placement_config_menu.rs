//! BOP's currently supported Java region-weight options, using its existing TOML file.
use std::{cell::RefCell, io::Write, path::Path};
const FIELDS: [(&str, &str, &str, i64); 5] = [
    ("overworld", "bop_primary_overworld_region_weight", "Overworld primary", 10),
    ("overworld", "bop_secondary_overworld_region_weight", "Overworld secondary", 8),
    ("overworld", "bop_overworld_rare_region_weight", "Overworld rare", 2),
    ("nether", "bop_nether_region_weight", "Nether common", 13),
    ("nether", "bop_nether_rare_region_weight", "Nether rare", 2),
];
thread_local! { static ERROR: RefCell<Option<String>> = const { RefCell::new(None) }; }
fn read(root: &Path) -> Result<toml::Value, String> {
    let path = root.join("biomesoplenty/generation.toml");
    let mut doc: toml::Value = match std::fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text).map_err(|e| e.to_string())?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => toml::Value::Table(Default::default()),
        Err(e) => return Err(e.to_string()),
    };
    for (section, key, _, default) in FIELDS {
        let table = doc.as_table_mut().ok_or("Expected a TOML table")?.entry(section)
            .or_insert_with(|| toml::Value::Table(Default::default())).as_table_mut().ok_or("Invalid config section")?;
        let value = table.entry(key).or_insert(toml::Value::Integer(default));
        if !value.as_integer().is_some_and(|n| (0..=i32::MAX as i64).contains(&n)) {
            return Err(format!("{section}.{key} must be an integer from 0 to {}", i32::MAX));
        }
    }
    Ok(doc)
}
pub fn buttons(root: &Path) -> Vec<String> {
    let mut rows = vec!["Restart game after edits; new chunks only".into(), "Zero disables a region".into(), "Reset rarity weights to defaults".into()];
    match read(root) {
        Ok(doc) => for (section, key, label, _) in FIELDS {
            let value = doc[section][key].as_integer().unwrap();
            rows.push(format!("{label}: {value} (-)"));
            rows.push(format!("{label}: {value} (+)"));
        },
        Err(e) => rows.push(format!("Cannot read configuration: {e}")),
    }
    ERROR.with(|e| if let Some(e) = e.borrow().as_ref() { rows.push(format!("Save failed: {e}")); });
    rows
}
fn change(root: &Path, button: usize) -> Result<(), String> {
    if button < 2 || button >= 3 + FIELDS.len() * 2 { return Ok(()); }
    let mut doc = read(root)?; // Refuse malformed input rather than overwriting it.
    if button == 2 {
        for (section, key, _, value) in FIELDS { doc[section][key] = toml::Value::Integer(value); }
    } else {
        let (section, key, _, _) = FIELDS[(button - 3) / 2];
        let delta = if (button - 3) % 2 == 0 { -1 } else { 1 };
        doc[section][key] = toml::Value::Integer((doc[section][key].as_integer().unwrap() + delta).clamp(0, i32::MAX as i64));
    }
    let path = root.join("biomesoplenty/generation.toml");
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let temp = path.with_extension(format!("toml.menu-{}", std::process::id()));
    let result = (|| -> Result<(), String> {
        let text = toml::to_string_pretty(&doc).map_err(|e| e.to_string())?;
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temp).map_err(|e| e.to_string())?;
        file.write_all(text.as_bytes()).and_then(|_| file.sync_all()).map_err(|e| e.to_string())?;
        std::fs::rename(&temp, &path).map_err(|e| e.to_string())
    })();
    if result.is_err() { let _ = std::fs::remove_file(temp); }
    result
}
pub fn click(root: &Path, button: usize) { ERROR.with(|e| *e.borrow_mut() = change(root, button).err()); }
