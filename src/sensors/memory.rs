//! `memory-meter` / `memory-graph` and `swap-meter` / `swap-graph` — physical memory and the
//! page files in use (SPEC §9.10–9.11) on the `memory` and `swap` topics, as a meter of the
//! load or a graph showing the used bytes or the load; a click opens Task Manager. Without a
//! page file the swap sensors show `n/a` and are inactive.

use std::time::Duration;

use serde::Deserialize;

use super::common::graph::{self, Columns, GraphStyle, GraphText, Growth};
use super::common::history::History;
use super::common::meter::Meter;
use super::cpu::TASK_MANAGER;
use super::format::{self, Numbers, percent};
use super::legacy::LegacyKey;
use super::schema::{FieldKind, Lit, PropField, SelectOption};
use super::{
    Category, ClickResult, ColorChoice, DrawCtx, HintContent, LayoutCtx, Props, Sensor, SensorCtx,
    SensorType, State, color, typed,
};
use crate::i18n::{t, t_args};
use crate::look::theme::Inactive;
use crate::platform::Canvas;
use crate::sampler::{Sample, Topic, TopicRequest};

/// A reading of a "used of total" topic: bytes, and the load 0–100. A total of 0 means there
/// is nothing to use (no page file).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Reading {
    total: u64,
    available: u64,
    used: u64,
    load: f32,
}

impl Reading {
    fn absent(&self) -> bool {
        self.total == 0
    }
}

/// Which topic a usage sensor shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Memory,
    Swap,
}

impl Kind {
    fn topic(self) -> Topic {
        match self {
            Kind::Memory => Topic::Memory,
            Kind::Swap => Topic::Swap,
        }
    }

    fn label(self) -> String {
        t(match self {
            Kind::Memory => "sensor.memory.label",
            Kind::Swap => "sensor.swap.label",
        })
    }

    fn reading(self, sample: &Sample) -> Option<Reading> {
        match (self, sample) {
            (Kind::Memory, Sample::Memory(m)) => Some(Reading {
                total: m.total,
                available: m.available,
                used: m.used,
                load: m.load,
            }),
            (Kind::Swap, Sample::Swap(s)) => Some(Reading {
                total: s.total,
                available: s.total.saturating_sub(s.used),
                used: s.used,
                load: s.load,
            }),
            _ => None,
        }
    }

    /// The hint (SPEC §9.10–9.11): total, available and load (the caller adds what a click
    /// does); for swap without a page file, just that.
    fn hint(self, r: &Reading, numbers: Numbers) -> String {
        if self == Kind::Swap && r.absent() {
            return t("sensor.swap.hint.none");
        }
        let bytes = |n: u64| format::bytes(n, None, numbers);
        let keys = match self {
            Kind::Memory => [
                "sensor.memory.hint.total",
                "sensor.memory.hint.available",
                "sensor.memory.hint.load",
            ],
            Kind::Swap => [
                "sensor.swap.hint.total",
                "sensor.swap.hint.available",
                "sensor.swap.hint.load",
            ],
        };
        let values = [
            bytes(r.total),
            bytes(r.available),
            percent(f64::from(r.load)),
        ];
        let lines = keys.into_iter().zip(values);
        let lines: Vec<String> = lines
            .map(|(label, value)| format!("{}\t{value}", t(label)))
            .collect();
        lines.join("\n")
    }
}

macro_rules! options {
    ($($value:literal => $label:literal),* $(,)?) => {
        &[$(SelectOption { value: Lit::Str($value), label: $label }),*]
    };
}

const METER_SCHEMA: &[PropField] = &[
    PropField::new(
        "interval",
        FieldKind::Interval {
            min: 1.0,
            max: 60.0,
        },
        "sensor.field.interval",
        Lit::Int(2),
    ),
    PropField::new(
        "color",
        FieldKind::ColorMode { dynamic: true },
        "sensor.field.color",
        Lit::ColorMode("standard", "#FFFFFF"),
    ),
];

