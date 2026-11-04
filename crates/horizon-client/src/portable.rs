//! Installation identities for offline clients; no Microsoft credentials or hardware identifiers.
//! A fresh archive has no key. The first connection creates one in the player's own folder.
use ring::{
    digest,
    rand::{SecureRandom, SystemRandom},
    signature::{self, Ed25519KeyPair, KeyPair},
};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
    sync::Mutex,
};

pub const CHANNEL: &str = "horizon:portable_v1";
pub const MARKER: &str = ".horizon-portable";
const DOMAIN: &[u8] = b"Horizon portable login v1\0";
static CREATE: Mutex<()> = Mutex::new(());

pub struct Identity(Ed25519KeyPair);
impl Identity {
    pub fn load_or_create(folder: &Path) -> Result<Self, String> {
        let _guard = CREATE.lock().unwrap();
        fs::create_dir_all(folder).map_err(|e| e.to_string())?;
        let path = folder.join("portable-identity.key");
        let seed = match fs::read(&path) {
            Ok(seed) => seed,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let mut seed = [0u8; 32];
                SystemRandom::new()
                    .fill(&mut seed)
                    .map_err(|_| "Cannot generate portable identity")?;
                let mut options = OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                match options.open(&path) {
                    Ok(mut file) => {
                        file.write_all(&seed)
                            .and_then(|()| file.sync_all())
                            .map_err(|e| e.to_string())?;
                        seed.to_vec()
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                        fs::read(&path).map_err(|e| e.to_string())?
                    }
                    Err(e) => return Err(e.to_string()),
                }
            }
            Err(e) => return Err(e.to_string()),
        };
        let key = Ed25519KeyPair::from_seed_unchecked(&seed).map_err(
            |_| "Invalid portable-identity.key; restore its backup to retain your player identity",
        )?;
        Ok(Self(key))
    }
    pub fn uuid(&self) -> u128 {
        uuid(self.0.public_key().as_ref())
    }
    pub fn answer(&self, name: &str, challenge: &[u8]) -> Result<Vec<u8>, String> {
        self.answer_bound(name, challenge, &[])
    }
    pub fn answer_bound(
        &self,
        name: &str,
        challenge: &[u8],
        binding: &[u8],
    ) -> Result<Vec<u8>, String> {
        let message = message(name, challenge, binding)?;
        let mut out = self.0.public_key().as_ref().to_vec();
        out.extend_from_slice(self.0.sign(&message).as_ref());
        Ok(out)
    }
}
fn message(name: &str, challenge: &[u8], binding: &[u8]) -> Result<Vec<u8>, String> {
    if challenge.len() != 32 || name.is_empty() || name.len() > 16 {
        return Err("Invalid portable login challenge".into());
    }
    let mut out = DOMAIN.to_vec();
    out.push(name.len() as u8);
    out.extend_from_slice(name.as_bytes());
    out.extend_from_slice(challenge);
    out.extend_from_slice(binding);
    Ok(out)
}
fn uuid(public: &[u8]) -> u128 {
    let mut input = DOMAIN.to_vec();
    input.extend_from_slice(public);
    let hash = digest::digest(&digest::SHA256, &input);
    let mut bytes: [u8; 16] = hash.as_ref()[..16].try_into().unwrap();
    // Version 8 keeps these identities separate from Microsoft v4 and legacy offline v3 UUIDs.
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    u128::from_be_bytes(bytes)
}
pub fn is_portable(uuid: u128) -> bool {
    (uuid >> 76) & 15 == 8 && (uuid >> 62) & 3 == 2
}
/// A separate username namespace prevents portable clients claiming a Microsoft player's name.
pub fn name(uuid: u128) -> String {
    format!("P{:015x}", uuid >> 68)
}
pub fn verify(name: &str, claimed: u128, challenge: &[u8], answer: &[u8]) -> Result<u128, String> {
    verify_bound(name, claimed, challenge, answer, &[])
}
pub fn verify_bound(
    name: &str,
    claimed: u128,
    challenge: &[u8],
    answer: &[u8],
    binding: &[u8],
) -> Result<u128, String> {
    let bad = || "Invalid portable identity proof".to_owned();
    if answer.len() != 96 {
        return Err(bad());
    }
    let id = uuid(&answer[..32]);
    if id != claimed {
        return Err(bad());
    }
    signature::UnparsedPublicKey::new(&signature::ED25519, &answer[..32])
        .verify(&message(name, challenge, binding)?, &answer[32..])
        .map_err(|_| bad())?;
    Ok(id)
}
/// Portable distributions keep everything beside the executable, independent of cwd/platform data.
/// Explicit arguments still win; dedicated servers never inherit client portable paths.
pub fn arguments() -> Vec<String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf));
    portable_arguments(args, root.as_deref())
}
fn portable_arguments(mut args: Vec<String>, root: Option<&Path>) -> Vec<String> {
    let Some(root) = root.filter(|p| p.join(MARKER).is_file()) else {
        return args;
    };
    if args.iter().any(|a| a == "--dedicated") {
        return args;
    }
    for (flag, path) in [
        ("--resources", root.join("resources")),
        ("--game-dir", root.join("data")),
        ("--home", root.join("data")),
        ("--player-dir", root.join("data")),
    ] {
        let present = args
            .iter()
            .any(|a| a == flag || (flag == "--game-dir" && a == "--gameDir"));
        if !present {
            args.extend([flag.into(), path.to_string_lossy().into_owned()]);
        }
    }
    args
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    fn folder() -> PathBuf {
        let mut random = [0; 8];
        SystemRandom::new().fill(&mut random).unwrap();
        let p =
            std::env::temp_dir().join(format!("horizon-portable-{:x}", u64::from_be_bytes(random)));
        fs::create_dir_all(&p).unwrap();
        p
    }
    #[test]
    fn stable_identity_and_proof() {
        let p = folder();
        let a = Identity::load_or_create(&p).unwrap();
        let b = Identity::load_or_create(&p).unwrap();
        assert_eq!(a.uuid(), b.uuid());
        assert!(is_portable(a.uuid()));
        let nonce = [7; 32];
        let proof = a.answer("Friend", &nonce).unwrap();
        assert_eq!(
            verify("Friend", a.uuid(), &nonce, &proof).unwrap(),
            a.uuid()
        );
        assert!(verify("Other", a.uuid(), &nonce, &proof).is_err());
        assert!(verify("Friend", a.uuid(), &[8; 32], &proof).is_err());
        assert!(verify("Friend", a.uuid() ^ 1, &nonce, &proof).is_err());
        let bound = a
            .answer_bound("Friend", &nonce, b"encrypted-session-A")
            .unwrap();
        assert!(verify_bound("Friend", a.uuid(), &nonce, &bound, b"encrypted-session-A").is_ok());
        assert!(verify_bound("Friend", a.uuid(), &nonce, &bound, b"encrypted-session-B").is_err());
        assert!(verify_bound("Friend", a.uuid(), &nonce, &proof, b"encrypted-session-A").is_err());
        let mut bad = proof.clone();
        bad[50] ^= 1;
        assert!(verify("Friend", a.uuid(), &nonce, &bad).is_err());
        assert!(verify("Friend", a.uuid(), &nonce, &proof[..95]).is_err());
        assert!(a.answer("Friend", &[1; 31]).is_err());
        assert_eq!(name(a.uuid()).len(), 16);
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn fresh_installs_differ_and_corrupt_keys_are_preserved() {
        let p = folder();
        let q = folder();
        let a = Identity::load_or_create(&p).unwrap();
        let b = Identity::load_or_create(&q).unwrap();
        assert_ne!(a.uuid(), b.uuid());
        fs::write(p.join("portable-identity.key"), b"bad").unwrap();
        assert!(Identity::load_or_create(&p).is_err());
        assert_eq!(fs::read(p.join("portable-identity.key")).unwrap(), b"bad");
        fs::remove_dir_all(p).unwrap();
        fs::remove_dir_all(q).unwrap();
    }
    #[test]
    fn portable_paths_respect_overrides_and_dedicated() {
        let p = folder();
        fs::write(p.join(MARKER), b"").unwrap();
        let args = portable_arguments(vec![], Some(&p));
        assert_eq!(args.len(), 8);
        assert!(args.contains(&p.join("resources").to_string_lossy().into_owned()));
        let overridden = portable_arguments(vec!["--game-dir".into(), "custom".into()], Some(&p));
        assert_eq!(overridden.iter().filter(|a| *a == "--game-dir").count(), 1);
        assert_eq!(overridden[1], "custom");
        let dedicated = vec!["--dedicated".to_owned()];
        assert_eq!(portable_arguments(dedicated.clone(), Some(&p)), dedicated);
        fs::remove_dir_all(p).unwrap();
    }
}
