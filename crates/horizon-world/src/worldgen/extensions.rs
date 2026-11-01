//! Namespaced additions to a host generator, without replacing its density/router.
use super::{
    DecodeError,
    biome_source::ResolvedBiomeSource,
    material_condition::{MaterialRule, MaterialRuleLibrary},
};
use crate::registry::{BlockRegistry, BlockStateId};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
fn protect(rule: &MaterialRule, bedrock: Option<BlockStateId>) -> Option<MaterialRule> {
    match rule {
        MaterialRule::Block(state) if Some(*state) == bedrock => Some(rule.clone()),
        MaterialRule::OreVein { .. } => Some(rule.clone()),
        MaterialRule::Reference(r) => protect(r, bedrock),
        MaterialRule::Condition { if_true, then_run } => {
            protect(then_run, bedrock).map(|r| MaterialRule::Condition {
                if_true: if_true.clone(),
                then_run: Box::new(r),
            })
        }
        MaterialRule::Sequence(rules) => {
            let out: Vec<_> = rules.iter().filter_map(|r| protect(r, bedrock)).collect();
            if out.is_empty() {
                None
            } else {
                Some(MaterialRule::Sequence(out))
            }
        }
        _ => None,
    }
}
pub fn compose(
    host: MaterialRule,
    mut source: ResolvedBiomeSource,
    settings: &str,
    extensions: &BTreeMap<String, String>,
    biomes: &BTreeSet<String>,
    registry: &BlockRegistry,
    library: &mut MaterialRuleLibrary,
) -> Result<(MaterialRule, ResolvedBiomeSource), DecodeError> {
    let mut rules = Vec::new();
    let mut replacements = BTreeMap::new();
    for (id, text) in extensions {
        let value: Value = serde_json::from_str(text)?;
        if value["settings"].as_str() != Some(settings) {
            continue;
        }
        if let Some(rule) = value["surface_rule"].as_str() {
            rules.push(library.rule(rule)?);
        }
        if let Some(entries) = value["biome_replacements"].as_object() {
            for (from, choices) in entries {
                let mut out = Vec::new();
                for choice in choices
                    .as_array()
                    .ok_or(DecodeError::Field("biome replacement choices"))?
                {
                    let biome = choice["biome"]
                        .as_str()
                        .ok_or(DecodeError::Field("replacement biome"))?;
                    let weight = choice["weight"]
                        .as_u64()
                        .filter(|w| *w > 0 && *w <= 1000000)
                        .ok_or(DecodeError::Field("replacement weight"))?;
                    if !biomes.contains(biome) {
                        return Err(DecodeError::Registry(format!(
                            "extension {id}: unknown biome {biome}"
                        )));
                    }
                    out.push((biome.to_owned(), weight as u32));
                }
                if out.is_empty() || replacements.insert(from.clone(), out).is_some() {
                    return Err(DecodeError::Field("duplicate/empty biome replacement"));
                }
            }
        }
    }
    if !replacements.is_empty() {
        source = ResolvedBiomeSource::Replaced(Box::new(source), replacements);
    }
    if rules.is_empty() {
        return Ok((host, source));
    }
    if let Some(protected) = protect(&host, registry.default_state("minecraft:bedrock")) {
        rules.insert(0, protected);
    }
    rules.push(host);
    Ok((MaterialRule::Sequence(rules), source))
}