const GRAPH_SCHEMA: &[PropField] = &[
    graph::INTERVAL,
    PropField::new(
        "display",
        FieldKind::Select {
            options: options!(
                "bytes" => "sensor.usage.display.bytes",
                "percent" => "sensor.usage.display.percent",
            ),
        },
        "sensor.usage.field.display",
        Lit::Str("bytes"),
    ),
    graph::WIDTH,
    graph::BK_COLOR,
    graph::BK_OPACITY,
    graph::GRID_COLOR,
    graph::GRID_OPACITY,
    graph::GRAPH_COLOR,
    graph::GRAPH_OPACITY,
];

/// SPEC §9.10–9.11.
const METER_LEGACY_KEYS: &[LegacyKey] = &[
    LegacyKey::new("interval", "interval"),
    LegacyKey::new("colorize", "color"),
];

/// SPEC §9.10–9.11.
const GRAPH_LEGACY_KEYS: &[LegacyKey] = graph::legacy_keys![
    LegacyKey::options(
        "display-mode",
        "display",
        &[("0", Lit::Str("bytes")), ("1", Lit::Str("percent"))],
    ),
    LegacyKey::new("graph-color", "graphColor"),
    LegacyKey::new("graph-opacity", "graphOpacity"),
];

fn topics(kind: Kind, interval: u64) -> Vec<TopicRequest> {
    vec![TopicRequest {
        topic: kind.topic(),
        interval: Duration::from_secs(interval.max(1)),
    }]
}

fn hint(kind: Kind, reading: Option<&Reading>, cx: &SensorCtx) -> Option<HintContent> {
    let numbers = Numbers::from(&cx.platform.locale(cx.settings.date_locale));
    let reading = reading?;
    let text = kind.hint(reading, numbers);
    Some(HintContent::Text(
        if kind == Kind::Swap && reading.absent() {
            text
        } else {
            cx.with_click_line(text, "sensor.cpu.hint.click")
        },
    ))
}

/// The meter's value, and the graph's in percent mode: the load; `n/a` when absent, nothing
/// before the first sample.
fn load(reading: Option<&Reading>) -> String {
    match reading {
        None => String::new(),
        Some(r) if r.absent() => t("format.notAvailable"),
        Some(r) => percent(f64::from(r.load)),
    }
}

/// How an absent reading dims the graphic (SPEC §8.5); `None` while it is there.
fn dimmed(reading: Option<&Reading>, cx: &DrawCtx) -> Option<Inactive> {
    reading
        .is_some_and(Reading::absent)
        .then_some(cx.theme.inactive)
}

/// A meter of a usage topic's load (SPEC §9.10).
pub struct UsageMeterType(Kind);

pub static MEMORY_METER: UsageMeterType = UsageMeterType(Kind::Memory);
pub static SWAP_METER: UsageMeterType = UsageMeterType(Kind::Swap);

