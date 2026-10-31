//! Fork source policy shared by native builds and release installs.
//! A mod's upstream compatibility pin must never remove our skin changes.
use crate::{backend, native_mods};

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
