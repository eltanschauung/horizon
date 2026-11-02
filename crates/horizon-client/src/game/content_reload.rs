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
        // The old renderer has the full game's atlas. Boot-screen UVs must
        // only be drawn with the boot atlas uploaded to the renderer.
        let boot = horizon_client::hud::HudAtlas::boot();
        let window = game
            .window
            .as_ref()
            .ok_or("no window for content reload")?
            .clone();
        if let Some(renderer) = &mut game.renderer {
            renderer.release_surface();
        }
        let renderer = loading_renderer(
            window,
            options.offscreen,
            Some(&boot),
            None,
            game.renderer.as_ref(),
            settings.horizon.api,
        )?;
        horizon_client::ui_theme::reset_atlas();
        Ok(Self {
            window: game.window.take(),
            renderer: Some(renderer),
            boot,
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
            ui_at: None,
            title_drawn: false,
            menu,
            error: None,
        })
    }
}
