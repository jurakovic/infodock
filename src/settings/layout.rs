//! The Layout manager on the Sensors page (SPEC §10.7): the sensors of each zone in a list, to
//! move (by dragging or with the arrow buttons), edit, remove and add, an accessible
//! alternative to dragging on the bar. The list follows the config, so changes made on the bar
//! show up at once.

use std::sync::Arc;

use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use super::picker::{self, Place};
use super::{Open, properties, with_open};
use crate::app::App;
use crate::config::model::{Layout, Zone};
use crate::i18n::t;
use crate::platform::Capabilities;
use crate::sensors::ops::index_after_removal;
use crate::sensors::registry::{Registry, registry};
use crate::sensors::{Props, schema};
use crate::ui;

/// The zones in the order the list shows them.
const ZONES: [Zone; 3] = [Zone::Left, Zone::Center, Zone::Right];

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LineKind {
    /// A zone's name, with its Add button.
    Heading,
    Sensor,
    /// The note in an empty zone.
    Empty,
}

/// A line of the list.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub kind: LineKind,
    pub zone: Zone,
    /// The sensor's index in its zone.
    pub index: usize,
    pub id: String,
    pub name: String,
    /// What tells the sensor apart (e.g. a Text sensor's text), or why it cannot run.
    pub detail: String,
    pub editable: bool,
}

impl Line {
    fn new(kind: LineKind, zone: Zone, name: String) -> Self {
        Self {
            kind,
            zone,
            index: 0,
            id: String::new(),
            name,
            detail: String::new(),
            editable: false,
        }
    }
}

fn zone_name(zone: Zone) -> String {
    t(match zone {
        Zone::Left => "settings.layout.left",
        Zone::Center => "settings.layout.center",
        Zone::Right => "settings.layout.right",
    })
}

/// The lines for `layout`: each zone's heading, then its sensors or the empty note.
pub fn lines(layout: &Layout, registry: &Registry, caps: &Capabilities) -> Vec<Line> {
    let mut lines = Vec::new();
    for zone in ZONES {
        lines.push(Line::new(LineKind::Heading, zone, zone_name(zone)));
        let sensors = layout.zone(zone);
        if sensors.is_empty() {
            lines.push(Line::new(LineKind::Empty, zone, t("settings.layout.empty")));
        }
        for (index, sensor) in sensors.iter().enumerate() {
            let name = registry.type_name(&sensor.kind);
            let detail = match registry.get(&sensor.kind) {
                None => t("settings.layout.unknown"),
                Some(ty) if !Registry::available(ty, caps) => t("app.notAvailable"),
                Some(ty) => {
                    let (props, _) = schema::resolve(ty.schema(), &sensor.props, &Props::new());
                    ty.summary(&props)
                        .filter(|summary| *summary != name)
                        .unwrap_or_default()
                }
            };
            lines.push(Line {
                kind: LineKind::Sensor,
                zone,
                index,
                id: sensor.id.clone(),
                editable: registry.editable(sensor, caps),
                name,
                detail,
            });
        }
    }
    lines
}

/// The insertion point, counted with the dragged sensor still in place, for a drop on the
/// boundary before line `boundary`: before the sensor there, else after the line above it.
pub fn insertion(lines: &[Line], boundary: usize) -> (Zone, usize) {
    if let Some(line) = lines.get(boundary)
        && line.kind == LineKind::Sensor
    {
        return (line.zone, line.index);
    }
    match boundary.checked_sub(1).and_then(|above| lines.get(above)) {
        Some(line) if line.kind == LineKind::Sensor => (line.zone, line.index + 1),
        Some(line) => (line.zone, 0),
        None => (Zone::Left, 0),
    }
}

/// Where sensor `id` goes when dropped at `insertion` (a [`ops::move_to`] target), or `None`
/// if it stays where it is.
///
/// [`ops::move_to`]: crate::sensors::ops::move_to
fn drop_move(layout: &Layout, id: &str, insertion: (Zone, usize)) -> Option<(Zone, usize)> {
    let source = layout.find(id)?;
    let (zone, index) = insertion;
    if source.0 == zone && (index == source.1 || index == source.1 + 1) {
        return None;
    }
    Some((zone, index_after_removal(source, zone, index)))
}

