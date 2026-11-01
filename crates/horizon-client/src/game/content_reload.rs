//! Rebuild immutable content registries at the title screen on the existing window.
use super::*;
impl Loading {
    pub(super) fn after_content_change(game: &mut Game<'_>) -> Result<Self> {
        let mut options = game.options.clone();
        options.world = None;
        options.server = None;
        let settings = game.settings.clone();
        let vanilla_root = game.vanilla_root();
        options.resources = vanilla_root.clone();
        let progress = Progress::new();
        let started = Instant::now();
        let (receiver, loader) =
            startup_menu::spawn_loader(&options, &settings, &vanilla_root, started, &progress)?;
        let mut menu = startup_menu::StartupMenu::new(&options, None)?;
        menu.resume_mod_options(game);
        Ok(Self {
            window: game.window.take(),
            renderer: game.renderer.take(),
            boot: horizon_client::hud::HudAtlas::boot(),
            gui: None,
            settings,
            fullscreen: game.fullscreen,
            video_modes: Default::default(),
            options: Some(options),
            vanilla_root,
            pack_error: None,
            started,
            panorama_rotation: game.panorama_rotation,
            progress,
            receiver,
            loaded: None,
            delivered: false,
            handle: Some(loader),
            shown: true,
            menu,
            error: None,
        })
    }
}
