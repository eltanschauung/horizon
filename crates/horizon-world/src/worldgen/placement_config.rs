//! Optional content-declared integer weight settings. Files stay beneath the game's config root.
use super::DecodeError;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    io::Write,
    path::{Component, Path},
};
fn error(message: impl Into<String>) -> DecodeError {
    DecodeError::Registry(message.into())
}
fn set(table: &mut toml::Table, key: &str, value: toml::Value) -> Result<(), DecodeError> {
    let mut parts = key.split('.').peekable();
    let mut at = table;
    while let Some(part) = parts.next() {
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            return Err(error("invalid configuration key"));
        }
        if parts.peek().is_none() {
            at.insert(part.into(), value);
            break;
        }
        at = at
            .entry(part)
            .or_insert_with(|| toml::Value::Table(toml::Table::new()))
            .as_table_mut()
            .ok_or_else(|| error("configuration key conflict"))?;
    }
    Ok(())
}
pub(super) fn parse_scale(value: &toml::Value) -> Option<f64> {
    value
        .as_float()
        .or_else(|| value.as_integer().map(|n| n as f64))
        .filter(|scale| (1.0..=2.0).contains(scale))
}

fn get<'a>(
    doc: &'a toml::Value,
    file: &str,
    key: &str,
) -> Result<Option<&'a toml::Value>, DecodeError> {
    let mut value = doc;
    for part in key.split('.') {
        let table = value
            .as_table()
            .ok_or_else(|| error(format!("{file}: invalid table in {key}")))?;
        let Some(next) = table.get(part) else {
            return Ok(None);
        };
        value = next;
    }
    Ok(Some(value))
}

pub fn configured(
    declarations: &BTreeMap<String, String>,
    root: Option<&Path>,
) -> Result<BTreeMap<String, String>, DecodeError> {
    let Some(root) = root else {
        return Ok(declarations.clone());
    };
    let mut docs = BTreeMap::<String, Value>::new();
    let mut defaults = BTreeMap::<String, toml::Table>::new();
    for (id, text) in declarations {
        let doc: Value = serde_json::from_str(text)?;
        if let Some(config) = doc.get("configuration") {
            let file = config["file"]
                .as_str()
                .ok_or_else(|| error("weight configuration file"))?;
            let key = config["key"]
                .as_str()
                .ok_or_else(|| error("weight configuration key"))?;
            let ns = id.split(':').next().unwrap_or_default();
            if !file.starts_with(&format!("{ns}/"))
                || !Path::new(file)
                    .components()
                    .all(|c| matches!(c, Component::Normal(_)))
            {
                return Err(error("weight configuration must stay in its namespace"));
            }
            let default = doc["weight"]
                .as_i64()
                .filter(|v| (0..=i32::MAX as i64).contains(v))
                .ok_or_else(|| error("integer default weight"))?;
            let table = defaults.entry(file.into()).or_default();
            set(table, key, toml::Value::Integer(default))?;
            if let Some(scale_key) = config["scale_key"].as_str() {
                set(table, scale_key, toml::Value::Float(1.0))?;
            }
        }
        docs.insert(id.clone(), doc);
    }
    let mut files = BTreeMap::new();
    for (file, table) in defaults {
        let path = root.join(&file);
        if !path.exists() {
            std::fs::create_dir_all(path.parent().unwrap())
                .map_err(|e| error(format!("{}: {e}", path.display())))?;
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut out) => {
                    let text = toml::to_string_pretty(&table).map_err(|e| error(e.to_string()))?;
                    out.write_all(format!("# Biome region weights. Zero disables a region. Restart after editing.\n{text}").as_bytes()).map_err(|e|error(e.to_string()))?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(error(format!("{}: {e}", path.display()))),
            }
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|e| error(format!("{}: {e}", path.display())))?;
        let parsed: toml::Value =
            toml::from_str(&text).map_err(|e| error(format!("{}: {e}", path.display())))?;
        files.insert(file, parsed);
    }
    for doc in docs.values_mut() {
        if let Some(config) = doc.get("configuration") {
            let file = config["file"].as_str().unwrap();
            let key = config["key"].as_str().unwrap();
            let weight = get(&files[file], file, key)?
                .map(|value| {
                    value
                        .as_integer()
                        .filter(|v| (0..=i32::MAX as i64).contains(v))
                        .ok_or_else(|| {
                            error(format!(
                                "{file}: {key} must be an integer from 0 to {}",
                                i32::MAX
                            ))
                        })
                })
                .transpose()?;
            let scale = config["scale_key"]
                .as_str()
                .map(|key| {
                    get(&files[file], file, key)?
                        .map(|value| {
                            parse_scale(value).ok_or_else(|| {
                                error(format!("{file}: {key} must be a number from 1.0 to 2.0"))
                            })
                        })
                        .transpose()
                })
                .transpose()?
                .flatten();
            if let Some(weight) = weight {
                doc["weight"] = Value::from(weight);
            }
            if let Some(scale) = scale {
                doc["region_scale"] = Value::from(scale);
            }
        }
    }
    Ok(docs
        .into_iter()
        .map(|(id, v)| (id, v.to_string()))
        .collect())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_zero_override_and_invalid_values() {
        let root = std::env::temp_dir().join(format!("region-config-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let docs=[("forest:primary".into(),r#"{"weight":10,"configuration":{"file":"forest/generation.toml","key":"overworld.primary"}}"#.into())].into();
        assert_eq!(
            serde_json::from_str::<Value>(
                &configured(&docs, Some(&root)).unwrap()["forest:primary"]
            )
            .unwrap()["weight"],
            10
        );
        let path = root.join("forest/generation.toml");
        std::fs::write(&path, "[overworld]\nprimary=0\n").unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(
                &configured(&docs, Some(&root)).unwrap()["forest:primary"]
            )
            .unwrap()["weight"],
            0
        );
        for invalid in ["-1", "1.5", "2147483648", "true"] {
            std::fs::write(&path, format!("[overworld]\nprimary={invalid}\n")).unwrap();
            assert!(configured(&docs, Some(&root)).is_err());
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