/// Where sensor `id` goes one step up or down the list: within its zone, or past the zone's
/// end into the next one. `None` at the very top or bottom.
pub fn step(layout: &Layout, id: &str, up: bool) -> Option<(Zone, usize)> {
    let (zone, index) = layout.find(id)?;
    let len = layout.zone(zone).len();
    let z = zone as usize;
    if up {
        if index > 0 {
            Some((zone, index - 1))
        } else {
            let above = ZONES.get(z.checked_sub(1)?)?;
            Some((*above, layout.zone(*above).len()))
        }
    } else if index + 1 < len {
        Some((zone, index + 1))
    } else {
        Some((*ZONES.get(z + 1)?, 0))
    }
}

/// Shows `layout` in the open window. The list is rebuilt only when a line changed, so other
/// changes keep the keyboard focus in it.
pub(super) fn fill(open: &Open, app: &App, layout: &Layout) {
    let lines = lines(layout, registry(), &app.platform.capabilities());
    let rows: Vec<ui::LayoutRow> = lines
        .iter()
        .map(|line| ui::LayoutRow {
            kind: match line.kind {
                LineKind::Heading => 0,
                LineKind::Sensor => 1,
                LineKind::Empty => 2,
            },
            zone: line.zone as i32,
            id: line.id.as_str().into(),
            name: line.name.as_str().into(),
            detail: line.detail.as_str().into(),
            editable: line.editable,
            up: line.kind == LineKind::Sensor && step(layout, &line.id, true).is_some(),
            down: line.kind == LineKind::Sensor && step(layout, &line.id, false).is_some(),
            index: line.index as i32,
            count: layout.zone(line.zone).len() as i32,
        })
        .collect();
    let state = open.window.global::<ui::SettingsState>();
    if state.get_layout_rows().iter().eq(rows.iter().cloned()) {
        return;
    }
    state.set_layout_rows(ModelRc::new(VecModel::from(rows)));
}

/// Shows why a change to the layout failed; clears it after one that worked.
fn report(result: Result<(), crate::config::ConfigError>) {
    let error: SharedString = match result {
        Ok(()) => SharedString::new(),
        Err(e) => {
            tracing::warn!(code = e.code(), "could not change the layout: {e}");
            e.message().into()
        }
    };
    with_open(|open| {
        open.window
            .global::<ui::SettingsState>()
            .set_save_error(error);
    });
}

/// Asks for the focus on sensor `id`'s Up (or Down) button once the list shows its move.
fn refocus(id: &str, up: bool) {
    with_open(|open| {
        let state = open.window.global::<ui::SettingsState>();
        state.set_layout_focus_id(id.into());
        state.set_layout_focus_up(up);
    });
}

