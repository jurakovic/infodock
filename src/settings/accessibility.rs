//! SPEC §14.4: every control on every Settings page has an accessible name that tells it apart,
//! and Tab reaches each one in reading order. The test walks the window's elements with Slint's
//! testing API and follows the keyboard focus as Tab moves it.

use std::sync::Arc;

use i_slint_backend_testing::{AccessibleRole, ElementHandle, ElementQuery};
use i_slint_core::accessibility::AccessibleStringProperty;
use i_slint_core::item_tree::ParentItemTraversalMode;
use i_slint_core::window::WindowInner;
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, LogicalSize};

use crate::app::App;
use crate::config::ConfigStore;
use crate::ui;

/// The roles of the elements a user acts on, which must have a name.
const CONTROLS: [AccessibleRole; 12] = [
    AccessibleRole::Button,
    AccessibleRole::Checkbox,
    AccessibleRole::Combobox,
    AccessibleRole::List,
    AccessibleRole::ListItem,
    AccessibleRole::RadioButton,
    AccessibleRole::Slider,
    AccessibleRole::Spinbox,
    AccessibleRole::Switch,
    AccessibleRole::Tab,
    AccessibleRole::TabList,
    AccessibleRole::TextInput,
];

/// The navigation's right edge, logical px: focus stops left of it are the navigation's.
const NAVIGATION: f32 = 181.0;

/// An element as screen readers see it: its role, name and place in the window.
#[derive(Debug, Clone, PartialEq)]
struct Seen {
    role: AccessibleRole,
    label: String,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

impl Seen {
    fn of(element: &ElementHandle) -> Option<Self> {
        let at = element.absolute_position();
        let size = element.size();
        Some(Self {
            role: element.accessible_role()?,
            label: element.accessible_label().unwrap_or_default().to_string(),
            x: at.x,
            y: at.y,
            width: size.width,
            height: size.height,
        })
    }

    fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x - 0.5
            && x <= self.x + self.width + 0.5
            && y >= self.y - 0.5
            && y <= self.y + self.height + 0.5
    }
}

/// The element with the keyboard focus: the focused item, or its nearest ancestor with an
/// accessible role (a widget's inner focus scope belongs to the widget).
fn focused(window: &slint::Window) -> Option<Seen> {
    let mut item = WindowInner::from_pub(window)
        .focus_item
        .borrow()
        .upgrade()?;
    while item.accessible_role() == AccessibleRole::None {
        item = item.parent_item(ParentItemTraversalMode::StopAtPopups)?;
    }
    let geometry = item.geometry();
    let at = item.map_to_window(geometry.origin);
    Some(Seen {
        role: item.accessible_role(),
        label: item
            .accessible_string_property(AccessibleStringProperty::Label)
            .unwrap_or_default()
            .to_string(),
        x: at.x,
        y: at.y,
        width: geometry.size.width,
        height: geometry.size.height,
    })
}

fn tab(window: &slint::Window) {
    let text: slint::SharedString = Key::Tab.into();
    window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window.dispatch_event(WindowEvent::KeyReleased { text });
    slint::platform::update_timers_and_animations();
}

/// The focus stops Tab visits once round, from the navigation on.
fn tab_stops(window: &slint::Window) -> Vec<Seen> {
    let mut stops: Vec<Seen> = Vec::new();
    for _ in 0..200 {
        tab(window);
        let Some(stop) = focused(window) else {
            continue;
        };
        if stops.first() == Some(&stop) {
            break;
        }
        stops.push(stop);
    }
    // The round starts where the focus was (the last page left it somewhere).
    if let Some(navigation) = stops.iter().position(|s| s.x < NAVIGATION) {
        stops.rotate_left(navigation);
    }
    stops
}

/// Whether `next` comes after `previous` in reading order: on a lower line, or further right on
/// the same one.
fn reads_after(previous: &Seen, next: &Seen) -> bool {
    let line = 4.0;
    next.y > previous.y + line || ((next.y - previous.y).abs() <= line && next.x > previous.x)
}

/// What is wrong on the page shown: unnamed controls, two with one name, controls Tab does not
/// reach, and stops out of reading order (the navigation first, then the page from top to
/// bottom). Also returns the stops.
fn check_page(window: &ui::SettingsWindow, page: &str) -> (Vec<String>, Vec<Seen>) {
    let mut problems = Vec::new();
    let controls: Vec<Seen> = ElementQuery::from_root(window)
        .match_descendants()
        .match_predicate(|e| {
            e.accessible_role().is_some_and(|r| CONTROLS.contains(&r))
                && e.size().width > 0.0
                && e.size().height > 0.0
                && e.computed_opacity() > 0.0
        })
        .find_all()
        .iter()
        .filter_map(Seen::of)
        .collect();
    for control in &controls {
        if control.label.trim().is_empty() {
            problems.push(format!("{page}: unnamed {control:?}"));
        }
    }

    let stops = tab_stops(window.window());
    let enabled = |c: &Seen| {
        ElementQuery::from_root(window)
            .match_descendants()
            .match_predicate({
                let c = c.clone();
                move |e| Seen::of(e).as_ref() == Some(&c)
            })
            .find_first()
            .and_then(|e| e.accessible_enabled())
            .unwrap_or(true)
    };
    // Items are reached with the arrow keys in their list, and a list may be a mere group (the
    // Layout manager's), so neither needs a stop of its own.
    for control in controls.iter().filter(|c| {
        !matches!(c.role, AccessibleRole::List | AccessibleRole::ListItem) && enabled(c)
    }) {
        let reached = stops
            .iter()
            .any(|s| control.contains(s.x + s.width / 2.0, s.y + s.height / 2.0));
        if !reached {
            problems.push(format!("{page}: Tab never reaches {control:?}"));
        }
    }
    for (i, stop) in stops.iter().enumerate() {
        if stops[..i]
            .iter()
            .any(|s| s.role == stop.role && s.label == stop.label)
        {
            problems.push(format!("{page}: two controls named {stop:?}"));
        }
    }
    let first_on_page = stops.iter().position(|s| s.x >= NAVIGATION);
    if let Some(first) = first_on_page
        && stops[first..].iter().any(|s| s.x < NAVIGATION)
    {
        problems.push(format!("{page}: the navigation is not first: {stops:?}"));
    }
    let on_page: Vec<&Seen> = stops.iter().filter(|s| s.x >= NAVIGATION).collect();
    for pair in on_page.windows(2) {
        if !reads_after(pair[0], pair[1]) {
            problems.push(format!(
                "{page}: Tab goes from {:?} back to {:?}",
                pair[0], pair[1]
            ));
        }
    }
    (problems, stops)
}

