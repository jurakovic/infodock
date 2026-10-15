//! All sensor types (SPEC §8.1), and the operations that need them: creating the sensor for a
//! layout entry, filling machine-derived props (§11.2) and describing the types for Settings
//! (§4.5).

use std::sync::LazyLock;

use super::placeholder::Placeholder;
use super::schema::{self, PropField};
use super::{Category, DefaultsCtx, Props, Sensor, SensorType, State};
use crate::config::model::{Layout, SensorInstance};
use crate::i18n::t;
use crate::platform::{Capabilities, Capability};

/// The sensor types of this version. More launchers and hardware arrive with M3.
static REGISTRY: LazyLock<Registry> = LazyLock::new(|| {
    Registry::new(vec![
        &super::text::TextType,
        &super::image::ImageType,
        &super::spacer::SpacerType,
        &super::cpu::CpuMeterType,
        &super::cpu::CpuGraphType,
        &super::cpu::CpuDualType,
        &super::memory::MEMORY_METER,
        &super::memory::MEMORY_GRAPH,
        &super::memory::SWAP_METER,
        &super::memory::SWAP_GRAPH,
        &super::disk::DiskMeterType,
        &super::network::NetworkGraphType,
        &super::date::DateType,
        &super::time::TimeType,
        &super::uptime::UptimeType,
        &super::shortcut::ShortcutType,
        &super::power::PowerType,
        &super::disk_launcher::DiskType,
        &super::recycle_bin::RecycleBinType,
        &super::battery::BatteryType,
        &super::wifi::WifiType,
        &super::folder_menu::FolderMenuType,
    ])
});

pub fn registry() -> &'static Registry {
    &REGISTRY
}

pub struct Registry {
    types: Vec<&'static dyn SensorType>,
}

/// A sensor type as Settings sees it (SPEC §4.5).
pub struct TypeInfo {
    pub id: &'static str,
    pub category: Category,
    pub name: String,
    pub description: String,
    pub requires: &'static [Capability],
    /// All required capabilities exist; the picker hides other types (SPEC §4.6).
    pub available: bool,
    pub schema: &'static [PropField],
    /// This machine's values for the machine-derived fields.
    pub machine_defaults: Props,
}

impl Registry {
    pub fn new(types: Vec<&'static dyn SensorType>) -> Self {
        Self { types }
    }

    pub fn types(&self) -> &[&'static dyn SensorType] {
        &self.types
    }

    /// Whether the sensor for `instance` has properties to edit: its type is known, can run
    /// here and shows at least one field.
    pub fn editable(&self, instance: &SensorInstance, caps: &Capabilities) -> bool {
        self.get(&instance.kind).is_some_and(|ty| {
            let (props, _) = schema::resolve(ty.schema(), &instance.props, &Props::new());
            Self::available(ty, caps) && ty.schema().iter().any(|f| f.visible(&props))
        })
    }

    /// The localised name of the sensor type `kind`, or the id of an unknown type.
    pub fn type_name(&self, kind: &str) -> String {
        self.get(kind)
            .map_or_else(|| kind.to_owned(), |ty| t(ty.name()))
    }

