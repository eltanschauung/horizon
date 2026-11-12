//! Materialize embedded third-party packs in an owned, versioned cache.
use std::{fs, io::{self, Write}, path::{Path, PathBuf}, sync::atomic::{AtomicU64, Ordering}};
const PACP: &[u8] = include_bytes!("../../bundled-resource-packs/pacp-2.5.4.zip");
static NEXT: AtomicU64 = AtomicU64::new(0);
pub(super) fn pacp(packs_dir: &Path) -> io::Result<PathBuf> {
    materialize(packs_dir, PACP, "pacp-2.5.4-0cd932771220323860775cda339aa0173cd621ce.zip")
}
pub(super) fn mushrooms_plus(packs_dir: &Path) -> io::Result<PathBuf> {
    materialize(packs_dir, include_bytes!("../../bundled-resource-packs/mushrooms-plus-26.3-1.4.zip"), "mushrooms-plus-8c5702208eeef94bdd929e1f8ed9401213f4d48791901bff3bfc476494b03747.zip")
}
fn materialize(packs_dir: &Path, bytes: &[u8], filename: &str) -> io::Result<PathBuf> {
    let dir = packs_dir.parent().unwrap_or_else(|| Path::new(".")).join(".horizon-cache/bundled-packs");
    let path = dir.join(filename);
    if fs::metadata(&path).is_ok_and(|m| m.len() == bytes.len() as u64) { return Ok(path); }
    fs::create_dir_all(&dir)?;
    let temp = dir.join(format!("pacp-{}-{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let result = (|| -> io::Result<()> {
        let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temp, &path)
    })();
    if result.is_err() { let _ = fs::remove_file(temp); }
    result?;
    Ok(path)
}
