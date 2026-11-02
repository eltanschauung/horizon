//! Keep the fork's installed mixins working across upstream's client-library and HUD split.
//! Only the incoming build copy is changed; the installed archives stay untouched.
use super::{Bundle, MANIFEST};
use std::{fs, path::Path};

pub(super) fn prepare(bundle: &Bundle, source: &Path) -> Result<(), String> {
    if !source
        .join("crates/horizon-client-lib/Cargo.toml")
        .is_file()
    {
        return Ok(());
    }
    let path = bundle.dir.join(MANIFEST);
    let text = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let mut manifest: toml::Table = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
    let mut changed = false;
    if let Some(entries) = manifest
        .get_mut("injection")
        .and_then(toml::Value::as_array_mut)
    {
        for entry in entries {
            let Some(table) = entry.as_table_mut() else {
                continue;
            };
            let Some(target) = table
                .get("target")
                .and_then(toml::Value::as_str)
                .map(str::to_owned)
            else {
                continue;
            };
            // These systems are now owned by horizon-client-lib; their Rust APIs are unchanged.
            if [
                "horizon_client::projectiles::",
                "horizon_client::item_render::",
            ]
            .iter()
            .any(|p| target.starts_with(p))
            {
                table.insert(
                    "target".into(),
                    target
                        .replacen("horizon_client::", "horizon_client_lib::", 1)
                        .into(),
                );
                changed = true;
            }
            // The pre-CIT Eating Animation helper constructs the old render-state shape.
            if bundle.id == "eatinganimation"
                && target == "horizon_client::item_render::ItemRenderer::resolve"
            {
                let helper = table
                    .get("helper")
                    .and_then(toml::Value::as_str)
                    .ok_or("Eating Animation helper missing")?;
                let helper = bundle.dir.join(helper);
                let original = fs::read_to_string(&helper).map_err(|e| e.to_string())?;
                if !original.contains("cit_glint:") {
                    let marker = "Break(ItemRenderState {";
                    if !original.contains(marker) {
                        return Err("Unsupported legacy Eating Animation render state".into());
                    }
                    let updated = original.replacen(
                        marker,
                        "Break(ItemRenderState { cit_glint: None, hdr_tier: 0, hdr_light: 0,",
                        1,
                    );
                    fs::write(helper, updated).map_err(|e| e.to_string())?;
                }
            }
            // AppleSkin used the former combined title draw as a once-per-frame HUD hook.
            // Draw after the health/food layer now, rather than on every new HUD layer.
            if bundle.id == "appleskin"
                && target == "horizon_client::game::titles::Game<'_>::draw_titles"
            {
                let helper = table
                    .get("helper")
                    .and_then(toml::Value::as_str)
                    .ok_or("AppleSkin HUD helper missing")?;
                let helper = bundle.dir.join(helper);
                let original = fs::read_to_string(&helper).map_err(|e| e.to_string())?;
                let signature = "fn appleskin_hud(";
                if !original.contains(signature) {
                    return Err("Unsupported legacy AppleSkin HUD helper".into());
                }
                let updated = original
                    .replacen(signature, "fn appleskin_hud(layer: &str, ", 1)
                    .replacen(
                        "{",
                        "{\n    if layer != \"minecraft:player_health\" { return; }",
                        1,
                    );
                fs::write(helper, updated).map_err(|e| e.to_string())?;
                table.insert(
                    "target".into(),
                    "horizon_client::game::native_hooks::Game<'_>::native_gui_layer_post".into(),
                );
                let bindings = table
                    .get_mut("bindings")
                    .and_then(toml::Value::as_array_mut)
                    .ok_or("AppleSkin HUD bindings missing")?;
                let binding: toml::Table = [
                    ("name".into(), "layer".into()),
                    ("ty".into(), "&str".into()),
                    ("value".into(), "layer".into()),
                ]
                .into_iter()
                .collect();
                bindings.insert(0, binding.into());
                changed = true;
            }
        }
    }
    if changed {
        fs::write(
            path,
            toml::to_string_pretty(&manifest).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}
