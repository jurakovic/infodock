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

/// The median of `times`, which must not be empty.
#[cfg(windows)]
pub fn median(times: &mut [std::time::Duration]) -> std::time::Duration {
    times.sort();
    times[times.len() / 2]
}

/// Records a measurement against its SPEC §14.1 budget in `target/budgets/<id>.json`. CI's
/// budget check puts it in the job summary and fails above the budget
/// (`.github/scripts/measure-memory.ps1`). The budget tests run on Windows only.
#[cfg(windows)]
pub fn record_budget(
    id: &str,
    label: &str,
    measured: std::time::Duration,
    budget: std::time::Duration,
    runs: usize,
) {
    let ms = |d: std::time::Duration| (d.as_secs_f64() * 1e6).round() / 1e3;
    println!(
        "{label}: {} ms, the median of {runs} runs (budget {} ms)",
        ms(measured),
        ms(budget)
    );
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/budgets");
    std::fs::create_dir_all(&dir).expect("budgets dir");
    let record = serde_json::json!({
        "label": label,
        "measuredMs": ms(measured),
        "budgetMs": ms(budget),
        "runs": runs,
    });
    std::fs::write(dir.join(format!("{id}.json")), record.to_string()).expect("budget file");
}
