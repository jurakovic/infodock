//! The sensor picker (SPEC §6.6), opened on `new?zone=…&index=…` or `new?target=<id>`: the
//! sensor types that can run here, grouped by category and sorted by name, a search box, and
//! for a target sensor an Insert Before / After choice. Adding a sensor opens its properties;
//! cancelling those removes it again (`properties.rs`).

use std::sync::Arc;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::{WINDOW, properties};
use crate::app::App;
use crate::config::model::{Layout, Zone};
use crate::i18n::t;
use crate::platform::Platform;
use crate::sensors::registry::Registry;
use crate::sensors::{Category, DefaultsCtx, Props};
use crate::ui;

/// The page index of the picker in the window.
pub(super) const PAGE: i32 = 7;

/// A type the picker offers.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub kind: &'static str,
    pub category: Category,
    pub name: String,
    pub description: String,
}

/// The types that can run on `platform`, by category and then name (SPEC §6.6), with any
/// note a type adds to its description on this machine.
pub fn entries(registry: &Registry, platform: &dyn Platform) -> Vec<Entry> {
    let caps = platform.capabilities();
    let ctx = DefaultsCtx { platform };
    let mut entries: Vec<Entry> = registry
        .types()
        .iter()
        .filter(|ty| Registry::available(**ty, &caps))
        .map(|ty| Entry {
            kind: ty.id(),
            category: ty.category(),
            name: t(ty.name()),
            description: match ty.picker_note(&ctx) {
                Some(note) => format!("{} {}", t(ty.description()), t(note)),
                None => t(ty.description()),
            },
        })
        .collect();
    entries.sort_by_cached_key(|e| (e.category, e.name.to_lowercase()));
    entries
}

/// A line of the list: a category heading or an entry (by index into [`entries`]).
#[derive(Debug, Clone, PartialEq)]
pub enum Line {
    Heading(String),
    Entry(usize),
}

/// The lines for the entries whose name or description contains `query` (any case).
pub fn lines(entries: &[Entry], query: &str) -> Vec<Line> {
    let query = query.trim().to_lowercase();
    let mut lines = Vec::new();
    let mut heading = None;
    for (i, e) in entries.iter().enumerate() {
        let matches = query.is_empty()
            || e.name.to_lowercase().contains(&query)
            || e.description.to_lowercase().contains(&query);
        if !matches {
            continue;
        }
        if heading != Some(e.category) {
            heading = Some(e.category);
            lines.push(Line::Heading(t(e.category.name())));
        }
        lines.push(Line::Entry(i));
    }
    lines
}

/// Where the picker inserts.
#[derive(Debug, Clone, PartialEq)]
pub enum Place {
    /// At `index` of `zone` (clamped by the add).
    At(Zone, usize),
    /// Before or after sensor `id`.
    Next(String),
}

/// The selection after a search listed `lines` (entry indices, `-1` for headings): the
/// selected entry while it is still listed, else the first match, so that Enter adds it.
pub fn searched(lines: &[i32], selected: i32) -> i32 {
    if selected >= 0 && lines.contains(&selected) {
        return selected;
    }
    lines.iter().copied().find(|e| *e >= 0).unwrap_or(-1)
}

/// The selection after Up or Down in the search box: the listed entry above or below the
/// selected one, staying at the ends; with none selected, the last or the first.
pub fn stepped(lines: &[i32], selected: i32, up: bool) -> i32 {
    let entries: Vec<i32> = lines.iter().copied().filter(|e| *e >= 0).collect();
    let next = match entries.iter().position(|e| *e == selected) {
        None if up => entries.last(),
        None => entries.first(),
        Some(i) if up => entries.get(i.saturating_sub(1)),
        Some(i) => entries.get(i + 1).or(entries.last()),
    };
    next.copied().unwrap_or(-1)
}

/// Where a `new?zone=…&index=…&target=…` route inserts. Without a target or index: the end of
/// the left or center zone, the start of the right one (SPEC §6.6).
pub fn place(
    layout: &Layout,
    zone: Option<Zone>,
    index: Option<usize>,
    target: Option<String>,
) -> Place {
    if let Some(id) = target {
        return Place::Next(id);
    }
    let zone = zone.unwrap_or(Zone::Left);
    let index = index.unwrap_or(match zone {
        Zone::Left => layout.left.len(),
        Zone::Center => layout.center.len(),
        Zone::Right => 0,
    });
    Place::At(zone, index)
}

/// The zone and index for `place`; `after` for a target sensor. `None` if the target is gone.
pub fn insertion(layout: &Layout, place: &Place, after: bool) -> Option<(Zone, usize)> {
    match place {
        Place::At(zone, index) => Some((*zone, *index)),
        Place::Next(id) => {
            let (zone, index) = layout.find(id)?;
            Some((zone, index + usize::from(after)))
        }
    }
}

/// What the open picker shows and inserts.
pub(super) struct PickerState {
    entries: Vec<Entry>,
    /// The entry index of each line (`-1` for headings), as shown.
    lines: Vec<i32>,
    place: Place,
}

