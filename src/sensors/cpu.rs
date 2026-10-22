//! `cpu-meter`, `cpu-graph` and `cpu-graph-dual` — the processors' load (SPEC §9.7–9.9) on the
//! `cpu` topic, as a meter or a graph of one or two series, with the hint all CPU sensors
//! share; a click opens Task Manager.

use std::time::Duration;

use serde::Deserialize;

use super::common::dual_graph::{self, Series};
use super::common::graph::{self, GraphStyle, GraphText, Growth, PlotStyle};
use super::common::history::History;
use super::common::meter::Meter;
use super::format::{Numbers, percent, percent_digits};
use super::legacy::LegacyKey;
use super::schema::{FieldKind, Lit, PropField};
use super::{
    Category, ClickResult, ColorChoice, DefaultsCtx, DrawCtx, HintContent, LayoutCtx, Props,
    Sensor, SensorCtx, SensorType, State, color, typed,
};
use crate::i18n::{t, t_count};
use crate::platform::{Canvas, Capability};
use crate::sampler::{CpuSample, Sample, Topic, TopicRequest};

/// Task Manager, opened by a click (SPEC §9.7).
pub(super) const TASK_MANAGER: &str = "taskmgr.exe";

const METER_SCHEMA: &[PropField] = &[
    graph::INTERVAL,
    PropField::new(
        "color",
        FieldKind::ColorMode { dynamic: true },
        "sensor.field.color",
        Lit::ColorMode("standard", "#FFFFFF"),
    ),
];

const GRAPH_SCHEMA: &[PropField] = &[
    graph::INTERVAL,
    graph::WIDTH,
    graph::BK_COLOR,
    graph::BK_OPACITY,
    graph::GRID_COLOR,
    graph::GRID_OPACITY,
    graph::GRAPH_COLOR,
    graph::GRAPH_OPACITY,
];

const DUAL_SCHEMA: &[PropField] = &[
    graph::INTERVAL,
    graph::WIDTH,
    graph::BK_COLOR,
    graph::BK_OPACITY,
    graph::GRID_COLOR,
    graph::GRID_OPACITY,
    PropField::new(
        "topSeries",
        FieldKind::CpuSeries,
        "sensor.cpuDual.field.topSeries",
        Lit::Str("core0"),
    ),
    PropField::new(
        "bottomSeries",
        FieldKind::CpuSeries,
        "sensor.cpuDual.field.bottomSeries",
        Lit::Str("core1"),
    ),
    PropField::new(
        "topColor",
        FieldKind::ColorMode { dynamic: true },
        "sensor.dual.field.topColor",
        Lit::ColorMode("custom", "#20B7FF"),
    ),
    PropField::new(
        "topOpacity",
        FieldKind::Opacity { min: 20 },
        "sensor.dual.field.topOpacity",
        Lit::Int(86),
    ),
    PropField::new(
        "bottomColor",
        FieldKind::ColorMode { dynamic: true },
        "sensor.dual.field.bottomColor",
        Lit::ColorMode("custom", "#FFAD5B"),
    ),
    PropField::new(
        "bottomOpacity",
        FieldKind::Opacity { min: 20 },
        "sensor.dual.field.bottomOpacity",
        Lit::Int(86),
    ),
    PropField::new(
        "flipBottom",
        FieldKind::Bool,
        "sensor.dual.field.flipBottom",
        Lit::Bool(true),
    )
    .describe("sensor.dual.field.flipBottom.help"),
];

/// SPEC §9.7.
const METER_LEGACY_KEYS: &[LegacyKey] = &[
    LegacyKey::new("interval", "interval"),
    LegacyKey::new("colorize", "color"),
];

/// SPEC §9.8.
const GRAPH_LEGACY_KEYS: &[LegacyKey] = graph::legacy_keys![
    LegacyKey::new("graph-color", "graphColor"),
    LegacyKey::new("graph-opacity", "graphOpacity"),
];

/// SPEC §9.9: the series keep their defaults.
const DUAL_LEGACY_KEYS: &[LegacyKey] = graph::legacy_keys![
    LegacyKey::new("graph-top-color", "topColor"),
    LegacyKey::new("graph-top-opacity", "topOpacity"),
    LegacyKey::new("graph-bottom-color", "bottomColor"),
    LegacyKey::new("graph-bottom-opacity", "bottomOpacity"),
    LegacyKey::new("graph-bottom-flip", "flipBottom"),
];

