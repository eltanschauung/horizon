use super::*;
#[test]
fn only_operator_mutes_survive_loading() {
    let dir = std::env::temp_dir().join(format!("manual-mutes-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(MUTES_FILE), r#"{"mutes":[{"uuid":"00000000000000000000000000000001","name":"automatic","until":null},{"uuid":"00000000000000000000000000000002","name":"manual","until":null,"by":"Op"}]}"#).unwrap();
    let (mut g, _) = Guard::load(Config::default(), &dir);
    assert!(g.muted(1, now_millis()).is_none());
    assert!(g.muted(2, now_millis()).is_some());
    g.mute(3, "new", None, "Op", None);
    let (g, _) = Guard::load(Config::default(), &dir);
    assert!(g.muted(3, now_millis()).is_some());
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn old_filter_configuration_cannot_enable_automatic_punishments() {
    let c = Config::from_server_toml(
        "[chat_filter]\nenabled=true\nmute_after=1\ntempban_after=1\nwords_file=\"anything\"\n",
    )
    .unwrap();
    assert_eq!(c, Config::default());
    assert_eq!(parse_duration("2h"), Some(7_200_000));
}
