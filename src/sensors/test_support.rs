//! Helpers for the sensor types' tests: one [`Harness`] (the fake platform, default settings,
//! the default modern theme, fixed text metrics at scale 1), props resolved from a type's
//! schema, and small sensor types and icon sources.

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::schema::{FieldKind, Lit, PropField, resolve};
use super::{
    Category, DefaultsCtx, DrawCtx, Fetched, IconSource, LayoutCtx, NoIcons, Props, Sensor,
    SensorCtx, SensorType, State, legacy,
};
use crate::config::model::Settings;
use crate::look::theme::Theme;
use crate::platform::recording::{FixedMetrics, RecordingCanvas};
use crate::platform::{Bitmap, Canvas, OsAppearance, Platform, RectF, TextEngine, TextStyle};
use crate::sampler::{Change, ChangeSink};
use crate::test_util::{FakePlatform, en_us};

/// What a sensor runs with in the tests: the fake platform, default settings and the default
/// modern theme; the changes its actions report are kept (SPEC §4.4).
pub struct Harness {
    pub platform: FakePlatform,
    pub settings: Settings,
    pub theme: Theme,
    changes: ChangeSink,
    reported: Arc<Mutex<Vec<Change>>>,
}

impl Harness {
    pub fn new() -> Self {
        Self::with_platform(FakePlatform::default())
    }

    pub fn with_platform(platform: FakePlatform) -> Self {
        let reported = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&reported);
        Self {
            platform,
            settings: Settings::default(),
            theme: theme(),
            changes: ChangeSink::new(move |c| sink.lock().expect("lock").push(c)),
            reported,
        }
    }

    /// The sensor context, without Alt and without animations.
    pub fn ctx(&self) -> SensorCtx<'_> {
        SensorCtx {
            settings: &self.settings,
            locked: self.settings.locked,
            alt: false,
            animate: false,
            locale: en_us(),
            numbers: Default::default(),
            platform: &self.platform,
            changes: &self.changes,
        }
    }

    /// The measuring context: fixed text metrics, scale 1, no shell icons.
    pub fn layout(&self) -> LayoutCtx<'_> {
        LayoutCtx {
            theme: &self.theme,
            text: &FixedMetrics,
            scale: 1.0,
            locale: en_us(),
            numbers: Default::default(),
            icons: &NoIcons,
        }
    }

    /// Measures `sensor` and draws it at x = 0 on a 28 px bar: its width and what it drew.
    pub fn drawn(&self, sensor: &mut dyn Sensor) -> (f32, String) {
        let width = sensor.measure(&self.layout());
        let mut canvas = RecordingCanvas::new(1.0);
        let rect = RectF::new(0.0, 0.0, width, 28.0);
        sensor.draw(
            &mut canvas,
            &DrawCtx {
                theme: &self.theme,
                rect,
            },
        );
        (width, canvas.dump())
    }

    /// What the sensor's actions reported changed, in order.
    pub fn reported(&self) -> Vec<Change> {
        self.reported.lock().expect("lock").clone()
    }
}

/// `stored` props (a JSON object) resolved by `schema`, every stored value taken.
pub fn resolved(schema: &[PropField], stored: Value) -> Props {
    let stored = stored.as_object().cloned().unwrap_or_default();
    let (props, ignored) = resolve(schema, &stored, &Props::new());
    assert!(ignored.is_empty(), "{ignored:?}");
    props
}

/// A sensor of `ty` for `stored` props.
pub fn created(ty: &dyn SensorType, stored: Value) -> Box<dyn Sensor> {
    ty.create(&resolved(ty.schema(), stored), &State::new())
        .expect("valid props")
}

/// Draws its text as the label, so tests can see what it was created with.
pub struct Echo(pub String);

impl Sensor for Echo {
    fn measure(&mut self, _cx: &LayoutCtx) -> f32 {
        10.0
    }

    fn draw(&self, canvas: &mut dyn Canvas, cx: &DrawCtx) {
        let layout = FixedMetrics.layout(&self.0, &TextStyle::new(&cx.theme.sensor_font, 10.0));
        canvas.draw_text(&layout, Default::default(), cx.theme.sensor_text);
    }
}

const CLOCK_SCHEMA: &[PropField] = &[
    PropField::new("use24h", FieldKind::Bool, "x", Lit::Bool(true)).machine(),
    PropField::new("showSeconds", FieldKind::Bool, "x", Lit::Bool(false)),
];

