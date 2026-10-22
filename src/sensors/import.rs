//! Importing sensors into the layout (SPEC §11.3–11.4). Each sensor a file brings gets a new id
//! and is checked against its type by the rules of §11.2, as a stored sensor is on load:
//! invalid values give way to their defaults (with a warning), numbers are clamped, keys its
//! type does not know are kept, machine-derived props are filled from this machine, and only
//! what differs from the defaults is stored. A type this version does not know is kept as it
//! is and shown as a placeholder.

use super::registry::Registry;
use super::schema;
use super::{DefaultsCtx, Props};
use crate::config::exchange::{ImportMode, ImportReport, Incoming, IncomingSensor, Warning};
use crate::config::model::{Config, IdGen, Layout, SensorInstance};

/// Adds what `incoming` brings to `config`, replacing its layout, sensor state and unsupported
/// legacy sensors first with [`ImportMode::Replace`], or appending to its zones with
/// [`ImportMode::Merge`].
pub fn import(
    config: &mut Config,
    registry: &Registry,
    ctx: &DefaultsCtx,
    incoming: Incoming,
    mode: ImportMode,
) -> ImportReport {
    let Incoming {
        sensors,
        unsupported,
        skipped,
        mut warnings,
    } = incoming;
    if mode == ImportMode::Replace {
        config.layout = Layout::default();
        config.state.clear();
        config.legacy.unsupported.clear();
    }
    let mut ids = IdGen::avoiding(&config.layout);
    let added = sensors.len();
    for IncomingSensor {
        zone,
        kind,
        props,
        state,
    } in sensors
    {
        let mut instance = SensorInstance {
            id: ids.fresh(),
            kind,
            props,
        };
        check(registry, ctx, &mut instance, &mut warnings);
        if !state.is_empty() {
            config.state.insert(instance.id.clone(), state);
        }
        config.layout.zone_mut(zone).push(instance);
    }
    config.legacy.unsupported.extend(unsupported);
    ImportReport {
        added,
        skipped,
        warnings,
    }
}

