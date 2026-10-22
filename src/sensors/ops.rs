//! Sensor operations used by Settings and the bar (SPEC §4.5): add, update, move and remove
//! sensors, and the props a live preview shows. Each is a function on a [`Config`]; the app
//! runs it inside a config-store update, which saves the result and notifies the bar and
//! Settings. What the machine says for machine-derived props is asked before, with
//! [`machine_props`], so the store is never locked while the machine is asked.

use super::registry::Registry;
use super::schema::{self, PropField};
use super::{DefaultsCtx, Props};
use crate::config::ConfigError;
use crate::config::model::{Config, IdGen, Layout, SensorInstance, Zone};

fn not_found(id: &str) -> ConfigError {
    ConfigError::NotFound(id.to_owned())
}

/// What the machine says for the machine-derived props a sensor of type `kind` with `props`
/// lacks once `changes` are applied, for [`add`], [`update`] and [`preview`]; empty, without
/// asking the machine, when it lacks none (SPEC §11.2). Fields `changes` sets are not asked
/// of the machine, so a dropped drive lists no drives.
pub fn machine_props(
    registry: &Registry,
    ctx: &DefaultsCtx,
    kind: &str,
    props: &Props,
    changes: &Props,
) -> Props {
    let probe = SensorInstance {
        id: String::new(),
        kind: kind.to_owned(),
        props: props.clone(),
    };
    registry
        .machine_props(&with_changes(&probe, changes), ctx)
        .unwrap_or_default()
}

/// [`machine_props`] for sensor `id` of `config`; empty if there is no such sensor.
pub fn machine_props_of(
    config: &Config,
    registry: &Registry,
    ctx: &DefaultsCtx,
    id: &str,
    changes: &Props,
) -> Props {
    config
        .layout
        .all()
        .find(|s| s.id == id)
        .map(|s| machine_props(registry, ctx, &s.kind, &s.props, changes))
        .unwrap_or_default()
}

/// Adds a sensor of type `kind` at `index` of `zone` (clamped to the zone) with `props` (only
/// the values that differ from the defaults; validated) and machine-derived props filled in
/// from `machine` ([`machine_props`]). Returns the new id.
pub fn add(
    config: &mut Config,
    registry: &Registry,
    machine: &Props,
    kind: &str,
    zone: Zone,
    index: usize,
    props: &Props,
) -> Result<String, ConfigError> {
    let mut instance = SensorInstance {
        id: IdGen::avoiding(&config.layout).fresh(),
        kind: kind.to_owned(),
        props: Props::new(),
    };
    instance.props = with_machine_props(registry, machine, &instance, props)?;
    let id = instance.id.clone();
    let list = config.layout.zone_mut(zone);
    list.insert(index.min(list.len()), instance);
    Ok(id)
}

/// The stored props of `instance` with `changes` (keys of its type's schema) applied: each
/// value is validated and normalised, and only what differs from the defaults is kept
/// (SPEC §8.2, §11.2). Fails with `invalid` for an unknown type, an unknown key or a value of
/// the wrong type, naming the keys.
fn updated_props(
    registry: &Registry,
    instance: &SensorInstance,
    changes: &Props,
) -> Result<Props, ConfigError> {
    let ty = registry
        .get(&instance.kind)
        .ok_or_else(|| ConfigError::UnknownType(instance.kind.clone()))?;
    let fields: &[PropField] = ty.schema();
    let (mut props, _) = schema::resolve(fields, &instance.props, &Props::new());
    let mut rejected = Vec::new();
    for (key, raw) in changes {
        let value = schema::field(fields, key)
            .and_then(|f| schema::normalize(f, raw, &f.default.to_value()));
        match value {
            Some(value) => {
                props.insert(key.clone(), value);
            }
            None => rejected.push(key.clone()),
        }
    }
    if !rejected.is_empty() {
        return Err(ConfigError::InvalidProps {
            kind: instance.kind.clone(),
            keys: rejected,
        });
    }
    Ok(schema::to_stored(fields, &props, &instance.props))
}

/// `instance` with the keys of `changes` it lacks, as they are (validated later).
fn with_changes(instance: &SensorInstance, changes: &Props) -> SensorInstance {
    let mut instance = instance.clone();
    for (key, value) in changes {
        instance
            .props
            .entry(key.clone())
            .or_insert_with(|| value.clone());
    }
    instance
}

/// [`updated_props`] for `instance` with the machine-derived props it lacks filled in first,
/// from `machine`: stored machine fields are kept as they are, so their static defaults must
/// not stand in for the machine's values (SPEC §11.2).
fn with_machine_props(
    registry: &Registry,
    machine: &Props,
    instance: &SensorInstance,
    changes: &Props,
) -> Result<Props, ConfigError> {
    let mut instance = with_changes(instance, changes);
    registry.fill_machine_props(&mut instance, machine);
    updated_props(registry, &instance, changes)
}

fn instance_mut<'a>(layout: &'a mut Layout, id: &str) -> Option<&'a mut SensorInstance> {
    layout.all_mut().find(|s| s.id == id)
}

