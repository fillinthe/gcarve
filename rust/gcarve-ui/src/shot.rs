//! PNG of a captured frame (`png_bytes`, used by apps too), and the dev-only screenshot: <APP>_SHOT=out.png (GCARVE_SHOT, GSKETCH_SHOT) saves frame <APP>_SHOT_FRAME (default 6, after
//! layout has settled) and exits. Used for UI checks; never compiled into release builds without the `screenshot` feature.

use eframe::egui::{self, Event, UserData, ViewportCommand};

pub fn tick(ctx: &egui::Context, frame: u64) {
    let app = crate::store::app().to_uppercase();
    let Ok(path) = std::env::var(format!("{app}_SHOT")) else { return };
    let at: u64 = std::env::var(format!("{app}_SHOT_FRAME")).ok().and_then(|s| s.parse().ok()).unwrap_or(6);
    ctx.request_repaint(); // keep frames coming until the capture
    if frame == at {
        ctx.send_viewport_cmd(ViewportCommand::Screenshot(UserData::default()));
    }
    let shot = ctx.input(|i| i.events.iter().find_map(|e| if let Event::Screenshot { image, .. } = e { Some(image.clone()) } else { None }));
    if let Some(img) = shot {
        std::fs::write(&path, png_bytes(&img)).expect("screenshot path");
        ctx.send_viewport_cmd(ViewportCommand::Close);
    }
}

/// A captured frame (or part of one) as PNG bytes: the dev screenshot above and GSketch's File › Export Image.
pub fn png_bytes(img: &egui::ColorImage) -> Vec<u8> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, img.width() as u32, img.height() as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
    if let Ok(mut w) = enc.write_header() {
        let _ = w.write_image_data(&bytes);
    }
    out
}

/// A PNG decoded to (width, height, RGBA bytes): the window / taskbar icon at start-up (`ViewportBuilder::with_icon`).
pub fn png_rgba(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let mut reader = png::Decoder::new(bytes).read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width, info.height);
    let px = &buf[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => px.to_vec(),
        png::ColorType::Rgb => px.chunks(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => px.chunks(2).flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        png::ColorType::Grayscale => px.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return None,
    };
    (info.bit_depth == png::BitDepth::Eight).then_some((w, h, rgba))
}
