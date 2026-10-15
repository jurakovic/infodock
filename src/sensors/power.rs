//! `power` — locks, signs out, switches user, sleeps, hibernates, shuts down or restarts the
//! computer (SPEC §9.21). The bar shows the mode's icon; a click asks first (unless the
//! confirmation is off) and then hands the action to the platform.

use serde::Deserialize;

use super::common::icon::{self, Icon};
use super::schema::{FieldKind, Lit, PropField};
use super::{
    Category, ClickResult, DrawCtx, HintContent, LayoutCtx, Props, Question, Sensor, SensorCtx,
    SensorType, State, typed,
};
use crate::i18n::{t, t_args};
use crate::platform::{Canvas, PowerAction};

/// The icon height, logical px (SPEC §9.21).
const ICON: f32 = 21.0;

/// The modes that can close apps without asking (`force`).
const FORCEABLE: &[Lit] = &[
    Lit::Str("signOut"),
    Lit::Str("shutDown"),
    Lit::Str("restart"),
];

const SCHEMA: &[PropField] = &[
    PropField::new(
        "mode",
        FieldKind::PowerMode,
        "sensor.power.field.mode",
        Lit::Str("sleep"),
    ),
    PropField::new(
        "confirm",
        FieldKind::Bool,
        "sensor.power.field.confirm",
        Lit::Bool(true),
    )
    .describe("sensor.power.field.confirm.help"),
    PropField::new(
        "force",
        FieldKind::Bool,
        "sensor.power.field.force",
        Lit::Bool(false),
    )
    .describe("sensor.power.field.force.help")
    .visible_if("mode", FORCEABLE),
    PropField::new(
        "opacity",
        FieldKind::Opacity { min: 20 },
        "sensor.field.opacity",
        Lit::Int(100),
    ),
];

/// The built-in icon of `action` (Appendix C).
fn svg(action: PowerAction) -> &'static [u8] {
    match action {
        PowerAction::Lock => include_bytes!("../../assets/icons/power-lock.svg"),
        PowerAction::SignOut => include_bytes!("../../assets/icons/power-signOut.svg"),
        PowerAction::SwitchUser => include_bytes!("../../assets/icons/power-switchUser.svg"),
        PowerAction::Sleep => include_bytes!("../../assets/icons/power-sleep.svg"),
        PowerAction::Hibernate => include_bytes!("../../assets/icons/power-hibernate.svg"),
        PowerAction::ShutDown => include_bytes!("../../assets/icons/power-shutDown.svg"),
        PowerAction::Restart => include_bytes!("../../assets/icons/power-restart.svg"),
    }
}

/// The mode's name, e.g. *Shut down*.
fn name(action: PowerAction) -> String {
    t(&format!("sensor.power.mode.{}", action.id()))
}

pub struct PowerType;