/// Applies `changes` to sensor `id` (see [`updated_props`]); machine-derived props it lacks
/// are filled in from `machine` ([`machine_props_of`]).
pub fn update(
    config: &mut Config,
    registry: &Registry,
    machine: &Props,
    id: &str,
    changes: &Props,
) -> Result<(), ConfigError> {
    let instance = instance_mut(&mut config.layout, id).ok_or_else(|| not_found(id))?;
    instance.props = with_machine_props(registry, machine, instance, changes)?;
    Ok(())
}

/// Sensor `id` as it would be with `changes`, for a live preview (SPEC §6.7); nothing is
/// saved.
pub fn preview(
    config: &Config,
    registry: &Registry,
    machine: &Props,
    id: &str,
    changes: &Props,
) -> Result<SensorInstance, ConfigError> {
    let instance = config
        .layout
        .all()
        .find(|s| s.id == id)
        .ok_or_else(|| not_found(id))?;
    Ok(SensorInstance {
        props: with_machine_props(registry, machine, instance, changes)?,
        ..instance.clone()
    })
}

/// Moves sensor `id` to `index` of `zone`, counted with the sensor already taken out of its
/// place; the index is clamped to the zone.
pub fn move_to(config: &mut Config, id: &str, zone: Zone, index: usize) -> Result<(), ConfigError> {
    let (from, at) = config.layout.find(id).ok_or_else(|| not_found(id))?;
    let instance = config.layout.zone_mut(from).remove(at);
    let list = config.layout.zone_mut(zone);
    list.insert(index.min(list.len()), instance);
    Ok(())
}

/// The [`move_to`] index for an insertion point counted while the source is still in place
/// (SPEC §6.2): within the same zone, a target after the source shifts down by one.
pub fn index_after_removal(source: (Zone, usize), zone: Zone, insertion: usize) -> usize {
    if source.0 == zone && source.1 < insertion {
        insertion - 1
    } else {
        insertion
    }
}

