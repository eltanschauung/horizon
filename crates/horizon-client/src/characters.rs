//! Character appearances supplied by content mods, and the instance's choice.
use crate::hud::HudAtlas;
use serde::{Deserialize, Serialize};
use std::{fs, io, io::Write, path::Path};

pub mod sync;
pub mod reservations;

pub const FALLBACK_SKIN: &str = "entity/player/wide/steve";
const CHOICE_FILE: &str = "character.json";

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Model {
    #[default]
    Wide,
    Slim,
}

/// A plain stack or a random draw from a pool, resolved by the server once.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StartingItem {
    #[serde(default)]
    pub item: String,
    #[serde(default)]
    pub choices: Vec<String>,
    #[serde(default = "one_count")]
    pub count: u32,
    #[serde(default)]
    pub min_count: Option<u32>,
    #[serde(default)]
    pub max_count: Option<u32>,
    #[serde(default = "one_count")]
    pub rolls: u32,
    #[serde(default)]
    pub enchantments: std::collections::BTreeMap<String, u32>,
}
fn one_count() -> u32 {
    1
}
fn default_size() -> f32 {
    1.0
}
fn character_size<'de, D: serde::Deserializer<'de>>(d: D) -> Result<f32, D::Error> {
    let size = f32::deserialize(d)?;
    Ok(if size.is_finite() {
        size.clamp(0.7, 1.2)
    } else {
        1.0
    })
}

fn default_health() -> f32 { 20.0 }
fn character_health<'de, D: serde::Deserializer<'de>>(d: D) -> Result<f32, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Health { Number(f32), Text(String) }
    let value = match Health::deserialize(d)? {
        Health::Number(value) => value,
        Health::Text(value) => value.parse().unwrap_or(20.0),
    };
    Ok(if value.is_finite() && value > 0.0 { value.clamp(1.0, 1024.0) } else { 20.0 })
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(from = "CharacterDefinition")]
pub struct Character {
    pub id: String,
    pub name: String,
    pub texture: String,
    /// Comma-separated race tags; abilities match complete trimmed tags.
    #[serde(default)]
    pub race: String,
    #[serde(default = "default_size", deserialize_with = "character_size")]
    pub size: f32,
    pub health: f32,
    #[serde(default)]
    pub starting_items: Vec<StartingItem>,
    #[serde(default)]
    pub model: Model,
    #[serde(default)]
    pub character_chat: bool,
    #[serde(default)]
    pub color: Option<String>,
}

// Resolve optional content fields before validation, rendering and synchronization.
#[derive(Deserialize)]
struct CharacterDefinition {
    id: String,
    name: String,
    #[serde(default)]
    texture: Option<String>,
    #[serde(default)]
    race: String,
    #[serde(default = "default_size", deserialize_with = "character_size")]
    size: f32,
    #[serde(default = "default_health", deserialize_with = "character_health")]
    health: f32,
    #[serde(default)]
    starting_items: Vec<StartingItem>,
    #[serde(default = "default_character_model")]
    model: Model,
    #[serde(default = "default_character_chat")]
    character_chat: bool,
    #[serde(default)]
    color: Option<String>,
}
fn default_character_model() -> Model { Model::Slim }
fn default_character_chat() -> bool { true }
impl From<CharacterDefinition> for Character {
    fn from(value: CharacterDefinition) -> Self {
        let texture = value.texture.unwrap_or_else(|| {
            value.id.split_once(':').map(|(namespace, name)|
                format!("{namespace}:entity/character/{name}")
            ).unwrap_or_default()
        });
        Self { id: value.id, name: value.name, texture, race: value.race,
            size: value.size, health: value.health, starting_items: value.starting_items, model: value.model,
            character_chat: value.character_chat, color: value.color }
    }
}

impl Character {
    pub fn has_race(&self, race: &str) -> bool {
        !race.is_empty()
            && self
                .race
                .split(',')
                .any(|tag| tag.trim().eq_ignore_ascii_case(race))
    }

    pub fn display_name(&self) -> crate::text::Text {
        crate::scoreboard::comp_text(&serde_json::json!({"text": self.name, "color": self.color}))
    }

    /// Character-labelled chat is the default; definitions may opt out without changing account identity.
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

/// Immutable catalog for one loaded resource generation. Shared by cloned pack bundles.
/// Networking uses indices and this precomputed digest, never filesystem reads.
pub struct Catalog {
    characters: Vec<Character>,
    by_id: std::collections::HashMap<String, u16>,
    pub digest: [u8; 32],
    pub digest_hex: String,
}
impl Catalog {
    pub fn load(root: &Path) -> Self {
        Self::new(roster(root))
    }
    pub fn new(mut characters: Vec<Character>) -> Self {
        use sha2::{Digest, Sha256};
        // u16::MAX is the wire's removal marker.
        characters.truncate(usize::from(u16::MAX));
        let digest: [u8; 32] =
            Sha256::digest(serde_json::to_vec(&characters).expect("character catalog serializes"))
                .into();
        let by_id = characters
            .iter()
            .enumerate()
            .map(|(i, c)| (c.id.clone(), i as u16))
            .collect();
        let digest_hex = digest.iter().map(|b| format!("{b:02x}")).collect();
        Self {
            characters,
            by_id,
            digest,
            digest_hex,
        }
    }
    pub fn get(&self, index: u16) -> Option<&Character> {
        self.characters.get(usize::from(index))
    }
    pub fn index(&self, id: &str) -> Option<u16> {
        self.by_id.get(id).copied()
    }
    pub fn resolve_legacy(&self, bytes: &[u8]) -> Option<u16> {
        let request: SelectionRequest = serde_json::from_slice(bytes).ok()?;
        (request.catalog == self.digest_hex)
            .then(|| self.index(&request.character))
            .flatten()
    }
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
    fn health_accepts_numbers_strings_and_safe_defaults() {
        for (value, expected) in [(serde_json::json!("16"), 16.0), (serde_json::json!(16), 16.0), (serde_json::json!("invalid"), 20.0), (serde_json::json!(-1), 20.0)] {
            let c: Character = serde_json::from_value(serde_json::json!({"id":"lunatic:test", "name":"Test", "health":value})).unwrap();
            assert_eq!(c.health, expected);
        }
    }
    #[test]
    fn races_use_whole_tags_and_sizes_have_safe_defaults_and_bounds() {
        let base = serde_json::json!({"id":"lunatic:cirno", "name":"Cirno", "texture":"lunatic:entity/character/cirno"});
        let c: Character = serde_json::from_value(base.clone()).unwrap();
        assert_eq!(c.size, 1.0);
        assert!(!c.has_race("fairy"));
        for (size, expected) in [(0.1, 0.7), (1.0, 1.0), (9.0, 1.2)] {
            let mut data = base.clone();
            data["size"] = serde_json::json!(size);
            data["race"] = serde_json::json!("youkai, Fairy ,ice_fairy,");
            let c: Character = serde_json::from_value(data).unwrap();
            assert_eq!(c.size, expected);
            assert!(c.has_race("fairy"));
            assert!(!c.has_race("air"));
            assert!(!c.has_race("flower_fairy"));
        }
    }
    #[test]
    fn character_defaults_and_chat_overrides_preserve_plain_text() {
        let mut c: Character = serde_json::from_value(serde_json::json!({
            "id":"lunatic:reimu", "name":"Hakurei Reimu"
        }))
        .unwrap();
        assert_eq!(c.texture, "lunatic:entity/character/reimu");
        assert_eq!(c.model, Model::Slim);
        assert!(c.character_chat);
        c.character_chat = false;
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