    pub fn get(&self, id: &str) -> Option<&'static dyn SensorType> {
        self.types.iter().copied().find(|ty| ty.id() == id)
    }

    /// Whether `ty` can run with `caps`.
    pub fn available(ty: &dyn SensorType, caps: &Capabilities) -> bool {
        ty.requires().iter().all(|&c| caps.has(c))
    }

    /// The sensor for a layout entry, or a placeholder if its type is unknown or cannot run
    /// here. Invalid stored props fall back to their defaults and are logged.
    pub fn create(
        &self,
        instance: &SensorInstance,
        state: &State,
        caps: &Capabilities,
    ) -> Box<dyn Sensor> {
        let Some(ty) = self.get(&instance.kind) else {
            return Box::new(Placeholder::unknown(&instance.kind));
        };
        if !Self::available(ty, caps) {
            return Box::new(Placeholder::unsupported(t(ty.name())));
        }
        let (props, ignored) = schema::resolve(ty.schema(), &instance.props, &Props::new());
        for i in ignored {
            tracing::warn!(
                "sensor {} ({}): ignoring invalid {} = {}",
                instance.id,
                instance.kind,
                i.key,
                i.value
            );
        }
        ty.create(&props, state).unwrap_or_else(|e| {
            tracing::error!("sensor type {} cannot read its props: {e}", ty.id());
            Box::new(Placeholder::unsupported(t(ty.name())))
        })
    }

    /// Gives `sensor`, created for an entry of the same type, the changed props of `instance`
    /// in place; `false` if it must be created anew (its type cannot run here, or the sensor
    /// cannot take them so).
    pub fn reconfigure(
        &self,
        instance: &SensorInstance,
        sensor: &mut dyn Sensor,
        caps: &Capabilities,
    ) -> bool {
        let Some(ty) = self.get(&instance.kind) else {
            return false;
        };
        if !Self::available(ty, caps) {
            return false;
        }
        let (props, _) = schema::resolve(ty.schema(), &instance.props, &Props::new());
        sensor.reconfigure(&props)
    }

    /// Stores the machine-derived props that entries of known types lack (SPEC §11.2), so a
    /// sensor keeps its meaning when drives, the locale or the hardware change. Returns whether
    /// anything was added.
    pub fn fill_machine_defaults(&self, layout: &mut Layout, ctx: &DefaultsCtx) -> bool {
        let mut changed = false;
        for instance in layout.all_mut() {
            changed |= self.fill_machine_props(instance, ctx);
        }
        changed
    }

    /// Stores the machine-derived props `instance` lacks; returns whether it added any.
    pub fn fill_machine_props(&self, instance: &mut SensorInstance, ctx: &DefaultsCtx) -> bool {
        let Some(ty) = self.get(&instance.kind) else {
            return false;
        };
        let missing = schema::missing_machine_fields(ty.schema(), &instance.props);
        if missing.is_empty() {
            return false;
        }
        let machine = ty.machine_defaults(ctx);
        for field in missing {
            let value = machine
                .get(field.key)
                .and_then(|v| schema::check(field, v))
                .unwrap_or_else(|| field.default.to_value());
            instance.props.insert(field.key.to_owned(), value);
        }
        true
    }

    /// Every type, sorted by category and then by localised name (SPEC §6.6).
    pub fn infos(&self, caps: &Capabilities, ctx: &DefaultsCtx) -> Vec<TypeInfo> {
        let mut infos: Vec<TypeInfo> = self
            .types
            .iter()
            .map(|ty| TypeInfo {
                id: ty.id(),
                category: ty.category(),
                name: t(ty.name()),
                description: t(ty.description()),
                requires: ty.requires(),
                available: Self::available(*ty, caps),
                schema: ty.schema(),
                machine_defaults: ty.machine_defaults(ctx),
            })
            .collect();
        infos.sort_by_cached_key(|i| (i.category, i.name.to_lowercase()));
        infos
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::bar::theme::Theme;
    use crate::config::model::Settings;
    use crate::platform::recording::{FixedMetrics, RecordingCanvas};
    use crate::platform::{OsAppearance, RectF};
    use crate::sensors::test_support::{Clock, Echo, NoPlatform};
    use crate::sensors::{DrawCtx, LayoutCtx};

    struct Battery;

    impl SensorType for Battery {
        fn id(&self) -> &'static str {
            "battery"
        }
        fn category(&self) -> Category {
            Category::System
        }
        fn name(&self) -> &'static str {
            "test.battery"
        }
        fn description(&self) -> &'static str {
            "test.battery.description"
        }
        fn requires(&self) -> &'static [Capability] {
            &[Capability::Battery]
        }
        fn create(&self, _props: &Props, _state: &State) -> serde_json::Result<Box<dyn Sensor>> {
            Ok(Box::new(Echo("battery".into())))
        }
    }

    fn registry() -> Registry {
        Registry::new(vec![&Clock, &Battery])
    }

    fn instance(kind: &str, props: Value) -> SensorInstance {
        SensorInstance {
            id: "s_test1".into(),
            kind: kind.into(),
            props: match props {
                Value::Object(m) => m,
                _ => Props::new(),
            },
        }
    }

    /// What `sensor` draws, after measuring.
    fn drawn(mut sensor: Box<dyn Sensor>) -> String {
        let theme = Theme::resolve(&Settings::default(), OsAppearance::default());
        sensor.measure(&LayoutCtx {
            theme: &theme,
            text: &FixedMetrics,
            scale: 1.0,
            numbers: Default::default(),
            icons: &crate::sensors::NoIcons,
        });
        let mut canvas = RecordingCanvas::new(1.0);
        sensor.draw(
            &mut canvas,
            &DrawCtx {
                theme: &theme,
                rect: RectF::new(0.0, 0.0, 50.0, 28.0),
            },
        );
        canvas.dump()
    }

    #[test]
    fn creates_sensors_with_resolved_props() {
        let r = registry();
        let caps = Capabilities::default();
        let sensor = r.create(
            &instance("clock", json!({ "showSeconds": "yes", "use24h": false })),
            &State::new(),
            &caps,
        );
        assert!(
            drawn(sensor).contains("use24h=false showSeconds=false"),
            "invalid values fall back to their defaults"
        );
    }

    #[test]
    fn unknown_and_unsupported_types_become_placeholders() {
        let r = registry();
        let none = Capabilities::default();
        let unknown = r.create(&instance("cpu-graph", json!({})), &State::new(), &none);
        assert!(unknown.inactive());
        assert!(drawn(unknown).contains("\"cpu-graph\""));

        let unsupported = r.create(&instance("battery", json!({})), &State::new(), &none);
        assert!(unsupported.inactive());
        let with_battery = Capabilities {
            battery: true,
            ..Capabilities::default()
        };
        let supported = r.create(
            &instance("battery", json!({})),
            &State::new(),
            &with_battery,
        );
        assert!(!supported.inactive());
    }

    #[test]
    fn fills_missing_machine_defaults_once() {
        let r = registry();
        let mut layout = Layout {
            left: vec![instance("clock", json!({}))],
            center: vec![instance("unknown", json!({ "a": 1 }))],
            right: vec![instance("clock", json!({ "use24h": true }))],
        };
        let ctx = DefaultsCtx {
            platform: &NoPlatform,
        };
        assert!(r.fill_machine_defaults(&mut layout, &ctx));
        assert_eq!(layout.left[0].props["use24h"], false);
        assert_eq!(layout.right[0].props["use24h"], true, "stored values stay");
        assert_eq!(
            Value::Object(layout.center[0].props.clone()),
            json!({ "a": 1 })
        );
        assert!(!r.fill_machine_defaults(&mut layout, &ctx));
    }

    #[test]
    fn describes_types_for_settings() {
        let r = registry();
        let ctx = DefaultsCtx {
            platform: &NoPlatform,
        };
        let infos = r.infos(&Capabilities::default(), &ctx);
        let ids: Vec<_> = infos.iter().map(|i| i.id).collect();
        assert_eq!(ids, ["clock", "battery"], "Info before System");
        assert!(infos[0].available);
        assert!(!infos[1].available);
        assert_eq!(infos[0].machine_defaults["use24h"], false);
        assert_eq!(infos[0].schema.len(), 2);
    }

    #[test]
    fn every_type_has_its_messages() {
        use crate::sensors::schema::FieldKind;
        let mut keys = Vec::new();
        for ty in &super::registry().types {
            keys.extend([ty.name(), ty.description(), ty.category().name()]);
            for field in ty.schema() {
                keys.push(field.label);
                keys.extend(field.description);
                match field.kind {
                    FieldKind::Select { options } => keys.extend(options.iter().map(|o| o.label)),
                    FieldKind::Slider { unit, .. } => keys.extend(unit),
                    _ => {}
                }
            }
        }
        let missing: Vec<_> = keys
            .into_iter()
            .filter(|k| !crate::i18n::exists(k))
            .collect();
        assert!(missing.is_empty(), "missing in en.json: {missing:?}");
    }

    #[test]
    fn every_type_creates_a_sensor_from_its_defaults() {
        let caps = Capabilities {
            recycle_bin: true,
            start_menu: true,
            wifi: true,
            battery: true,
            eject: true,
            per_core_cpu: true,
            ..Capabilities::default()
        };
        for ty in &super::registry().types {
            let entry = instance(ty.id(), json!({}));
            let (props, _) = schema::resolve(ty.schema(), &entry.props, &Props::new());
            assert!(ty.create(&props, &State::new()).is_ok(), "{}", ty.id());
            assert!(
                !super::registry()
                    .create(&entry, &State::new(), &caps)
                    .inactive()
            );
        }
    }

    #[test]
    fn the_registry_ids_are_unique() {
        let ids: Vec<_> = super::registry().types.iter().map(|t| t.id()).collect();
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(ids.len(), unique.len());
    }
}
