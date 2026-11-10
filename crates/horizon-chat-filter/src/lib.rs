//! Persistent operator-issued mutes and duration utilities. No word matching or automatic punishments.
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The mutes, beside server.properties.
const MINUTE: i64 = 60_000;

pub const MUTES_FILE: &str = "chat-filter-mutes.json";

/// Legacy section controlling operator mute commands; old word-filter options are ignored.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub enabled: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self { enabled: true }
    }
}

impl Config {
    /// The `[chat_filter]` section of a whole `server.toml`; the default without it.
    pub fn from_server_toml(text: &str) -> Result<Self, String> {
        #[derive(Deserialize)]
        struct File {
            #[serde(default)]
            chat_filter: Config,
        }
        // Other sections are the supervisor's to check: only this one is read here.
        let mut table: toml::Table = toml::from_str(text).map_err(|e| e.to_string())?;
        let mut only = toml::Table::new();
        if let Some(section) = table.remove("chat_filter") {
            only.insert("chat_filter".into(), section);
        }
        let file: File = only
            .try_into()
            .map_err(|e: toml::de::Error| format!("[chat_filter]: {e}"))?;
        file.chat_filter.check()?;
        Ok(file.chat_filter)
    }

    pub fn check(&self) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Mute {
    pub uuid: String,
    pub name: String,
    /// Epoch milliseconds; `None`: until `/unmute`.
    pub until: Option<i64>,
    /// The operator who muted them (`/mute`, `/tempmute`); `None`: the filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl Mute {
    pub fn active(&self, now: i64) -> bool {
        self.until.is_none_or(|until| until > now)
    }
}

#[derive(Default, Serialize, Deserialize)]
struct MutesFile {
    mutes: Vec<Mute>,
}

/// Persistent operator-issued mutes.
pub struct Guard {
    pub config: Config,
    mutes: HashMap<u128, Mute>,
    /// Where the mutes are kept; `None` keeps them in memory only (tests).
    file: Option<PathBuf>,
}

impl Guard {
    pub fn new(config: Config, file: Option<PathBuf>) -> Self {
        let mut mutes = HashMap::new();
        if let Some(text) = file.as_ref().and_then(|f| std::fs::read_to_string(f).ok())
            && let Ok(saved) = serde_json::from_str::<MutesFile>(&text)
        {
            for m in saved.mutes {
                if m.by.is_some()
                    && let Some(uuid) = parse_uuid(&m.uuid)
                {
                    mutes.insert(uuid, m);
                }
            }
        }
        Self {
            config,
            mutes,
            file,
        }
    }

    /// Load operator-issued mutes, ignoring historical automatic mutes.
    pub fn load(config: Config, dir: &Path) -> (Self, Option<String>) {
        (Self::new(config, Some(dir.join(MUTES_FILE))), None)
    }

    /// The player's mute, if they are muted at `now`.
    pub fn muted(&self, uuid: u128, now: i64) -> Option<&Mute> {
        self.mutes.get(&uuid).filter(|m| m.active(now))
    }

    /// The mutes in force at `now`, by name.
    pub fn mutes(&self, now: i64) -> Vec<&Mute> {
        let mut mutes: Vec<&Mute> = self.mutes.values().filter(|m| m.active(now)).collect();
        mutes.sort_by(|a, b| a.name.cmp(&b.name));
        mutes
    }

    /// An operator's `/mute` (`until` `None`) or `/tempmute`: replaces the player's mute.
    pub fn mute(
        &mut self,
        uuid: u128,
        name: &str,
        until: Option<i64>,
        by: &str,
        reason: Option<String>,
    ) {
        self.mutes.insert(
            uuid,
            Mute {
                uuid: uuid_string(uuid),
                name: name.to_owned(),
                until,
                by: Some(by.to_owned()),
                reason,
            },
        );
        self.save();
    }

    /// Lifts a player's mute by name; the name as muted, if they were.
    pub fn unmute(&mut self, name: &str) -> Option<String> {
        let uuid = *self
            .mutes
            .iter()
            .find(|(_, m)| m.name.eq_ignore_ascii_case(name))?
            .0;
        let mute = self.mutes.remove(&uuid)?;
        self.save();
        Some(mute.name)
    }

    fn save(&mut self) {
        let Some(file) = &self.file else {
            return;
        };
        let now = now_millis();
        self.mutes.retain(|_, m| m.active(now));
        let mut mutes: Vec<Mute> = self.mutes.values().cloned().collect();
        mutes.sort_by(|a, b| a.name.cmp(&b.name));
        if let Ok(text) = serde_json::to_string_pretty(&MutesFile { mutes }) {
            let tmp = file.with_extension("json.tmp");
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(&tmp, file);
            }
        }
    }
}

/// `/tempmute`'s duration in milliseconds: a positive whole number and a unit, `s`, `m`, `h`, `d` or
/// `w` (`10m`, `2h`, `1d`).
pub fn parse_duration(text: &str) -> Option<i64> {
    let unit = match text.chars().last()? {
        's' => 1_000,
        'm' => MINUTE,
        'h' => 60 * MINUTE,
        'd' => 24 * 60 * MINUTE,
        'w' => 7 * 24 * 60 * MINUTE,
        _ => return None,
    };
    let count: i64 = text[..text.len() - 1].parse().ok()?;
    (count > 0).then(|| count.checked_mul(unit)).flatten()
}

/// The unit used when presenting a moderation duration to a player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DurationUnit {
    Minute,
    Hour,
    Day,
}

/// A duration's rounded count and unit, shared by logs and translated replies.
pub fn duration_parts(millis: i64) -> (i64, DurationUnit) {
    let (unit, kind) = if millis <= 60 * MINUTE {
        (MINUTE, DurationUnit::Minute)
    } else if millis <= 48 * 60 * MINUTE {
        (60 * MINUTE, DurationUnit::Hour)
    } else {
        (24 * 60 * MINUTE, DurationUnit::Day)
    };
    (
        millis.div_euclid(unit) + i64::from(millis.rem_euclid(unit) > 0),
        kind,
    )
}

/// `millis` for logs, rounded up: minutes up to an hour, hours up to two days, then days.
pub fn duration_text(millis: i64) -> String {
    let (count, unit) = duration_parts(millis);
    let unit = match unit {
        DurationUnit::Minute => "minute",
        DurationUnit::Hour => "hour",
        DurationUnit::Day => "day",
    };
    format!("{count} {unit}{}", if count == 1 { "" } else { "s" })
}

pub fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// `YYYY-MM-DDTHH:MM:SSZ` for epoch milliseconds.
pub fn utc(millis: i64) -> String {
    let secs = millis.div_euclid(1000);
    let (days, rest) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest / 60 % 60,
        rest % 60
    )
}

fn uuid_string(u: u128) -> String {
    let h = format!("{u:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    )
}

fn parse_uuid(s: &str) -> Option<u128> {
    u128::from_str_radix(&s.replace('-', ""), 16).ok()
}

#[cfg(test)]
mod tests;
