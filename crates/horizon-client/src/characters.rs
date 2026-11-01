//! Character appearances supplied by content mods, and the instance's choice.
use crate::hud::HudAtlas;
use serde::{Deserialize, Serialize};
use std::{fs, io, io::Write, path::Path};

pub const FALLBACK_SKIN: &str = "entity/player/wide/steve";
const CHOICE_FILE: &str = "character.json";

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Model {
    #[default]
    Wide,
    Slim,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Character {
    pub id: String,
    pub name: String,
    pub texture: String,
    #[serde(default)]
    pub model: Model,
    #[serde(default)]
    pub character_chat: bool,
    #[serde(default)]
    pub color: Option<String>,
}

impl Character {
    pub fn display_name(&self) -> crate::text::Text {
        crate::scoreboard::comp_text(&serde_json::json!({"text": self.name, "color": self.color}))
    }

    /// Content mods opt into character-labelled chat without changing account identity.
    pub fn chat_line(&self, content: &serde_json::Value) -> Option<crate::text::Text> {
        self.character_chat.then(|| {
            let mut line = crate::text::Text::empty();
            line.push(self.display_name());
            line.push(crate::text::Text::literal(" : "));
            line.push(crate::scoreboard::comp_text(content));
            line
        })
    }

    /// Steve's failure fallback retains its wide arms.
    pub fn is_slim(&self, atlas: &HudAtlas) -> bool {
        self.model == Model::Slim && atlas.sprites.contains_key(&self.texture)
    }

    /// A broken appearance uses Steve without adding him to the roster.
    pub fn skin(&self, atlas: &HudAtlas) -> &str {
        if atlas.sprites.contains_key(&self.texture) {
            &self.texture
        } else {
            FALLBACK_SKIN
        }
    }
}

fn identifier_part(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Only enabled mods' namespaces are searched; no machine-specific paths.
pub fn roster(root: &Path) -> Vec<Character> {
    #[derive(Deserialize)]
    struct Catalog {
        characters: Vec<Character>,
    }
    let mut characters = Vec::new();
    let mut namespaces = crate::mod_content::namespaces(root);
    namespaces.sort();
    for namespace in namespaces {
        let path = root.join(format!("assets/{namespace}/horizon/characters.json"));
        let Some(catalog) = fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Catalog>(&bytes).ok())
        else {
            continue;
        };
        for character in catalog.characters {
            let prefix = format!("{namespace}:");
            let valid_id = character
                .id
                .strip_prefix(&prefix)
                .is_some_and(identifier_part);
            let valid_texture = character
                .texture
                .strip_prefix(&format!("{namespace}:entity/character/"))
                .is_some_and(identifier_part);
            if valid_id
                && valid_texture
                && !character.name.trim().is_empty()
                && !characters.iter().any(|c: &Character| c.id == character.id)
            {
                characters.push(character);
            }
        }
    }
    characters
}

#[derive(Deserialize, Serialize)]
struct Choice {
    character: String,
}

pub fn selected(root: &Path, instance: &Path) -> Option<Character> {
    let choice: Choice =
        serde_json::from_slice(&fs::read(instance.join(CHOICE_FILE)).ok()?).ok()?;
    roster(root).into_iter().find(|c| c.id == choice.character)
}

/// Save only this preference, atomically; never rewrite options or account files.
pub fn save(instance: &Path, character: &Character) -> io::Result<()> {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = instance.join(format!(".character-{}-{nonce}.tmp", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let result = (|| {
        let bytes = serde_json::to_vec_pretty(&Choice {
            character: character.id.clone(),
        })?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, instance.join(CHOICE_FILE))
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn character_chat_is_opt_in_and_treats_names_and_messages_as_text() {
        let mut c: Character = serde_json::from_value(serde_json::json!({
            "id":"lunatic:reimu", "name":"Hakurei Reimu", "texture":"lunatic:entity/character/reimu"
        }))
        .unwrap();
        assert!(c.chat_line(&serde_json::json!("hello")).is_none());
        c.character_chat = true;
        c.color = Some("red".into());
        assert_eq!(c.display_name().plain(), "Hakurei Reimu");
        assert_eq!(c.display_name().spans[0].style.color, Some(0xFF5555));
        assert_eq!(
            c.chat_line(&serde_json::json!({"text":"hello", "color":"red"}))
                .unwrap()
                .plain(),
            "Hakurei Reimu : hello"
        );
    }
}

/// A selection carries no claimed player identity or client-defined appearance.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SelectionRequest {
    pub character: String,
    pub catalog: String,
}
pub fn catalog_digest(roster: &[Character]) -> String {
    use sha2::{Digest, Sha256};
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(roster).unwrap_or_default())
    )
}
pub fn resolve_selection(bytes: &[u8], roster: &[Character]) -> Option<Character> {
    let request: SelectionRequest = serde_json::from_slice(bytes).ok()?;
    if request.catalog != catalog_digest(roster) {
        return None;
    }
    roster.iter().find(|c| c.id == request.character).cloned()
}

#[cfg(test)]
mod network_tests {
    use super::*;
    fn catalog() -> Vec<Character> {
        serde_json::from_value(serde_json::json!([
            {"id":"lunatic:reimu","name":"Reimu","texture":"lunatic:entity/character/reimu","character_chat":true,"color":"red"},
            {"id":"lunatic:marisa","name":"Marisa","texture":"lunatic:entity/character/marisa","character_chat":true,"color":"yellow"}
        ])).unwrap()
    }
    #[test]
    fn server_validates_ids_and_catalog_instead_of_trusting_client_metadata() {
        let roster = catalog();
        let request = |id: &str| {
            serde_json::to_vec(&SelectionRequest {
                character: id.into(),
                catalog: catalog_digest(&roster),
            })
            .unwrap()
        };
        assert_eq!(
            resolve_selection(&request("lunatic:reimu"), &roster)
                .unwrap()
                .color
                .as_deref(),
            Some("red")
        );
        assert!(resolve_selection(&request("lunatic:unknown"), &roster).is_none());
        let mut tampered = roster.clone();
        tampered[0].color = Some("blue".into());
        assert!(resolve_selection(&request("lunatic:reimu"), &tampered).is_none());
        let mut impersonation: serde_json::Value =
            serde_json::from_slice(&request("lunatic:reimu")).unwrap();
        impersonation["uuid"] = "another-player".into();
        assert!(resolve_selection(&serde_json::to_vec(&impersonation).unwrap(), &roster).is_none());
        assert!(resolve_selection(b"not json", &roster).is_none());
    }
}
