//! `network-graph` — a network connection's traffic (SPEC §9.13) on the `net/<id|auto>` topic:
//! receive above the middle and send below it, each scaled to its configured maximum or the
//! largest rate in view, with both rates beside the plot. A click opens Network Connections.

use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

use super::common::dual_graph::{self, Series};
use super::common::graph::{self, GraphText, PlotStyle};
use super::common::history::History;
use super::format::{Numbers, rate};
use super::schema::{FieldKind, Lit, PropField};
use super::{
    Category, ClickResult, ColorChoice, DrawCtx, HintContent, LayoutCtx, Props, Sensor, SensorCtx,
    SensorType, State, typed,
};
use crate::i18n::{t, t_args};
use crate::platform::Canvas;
use crate::sampler::{NetSample, Sample, Topic, TopicRequest};

/// Network Connections, opened by a click (SPEC §9.13: `control.exe ncpa.cpl`).
const NETWORK_CONNECTIONS: &str = "ncpa.cpl";
/// The smallest scale, bytes per second (SPEC §9.13: 1 KB/s).
const MIN_SCALE: f32 = 1024.0;

const SCHEMA: &[PropField] = &[
    PropField::new(
        "adapter",
        FieldKind::NetAdapter,
        "sensor.network.field.adapter",
        Lit::Null,
    )
    .describe("sensor.network.field.adapter.help"),
    PropField::new(
        "bandwidth",
        FieldKind::Text {
            empty_is_default: false,
        },
        "sensor.network.field.bandwidth",
        Lit::Str(""),
    )
    .describe("sensor.network.field.bandwidth.help"),
    graph::INTERVAL,
    graph::WIDTH,
    graph::BK_COLOR,
    graph::BK_OPACITY,
    graph::GRID_COLOR,
    graph::GRID_OPACITY,
    PropField::new(
        "topColor",
        FieldKind::ColorMode { dynamic: true },
        "sensor.network.field.topColor",
        Lit::ColorMode("custom", "#20B7FF"),
    ),
    PropField::new(
        "topOpacity",
        FieldKind::Opacity { min: 20 },
        "sensor.network.field.topOpacity",
        Lit::Int(86),
    ),
    PropField::new(
        "bottomColor",
        FieldKind::ColorMode { dynamic: true },
        "sensor.network.field.bottomColor",
        Lit::ColorMode("custom", "#FFAD5B"),
    ),
    PropField::new(
        "bottomOpacity",
        FieldKind::Opacity { min: 20 },
        "sensor.network.field.bottomOpacity",
        Lit::Int(86),
    ),
    PropField::new(
        "flipBottom",
        FieldKind::Bool,
        "sensor.network.field.flipBottom",
        Lit::Bool(true),
    )
    .describe("sensor.network.field.flipBottom.help"),
];

/// The configured maxima (SPEC §9.13), bytes per second: `"in,out"`, or one value for both.
/// Anything that is not a positive number scales automatically.
pub fn bandwidth(text: &str) -> (Option<f32>, Option<f32>) {
    let parse = |s: &str| {
        s.trim()
            .parse::<f32>()
            .ok()
            .filter(|v| v.is_finite() && *v > 0.0)
    };
    match text.split_once(',') {
        Some((rx, tx)) => (parse(rx), parse(tx)),
        None => {
            let both = parse(text);
            (both, both)
        }
    }
}

/// A series' scale (SPEC §9.13): the configured maximum, the largest rate in view or 1 KB/s,
/// whichever is largest.
pub fn scale(configured: Option<f32>, history: &History) -> f32 {
    let seen = history
        .iter()
        .filter(|v| v.is_finite())
        .fold(0.0f32, f32::max);
    configured.unwrap_or(0.0).max(seen).max(MIN_SCALE)
}

pub struct NetworkGraphType;

