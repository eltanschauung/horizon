//! Ely.by account controls share the launcher's normal text fields and account switcher.
use super::*;
use servers_page::Field;
impl Launcher {
    pub(super) fn ely_panel(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let busy = self.ely_busy;
        let mut panel = div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .p(px(16.))
            .child("Ely.by");
        for a in self.accounts.ely.clone() {
            let key = account::ely::key(&a.uuid);
            let selected = self.accounts.selected.as_deref() == Some(&key);
            let play_key = key.clone();
            let remove_key = key.clone();
            let play = button(
                format!("ely-play-{key}"),
                if selected { "Selected" } else { "Play as" },
                Kind::Secondary,
                &t,
            );
            let play = if selected || busy {
                disabled(play)
            } else {
                play.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.play_as(&Player::Account(play_key.clone()));
                    cx.notify();
                }))
            };
            let remove = button(format!("ely-remove-{key}"), "Sign out", Kind::Ghost, &t);
            let remove = if busy {
                disabled(remove)
            } else {
                remove.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.ely_sign_out(remove_key.clone());
                    cx.notify();
                }))
            };
            panel = panel.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().flex_1().child(a.name))
                    .child(play)
                    .child(remove),
            );
        }
        panel = panel
            .child(self.field(Field::ElyLogin, "Email or username", 420., cx))
            .child(self.field(Field::ElyPassword, "Password", 420., cx))
            .child(self.field(Field::ElyCode, "Two-factor code (optional)", 420., cx));
        let sign = button(
            "ely-sign-in",
            if busy {
                "Signing in..."
            } else {
                "Sign in with Ely.by"
            },
            Kind::Primary,
            &t,
        );
        let sign = if busy {
            disabled(sign)
        } else {
            sign.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.ely_sign_in();
                cx.notify();
            }))
        };
        panel.child(div().flex().gap(px(12.)).child(sign).child(button("ely-register","Create an Ely.by account",Kind::Ghost,&t).on_click(|_,_,cx|cx.open_url("https://account.ely.by/register"))))
            .children(self.ely_error.clone().map(|e|div().text_color(t.danger).child(e)))
            .child(div().text_size(px(12.)).child("Password is never saved. Ely.by does not provide a Minecraft license; Minecraft downloads still require ownership."))
    }
    pub(super) fn ely_sign_in(&mut self) {
        if self.ely_busy {
            return;
        }
        let login = self.servers.value(Field::ElyLogin).to_owned();
        let password = self.servers.value(Field::ElyPassword).to_owned();
        let otp = self.servers.value(Field::ElyCode).to_owned();
        self.ely_busy = true;
        self.ely_error = None;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = account::ely::sign_in(&login, &password, &otp).map(|_| ());
            let _ = tx.send(Msg::ElyAccount(result));
        });
    }
    fn ely_sign_out(&mut self, key: String) {
        if self.ely_busy {
            return;
        }
        self.ely_busy = true;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = account::ely::sign_out(&key);
            let _ = tx.send(Msg::ElyAccount(result));
        });
    }
}
