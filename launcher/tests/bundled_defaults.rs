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

#[test]
fn release_tags_recognize_their_bundled_native_build() {
    let root = std::env::temp_dir().join(format!("release-native-{}", std::process::id()));
    let path = root.join("fork-versions/v0.3.2");
    fs::create_dir_all(&path).unwrap();
    let metadata = serde_json::json!({"base":"v0.3.2","horizon":"8cf2519dd64b3d12af9eecb866390f0db4907b3f","target":"client","mods":[{"id":"lunatic","version":"0.7.1"}]});
    fs::write(path.join("native-mods.json"), metadata.to_string()).unwrap();
    std::env::set_var("HORIZON_LAUNCHER_HOME", &root);
    let mods = horizon_launcher::native_status::built("v0.3.2").unwrap();
    assert_eq!(mods[0].0, "lunatic");
    assert!(horizon_launcher::native_status::Build::read("v0.3.2").is_some());
    std::env::remove_var("HORIZON_LAUNCHER_HOME");
    fs::remove_dir_all(root).unwrap();
}
