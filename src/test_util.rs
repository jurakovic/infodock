//! Helpers shared by tests.

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::config::model::DateLocale;
use crate::platform::time::{Date, LocalTime};
use crate::platform::{
    Bitmap, Capabilities, EjectReport, FileOp, KnownFolder, Launch, Locale, MonitorInfo,
    NetInterface, OsAppearance, Platform, PowerAction, Volume,
};

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

/// The en-US locale, for contexts in tests.
pub fn en_us() -> &'static Locale {
    static EN_US: std::sync::LazyLock<Locale> = std::sync::LazyLock::new(Locale::en_us);
    &EN_US
}

/// One platform for the tests (SPEC §16), the same on every OS: no monitors, the default
/// appearance, no uptime, a settable locale (en-US) and clock (2026-10-13, a Tuesday). It
/// answers with what a test gave it (capabilities, volumes, connections, shell icons, …) and
/// records what it is asked to do in [`FakePlatform::log`] instead of doing it, so tests that
/// click, drop or eject run on Windows too.
pub struct FakePlatform {
    capabilities: Capabilities,
    /// The capability probe has not answered until [`FakePlatform::answer_probe`].
    slow_probe: bool,
    probed: AtomicBool,
    processors: Option<u32>,
    volumes: Vec<Volume>,
    connections: Vec<NetInterface>,
    /// Paths with a shell icon.
    shell_icons: Vec<String>,
    /// `volume_of` knows no volumes, so every drop copies.
    no_volumes: bool,
    time: Mutex<Option<LocalTime>>,
    /// The locale for every `DateLocale`; en-US until set.
    locale: Mutex<Option<Locale>>,
    log: Mutex<Vec<String>>,
}

/// A platform with nothing set, for tests that only read from it (machine defaults, the legacy
/// import), e.g. in a `static` context.
pub static NO_PLATFORM: FakePlatform = FakePlatform::new();

impl Default for FakePlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl FakePlatform {
    pub const fn new() -> Self {
        Self {
            capabilities: Capabilities {
                appbar: false,
                recycle_bin: false,
                start_menu: false,
                wifi: false,
                battery: false,
                power_modes: Vec::new(),
                eject: false,
                per_core_cpu: false,
            },
            slow_probe: false,
            probed: AtomicBool::new(false),
            processors: None,
            volumes: Vec::new(),
            connections: Vec::new(),
            shell_icons: Vec::new(),
            no_volumes: false,
            time: Mutex::new(None),
            locale: Mutex::new(None),
            log: Mutex::new(Vec::new()),
        }
    }

    pub fn with_capabilities(mut self, capabilities: Capabilities) -> Self {
        self.capabilities = capabilities;
        self
    }

    /// The capabilities are not known until [`FakePlatform::answer_probe`] (SPEC §14.1).
    pub fn with_slow_probe(mut self) -> Self {
        self.slow_probe = true;
        self
    }

    pub fn with_processors(mut self, processors: u32) -> Self {
        self.processors = Some(processors);
        self
    }

    pub fn with_volumes(mut self, volumes: Vec<Volume>) -> Self {
        self.volumes = volumes;
        self
    }

    pub fn with_connections(mut self, connections: Vec<NetInterface>) -> Self {
        self.connections = connections;
        self
    }

    /// `path` has a white shell icon.
    pub fn with_shell_icon(mut self, path: &str) -> Self {
        self.shell_icons.push(path.to_owned());
        self
    }

    /// No path lies on a known volume, so drops copy rather than move (SPEC §9.19).
    pub fn without_volumes(mut self) -> Self {
        self.no_volumes = true;
        self
    }

    pub fn answer_probe(&self) {
        self.probed.store(true, Ordering::SeqCst);
    }

    /// Sets the clock to `hour:minute:second` on 2026-10-13.
    pub fn set_time(&self, hour: u8, minute: u8, second: u8) {
        let date = Date::new(2026, 10, 13).expect("a date");
        *lock(&self.time) = Some(LocalTime {
            date,
            hour,
            minute,
            second,
            millis: 0,
        });
    }