impl SensorType for PowerType {
    fn id(&self) -> &'static str {
        "power"
    }

    fn category(&self) -> Category {
        Category::Launchers
    }

    fn name(&self) -> &'static str {
        "sensor.power.name"
    }

    fn description(&self) -> &'static str {
        "sensor.power.description"
    }

    fn legacy_types(&self) -> &'static [&'static str] {
        &["DockPower"]
    }

    fn schema(&self) -> &'static [PropField] {
        SCHEMA
    }

    fn create(&self, props: &Props, _state: &State) -> serde_json::Result<Box<dyn Sensor>> {
        let props: PowerProps = typed(props)?;
        Ok(Box::new(Power {
            action: PowerAction::from_id(&props.mode).unwrap_or(PowerAction::Sleep),
            props,
            icon: None,
        }))
    }

    fn summary(&self, props: &Props) -> Option<String> {
        let mode = props.get("mode").and_then(|v| v.as_str())?;
        PowerAction::from_id(mode).map(name)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PowerProps {
    mode: String,
    confirm: bool,
    force: bool,
    /// Percent.
    opacity: u32,
}

struct Power {
    props: PowerProps,
    action: PowerAction,
    icon: Option<Icon>,
}

impl Sensor for Power {
    fn measure(&mut self, cx: &LayoutCtx) -> f32 {
        let px = icon::physical(ICON, cx.scale);
        let opacity = self.props.opacity.min(100) as f32 / 100.0;
        // Built-in and monochrome: in the sensor text colour (SPEC §8.4).
        let tint = Some(cx.theme.sensor_text);
        self.icon = Icon::svg(svg(self.action), px).map(|b| Icon::new(b, tint, opacity, None));
        self.icon.as_ref().map_or(ICON, |i| i.width(cx.scale))
    }

    fn draw(&self, canvas: &mut dyn Canvas, cx: &DrawCtx) {
        if let Some(icon) = &self.icon {
            icon.draw(canvas, cx);
        }
    }

    fn hint(&self, cx: &SensorCtx) -> Option<HintContent> {
        Some(HintContent::Text(if self.offered(cx) {
            name(self.action)
        } else {
            t_args("sensor.power.unavailable", &[("mode", &name(self.action))])
        }))
    }

    /// Nothing, for a mode this computer lacks (a config from another machine).
    fn click(&mut self, cx: &SensorCtx) -> ClickResult {
        if !self.offered(cx) {
            return ClickResult::None;
        }
        if !self.props.confirm {
            return self.act(cx);
        }
        ClickResult::Confirm(Question {
            message: t(&format!("sensor.power.confirm.{}", self.action.id())),
            action: name(self.action),
        })
    }

    fn confirmed(&mut self, cx: &SensorCtx) -> bool {
        self.act(cx);
        false
    }

    fn accessible_value(&self, _cx: &SensorCtx) -> String {
        name(self.action)
    }
}

impl Power {
    /// Whether this computer offers the mode (`capabilities.power_modes`, SPEC §9.21).
    fn offered(&self, cx: &SensorCtx) -> bool {
        let id = self.action.id();
        cx.platform
            .capabilities()
            .power_modes
            .iter()
            .any(|mode| mode == id)
    }

    fn act(&self, cx: &SensorCtx) -> ClickResult {
        let force = self.props.force && FORCEABLE.contains(&Lit::Str(self.action.id()));
        cx.platform.power(self.action, force);
        ClickResult::Handled
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use serde_json::json;

    use super::*;
    use crate::config::model::Settings;
    use crate::platform::{Capabilities, MonitorInfo, OsAppearance, Platform};
    use crate::sensors::schema::resolve;
    use crate::sensors::test_support::{draw, measure, theme};

    fn create(stored: serde_json::Value) -> Box<dyn Sensor> {
        let stored = stored.as_object().cloned().unwrap_or_default();
        let (props, ignored) = resolve(SCHEMA, &stored, &Props::new());
        assert!(ignored.is_empty(), "{ignored:?}");
        PowerType
            .create(&props, &State::new())
            .expect("valid props")
    }

    /// Records the power actions it is asked for.
    #[derive(Default)]
    struct Recorder(Mutex<Vec<String>>);

    impl Platform for Recorder {
        /// Every mode but hibernate.
        fn capabilities(&self) -> Capabilities {
            Capabilities {
                power_modes: PowerAction::ALL
                    .into_iter()
                    .filter(|a| *a != PowerAction::Hibernate)
                    .map(|a| a.id().to_owned())
                    .collect(),
                ..Capabilities::default()
            }
        }
        fn monitors(&self) -> Vec<MonitorInfo> {
            Vec::new()
        }
        fn appearance(&self) -> OsAppearance {
            OsAppearance::default()
        }
        fn uptime(&self) -> std::time::Duration {
            std::time::Duration::ZERO
        }
        fn power(&self, action: PowerAction, force: bool) {
            self.0
                .lock()
                .expect("lock")
                .push(format!("{action:?} force={force}"));
        }
    }

    fn with_ctx(platform: &Recorder, f: impl FnOnce(&SensorCtx)) {
        let (settings, theme) = (Settings::default(), theme());
        f(&SensorCtx {
            id: "s_1",
            settings: &settings,
            theme: &theme,
            locked: false,
            alt: false,
            animate: false,
            changes: &crate::sampler::ChangeSink::NONE,
            platform,
        });
    }

    #[test]
    fn shows_the_modes_icon_21_px_tall() {
        let mut sensor = create(json!({ "mode": "shutDown", "opacity": 60 }));
        assert_eq!(measure(sensor.as_mut()), 21.0);
        assert_eq!(
            draw(sensor.as_ref(), 21.0),
            "bitmap 21×21 into 7,4 21×21 opacity 0.6\n",
            "centred on the 28 px bar"
        );
        let mut hint = None;
        with_ctx(&Recorder::default(), |cx| hint = sensor.hint(cx));
        assert_eq!(hint, Some(HintContent::Text("Shut down".into())));
        // Every mode has an icon and a name.
        for action in PowerAction::ALL {
            let mut sensor = create(json!({ "mode": action.id() }));
            assert_eq!(measure(sensor.as_mut()), 21.0, "{action:?}");
            assert!(!name(action).starts_with("sensor."), "{action:?}");
        }
    }

    #[test]
    fn the_icon_takes_the_text_colour_so_it_reads_on_a_light_bar() {
        let light = crate::bar::theme::Theme::resolve(
            &Settings::default(),
            crate::platform::OsAppearance {
                dark: false,
                ..Default::default()
            },
        );
        let stored = json!({});
        let (props, _) = resolve(SCHEMA, stored.as_object().expect("object"), &Props::new());
        let mut sensor = Power {
            props: crate::sensors::typed(&props).expect("valid props"),
            action: PowerAction::Sleep,
            icon: None,
        };
        sensor.measure(&crate::sensors::LayoutCtx {
            theme: &light,
            text: &crate::platform::recording::FixedMetrics,
            scale: 1.0,
            numbers: Default::default(),
            icons: &crate::sensors::NoIcons,
        });
        let bitmap = sensor.icon.as_ref().expect("an icon").bitmap();
        // Every fully opaque pixel is the dark text colour, not white.
        let opaque: Vec<_> = bitmap
            .pixels()
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|px| px[3] == 255)
            .collect();
        assert!(!opaque.is_empty());
        assert!(opaque.iter().all(|px| px[0] < 64), "dark on a light bar");
    }

    #[test]
    fn asks_first_then_acts() {
        let platform = Recorder::default();
        let mut sensor = create(json!({ "mode": "restart", "force": true }));
        with_ctx(&platform, |cx| {
            assert_eq!(
                sensor.click(cx),
                ClickResult::Confirm(Question {
                    message: "Do you really want to restart this computer?".into(),
                    action: "Restart".into(),
                })
            );
            assert!(platform.0.lock().expect("lock").is_empty(), "not yet");
            sensor.confirmed(cx);
        });
        assert_eq!(*platform.0.lock().expect("lock"), ["Restart force=true"]);
    }

    #[test]
    fn without_confirmation_acts_at_once_and_force_only_where_it_applies() {
        let platform = Recorder::default();
        let mut lock = create(json!({ "mode": "lock", "confirm": false, "force": true }));
        with_ctx(&platform, |cx| {
            assert_eq!(lock.click(cx), ClickResult::Handled);
        });
        assert_eq!(*platform.0.lock().expect("lock"), ["Lock force=false"]);
    }

    #[test]
    fn force_is_shown_for_the_modes_that_close_apps() {
        let force = &SCHEMA[2];
        let props = |mode: &str| {
            let stored = json!({ "mode": mode });
            resolve(SCHEMA, stored.as_object().expect("object"), &Props::new()).0
        };
        assert!(force.visible(&props("shutDown")));
        assert!(!force.visible(&props("sleep")));
        assert_eq!(
            PowerType.summary(&props("hibernate")),
            Some("Hibernate".into())
        );
    }

    #[test]
    fn a_mode_this_computer_lacks_does_nothing() {
        let mut sensor = create(json!({ "mode": "hibernate", "confirm": false }));
        let platform = Recorder::default();
        with_ctx(&platform, |cx| {
            assert_eq!(
                sensor.hint(cx),
                Some(HintContent::Text(
                    "Hibernate is not available on this computer".into()
                ))
            );
            assert_eq!(sensor.click(cx), ClickResult::None);
        });
        assert!(platform.0.lock().expect("lock").is_empty());
    }
}