/// Removes sensor `id` and its runtime state.
pub fn remove(config: &mut Config, id: &str) -> Result<SensorInstance, ConfigError> {
    let (zone, index) = config.layout.find(id).ok_or_else(|| not_found(id))?;
    config.state.remove(id);
    Ok(config.layout.zone_mut(zone).remove(index))
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::sensors::registry::registry;
    use crate::sensors::test_support::Clock;
    use crate::test_util::NO_PLATFORM;

    static CTX: DefaultsCtx = DefaultsCtx {
        platform: &NO_PLATFORM,
    };

    fn props(v: Value) -> Props {
        v.as_object().cloned().unwrap_or_default()
    }

    fn ids(config: &Config, zone: Zone) -> Vec<String> {
        config
            .layout
            .zone(zone)
            .iter()
            .map(|s| s.id.clone())
            .collect()
    }

    fn config() -> Config {
        Config::first_run("1.0.0")
    }

    #[test]
    fn adds_sensors_with_validated_props() {
        let mut c = config();
        let id = add(
            &mut c,
            registry(),
            &Props::new(),
            "text",
            Zone::Right,
            0,
            &props(json!({ "text": "Hello", "fontSize": 13 })),
        )
        .expect("added");
        assert_eq!(c.layout.right[0].id, id);
        // The default size is not stored.
        assert_eq!(
            Value::Object(c.layout.right[0].props.clone()),
            json!({ "text": "Hello" })
        );

        let end = add(
            &mut c,
            registry(),
            &Props::new(),
            "spacer",
            Zone::Left,
            99,
            &Props::new(),
        )
        .expect("added");
        assert_eq!(ids(&c, Zone::Left).last(), Some(&end));

        let before = c.clone();
        assert!(
            add(
                &mut c,
                registry(),
                &Props::new(),
                "mail",
                Zone::Left,
                0,
                &Props::new()
            )
            .is_err()
        );
        assert!(
            add(
                &mut c,
                registry(),
                &Props::new(),
                "text",
                Zone::Left,
                0,
                &props(json!({ "bogus": 1 }))
            )
            .is_err()
        );
        assert_eq!(c, before, "failed adds change nothing");
    }

    #[test]
    fn updates_only_valid_known_props() {
        let mut c = config();
        let logo = c.layout.center[0].id.clone();
        update(
            &mut c,
            registry(),
            &Props::new(),
            &logo,
            &props(json!({ "opacity": 5, "color": { "mode": "custom", "color": "#abcdef" } })),
        )
        .expect("updated");
        assert_eq!(
            Value::Object(c.layout.center[0].props.clone()),
            json!({
                "popupText": "Infodock %v",
                "opacity": 20,
                "color": { "mode": "custom", "color": "#ABCDEF" }
            })
        );
        // Back to the default: no longer stored.
        update(
            &mut c,
            registry(),
            &Props::new(),
            &logo,
            &props(json!({ "opacity": 100 })),
        )
        .expect("updated");
        assert!(!c.layout.center[0].props.contains_key("opacity"));

        let err = update(
            &mut c,
            registry(),
            &Props::new(),
            &logo,
            &props(json!({ "opacity": "full", "size": 3 })),
        )
        .expect_err("invalid");
        assert_eq!(err.to_string(), "invalid props for image: opacity, size");
        assert_eq!(err.code(), "invalid");
        let none = Props::new();
        let gone = update(&mut c, registry(), &none, "s_nope", &none).expect_err("no sensor");
        assert_eq!(gone.code(), "notFound");
        // Placeholders (unknown types) cannot be edited.
        let mail = SensorInstance {
            id: "s_mail".into(),
            kind: "mail".into(),
            props: Props::new(),
        };
        c.layout.left.push(mail);
        let unknown = update(&mut c, registry(), &none, "s_mail", &none).expect_err("unknown");
        assert_eq!(unknown.code(), "unknownType");
    }

    #[test]
    fn previews_without_saving() {
        let c = config();
        let logo = c.layout.center[0].id.clone();
        let shown = preview(
            &c,
            registry(),
            &Props::new(),
            &logo,
            &props(json!({ "opacity": 50 })),
        )
        .expect("preview");
        assert_eq!(shown.props["opacity"], 50);
        assert_eq!(shown.id, logo);
        assert!(!c.layout.center[0].props.contains_key("opacity"));
    }

    #[test]
    fn moves_between_and_within_zones() {
        let mut c = config();
        let [cpu, memory, network] = [0, 1, 2].map(|i| c.layout.left[i].id.clone());
        // Drag CPU to the right of Network: insertion point 3 before removal → 2.
        let index = index_after_removal((Zone::Left, 0), Zone::Left, 3);
        assert_eq!(index, 2);
        move_to(&mut c, &cpu, Zone::Left, index).expect("moved");
        assert_eq!(
            ids(&c, Zone::Left),
            [memory.clone(), network.clone(), cpu.clone()]
        );

        // To the start of the right zone; other zones need no shift.
        assert_eq!(index_after_removal((Zone::Left, 1), Zone::Right, 0), 0);
        move_to(&mut c, &network, Zone::Right, 0).expect("moved");
        assert_eq!(ids(&c, Zone::Left), [memory, cpu]);
        assert_eq!(ids(&c, Zone::Right)[0], network);
        // Moving left within a zone: no shift.
        assert_eq!(index_after_removal((Zone::Right, 2), Zone::Right, 0), 0);
        assert!(move_to(&mut c, "s_nope", Zone::Left, 0).is_err());
    }

    #[test]
    fn removes_a_sensor_and_its_state() {
        let mut c = config();
        let date = c.layout.right[0].id.clone();
        c.state.insert(date.clone(), props(json!({ "x": 1 })));
        let removed = remove(&mut c, &date).expect("removed");
        assert_eq!(removed.kind, "date");
        assert_eq!(c.layout.right.len(), 1);
        assert!(!c.state.contains_key(&date));
        assert!(remove(&mut c, &date).is_err());
    }

    #[test]
    fn machine_derived_props_come_from_the_machine() {
        // Clock's `use24h` defaults to true; the machine says false.
        let registry = Registry::new(vec![&Clock]);
        let mut c = Config::default();
        let none = Props::new();
        let machine = machine_props(&registry, &CTX, "clock", &none, &none);
        assert_eq!(machine["use24h"], false);
        let id = add(&mut c, &registry, &machine, "clock", Zone::Left, 0, &none).expect("added");
        assert_eq!(c.layout.left[0].props["use24h"], false);
        // An instance stored before the field existed gets the machine's value too.
        c.layout.left[0].props.remove("use24h");
        let shown = props(json!({ "showSeconds": true }));
        let machine = machine_props_of(&c, &registry, &CTX, &id, &shown);
        let previewed = preview(&c, &registry, &machine, &id, &shown).expect("preview");
        assert_eq!(previewed.props["use24h"], false);
        update(&mut c, &registry, &machine, &id, &shown).expect("updated");
        assert_eq!(c.layout.left[0].props["use24h"], false);
        assert_eq!(c.layout.left[0].props["showSeconds"], true);
        // Now that it is stored, the machine is not asked again.
        let before = crate::sensors::test_support::machine_reads();
        assert!(machine_props_of(&c, &registry, &CTX, &id, &shown).is_empty());
        assert!(machine_props_of(&c, &registry, &CTX, "s_nope", &shown).is_empty());
        assert_eq!(crate::sensors::test_support::machine_reads(), before);
    }

    #[test]
    fn given_machine_props_are_not_asked_of_the_machine() {
        let registry = Registry::new(vec![&Clock]);
        let mut c = Config::default();
        let before = crate::sensors::test_support::machine_reads();
        let given = props(json!({ "use24h": true }));
        let machine = machine_props(&registry, &CTX, "clock", &Props::new(), &given);
        assert!(machine.is_empty());
        add(&mut c, &registry, &machine, "clock", Zone::Left, 0, &given).expect("added");
        assert_eq!(crate::sensors::test_support::machine_reads(), before);
        assert_eq!(c.layout.left[0].props["use24h"], true);
    }
}
