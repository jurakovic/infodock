//! `battery` — Battery Life (SPEC §9.14): a layered battery icon whose level shows the charge,
//! with the state above the percentage or the time left. A click switches between the two for
//! the current state and saves it; while charging it can calculate the time to full itself and
//! tell, once per charge, that the charger can come off.

use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::Value;

use super::common::icon::{Icon, IconSize};
use super::common::icon_label::GAP;
use super::common::two_line::{Align, TwoLine};
use super::format;
use super::legacy::LegacyKey;
use super::schema::{FieldKind, Lit, PropField, SelectOption};
use super::{
    Capability, Category, ClickResult, DrawCtx, HintContent, LayoutCtx, Props, Sensor, SensorCtx,
    SensorType, State, typed,
};
use crate::i18n::{t, t_args};
use crate::look::theme::Inactive;
use crate::platform::{BatteryState, Bitmap, Canvas, Color, RectF};
use crate::sampler::{BATTERY_EVERY, BatterySample, Sample, Topic, TopicRequest};

/// The icon layers (Appendix C), in a 19×23 view box.
const BACK: &[u8] = include_bytes!("../../assets/icons/battery-back.svg");
const FRONT: &[u8] = include_bytes!("../../assets/icons/battery-front.svg");
const CHARGING: &[u8] = include_bytes!("../../assets/icons/battery-charging.svg");
const CHARGED: &[u8] = include_bytes!("../../assets/icons/battery-charged.svg");
const NONE: &[u8] = include_bytes!("../../assets/icons/battery-none.svg");
/// The icon's drawn size, logical px (SPEC §9.14), and the level's area inside it.
const ICON: (f32, f32) = (19.0, 23.0);
const LEVEL: RectF = RectF {
    left: 4.3,
    top: 4.8,
    right: 14.7,
    bottom: 20.4,
};
/// Below this percentage the battery is low.
const LOW: f32 = 15.0;
/// The level cap is hidden above this share of the level area.
const CAP_UNTIL: f32 = 0.88;
/// Smoothing of the time to empty (a moving average) and of the calculated time to full.
const EMPTY_ALPHA: f32 = 0.1;
const FULL_STEPS: f32 = 30.0;

/// Select options: `$head`s, then percentages labelled `80%` and so on.
macro_rules! levels {
    ($($head:expr,)* ; $($value:literal),*) => {
        &[$($head,)* $(SelectOption {
            value: Lit::Int($value),
            label: concat!("sensor.battery.level.", $value),
        }),*]
    };
}

const SCHEMA: &[PropField] = &[
    PropField::new(
        "battery",
        FieldKind::Battery,
        "sensor.battery.field.battery",
        Lit::Int(0),
    ),
    PropField::new(
        "onBatteryShow",
        FieldKind::Select {
            options: &[
                SelectOption {
                    value: Lit::Str("time"),
                    label: "sensor.battery.show.timeLeft",
                },
                SelectOption {
                    value: Lit::Str("percent"),
                    label: "sensor.battery.show.percent",
                },
            ],
        },
        "sensor.battery.field.onBatteryShow",
        Lit::Str("percent"),
    )
    .describe("sensor.battery.field.show.help"),
    PropField::new(
        "smoothTimeToEmpty",
        FieldKind::Bool,
        "sensor.battery.field.smoothTimeToEmpty",
        Lit::Bool(false),
    )
    .describe("sensor.battery.field.smoothTimeToEmpty.help"),
    PropField::new(
        "chargingShow",
        FieldKind::Select {
            options: &[
                SelectOption {
                    value: Lit::Str("time"),
                    label: "sensor.battery.show.timeToFull",
                },
                SelectOption {
                    value: Lit::Str("percent"),
                    label: "sensor.battery.show.percent",
                },
            ],
        },
        "sensor.battery.field.chargingShow",
        Lit::Str("percent"),
    ),
    PropField::new(
        "timeToFullSource",
        FieldKind::Select {
            options: &[
                SelectOption {
                    value: Lit::Str("os"),
                    label: "sensor.battery.timeToFull.os",
                },
                SelectOption {
                    value: Lit::Str("calculated"),
                    label: "sensor.battery.timeToFull.calculated",
                },
            ],
        },
        "sensor.battery.field.timeToFullSource",
        Lit::Str("os"),
    )
    .describe("sensor.battery.field.timeToFullSource.help"),
    PropField::new(
        "assumeChargedAt",
        FieldKind::Select {
            options: levels!(
                SelectOption {
                    value: Lit::Str("os"),
                    label: "sensor.battery.assume.os",
                },
                ;
                95, 96, 97, 98, 99, 100
            ),
        },
        "sensor.battery.field.assumeChargedAt",
        Lit::Str("os"),
    )
    .describe("sensor.battery.field.assumeChargedAt.help"),
    PropField::new(
        "alertAbove",
        FieldKind::Select {
            options: levels!(
                SelectOption {
                    value: Lit::Str("never"),
                    label: "sensor.battery.alert.never",
                },
                ;
                80, 85, 90, 91, 92, 93, 94, 95
            ),
        },
        "sensor.battery.field.alertAbove",
        Lit::Str("never"),
    )
    .describe("sensor.battery.field.alertAbove.help"),
];