impl SensorType for NetworkGraphType {
    fn id(&self) -> &'static str {
        "network-graph"
    }

    fn category(&self) -> Category {
        Category::Network
    }

    fn name(&self) -> &'static str {
        "sensor.networkGraph.name"
    }

    fn description(&self) -> &'static str {
        "sensor.networkGraph.description"
    }

    fn legacy_types(&self) -> &'static [&'static str] {
        &["DockNetwork"]
    }

    fn schema(&self) -> &'static [PropField] {
        SCHEMA
    }

    fn create(&self, props: &Props, _state: &State) -> serde_json::Result<Box<dyn Sensor>> {
        let props: NetworkProps = typed(props)?;
        let columns = props.plot.plot_width() as usize;
        let maxima = bandwidth(&props.bandwidth);
        Ok(Box::new(NetworkGraph {
            props,
            maxima,
            sample: None,
            rx: History::new(columns),
            tx: History::new(columns),
            text: None,
        }))
    }

    fn summary(&self, props: &Props) -> Option<String> {
        Some(adapter_name(props.get("adapter").and_then(adapter)))
    }
}

/// A chosen adapter, as stored: `{ id, name }`; `null` is Automatic.
#[derive(Debug, Clone, PartialEq, Deserialize)]
struct Adapter {
    id: String,
    name: String,
}

fn adapter(value: &Value) -> Option<Adapter> {
    serde_json::from_value(value.clone()).ok()
}

