//! `disk-meter` — how full a drive is (SPEC §9.12) on the `disk/<X:\>` topic: a meter of the
//! used fraction with the used or free share or size as its value. A click opens the drive in
//! Explorer; a drive that is missing (or has no medium) shows `n/a` and is inactive.

use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

use super::common::meter::Meter;
use super::format::{self, Numbers, percent};
use super::schema::{FieldKind, Lit, PropField, SelectOption};
use super::{
    Category, ClickResult, ColorChoice, DefaultsCtx, DrawCtx, HintContent, LayoutCtx, Props,
    Sensor, SensorCtx, SensorType, State, color, typed,
};
use crate::i18n::{t, t_args};
use crate::platform::{Canvas, DriveKind, Volume};
use crate::sampler::{DiskSample, Sample, Topic, TopicRequest};

/// The drive when the machine has no fixed one (and the static default).
const FALLBACK_DRIVE: &str = "C:\\";

macro_rules! options {
    ($($value:literal => $label:literal),* $(,)?) => {
        &[$(SelectOption { value: Lit::Str($value), label: $label }),*]
    };
}

const SCHEMA: &[PropField] = &[
    PropField::new(
        "title",
        FieldKind::Text {
            empty_is_default: false,
        },
        "sensor.disk.field.title",
        Lit::Str(""),
    )
    .describe("sensor.disk.field.title.help"),
    PropField::new(
        "drive",
        FieldKind::Drive { hide_optical: true },
        "sensor.disk.field.drive",
        Lit::Str(FALLBACK_DRIVE),
    )
    .machine(),
    PropField::new(
        "mode",
        FieldKind::Select {
            options: options!(
                "usedPercent" => "sensor.disk.mode.usedPercent",
                "freePercent" => "sensor.disk.mode.freePercent",
                "usedBytes" => "sensor.disk.mode.usedBytes",
                "freeBytes" => "sensor.disk.mode.freeBytes",
            ),
        },
        "sensor.disk.field.mode",
        Lit::Str("usedPercent"),
    ),
    PropField::new(
        "interval",
        FieldKind::Interval {
            min: 10.0,
            max: 300.0,
        },
        "sensor.field.interval",
        Lit::Int(30),
    ),
    PropField::new(
        "color",
        FieldKind::ColorMode { dynamic: true },
        "sensor.field.color",
        Lit::ColorMode("standard", "#FFFFFF"),
    ),
];

/// The first fixed drive (SPEC §9.12), the machine-derived default.
pub fn first_fixed(volumes: &[Volume]) -> Option<&str> {
    volumes
        .iter()
        .find(|v| v.kind == DriveKind::Fixed)
        .map(|v| v.root.as_str())
}

pub struct DiskMeterType;

