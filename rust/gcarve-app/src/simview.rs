//! Simulate mode: the shaded cut as one texture drawn in board coordinates by the canvas. Filled by
//! playback::Player (incremental stamping); the canvas only reads it.

use eframe::egui::TextureHandle;

#[derive(Default)]
pub struct SimView {
    pub texture: Option<TextureHandle>,
    /// board-space rectangle the texture covers: x0, y0, width, height (inches)
    pub rect: Option<[f64; 4]>,
}
