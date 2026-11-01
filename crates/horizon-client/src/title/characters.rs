//! Character Select uses the prepared player model, with no world or player.
use super::{Button, centered, wrapped};
use crate::{
    characters::{self, Character},
    hud::{HudAtlas, HudBatch},
    mobs::{EntityModelDocument, Part, anim::Poses, render},
};
use std::path::{Path, PathBuf};

pub(super) struct Screen {
    pub roster: Vec<Character>,
    pub selected: Option<usize>,
    pub preview: usize,
    page: usize,
    model: Option<Part>,
    slim_model: Option<Part>,
    instance: PathBuf,
    error: Option<String>,
}

impl Screen {
    pub fn new(root: &Path, instance: PathBuf) -> Self {
        let roster = characters::roster(root);
        let selected = characters::selected(root, &instance)
            .and_then(|choice| roster.iter().position(|c| *c == choice));
        Self {
            roster,
            selected,
            preview: selected.unwrap_or(0),
            page: 0,
            model: None,
            slim_model: None,
            instance,
            error: None,
        }
    }

    pub fn open(&mut self, root: &Path, h: f32) {
        self.preview = self.selected.unwrap_or(0);
        self.page = self.preview / Self::page_size(h);
        self.error = None;
        if self.model.is_none()
            && let Ok(models) = EntityModelDocument::load(root)
        {
            self.model = models.model("minecraft:player#main");
            self.slim_model = models.model("minecraft:player_slim#main");
        }
    }

    fn page_size(h: f32) -> usize {
        (((h - 104.0) / 22.0).floor() as usize).clamp(1, 12)
    }

    fn page(&self, h: f32) -> usize {
        self.page
            .min(self.roster.len().saturating_sub(1) / Self::page_size(h))
    }

    pub fn selected_character(&self) -> Option<Character> {
        self.selected.and_then(|i| self.roster.get(i)).cloned()
    }

    pub fn buttons(&self, w: f32, h: f32) -> Vec<Button> {
        let left = (w / 2.0 - 194.0).max(12.0).floor();
        let width = (w / 2.0 - left - 6.0).floor();
        let button = |x, y, w, label: String, enabled, id| Button {
            x,
            y,
            w,
            label,
            enabled,
            id,
            icon: None,
            trailing: None,
        };
        let size = Self::page_size(h);
        let start = self.page(h) * size;
        let mut buttons: Vec<_> = self
            .roster
            .iter()
            .enumerate()
            .skip(start)
            .take(size)
            .map(|(i, character)| {
                button(
                    left,
                    46.0 + (i - start) as f32 * 22.0,
                    width,
                    character.name.clone(),
                    true,
                    10 + (i - start) as u8,
                )
            })
            .collect();
        buttons.extend([
            button(
                left,
                h - 52.0,
                (width - 4.0) / 2.0,
                "Previous".into(),
                self.page(h) > 0,
                200,
            ),
            button(
                left + (width + 4.0) / 2.0,
                h - 52.0,
                (width - 4.0) / 2.0,
                "Next".into(),
                start + size < self.roster.len(),
                201,
            ),
            button(
                w / 2.0 - 102.0,
                h - 26.0,
                100.0,
                "Select Character".into(),
                !self.roster.is_empty(),
                202,
            ),
            button(w / 2.0 + 2.0, h - 26.0, 100.0, "Back".into(), true, 203),
        ]);
        buttons
    }

    pub fn click(&mut self, id: u8, h: f32) -> Option<Character> {
        let size = Self::page_size(h);
        self.page = self.page(h);
        match id {
            10..=21 => {
                let i = self.page * size + usize::from(id - 10);
                if i < self.roster.len() {
                    self.preview = i;
                    self.error = None;
                }
            }
            200 if self.page > 0 => self.page -= 1,
            201 if (self.page + 1) * size < self.roster.len() => self.page += 1,
            202 => {
                let character = self.roster.get(self.preview)?;
                match characters::save(&self.instance, character) {
                    Ok(()) => {
                        self.selected = Some(self.preview);
                        return Some(character.clone());
                    }
                    Err(_) => {
                        self.error = Some("Could not save your character. Please try again.".into())
                    }
                }
            }
            _ => {}
        }
        None
    }