impl SensorType for UsageMeterType {
    fn id(&self) -> &'static str {
        match self.0 {
            Kind::Memory => "memory-meter",
            Kind::Swap => "swap-meter",
        }
    }

    fn category(&self) -> Category {
        Category::System
    }

    fn name(&self) -> &'static str {
        match self.0 {
            Kind::Memory => "sensor.memoryMeter.name",
            Kind::Swap => "sensor.swapMeter.name",
        }
    }

    fn description(&self) -> &'static str {
        match self.0 {
            Kind::Memory => "sensor.memoryMeter.description",
            Kind::Swap => "sensor.swapMeter.description",
        }
    }

    fn legacy_types(&self) -> &'static [&'static str] {
        match self.0 {
            Kind::Memory => &["DockMemMeter"],
            Kind::Swap => &["DockSwapMeter"],
        }
    }

    fn legacy_keys(&self) -> &'static [LegacyKey] {
        METER_LEGACY_KEYS
    }

    fn schema(&self) -> &'static [PropField] {
        METER_SCHEMA
    }

    fn create(&self, props: &Props, _state: &State) -> serde_json::Result<Box<dyn Sensor>> {
        Ok(Box::new(UsageMeter {
            kind: self.0,
            props: typed(props)?,
            reading: None,
            meter: None,
        }))
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MeterProps {
    interval: u64,
    color: ColorChoice,
}

struct UsageMeter {
    kind: Kind,
    props: MeterProps,
    reading: Option<Reading>,
    meter: Option<Meter>,
}

impl Sensor for UsageMeter {
    fn measure(&mut self, cx: &LayoutCtx) -> f32 {
        let value = load(self.reading.as_ref());
        let meter = Meter::new(cx.text, cx.theme, &self.kind.label(), &value, true);
        let width = meter.width();
        self.meter = Some(meter);
        width
    }

    fn draw(&self, canvas: &mut dyn Canvas, cx: &DrawCtx) {
        let Some(meter) = &self.meter else {
            return;
        };
        let fraction = self.reading.as_ref().map_or(0.0, |r| r.load / 100.0);
        let fill = color::resolve(&self.props.color, Meter::standard_fill(cx.theme), fraction);
        meter.draw(
            canvas,
            cx,
            fraction,
            fill,
            dimmed(self.reading.as_ref(), cx),
        );
    }

    fn hint(&self, cx: &SensorCtx) -> Option<HintContent> {
        hint(self.kind, self.reading.as_ref(), cx)
    }

    fn click(&mut self, cx: &SensorCtx) -> ClickResult {
        cx.platform.open(TASK_MANAGER);
        ClickResult::Handled
    }

    fn accessible_value(&self, _cx: &SensorCtx) -> String {
        load(self.reading.as_ref())
    }

    fn inactive(&self) -> bool {
        self.reading.as_ref().is_some_and(Reading::absent)
    }

    fn topics(&self) -> Vec<TopicRequest> {
        topics(self.kind, self.props.interval)
    }

    fn on_sample(&mut self, _topic: &Topic, sample: &Sample, _cx: &SensorCtx) -> bool {
        let Some(reading) = self.kind.reading(sample) else {
            return false;
        };
        let changed = self.reading != Some(reading);
        self.reading = Some(reading);
        changed
    }
}

/// A graph of a usage topic's load, showing the used bytes or the load (SPEC §9.10).
pub struct UsageGraphType(Kind);

pub static MEMORY_GRAPH: UsageGraphType = UsageGraphType(Kind::Memory);
pub static SWAP_GRAPH: UsageGraphType = UsageGraphType(Kind::Swap);

impl SensorType for UsageGraphType {
    fn id(&self) -> &'static str {
        match self.0 {
            Kind::Memory => "memory-graph",
            Kind::Swap => "swap-graph",
        }
    }

    fn category(&self) -> Category {
        Category::System
    }

    fn name(&self) -> &'static str {
        match self.0 {
            Kind::Memory => "sensor.memoryGraph.name",
            Kind::Swap => "sensor.swapGraph.name",
        }
    }

    fn description(&self) -> &'static str {
        match self.0 {
            Kind::Memory => "sensor.memoryGraph.description",
            Kind::Swap => "sensor.swapGraph.description",
        }
    }

    fn legacy_types(&self) -> &'static [&'static str] {
        match self.0 {
            Kind::Memory => &["DockMemGraph"],
            Kind::Swap => &["DockSwapGraph"],
        }
    }

    fn legacy_keys(&self) -> &'static [LegacyKey] {
        GRAPH_LEGACY_KEYS
    }

    fn schema(&self) -> &'static [PropField] {
        GRAPH_SCHEMA
    }

    fn create(&self, props: &Props, _state: &State) -> serde_json::Result<Box<dyn Sensor>> {
        let props: GraphProps = typed(props)?;
        let columns = props.style.plot_width() as usize;
        Ok(Box::new(UsageGraph {
            kind: self.0,
            props,
            reading: None,
            history: History::new(columns),
            text: None,
        }))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Display {
    /// The used bytes, e.g. `5.21 GB`.
    Bytes,
    Percent,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphProps {
    interval: u64,
    display: Display,
    #[serde(flatten)]
    style: GraphStyle,
}

struct UsageGraph {
    kind: Kind,
    props: GraphProps,
    reading: Option<Reading>,
    history: History,
    text: Option<GraphText>,
}

impl UsageGraph {
    fn value(&self, numbers: Numbers) -> String {
        match (self.props.display, &self.reading) {
            (Display::Bytes, Some(r)) if !r.absent() => format::bytes(r.used, None, numbers),
            (_, reading) => load(reading.as_ref()),
        }
    }
}

impl Sensor for UsageGraph {
    fn measure(&mut self, cx: &LayoutCtx) -> f32 {
        let value = self.value(cx.numbers);
        // The column fits `9999 MB` in bytes mode (SPEC §9.10).
        let reserve = match self.props.display {
            Display::Bytes => t_args("format.mb", &[("value", "9999")]),
            Display::Percent => percent(100.0),
        };
        let text = GraphText::new(cx.text, cx.theme, &self.kind.label(), &value, &reserve);
        let width = text.sensor_width(self.props.style.plot_width());
        self.text = Some(text);
        width
    }

    fn draw(&self, canvas: &mut dyn Canvas, cx: &DrawCtx) {
        let style = &self.props.style;
        let plot = graph::plot_rect(cx.rect, style.plot_width());
        let mut backdrop = style.backdrop();
        let mut columns = style.columns(cx.theme);
        if let Some(how) = dimmed(self.reading.as_ref(), cx) {
            backdrop.background = color::inactive(backdrop.background, how);
            backdrop.grid = color::inactive(backdrop.grid, how);
            if let Columns::Solid(c) = columns {
                columns = Columns::Solid(color::inactive(c, how));
            }
        }
        graph::draw_backdrop(canvas, plot, backdrop);
        graph::draw_columns(canvas, plot, &self.history, 100.0, columns, Growth::Up);
        if let Some(text) = &self.text {
            text.draw(canvas, cx);
        }
    }

    fn hint(&self, cx: &SensorCtx) -> Option<HintContent> {
        hint(self.kind, self.reading.as_ref(), cx)
    }

    fn click(&mut self, cx: &SensorCtx) -> ClickResult {
        cx.platform.open(TASK_MANAGER);
        ClickResult::Handled
    }

    fn accessible_value(&self, cx: &SensorCtx) -> String {
        self.value(Numbers::from(&cx.platform.locale(cx.settings.date_locale)))
    }

    fn inactive(&self) -> bool {
        self.reading.as_ref().is_some_and(Reading::absent)
    }

    fn topics(&self) -> Vec<TopicRequest> {
        topics(self.kind, self.props.interval)
    }

    fn on_sample(&mut self, _topic: &Topic, sample: &Sample, _cx: &SensorCtx) -> bool {
        let Some(reading) = self.kind.reading(sample) else {
            return false;
        };
        self.history.push(reading.load);
        self.reading = Some(reading);
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
    use crate::sampler::{MemorySample, SwapSample};
    use crate::sensors::schema::resolve;
    use crate::sensors::test_support::{FakeClock, theme};

    const GB: u64 = 1 << 30;

    fn create(ty: &dyn SensorType, stored: serde_json::Value) -> Box<dyn Sensor> {
        let stored = stored.as_object().cloned().unwrap_or_default();
        let (props, ignored) = resolve(ty.schema(), &stored, &Props::new());
        assert!(ignored.is_empty(), "{ignored:?}");
        ty.create(&props, &State::new()).expect("valid props")
    }

    fn memory(used: u64, load: f32) -> Sample {
        let total = 32 * GB;
        Sample::Memory(MemorySample {
            total,
            available: total - used,
            used,
            load,
        })
    }

    struct Harness {
        clock: FakeClock,
        settings: Settings,
        theme: crate::look::theme::Theme,
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
                settings: &self.settings,
                locked: false,
                alt: false,
                animate: true,
                changes: &crate::sampler::ChangeSink::NONE,
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
    fn meter_shows_the_load() {
        let h = Harness::new();
        let mut meter = create(&MEMORY_METER, json!({}));
        assert_eq!(meter.topics()[0].topic, Topic::Memory);
        assert_eq!(meter.topics()[0].interval, Duration::from_secs(2));
        assert!(meter.on_sample(&Topic::Memory, &memory(14 * GB, 50.0), &h.ctx()));
        assert!(
            !meter.on_sample(&Topic::Memory, &memory(14 * GB, 50.0), &h.ctx()),
            "unchanged"
        );
        let (width, dump) = h.drawn(meter.as_mut());
        assert_eq!(width, 28.0);
        let lines: Vec<&str> = dump.lines().collect();
        // Half of the 22 px bar.
        assert_eq!(lines[1], "fill rect 0,14 4×11 #FFFFFF/59%");
        assert!(lines[2].starts_with("text \"RAM\""), "{dump}");
        assert!(lines[3].starts_with("text \"50%\""), "{dump}");
        assert_eq!(meter.accessible_value(&h.ctx()), "50%");
    }

    #[test]
    fn graph_shows_used_bytes_or_the_load() {
        let h = Harness::new();
        let mut graph = create(&MEMORY_GRAPH, json!({}));
        assert_eq!(graph.topics()[0].interval, Duration::from_secs(1));
        graph.on_sample(&Topic::Memory, &memory(5 * GB + GB / 5, 16.3), &h.ctx());
        let (width, dump) = h.drawn(graph.as_mut());
        // A 50 px plot, the gap and a column for "9999 MB" (7 characters at 5 px).
        assert_eq!(width, 50.0 + 5.0 + 35.0);
        let lines: Vec<&str> = dump.lines().collect();
        // 16.3 % of the 24 px plot.
        assert_eq!(lines[0], "fill rect 49,22 1×4 #00FF00/55%");
        assert!(lines[1].starts_with("text \"RAM\""), "{dump}");
        assert!(lines[2].starts_with("text \"5.20 GB\""), "{dump}");

        let mut graph = create(&MEMORY_GRAPH, json!({ "display": "percent" }));
        graph.on_sample(&Topic::Memory, &memory(5 * GB, 16.3), &h.ctx());
        let (width, dump) = h.drawn(graph.as_mut());
        assert_eq!(width, 75.0, "a column for 100%");
        assert!(dump.contains("text \"16%\""), "{dump}");
    }

    #[test]
    fn bar_text_follows_the_locale() {
        let h = Harness::new();
        let mut graph = create(&MEMORY_GRAPH, json!({}));
        graph.on_sample(&Topic::Memory, &memory(5 * GB + GB / 5, 16.3), &h.ctx());
        graph.measure(&LayoutCtx {
            theme: &h.theme,
            text: &FixedMetrics,
            scale: 1.0,
            numbers: Numbers { decimal: ',' },
            icons: &crate::sensors::NoIcons,
        });
        let mut canvas = RecordingCanvas::new(1.0);
        let rect = RectF::new(0.0, 0.0, 90.0, 28.0);
        graph.draw(
            &mut canvas,
            &DrawCtx {
                theme: &h.theme,
                rect,
            },
        );
        assert!(
            canvas.dump().contains("text \"5,20 GB\""),
            "{}",
            canvas.dump()
        );
    }

    #[test]
    fn nothing_before_the_first_sample() {
        let h = Harness::new();
        let mut graph = create(&MEMORY_GRAPH, json!({}));
        let (_, dump) = h.drawn(graph.as_mut());
        assert!(dump.contains("text \"\""), "an empty value: {dump}");
        assert!(graph.hint(&h.ctx()).is_none());
        assert!(
            !graph.on_sample(&Topic::Cpu, &Sample::Cpu(Default::default()), &h.ctx()),
            "another topic"
        );
    }

    #[test]
    fn hint_and_click() {
        let h = Harness::new();
        let mut meter = create(&MEMORY_METER, json!({}));
        let total = 31 * GB + 7 * GB / 10;
        let sample = Sample::Memory(MemorySample {
            total,
            available: 18 * GB + GB / 5,
            used: total - (18 * GB + GB / 5),
            load: 43.0,
        });
        meter.on_sample(&Topic::Memory, &sample, &h.ctx());
        assert_eq!(
            meter.hint(&h.ctx()),
            Some(HintContent::Text(
                "Total Physical:\t31.7 GB\nAvailable Physical:\t18.2 GB\nMemory Load:\t43%\n\
                 Click to open Task Manager"
                    .into()
            ))
        );
        assert_eq!(meter.click(&h.ctx()), ClickResult::Handled);
        assert_eq!(h.clock.log(), ["open taskmgr.exe"]);
    }

    #[test]
    fn swap_shares_the_view() {
        let h = Harness::new();
        let mut meter = create(&SWAP_METER, json!({}));
        assert_eq!(meter.topics()[0].topic, Topic::Swap);
        let swap = Sample::Swap(SwapSample {
            total: 8 * GB,
            used: 3 * GB / 5,
            load: 7.5,
        });
        meter.on_sample(&Topic::Swap, &swap, &h.ctx());
        assert!(!meter.inactive());
        let (_, dump) = h.drawn(meter.as_mut());
        assert!(dump.contains("text \"Swap\""), "{dump}");
        assert!(dump.contains("text \"8%\""), "{dump}");
        assert_eq!(
            meter.hint(&h.ctx()),
            Some(HintContent::Text(
                "Total Virtual:\t8.00 GB\nAvailable Virtual:\t7.40 GB\nSwap Load:\t8%\n\
                 Click to open Task Manager"
                    .into()
            ))
        );
        let mut graph = create(&SWAP_GRAPH, json!({}));
        graph.on_sample(&Topic::Swap, &swap, &h.ctx());
        let (_, dump) = h.drawn(graph.as_mut());
        assert!(dump.contains("text \"614 MB\""), "{dump}");
    }

    #[test]
    fn swap_without_a_page_file_is_inactive() {
        let h = Harness::new();
        let none = Sample::Swap(SwapSample::default());
        let mut meter = create(&SWAP_METER, json!({}));
        meter.on_sample(&Topic::Swap, &none, &h.ctx());
        assert!(meter.inactive());
        let (_, dump) = h.drawn(meter.as_mut());
        let lines: Vec<&str> = dump.lines().collect();
        // The bar at the inactive opacity (78 % of 31 %); the text as it is.
        assert_eq!(lines[0], "fill rect 0,3 4×22 #FFFFFF/24%");
        assert!(lines[2].starts_with("text \"n/a\""), "{dump}");
        assert_eq!(
            meter.hint(&h.ctx()),
            Some(HintContent::Text("No page file configured".into()))
        );
        let mut graph = create(&SWAP_GRAPH, json!({ "bkOpacity": 100 }));
        graph.on_sample(&Topic::Swap, &none, &h.ctx());
        assert!(graph.inactive());
        let (_, dump) = h.drawn(graph.as_mut());
        assert!(
            dump.starts_with("fill rect 0,2 50×24 #000000/78%"),
            "{dump}"
        );
        assert!(dump.contains("text \"n/a\""), "{dump}");
    }
}
