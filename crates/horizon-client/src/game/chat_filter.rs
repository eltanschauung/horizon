//! Operator-issued mute commands. No automatic text moderation.
use super::{Game, dedicated, lan::LanHost};
use horizon_client::lan::hub::{HostEvent, chat_filter::Place};
use horizon_protocol::packets::play::{self, Clientbound as C};
use horizon_protocol::text::{literal, translatable};

/// The operator level the mute commands need, as `/ban`'s.
const MUTE_LEVEL: u8 = 3;

/// The commands of `mute_command`.
pub(super) const MUTE_COMMANDS: [&str; 4] = ["mute", "tempmute", "mutelist", "unmute"];

impl Game<'_> {
    pub(super) fn chat_filter_events(&mut self, _host: &mut LanHost, events: Vec<HostEvent>) {
        for event in events {
            self.lan_log_event(event);
        }
    }

    /// The message arguments of a guest's command (`/msg`, `/me`, `/say`, `/teammsg`...) through the
    /// filter: false when it refused them.
    pub(super) fn chat_filter_command(
        &mut self,
        host: &mut LanHost,
        i: usize,
        command: &str,
    ) -> bool {
        if !host.hub.chat_filter_on() {
            return true;
        }
        let permission = host.hub.guests[i].permission;
        let arguments = self.signable_arguments(permission, command);
        if arguments.is_empty() {
            return true;
        }
        let texts: Vec<&str> = arguments.iter().map(|(_, t)| t.as_str()).collect();
        let mut events = Vec::new();
        let allowed = host
            .hub
            .chat_allowed(i, &texts, Place::Command, &mut events);
        self.chat_filter_events(host, events);
        allowed
    }

    /// A guest's text for a sign, an anvil or a book through the filter (`conn`'s guest).
    pub(super) fn chat_filter_text(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        texts: &[&str],
        place: Place,
    ) -> bool {
        if !host.hub.chat_filter_on() {
            return true;
        }
        let Some(i) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return true;
        };
        let mut events = Vec::new();
        let allowed = host.hub.chat_allowed(i, texts, place, &mut events);
        self.chat_filter_events(host, events);
        allowed
    }

    /// A mute command from a guest: operators of level 3 (`/ban`'s); vanilla's unknown command for
    /// the others.
    pub(super) fn guest_mute_command(&mut self, host: &mut LanHost, conn: u32, words: &[&str]) {
        let Some(g) = host.hub.guest(conn) else {
            return;
        };
        let (permission, by) = (g.permission, g.name.clone());
        let replies = if permission < MUTE_LEVEL {
            vec![translatable("command.unknown.command", &[])]
        } else {
            let words: Vec<&str> = words.iter().copied().filter(|w| !w.is_empty()).collect();
            self.mute_command(host, &words, &by)
                .iter()
                .map(|line| literal(line))
                .collect()
        };
        for content in replies {
            host.hub.send_to(
                conn,
                &C::SystemChat(play::SystemChat {
                    content,
                    overlay: false,
                }),
            );
        }
    }

    /// `/mute <player> [reason]`, `/tempmute <player> <duration> [reason]`, `/mutelist` and
    /// `/unmute <player>` (`words`, the command first) by `by`: the lines to answer.
    pub(super) fn mute_command(
        &mut self,
        host: &mut LanHost,
        words: &[&str],
        by: &str,
    ) -> Vec<String> {
        let reason = |from: usize| (words.len() > from).then(|| words[from..].join(" "));
        match words {
            ["mutelist"] => {
                let now = horizon_chat_filter::now_millis();
                let mutes = host.hub.mutes();
                if mutes.is_empty() {
                    return vec!["There are no mutes".into()];
                }
                let mut lines = vec![format!("There are {} mute(s):", mutes.len())];
                for m in mutes {
                    let by = m.by.as_deref().unwrap_or("the chat filter");
                    let until = m.until.map_or_else(
                        || "until unmuted".into(),
                        |u| {
                            let left = horizon_chat_filter::duration_text(u - now);
                            format!("for {}", left.replacen(' ', " more ", 1))
                        },
                    );
                    let why = m.reason.map(|r| format!(": {r}")).unwrap_or_default();
                    lines.push(format!("{} was muted by {by} {until}{why}", m.name));
                }
                lines
            }
            ["unmute", name] => vec![self.unmute(host, name, by)],
            ["mute", name, ..] => vec![self.mute(host, name, None, reason(2), by)],
            ["tempmute", name, time, ..] => match horizon_chat_filter::parse_duration(time) {
                Some(ms) => vec![self.mute(host, name, Some(ms), reason(3), by)],
                None => vec![format!("{time} isn't a duration: 10m, 2h, 1d")],
            },
            ["mute", ..] => vec!["/mute <player> [reason]".into()],
            ["tempmute", ..] => vec!["/tempmute <player> <duration> [reason]".into()],
            ["unmute", ..] => vec!["/unmute <player>".into()],
            _ => vec!["/mutelist".into()],
        }
    }

    /// Mutes `name` for `time` milliseconds (`None`: until `/unmute`), logged; what to answer.
    fn mute(
        &mut self,
        host: &mut LanHost,
        name: &str,
        time: Option<i64>,
        reason: Option<String>,
        by: &str,
    ) -> String {
        // A player here, else one the server has seen (the user cache).
        let user = host
            .hub
            .guests
            .iter()
            .find(|g| g.name.eq_ignore_ascii_case(name))
            .map(|g| (g.uuid, g.name.clone()))
            .or_else(|| self.resolve_name(name).map(|u| (u.id, u.name)));
        let Some((uuid, name)) = user else {
            return format!("No player was found named {name}");
        };
        let until = time.map(|ms| horizon_chat_filter::now_millis() + ms);
        let what = time.map_or_else(
            || "until unmuted".into(),
            |ms| format!("for {}", horizon_chat_filter::duration_text(ms)),
        );
        let why = reason
            .as_deref()
            .map(|r| format!(": {r}"))
            .unwrap_or_default();
        host.hub.mute(uuid, &name, until, by, reason);
        if self.options.dedicated.is_some() {
            dedicated::log(
                "INFO",
                &format!("[chat-filter] {by} muted {name} {what}{why}"),
            );
        }
        format!("Muted {name} {what}{why}")
    }

    /// Lifts `name`'s mute, logged; what to answer.
    pub(super) fn unmute(&mut self, host: &mut LanHost, name: &str, by: &str) -> String {
        match host.hub.unmute(name) {
            Some(name) => {
                if self.options.dedicated.is_some() {
                    dedicated::log("INFO", &format!("[chat-filter] {by} unmuted {name}"));
                }
                format!("Unmuted {name}")
            }
            None => format!("{name} isn't muted"),
        }
    }
}
