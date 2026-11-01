#!/usr/bin/env python3
"""Build a character-aware ChatHeads bundle: INPUT.hnmod OUTPUT.hnmod."""
import json
from pathlib import Path
import sys
import zipfile


def replace(text, old, new):
    if text.count(old) != 1:
        raise ValueError('ChatHeads source changed; review compatibility patch')
    return text.replace(old, new)


def patch(files):
    root = 'mixins/chatheads/'
    runtime = files[root + 'runtime.rs'].decode()
    runtime = replace(runtime, "    let name = line.strip_prefix('<')?.split_once('>')?.0;", '''    if !line.starts_with('<') && let Some((name, _)) = line.split_once(" : ") {
        return (!name.is_empty()).then_some(name);
    }
    let name = line.strip_prefix('<')?.split_once('>')?.0;''')
    runtime = replace(runtime, 'pub fn sender(line: &str) -> Option<&str> {', 'pub fn sender(line: &str) -> Option<&str> {\n    if line.starts_with("@skin:") { return Some(line); }')
    files[root + 'runtime.rs'] = runtime.encode()
    shift = files[root + 'shift.rs'].decode()
    shift += '''
// Character labels and skin locations already encountered in this session.
// Preserve earlier characters' heads after the player changes their selection.
static CHARACTERS: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
pub fn register_character(name: &str, skin: &str) {
    let mut characters = CHARACTERS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(entry) = characters.iter_mut().find(|(n, _)| n == name) {
        entry.1 = skin.to_owned();
    } else {
        characters.push((name.to_owned(), skin.to_owned()));
    }
}
pub fn character_skin(name: &str) -> Option<String> {
    CHARACTERS.lock().unwrap_or_else(|e| e.into_inner())
        .iter().find(|(n, _)| n == name).map(|(_, skin)| skin.clone())
}
fn character_line(line: &str) -> bool {
    line.split_once(" : ").is_some_and(|(name, _)| character_skin(name).is_some())
}
'''
    shift = replace(shift, "if plain.starts_with('<') {", "if plain.starts_with('<') || character_line(&plain) {")
    shift = replace(shift, "if plain.starts_with('<') || character_line(&plain) {", """if let Some(skin) = text.component().get("lunatic_skin").and_then(|v| v.as_str()) {
            heads.push((*x, *y, (*color >> 24) as u8, format!("@skin:{skin}")));
        } else if plain.starts_with('<') || character_line(&plain) {""")
    files[root + 'shift.rs'] = shift.encode()
    glue = files[root + 'glue.rs'].decode()
    glue = replace(glue, 'pub(crate) fn mark(game: &Game<\'_>) {', '''pub(crate) fn mark(game: &Game<'_>) {
    if let Some(character) = game.character.as_ref().filter(|c| c.character_chat) {
        layout::register_character(&character.name, character.skin(&game.gui.atlas));
    }''')
    glue = replace(glue, '    let skin = match atlas.object_skins.get(&key) {', '''    let character_skin = name.strip_prefix("@skin:").map(str::to_owned).or_else(|| layout::character_skin(name));
    let skin = match character_skin.as_ref().or_else(|| atlas.object_skins.get(&key)) {''')
    files[root + 'glue.rs'] = glue.encode()
    manifest = files[root + 'horizon-mixin.toml'].decode()
    files[root + 'horizon-mixin.toml'] = replace(manifest, 'version = "0.1.0"', 'version = "0.1.0+character.2"').encode()
    metadata = json.loads(files['horizon-native.json'])
    metadata['version'] = '0.1.0+character.2'
    files['horizon-native.json'] = (json.dumps(metadata, indent=2) + '\n').encode()
    return files


def main():
    source, target = map(Path, sys.argv[1:])
    with zipfile.ZipFile(source) as archive:
        files = patch({n: archive.read(n) for n in archive.namelist()})
    target.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(target, 'w', zipfile.ZIP_DEFLATED) as archive:
        for name, data in files.items():
            archive.writestr(name, data)
    print(target)


if __name__ == '__main__':
    main()
