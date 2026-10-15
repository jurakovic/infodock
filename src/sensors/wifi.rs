//! `wifi` — Wi-Fi Signal (SPEC §9.15): a signal icon in six levels with the network name above
//! the signal in percent or dBm (or the name). Offline it is inactive. When Windows withholds
//! the network name for want of location access, it says so and a click opens those settings.

use std::time::Duration;

use serde::Deserialize;

use super::common::icon::{self, Icon};
use super::common::icon_label::IconLabel;
use super::common::two_line::TwoLine;
use super::format;
use super::schema::{FieldKind, Lit, PropField, SelectOption};
use super::{
    Capability, Category, ClickResult, DrawCtx, HintContent, LayoutCtx, Props, Sensor, SensorCtx,
    SensorType, State, typed,
};
use crate::i18n::{t, t_args};
use crate::platform::{Canvas, WifiStatus};
use crate::sampler::{Sample, Topic, TopicRequest};

/// The icons by signal level (Appendix C).
const LEVELS: [&[u8]; 6] = [
    include_bytes!("../../assets/icons/wifi-0.svg"),
    include_bytes!("../../assets/icons/wifi-1.svg"),
    include_bytes!("../../assets/icons/wifi-2.svg"),
    include_bytes!("../../assets/icons/wifi-3.svg"),
    include_bytes!("../../assets/icons/wifi-4.svg"),
    include_bytes!("../../assets/icons/wifi-5.svg"),
];
/// The icon is drawn at this opacity of the text colour (SPEC §9.15).
const ICON_OPACITY: f32 = 0.75;
/// What a click opens.
const WIFI_SETTINGS: &str = "ms-settings:network-wifi";
const LOCATION_SETTINGS: &str = "ms-settings:privacy-location";

const SCHEMA: &[PropField] = &[
    PropField::new(
        "title",
        FieldKind::Text {
            empty_is_default: false,
        },
        "sensor.wifi.field.title",
        Lit::Str(""),
    )
    .describe("sensor.wifi.field.title.help"),
    PropField::new(
        "interval",
        FieldKind::Interval {
            min: 1.0,
            max: 30.0,
        },
        "sensor.field.interval",
        Lit::Int(2),
    ),
    PropField::new(
        "display",
        FieldKind::Select {
            options: &[
                SelectOption {
                    value: Lit::Str("name"),
                    label: "sensor.wifi.display.name",
                },
                SelectOption {
                    value: Lit::Str("percent"),
                    label: "sensor.wifi.display.percent",
                },
                SelectOption {
                    value: Lit::Str("dbm"),
                    label: "sensor.wifi.display.dbm",
                },
            ],
        },
        "sensor.wifi.field.display",
        Lit::Str("percent"),
    ),
];

pub struct WifiType;