fn rows(entries: &[Entry], query: &str) -> (Vec<ui::PickerRow>, Vec<i32>) {
    let lines = lines(entries, query);
    let rows = lines
        .iter()
        .map(|line| match line {
            Line::Heading(text) => ui::PickerRow {
                heading: true,
                name: text.as_str().into(),
                description: SharedString::new(),
                entry: -1,
            },
            Line::Entry(i) => ui::PickerRow {
                heading: false,
                name: entries[*i].name.as_str().into(),
                description: entries[*i].description.as_str().into(),
                entry: *i as i32,
            },
        })
        .collect();
    let indices = lines
        .iter()
        .map(|line| match line {
            Line::Heading(_) => -1,
            Line::Entry(i) => *i as i32,
        })
        .collect();
    (rows, indices)
}

/// Opens the picker inserting at `place`.
pub(super) fn open_picker(app: &App, place: Place) {
    WINDOW.with(|w| {
        let Ok(mut w) = w.try_borrow_mut() else {
            return;
        };
        let Some(open) = w.as_mut() else {
            return;
        };
        properties::cancel(open, app);
        let state = open.window.global::<ui::SettingsState>();
        let entries = entries(crate::sensors::registry::registry(), app.platform.as_ref());
        let (shown, lines) = rows(&entries, "");
        state.set_picker_rows(ModelRc::new(VecModel::from(shown)));
        state.set_picker_search(SharedString::new());
        state.set_picker_selected(-1);
        state.set_picker_after(1);
        let target = match &place {
            Place::Next(id) => app
                .store
                .get()
                .layout
                .all()
                .find(|s| &s.id == id)
                .map(|s| crate::sensors::registry::registry().type_name(&s.kind))
                .unwrap_or_default(),
            Place::At(..) => String::new(),
        };
        state.set_picker_target(target.into());
        state.set_save_error(SharedString::new());
        open.picker = Some(PickerState {
            entries,
            lines,
            place,
        });
        state.set_page(PAGE);
    });
}

/// Adds the selected type and opens its properties.
fn add(app: &Arc<App>) {
    let pending = WINDOW.with(|w| {
        let w = w.try_borrow().ok()?;
        let open = w.as_ref()?;
        let picker = open.picker.as_ref()?;
        let state = open.window.global::<ui::SettingsState>();
        let entry = usize::try_from(state.get_picker_selected()).ok()?;
        let kind = picker.entries.get(entry)?.kind;
        let after = state.get_picker_after() == 1;
        Some((kind, picker.place.clone(), after))
    });
    let Some((kind, place, after)) = pending else {
        return;
    };
    let layout = app.store.get().layout;
    // A target that has gone since: the end of the left zone.
    let (zone, index) =
        insertion(&layout, &place, after).unwrap_or((Zone::Left, layout.left.len()));
    match app.add_sensor(kind, zone, index, &Props::new()) {
        Ok(id) => {
            WINDOW.with(|w| {
                if let Ok(mut w) = w.try_borrow_mut()
                    && let Some(open) = w.as_mut()
                {
                    open.picker = None;
                }
            });
            properties::open_form(app, &id, true);
        }
        Err(e) => WINDOW.with(|w| {
            if let Ok(w) = w.try_borrow()
                && let Some(open) = w.as_ref()
            {
                let state = open.window.global::<ui::SettingsState>();
                state.set_save_error(e.to_string().into());
            }
        }),
    }
}

