//! Fork source policy shared by native builds and release installs.
//! A mod's upstream compatibility pin must never remove our skin changes.
use crate::{backend, native_mods};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, path::Path};

/// Signed fork releases carry content and editable native bundles beside the prebuild.
/// Update managed defaults, preserving player-modified packages and selections.
pub fn install_bundled_mods(version: &Path, home: &Path) -> Result<(), String> {
    let record = home.join(".horizon-bundled-mods.json");
    let read = |p: &Path| -> BTreeMap<String, String> {
        fs::read(p)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    };
    let mut managed = read(&record);
    let previous = read(&version.join("fork-mods/previous-defaults.json"));
    let mut changed = false;
    for (folder, extension) in [("horizon-mods", "hmod"), ("horizon-native-mods", "hnmod")] {
        let source = version.join("fork-mods").join(folder);
        if !source.exists() {
            continue;
        }
        let target = home.join(folder);
        fs::create_dir_all(&target).map_err(|e| e.to_string())?;
        for entry in fs::read_dir(source).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if !entry.file_type().map_err(|e| e.to_string())?.is_file()
                || path.extension().and_then(|e| e.to_str()) != Some(extension)
            {
                return Err("Invalid bundled mod file".into());
            }
            let destination = target.join(entry.file_name());
            let key = format!("{folder}/{}", entry.file_name().to_string_lossy());
            if destination.exists() {
                let digest = format!(
                    "{:x}",
                    Sha256::digest(fs::read(&destination).map_err(|e| e.to_string())?)
                );
                if managed.get(&key) != Some(&digest) && previous.get(&key) != Some(&digest) {
                    continue;
                }
            }
            let temporary = destination.with_extension("installing");
            fs::copy(&path, &temporary).map_err(|e| e.to_string())?;
            // Windows cannot rename over an existing file.
            if destination.exists() {
                fs::remove_file(&destination).map_err(|e| e.to_string())?;
            }
            fs::rename(&temporary, &destination).map_err(|e| e.to_string())?;
            managed.insert(
                key,
                format!(
                    "{:x}",
                    Sha256::digest(fs::read(&path).map_err(|e| e.to_string())?)
                ),
            );
            changed = true;
        }
    }
    if changed {
        let temp = record.with_extension("json.tmp");
        fs::write(
            &temp,
            serde_json::to_vec_pretty(&managed).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        if record.exists() {
            fs::remove_file(&record).map_err(|e| e.to_string())?;
        }
        fs::rename(temp, record).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Last of the three commits implementing this fork's skin behavior.
pub const REQUIRED_COMMIT: &str = "b543e8e4e54d8a5a08c99f06e4a61f9c599abdfc";

pub fn build_revision() -> Result<String, String> {
    let revision = match std::env::var("HORIZON_FORK_REVISION") {
        Ok(revision) => {
            if revision.len() != 40 || !revision.bytes().all(|c| c.is_ascii_hexdigit()) {
                return Err("HORIZON_FORK_REVISION must be a full commit hash".into());
            }
            revision
        }
        Err(_) => native_mods::revision_of("main")?,
    };
    verify_revision(&revision)?;
    Ok(revision)
}

/// Developer workflow: use the normal staging, compiler, profile and target cache,
/// but only type-check. Never install an artifact or change the selected instance.
pub fn preflight() -> bool {
    std::env::var("HORIZON_FORK_PREFLIGHT").as_deref() == Ok("1")
}

pub fn verify_revision(revision: &str) -> Result<(), String> {
    if revision == REQUIRED_COMMIT {
        return Ok(());
    }
    let url = format!(
        "https://api.github.com/repos/{}/compare/{REQUIRED_COMMIT}...{revision}",
        backend::REPO
    );
    let compared = backend::get(&url)?
        .into_json()
        .map_err(|_| "Could not verify fork history")?;
    require_skin_history(&compared)
}

fn require_skin_history(compared: &serde_json::Value) -> Result<(), String> {
    match compared.get("status").and_then(serde_json::Value::as_str) {
        Some("ahead" | "identical") => Ok(()),
        _ => Err("This version does not include your fork's skin changes. Build from eltanschauung/horizon main instead.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_mods_install_on_first_run_and_preserve_player_files() {
        let root = std::env::temp_dir().join(format!("fork-bundles-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let version = root.join("version");
        let home = root.join("home");
        let source = version.join("fork-mods/horizon-mods");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("lunatic.hmod"), b"bundled").unwrap();
        install_bundled_mods(&version, &home).unwrap();
        let installed = home.join("horizon-mods/lunatic.hmod");
        assert_eq!(fs::read(&installed).unwrap(), b"bundled");
        fs::write(source.join("lunatic.hmod"), b"updated").unwrap();
        install_bundled_mods(&version, &home).unwrap();
        assert_eq!(fs::read(&installed).unwrap(), b"updated");

        fs::write(&installed, b"custom").unwrap();
        fs::write(home.join("horizon-mods.txt"), b"mod.lunatic:false\n").unwrap();
        install_bundled_mods(&version, &home).unwrap();
        assert_eq!(fs::read(installed).unwrap(), b"custom");
        assert_eq!(
            fs::read(home.join("horizon-mods.txt")).unwrap(),
            b"mod.lunatic:false\n"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn upstream_ancestors_and_diverged_versions_cannot_replace_the_fork() {
        for status in ["behind", "diverged", "", "unknown"] {
            assert!(require_skin_history(&serde_json::json!({"status":status})).is_err());
        }
        assert!(require_skin_history(&serde_json::json!({})).is_err());
        for status in ["ahead", "identical"] {
            assert!(require_skin_history(&serde_json::json!({"status":status})).is_ok());
        }
    }
}
