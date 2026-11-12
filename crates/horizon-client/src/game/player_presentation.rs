//! Viewer-dependent player presentation, shared by body, equipment and labels.
use super::Game;
use horizon_client::{mobs::Mob, mesher::Vertex};
impl Game<'_> {
    pub(super) fn player_presentation_opacity(&self, _mob: &Mob, _camera: glam::DVec3) -> f32 { 1.0 }
    pub(super) fn player_sensory_visible(&self, _uuid: u128) -> bool { true }
}
pub(super) fn fade_vertices(vertices: &mut [Vertex], opacity: f32) {
    let opacity = if opacity.is_finite() { opacity.clamp(0.0, 1.0) } else { 1.0 };
    if opacity == 1.0 { return; }
    for v in vertices { let a = ((v.color >> 24) as f32 * opacity).round() as u32; v.color = (v.color & 0x00ff_ffff) | (a << 24); }
}