/// Handles the picker's callbacks.
pub(super) fn connect(window: &ui::SettingsWindow, app: &Arc<App>) {
    let state = window.global::<ui::SettingsState>();
    state.on_picker_searched(|query| {
        WINDOW.with(|w| {
            let Ok(mut w) = w.try_borrow_mut() else {
                return;
            };
            let Some(open) = w.as_mut() else {
                return;
            };
            let Some(picker) = open.picker.as_mut() else {
                return;
            };
            let (shown, lines) = rows(&picker.entries, &query);
            let state = open.window.global::<ui::SettingsState>();
            state.set_picker_selected(searched(&lines, state.get_picker_selected()));
            picker.lines = lines;
            state.set_picker_rows(ModelRc::new(VecModel::from(shown)));
        });
    });
    state.on_picker_step(|up| {
        WINDOW.with(|w| {
            if let Ok(w) = w.try_borrow()
                && let Some(open) = w.as_ref()
                && let Some(picker) = open.picker.as_ref()
            {
                let state = open.window.global::<ui::SettingsState>();
                state.set_picker_selected(stepped(&picker.lines, state.get_picker_selected(), up));
            }
        });
    });
    let a = Arc::clone(app);
    state.on_picker_add(move || add(&a));
    state.on_picker_cancel(|| {
        WINDOW.with(|w| {
            if let Ok(mut w) = w.try_borrow_mut()
                && let Some(open) = w.as_mut()
            {
                open.picker = None;
                open.window
                    .global::<ui::SettingsState>()
                    .set_page(super::model::SENSORS_PAGE as i32);
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::model::Config;
    use crate::platform::{Capabilities, MonitorInfo, OsAppearance};
    use crate::sensors::registry::registry;
    use crate::sensors::test_support::NoPlatform;

    /// A single-processor computer with per-processor counters.
    struct OneProcessor;

    impl Platform for OneProcessor {
        fn capabilities(&self) -> Capabilities {
            Capabilities {
                per_core_cpu: true,
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
        fn logical_processors(&self) -> u32 {
            1
        }
    }

    #[test]
    fn notes_one_processor_for_the_dual_graph() {
        let entries = entries(registry(), &OneProcessor);
        let dual = entries
            .iter()
            .find(|e| e.kind == "cpu-graph-dual")
            .expect("listed with per-processor counters");
        assert!(
            dual.description
                .ends_with("both halves show the same data."),
            "{}",
            dual.description
        );
    }

    #[test]
    fn lists_types_by_category_and_name() {
        let entries = entries(registry(), &NoPlatform);
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Image",
                "Spacer",
                "Text",
                "Date",
                "Time",
                "Uptime",
                "CPU Graph",
                "CPU Meter",
                "Disk Meter",
                "Memory Graph",
                "Memory Meter",
                "Swap Graph",
                "Swap Meter",
                "Network Graph",
                "Disk",
                "Folder Menu",
                "Power",
                "Shortcut"
            ]
        );
        assert_eq!(
            lines(&entries, ""),
            [
                Line::Heading("General".into()),
                Line::Entry(0),
                Line::Entry(1),
                Line::Entry(2),
                Line::Heading("Info".into()),
                Line::Entry(3),
                Line::Entry(4),
                Line::Entry(5),
                Line::Heading("System".into()),
                Line::Entry(6),
                Line::Entry(7),
                Line::Entry(8),
                Line::Entry(9),
                Line::Entry(10),
                Line::Entry(11),
                Line::Entry(12),
                Line::Heading("Network".into()),
                Line::Entry(13),
                Line::Heading("Launchers".into()),
                Line::Entry(14),
                Line::Entry(15),
                Line::Entry(16),
                Line::Entry(17)
            ]
        );
    }

    #[test]
    fn search_matches_names_and_descriptions() {
        let entries = entries(registry(), &NoPlatform);
        assert_eq!(
            lines(&entries, "TEXT"),
            [Line::Heading("General".into()), Line::Entry(2)]
        );
        // "Empty space between sensors…" is the Spacer's description.
        assert_eq!(lines(&entries, "space")[1], Line::Entry(1));
        assert!(lines(&entries, "battery").is_empty());
    }

    #[test]
    fn a_search_keeps_the_selection_or_selects_the_first_match() {
        // A heading, then entries 0 and 2; another heading, then entry 5.
        let lines = [-1, 0, 2, -1, 5];
        assert_eq!(searched(&lines, 2), 2);
        assert_eq!(searched(&lines, 1), 0, "no longer listed");
        assert_eq!(searched(&lines, -1), 0);
        assert_eq!(searched(&[], 2), -1, "nothing matches");
    }

    #[test]
    fn up_and_down_move_over_the_entries() {
        let lines = [-1, 0, 2, -1, 5];
        assert_eq!(stepped(&lines, -1, false), 0);
        assert_eq!(stepped(&lines, -1, true), 5);
        assert_eq!(stepped(&lines, 0, false), 2);
        assert_eq!(stepped(&lines, 2, false), 5, "past the heading");
        assert_eq!(stepped(&lines, 5, false), 5, "stays at the end");
        assert_eq!(stepped(&lines, 5, true), 2);
        assert_eq!(stepped(&lines, 0, true), 0, "stays at the start");
        assert_eq!(stepped(&[], -1, false), -1);
    }

    #[test]
    fn inserts_at_the_zone_or_next_to_the_target() {
        let config = Config::first_run("1.0.0");
        let layout = &config.layout;
        assert_eq!(
            insertion(layout, &Place::At(Zone::Right, 0), true),
            Some((Zone::Right, 0))
        );
        let memory = layout.left[1].id.clone();
        let next = Place::Next(memory);
        assert_eq!(insertion(layout, &next, false), Some((Zone::Left, 1)));
        assert_eq!(insertion(layout, &next, true), Some((Zone::Left, 2)));
        assert_eq!(insertion(layout, &Place::Next("s_gone".into()), true), None);
    }

    #[test]
    fn routes_without_an_index_append_or_start_the_right_zone() {
        let config = Config::first_run("1.0.0");
        let layout = &config.layout;
        let left = layout.left.len();
        assert_eq!(place(layout, None, None, None), Place::At(Zone::Left, left));
        assert_eq!(
            place(layout, Some(Zone::Center), None, None),
            Place::At(Zone::Center, layout.center.len())
        );
        assert_eq!(
            place(layout, Some(Zone::Right), None, None),
            Place::At(Zone::Right, 0)
        );
        assert_eq!(
            place(layout, Some(Zone::Right), Some(2), None),
            Place::At(Zone::Right, 2)
        );
        assert_eq!(
            place(layout, Some(Zone::Right), Some(2), Some("s_1".into())),
            Place::Next("s_1".into())
        );
    }
}