fn adapter_name(adapter: Option<Adapter>) -> String {
    adapter.map_or_else(|| t("sensor.network.automatic"), |a| a.name)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NetworkProps {
    adapter: Option<Adapter>,
    bandwidth: String,
    interval: u64,
    #[serde(flatten)]
    plot: PlotStyle,
    top_color: ColorChoice,
    top_opacity: u32,
    bottom_color: ColorChoice,
    bottom_opacity: u32,
    flip_bottom: bool,
}

struct NetworkGraph {
    props: NetworkProps,
    /// The configured maxima of receive and send.
    maxima: (Option<f32>, Option<f32>),
    sample: Option<NetSample>,
    /// Bytes per second received and sent, one per column.
    rx: History,
    tx: History,
    text: Option<GraphText>,
}

impl NetworkGraph {
    fn topic(&self) -> Topic {
        Topic::Net(self.props.adapter.as_ref().map(|a| a.id.clone()))
    }

    /// The receive and send rates; nothing before the first sample.
    fn rates(&self, numbers: Numbers) -> (String, String) {
        match &self.sample {
            Some(s) => (rate(s.rx_rate, numbers), rate(s.tx_rate, numbers)),
            None => Default::default(),
        }
    }

    fn scales(&self) -> (f32, f32) {
        (
            scale(self.maxima.0, &self.rx),
            scale(self.maxima.1, &self.tx),
        )
    }
}

impl Sensor for NetworkGraph {
    fn measure(&mut self, cx: &LayoutCtx) -> f32 {
        // Measuring has no locale; the decimal separator only changes the text, not its width.
        let (rx, tx) = self.rates(Numbers::default());
        // The column fits `1023 MB/s` (SPEC §9.13).
        let reserve = t_args("format.rate.mb", &[("value", "1023")]);
        let text = GraphText::new(cx.text, cx.theme, &rx, &tx, &reserve);
        let width = text.sensor_width(self.props.plot.plot_width());
        self.text = Some(text);
        width
    }

    fn draw(&self, canvas: &mut dyn Canvas, cx: &DrawCtx) {
        let p = &self.props;
        let plot = graph::plot_rect(cx.rect, p.plot.plot_width());
        graph::draw_backdrop(canvas, plot, p.plot.backdrop());
        let (rx_scale, tx_scale) = self.scales();
        let top = Series {
            history: &self.rx,
            max: rx_scale,
            columns: graph::columns(&p.top_color, p.top_opacity, cx.theme),
        };
        let bottom = Series {
            history: &self.tx,
            max: tx_scale,
            columns: graph::columns(&p.bottom_color, p.bottom_opacity, cx.theme),
        };
        dual_graph::draw(canvas, plot, &top, &bottom, p.flip_bottom);
        if let Some(text) = &self.text {
            text.draw(canvas, cx);
        }
    }

    fn hint(&self, cx: &SensorCtx) -> Option<HintContent> {
        let s = self.sample.as_ref()?;
        let numbers = cx.platform.locale(cx.settings.date_locale).numbers();
        let name = if !s.present {
            adapter_name(self.props.adapter.clone())
        } else if s.description.is_empty() || s.description == s.alias {
            s.alias.clone()
        } else {
            t_args(
                "sensor.network.hint.named",
                &[("alias", &s.alias), ("description", &s.description)],
            )
        };
        let status = t(if s.present && s.up {
            "sensor.network.hint.active"
        } else {
            "sensor.network.hint.inactive"
        });
        let (rx_scale, tx_scale) = self.scales();
        let traffic = |value: f64, max: f32| {
            t_args(
                "sensor.network.hint.rate",
                &[
                    ("rate", &rate(value, numbers)),
                    ("max", &rate(f64::from(max), numbers)),
                ],
            )
        };
        let mut text = t_args("sensor.network.hint.adapter", &[("name", &name)]);
        for (label, value) in [
            ("sensor.network.hint.status", status),
            ("sensor.network.hint.receive", traffic(s.rx_rate, rx_scale)),
            ("sensor.network.hint.send", traffic(s.tx_rate, tx_scale)),
        ] {
            text.push_str(&format!("\n   {}\t{value}", t(label)));
        }
        text.push('\n');
        text.push_str(&t("sensor.network.hint.click"));
        Some(HintContent::Text(text))
    }

    fn click(&mut self, cx: &SensorCtx) -> ClickResult {
        cx.platform.open(NETWORK_CONNECTIONS);
        ClickResult::Handled
    }

    fn accessible_value(&self, cx: &SensorCtx) -> String {
        if self.sample.is_none() {
            return String::new();
        }
        let (rx, tx) = self.rates(cx.platform.locale(cx.settings.date_locale).numbers());
        t_args("sensor.network.accessible", &[("rx", &rx), ("tx", &tx)])
    }

    fn topics(&self) -> Vec<TopicRequest> {
        vec![TopicRequest {
            topic: self.topic(),
            interval: Duration::from_secs(self.props.interval.max(1)),
        }]
    }

    fn on_sample(&mut self, topic: &Topic, sample: &Sample, _cx: &SensorCtx) -> bool {
        let Sample::Net(net) = sample else {
            return false;
        };
        if *topic != self.topic() {
            return false;
        }
        self.rx.push(net.rx_rate as f32);
        self.tx.push(net.tx_rate as f32);
        self.sample = Some(net.clone());
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
    use crate::sensors::test_support::{FakeClock, theme};

    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * KB;

    fn create(stored: serde_json::Value) -> Box<dyn Sensor> {
        let stored = stored.as_object().cloned().unwrap_or_default();
        let (props, ignored) = resolve(SCHEMA, &stored, &Props::new());
        assert!(ignored.is_empty(), "{ignored:?}");
        NetworkGraphType
            .create(&props, &State::new())
            .expect("valid props")
    }

    fn net(rx: f64, tx: f64) -> Sample {
        Sample::Net(NetSample {
            present: true,
            id: "WIFI".into(),
            alias: "Wi-Fi".into(),
            description: "Intel(R) Wi-Fi 6E AX211".into(),
            up: true,
            rx_rate: rx,
            tx_rate: tx,
            ..NetSample::default()
        })
    }

    const AUTO: Topic = Topic::Net(None);

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
                id: "s_net",
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
    fn plots_receive_up_and_send_down_scaled_to_the_view() {
        let h = Harness::new();
        let mut graph = create(json!({}));
        assert_eq!(graph.topics()[0].topic, AUTO);
        graph.on_sample(&AUTO, &net(2.0 * MB, 100.0 * KB), &h.ctx());
        graph.on_sample(&AUTO, &net(1.0 * MB, 200.0 * KB), &h.ctx());
        let (width, dump) = h.drawn(graph.as_mut());
        // A 50 px plot, the gap and a column for "1023 MB/s" (9 characters).
        assert_eq!(width, 50.0 + 5.0 + 45.0);
        let lines: Vec<&str> = dump.lines().collect();
        // Halves of 11.5 px. Receive is scaled to its 2 MB/s peak, so the newest 1 MB/s is
        // half of the upper half; send to 200 KB/s, so the newest fills the lower half.
        assert_eq!(lines[0], "fill rect 49,8 1×6 #20B7FF/86%");
        assert_eq!(lines[1], "fill rect 48,2 1×12 #20B7FF/86%");
        assert_eq!(lines[2], "fill rect 49,15 1×11 #FFAD5B/86%");
        assert!(lines[4].starts_with("text \"1.00 MB/s\""), "{dump}");
        assert!(lines[5].starts_with("text \"200 KB/s\""), "{dump}");
        assert!(
            !graph.on_sample(&Topic::Net(Some("ETH".into())), &net(1.0, 1.0), &h.ctx()),
            "another adapter"
        );
    }

    #[test]
    fn configured_maxima_and_the_smallest_scale() {
        assert_eq!(bandwidth(""), (None, None));
        assert_eq!(bandwidth("1000"), (Some(1000.0), Some(1000.0)));
        assert_eq!(
            bandwidth(" 12500000 , 2500000"),
            (Some(12.5e6), Some(2.5e6))
        );
        assert_eq!(bandwidth("fast,-1"), (None, None));
        let mut history = History::new(5);
        assert_eq!(scale(None, &history), 1024.0, "1 KB/s at least");
        history.push(5000.0);
        assert_eq!(scale(None, &history), 5000.0);
        assert_eq!(scale(Some(8000.0), &history), 8000.0);
        assert_eq!(scale(Some(10.0), &history), 5000.0, "the view wins");
    }

    #[test]
    fn hint_and_click() {
        let h = Harness::new();
        let mut graph = create(json!({ "bandwidth": "12373196,2422210" }));
        assert!(graph.hint(&h.ctx()).is_none());
        graph.on_sample(&AUTO, &net(1.24 * MB, 86.0 * KB), &h.ctx());
        assert_eq!(
            graph.hint(&h.ctx()),
            Some(HintContent::Text(
                "Adapter: Wi-Fi (Intel(R) Wi-Fi 6E AX211)\n   Status:\tactive\n   \
                 Receive:\t1.24 MB/s (max. 11.8 MB/s)\n   Send:\t86.0 KB/s (max. 2.31 MB/s)\n\
                 Click to open Network Connections"
                    .into()
            ))
        );
        assert_eq!(
            graph.accessible_value(&h.ctx()),
            "Receive 1.24 MB/s, send 86.0 KB/s"
        );
        assert_eq!(graph.click(&h.ctx()), ClickResult::Handled);
        assert_eq!(h.clock.log(), ["open ncpa.cpl"]);
    }

    #[test]
    fn a_missing_adapter_is_inactive_in_the_hint() {
        let h = Harness::new();
        let chosen = json!({ "adapter": { "id": "ETH", "name": "Ethernet 2" } });
        let mut graph = create(chosen);
        let topic = Topic::Net(Some("ETH".into()));
        assert_eq!(graph.topics()[0].topic, topic);
        graph.on_sample(&topic, &Sample::Net(NetSample::default()), &h.ctx());
        let Some(HintContent::Text(hint)) = graph.hint(&h.ctx()) else {
            panic!("a hint");
        };
        assert!(
            hint.starts_with("Adapter: Ethernet 2\n   Status:\tinactive\n"),
            "{hint}"
        );
        let (props, _) = resolve(SCHEMA, &Props::new(), &Props::new());
        assert_eq!(
            NetworkGraphType.summary(&props).as_deref(),
            Some("Automatic")
        );
    }
}