/// SPEC §9.14: `-1` is the OS's own value or never.
const LEGACY_KEYS: &[LegacyKey] = &[
    LegacyKey::new("battery", "battery"),
    LegacyKey::options(
        "when-in-use-display-percent",
        "onBatteryShow",
        &[("no", Lit::Str("time")), ("yes", Lit::Str("percent"))],
    ),
    LegacyKey::new("smooth-time-to-empty", "smoothTimeToEmpty"),
    LegacyKey::options(
        "when-charging-display-percent",
        "chargingShow",
        &[("no", Lit::Str("time")), ("yes", Lit::Str("percent"))],
    ),
    LegacyKey::options(
        "calculate-time-to-full",
        "timeToFullSource",
        &[("no", Lit::Str("os")), ("yes", Lit::Str("calculated"))],
    ),
    LegacyKey::options(
        "assume-charged",
        "assumeChargedAt",
        &[("-1", Lit::Str("os"))],
    ),
    LegacyKey::options("alert-level", "alertAbove", &[("-1", Lit::Str("never"))]),
];

pub struct BatteryType;

impl SensorType for BatteryType {
    fn id(&self) -> &'static str {
        "battery"
    }

    fn category(&self) -> Category {
        Category::System
    }

    fn name(&self) -> &'static str {
        "sensor.battery.name"
    }

    fn description(&self) -> &'static str {
        "sensor.battery.description"
    }

    fn legacy_types(&self) -> &'static [&'static str] {
        &["DockBatteryLife"]
    }

    fn legacy_keys(&self) -> &'static [LegacyKey] {
        LEGACY_KEYS
    }

    fn requires(&self) -> &'static [Capability] {
        &[Capability::Battery]
    }

    fn schema(&self) -> &'static [PropField] {
        SCHEMA
    }

    fn create(&self, props: &Props, _state: &State) -> serde_json::Result<Box<dyn Sensor>> {
        Ok(Box::new(BatterySensor::new(typed(props)?)))
    }
}

/// What a line shows: the percentage or a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Show {
    Time,
    Percent,
}