    /// From now on the locale is `locale`, as when the OS regional format changes.
    pub fn set_locale(&self, locale: Locale) {
        *lock(&self.locale) = Some(locale);
    }

    /// What it was asked to do, in order.
    pub fn log(&self) -> Vec<String> {
        lock(&self.log).clone()
    }

    fn record(&self, entry: String) {
        lock(&self.log).push(entry);
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Platform for FakePlatform {
    fn capabilities(&self) -> Capabilities {
        self.capabilities.clone()
    }
    fn probed_capabilities(&self) -> Option<Capabilities> {
        (!self.slow_probe || self.probed.load(Ordering::SeqCst)).then(|| self.capabilities())
    }
    fn monitors(&self) -> Vec<MonitorInfo> {
        Vec::new()
    }
    fn appearance(&self) -> OsAppearance {
        OsAppearance::default()
    }
    fn uptime(&self) -> Duration {
        Duration::ZERO
    }
    fn logical_processors(&self) -> u32 {
        self.processors
            .unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get() as u32))
    }
    fn volumes(&self) -> Vec<Volume> {
        self.volumes.clone()
    }
    fn net_interfaces(&self) -> Vec<NetInterface> {
        self.connections.clone()
    }
    /// The set time, or else now in UTC.
    fn local_time(&self) -> LocalTime {
        lock(&self.time).unwrap_or_else(|| {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default();
            LocalTime::from_unix(now.as_secs() as i64, now.subsec_millis() as u16)
        })
    }
    fn locale(&self, _which: DateLocale) -> Locale {
        lock(&self.locale).clone().unwrap_or_else(Locale::en_us)
    }
    fn shell_icon(&self, path: &str, px: u32) -> Option<Bitmap> {
        self.shell_icons.iter().any(|p| p == path).then(|| {
            Bitmap::from_rgba(px, px, vec![255; (px * px * 4) as usize]).expect("a bitmap")
        })
    }
    fn volume_of(&self, path: &str) -> Option<String> {
        if self.no_volumes {
            None
        } else {
            crate::platform::path_root(path)
        }
    }
    fn known_folder(&self, folder: KnownFolder) -> Option<String> {
        Some(match folder {
            KnownFolder::Programs => "C:\\Users\\me\\Programs".into(),
            KnownFolder::CommonPrograms => "C:\\ProgramData\\Programs".into(),
        })
    }
    fn play_sound(&self, wav: &'static [u8]) {
        self.record(format!("play {}", wav.len()));
    }
    fn open(&self, target: &str) {
        self.record(format!("open {target}"));
    }
    fn launch(&self, launch: &Launch) {
        self.record(format!(
            "launch {} args={} dir={} admin={}",
            launch.target,
            launch.args,
            launch.dir.unwrap_or_default(),
            launch.admin
        ));
    }
    fn reveal(&self, path: &str) {
        self.record(format!("reveal {path}"));
    }
    fn eject(&self, root: &str, report: EjectReport) {
        let (title, body) = report(&Ok(()));
        self.record(format!("eject {root}: {title}: {body}"));
    }
    fn file_operation(
        &self,
        op: FileOp,
        paths: Vec<String>,
        folder: String,
        done: Box<dyn FnOnce() + Send>,
    ) {
        self.record(format!("{op:?} {} into {folder}", paths.join(", ")));
        done();
    }
    fn recycle(&self, paths: Vec<String>, done: Box<dyn FnOnce() + Send>) {
        self.record(format!("recycle {}", paths.join(", ")));
        done();
    }
    fn empty_recycle_bin(&self, done: Box<dyn FnOnce() + Send>) {
        self.record("empty recycle bin".into());
        done();
    }
    fn create_url_file(&self, folder: &str, stem: &str, url: &str) {
        self.record(format!("url file {folder}\\{stem}.url {url}"));
    }
    fn power(&self, action: PowerAction, force: bool) {
        self.record(format!("{action:?} force={force}"));
    }
    fn notify(&self, title: &str, body: &str) {
        self.record(format!("notify {title}: {body}"));
    }
}