/// A type with a machine-derived field: `use24h` defaults to `true`, the machine says
/// `false`. It draws its props.
pub struct Clock;

impl SensorType for Clock {
    fn id(&self) -> &'static str {
        "clock"
    }
    fn category(&self) -> Category {
        Category::Info
    }
    fn name(&self) -> &'static str {
        "test.clock"
    }
    fn description(&self) -> &'static str {
        "test.clock.description"
    }
    fn schema(&self) -> &'static [PropField] {
        CLOCK_SCHEMA
    }
    fn machine_defaults(&self, _ctx: &DefaultsCtx) -> Props {
        MACHINE_READS.with(|n| n.set(n.get() + 1));
        let mut p = Props::new();
        p.insert("use24h".into(), json!(false));
        p
    }
    fn create(&self, props: &Props, _state: &State) -> serde_json::Result<Box<dyn Sensor>> {
        Ok(Box::new(Echo(format!(
            "use24h={} showSeconds={}",
            props["use24h"], props["showSeconds"]
        ))))
    }
}

thread_local! {
    /// How often this thread asked [`Clock`] for its machine defaults.
    static MACHINE_READS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

pub fn machine_reads() -> u32 {
    MACHINE_READS.with(std::cell::Cell::get)
}

pub fn theme() -> Theme {
    Theme::resolve(&Settings::default(), OsAppearance::default())
}

/// What `ty` makes of a legacy sensor of type `legacy_type` with the properties `pairs`
/// on `platform`: its props (not yet checked by the import), state and warnings.
pub fn convert_legacy(
    ty: &dyn SensorType,
    platform: &dyn Platform,
    legacy_type: &str,
    pairs: &[(&str, &str)],
) -> (Props, State, Vec<String>) {
    let raw = pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    let defaults = DefaultsCtx { platform };
    let mut cx = legacy::LegacyCtx {
        legacy_type,
        defaults: &defaults,
        warnings: Vec::new(),
        state: State::new(),
    };
    let props = ty.convert_legacy(&raw, &mut cx);
    let warnings = cx.warnings.iter().map(|w| w.message()).collect();
    (props, cx.state, warnings)
}

pub fn measure(sensor: &mut dyn Sensor) -> f32 {
    measure_with(sensor, &NoIcons)
}

/// Measures `sensor` with its shell icons and image files from `icons`.
pub fn measure_with(sensor: &mut dyn Sensor, icons: &dyn IconSource) -> f32 {
    let h = Harness::new();
    sensor.measure(&LayoutCtx {
        icons,
        ..h.layout()
    })
}

/// Image files as the bar reads them, off the bar thread: loading until the test says it
/// arrived (or failed); no shell icons.
#[derive(Default)]
pub struct Later {
    image: Mutex<Option<Fetched>>,
    asked: Mutex<Vec<(String, u32)>>,
}

impl Later {
    pub fn arrive(&self, bitmap: Bitmap) {
        *self.image.lock().expect("lock") = Some(Fetched::Ready(bitmap));
    }

    pub fn fail(&self) {
        *self.image.lock().expect("lock") = Some(Fetched::None);
    }

    /// The images asked for, by path and height.
    pub fn asked(&self) -> Vec<(String, u32)> {
        self.asked.lock().expect("lock").clone()
    }
}

impl IconSource for Later {
    fn shell_icon(&self, _path: &str, _px: u32) -> Fetched {
        Fetched::None
    }

    fn image(&self, path: &str, px: u32) -> Fetched {
        let mut asked = self.asked.lock().expect("lock");
        if !asked.iter().any(|(p, h)| p == path && *h == px) {
            asked.push((path.to_owned(), px));
        }
        self.image
            .lock()
            .expect("lock")
            .clone()
            .unwrap_or(Fetched::Loading)
    }

    fn forget(&self, _path: &str) {}
}

/// Gives `sensor` `width`, as the bar does when the zones do not fit (SPEC §5.4).
pub fn fit(sensor: &mut dyn Sensor, width: f32) {
    sensor.fit(width, &Harness::new().layout());
}

/// What `sensor` draws at x = 7 with `width`, on a 28 px bar.
pub fn draw(sensor: &dyn Sensor, width: f32) -> String {
    let mut canvas = RecordingCanvas::new(1.0);
    sensor.draw(
        &mut canvas,
        &DrawCtx {
            theme: &theme(),
            rect: RectF::new(7.0, 0.0, 7.0 + width, 28.0),
        },
    );
    canvas.dump()
}
