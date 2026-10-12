//! Helpers shared by tests.

use std::path::PathBuf;

/// `target/screenshots/<name>.png`, creating the folder. CI uploads the folder (SPEC §16).
pub fn screenshot_path(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/screenshots");
    std::fs::create_dir_all(&dir).expect("screenshot dir");
    dir.join(format!("{name}.png"))
}

/// Writes straight (not premultiplied) RGBA rows as a PNG file.
pub fn write_png(path: &std::path::Path, width: u32, height: u32, rgba: &[u8]) {
    let file = std::fs::File::create(path).expect("screenshot file");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .and_then(|mut w| w.write_image_data(rgba))
        .expect("PNG written");
}
