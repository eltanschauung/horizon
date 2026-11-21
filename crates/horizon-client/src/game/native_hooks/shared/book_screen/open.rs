use super::*;

impl Game<'_> {
    /// The container slots (of `size`) that the mod's screen for the block at
    /// `pos` does not show: the server's menu does not have them either, so a
    /// shift-click never moves an item where the player cannot see it.
    pub(in crate::game) fn native_hidden_slots(&self, pos: Pos, size: usize) -> Vec<usize> {
        let Some(menu) = self.mod_menu_at(pos) else {
            return Vec::new();
        };
        let block = self
            .res
            .facts()
            .get(self.world.block(pos.0, pos.1, pos.2))
            .name
            .to_string();
        let mut screen = mod_api::RecipeBookLayout::registered_screen(menu);
        if screen.is_none() {
            self.native_screen(&block, &mut screen);
        }
        let Some(screen) = screen else {
            return Vec::new();
        };
        (0..size)
            .filter(|&i| screen.slots.get(i).is_none_or(Option::is_none))
            .collect()
    }
    /// A menu opened on this client with its slots under `key`, on block
    /// `target`: a book-enabled menu uses its startup-validated registered screen;
    /// other mod menus obtain their screen through `native_screen`.
    /// `extra` is what the server sent for this window first.
    pub(in crate::game) fn open_native_screen(
        &mut self,
        key: Pos,
        target: Option<Pos>,
        window: i32,
        menu: &str,
        extra: Option<horizon_protocol::menu_extra::Message>,
    ) {
        self.clear_mod_book_placement_screen();
        self.mod_screen = None;
        let blockless = crate::game::menu_screens::blockless_menu_screen(menu);
        let (block, screen) = if blockless.is_some() {
            (menu.to_owned(), blockless)
        } else if let Some(t) = target {
            let block = self
                .res
                .facts()
                .get(self.world.block(t.0, t.1, t.2))
                .name
                .to_string();
            if block.starts_with("minecraft:") {
                return;
            }
            // The last clicked block can outlive its menu. It may only supply a screen
            // for the menu it opens itself.
            let mut container = None;
            self.native_block_entity_container(&block, &mut container);
            let opens = container.and_then(|c| mod_menu(c.menu.as_deref()?).map(|(m, _)| m));
            if opens != Some(menu) {
                return;
            }
            let mut screen = mod_api::RecipeBookLayout::registered_screen(menu);
            if screen.is_none() {
                self.native_screen(&block, &mut screen);
            }
            (block, screen)
        } else {
            return;
        };
        if let Some(screen) = screen {
            self.mod_screen = Some(OpenNativeScreen {
                layout_revision: {
                    static NEXT: std::sync::atomic::AtomicU64 =
                        std::sync::atomic::AtomicU64::new(1);
                    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                },
                key,
                window,
                block,
                screen,
                data: Vec::new(),
                extra,
            });
            self.begin_mod_book_placement();
            self.ensure_mod_book_catalog();
        }
    }
}
