//! Operator-issued chat mutes. Text contents are never inspected.
use super::{HostEvent, Hub, chat::now_millis};
use horizon_chat_filter::{Guard, Mute, duration_text};
use horizon_protocol::{
    packets::play::{self, Clientbound as C},
    text::literal,
};
use horizon_world::nbt::{JavaString, Tag};

/// Where a player's text was going.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// A chat message.
    Chat,
    /// A command's message: `/msg`, `/tell`, `/w`, `/me`, `/say`, `/teammsg`.
    Command,
    Sign,
    Anvil,
    Book,
}

impl Place {
    pub fn name(self) -> &'static str {
        match self {
            Place::Chat => "chat",
            Place::Command => "command",
            Place::Sign => "sign",
            Place::Anvil => "anvil",
            Place::Book => "book",
        }
    }
    /// Chat and commands: a muted player can't use them.
    fn is_chat(self) -> bool {
        matches!(self, Place::Chat | Place::Command)
    }
}

/// What a muted player reads: when they try to chat, and when an operator mutes them.
pub fn muted_notice(mute: &Mute, now: i64) -> String {
    let time = match mute.until {
        // "2 more minutes".
        Some(until) => format!(
            "for {}",
            duration_text(until - now).replacen(' ', " more ", 1)
        ),
        None => "until an operator unmutes you".into(),
    };
    let why = match (&mute.by, &mute.reason) {
        (None, _) => "",
        (Some(_), Some(reason)) => reason,
        (Some(_), None) => "",
    };
    if why.is_empty() {
        format!("You're muted {time}.")
    } else {
        format!("You're muted {time}: {why}.")
    }
}

/// The text in red.
fn red(text: &str) -> Tag {
    let mut t = literal(text);
    if let Tag::Compound(map) = &mut t {
        map.insert(
            JavaString::from("color"),
            Tag::String(JavaString::from("red")),
        );
    }
    t
}

impl Hub {
    /// `server.toml`'s `[chat_filter]` (dedicated servers); `None` turns it off.
    pub fn set_chat_filter(&mut self, guard: Option<Guard>) {
        self.chat_filter = guard.map(Box::new);
    }

    pub fn chat_filter_on(&self) -> bool {
        self.chat_filter.is_some()
    }

    /// Only operator-issued mutes can block chat or message commands. Text contents are ignored.
    pub fn chat_allowed(
        &mut self,
        i: usize,
        _texts: &[&str],
        place: Place,
        _events: &mut Vec<HostEvent>,
    ) -> bool {
        let Some(guard) = self.chat_filter.as_deref_mut() else {
            return true;
        };
        let (uuid, now) = (self.guests[i].uuid, now_millis());
        let Some(mute) = place.is_chat().then(|| guard.muted(uuid, now)).flatten() else {
            return true;
        };
        self.guests[i].send(&C::SystemChat(play::SystemChat {
            content: red(&muted_notice(mute, now)),
            overlay: false,
        }));
        false
    }

    /// An operator's `/mute` or `/tempmute`; the player is told if they are here.
    pub fn mute(
        &mut self,
        uuid: u128,
        name: &str,
        until: Option<i64>,
        by: &str,
        reason: Option<String>,
    ) {
        let Some(guard) = self.chat_filter.as_deref_mut() else {
            return;
        };
        let now = now_millis();
        guard.mute(uuid, name, until, by, reason);
        let Some(text) = guard.muted(uuid, now).map(|m| muted_notice(m, now)) else {
            return;
        };
        for g in self.guests.iter().filter(|g| g.uuid == uuid) {
            g.send(&C::SystemChat(play::SystemChat {
                content: red(&text),
                overlay: false,
            }));
        }
    }

    /// The mutes in force, by name.
    pub fn mutes(&self) -> Vec<Mute> {
        self.chat_filter.as_deref().map_or_else(Vec::new, |g| {
            g.mutes(now_millis()).into_iter().cloned().collect()
        })
    }

    /// `/unmute <player>`: the name as it was muted, if they were.
    pub fn unmute(&mut self, name: &str) -> Option<String> {
        self.chat_filter.as_deref_mut()?.unmute(name)
    }
}
