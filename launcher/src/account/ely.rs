//! Ely.by sessions, kept separate from Microsoft identities and ownership proofs.
//! Passwords are sent only to Ely.by and are never persisted or logged.
use super::*;
use serde::{Deserialize, Serialize};
const AUTH: &str = "https://authserver.ely.by";
pub const TWO_FACTOR: &str = "Enter your Ely.by two-factor code and try again.";
#[derive(Clone, Serialize, Deserialize)]
struct Tokens {
    access: String,
    client: String,
}
impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ElyTokens(<hidden>)")
    }
}
pub fn key(uuid: &str) -> String {
    format!("ely:{uuid}")
}
fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(20))
        .redirects(0)
        .build()
}
fn post(base: &str, path: &str, body: &Value) -> Result<Value, String> {
    match agent()
        .post(&format!("{base}{path}"))
        .send_json(body.clone())
    {
        Ok(r) => r
            .into_json()
            .map_err(|_| "Ely.by returned an invalid response".into()),
        Err(ureq::Error::Status(status, r)) => {
            let error: Value = r.into_json().unwrap_or(Value::Null);
            if error.get("errorMessage").and_then(Value::as_str)
                == Some("Account protected with two factor auth.")
            {
                Err(TWO_FACTOR.into())
            } else if status == 429 {
                Err("Ely.by is receiving too many attempts. Try again shortly.".into())
            } else if status >= 500 {
                Err("Ely.by is temporarily unavailable. Try again shortly.".into())
            } else {
                Err("Ely.by sign-in failed. Check your login, password and two-factor code.".into())
            }
        }
        Err(_) => Err("Could not reach Ely.by. Check your connection and retry.".into()),
    }
}
fn response(v: Value, client: &str) -> Result<(Account, Tokens), String> {
    let profile = &v["selectedProfile"];
    let uuid = profile["id"]
        .as_str()
        .unwrap_or_default()
        .replace('-', "")
        .to_lowercase();
    let name = profile["name"].as_str().unwrap_or_default();
    if uuid.len() != 32
        || !uuid.bytes().all(|b| b.is_ascii_hexdigit())
        || !u128::from_str_radix(&uuid, 16).is_ok_and(horizon_platform::auth_provider::valid_ely_uuid)
    {
        return Err("Ely.by returned an invalid account UUID.".into());
    }
    if !horizon_platform::auth_provider::valid_ely_name(name) {
        return Err("Ely.by returned an invalid account name.".into());
    }
    let access = v["accessToken"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("Ely.by returned no session token")?
        .to_owned();
    if v["clientToken"].as_str() != Some(client) {
        return Err("Ely.by returned an unexpected launcher session.".into());
    }
    Ok((
        Account {
            uuid,
            name: name.into(),
            skin: None,
            cape: None,
        },
        Tokens {
            access,
            client: client.into(),
        },
    ))
}
fn authenticate_at(
    base: &str,
    login: &str,
    password: &str,
    otp: &str,
    client: &str,
) -> Result<(Account, Tokens), String> {
    if login.trim().is_empty() || password.is_empty() {
        return Err("Enter your Ely.by email or username and password.".into());
    }
    let otp = otp.trim();
    if !otp.is_empty() && (otp.len() != 6 || !otp.bytes().all(|b| b.is_ascii_digit())) {
        return Err("The two-factor code must contain six digits.".into());
    }
    let password = if otp.is_empty() {
        password.to_owned()
    } else {
        format!("{password}:{otp}")
    };
    response(
        post(
            base,
            "/auth/authenticate",
            &json!({"username":login.trim(),"password":password,"clientToken":client,"requestUser":true}),
        )?,
        client,
    )
}
fn read(store: &dyn SecretStore, uuid: &str) -> Result<Tokens, String> {
    let text = store.get(&key(uuid))?.ok_or("Sign in to Ely.by again.")?;
    serde_json::from_str(&text)
        .map_err(|_| "The Ely.by session could not be read. Sign in again.".into())
}
pub fn sign_in(login: &str, password: &str, otp: &str) -> Result<Account, String> {
    let mut random = [0u8; 32];
    getrandom::fill(&mut random).map_err(|_| "Could not create a launcher session")?;
    let client = random
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let (account, tokens) = authenticate_at(AUTH, login, password, otp, &client)?;
    let mut account = account;
    refresh_look(&mut account);
    let store = super::store();
    let _lock = super::session::lock(store)?;
    store.set(
        &key(&account.uuid),
        &serde_json::to_string(&tokens).map_err(|_| "Could not save Ely.by sign-in")?,
    )?;
    let mut accounts = Accounts::load();
    if let Some(old) = accounts.ely.iter_mut().find(|a| a.uuid == account.uuid) {
        *old = account.clone();
    } else {
        accounts.ely.push(account.clone());
    }
    accounts.selected = Some(key(&account.uuid));
    accounts.save()?;
    Ok(account)
}
pub fn game_session(
    home: &Path,
    accounts: &mut Accounts,
    selected: &str,
) -> Result<GameSession, String> {
    let uuid = selected
        .strip_prefix("ely:")
        .ok_or("Invalid Ely.by account")?;
    let store = super::store();
    let _lock = super::session::lock(store)?;
    *accounts = Accounts::load();
    if !accounts.ely.iter().any(|a| a.uuid == uuid) {
        return Err("Sign in to Ely.by again.".into());
    }
    let tokens = read(store, uuid)?;
    let (mut profile, tokens) = refresh_at(AUTH, &tokens)?;
    if profile.uuid != uuid {
        return Err("Ely.by refreshed a different account. Sign in again.".into());
    }
    refresh_look(&mut profile);
    store.set(
        selected,
        &serde_json::to_string(&tokens).map_err(|_| "Could not save Ely.by session")?,
    )?;
    if let Some(account) = accounts.ely.iter_mut().find(|a| a.uuid == uuid) {
        *account = profile.clone();
    }
    accounts.save()?;
    let file = super::write_session_body(
        &home.join("sessions"),
        &json!({"name":profile.name,"uuid":profile.uuid,"access_token":tokens.access,"user_type":"ely"}),
    )?;
    let file = super::track_session_file(file)?;
    Ok(GameSession {
        file: Some(file),
        offline: None,
        env: Vec::new(),
    })
}

fn refresh_at(base: &str, tokens: &Tokens) -> Result<(Account, Tokens), String> {
    response(
        post(
            base,
            "/auth/refresh",
            &json!({"accessToken":tokens.access,"clientToken":tokens.client,"requestUser":true}),
        )?,
        &tokens.client,
    )
}
fn refresh_look(account: &mut Account) {
    use base64::Engine;
    let profile = agent()
        .get(&format!("{AUTH}/session/profile/{}", account.uuid))
        .call()
        .ok()
        .and_then(|r| r.into_json::<Value>().ok());
    let Some(textures) = profile
        .and_then(|p| {
            p["properties"]
                .as_array()?
                .iter()
                .find(|v| v["name"] == "textures")?["value"]
                .as_str()
                .map(str::to_owned)
        })
        .and_then(|v| base64::engine::general_purpose::STANDARD.decode(v).ok())
        .and_then(|v| serde_json::from_slice::<Value>(&v).ok())
    else {
        return;
    };
    account.skin = textures["textures"]["SKIN"]["url"]
        .as_str()
        .filter(|u| allowed_skin_url(u))
        .map(|u| {
            (
                u.to_owned(),
                if textures["textures"]["SKIN"]["metadata"]["model"] == "slim" {
                    "slim"
                } else {
                    "classic"
                }
                .into(),
            )
        });
    account.cape = textures["textures"]["CAPE"]["url"]
        .as_str()
        .filter(|u| allowed_skin_url(u))
        .map(str::to_owned);
}
pub fn sign_out(selected: &str) -> Result<(), String> {
    let uuid = selected
        .strip_prefix("ely:")
        .ok_or("Invalid Ely.by account")?;
    let store = super::store();
    let _lock = super::session::lock(store)?;
    // Invalidate only this launcher session, never all devices. Forget locally even offline.
    if let Ok(tokens) = read(store, uuid) {
        let _ = agent()
            .post(&format!("{AUTH}/auth/invalidate"))
            .send_json(json!({"accessToken":tokens.access,"clientToken":tokens.client}));
    }
    let mut accounts = Accounts::load();
    accounts.remove_locked(store, selected)?;
    accounts.save()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::TcpListener,
    };
    const UUID: &str = "11111111222243338444555555555555";
    fn reply(token: &str) -> Value {
        json!({"selectedProfile":{"id":UUID,"name":"Ely_Player"},"clientToken":"client","accessToken":token})
    }
    /// Every request is captured in memory; only dummy credentials reach loopback.
    fn mock(
        responses: Vec<(u16, Value)>,
    ) -> (String, std::thread::JoinHandle<Vec<(String, Value)>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut input = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                input.read_line(&mut line).unwrap();
                let path = line.split_whitespace().nth(1).unwrap().to_owned();
                let mut len = 0;
                loop {
                    let mut line = String::new();
                    input.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some((k, v)) = line.split_once(':') {
                        if k.eq_ignore_ascii_case("content-length") {
                            len = v.trim().parse().unwrap();
                        }
                    }
                }
                let mut bytes = vec![0; len];
                input.read_exact(&mut bytes).unwrap();
                requests.push((path, serde_json::from_slice(&bytes).unwrap()));
                let body = body.to_string();
                write!(stream,"HTTP/1.1 {status} Mock\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            }
            requests
        });
        (url, worker)
    }
    #[test]
    fn ely_two_factor_and_refresh_follow_documented_protocol() {
        let (url, worker) = mock(vec![
            (
                401,
                json!({"error":"ForbiddenOperationException","errorMessage":"Account protected with two factor auth."}),
            ),
            (200, reply("dummy-one")),
            (200, reply("dummy-two")),
        ]);
        let error = authenticate_at(&url, " user ", "dummy-password", "", "client").unwrap_err();
        assert!(error.contains("two-factor"));
        assert!(!error.contains("dummy-password"));
        let (account, tokens) =
            authenticate_at(&url, " user ", "dummy-password", "123456", "client").unwrap();
        assert_eq!(account.uuid, UUID);
        let (_, fresh) = refresh_at(&url, &tokens).unwrap();
        assert_eq!(fresh.access, "dummy-two");
        assert!(!format!("{fresh:?}").contains("dummy-two"));
        let requests = worker.join().unwrap();
        assert_eq!(requests[1].0, "/auth/authenticate");
        assert_eq!(requests[1].1["username"], "user");
        assert_eq!(requests[1].1["password"], "dummy-password:123456");
        assert_eq!(requests[2].0, "/auth/refresh");
        assert_eq!(requests[2].1["accessToken"], "dummy-one");
        assert!(requests[2].1.get("password").is_none());
    }
    #[test]
    fn ely_rejects_invalid_profiles_sessions_and_otp_without_network() {
        assert!(
            authenticate_at("http://127.0.0.1:1", "user", "pw", "xx", "client")
                .unwrap_err()
                .contains("six digits")
        );
        let mut v = reply("dummy");
        v["clientToken"] = json!("other");
        assert!(response(v, "client").is_err());
        let mut v = reply("dummy");
        v["selectedProfile"]["id"] = json!("11111111222283338444555555555555");
        assert!(response(v, "client").is_err());
        let mut v = reply("dummy");
        v["accessToken"] = json!("");
        assert!(response(v, "client").is_err());
    }
    #[test]
    fn ely_storage_selection_and_removal_preserve_microsoft_identity() {
        let store = MemoryStore::default();
        store.set(UUID, "microsoft-secret").unwrap();
        let (profile, tokens) = response(reply("ely-secret"), "client").unwrap();
        store
            .set(&key(UUID), &serde_json::to_string(&tokens).unwrap())
            .unwrap();
        let mut accounts: Accounts =
            serde_json::from_value(json!({"accounts":[profile.clone()],"selected":UUID})).unwrap();
        assert!(accounts.ely.is_empty());
        accounts.ely.push(profile);
        accounts
            .play_as(&store, Player::Account(key(UUID)))
            .unwrap();
        assert_eq!(accounts.player(), Player::Account(key(UUID)));
        assert_eq!(accounts.selected().unwrap().uuid, UUID);
        assert!(!accounts.has_owner(&store));
        assert_eq!(read(&store, UUID).unwrap().access, "ely-secret");
        let metadata = serde_json::to_string(&accounts).unwrap();
        assert!(!metadata.contains("secret"));
        accounts.remove(&store, &key(UUID)).unwrap();
        assert!(accounts.ely.is_empty());
        assert_eq!(accounts.accounts.len(), 1);
        assert_eq!(
            store.get(UUID).unwrap().as_deref(),
            Some("microsoft-secret")
        );
        assert!(store.get(&key(UUID)).unwrap().is_none());
    }
    #[test]
    fn ely_accepts_registration_uuid_and_provider_name_shapes() {
        for id in [
            "11111111222243338444555555555555",
            "11111111-2222-5333-8444-555555555555",
        ] {
            for name in ["Lunar-Moon", "name.name", "Игрок"] {
                let mut v = reply("dummy");
                v["selectedProfile"] = json!({"id":id,"name":name});
                let (account, _) = response(v, "client").unwrap();
                assert_eq!(account.name, name);
            }
        }
    }
    #[test]
    fn ely_skin_hosts_are_restricted() {
        assert!(allowed_skin_url("https://ely.by/storage/skins/abc.png"));
        assert!(allowed_skin_url(
            "https://skinsystem.ely.by/skins/Ely_Player.png"
        ));
        assert!(!allowed_skin_url(
            "https://ely.by.attacker.test/storage/skins/a.png"
        ));
        assert!(!allowed_skin_url("https://ely.by/storage/skins/../private"));
        assert!(!allowed_skin_url("http://127.0.0.1/private"));
    }
    #[test]
    fn ely_session_handoff_is_private_and_uses_shared_cleanup() {
        let dir = std::env::temp_dir().join(format!("hz-ely-session-{}", std::process::id()));
        let file = super::super::write_session_body(
            &dir,
            &json!({"name":"Ely_Player","uuid":UUID,"access_token":"dummy","user_type":"ely"}),
        )
        .unwrap();
        let file = super::super::track_session_file(file).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&file).unwrap().permissions().mode() & 0o077,
                0
            );
        }
        let value: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(value["user_type"], "ely");
        super::super::reap(&mut vec![file.clone()], Duration::ZERO);
        assert!(!file.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn ely_provider_errors_do_not_echo_response_secrets() {
        let (url, worker) = mock(vec![(
            401,
            json!({"error":"ForbiddenOperationException","errorMessage":"secret-from-provider"}),
        )]);
        let error = authenticate_at(&url, "user", "dummy-password", "", "client").unwrap_err();
        assert!(!error.contains("secret-from-provider"));
        assert!(!error.contains("dummy-password"));
        worker.join().unwrap();
    }
}

pub(crate) fn allowed_skin_url(url: &str) -> bool {
    [
        "https://ely.by/storage/skins/",
        "http://ely.by/storage/skins/",
        "https://ely.by/storage/capes/",
        "http://ely.by/storage/capes/",
        "https://textures.minecraft.net/texture/",
        "http://textures.minecraft.net/texture/",
        "https://skinsystem.ely.by/skins/",
        "http://skinsystem.ely.by/skins/",
        "https://skinsystem.ely.by/cloaks/",
        "http://skinsystem.ely.by/cloaks/",
    ]
    .iter()
    .filter_map(|prefix| url.strip_prefix(prefix))
    .any(|suffix| {
        !suffix.is_empty()
            && suffix
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    })
}