    pub fn draw(&self, atlas: &HudAtlas, batch: &mut HudBatch, cursor: (f32, f32), seconds: f32) {
        let (w, h) = (batch.width.ceil(), batch.height.ceil());
        crate::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false);
        centered(atlas, batch, "Character Select", 12.0, u32::MAX);
        centered(
            atlas,
            batch,
            "Choose your appearance, then select your character.",
            28.0,
            0xffaaaaaa,
        );
        for button in self.buttons(w, h) {
            if button.id < 200
                && self.page(h) * Self::page_size(h) + usize::from(button.id - 10) == self.preview
            {
                batch.fill(
                    atlas,
                    button.x - 1.0,
                    button.y - 1.0,
                    button.w + 2.0,
                    22.0,
                    0xff55ffff,
                );
            }
        }
        let (left, right, top, bottom) = (
            w / 2.0 + 8.0,
            (w / 2.0 + 194.0).min(w - 12.0),
            46.0,
            h - 58.0,
        );
        batch.fill(atlas, left, top, right - left, bottom - top, 0x70000000);
        if let Some(character) = self.roster.get(self.preview) {
            let cx = (left + right) / 2.0;
            let lines = wrapped(atlas, &character.name, right - left - 12.0);
            for (i, line) in lines.iter().enumerate() {
                batch.text(
                    atlas,
                    line,
                    cx - HudBatch::text_width(atlas, line) / 2.0,
                    top + 7.0 + i as f32 * 10.0,
                    u32::MAX,
                    true,
                );
            }
            let model = if character.is_slim(atlas) {
                self.slim_model.as_ref().or(self.model.as_ref())
            } else {
                self.model.as_ref()
            };
            if let Some(model) = model {
                let bounds = [
                    left,
                    top + 12.0 + lines.len() as f32 * 10.0,
                    right,
                    bottom - 6.0,
                ];
                draw_preview(
                    model,
                    character.skin(atlas),
                    atlas,
                    batch,
                    bounds,
                    cursor,
                    seconds,
                );
            }
        } else {
            centered(
                atlas,
                batch,
                "No characters available. Enable Lunatic and restart.",
                h / 2.0,
                0xffaaaaaa,
            );
        }
        if let Some(error) = &self.error {
            centered(atlas, batch, error, h - 64.0, 0xff5555ff);
        }
    }
}

fn draw_preview(
    model: &Part,
    skin: &str,
    atlas: &HudAtlas,
    batch: &mut HudBatch,
    bounds: [f32; 4],
    cursor: (f32, f32),
    seconds: f32,
) {
    let Some(texture) = atlas.sprites.get(skin) else {
        return;
    };
    let [left, top, right, bottom] = bounds;
    let cx = (left + right) / 2.0;
    let mut poses = Poses::new(model);
    let age = seconds * 20.0;
    for (arm, sign) in [("right_arm", 1.0), ("left_arm", -1.0)] {
        poses.zrot(arm, sign * ((age * 0.09).cos() * 0.05 + 0.05));
        poses.xrot(arm, sign * (age * 0.067).sin() * 0.05);
    }
    let yaw = ((cursor.0 - cx) / 120.0).clamp(-0.4, 0.4);
    poses.yrot("head", yaw);
    poses.xrot(
        "head",
        ((cursor.1 - (top + bottom) / 2.0) / 160.0).clamp(-0.2, 0.2),
    );
    let size = ((bottom - top) / 2.2).min((right - left) / 1.8);
    let transform = glam::Mat4::from_translation(glam::Vec3::new(cx, bottom - 2.0, 0.0))
        * glam::Mat4::from_scale(glam::Vec3::new(size, size, -size))
        * glam::Mat4::from_rotation_x(-0.08)
        * glam::Mat4::from_rotation_y(-0.35)
        * glam::Mat4::from_translation(glam::Vec3::new(0.0, -1.5, 0.0));
    let mut vertices = Vec::new();
    render::draw(
        model,
        transform,
        &poses,
        [texture.u0, texture.v0, texture.u1, texture.v1],
        u32::MAX,
        240 | 240 << 8,
        0,
        &mut vertices,
    );
    let start = batch.vertices.len();
    // Real model faces, projected into the UI's depth buffer; no screenshot or flat skin card.
    for quad in vertices.as_chunks::<4>().0 {
        let points = quad.map(|v| glam::Vec3::from_array(v.pos));
        let normal = (points[1] - points[0])
            .cross(points[2] - points[0])
            .normalize_or_zero();
        if normal.z >= 0.0 {
            continue;
        }
        let shade = (0.65 + 0.35 * (-normal.z)).clamp(0.0, 1.0);
        let color = (255.0 * shade) as u32;
        batch.quad_3d(
            quad.map(|v| v.pos),
            quad.map(|v| v.uv),
            0xff000000 | (color * 0x010101),
        );
    }
    batch.depth_since(start, bounds);
}