impl SensorType for DiskMeterType {
    fn id(&self) -> &'static str {
        "disk-meter"
    }

    fn category(&self) -> Category {
        Category::System
    }

    fn name(&self) -> &'static str {
        "sensor.diskMeter.name"
    }

    fn description(&self) -> &'static str {
        "sensor.diskMeter.description"
    }

    fn legacy_types(&self) -> &'static [&'static str] {
        &["DockDiskMeter"]
    }

    fn schema(&self) -> &'static [PropField] {
        SCHEMA
    }

    fn machine_defaults(&self, ctx: &DefaultsCtx) -> Props {
        let volumes = ctx.platform.volumes();
        let drive = first_fixed(&volumes).unwrap_or(FALLBACK_DRIVE);
        let mut props = Props::new();
        props.insert("drive".into(), Value::from(drive));
        props
    }

    fn create(&self, props: &Props, _state: &State) -> serde_json::Result<Box<dyn Sensor>> {
        Ok(Box::new(DiskMeter {
            props: typed(props)?,
            sample: None,
            meter: None,
        }))
    }

    fn summary(&self, props: &Props) -> Option<String> {
        props
            .get("drive")
            .and_then(Value::as_str)
            .map(str::to_owned)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Mode {
    UsedPercent,
    FreePercent,
    UsedBytes,
    FreeBytes,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiskProps {
    /// Empty: `HD: C:\`.
    title: String,
    drive: String,
    mode: Mode,
    interval: u64,
    color: ColorChoice,
}

struct DiskMeter {
    props: DiskProps,
    sample: Option<DiskSample>,
    meter: Option<Meter>,
}

fn used_fraction(s: &DiskSample) -> f32 {
    if s.total > 0 {
        (s.total - s.free.min(s.total)) as f32 / s.total as f32
    } else {
        0.0
    }
}

impl DiskMeter {
    fn label(&self) -> String {
        if self.props.title.is_empty() {
            t_args("sensor.disk.label", &[("drive", &self.props.drive)])
        } else {
            self.props.title.clone()
        }
    }

    fn absent(&self) -> bool {
        self.sample.is_some_and(|s| !s.present)
    }

    /// The value line (SPEC §9.12): the used or free share or size; `n/a` for an absent
    /// drive, nothing before the first sample.
    fn value(&self, numbers: Numbers) -> String {
        let Some(s) = self.sample else {
            return String::new();
        };
        if !s.present {
            return t("format.notAvailable");
        }
        let used = s.total - s.free.min(s.total);
        let share = |part: u64| {
            if s.total > 0 {
                100.0 * part as f64 / s.total as f64
            } else {
                0.0
            }
        };
        match self.props.mode {
            Mode::UsedPercent => percent(share(used)),
            Mode::FreePercent => percent(share(s.free)),
            Mode::UsedBytes => format::bytes(used, None, numbers),
            Mode::FreeBytes => format::bytes(s.free, None, numbers),
        }
    }

    fn bytes_mode(&self) -> bool {
        matches!(self.props.mode, Mode::UsedBytes | Mode::FreeBytes)
    }
}

impl Sensor for DiskMeter {
    fn measure(&mut self, cx: &LayoutCtx) -> f32 {
        let value = self.value(cx.numbers);
        // In the byte modes the value sits under the label (SPEC §9.12).
        let meter = Meter::new(cx.text, cx.theme, &self.label(), &value, !self.bytes_mode());
        let width = meter.width();
        self.meter = Some(meter);
        width
    }

    fn draw(&self, canvas: &mut dyn Canvas, cx: &DrawCtx) {
        let Some(meter) = &self.meter else {
            return;
        };
        let fraction = self
            .sample
            .as_ref()
            .filter(|s| s.present)
            .map_or(0.0, used_fraction);
        let fill = color::resolve(&self.props.color, Meter::standard_fill(cx.theme), fraction);
        let inactive = self.absent().then_some(cx.theme.inactive);
        meter.draw(canvas, cx, fraction, fill, inactive);
    }

    fn hint(&self, cx: &SensorCtx) -> Option<HintContent> {
        let s = self.sample?;
        let numbers = cx.platform.locale(cx.settings.date_locale).numbers();
        let mut text = t_args("sensor.disk.hint.title", &[("drive", &self.props.drive)]);
        if !s.present {
            text.push('\n');
            text.push_str(&t("sensor.disk.hint.absent"));
            return Some(HintContent::Text(text));
        }
        let bytes = |n: u64| format::bytes(n, None, numbers);
        let used = percent(100.0 * f64::from(used_fraction(&s)));
        for (label, value) in [
            ("sensor.disk.hint.total", bytes(s.total)),
            ("sensor.disk.hint.free", bytes(s.free)),
            ("sensor.disk.hint.used", used),
        ] {
            text.push_str(&format!("\n{}\t{value}", t(label)));
        }
        Some(HintContent::Text(text))
    }

    fn click(&mut self, cx: &SensorCtx) -> ClickResult {
        if self.absent() {
            return ClickResult::None;
        }
        cx.platform.open(&self.props.drive);
        ClickResult::Handled
    }

    fn inactive(&self) -> bool {
        self.absent()
    }

    fn accessible_value(&self, cx: &SensorCtx) -> String {
        self.value(cx.platform.locale(cx.settings.date_locale).numbers())
    }

    fn topics(&self) -> Vec<TopicRequest> {
        vec![TopicRequest {
            topic: Topic::Disk(self.props.drive.clone()),
            interval: Duration::from_secs(self.props.interval.max(1)),
        }]
    }

    fn on_sample(&mut self, topic: &Topic, sample: &Sample, _cx: &SensorCtx) -> bool {
        let (Topic::Disk(drive), Sample::Disk(disk)) = (topic, sample) else {
            return false;
        };
        if *drive != self.props.drive || self.sample == Some(*disk) {
            return false;
        }
        self.sample = Some(*disk);
        true
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::config::model::Settings;
    use crate::platform::RectF;
    use crate::platform::recording::{FixedMetrics, RecordingCanvas};
    use crate::sensors::schema::resolve;
    use crate::sensors::test_support::{FakeClock, NoPlatform, theme};

    const GB: u64 = 1 << 30;

    fn create(stored: serde_json::Value) -> Box<dyn Sensor> {
        let stored = stored.as_object().cloned().unwrap_or_default();
        let (props, ignored) = resolve(SCHEMA, &stored, &Props::new());
        assert!(ignored.is_empty(), "{ignored:?}");
        DiskMeterType
            .create(&props, &State::new())
            .expect("valid props")
    }

    fn disk(total: u64, free: u64) -> Sample {
        Sample::Disk(DiskSample {
            present: true,
            total,
            free,
        })
    }

    fn c() -> Topic {
        Topic::Disk("C:\\".into())
    }

    struct Harness {
        clock: FakeClock,
        settings: Settings,
        theme: crate::bar::theme::Theme,
    }

    impl Harness {
        fn new() -> Self {
            Self {
                clock: FakeClock::default(),
                settings: Settings::default(),
                theme: theme(),
            }
        }

        fn ctx(&self) -> SensorCtx<'_> {
            SensorCtx {
                id: "s_disk",
                settings: &self.settings,
                theme: &self.theme,
                locked: false,
                alt: false,
                animate: true,
                platform: &self.clock,
            }
        }

        fn drawn(&self, sensor: &mut dyn Sensor) -> (f32, String) {
            let width = sensor.measure(&LayoutCtx {
                theme: &self.theme,
                text: &FixedMetrics,
                scale: 1.0,
                numbers: Default::default(),
                icons: &crate::sensors::NoIcons,
            });
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
    }

    #[test]
    fn meter_shows_the_used_share() {
        let h = Harness::new();
        let mut meter = create(json!({}));
        assert_eq!(
            meter.topics()[0],
            TopicRequest {
                topic: c(),
                interval: Duration::from_secs(30)
            }
        );
        assert!(meter.on_sample(&c(), &disk(1000 * GB, 250 * GB), &h.ctx()));
        assert!(!meter.on_sample(&c(), &disk(1000 * GB, 250 * GB), &h.ctx()));
        assert!(
            !meter.on_sample(&Topic::Disk("D:\\".into()), &disk(1, 1), &h.ctx()),
            "another drive"
        );
        let (width, dump) = h.drawn(meter.as_mut());
        // "HD: C:\" is 7 characters: 8 + 35 px.
        assert_eq!(width, 43.0);
        let lines: Vec<&str> = dump.lines().collect();
        // 75 % of the 22 px bar: 16.5 px from 25 → 8.5 → 9 (whole px).
        assert_eq!(lines[1], "fill rect 0,9 4×16 #FFFFFF/59%");
        // The recording canvas escapes the backslash.
        assert!(lines[2].starts_with(r#"text "HD: C:\\""#), "{dump}");
        assert!(lines[3].starts_with("text \"75%\""), "{dump}");
    }

    #[test]
    fn the_value_follows_the_mode() {
        let h = Harness::new();
        let value = |mode: &str| {
            let mut meter = create(json!({ "mode": mode, "title": "System" }));
            meter.on_sample(&c(), &disk(953 * GB, 241 * GB), &h.ctx());
            let (width, dump) = h.drawn(meter.as_mut());
            (meter.accessible_value(&h.ctx()), width, dump)
        };
        assert_eq!(value("usedPercent").0, "75%");
        assert_eq!(value("freePercent").0, "25%");
        assert_eq!(value("usedBytes").0, "712 GB");
        let (free, width, dump) = value("freeBytes");
        assert_eq!(free, "241 GB");
        // Under the label: "System" and "241 GB" are both 6 characters.
        assert_eq!(width, 38.0);
        assert!(
            dump.contains("text \"241 GB\"") && dump.contains(" at 8,14 "),
            "{dump}"
        );
    }

    #[test]
    fn absent_drives_are_inactive() {
        let h = Harness::new();
        let mut meter = create(json!({ "drive": "E:\\" }));
        assert!(!meter.inactive(), "nothing read yet");
        let absent = Sample::Disk(DiskSample::default());
        meter.on_sample(&Topic::Disk("E:\\".into()), &absent, &h.ctx());
        assert!(meter.inactive());
        let (_, dump) = h.drawn(meter.as_mut());
        assert!(
            dump.starts_with("fill rect 0,3 4×22 #FFFFFF/24%\n"),
            "{dump}"
        );
        assert!(dump.contains("text \"n/a\""), "{dump}");
        assert_eq!(
            meter.hint(&h.ctx()),
            Some(HintContent::Text(
                "Disk Meter for E:\\\nThe drive is not available".into()
            ))
        );
        assert_eq!(meter.click(&h.ctx()), ClickResult::None);
    }

    #[test]
    fn hint_and_click() {
        let h = Harness::new();
        let mut meter = create(json!({}));
        assert!(meter.hint(&h.ctx()).is_none());
        meter.on_sample(&c(), &disk(953 * GB, 241 * GB), &h.ctx());
        assert_eq!(
            meter.hint(&h.ctx()),
            Some(HintContent::Text(
                "Disk Meter for C:\\\nTotal Space:\t953 GB\nFree Space:\t241 GB\n\
                 Disk Space Used:\t75%"
                    .into()
            ))
        );
        assert_eq!(meter.click(&h.ctx()), ClickResult::Handled);
        assert_eq!(h.clock.log(), ["open C:\\"]);
    }

    #[test]
    fn the_first_fixed_drive_is_the_default() {
        let volume = |root: &str, kind| Volume {
            root: root.into(),
            kind,
            label: String::new(),
        };
        let volumes = [
            volume("A:\\", DriveKind::Removable),
            volume("D:\\", DriveKind::Fixed),
            volume("E:\\", DriveKind::Fixed),
        ];
        assert_eq!(first_fixed(&volumes), Some("D:\\"));
        assert_eq!(first_fixed(&volumes[..1]), None);
        let ctx = DefaultsCtx {
            platform: &NoPlatform,
        };
        assert_eq!(DiskMeterType.machine_defaults(&ctx)["drive"], "C:\\");
        let (props, _) = resolve(SCHEMA, &Props::new(), &Props::new());
        assert_eq!(DiskMeterType.summary(&props).as_deref(), Some("C:\\"));
    }
}