/// A series' value in `sample` (SPEC §9.9): `total`, `user`, `system` or `core<i>`; a
/// processor this computer lacks, or one that could not be read (NaN), shows the total.
fn series_value(series: &str, sample: &CpuSample) -> f32 {
    match series {
        "user" => sample.user,
        "system" => sample.kernel,
        core => core
            .strip_prefix("core")
            .and_then(|i| i.parse::<usize>().ok())
            .and_then(|i| sample.cores.get(i).copied())
            .filter(|v| v.is_finite())
            .unwrap_or(sample.total),
    }
}

/// The hint of every CPU sensor (SPEC §9.7): the processor count and the busy, user, system
/// and idle shares with one decimal (the caller adds what a click does). User and system are
/// clamped so that they add up to at most 100.
pub fn hint(sample: &CpuSample, numbers: Numbers) -> String {
    let share = |value: f32| percent_digits(f64::from(value), 1, numbers);
    let user = sample.user.clamp(0.0, 100.0);
    let system = sample.kernel.clamp(0.0, 100.0 - user);
    let lines = [
        ("sensor.cpu.hint.activity", sample.total),
        ("sensor.cpu.hint.user", user),
        ("sensor.cpu.hint.system", system),
        ("sensor.cpu.hint.idle", sample.idle),
    ];
    let mut text = t_count("sensor.cpu.hint.uses", u64::from(sample.logical), &[]);
    for (label, value) in lines {
        text.push_str(&format!("\n   {}\t{}", t(label), share(value)));
    }
    text
}

fn topics(interval: u64) -> Vec<TopicRequest> {
    vec![TopicRequest {
        topic: Topic::Cpu,
        interval: Duration::from_secs(interval.max(1)),
    }]
}

fn cpu(sample: &Sample) -> Option<&CpuSample> {
    match sample {
        Sample::Cpu(cpu) => Some(cpu),
        _ => None,
    }
}

fn shared_hint(sample: Option<&CpuSample>, cx: &SensorCtx) -> Option<HintContent> {
    let numbers = cx.numbers;
    let text = hint(sample?, numbers);
    Some(HintContent::Text(
        cx.with_click_line(text, "sensor.cpu.hint.click"),
    ))
}

fn value(sample: Option<&CpuSample>) -> String {
    sample
        .map(|s| percent(f64::from(s.total)))
        .unwrap_or_default()
}

pub struct CpuMeterType;

