//! `power` — locks, signs out, switches user, sleeps, hibernates, shuts down or restarts the
//! computer (SPEC §9.21). The bar shows the mode's icon; a click asks first (unless the
//! confirmation is off) and then hands the action to the platform.

use serde::Deserialize;

use super::common::icon::{self, BarIcon, IconSize, Look};
use super::legacy::LegacyKey;
use super::schema::{FieldKind, Lit, PropField};
use super::{
    Category, ClickResult, DrawCtx, HintContent, LayoutCtx, Props, Question, Sensor, SensorCtx,
    SensorType, State, typed,
};
use crate::i18n::{t, t_args};
use crate::platform::{Canvas, PowerAction};

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
    icon::OPACITY,
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

/// SPEC §9.21: Infodock runs the power action itself, so the legacy `path` goes.
const LEGACY_KEYS: &[LegacyKey] = &[
    LegacyKey::options(
        "mode",
        "mode",
        &[
            ("0", Lit::Str("lock")),
            ("1", Lit::Str("signOut")),
            ("2", Lit::Str("switchUser")),
            ("3", Lit::Str("sleep")),
            ("4", Lit::Str("hibernate")),
            ("5", Lit::Str("shutDown")),
            ("6", Lit::Str("restart")),
        ],
    ),
    LegacyKey::new("safety-dialog", "confirm"),
    icon::LEGACY_OPACITY,
    LegacyKey::dropped("path"),
];

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

    fn legacy_keys(&self) -> &'static [LegacyKey] {
        LEGACY_KEYS
    }

    fn schema(&self) -> &'static [PropField] {
        SCHEMA
    }

    fn create(&self, props: &Props, _state: &State) -> serde_json::Result<Box<dyn Sensor>> {
        let props: PowerProps = typed(props)?;
        Ok(Box::new(Power {
            action: PowerAction::from_id(&props.mode).unwrap_or(PowerAction::Sleep),
            props,
            icon: BarIcon::default(),
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
    icon: BarIcon,
}

impl Sensor for Power {
    fn measure(&mut self, cx: &LayoutCtx) -> f32 {
        let look = Look::built_in(IconSize::Small, svg(self.action), self.props.opacity);
        self.icon.measure(look, cx)
    }

    fn draw(&self, canvas: &mut dyn Canvas, cx: &DrawCtx) {
        self.icon.draw(canvas, cx);
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
    use serde_json::json;

    use super::*;
    use crate::config::model::Settings;
    use crate::platform::Capabilities;
    use crate::sensors::schema::resolve;
    use crate::sensors::test_support::{Harness, created, draw, measure};
    use crate::test_util::FakePlatform;

    fn create(stored: serde_json::Value) -> Box<dyn Sensor> {
        created(&PowerType, stored)
    }

    /// Every mode but hibernate; records the power actions it is asked for.
    fn recorder() -> Harness {
        Harness::with_platform(
            FakePlatform::default().with_capabilities(Capabilities {
                power_modes: PowerAction::ALL
                    .into_iter()
                    .filter(|a| *a != PowerAction::Hibernate)
                    .map(|a| a.id().to_owned())
                    .collect(),
                ..Capabilities::default()
            }),
        )
    }

    fn with_ctx(h: &Harness, f: impl FnOnce(&SensorCtx)) {
        f(&h.ctx());
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
        let hint = sensor.hint(&recorder().ctx());
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
        let light = crate::look::theme::Theme::resolve(
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
            icon: BarIcon::default(),
        };
        let h = Harness::new();
        sensor.measure(&crate::sensors::LayoutCtx {
            theme: &light,
            ..h.layout()
        });
        let bitmap = sensor.icon.icon().expect("an icon").bitmap();
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
        let h = recorder();
        let mut sensor = create(json!({ "mode": "restart", "force": true }));
        with_ctx(&h, |cx| {
            assert_eq!(
                sensor.click(cx),
                ClickResult::Confirm(Question {
                    message: "Do you really want to restart this computer?".into(),
                    action: "Restart".into(),
                })
            );
            assert!(h.platform.log().is_empty(), "not yet");
            sensor.confirmed(cx);
        });
        assert_eq!(h.platform.log(), ["Restart force=true"]);
    }

    #[test]
    fn without_confirmation_acts_at_once_and_force_only_where_it_applies() {
        let h = recorder();
        let mut lock = create(json!({ "mode": "lock", "confirm": false, "force": true }));
        with_ctx(&h, |cx| {
            assert_eq!(lock.click(cx), ClickResult::Handled);
        });
        assert_eq!(h.platform.log(), ["Lock force=false"]);
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
        let h = recorder();
        with_ctx(&h, |cx| {
            assert_eq!(
                sensor.hint(cx),
                Some(HintContent::Text(
                    "Hibernate is not available on this computer".into()
                ))
            );
            assert_eq!(sensor.click(cx), ClickResult::None);
        });
        assert!(h.platform.log().is_empty());
    }
}