impl SensorType for WifiType {
    fn id(&self) -> &'static str {
        "wifi"
    }

    fn category(&self) -> Category {
        Category::Network
    }

    fn name(&self) -> &'static str {
        "sensor.wifi.name"
    }

    fn description(&self) -> &'static str {
        "sensor.wifi.description"
    }

    fn legacy_types(&self) -> &'static [&'static str] {
        &["DockWiFi"]
    }

    fn requires(&self) -> &'static [Capability] {
        &[Capability::Wifi]
    }

    fn schema(&self) -> &'static [PropField] {
        SCHEMA
    }

    fn create(&self, props: &Props, _state: &State) -> serde_json::Result<Box<dyn Sensor>> {
        Ok(Box::new(Wifi {
            props: typed(props)?,
            status: None,
            view: None,
        }))
    }

    fn summary(&self, props: &Props) -> Option<String> {
        let title = props.get("title").and_then(|v| v.as_str())?;
        (!title.is_empty()).then(|| title.to_owned())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Display {
    Name,
    Percent,
    Dbm,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WifiProps {
    /// Empty: the network name.
    title: String,
    /// Seconds.
    interval: u64,
    display: Display,
}

/// The signal quality (0–100) Windows gives a signal of `rssi` dBm: −100 dBm and below is 0,
/// −50 dBm and above is 100, linear between.
pub fn quality_from_rssi(rssi: i32) -> u32 {
    (2 * (rssi + 100)).clamp(0, 100) as u32
}

/// The icon level (SPEC §9.15): 0 offline or with the radio off, otherwise 1–5 by quality.
pub fn level(status: &WifiStatus) -> usize {
    if !online(status) {
        return 0;
    }
    match status.quality {
        80.. => 5,
        60.. => 4,
        40.. => 3,
        20.. => 2,
        _ => 1,
    }
}

/// Connected through a radio that is on.
fn online(status: &WifiStatus) -> bool {
    status.available && status.radio_on && status.connected
}

/// The band and channel of a centre frequency in MHz, e.g. 5180 → (`5 GHz`, 36).
pub fn band_channel(mhz: u32) -> Option<(&'static str, u32)> {
    Some(match mhz {
        2484 => ("sensor.wifi.band.2", 14),
        2412..=2472 => ("sensor.wifi.band.2", (mhz - 2407) / 5),
        5000..=5925 => ("sensor.wifi.band.5", (mhz - 5000) / 5),
        // 6 GHz: channel n at 5950 + 5n MHz, and channel 2 below them at 5935 MHz.
        5935 => ("sensor.wifi.band.6", 2),
        5955..=7125 => ("sensor.wifi.band.6", (mhz - 5950) / 5),
        _ => return None,
    })
}

struct Wifi {
    props: WifiProps,
    /// From the `wifi` topic; `None` before its first sample.
    status: Option<WifiStatus>,
    view: Option<IconLabel>,
}

impl Wifi {
    fn online(&self) -> bool {
        self.status.as_ref().is_some_and(online)
    }

    fn location_denied(&self) -> bool {
        self.status
            .as_ref()
            .is_some_and(|s| online(s) && s.location_denied)
    }

    /// The network name, or *Wi-Fi* while Windows withholds it or there is none.
    fn network(&self) -> String {
        self.status
            .as_ref()
            .filter(|s| online(s))
            .and_then(|s| s.ssid.clone())
            .unwrap_or_else(|| t("sensor.wifi.generic"))
    }

    /// The first line: the title, or the network name.
    fn label(&self) -> String {
        if self.props.title.is_empty() {
            self.network()
        } else {
            self.props.title.clone()
        }
    }

    fn percent(status: &WifiStatus) -> String {
        format::percent(f64::from(status.quality))
    }

    /// The second line: `offline`, or the signal as `display` says (the name only below a
    /// title of its own; without a known name or level, the percentage).
    fn value(&self) -> String {
        let Some(status) = self.status.as_ref() else {
            return String::new();
        };
        if !online(status) {
            return t("sensor.wifi.offline");
        }
        match self.props.display {
            Display::Name if !self.props.title.is_empty() => {
                status.ssid.clone().unwrap_or_else(|| Self::percent(status))
            }
            Display::Dbm => status.rssi_dbm.map_or_else(
                || Self::percent(status),
                |dbm| t_args("sensor.wifi.dbm", &[("value", &dbm.to_string())]),
            ),
            Display::Name | Display::Percent => Self::percent(status),
        }
    }

    /// `802.11ax, 5 GHz, channel 36, 1201 Mbps`, as far as known.
    fn info(status: &WifiStatus) -> String {
        let mut parts: Vec<String> = status.phy.iter().map(|p| (*p).to_owned()).collect();
        if let Some((band, channel)) = status.frequency_mhz.and_then(band_channel) {
            parts.push(t(band));
            parts.push(t_args(
                "sensor.wifi.channel",
                &[("channel", &channel.to_string())],
            ));
        }
        if let Some(mbps) = status.link_mbps {
            parts.push(t_args("sensor.wifi.mbps", &[("value", &mbps.to_string())]));
        }
        parts.join(", ")
    }

    fn hint_text(&self, cx: &SensorCtx) -> String {
        let mut lines = vec![t("sensor.wifi.hint.title")];
        match self.status.as_ref() {
            None => {}
            Some(s) if !s.radio_on && s.available => lines.push(t("sensor.wifi.hint.radioOff")),
            Some(s) if !online(s) => lines.push(t("sensor.wifi.hint.offline")),
            Some(s) => {
                let mut rows = Vec::new();
                if let Some(ssid) = &s.ssid {
                    rows.push(("sensor.wifi.hint.network", ssid.clone()));
                }
                let info = Self::info(s);
                if !info.is_empty() {
                    rows.push(("sensor.wifi.hint.info", info));
                }
                if let Some(dbm) = s.rssi_dbm {
                    let value = t_args("sensor.wifi.dbm", &[("value", &dbm.to_string())]);
                    rows.push(("sensor.wifi.hint.level", value));
                }
                rows.push(("sensor.wifi.hint.percent", Self::percent(s)));
                lines.extend(rows.into_iter().map(|(k, v)| format!("{}\t{v}", t(k))));
                if s.location_denied {
                    lines.push(t("sensor.wifi.hint.location"));
                }
            }
        }
        let click = if self.location_denied() {
            "sensor.wifi.hint.clickLocation"
        } else {
            "sensor.wifi.hint.click"
        };
        cx.with_click_line(lines.join("\n"), click)
    }
}

impl Sensor for Wifi {
    fn measure(&mut self, cx: &LayoutCtx) -> f32 {
        let logical = cx.theme.bar_height.saturating_sub(8).max(12) as f32;
        let px = icon::physical(logical, cx.scale);
        let level = self.status.as_ref().map_or(0, level);
        let inactive = self.inactive().then_some(cx.theme.inactive);
        // Built-in and monochrome: in the text colour, at 75 % (SPEC §9.15).
        let icon = Icon::svg(LEVELS[level], px)
            .map(|b| Icon::new(b, Some(cx.theme.sensor_text), ICON_OPACITY, inactive));
        let icon_width = icon
            .as_ref()
            .map_or((px as f32 / cx.scale).ceil(), |i| i.width(cx.scale));
        let text = TwoLine::new(cx.text, cx.theme, &self.label(), &self.value());
        let view = IconLabel::new(icon, icon_width, text);
        let width = view.width();
        self.view = Some(view);
        width
    }

    fn draw(&self, canvas: &mut dyn Canvas, cx: &DrawCtx) {
        if let Some(view) = &self.view {
            view.draw(canvas, cx, cx.theme.sensor_text);
        }
    }

    fn hint(&self, cx: &SensorCtx) -> Option<HintContent> {
        Some(HintContent::Text(self.hint_text(cx)))
    }

    /// Opens the Wi-Fi settings, or the location settings while they withhold the name.
    fn click(&mut self, cx: &SensorCtx) -> ClickResult {
        cx.platform.open(if self.location_denied() {
            LOCATION_SETTINGS
        } else {
            WIFI_SETTINGS
        });
        ClickResult::Handled
    }

    /// Offline (SPEC §8.5).
    fn inactive(&self) -> bool {
        self.status.is_some() && !self.online()
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
            topic: Topic::Wifi,
            interval: Duration::from_secs(self.props.interval.max(1)),
        }]
    }

    fn on_sample(&mut self, _topic: &Topic, sample: &Sample, _cx: &SensorCtx) -> bool {
        let Sample::Wifi(status) = sample else {
            return false;
        };
        if self.status.as_ref() == Some(status) {
            return false;
        }
        self.status = Some(status.clone());
        true
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::config::model::{ClicksWhenLocked, Settings};
    use crate::platform::Platform;
    use crate::sampler::ChangeSink;
    use crate::sensors::schema::resolve;
    use crate::sensors::test_support::{FakeClock, draw, measure, theme};

    fn create(stored: serde_json::Value) -> Wifi {
        let stored = stored.as_object().cloned().unwrap_or_default();
        let (props, ignored) = resolve(SCHEMA, &stored, &Props::new());
        assert!(ignored.is_empty(), "{ignored:?}");
        Wifi {
            props: typed(&props).expect("valid props"),
            status: None,
            view: None,
        }
    }

    fn connected() -> WifiStatus {
        WifiStatus {
            available: true,
            radio_on: true,
            connected: true,
            ssid: Some("HomeNet".into()),
            quality: 78,
            rssi_dbm: Some(-58),
            frequency_mhz: Some(5180),
            link_mbps: Some(1201),
            phy: Some("802.11ax"),
            location_denied: false,
        }
    }

    struct Harness {
        platform: FakeClock,
        settings: Settings,
        theme: crate::bar::theme::Theme,
    }

    impl Harness {
        fn new() -> Self {
            Self {
                platform: FakeClock::default(),
                settings: Settings::default(),
                theme: theme(),
            }
        }

        fn ctx(&self) -> SensorCtx<'_> {
            SensorCtx {
                id: "s_1",
                settings: &self.settings,
                theme: &self.theme,
                locked: self.settings.locked,
                alt: false,
                animate: false,
                platform: &self.platform as &dyn Platform,
                changes: &ChangeSink::NONE,
            }
        }

        fn feed(&self, sensor: &mut Wifi, status: WifiStatus) -> bool {
            sensor.on_sample(&Topic::Wifi, &Sample::Wifi(status), &self.ctx())
        }
    }

    #[test]
    fn levels_bands_and_rssi() {
        let at = |quality| {
            level(&WifiStatus {
                quality,
                ..connected()
            })
        };
        assert_eq!(
            [at(80), at(79), at(60), at(40), at(20), at(19)],
            [5, 4, 4, 3, 2, 1]
        );
        let off = WifiStatus {
            radio_on: false,
            ..connected()
        };
        assert_eq!(level(&off), 0);
        assert_eq!(band_channel(2412), Some(("sensor.wifi.band.2", 1)));
        assert_eq!(band_channel(2484), Some(("sensor.wifi.band.2", 14)));
        assert_eq!(band_channel(5180), Some(("sensor.wifi.band.5", 36)));
        assert_eq!(band_channel(5955), Some(("sensor.wifi.band.6", 1)));
        assert_eq!(band_channel(5935), Some(("sensor.wifi.band.6", 2)));
        assert_eq!(band_channel(5940), None, "between the bands' grids");
        assert_eq!(band_channel(900), None);
        assert_eq!(quality_from_rssi(-58), 84);
        assert_eq!((quality_from_rssi(-110), quality_from_rssi(-40)), (0, 100));
    }

    #[test]
    fn name_above_the_signal() {
        let h = Harness::new();
        let mut wifi = create(json!({}));
        assert!(h.feed(&mut wifi, connected()));
        assert!(!h.feed(&mut wifi, connected()));
        assert_eq!(
            (wifi.label().as_str(), wifi.value().as_str()),
            ("HomeNet", "78%")
        );
        let dbm = create(json!({ "display": "dbm" }));
        let mut dbm = dbm;
        h.feed(&mut dbm, connected());
        assert_eq!(dbm.value(), "-58 dBm");
        // The name below the name would say it twice: the percentage instead.
        let mut name = create(json!({ "display": "name" }));
        h.feed(&mut name, connected());
        assert_eq!(name.value(), "78%");
        let mut titled = create(json!({ "display": "name", "title": "Office" }));
        h.feed(&mut titled, connected());
        assert_eq!(
            (titled.label().as_str(), titled.value().as_str()),
            ("Office", "HomeNet")
        );
        // Fixed metrics: a 20 px icon 24 px wide, 3 px, "HomeNet" (35 px).
        assert_eq!(measure(&mut wifi), 62.0);
        let dump = draw(&wifi, 62.0);
        assert!(
            dump.starts_with("bitmap 24×20 into 7,4 24×20 opacity 0.75\n"),
            "{dump}"
        );
    }

    #[test]
    fn offline_is_inactive() {
        let h = Harness::new();
        let mut wifi = create(json!({}));
        assert!(!wifi.inactive(), "unknown yet");
        let offline = WifiStatus {
            connected: false,
            ssid: None,
            ..connected()
        };
        h.feed(&mut wifi, offline);
        assert!(wifi.inactive());
        assert_eq!(
            (wifi.label().as_str(), wifi.value().as_str()),
            ("Wi-Fi", "offline")
        );
        assert_eq!(
            wifi.hint_text(&h.ctx()),
            "Wi-Fi Signal\nOffline\nClick to open the Wi-Fi settings"
        );
        let radio_off = WifiStatus {
            radio_on: false,
            ..connected()
        };
        h.feed(&mut wifi, radio_off);
        assert!(wifi.inactive());
        assert!(wifi.hint_text(&h.ctx()).contains("\nWi-Fi is turned off\n"));
        measure(&mut wifi);
        assert!(draw(&wifi, 60.0).starts_with("bitmap 24×20 into 7,4 24×20 opacity 0.58"));
    }

    #[test]
    fn hint_and_click() {
        let mut h = Harness::new();
        let mut wifi = create(json!({}));
        h.feed(&mut wifi, connected());
        assert_eq!(
            wifi.hint_text(&h.ctx()),
            "Wi-Fi Signal\nNetwork:\tHomeNet\nInfo:\t802.11ax, 5 GHz, channel 36, 1201 Mbps\n\
             Signal level:\t-58 dBm\nSignal percentage:\t78%\nClick to open the Wi-Fi settings"
        );
        assert_eq!(wifi.click(&h.ctx()), ClickResult::Handled);
        h.settings.locked = true;
        h.settings.clicks_when_locked = ClicksWhenLocked::AltOnly;
        assert!(!wifi.hint_text(&h.ctx()).contains("Click"));
        assert_eq!(h.platform.log(), ["open ms-settings:network-wifi"]);
    }

    #[test]
    fn without_location_access_the_name_is_withheld() {
        let h = Harness::new();
        let mut wifi = create(json!({}));
        let denied = WifiStatus {
            ssid: None,
            frequency_mhz: None,
            link_mbps: None,
            phy: None,
            quality: 84,
            location_denied: true,
            ..connected()
        };
        h.feed(&mut wifi, denied);
        assert_eq!(
            (wifi.label().as_str(), wifi.value().as_str()),
            ("Wi-Fi", "84%")
        );
        assert!(!wifi.inactive());
        assert_eq!(
            wifi.hint_text(&h.ctx()),
            "Wi-Fi Signal\nSignal level:\t-58 dBm\nSignal percentage:\t84%\n\
             Allow location access to show the network name\n\
             Click to open the location settings"
        );
        wifi.click(&h.ctx());
        assert_eq!(h.platform.log(), ["open ms-settings:privacy-location"]);
    }

    #[test]
    fn reads_at_its_interval() {
        let wifi = create(json!({ "interval": 5 }));
        assert_eq!(
            wifi.topics(),
            [TopicRequest {
                topic: Topic::Wifi,
                interval: Duration::from_secs(5),
            }]
        );
        assert_eq!(WifiType.requires(), [Capability::Wifi]);
    }
}