impl SensorType for CpuMeterType {
    fn id(&self) -> &'static str {
        "cpu-meter"
    }

    fn category(&self) -> Category {
        Category::System
    }

    fn name(&self) -> &'static str {
        "sensor.cpuMeter.name"
    }

    fn description(&self) -> &'static str {
        "sensor.cpuMeter.description"
    }

    fn legacy_types(&self) -> &'static [&'static str] {
        &["DockCPUMeter"]
    }

    fn legacy_keys(&self) -> &'static [LegacyKey] {
        METER_LEGACY_KEYS
    }

    fn schema(&self) -> &'static [PropField] {
        METER_SCHEMA
    }

    fn create(&self, props: &Props, _state: &State) -> serde_json::Result<Box<dyn Sensor>> {
        Ok(Box::new(CpuMeter {
            props: typed(props)?,
            sample: None,
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

struct CpuMeter {
    props: MeterProps,
    sample: Option<CpuSample>,
    meter: Option<Meter>,
}

impl Sensor for CpuMeter {
    fn measure(&mut self, cx: &LayoutCtx) -> f32 {
        let label = t("sensor.cpu.label");
        let meter = Meter::new(
            cx.text,
            cx.theme,
            &label,
            &value(self.sample.as_ref()),
            true,
        );
        let width = meter.width();
        self.meter = Some(meter);
        width
    }

    fn draw(&self, canvas: &mut dyn Canvas, cx: &DrawCtx) {
        let Some(meter) = &self.meter else {
            return;
        };
        let fraction = self.sample.as_ref().map_or(0.0, |s| s.total / 100.0);
        let fill = color::resolve(&self.props.color, Meter::standard_fill(cx.theme), fraction);
        meter.draw(canvas, cx, fraction, fill, None);
    }

    fn hint(&self, cx: &SensorCtx) -> Option<HintContent> {
        shared_hint(self.sample.as_ref(), cx)
    }

    fn click(&mut self, cx: &SensorCtx) -> ClickResult {
        cx.platform.open(TASK_MANAGER);
        ClickResult::Handled
    }

    fn accessible_value(&self, _cx: &SensorCtx) -> String {
        value(self.sample.as_ref())
    }

    fn topics(&self) -> Vec<TopicRequest> {
        topics(self.props.interval)
    }

    fn on_sample(&mut self, _topic: &Topic, sample: &Sample, _cx: &SensorCtx) -> bool {
        let Some(cpu) = cpu(sample) else {
            return false;
        };
        self.sample = Some(cpu.clone());
        true
    }
}

pub struct CpuGraphType;

impl SensorType for CpuGraphType {
    fn id(&self) -> &'static str {
        "cpu-graph"
    }

    fn category(&self) -> Category {
        Category::System
    }

    fn name(&self) -> &'static str {
        "sensor.cpuGraph.name"
    }

    fn description(&self) -> &'static str {
        "sensor.cpuGraph.description"
    }

    fn legacy_types(&self) -> &'static [&'static str] {
        &["DockCPUGraph"]
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
        Ok(Box::new(CpuGraph {
            props,
            sample: None,
            history: History::new(columns),
            text: None,
        }))
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphProps {
    interval: u64,
    #[serde(flatten)]
    style: GraphStyle,
}

struct CpuGraph {
    props: GraphProps,
    sample: Option<CpuSample>,
    history: History,
    text: Option<GraphText>,
}

impl Sensor for CpuGraph {
    fn measure(&mut self, cx: &LayoutCtx) -> f32 {
        let label = t("sensor.cpu.label");
        let text = GraphText::new(
            cx.text,
            cx.theme,
            &label,
            &value(self.sample.as_ref()),
            &percent(100.0),
        );
        let width = text.sensor_width(self.props.style.plot_width());
        self.text = Some(text);
        width
    }

    fn draw(&self, canvas: &mut dyn Canvas, cx: &DrawCtx) {
        let style = &self.props.style;
        let plot = graph::plot_rect(cx.rect, style.plot_width());
        graph::draw_backdrop(canvas, plot, style.backdrop());
        let columns = style.columns(cx.theme);
        graph::draw_columns(canvas, plot, &self.history, 100.0, columns, Growth::Up);
        if let Some(text) = &self.text {
            text.draw(canvas, cx);
        }
    }

    fn hint(&self, cx: &SensorCtx) -> Option<HintContent> {
        shared_hint(self.sample.as_ref(), cx)
    }

    fn click(&mut self, cx: &SensorCtx) -> ClickResult {
        cx.platform.open(TASK_MANAGER);
        ClickResult::Handled
    }

    fn accessible_value(&self, _cx: &SensorCtx) -> String {
        value(self.sample.as_ref())
    }

    fn topics(&self) -> Vec<TopicRequest> {
        topics(self.props.interval)
    }

    fn on_sample(&mut self, _topic: &Topic, sample: &Sample, _cx: &SensorCtx) -> bool {
        let Some(cpu) = cpu(sample) else {
            return false;
        };
        self.history.push(cpu.total);
        self.sample = Some(cpu.clone());
        true
    }
}

pub struct CpuDualType;

impl SensorType for CpuDualType {
    fn id(&self) -> &'static str {
        "cpu-graph-dual"
    }

    fn category(&self) -> Category {
        Category::System
    }

    fn name(&self) -> &'static str {
        "sensor.cpuDual.name"
    }

    fn description(&self) -> &'static str {
        "sensor.cpuDual.description"
    }

    fn requires(&self) -> &'static [Capability] {
        &[Capability::PerCoreCpu]
    }

    fn legacy_types(&self) -> &'static [&'static str] {
        &["DockCPUGraphDual"]
    }

    fn legacy_keys(&self) -> &'static [LegacyKey] {
        DUAL_LEGACY_KEYS
    }

    /// SPEC §9.9: both halves show the one processor.
    fn picker_note(&self, ctx: &DefaultsCtx) -> Option<&'static str> {
        (ctx.platform.logical_processors() == 1).then_some("sensor.cpuDual.singleProcessor")
    }

    fn schema(&self) -> &'static [PropField] {
        DUAL_SCHEMA
    }

    fn create(&self, props: &Props, _state: &State) -> serde_json::Result<Box<dyn Sensor>> {
        let props: DualProps = typed(props)?;
        let columns = props.plot.plot_width() as usize;
        Ok(Box::new(CpuDual {
            props,
            sample: None,
            top: History::new(columns),
            bottom: History::new(columns),
            text: None,
        }))
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DualProps {
    interval: u64,
    #[serde(flatten)]
    plot: PlotStyle,
    top_series: String,
    bottom_series: String,
    top_color: ColorChoice,
    top_opacity: u32,
    bottom_color: ColorChoice,
    bottom_opacity: u32,
    flip_bottom: bool,
}

struct CpuDual {
    props: DualProps,
    sample: Option<CpuSample>,
    top: History,
    bottom: History,
    text: Option<GraphText>,
}

impl Sensor for CpuDual {
    fn measure(&mut self, cx: &LayoutCtx) -> f32 {
        let label = t("sensor.cpu.labelDual");
        let text = GraphText::new(
            cx.text,
            cx.theme,
            &label,
            &value(self.sample.as_ref()),
            &percent(100.0),
        );
        let width = text.sensor_width(self.props.plot.plot_width());
        self.text = Some(text);
        width
    }

    fn draw(&self, canvas: &mut dyn Canvas, cx: &DrawCtx) {
        let p = &self.props;
        let plot = graph::plot_rect(cx.rect, p.plot.plot_width());
        graph::draw_backdrop(canvas, plot, p.plot.backdrop());
        let top = Series {
            history: &self.top,
            max: 100.0,
            columns: graph::columns(&p.top_color, p.top_opacity, cx.theme),
        };
        let bottom = Series {
            history: &self.bottom,
            max: 100.0,
            columns: graph::columns(&p.bottom_color, p.bottom_opacity, cx.theme),
        };
        dual_graph::draw(canvas, plot, &top, &bottom, p.flip_bottom);
        if let Some(text) = &self.text {
            text.draw(canvas, cx);
        }
    }

    fn hint(&self, cx: &SensorCtx) -> Option<HintContent> {
        shared_hint(self.sample.as_ref(), cx)
    }

    fn click(&mut self, cx: &SensorCtx) -> ClickResult {
        cx.platform.open(TASK_MANAGER);
        ClickResult::Handled
    }

    fn accessible_value(&self, _cx: &SensorCtx) -> String {
        value(self.sample.as_ref())
    }

    fn topics(&self) -> Vec<TopicRequest> {
        topics(self.props.interval)
    }

    fn on_sample(&mut self, _topic: &Topic, sample: &Sample, _cx: &SensorCtx) -> bool {
        let Some(cpu) = cpu(sample) else {
            return false;
        };
        self.top.push(series_value(&self.props.top_series, cpu));
        self.bottom
            .push(series_value(&self.props.bottom_series, cpu));
        self.sample = Some(cpu.clone());
        true
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::sensors::test_support::{Harness, created};

    fn cpu(total: f32) -> Sample {
        Sample::Cpu(CpuSample {
            total,
            user: 25.1,
            kernel: 11.94,
            idle: 63.0,
            cores: Vec::new(),
            logical: 8,
        })
    }

    #[test]
    fn meter_shows_the_total() {
        let h = Harness::new();
        let mut meter = created(&CpuMeterType, json!({ "interval": 2 }));
        assert_eq!(meter.topics()[0].interval, Duration::from_secs(2));
        assert!(meter.on_sample(&Topic::Cpu, &cpu(37.4), &h.ctx()));
        let (width, dump) = h.drawn(meter.as_mut());
        assert_eq!(width, 28.0);
        let lines: Vec<&str> = dump.lines().collect();
        // The bar 3..25; 37.4 % of 22 px is 8.2 px, so the fill starts at 16.8 → 17.
        assert_eq!(lines[1], "fill rect 0,17 4×8 #FFFFFF/59%");
        assert!(lines[3].starts_with("text \"37%\""), "{dump}");
    }

    #[test]
    fn graph_plots_the_history() {
        let h = Harness::new();
        let mut graph = created(&CpuGraphType, json!({ "width": 57 }));
        for total in [100.0, 50.0] {
            graph.on_sample(&Topic::Cpu, &cpu(total), &h.ctx());
        }
        let (width, dump) = h.drawn(graph.as_mut());
        // A 55 px plot (57 rounds down), the gap, and a column for "100%".
        assert_eq!(width, 55.0 + 5.0 + 20.0);
        let lines: Vec<&str> = dump.lines().collect();
        // Transparent background and grid; the default custom green at 55 %.
        assert_eq!(lines[0], "fill rect 54,14 1×12 #00FF00/55%");
        assert_eq!(lines[1], "fill rect 53,2 1×24 #00FF00/55%");
        assert!(lines[2].starts_with("text \"CPU\""), "{dump}");
        assert!(lines[3].starts_with("text \"50%\""), "{dump}");
    }

    #[test]
    fn dynamic_graph_colour_and_backdrop() {
        let h = Harness::new();
        let mut graph = created(
            &CpuGraphType,
            json!({
                "graphColor": { "mode": "dynamic", "color": "#00FF00" },
                "bkOpacity": 50,
                "gridOpacity": 100,
            }),
        );
        graph.on_sample(&Topic::Cpu, &cpu(100.0), &h.ctx());
        let (_, dump) = h.drawn(graph.as_mut());
        assert!(
            dump.starts_with("fill rect 0,2 50×24 #000000/50%\n"),
            "{dump}"
        );
        assert!(dump.contains("#A0FF9B"), "{dump}");
        assert!(dump.contains("fill rect 49,2 1×24 #FF0000/55%"), "{dump}");
    }

    #[test]
    fn dual_graph_shows_two_series() {
        let h = Harness::new();
        let mut dual = created(&CpuDualType, json!({ "bottomSeries": "core7" }));
        let sample = Sample::Cpu(CpuSample {
            total: 30.0,
            cores: vec![100.0, 50.0],
            logical: 2,
            ..CpuSample::default()
        });
        dual.on_sample(&Topic::Cpu, &sample, &h.ctx());
        let (width, dump) = h.drawn(dual.as_mut());
        assert_eq!(width, 75.0);
        let lines: Vec<&str> = dump.lines().collect();
        // Halves of 11 px (2..13 and 15..26): core0 at 100 % fills the top one up from the
        // middle; core7 does not exist here, so the bottom shows the total (30 %, 3.3 px) down
        // from the middle.
        assert_eq!(lines[0], "fill rect 49,2 1×11 #20B7FF/86%");
        assert_eq!(lines[1], "fill rect 49,15 1×3 #FFAD5B/86%");
        assert!(lines[2].starts_with("text \"CPUs\""), "{dump}");
        let user = CpuSample {
            user: 7.0,
            ..CpuSample::default()
        };
        assert_eq!(series_value("user", &user), 7.0);
        let unread = CpuSample {
            total: 30.0,
            cores: vec![f32::NAN, 50.0],
            ..CpuSample::default()
        };
        assert_eq!(series_value("core0", &unread), 30.0, "could not be read");
        assert_eq!(series_value("core1", &unread), 50.0, "keeps its place");
    }

    #[test]
    fn hint_and_click() {
        let h = Harness::new();
        let mut meter = created(&CpuMeterType, json!({}));
        assert!(meter.hint(&h.ctx()).is_none(), "nothing read yet");
        meter.on_sample(&Topic::Cpu, &cpu(37.0), &h.ctx());
        assert_eq!(
            meter.hint(&h.ctx()),
            Some(HintContent::Text(
                "8 CPUs use\n   activity:\t37.0%\n   by user tasks:\t25.1%\n   \
                 by system:\t11.9%\n   idle:\t63.0%\nClick to open Task Manager"
                    .into()
            ))
        );
        assert_eq!(meter.click(&h.ctx()), ClickResult::Handled);
        assert_eq!(h.platform.log(), ["open taskmgr.exe"]);
        let one = CpuSample {
            logical: 1,
            user: 80.0,
            kernel: 40.0,
            ..CpuSample::default()
        };
        let text = hint(&one, Numbers::default());
        assert!(text.starts_with("1 CPU uses"), "{text}");
        assert!(text.contains("by system:\t20.0%"), "clamped: {text}");
        // Locked with clicks only while Alt is held: no click line (SPEC §6.1).
        let mut h = h;
        h.settings.locked = true;
        h.settings.clicks_when_locked = crate::config::model::ClicksWhenLocked::AltOnly;
        let Some(HintContent::Text(locked)) = meter.hint(&h.ctx()) else {
            panic!("a hint");
        };
        assert!(locked.ends_with("idle:\t63.0%"), "{locked}");
    }
}
