//! Who plays: the top bar's quick switch (a menu under the account chip) between the signed-in
//! Microsoft accounts.
use super::*;
use crate::servers_page::Field;

impl Launcher {
    /// The chip's two lines: the name that plays, and how.
    pub(super) fn who_plays(&self) -> (String, &'static str) {
        match self.accounts.player() {
            Player::Account(_) => (
                self.accounts
                    .selected()
                    .map_or_else(String::new, |a| a.name.clone()),
                tr(
                    if self
                        .accounts
                        .selected
                        .as_deref()
                        .is_some_and(|k| k.starts_with("ely:"))
                    {
                        "Ely.by account"
                    } else {
                        "Microsoft account"
                    },
                ),
            ),
            Player::Offline(name) => (name, tr("Offline")),
            Player::GameName => (
                self.offline_name.clone().unwrap_or_else(|| "Player".into()),
                tr("Offline"),
            ),
        }
    }

    pub(super) fn play_as(&mut self, who: &Player) {
        let mut accounts = Accounts::load();
        let done = accounts
            .play_as(account::store(), who.clone())
            .and_then(|()| accounts.save());
        match done {
            Ok(()) => {
                self.accounts = accounts;
                self.load_profile_heads();
            }
            Err(e) => self.sign_in_state = Some(SignInState::Done(Err(e))),
        }
        self.account_menu = false;
    }

    /// Everyone the player can play as, with how each signs in.
    pub(super) fn players(&self) -> Vec<(Player, String, &'static str)> {
        let mut out: Vec<_> = self
            .accounts
            .accounts
            .iter()
            .map(|a| {
                (
                    Player::Account(a.uuid.clone()),
                    a.name.clone(),
                    tr("Microsoft"),
                )
            })
            .collect();
        out.extend(self.accounts.ely.iter().map(|a| {
            (
                Player::Account(account::ely::key(&a.uuid)),
                a.name.clone(),
                tr("Ely.by"),
            )
        }));
        out.extend(
            self.accounts
                .offline
                .iter()
                .map(|n| (Player::Offline(n.clone()), n.clone(), tr("Offline"))),
        );
        out.push((
            Player::GameName,
            self.offline_name.clone().unwrap_or_else(|| "Player".into()),
            tr("Offline, the game's name"),
        ));
        out
    }

    /// The menu under the account chip, while open.
    pub(super) fn account_menu(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        if !self.account_menu {
            return None;
        }
        let t = self.theme;
        let now = self.accounts.player();
        let mut menu = div()
            .id("account-menu")
            .absolute()
            .top(px(46.))
            .right_0()
            .w(px(280.))
            .p(px(6.))
            .border_2()
            .border_color(t.outline)
            .bg(t.surface)
            .flex()
            .flex_col()
            .occlude()
            .on_mouse_down_out(cx.listener(|this, event: &MouseDownEvent, _, cx| {
                this.account_menu = false;
                this.menu_closed_at = Some(event.position);
                cx.notify();
            }));
        for (i, (who, name, how)) in self.players().into_iter().enumerate() {
            let chosen = who == now;
            menu = menu.child(
                div()
                    .id(SharedString::from(format!("account-pick-{i}")))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(8.))
                    .h(px(46.))
                    .px(px(10.))
                    .rounded(px(theme::RADIUS))
                    .cursor_pointer()
                    .hover(move |s| s.bg(t.plate_soft))
                    .child(self.profile_heads.draw(
                        match &who {
                            Player::Account(uuid) => Some(uuid),
                            _ => None,
                        },
                        24.,
                        self.scale,
                        &t,
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(text(theme::LABEL).truncate().text_color(t.ink).child(name))
                            .child(text(theme::CAPTION).text_color(t.muted).child(how)),
                    )
                    .when(chosen, |d| d.child(div().size(px(6.)).bg(t.accent)))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.play_as(&who);
                        cx.notify();
                    })),
            );
        }
        menu = menu.child(div().my(px(6.)).h(px(1.)).bg(t.rule)).child(
            div()
                .id("account-manage")
                .flex()
                .items_center()
                .h(px(32.))
                .px(px(10.))
                .rounded(px(theme::RADIUS))
                .text_size(px(theme::LABEL.px))
                .text_color(t.muted)
                .cursor_pointer()
                .hover(move |s| s.bg(t.plate_soft).text_color(t.ink))
                .child(tr("Manage accounts"))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.account_menu = false;
                    this.go(Page::Settings);
                    cx.notify();
                })),
        );
        Some(gpui::deferred(menu).with_priority(1))
    }

    fn add_offline_name(&mut self) {
        let name = self.servers.value(Field::OfflineName).trim().to_string();
        if name.is_empty() {
            return;
        }
        self.play_as(&Player::Offline(name));
        if !matches!(self.sign_in_state, Some(SignInState::Done(Err(_)))) {
            self.servers.text(Field::OfflineName).set(String::new());
        }
    }

    /// Forgets an offline name and its skin.
    pub(super) fn forget_offline(&mut self, name: &str) {
        let mut accounts = Accounts::load();
        if accounts
            .remove_offline(name, &horizon_launcher::skins::dir())
            .is_ok()
            && accounts.save().is_ok()
        {
            self.accounts = accounts;
        }
    }

    /// Enter in the offline name field.
    pub(super) fn offline_name_enter(&mut self) {
        self.add_offline_name();
    }
}
