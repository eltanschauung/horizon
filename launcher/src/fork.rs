//! Fork source policy shared by native builds and release installs.
//! A mod's upstream compatibility pin must never remove our skin changes.
use crate::{backend, native_mods};
use std::{fs, path::Path};

/// Signed fork releases carry content and editable native bundles beside the prebuild.
/// Install missing defaults without replacing a player's existing mods or choices.
pub fn install_bundled_mods(version: &Path, home: &Path) -> Result<(), String> {
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
            if destination.exists() {
                continue;
            }
            let temporary = destination.with_extension("installing");
            fs::copy(&path, &temporary).map_err(|e| e.to_string())?;
            fs::rename(&temporary, &destination).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Last of the three commits implementing this fork's skin behavior.
pub const REQUIRED_COMMIT: &str = "b543e8e4e54d8a5a08c99f06e4a61f9c599abdfc";

pub fn build_revision() -> Result<String, String> {
    let revision = native_mods::revision_of("main")?;
    verify_revision(&revision)?;
    Ok(revision)
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