/// Brings the props of `instance` into their stored form for its type.
fn check(
    registry: &Registry,
    ctx: &DefaultsCtx,
    instance: &mut SensorInstance,
    warnings: &mut Vec<Warning>,
) {
    let Some(ty) = registry.get(&instance.kind) else {
        warnings.push(Warning::UnknownType {
            kind: instance.kind.clone(),
        });
        return;
    };
    let fields = ty.schema();
    // Invalid values go first, so an invalid machine-derived one is asked of the machine.
    for field in fields {
        if let Some(raw) = instance.props.get(field.key)
            && schema::check(field, raw).is_none()
        {
            instance.props.remove(field.key);
            warnings.push(Warning::InvalidValue {
                sensor: registry.type_name(&instance.kind),
                key: field.key.to_owned(),
            });
        }
    }
    registry.fill_machine_props(instance, ctx);
    let (props, _) = schema::resolve(fields, &instance.props, &Props::new());
    instance.props = schema::to_stored(fields, &props, &instance.props);
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::config::exchange::{SkipReason, Skipped};
    use crate::config::model::{LegacyUnsupported, Zone};
    use crate::sensors::registry::registry;
    use crate::sensors::test_support::Clock;
    use crate::test_util::NO_PLATFORM;

    static CTX: DefaultsCtx = DefaultsCtx {
        platform: &NO_PLATFORM,
    };

    fn props(v: Value) -> Props {
        v.as_object().cloned().unwrap_or_default()
    }

    fn sensor(zone: Zone, kind: &str, p: Value) -> IncomingSensor {
        IncomingSensor {
            zone,
            kind: kind.into(),
            props: props(p),
            state: Props::new(),
        }
    }

    fn kinds(config: &Config, zone: Zone) -> Vec<&str> {
        config
            .layout
            .zone(zone)
            .iter()
            .map(|s| s.kind.as_str())
            .collect()
    }

    fn unsupported(kind: &str) -> LegacyUnsupported {
        LegacyUnsupported {
            kind: kind.into(),
            zone: Zone::Center,
            props: Default::default(),
        }
    }

    #[test]
    fn replace_takes_the_place_of_the_layout() {
        let mut c = Config::first_run("1.0.0");
        let old = c.layout.right[0].id.clone();
        c.state.insert(old.clone(), props(json!({ "x": 1 })));
        c.legacy.unsupported.push(unsupported("DockGMail"));
        let mut text = sensor(Zone::Right, "text", json!({ "text": "Hi" }));
        text.state = props(json!({ "seen": 2 }));
        let incoming = Incoming {
            sensors: vec![text, sensor(Zone::Left, "spacer", json!({}))],
            unsupported: vec![unsupported("DockRSSSensor")],
            skipped: vec![Skipped {
                kind: String::new(),
                reason: SkipReason::NotASensor,
            }],
            warnings: Vec::new(),
        };
        let report = import(&mut c, registry(), &CTX, incoming, ImportMode::Replace);
        assert_eq!(report.added, 2);
        assert_eq!(report.skipped.len(), 1);
        assert!(report.warnings.is_empty());
        assert_eq!(kinds(&c, Zone::Left), ["spacer"]);
        assert!(c.layout.center.is_empty());
        assert_eq!(kinds(&c, Zone::Right), ["text"]);
        let id = &c.layout.right[0].id;
        assert_eq!(c.state.keys().collect::<Vec<_>>(), [id]);
        assert_eq!(c.state[id], props(json!({ "seen": 2 })));
        assert_eq!(c.legacy.unsupported, [unsupported("DockRSSSensor")]);
    }

    #[test]
    fn merge_appends_to_the_zones_with_new_ids() {
        let mut c = Config::first_run("1.0.0");
        let before = c.clone();
        c.legacy.unsupported.push(unsupported("DockGMail"));
        let again = crate::config::exchange::parse(
            &crate::config::exchange::export(&before, crate::config::exchange::ExportScope::Layout)
                .expect("exported"),
        )
        .expect("parsed");
        let report = import(&mut c, registry(), &CTX, again, ImportMode::Merge);
        assert_eq!(report.added, 6);
        assert_eq!(
            kinds(&c, Zone::Left),
            [
                "cpu-graph",
                "memory-meter",
                "network-graph",
                "cpu-graph",
                "memory-meter",
                "network-graph"
            ]
        );
        assert_eq!(kinds(&c, Zone::Center), ["image", "image"]);
        assert_eq!(c.layout.center[1].props["popupText"], "Infodock %v");
        let ids: std::collections::HashSet<_> = c.layout.all().map(|s| &s.id).collect();
        assert_eq!(ids.len(), 12, "every id is new");
        assert_eq!(c.legacy.unsupported, [unsupported("DockGMail")]);
    }

    #[test]
    fn props_are_checked_against_their_type() {
        let mut c = Config::default();
        let incoming = Incoming {
            sensors: vec![sensor(
                Zone::Left,
                "image",
                json!({
                    "opacity": 5,
                    "color": { "mode": "custom", "color": "#abcdef" },
                    "popupText": 3,
                    "fromNewerVersion": true,
                    "image": "",
                }),
            )],
            ..Incoming::default()
        };
        let report = import(&mut c, registry(), &CTX, incoming, ImportMode::Merge);
        assert_eq!(
            Value::Object(c.layout.left[0].props.clone()),
            json!({
                "opacity": 20,
                "color": { "mode": "custom", "color": "#ABCDEF" },
                "fromNewerVersion": true,
            })
        );
        assert_eq!(
            report.warnings,
            [Warning::InvalidValue {
                sensor: "Image".into(),
                key: "popupText".into()
            }]
        );
        assert_eq!(
            report.warnings[0].message(),
            "Image: the value of “popupText” cannot be used, so its default is used instead."
        );
    }

    #[test]
    fn unknown_types_are_kept_as_they_are() {
        let mut c = Config::default();
        let mail = sensor(Zone::Center, "mail", json!({ "account": "me" }));
        let incoming = Incoming {
            sensors: vec![mail.clone()],
            ..Incoming::default()
        };
        let report = import(&mut c, registry(), &CTX, incoming, ImportMode::Merge);
        assert_eq!(c.layout.center[0].kind, "mail");
        assert_eq!(c.layout.center[0].props, mail.props);
        assert_eq!(
            report.warnings,
            [Warning::UnknownType {
                kind: "mail".into()
            }]
        );
    }

    #[test]
    fn machine_derived_props_come_from_this_machine() {
        // Clock's `use24h` defaults to true; the machine says false.
        let registry = Registry::new(vec![&Clock]);
        let mut c = Config::default();
        let incoming = Incoming {
            sensors: vec![
                sensor(Zone::Left, "clock", json!({})),
                sensor(Zone::Left, "clock", json!({ "use24h": true })),
                sensor(Zone::Left, "clock", json!({ "use24h": "yes" })),
            ],
            ..Incoming::default()
        };
        let report = import(&mut c, &registry, &CTX, incoming, ImportMode::Merge);
        let use24h: Vec<&Value> = c.layout.left.iter().map(|s| &s.props["use24h"]).collect();
        assert_eq!(use24h, [&json!(false), &json!(true), &json!(false)]);
        assert_eq!(report.warnings.len(), 1);
    }
}