/// The first element with the accessible role `role` and the name `label`.
fn element(window: &ui::SettingsWindow, role: AccessibleRole, label: &str) -> ElementHandle {
    ElementHandle::find_by_accessible_label(window, label)
        .find(|e| e.accessible_role() == Some(role))
        .unwrap_or_else(|| panic!("a {role:?} named {label:?}"))
}

#[test]
fn every_control_has_a_name_and_tab_reaches_it_in_reading_order() {
    i_slint_backend_testing::init_no_event_loop();
    let dir = tempfile::tempdir().expect("temp dir");
    let (store, _) = ConfigStore::open(dir.path(), "test");
    let app = Arc::new(App::new(
        store,
        crate::platform::current(),
        dir.path().join("skins"),
    ));
    let logo = app.store.get().layout.center[0].id.clone();
    let routes = [
        "general".to_owned(),
        "bar".to_owned(),
        "appearance".to_owned(),
        "sensors".to_owned(),
        "popups".to_owned(),
        "applications".to_owned(),
        "help".to_owned(),
        "about".to_owned(),
        format!("sensor/{logo}"),
        "new?zone=left&index=0".to_owned(),
        "welcome".to_owned(),
    ];
    let mut problems = Vec::new();
    let classic = "appearance (classic)".to_owned();
    for route in routes.iter().chain([&classic]) {
        if *route == classic {
            // The skin gallery shows with the classic theme.
            app.store
                .update(|c| {
                    c.settings.theme = crate::config::model::Theme::Classic;
                    Ok(())
                })
                .expect("saved");
            super::WINDOW.with(|w| {
                if let Some(open) = w.borrow_mut().as_mut() {
                    super::fill(open, &app, &app.store.get().settings);
                }
            });
        }
        super::open(&app, route.split(' ').next().unwrap_or_default());
        let window = super::WINDOW
            .with(|w| w.borrow().as_ref().map(|open| open.window.clone_strong()))
            .expect("Settings is open");
        // Tall enough that no page scrolls while Tab moves through it.
        window.window().set_size(LogicalSize::new(760.0, 3000.0));
        slint::platform::update_timers_and_animations();
        // The window numbers its pages as Rust does.
        let pages = window.global::<ui::Pages>();
        assert_eq!(
            [
                pages.get_properties(),
                pages.get_picker(),
                pages.get_welcome()
            ],
            [
                super::properties::PAGE,
                super::picker::PAGE,
                super::welcome::PAGE
            ]
        );
        let page = |name: &str| super::model::PAGES.iter().position(|p| *p == name);
        assert_eq!(Some(pages.get_help() as usize), page("help"));
        assert_eq!(Some(pages.get_about() as usize), page("about"));
        let (found, stops) = check_page(&window, route);
        problems.extend(found);
        assert!(stops.len() > 2, "{route}: Tab stops at {stops:?}");

        // Lists of choices say which one is chosen, and items their place (SPEC §14.4).
        let t = crate::i18n::t;
        if route == "sensors" {
            let cpu = element(&window, AccessibleRole::ListItem, "CPU Graph");
            assert_eq!(
                (cpu.accessible_item_index(), cpu.accessible_item_count()),
                (Some(0), Some(3))
            );
            let down = crate::i18n::t_args("settings.layout.downNamed", &[("name", "CPU Graph")]);
            element(&window, AccessibleRole::Button, &down);
        }
        if route == "welcome" {
            let themes = element(
                &window,
                AccessibleRole::List,
                &t("settings.appearance.theme"),
            );
            assert_eq!(
                themes.accessible_value().as_deref(),
                Some(t("settings.appearance.theme.modern").as_str())
            );
        }
        if *route == classic {
            let skins = element(
                &window,
                AccessibleRole::List,
                &t("settings.appearance.skin"),
            );
            assert!(!skins.accessible_value().unwrap_or_default().is_empty());
            assert!(
                stops
                    .iter()
                    .any(|s| s.role == AccessibleRole::List
                        && s.label == t("settings.appearance.skin")),
                "Tab reaches the gallery"
            );
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
