//! The fork's server-pinned client channel. Updates use immutable, hash-checked
//! bundles published only after the corresponding server deployment is healthy.
//! No moving source branch, account credentials or player data enter the channel.
use crate::{backend, browser, instances, native_mods};
use horizon_platform::mod_package::manifest::Package as ModPackage;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

pub const CHANNEL: &str =
    "https://raw.githubusercontent.com/eltanschauung/horizon/server-sync/manifest.json";
const RECORD: &str = ".horizon-server-sync/manifest.json";
const LIMIT: u64 = 1024 * 1024;
const SCHEMA: u32 = 2;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Native,
    Content,
    Runtime,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Package {
    pub id: String,
    pub version: String,
    pub kind: Kind,
    pub sha256: String,
    pub size: u64,
    pub url: String,
    pub required: bool,
    #[serde(default)]
    pub platform: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: u32,
    pub generation: String,
    pub game_revision: String,
    pub base: String,
    pub packages: Vec<Package>,
}
#[derive(Clone, Debug)]
pub enum Outcome {
    Ready(String),
    Prepare(String),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Downloading,
    Building,
}
impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Downloading => "Downloading Updates...",
            Self::Building => "Building...",
        }
    }
}
fn hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| b.is_ascii_hexdigit())
}
fn id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}
fn hash(p: &Path) -> Result<String, String> {
    let mut f = fs::File::open(p).map_err(|e| e.to_string())?;
    let mut h = Sha256::new();
    let mut bytes = [0u8; 65536];
    loop {
        let n = f.read(&mut bytes).map_err(err)?;
        if n == 0 {
            break;
        }
        h.update(&bytes[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
impl Manifest {
    pub fn validate(&self) -> Result<(), String> {
        self.validate_schema(SCHEMA)
    }

    fn validate_schema(&self, schema: u32) -> Result<(), String> {
        if self.schema != schema
            || !hex(&self.generation, 64)
            || !hex(&self.game_revision, 40)
            || self.base.is_empty()
            || self.base.len() > 100
            || !self
                .base
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            || self.packages.is_empty()
            || self.packages.len() > 256
        {
            return Err("Invalid server update manifest".into());
        }
        let mut seen = BTreeSet::new();
        let mut total = 0u64;
        for p in &self.packages {
            if !id(&p.id)
                || p.version.is_empty()
                || p.version.len() > 128
                || !hex(&p.sha256, 64)
                || p.size == 0
                || p.size > 512 * 1024 * 1024
                || !seen.insert((
                    if schema == SCHEMA && p.kind != Kind::Runtime {
                        "mod".to_owned()
                    } else {
                        format!("{:?}", p.kind)
                    },
                    p.id.as_str(),
                ))
                || !p.url.starts_with(&format!(
                    "https://github.com/{}/releases/download/",
                    backend::REPO
                ))
                || p.url.contains(['?', '#'])
                || p.url.contains("/../")
                || p.platform.as_deref().is_some_and(|s| s != "windows-x86_64")
            {
                return Err(format!("Invalid server update package: {}", p.id));
            }
            if p.kind == Kind::Runtime
                && (!matches!(p.id.as_str(), "libopus" | "opus_license")
                    || p.platform.as_deref() != Some("windows-x86_64"))
            {
                return Err("Unknown server runtime file".into());
            }
            total = total.checked_add(p.size).ok_or("Update size overflow")?;
        }
        if total > 4 * 1024 * 1024 * 1024 {
            return Err("Server update exceeds 4 GB".into());
        }
        Ok(())
    }
}
impl Package {
    fn applies(&self) -> bool {
        self.platform.is_none()
            || (cfg!(all(windows, target_arch = "x86_64"))
                && self.platform.as_deref() == Some("windows-x86_64"))
    }
    fn relative(&self) -> PathBuf {
        match self.kind {
            Kind::Native => PathBuf::from(format!("horizon-native-mods/{}.hmod", self.id)),
            Kind::Content => PathBuf::from(format!("horizon-mods/{}.hmod", self.id)),
            Kind::Runtime => PathBuf::from(match self.id.as_str() {
                "libopus" => "libopus-0.dll",
                _ => "OPUS-LICENSE.txt",
            }),
        }
    }
}
static POLICY: OnceLock<Mutex<BTreeMap<PathBuf, Option<Manifest>>>> = OnceLock::new();
fn policy(home: &Path) -> Option<Manifest> {
    let mut cache = POLICY.get_or_init(Default::default).lock().unwrap();
    cache
        .entry(home.to_owned())
        .or_insert_with(|| read_record(home))
        .clone()
}
fn read_record(home: &Path) -> Option<Manifest> {
    let b = fs::read(home.join(RECORD)).ok()?;
    if b.len() > LIMIT as usize {
        return None;
    }
    let m: Manifest = serde_json::from_slice(&b).ok()?;
    if m.schema == 1 {
        m.validate_schema(1).ok()?;
    } else {
        m.validate().ok()?;
    }
    Some(m)
}
pub fn required(home: &Path, mod_id: &str) -> bool {
    policy(home).is_some_and(|m| {
        m.packages
            .iter()
            .any(|p| p.kind != Kind::Runtime && p.id == mod_id && p.required)
    })
}
pub fn managed(home: &Path, mod_id: &str) -> bool {
    policy(home).is_some_and(|m| {
        m.packages
            .iter()
            .any(|p| p.kind != Kind::Runtime && p.id == mod_id)
    })
}
pub fn manages_instance(i: &instances::Instance) -> bool {
    i.java.is_none() && matches!(i.kind, instances::Kind::Default | instances::Kind::Own)
}
fn fetch_manifest() -> Result<Manifest, String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(err)?
        .as_millis();
    let response = backend::get(&format!("{CHANNEL}?check={now}"))?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(err)?;
    if bytes.len() > LIMIT as usize {
        return Err("Server manifest is too large".into());
    }
    let signature = backend::get(&format!("{CHANNEL}.sig?check={now}"))?;
    let mut text = String::new();
    signature
        .into_reader()
        .take(4097)
        .read_to_string(&mut text)
        .map_err(err)?;
    if text.len() > 4096 {
        return Err("Server manifest signature is too large".into());
    }
    let sums = format!("{:x}  manifest.json\n", Sha256::digest(&bytes));
    horizon_release_sign::verify(
        "server-sync",
        sums.as_bytes(),
        &text,
        &horizon_release_sign::trusted_keys_for(backend::REPO),
    )?;
    let m: Manifest = serde_json::from_slice(&bytes).map_err(err)?;
    m.validate()?;
    Ok(m)
}
#[cfg(not(windows))]
fn replace(from: &Path, to: &Path) -> Result<(), String> {
    fs::rename(from, to).map_err(err)
}
#[cfg(windows)]
fn replace(from: &Path, to: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    let ok = unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(())
    }
}
fn atomic_copy(src: &Path, dst: &Path) -> Result<(), String> {
    if dst.is_symlink() {
        return Err("Refusing a symlink update destination".into());
    }
    fs::create_dir_all(dst.parent().ok_or("Missing update parent")?).map_err(err)?;
    let tmp = dst.with_extension("server-sync-new");
    fs::copy(src, &tmp).map_err(err)?;
    fs::OpenOptions::new()
        .write(true)
        .open(&tmp)
        .map_err(err)?
        .sync_all()
        .map_err(err)?;
    replace(&tmp, dst)
}
fn check_package(path: &Path, entry: &Package, revision: &str) -> Result<(), String> {
    let package = ModPackage::read(path)?;
    if package.is_native() != (entry.kind == Kind::Native)
        || package.manifest.id != entry.id
        || package.manifest.version != entry.version
    {
        return Err(format!("Update identity mismatch: {}", entry.id));
    }
    if package
        .manifest
        .native
        .is_some_and(|native| native.horizon != revision)
    {
        return Err(format!(
            "{} was not built for server SDK {revision}",
            entry.id
        ));
    }
    Ok(())
}
/// Recover only paths recorded by our own bounded transaction, never worlds,
/// account files, or arbitrary paths supplied by a downloaded manifest.
fn recover(home: &Path, runtime: &Path) -> Result<(), String> {
    let backup = home.join(".horizon-server-sync/rollback");
    if !backup.exists() {
        return Ok(());
    }
    let journal = backup.join("journal.json");
    if !journal.exists() {
        fs::remove_dir_all(&backup).map_err(err)?;
        return Ok(());
    }
    let touched: BTreeMap<PathBuf, Option<PathBuf>> =
        serde_json::from_slice(&fs::read(journal).map_err(err)?).map_err(err)?;
    if touched.len() > 1024 {
        return Err("Invalid recovery journal".into());
    }
    for (dest, src) in &touched {
        let allowed = dest == &home.join("horizon-mods.txt")
            || dest == &home.join(RECORD)
            || dest.parent() == Some(home.join("horizon-mods").as_path())
            || dest.parent() == Some(home.join("horizon-native-mods").as_path())
            || dest == &runtime.join("libopus-0.dll")
            || dest == &runtime.join("OPUS-LICENSE.txt");
        if !allowed
            || src
                .as_ref()
                .is_some_and(|p| p.parent() != Some(backup.as_path()))
        {
            return Err("Invalid recovery path".into());
        }
        match src {
            Some(p) => atomic_copy(p, dest)?,
            None => {
                if dest.exists() {
                    fs::remove_file(dest).map_err(err)?;
                }
            }
        }
    }
    fs::remove_dir_all(backup).map_err(err)?;
    POLICY
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .remove(home);
    Ok(())
}

/// A verified package transaction. Failed builds restore the previous mod files,
/// choices and policy; the previous executable remains selected and usable.
fn apply(
    home: &Path,
    m: &Manifest,
    files: &Path,
    runtime: &Path,
    build: impl FnOnce() -> Result<String, String>,
) -> Result<String, String> {
    m.validate()?;
    recover(home, runtime)?;
    // Verify EVERY download before changing even the first active file.
    for p in m.packages.iter().filter(|p| p.applies()) {
        let file = files.join(&p.sha256);
        if hash(&file)? != p.sha256 || fs::metadata(&file).map_err(err)?.len() != p.size {
            return Err(format!("Update checksum mismatch: {}", p.id));
        }
        if p.kind != Kind::Runtime {
            check_package(&file, p, &m.game_revision)?;
        }
    }
    let backup = home.join(".horizon-server-sync/rollback");
    fs::create_dir_all(&backup).map_err(err)?;
    POLICY
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .insert(home.to_owned(), Some(m.clone()));
    let mut touched: BTreeMap<PathBuf, Option<PathBuf>> = BTreeMap::new();
    let mut remember = |dest: &Path| -> Result<(), String> {
        if touched.contains_key(dest) {
            return Ok(());
        }
        if dest.is_symlink() {
            return Err("Refusing a symlink update destination".into());
        }
        let original = if dest.exists() {
            let p = backup.join(touched.len().to_string());
            fs::copy(dest, &p).map_err(err)?;
            Some(p)
        } else {
            None
        };
        touched.insert(dest.to_owned(), original);
        let journal = backup.join("journal.json");
        let temp = backup.join("journal.new");
        fs::write(&temp, serde_json::to_vec(&touched).map_err(err)?).map_err(err)?;
        fs::OpenOptions::new()
            .write(true)
            .open(&temp)
            .map_err(err)?
            .sync_all()
            .map_err(err)?;
        replace(&temp, &journal)?;
        Ok(())
    };
    let result = (|| {
        remember(&home.join("horizon-mods.txt"))?;
        remember(&home.join(RECORD))?;
        // Replace duplicate filenames for a managed id, but preserve the originals
        // in the rollback transaction until the replacement compiled successfully.
        let previous = read_record(home);
        for (folder, kind) in [
            ("horizon-native-mods", Kind::Native),
            ("horizon-mods", Kind::Content),
        ] {
            if let Ok(entries) = fs::read_dir(home.join(folder)) {
                for entry in entries {
                    let entry = entry.map_err(err)?;
                    if !entry.file_type().map_err(err)?.is_file() {
                        continue;
                    }
                    let path = entry.path();
                    let extension = path.extension().and_then(|s| s.to_str());
                    if kind == Kind::Native
                        && (extension == Some("hnmod")
                            || (extension == Some("hmod")
                                && crate::native_status::legacy_format(&path)))
                    {
                        remember(&path)?;
                        fs::remove_file(&path).map_err(err)?;
                        continue;
                    }
                    if extension != Some("hmod") {
                        continue;
                    }
                    if let Ok(package) = ModPackage::read(&path) {
                        let id = package.manifest.id;
                        let wanted = m.packages.iter().find(|p| p.id == id && p.kind == kind);
                        let obsolete = previous.as_ref().is_some_and(|old| {
                            old.packages.iter().any(|p| p.id == id && p.kind == kind)
                        }) || m
                            .packages
                            .iter()
                            .any(|p| p.id == id && p.kind != Kind::Runtime);
                        if wanted.is_some_and(|p| home.join(p.relative()) != path)
                            || (wanted.is_none() && obsolete)
                        {
                            remember(&path)?;
                            fs::remove_file(path).map_err(err)?;
                        }
                    }
                }
            }
        }
        for p in m.packages.iter().filter(|p| p.applies()) {
            let dest = if p.kind == Kind::Runtime {
                runtime.join(p.relative())
            } else {
                home.join(p.relative())
            };
            if hash(&dest).as_deref() != Ok(p.sha256.as_str()) {
                remember(&dest)?;
                atomic_copy(&files.join(&p.sha256), &dest)?;
            }
        }
        // The caller holds the home lock. Apply the managed choices and journal
        // them in the same transaction as the package changes.
        let mut settings = horizon_platform::mod_package::selections::load(home);
        for p in m.packages.iter().filter(|p| p.kind != Kind::Runtime) {
            let key = format!("mod.{}", p.id);
            let matches = |line: &str| line.split_once(':').is_some_and(|(k, _)| k.trim() == key);
            if p.required || !settings.lines().any(matches) {
                let mut kept: Vec<_> = settings.lines().filter(|line| !matches(line)).collect();
                let enabled = format!("{key}:true");
                kept.push(&enabled);
                settings = kept.join("\n") + "\n";
            }
        }
        let choices = backup.join("choices-new");
        fs::write(&choices, settings).map_err(err)?;
        atomic_copy(&choices, &home.join("horizon-mods.txt"))?;
        let version = build()?;
        let bytes = serde_json::to_vec_pretty(m).map_err(err)?;
        let tmp = backup.join("manifest-new");
        fs::write(&tmp, bytes).map_err(err)?;
        atomic_copy(&tmp, &home.join(RECORD))?;
        Ok(version)
    })();
    drop(remember);
    if result.is_err() {
        for (dest, src) in &touched {
            match src {
                Some(p) => atomic_copy(p, dest)?,
                None => {
                    if dest.exists() {
                        fs::remove_file(dest).map_err(err)?;
                    }
                }
            }
        }
    }
    // Identical repeat checks share a backup, but later player edits at the
    // same channel generation must never overwrite or discard earlier originals.
    let mut original_hash = Sha256::new();
    for (dest, src) in &touched {
        original_hash.update(dest.to_string_lossy().as_bytes());
        if let Some(src) = src {
            original_hash.update(hash(src)?.as_bytes());
        }
    }
    let archive_key = format!("{}-{:x}", m.generation, original_hash.finalize());
    let archived = home.join(".horizon-server-sync/backups").join(archive_key);
    fs::create_dir_all(archived.parent().unwrap()).map_err(err)?;
    if archived.exists() {
        fs::remove_dir_all(&backup).map_err(err)?;
    } else {
        fs::rename(&backup, &archived).map_err(err)?;
    }
    POLICY
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .remove(home);
    result
}
/// Inspect the live channel and installed inputs without installing or building.
pub fn check(
    instance: &instances::Instance,
    current: Option<&str>,
) -> Result<serde_json::Value, String> {
    let m = fetch_manifest()?;
    let missing: Vec<_> = m
        .packages
        .iter()
        .filter(|p| p.applies() && p.kind != Kind::Runtime)
        .filter(|p| hash(&instance.home.join(p.relative())).as_deref() != Ok(p.sha256.as_str()))
        .map(|p| p.id.clone())
        .collect();
    let built = current.and_then(native_mods::about);
    Ok(
        serde_json::json!({"generation":m.generation,"revision":m.game_revision,"packages":m.packages.len(),"changed_packages":missing,
        "build_matches":built.is_some_and(|a|a.horizon==m.game_revision&&a.left_out.is_empty()),
        "required":m.packages.iter().filter(|p|p.required&&p.kind!=Kind::Runtime).map(|p|&p.id).collect::<Vec<_>>()}),
    )
}

/// Synchronize an instance and build exactly the server's commit. The download
/// and build callbacks are deliberately separate, each with its own UI timer.
pub fn sync(
    instance: &instances::Instance,
    current: Option<&str>,
    progress: impl Fn(Phase, String),
    registry_consent: impl Fn(&native_mods::RegistryInventory) -> bool,
) -> Result<Outcome, String> {
    native_mods::require_consent().map_err(String::from)?;
    if !manages_instance(instance) {
        return Err("This instance is not server-managed".into());
    }
    progress(
        Phase::Downloading,
        "Checking the dedicated server's updates".into(),
    );
    let cache = backend::data_dir().join("server-sync");
    fs::create_dir_all(&cache).map_err(err)?;
    let lease = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(cache.join("update.lock"))
        .map_err(err)?;
    lease.lock().map_err(err)?;
    let m = fetch_manifest()?;
    let files = cache.join("files");
    fs::create_dir_all(&files).map_err(err)?;
    let keep: BTreeSet<_> = m.packages.iter().map(|p| p.sha256.as_str()).collect();
    for entry in fs::read_dir(&files).map_err(err)? {
        let entry = entry.map_err(err)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.file_type().map_err(err)?.is_file()
            && hex(&name, 64)
            && !keep.contains(name.as_str())
        {
            fs::remove_file(entry.path()).map_err(err)?;
        }
    }
    for p in m.packages.iter().filter(|p| p.applies()) {
        let dest = files.join(&p.sha256);
        if hash(&dest).as_deref() == Ok(p.sha256.as_str()) {
            continue;
        }
        progress(
            Phase::Downloading,
            format!("Downloading {} {}", p.id, p.version),
        );
        backend::download(&p.url, p.size, &p.sha256, &dest, &|_| {})?;
    }
    if !backend::installed().contains(&m.base) {
        backend::install_tag(&m.base, false, |_| {})?;
    }
    let base = m.base.clone();
    if native_mods::revision_of(&base)? != m.game_revision {
        return Err(format!(
            "Release {base} does not supply the server's SDK revision"
        ));
    }
    if !backend::is_prepared(&base) {
        return Ok(Outcome::Prepare(base));
    }
    let runtime = backend::version_dir(&base);
    let _lock = browser::lock_home(&instance.home).map_err(err)?;
    let version = apply(&instance.home, &m, &files, &runtime, || {
        // Only the signed inputs enter staging, including content mods with WASM listeners.
        let approved = tempfile::tempdir_in(&cache).map_err(err)?;
        for p in m
            .packages
            .iter()
            .filter(|p| p.applies() && p.kind != Kind::Runtime)
        {
            atomic_copy(&files.join(&p.sha256), &approved.path().join(p.relative()))?;
        }
        fs::copy(
            instance.home.join("horizon-mods.txt"),
            approved.path().join("horizon-mods.txt"),
        )
        .map_err(err)?;
        if let Some(current) = current.filter(|v| {
            native_mods::about(v)
                .is_some_and(|a| a.horizon == m.game_revision && a.left_out.is_empty())
                && backend::is_prepared(v)
        }) {
            let enabled = crate::native_status::enabled(approved.path());
            let built = crate::native_status::built(current);
            if crate::native_status::state(&enabled, built.as_deref())
                == crate::native_status::State::UpToDate
            {
                return Ok(current.to_owned());
            }
        }
        progress(Phase::Building, "Building the server's mod set".into());
        native_mods::build_pinned(
            &base,
            approved.path(),
            &m.game_revision,
            native_mods::Profile::Optimized,
            |step| {
                let phase = match step {
                    native_mods::Step::Rust(_)
                    | native_mods::Step::Tools { .. }
                    | native_mods::Step::Source(_) => Phase::Downloading,
                    _ => Phase::Building,
                };
                progress(phase, step.label());
            },
            &registry_consent,
        )
    })?;
    // The executable is selected only AFTER both package and build verification.
    instances::edit(&instance.id, |meta| meta.version = Some(version.clone()))?;
    Ok(Outcome::Ready(version))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "horizon-sync-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn fixture(root: &Path, required: bool) -> (Manifest, PathBuf, PathBuf) {
        let cache = root.join("cache");
        let home = root.join("home");
        fs::create_dir_all(&cache).unwrap();
        fs::create_dir_all(&home).unwrap();
        let file = cache.join("fixture");
        let mut zip = zip::ZipWriter::new(fs::File::create(&file).unwrap());
        zip.start_file("horizon-mod.toml", zip::write::SimpleFileOptions::default())
            .unwrap();
        write!(
            zip,
            "id='test_mod'\nname='Test mod'\nversion='1'\napi='0.2'\ncapabilities=['registries']\n[native]\ngame='26.3'\nhorizon='{}'\nmixins='{}'\n",
            "b".repeat(40),
            "c".repeat(40),
        ).unwrap();
        zip.start_file("Cargo.toml", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"[package]\nname='mod-test_mod'\nversion='1.0.0'\nedition='2024'\n")
            .unwrap();
        zip.start_file("src/lib.rs", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"// test source\n").unwrap();
        zip.finish().unwrap();
        let digest = hash(&file).unwrap();
        let size = fs::metadata(&file).unwrap().len();
        fs::rename(file, cache.join(&digest)).unwrap();
        let m = Manifest {
            schema: SCHEMA,
            generation: "a".repeat(64),
            game_revision: "b".repeat(40),
            base: "v0.3.3".into(),
            packages: vec![Package {
                id: "test_mod".into(),
                version: "1".into(),
                kind: Kind::Native,
                sha256: digest,
                size,
                url: format!(
                    "https://github.com/{}/releases/download/sync-test/test.hmod",
                    backend::REPO
                ),
                required,
                platform: None,
            }],
        };
        (m, cache, home)
    }
    #[test]
    fn invalid_origins_paths_and_duplicates_are_rejected() {
        let t = Temp::new();
        let (mut m, _, _) = fixture(&t.0, true);
        m.validate().unwrap();
        m.packages[0].url = "https://evil.example/mod.hmod".into();
        assert!(m.validate().is_err());
        m.packages[0].url = format!(
            "https://github.com/{}/releases/download/x/a.hmod",
            backend::REPO
        );
        m.packages[0].id = "../outside".into();
        assert!(m.validate().is_err());
        m.packages[0].id = "test_mod".into();
        m.packages.push(m.packages[0].clone());
        assert!(m.validate().is_err());
    }
    #[test]
    fn bad_download_changes_no_active_files() {
        let t = Temp::new();
        let (m, files, home) = fixture(&t.0, true);
        fs::write(files.join(&m.packages[0].sha256), b"corrupt").unwrap();
        fs::write(home.join("horizon-mods.txt"), "mod.test_mod:false\n").unwrap();
        assert!(
            apply(&home, &m, &files, &t.0.join("runtime"), || panic!(
                "must not build"
            ))
            .is_err()
        );
        assert_eq!(
            fs::read_to_string(home.join("horizon-mods.txt")).unwrap(),
            "mod.test_mod:false\n"
        );
        assert!(!home.join("horizon-native-mods").exists());
    }
    #[test]
    fn failed_build_restores_packages_choices_and_policy() {
        let t = Temp::new();
        let (m, files, home) = fixture(&t.0, true);
        fs::create_dir_all(home.join("horizon-native-mods")).unwrap();
        fs::write(
            home.join("horizon-native-mods/test_mod.hmod"),
            b"old package",
        )
        .unwrap();
        fs::write(
            home.join("horizon-mods.txt"),
            "mod.test_mod:false\nmod.custom:true\n",
        )
        .unwrap();
        assert!(
            apply(&home, &m, &files, &t.0.join("runtime"), || Err(
                "compiler failed".into()
            ))
            .is_err()
        );
        assert_eq!(
            fs::read(home.join("horizon-native-mods/test_mod.hmod")).unwrap(),
            b"old package"
        );
        assert_eq!(
            fs::read_to_string(home.join("horizon-mods.txt")).unwrap(),
            "mod.test_mod:false\nmod.custom:true\n"
        );
        assert!(!required(&home, "test_mod"));
        assert!(!home.join(RECORD).exists());
    }
    #[test]
    fn legacy_native_install_is_archived_only_on_success() {
        let t = Temp::new();
        let (m, files, home) = fixture(&t.0, true);
        let folder = home.join("horizon-native-mods");
        fs::create_dir_all(&folder).unwrap();
        let old = folder.join("test_mod_native.hnmod");
        let choices = home.join("horizon-mods.txt");
        let runtime = t.0.join("runtime");
        fs::write(&old, b"legacy package").unwrap();
        fs::write(&choices, "mod.custom:false\n").unwrap();

        assert!(
            apply(&home, &m, &files, &runtime, || {
                assert!(!old.exists());
                Err("Registry dependency compilation was not approved".into())
            })
            .is_err()
        );
        assert_eq!(fs::read(&old).unwrap(), b"legacy package");
        assert_eq!(fs::read_to_string(&choices).unwrap(), "mod.custom:false\n");

        apply(&home, &m, &files, &runtime, || {
            assert!(!old.exists());
            assert!(folder.join("test_mod.hmod").is_file());
            Ok("built".into())
        })
        .unwrap();
        assert!(!old.exists());
        let archived = fs::read_dir(home.join(".horizon-server-sync/backups"))
            .unwrap()
            .any(|path| {
                let dir = path.unwrap().path();
                let journal: BTreeMap<PathBuf, Option<PathBuf>> =
                    serde_json::from_slice(&fs::read(dir.join("journal.json")).unwrap()).unwrap();
                journal
                    .get(&old)
                    .and_then(Option::as_ref)
                    .is_some_and(|saved| {
                        fs::read(dir.join(saved.file_name().unwrap())).unwrap() == b"legacy package"
                    })
            });
        assert!(archived);
        assert!(!browser::java::is_enabled(&home, "custom"));
    }

    #[test]
    fn required_mods_are_enabled_and_cannot_be_disabled() {
        let t = Temp::new();
        let (m, files, home) = fixture(&t.0, true);
        fs::write(
            home.join("horizon-mods.txt"),
            "mod.test_mod:false\nmod.custom:false\n",
        )
        .unwrap();
        assert_eq!(
            apply(&home, &m, &files, &t.0.join("runtime"), || Ok(
                "built".into()
            ))
            .unwrap(),
            "built"
        );
        assert!(required(&home, "test_mod"));
        assert!(browser::java::is_enabled(&home, "test_mod"));
        assert!(browser::java::set_enabled(&home, "test_mod", false).is_err());
        assert!(!browser::java::is_enabled(&home, "custom"));
    }
    #[test]
    fn optional_disabled_mod_stays_disabled() {
        let t = Temp::new();
        let (m, files, home) = fixture(&t.0, false);
        fs::write(home.join("horizon-mods.txt"), "mod.test_mod:false\n").unwrap();
        apply(&home, &m, &files, &t.0.join("runtime"), || {
            Ok("built".into())
        })
        .unwrap();
        assert!(!required(&home, "test_mod"));
        assert!(!browser::java::is_enabled(&home, "test_mod"));
        assert!(browser::java::set_enabled(&home, "test_mod", false).is_ok());
    }
    #[test]
    fn later_player_edits_are_backed_up_even_at_the_same_channel_generation() {
        let t = Temp::new();
        let (m, files, home) = fixture(&t.0, true);
        apply(&home, &m, &files, &t.0.join("runtime"), || {
            Ok("built".into())
        })
        .unwrap();
        fs::write(
            home.join("horizon-native-mods/test_mod.hmod"),
            b"player edited package",
        )
        .unwrap();
        apply(&home, &m, &files, &t.0.join("runtime"), || {
            Ok("built".into())
        })
        .unwrap();
        let preserved = fs::read_dir(home.join(".horizon-server-sync/backups"))
            .unwrap()
            .any(|entry| {
                fs::read_dir(entry.unwrap().path()).unwrap().any(|file| {
                    fs::read(file.unwrap().path()).ok().as_deref() == Some(b"player edited package")
                })
            });
        assert!(preserved);
    }
    #[test]
    fn interrupted_transaction_recovers_before_next_update() {
        let t = Temp::new();
        let home = t.0.join("home");
        let backup = home.join(".horizon-server-sync/rollback");
        fs::create_dir_all(&backup).unwrap();
        let dest = home.join("horizon-mods.txt");
        let saved = backup.join("0");
        fs::write(&dest, b"partial").unwrap();
        fs::write(&saved, b"original").unwrap();
        let journal = BTreeMap::from([(dest.clone(), Some(saved))]);
        fs::write(
            backup.join("journal.json"),
            serde_json::to_vec(&journal).unwrap(),
        )
        .unwrap();
        recover(&home, &t.0.join("runtime")).unwrap();
        assert_eq!(fs::read(dest).unwrap(), b"original");
        assert!(!backup.exists());
    }
}
