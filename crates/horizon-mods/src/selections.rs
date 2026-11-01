//! Shared first-start selections for the launcher and game. No packages or code are downloaded.
use std::{fs, io::ErrorKind, path::Path};

pub const DEFAULTS: &str = include_str!("default-selections.txt");

/// Saved choices, including an empty file, override all fork defaults.
/// Unreadable files never cause a mod to become enabled unexpectedly.
pub fn load(home: &Path) -> String {
    match fs::read_to_string(home.join("horizon-mods.txt")) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => DEFAULTS.to_owned(),
        Err(_) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_selections_and_read_errors_do_not_fall_back_to_defaults() {
        let home =
            std::env::temp_dir().join(format!("horizon-selection-load-{}", std::process::id()));
        fs::create_dir_all(&home).unwrap();
        assert!(load(&home).contains("mod.minimap:true"));
        let path = home.join("horizon-mods.txt");
        fs::write(&path, "mod.zoom:false\nmod.other:true\n").unwrap();
        assert_eq!(load(&home), "mod.zoom:false\nmod.other:true\n");
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert_eq!(load(&home), "");
        fs::remove_dir_all(home).unwrap();
    }
}
