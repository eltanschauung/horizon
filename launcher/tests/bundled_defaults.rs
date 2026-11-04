use horizon_launcher::fork::install_bundled_mods;
use sha2::{Digest, Sha256};
use std::fs;
#[test]
fn migrate_old_release_defaults_but_preserve_custom_mods_and_selections() {
    let root = std::env::temp_dir().join(format!("release-migration-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let version = root.join("version");
    let home = root.join("home");
    let bundles = version.join("fork-mods/horizon-mods");
    fs::create_dir_all(&bundles).unwrap();
    fs::create_dir_all(home.join("horizon-mods")).unwrap();
    fs::write(bundles.join("lunatic.hmod"), b"new").unwrap();
    fs::write(home.join("horizon-mods/lunatic.hmod"), b"old").unwrap();
    fs::write(home.join("horizon-mods.txt"), b"mod.lunatic:false\n").unwrap();
    let prior =
        serde_json::json!({"horizon-mods/lunatic.hmod":format!("{:x}", Sha256::digest(b"old"))});
    fs::write(
        version.join("fork-mods/previous-defaults.json"),
        prior.to_string(),
    )
    .unwrap();
    install_bundled_mods(&version, &home).unwrap();
    assert_eq!(
        fs::read(home.join("horizon-mods/lunatic.hmod")).unwrap(),
        b"new"
    );
    fs::write(home.join("horizon-mods/lunatic.hmod"), b"custom").unwrap();
    install_bundled_mods(&version, &home).unwrap();
    assert_eq!(
        fs::read(home.join("horizon-mods/lunatic.hmod")).unwrap(),
        b"custom"
    );
    assert_eq!(
        fs::read(home.join("horizon-mods.txt")).unwrap(),
        b"mod.lunatic:false\n"
    );
    fs::remove_dir_all(root).unwrap();
}