/// Handles the list's callbacks.
pub(super) fn connect(window: &ui::SettingsWindow, app: &Arc<App>) {
    let state = window.global::<ui::SettingsState>();
    let a = Arc::clone(app);
    state.on_layout_move(move |id, up| {
        if let Some((zone, index)) = step(&a.store.get().layout, &id, up) {
            let result = a.move_sensor(&id, zone, index);
            if result.is_ok() {
                refocus(&id, up);
            }
            report(result);
        }
    });
    let a = Arc::clone(app);
    state.on_layout_drop(move |id, boundary| {
        let layout = a.store.get().layout;
        let lines = lines(&layout, registry(), &a.platform.capabilities());
        let Ok(boundary) = usize::try_from(boundary) else {
            return;
        };
        if let Some((zone, index)) = drop_move(&layout, &id, insertion(&lines, boundary)) {
            report(a.move_sensor(&id, zone, index));
        }
    });
    let a = Arc::clone(app);
    state.on_layout_edit(move |id| properties::open_form(&a, &id, false));
    let a = Arc::clone(app);
    state.on_layout_remove(move |id| report(a.remove_sensor(&id)));
    let a = Arc::clone(app);
    state.on_layout_add(move |zone| {
        let Some(zone) = usize::try_from(zone).ok().and_then(|z| ZONES.get(z)) else {
            return;
        };
        // The Add button of a zone appends to it.
        let index = a.store.get().layout.zone(*zone).len();
        picker::open_picker(&a, Place::At(*zone, index));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::model::{Config, SensorInstance};

    fn layout() -> Layout {
        // Left: cpu-graph, memory-meter, network-graph; center: image; right: date, time.
        Config::first_run("1.0.0").layout
    }

    fn shown(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|l| match l.kind {
                LineKind::Heading => format!("# {}", l.name),
                LineKind::Empty => format!("({})", l.name),
                LineKind::Sensor => format!("{} {}: {}", l.index, l.name, l.detail),
            })
            .collect()
    }

    #[test]
    fn lists_each_zone_with_its_sensors() {
        let mut layout = layout();
        layout.left.truncate(2);
        layout.right.clear();
        // A type from a newer version.
        layout.left.push(SensorInstance {
            id: "s_mail".into(),
            kind: "mail".into(),
            props: Default::default(),
        });
        let lines = lines(&layout, registry(), &Capabilities::default());
        assert_eq!(
            shown(&lines),
            [
                "# Left",
                "0 CPU Graph: ",
                "1 Memory Meter: ",
                "2 mail: Not available in this version",
                "# Centre",
                "0 Image: Infodock logo",
                "# Right",
                "(No sensors)",
            ]
        );
        assert!(lines[1].editable, "the CPU graph");
        assert!(!lines[3].editable, "a placeholder");
        assert!(lines[5].editable);
    }

    #[test]
    fn drops_between_lines() {
        let layout = layout();
        let lines = lines(&layout, registry(), &Capabilities::default());
        // 0 # Left, 1–3 left sensors, 4 # Centre, 5 image, 6 # Right, 7–8 right sensors.
        assert_eq!(insertion(&lines, 0), (Zone::Left, 0));
        assert_eq!(insertion(&lines, 1), (Zone::Left, 0));
        assert_eq!(insertion(&lines, 3), (Zone::Left, 2));
        // Above the centre heading: the end of the left zone.
        assert_eq!(insertion(&lines, 4), (Zone::Left, 3));
        assert_eq!(insertion(&lines, 5), (Zone::Center, 0));
        assert_eq!(insertion(&lines, 6), (Zone::Center, 1));
        assert_eq!(insertion(&lines, 7), (Zone::Right, 0));
        assert_eq!(insertion(&lines, 9), (Zone::Right, 2));
        assert_eq!(insertion(&lines, 99), (Zone::Left, 0));

        let memory = &layout.left[1].id;
        // Next to itself: no move.
        assert_eq!(drop_move(&layout, memory, (Zone::Left, 1)), None);
        assert_eq!(drop_move(&layout, memory, (Zone::Left, 2)), None);
        // After the network graph: index 3 counted in place, 2 after removal.
        assert_eq!(
            drop_move(&layout, memory, (Zone::Left, 3)),
            Some((Zone::Left, 2))
        );
        assert_eq!(
            drop_move(&layout, memory, (Zone::Right, 0)),
            Some((Zone::Right, 0))
        );
    }

    #[test]
    fn drops_into_an_empty_zone() {
        let mut layout = layout();
        layout.center.clear();
        let lines = lines(&layout, registry(), &Capabilities::default());
        // 4 # Centre, 5 (No sensors), 6 # Right.
        assert_eq!(insertion(&lines, 5), (Zone::Center, 0));
        assert_eq!(insertion(&lines, 6), (Zone::Center, 0));
    }

    #[test]
    fn steps_cross_zones() {
        let layout = layout();
        let first = &layout.left[0].id;
        let logo = &layout.center[0].id;
        let last = &layout.right[1].id;
        assert_eq!(step(&layout, first, true), None);
        assert_eq!(step(&layout, first, false), Some((Zone::Left, 1)));
        assert_eq!(step(&layout, logo, true), Some((Zone::Left, 3)));
        assert_eq!(step(&layout, logo, false), Some((Zone::Right, 0)));
        assert_eq!(step(&layout, last, true), Some((Zone::Right, 0)));
        assert_eq!(step(&layout, last, false), None);
        assert_eq!(step(&layout, "s_gone", false), None);
    }
}
