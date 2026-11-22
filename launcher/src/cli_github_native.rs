use super::*;

pub(super) fn github_native(
    home: &Path,
    java: bool,
    found: &github_mods::Resolved,
    only: Option<&str>,
    all: bool,
) -> Result<(), String> {
    if java {
        return Err("Native mods from GitHub are for Horizon instances.".into());
    }
    println!("{}", found.label());
    println!("Source, to read: {}", found.source_url());
    eprintln!("{}", github_mods::WARNING);
    let mods = github_mods::fetch(found)?;
    let chosen = github_mods::choose(&mods, only, all);
    let done = chosen.and_then(|chosen| {
        chosen
            .into_iter()
            .map(|p| github_mods::install(home, found, p))
            .collect::<Result<Vec<_>, _>>()
    });
    github_mods::discard(found);
    let done = done?;
    for o in done {
        println!(
            "Installed {} {} from GitHub ({}), sha256 {}.",
            o.id,
            o.version,
            o.label(),
            o.sha256
        );
    }
    println!("Build it next: horizon-launcher native-build");
    Ok(())
}