impl Show {
    fn id(self) -> &'static str {
        match self {
            Self::Time => "time",
            Self::Percent => "percent",
        }
    }

    fn toggled(self) -> Self {
        match self {
            Self::Time => Self::Percent,
            Self::Percent => Self::Time,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Source {
    Os,
    Calculated,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BatteryProps {
    /// The battery's place in the system's list.
    battery: u32,
    on_battery_show: Show,
    smooth_time_to_empty: bool,
    charging_show: Show,
    time_to_full_source: Source,
    /// `"os"` or a percentage at which the battery counts as charged.
    assume_charged_at: Value,
    /// `"never"` or the percentage above which the charge alert shows.
    alert_above: Value,
}

impl BatteryProps {
    /// The percentage at which the battery counts as charged; `None`: as the OS says.
    fn assume_charged_at(&self) -> Option<f32> {
        self.assume_charged_at.as_u64().map(|p| p as f32)
    }

    fn alert_above(&self) -> Option<f32> {
        self.alert_above.as_u64().map(|p| p as f32)
    }
}

/// What the battery is doing, as the sensor shows it (SPEC §9.14).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    NoBattery,
    /// On AC power and charging: the OS says so, or the charge is below *assume charged at*.
    Charging,
    /// On AC power and not charging.
    Full,
    /// On battery.
    Draining {
        low: bool,
    },
}

/// The status of `sample` with *assume charged at* `assume` (`None`: as the OS says).
pub fn status(sample: &BatterySample, assume: Option<f32>) -> Status {
    if !sample.present {
        return Status::NoBattery;
    }
    if !sample.ac_online {
        return Status::Draining {
            // Whole percentages, as shown (SPEC §9.14).
            low: sample.percent.round() < LOW,
        };
    }
    let charging = match assume {
        None => sample.state == BatteryState::Charging,
        Some(at) => sample.percent.round() < at,
    };
    if charging {
        Status::Charging
    } else {
        Status::Full
    }
}

/// The share of the level area shown for `percent`, piecewise for visibility (SPEC §9.14):
/// 0–15 % → 0–30 %, 15–40 % → 30–70 %, 40–100 % → 70–100 %.
fn level_share(percent: f32) -> f32 {
    let p = percent.clamp(0.0, 100.0);
    if p < 15.0 {
        p / 15.0 * 0.30
    } else if p < 40.0 {
        0.30 + (p - 15.0) / 25.0 * 0.40
    } else {
        0.70 + (p - 40.0) / 60.0 * 0.30
    }
}

/// The level's colour (SPEC §9.14): red through yellow to green, `#FF{g}00` below 50 % and
/// `#{r}FF00` from 50 %.
fn level_color(percent: f32) -> Color {
    let p = percent.clamp(0.0, 100.0).round();
    if p < 50.0 {
        Color::rgba(255, (p * 5.1).round() as u8, 0, 1.0)
    } else {
        Color::rgba(((100.0 - p) * 5.1).round() as u8, 255, 0, 1.0)
    }
}

/// Minutes in a duration.
fn minutes(d: Duration) -> f32 {
    d.as_secs_f32() / 60.0
}

/// A charge in progress: since when, from which whole percentage, and the calculated time to
/// full.
#[derive(Debug, Clone, PartialEq)]
struct Charge {
    since: Instant,
    from: f32,
    /// Minutes; `None` until the charge has grown.
    estimate: Option<f32>,
}

/// What the sensor gathers from sample to sample: the charge in progress, when the computer
/// went on battery, the smoothed time to empty, and whether this charge was announced.
#[derive(Debug, Default)]
struct Tracker {
    charge: Option<Charge>,
    /// Whether the last sample was on AC power.
    on_ac: Option<bool>,
    /// When it went on battery; only known when it was seen.
    unplugged: Option<Instant>,
    /// The time on battery at the last sample.
    in_use: Option<Duration>,
    /// Minutes.
    smoothed_empty: Option<f32>,
    alerted: bool,
}

impl Tracker {
    /// Takes a sample at `now` with the status it has, charging up to `target` percent.
    fn update(&mut self, s: &BatterySample, status: Status, target: f32, now: Instant) {
        if self.on_ac == Some(true) && !s.ac_online {
            self.unplugged = Some(now);
        }
        if s.ac_online {
            self.unplugged = None;
        }
        self.on_ac = Some(s.ac_online);
        self.in_use = self.unplugged.map(|at| now.saturating_duration_since(at));

        if status == Status::Charging {
            let p = s.percent.round();
            let os = s.time_to_full.map(minutes);
            let charge = self.charge.get_or_insert(Charge {
                since: now,
                from: p,
                estimate: None,
            });
            // The time per percent so far, for what is left (SPEC §9.14): the first estimate
            // errs long, later ones move a thirtieth of the way.
            if p > charge.from {
                let elapsed = now.saturating_duration_since(charge.since).as_secs_f32();
                let estimate = elapsed / (p - charge.from) * (target - p).max(0.0) / 60.0;
                charge.estimate = Some(match charge.estimate {
                    None => os.map_or(estimate, |os| os.max(estimate)),
                    Some(t) => t + (estimate - t) / FULL_STEPS,
                });
            }
        } else if !s.ac_online {
            // Unplugged: the next charge is a new one. On AC a charge that pauses (full, or
            // Windows holding near full) goes on, so the alert is not repeated.
            self.charge = None;
            self.alerted = false;
        }

        match (status, s.time_to_empty) {
            (Status::Draining { .. }, Some(t)) => {
                let m = minutes(t);
                self.smoothed_empty =
                    Some(self.smoothed_empty.map_or(m, |v| v + EMPTY_ALPHA * (m - v)));
            }
            (Status::Draining { .. }, None) => {}
            _ => self.smoothed_empty = None,
        }
    }
}

/// The icon's layers, decoded and tinted at a physical size.
struct BatteryIcon {
    back: Option<Bitmap>,
    front: Option<Bitmap>,
    overlay: Option<Bitmap>,
    /// The level's share of its area and colour; `None` at 0 % and without a battery.
    level: Option<(f32, Color)>,
    opacity: f32,
}

impl BatteryIcon {
    /// Draws the layers at the left of `rect`, centred vertically: back, level, level cap,
    /// front, overlay.
    fn draw(&self, canvas: &mut dyn Canvas, rect: RectF, logical: (f32, f32)) {
        let (w, h) = logical;
        let left = canvas.snap(rect.left);
        let top = canvas.snap(rect.top + (rect.height() - h) / 2.0);
        let dest = RectF::new(left, top, left + w, top + h);
        if let Some(back) = &self.back {
            canvas.draw_bitmap(back, dest, self.opacity);
        }
        if let Some((share, color)) = self.level {
            let k = h / ICON.1;
            let area = RectF::new(
                left + LEVEL.left * k,
                top + LEVEL.top * k,
                left + LEVEL.right * k,
                top + LEVEL.bottom * k,
            );
            let level_top = canvas.snap(area.bottom - share * area.height());
            let bottom = canvas.snap(area.bottom);
            let color = color.with_alpha(color.a * self.opacity);
            canvas.fill_rect(RectF::new(area.left, level_top, area.right, bottom), color);
            if share <= CAP_UNTIL {
                // The level cap: a lighter edge on top of the level.
                let cap = Color::WHITE.with_alpha(0.55 * self.opacity);
                canvas.fill_rect(
                    RectF::new(area.left, level_top, area.right, level_top + k),
                    cap,
                );
            }
        }
        for layer in [&self.front, &self.overlay].into_iter().flatten() {
            canvas.draw_bitmap(layer, dest, self.opacity);
        }
    }
}

/// What is drawn, after measuring.
struct View {
    icon: BatteryIcon,
    /// The icon's size, logical px.
    icon_size: (f32, f32),
    text: TwoLine,
}

struct BatterySensor {
    props: BatteryProps,
    /// The last sample; `None` before the first.
    sample: Option<BatterySample>,
    track: Tracker,
    /// Props changed by a click, until the bar takes them.
    changed: Option<Props>,
    view: Option<View>,
}

impl BatterySensor {
    fn new(props: BatteryProps) -> Self {
        Self {
            props,
            sample: None,
            track: Tracker::default(),
            changed: None,
            view: None,
        }
    }

    fn status(&self) -> Option<Status> {
        self.sample
            .as_ref()
            .map(|s| status(s, self.props.assume_charged_at()))
    }

    /// The time to empty shown, minutes: the battery's estimate, smoothed if asked.
    fn time_to_empty(&self) -> Option<f32> {
        if self.props.smooth_time_to_empty {
            self.track.smoothed_empty
        } else {
            self.sample.as_ref()?.time_to_empty.map(minutes)
        }
    }

    /// The time to full shown, minutes: the battery's estimate or the calculated one.
    fn time_to_full(&self) -> Option<f32> {
        match self.props.time_to_full_source {
            Source::Os => self.sample.as_ref()?.time_to_full.map(minutes),
            Source::Calculated => self.track.charge.as_ref()?.estimate,
        }
    }

    fn percent(&self) -> String {
        self.sample
            .as_ref()
            .map_or_else(String::new, |s| format::percent(f64::from(s.percent)))
    }

    /// The first line: the state (SPEC §9.14).
    fn label(&self) -> String {
        let key = match self.status() {
            None => return String::new(),
            Some(Status::NoBattery) => "sensor.battery.state.none",
            Some(Status::Charging) => "sensor.battery.state.charging",
            Some(Status::Full) => "sensor.battery.state.full",
            Some(Status::Draining { low: false }) => "sensor.battery.state.inUse",
            Some(Status::Draining { low: true }) => "sensor.battery.state.low",
        };
        t(key)
    }

    /// The second line: the percentage, or the time left or to full (`*` while unknown).
    fn value(&self) -> String {
        let time = |m: Option<f32>| m.map(|m| format::hours_minutes((m * 60.0).round() as u64));
        match self.status() {
            None | Some(Status::NoBattery) => String::new(),
            Some(Status::Draining { .. }) => match self.props.on_battery_show {
                Show::Percent => self.percent(),
                Show::Time => time(self.time_to_empty()).map_or_else(
                    || "*".to_owned(),
                    |t| t_args("sensor.battery.value.left", &[("time", &t)]),
                ),
            },
            Some(Status::Charging) => match self.props.charging_show {
                Show::Percent => self.percent(),
                Show::Time => time(self.time_to_full()).unwrap_or_else(|| "*".to_owned()),
            },
            // Assumed charged: the percentage would contradict *Full*.
            Some(Status::Full) => match self.props.assume_charged_at() {
                None => self.percent(),
                Some(_) => String::new(),
            },
        }
    }

    /// The battery's name, or *Battery {n}*.
    fn name(&self) -> String {
        match self.sample.as_ref().map(|s| s.name.as_str()) {
            Some(name) if !name.is_empty() => name.to_owned(),
            _ => t_args(
                "sensor.battery.numbered",
                &[("n", &(u64::from(self.props.battery) + 1).to_string())],
            ),
        }
    }

    fn hint_text(&self) -> Option<String> {
        let sample = self.sample.as_ref()?;
        let status = self.status()?;
        let mut lines = vec![self.name()];
        let hm = |m: f32| format::hours_minutes((m * 60.0).round() as u64);
        if status == Status::NoBattery {
            lines.push(t("sensor.battery.hint.none"));
            return Some(lines.join("\n"));
        }
        lines.push(t_args(
            "sensor.battery.hint.capacity",
            &[("percent", &self.percent())],
        ));
        lines.push(match status {
            Status::Draining { .. } => self.time_to_empty().map_or_else(
                || t("sensor.battery.hint.calculatingLeft"),
                |m| t_args("sensor.battery.hint.left", &[("time", &hm(m))]),
            ),
            Status::Charging => self.time_to_full().map_or_else(
                || t("sensor.battery.hint.calculatingFull"),
                |m| t_args("sensor.battery.hint.untilFull", &[("time", &hm(m))]),
            ),
            Status::Full | Status::NoBattery => t("sensor.battery.hint.charged"),
        });
        if !sample.ac_online
            && let Some(in_use) = self.track.in_use
        {
            let time = format::hours_minutes(in_use.as_secs());
            lines.push(t_args("sensor.battery.hint.inUse", &[("time", &time)]));
        }
        Some(lines.join("\n"))
    }

    /// The icon's layers for the current status, `px` physical px tall.
    fn icon(&self, px: u32, tint: Color, opacity: f32) -> BatteryIcon {
        let status = self.status();
        let layer = |bytes: &[u8]| Icon::svg(bytes, px).map(|b| b.tinted(tint));
        let overlay = match status {
            Some(Status::Charging) => Some(CHARGING),
            Some(Status::Full) => Some(CHARGED),
            Some(Status::NoBattery) => Some(NONE),
            Some(Status::Draining { .. }) | None => None,
        };
        let level = self
            .sample
            .as_ref()
            .filter(|s| s.present && s.percent.round() > 0.0)
            .map(|s| (level_share(s.percent), level_color(s.percent)));
        BatteryIcon {
            back: layer(BACK),
            front: layer(FRONT),
            overlay: overlay.and_then(layer),
            level,
            opacity,
        }
    }
}

impl Sensor for BatterySensor {
    fn measure(&mut self, cx: &LayoutCtx) -> f32 {
        // 19×23, smaller only in a bar too low for it.
        let px = IconSize::Battery.physical(cx.theme, cx.scale);
        let logical_h = px as f32 / cx.scale;
        let logical_w = (ICON.0 / ICON.1 * logical_h).ceil();
        let (tint, opacity) = match self.inactive().then_some(cx.theme.inactive) {
            Some(Inactive::Opacity(fade)) => (cx.theme.sensor_text, fade),
            Some(Inactive::Colorize(color)) => (color, 1.0),
            None => (cx.theme.sensor_text, 1.0),
        };
        let view = View {
            icon: self.icon(px, tint, opacity),
            icon_size: (logical_w, logical_h),
            text: TwoLine::new(cx.text, cx.theme, &self.label(), &self.value()),
        };
        let width = (logical_w + GAP + view.text.width()).ceil();
        self.view = Some(view);
        width
    }

    fn draw(&self, canvas: &mut dyn Canvas, cx: &DrawCtx) {
        let Some(view) = &self.view else {
            return;
        };
        view.icon.draw(canvas, cx.rect, view.icon_size);
        let x = cx.rect.left + view.icon_size.0 + GAP;
        view.text.draw(
            canvas,
            cx.theme,
            (cx.rect.top, cx.rect.height()),
            (Align::Left(x), Align::Left(x)),
            cx.theme.sensor_text,
        );
    }

    fn hint(&self, _cx: &SensorCtx) -> Option<HintContent> {
        self.hint_text().map(HintContent::Text)
    }

    /// Switches between the percentage and the time for the current state and saves it.
    fn click(&mut self, _cx: &SensorCtx) -> ClickResult {
        let (key, show) = match self.status() {
            None | Some(Status::NoBattery) => return ClickResult::None,
            Some(Status::Draining { .. }) => ("onBatteryShow", &mut self.props.on_battery_show),
            Some(Status::Charging | Status::Full) => {
                ("chargingShow", &mut self.props.charging_show)
            }
        };
        *show = show.toggled();
        let mut changes = Props::new();
        changes.insert(key.into(), Value::from(show.id()));
        self.changed = Some(changes);
        ClickResult::Handled
    }

    fn take_props(&mut self) -> Option<Props> {
        self.changed.take()
    }

    /// Keeps the charge history unless another battery is chosen.
    fn reconfigure(&mut self, props: &Props) -> bool {
        match typed::<BatteryProps>(props) {
            Ok(props) if props.battery == self.props.battery => {
                self.props = props;
                true
            }
            _ => false,
        }
    }

    fn inactive(&self) -> bool {
        self.status() == Some(Status::NoBattery)
    }

    fn accessible_value(&self, _cx: &SensorCtx) -> String {
        [self.label(), self.value()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn topics(&self) -> Vec<TopicRequest> {
        vec![TopicRequest {
            topic: Topic::Battery(self.props.battery),
            interval: BATTERY_EVERY,
        }]
    }

    fn on_sample(&mut self, _topic: &Topic, sample: &Sample, cx: &SensorCtx) -> bool {
        let Sample::Battery(sample) = sample else {
            return false;
        };
        let before = (self.label(), self.value(), self.hint_text());
        let assume = self.props.assume_charged_at();
        let status = status(sample, assume);
        let target = assume.unwrap_or(100.0);
        self.track.update(sample, status, target, Instant::now());
        self.sample = Some(sample.clone());
        // Once per charge: the charger can come off (SPEC §9.14).
        if status == Status::Charging
            && !self.track.alerted
            && let Some(above) = self.props.alert_above()
            && sample.percent.round() > above
        {
            self.track.alerted = true;
            let percent = format::percent(f64::from(sample.percent));
            cx.platform.notify(
                &t("sensor.battery.name"),
                &t_args("sensor.battery.alert", &[("percent", &percent)]),
            );
        }
        before != (self.label(), self.value(), self.hint_text())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::sensors::test_support::{Harness, draw, measure, resolved};

    fn props(stored: serde_json::Value) -> Props {
        resolved(SCHEMA, stored)
    }

    fn create(stored: serde_json::Value) -> BatterySensor {
        BatterySensor::new(typed(&props(stored)).expect("valid props"))
    }

    fn sample(percent: f32, ac: bool, state: BatteryState) -> BatterySample {
        BatterySample {
            present: true,
            name: "SMP 5B10".into(),
            percent,
            ac_online: ac,
            state,
            time_to_empty: None,
            time_to_full: None,
        }
    }

    fn feed(h: &Harness, sensor: &mut BatterySensor, s: BatterySample) -> bool {
        sensor.on_sample(&Topic::Battery(0), &Sample::Battery(s), &h.ctx())
    }

    #[test]
    fn states_follow_the_power_line_and_assumed_charge() {
        let on_battery = sample(73.0, false, BatteryState::Discharging);
        assert_eq!(status(&on_battery, None), Status::Draining { low: false });
        let low = sample(14.0, false, BatteryState::Discharging);
        assert_eq!(status(&low, None), Status::Draining { low: true });
        let charging = sample(73.0, true, BatteryState::Charging);
        assert_eq!(status(&charging, None), Status::Charging);
        // Held at 80 % by a charge limit: the OS says it is not charging.
        let held = sample(80.0, true, BatteryState::NotCharging);
        assert_eq!(status(&held, None), Status::Full);
        // Assumed charged at 95 %: below it counts as charging, from it as full.
        assert_eq!(status(&held, Some(95.0)), Status::Charging);
        let nearly = sample(96.0, true, BatteryState::Charging);
        assert_eq!(status(&nearly, Some(95.0)), Status::Full);
        let gone = BatterySample {
            present: false,
            ..held
        };
        assert_eq!(status(&gone, None), Status::NoBattery);
    }

    #[test]
    fn the_level_is_piecewise_and_coloured_by_charge() {
        let close = |a: f32, b: f32| (a - b).abs() < 1e-5;
        for (percent, share) in [
            (0.0, 0.0),
            (15.0, 0.30),
            (27.5, 0.50),
            (40.0, 0.70),
            (100.0, 1.0),
        ] {
            assert!(close(level_share(percent), share), "{percent}");
        }
        assert_eq!(level_color(0.0), Color::rgba(255, 0, 0, 1.0));
        assert_eq!(level_color(25.0), Color::rgba(255, 128, 0, 1.0));
        assert_eq!(level_color(50.0), Color::rgba(255, 255, 0, 1.0));
        assert_eq!(level_color(100.0), Color::rgba(0, 255, 0, 1.0));
    }

    #[test]
    fn labels_values_and_the_icon() {
        let h = Harness::new();
        let mut battery = create(json!({}));
        assert_eq!(
            (battery.label(), battery.value()),
            (String::new(), String::new())
        );
        assert!(feed(
            &h,
            &mut battery,
            sample(73.0, false, BatteryState::Discharging)
        ));
        assert_eq!(
            (battery.label().as_str(), battery.value().as_str()),
            ("In Use", "73%")
        );
        assert!(!battery.inactive());
        // 19 + 3 + "In Use" (6 × 5 px).
        assert_eq!(measure(&mut battery), 52.0);
        let dump = draw(&battery, 52.0);
        let lines: Vec<&str> = dump.lines().collect();
        // Centred: 2.5 px from the top, on whole px.
        assert_eq!(lines[0], "bitmap 19×23 into 7,3 19×23 opacity 1", "{dump}");
        // The level: 70 % + 33/60 × 30 % of the 15.6 px area, in its colour; then the cap.
        assert_eq!(lines[1], "fill rect 11.3,10 10.4×13 #8AFF00", "{dump}");
        assert_eq!(lines[2], "fill rect 11.3,10 10.4×1 #FFFFFF/55%", "{dump}");
        assert_eq!(
            lines[3], "bitmap 19×23 into 7,3 19×23 opacity 1",
            "front: {dump}"
        );
        assert!(
            lines[4].contains("\"In Use\"") && lines[4].contains(" at 29,"),
            "{dump}"
        );

        feed(
            &h,
            &mut battery,
            sample(10.0, false, BatteryState::Discharging),
        );
        assert_eq!(battery.label(), "Low Battery");
        feed(&h, &mut battery, sample(60.0, true, BatteryState::Charging));
        assert_eq!(
            (battery.label().as_str(), battery.value().as_str()),
            ("Charging", "60%")
        );
        feed(&h, &mut battery, sample(100.0, true, BatteryState::Full));
        assert_eq!(
            (battery.label().as_str(), battery.value().as_str()),
            ("Full", "100%")
        );
        // Assumed charged: no percentage next to Full.
        let mut assumed = create(json!({ "assumeChargedAt": 95 }));
        feed(&h, &mut assumed, sample(97.0, true, BatteryState::Charging));
        assert_eq!(
            (assumed.label().as_str(), assumed.value().as_str()),
            ("Full", "")
        );
        // No battery: inactive, crossed out, no level.
        let mut none = create(json!({ "battery": 1 }));
        feed(
            &h,
            &mut none,
            BatterySample {
                present: false,
                ..sample(0.0, true, BatteryState::Unknown)
            },
        );
        assert_eq!(none.label(), "No Battery");
        assert!(none.inactive());
        measure(&mut none);
        let dump = draw(&none, 60.0);
        assert!(!dump.contains("fill rect"), "{dump}");
        assert!(
            dump.starts_with("bitmap 19×23 into 7,3 19×23 opacity 0.78"),
            "{dump}"
        );
    }

    #[test]
    fn times_left_and_to_full() {
        let h = Harness::new();
        let mut battery = create(json!({ "onBatteryShow": "time", "chargingShow": "time" }));
        feed(
            &h,
            &mut battery,
            sample(73.0, false, BatteryState::Discharging),
        );
        assert_eq!(battery.value(), "*", "unknown yet");
        let left = BatterySample {
            time_to_empty: Some(Duration::from_secs(2 * 3600 + 41 * 60)),
            ..sample(73.0, false, BatteryState::Discharging)
        };
        feed(&h, &mut battery, left);
        assert_eq!(battery.value(), "~ 2:41");
        let to_full = BatterySample {
            time_to_full: Some(Duration::from_secs(52 * 60)),
            ..sample(73.0, true, BatteryState::Charging)
        };
        feed(&h, &mut battery, to_full);
        assert_eq!(battery.value(), "0:52");
        assert_eq!(
            battery.hint_text().as_deref(),
            Some("SMP 5B10\nCapacity is 73%\n0:52 Until Full")
        );
    }

    #[test]
    fn the_time_to_full_can_be_calculated() {
        let mut track = Tracker::default();
        let t0 = Instant::now();
        let at = |p: f32, os: Option<u64>| BatterySample {
            time_to_full: os.map(|m| Duration::from_secs(m * 60)),
            ..sample(p, true, BatteryState::Charging)
        };
        let min = |m: u64| t0 + Duration::from_secs(m * 60);
        track.update(&at(50.0, Some(20)), Status::Charging, 100.0, t0);
        assert_eq!(track.charge.as_ref().and_then(|c| c.estimate), None);
        // 2 % in 4 minutes: 2 minutes per percent, 48 % to go: 96 minutes, longer than the
        // OS's 20.
        track.update(&at(52.0, Some(20)), Status::Charging, 100.0, min(4));
        assert_eq!(track.charge.as_ref().and_then(|c| c.estimate), Some(96.0));
        // Then a thirtieth of the way to each new estimate: 3 % in 9 minutes, 47 % to go is
        // 141 minutes; 96 + 45/30 = 97.5.
        track.update(&at(53.0, None), Status::Charging, 100.0, min(9));
        assert_eq!(track.charge.as_ref().and_then(|c| c.estimate), Some(97.5));
        // Unplugged: the charge is over.
        track.update(
            &sample(53.0, false, BatteryState::Discharging),
            Status::Draining { low: false },
            100.0,
            min(10),
        );
        assert!(track.charge.is_none());
        assert_eq!(track.in_use, Some(Duration::ZERO), "seen going on battery");
        track.update(
            &sample(52.0, false, BatteryState::Discharging),
            Status::Draining { low: false },
            100.0,
            min(82),
        );
        assert_eq!(track.in_use, Some(Duration::from_secs(72 * 60)));
    }

    #[test]
    fn the_time_to_empty_can_be_smoothed() {
        let mut track = Tracker::default();
        let now = Instant::now();
        let left = |m: u64| BatterySample {
            time_to_empty: Some(Duration::from_secs(m * 60)),
            ..sample(60.0, false, BatteryState::Discharging)
        };
        let draining = Status::Draining { low: false };
        track.update(&left(100), draining, 100.0, now);
        track.update(&left(200), draining, 100.0, now);
        assert_eq!(track.smoothed_empty, Some(110.0));
    }

    #[test]
    fn hint_lines() {
        let h = Harness::new();
        let mut battery = create(json!({}));
        assert_eq!(battery.hint(&h.ctx()), None);
        feed(
            &h,
            &mut battery,
            sample(73.0, false, BatteryState::Discharging),
        );
        assert_eq!(
            battery.hint_text().as_deref(),
            Some("SMP 5B10\nCapacity is 73%\nCalculating Time Remaining")
        );
        feed(&h, &mut battery, sample(100.0, true, BatteryState::Full));
        assert_eq!(
            battery.hint_text().as_deref(),
            Some("SMP 5B10\nCapacity is 100%\nBattery Is Charged")
        );
        // Unplugged while watched: the time on battery joins.
        let unplugged = BatterySample {
            time_to_empty: Some(Duration::from_secs(161 * 60)),
            ..sample(99.0, false, BatteryState::Discharging)
        };
        feed(&h, &mut battery, unplugged);
        assert_eq!(
            battery.hint_text().as_deref(),
            Some("SMP 5B10\nCapacity is 99%\n2:41 Remaining\n0:00 Already In Use")
        );
        let unnamed = BatterySample {
            name: String::new(),
            ..sample(50.0, false, BatteryState::Discharging)
        };
        feed(&h, &mut battery, unnamed);
        assert!(
            battery
                .hint_text()
                .is_some_and(|t| t.starts_with("Battery 1\n"))
        );
    }

    #[test]
    fn a_click_toggles_and_saves_what_the_state_shows() {
        let h = Harness::new();
        let mut battery = create(json!({}));
        assert_eq!(
            battery.click(&h.ctx()),
            ClickResult::None,
            "nothing known yet"
        );
        feed(
            &h,
            &mut battery,
            sample(73.0, false, BatteryState::Discharging),
        );
        assert_eq!(battery.click(&h.ctx()), ClickResult::Handled);
        assert_eq!(battery.value(), "*");
        assert_eq!(
            battery.take_props(),
            Some(props(json!({ "onBatteryShow": "time" }))).map(|p| p
                .into_iter()
                .filter(|(k, _)| k == "onBatteryShow")
                .collect())
        );
        assert_eq!(battery.take_props(), None, "taken once");
        // Charging toggles its own line.
        feed(&h, &mut battery, sample(74.0, true, BatteryState::Charging));
        battery.click(&h.ctx());
        assert_eq!(
            battery.take_props(),
            Some(
                json!({ "chargingShow": "time" })
                    .as_object()
                    .cloned()
                    .unwrap_or_default()
            )
        );
        // The saved props come back: taken in place, keeping the history.
        let saved = props(json!({ "onBatteryShow": "time", "chargingShow": "time" }));
        assert!(battery.reconfigure(&saved));
        assert!(battery.track.charge.is_some());
        assert!(
            !battery.reconfigure(&props(json!({ "battery": 1 }))),
            "another battery"
        );
    }

    #[test]
    fn the_alert_shows_once_per_charge() {
        let h = Harness::new();
        let mut battery = create(json!({ "alertAbove": 90 }));
        feed(&h, &mut battery, sample(90.0, true, BatteryState::Charging));
        assert!(h.platform.log().is_empty());
        feed(&h, &mut battery, sample(91.0, true, BatteryState::Charging));
        feed(&h, &mut battery, sample(92.0, true, BatteryState::Charging));
        assert_eq!(
            h.platform.log(),
            ["notify Battery: Battery at 91% — you can unplug the charger"]
        );
        // Full and charging again on AC (99 % ↔ 100 %): the same charge, no new alert.
        feed(&h, &mut battery, sample(100.0, true, BatteryState::Full));
        feed(&h, &mut battery, sample(99.0, true, BatteryState::Charging));
        assert_eq!(h.platform.log().len(), 1);
        // Unplugged and charging again: a new charge, a new alert.
        feed(
            &h,
            &mut battery,
            sample(91.0, false, BatteryState::Discharging),
        );
        feed(&h, &mut battery, sample(92.0, true, BatteryState::Charging));
        assert_eq!(h.platform.log().len(), 2);
    }

    #[test]
    fn reads_its_battery() {
        let battery = create(json!({ "battery": 1 }));
        assert_eq!(
            battery.topics(),
            [TopicRequest {
                topic: Topic::Battery(1),
                interval: BATTERY_EVERY,
            }]
        );
        assert_eq!(BatteryType.requires(), [Capability::Battery]);
    }
}
