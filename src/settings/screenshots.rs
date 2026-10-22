//! Renders each Settings page to `target/screenshots/settings-*.png` with the software
//! renderer (`cargo test render_settings_pages -- --ignored`), and drives the window with
//! clicks and keys as the user gives them. CI uploads the PNGs.

use std::rc::Rc;
use std::sync::Arc;

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use serde_json::json;
use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::platform::{Key, Platform, PointerEventButton, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, PhysicalSize, SharedString};

use super::window::{set as set_window, with_open, with_open_mut};
use super::{Open, fill};
use crate::app::App;
use crate::config::ConfigStore;
use crate::i18n::t;
use crate::test_util::{screenshot_path, write_png};
use crate::ui;

struct Offscreen(Rc<MinimalSoftwareWindow>);

impl Platform for Offscreen {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

/// Where the shown element with the accessible role `role` and the name `label` is in the
/// window: its top left corner and size, logical px. Clicks find their control by it, so they
/// follow the layout.
fn bounds(ui: &ui::SettingsWindow, role: AccessibleRole, label: &str) -> (f32, f32, f32, f32) {
    let element = ElementHandle::find_by_accessible_label(ui, label)
        .find(|e| e.accessible_role() == Some(role) && e.size().width > 0.0)
        .unwrap_or_else(|| panic!("a {role:?} named {label:?}"));
    let (at, size) = (element.absolute_position(), element.size());
    (at.x, at.y, size.width, size.height)
}

/// The centre of the element `bounds` finds.
fn center(ui: &ui::SettingsWindow, role: AccessibleRole, label: &str) -> (f32, f32) {
    let (x, y, width, height) = bounds(ui, role, label);
    (x + width / 2.0, y + height / 2.0)
}

/// Where a click on the slider named `label` lands on its thumb, which sits at `fraction` of
/// its range, so the click changes nothing.
fn thumb(ui: &ui::SettingsWindow, label: &str, fraction: f32) -> (f32, f32) {
    // The Fluent thumb is 20 px wide and travels the slider's width less its own.
    let (x, y, width, height) = bounds(ui, AccessibleRole::Slider, label);
    (x + 10.0 + (width - 20.0) * fraction, y + height / 2.0)
}

/// Renders each Settings page with the software renderer, for reviewing the layout without
/// running the app. CI uploads the PNGs.
#[test]
#[ignore = "writes PNGs to target/screenshots; run with --ignored"]
fn render_settings_pages() {
    let (width, height) = (760, 560);
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Offscreen(window.clone())))
        .expect("the first Slint use in this process");
    let dir = tempfile::tempdir().expect("temp dir");
    let (store, _) = ConfigStore::open(dir.path(), "test");
    let app = Arc::new(App::new(
        store,
        crate::platform::current(),
        dir.path().join("skins"),
    ));

    let ui = ui::SettingsWindow::new().expect("the window builds");
    crate::install_i18n!(ui);
    super::connect(&ui, &app);
    let mut open = Open {
        window: ui.clone_strong(),
        monitor_keys: Vec::new(),
        families: vec!["Arial".into(), "Consolas".into(), "Segoe UI".into()],
        fonts: Vec::new(),
        form: None,
        picker: None,
        gallery: super::appearance::Gallery::default(),
        scheme: None,
        welcome: super::welcome::Welcome::default(),
    };
    fill(&mut open, &app, &app.store.get().settings);
    super::layout::fill(&open, &app, &app.store.get().layout);
    ui.show().expect("shown");

    let render_sized = |name: &str, height: u32| {
        window.set_size(PhysicalSize::new(width, height));
        // Let state animations (e.g. a button becoming disabled) finish.
        std::thread::sleep(std::time::Duration::from_millis(200));
        slint::platform::update_timers_and_animations();
        window.request_redraw();
        let mut pixels = vec![PremultipliedRgbaColor::default(); (width * height) as usize];
        window.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, width as usize);
        });
        // The window is opaque, so premultiplied and straight alpha agree.
        let rgba: Vec<u8> = pixels
            .iter()
            .flat_map(|p| [p.red, p.green, p.blue, p.alpha])
            .collect();
        write_png(&screenshot_path(name), width, height, &rgba);
    };
    let render = |name: &str| render_sized(name, height);
    let state = ui.global::<ui::SettingsState>();
    for (index, page) in super::model::PAGES.iter().enumerate() {
        state.set_page(index as i32);
        render(&format!("settings-{page}"));
    }
    // The whole Help and About pages: the tips; the licence, the crates and Slint's
    // attribution.
    for page in ["help", "about"] {
        let index = super::model::PAGES.iter().position(|p| *p == page);
        state.set_page(index.unwrap_or_default() as i32);
        render_sized(&format!("settings-{page}-full"), 1500);
    }
    // The whole Bar page.
    state.set_page(1);
    render_sized("settings-bar-full", 1000);
    state.set_mode(1);
    render("settings-bar-floating");
    state.set_mode(0);
    // Appearance with the classic theme: the skin gallery with a pack made here (a blue
    // gradient; a swatch where there is no renderer), the custom image and a missing skin.
    let pack = app.skins.join("blue-steel");
    std::fs::create_dir_all(&pack).expect("pack folder");
    std::fs::write(
        pack.join("skin.json"),
        r#"{ "name": "Blue steel", "image": "tile.png" }"#,
    )
    .expect("skin.json");
    let tile: Vec<u8> = (0..25u8)
        .flat_map(|y| [40 + y * 4, 80 + y * 3, 160 + y * 3, 255].repeat(8))
        .collect();
    write_png(&pack.join("tile.png"), 8, 25, &tile);
    let broken = app.skins.join("broken");
    std::fs::create_dir_all(&broken).expect("pack folder");
    std::fs::write(broken.join("skin.json"), r#"{ "image": "tile.png" }"#).expect("skin.json");
    let set = |open: &mut Open, change: &dyn Fn(&mut crate::config::model::Settings)| {
        app.store
            .update(|c| {
                change(&mut c.settings);
                Ok(())
            })
            .expect("saved");
        fill(open, &app, &app.store.get().settings);
    };
    state.set_page(super::appearance::PAGE);
    set(&mut open, &|s| {
        s.theme = crate::config::model::ThemeId::Classic
    });
    let installed = crate::i18n::t_args(
        "settings.appearance.packs.installed",
        &[("name", "Blue steel")],
    );
    state.set_skin_install_message(installed.into());
    render_sized("settings-appearance-classic", 1200);
    state.set_skin_install_message(SharedString::new());
    set(&mut open, &|s| {
        s.classic.skin = "custom".into();
        s.classic.custom_skin = "wood.png".into();
    });
    render_sized("settings-appearance-custom", 1000);
    set(&mut open, &|s| s.classic.skin = "gone".into());
    render("settings-appearance-missing");
    // The colour scheme (SPEC §10.3), whatever the OS: dark, then light.
    set(&mut open, &|s| {
        *s = crate::config::model::Settings {
            color_scheme: crate::config::model::ColorScheme::Dark,
            ..Default::default()
        };
    });
    // The colours animate from the first frame that reads them: drawn twice.
    (0..2).for_each(|_| render("settings-appearance-dark"));
    state.set_page(super::model::SENSORS_PAGE as i32);
    render("settings-sensors-dark");
    state.set_page(super::appearance::PAGE);
    set(&mut open, &|s| {
        s.color_scheme = crate::config::model::ColorScheme::Light
    });
    (0..2).for_each(|_| render("settings-appearance-light"));
    set(&mut open, &|s| {
        *s = crate::config::model::Settings::default();
    });
    // The whole Sensors page, with the Layout manager; then a Remove waiting to be confirmed.
    state.set_page(super::model::SENSORS_PAGE as i32);
    render_sized("settings-sensors-layout", 1000);
    let logo = app.store.get().layout.center[0].id.clone();
    state.set_layout_confirm(logo.as_str().into());
    render_sized("settings-sensors-remove", 1000);
    state.set_layout_confirm(SharedString::new());
    // Drag memory-meter onto the lower half of network-graph: it moves to the end of the left
    // zone.
    let pointer = |event: fn(LogicalPosition) -> WindowEvent, x: f32, y: f32| {
        window.dispatch_event(event(LogicalPosition::new(x, y)));
    };
    window.set_size(PhysicalSize::new(width, 1000));
    let (x, from) = center(&ui, AccessibleRole::ListItem, "Memory Meter");
    let (_, top, _, line) = bounds(&ui, AccessibleRole::ListItem, "Network Graph, Automatic");
    let to = top + line * 0.75;
    pointer(
        |position| WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        },
        x,
        from,
    );
    for y in [from + 11.0, (from + to) / 2.0, to] {
        pointer(|position| WindowEvent::PointerMoved { position }, x, y);
    }
    render_sized("settings-sensors-drag", 1000);
    pointer(
        |position| WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        },
        x,
        to,
    );
    let kinds: Vec<_> = app
        .store
        .get()
        .layout
        .left
        .iter()
        .map(|s| s.kind.clone())
        .collect();
    assert_eq!(kinds, ["cpu-graph", "network-graph", "memory-meter"]);

    // The hotkey button while it waits for keys, with a refused combination.
    state.set_page(0);
    state.set_recording_hotkey(true);
    let taken = crate::i18n::t_args("settings.general.hotkey.taken", &[("hotkey", "Ctrl+K")]);
    state.set_hotkey_error(taken.into());
    render("settings-general-recording");
    state.set_recording_hotkey(false);
    state.set_hotkey_error(SharedString::new());

    // From here on the callbacks find the window, as in the app.
    set_window(Some(open));
    // Clicks and keys as the user gives them. Neither the navigation nor a button takes the
    // focus, so no input may wait for its field to lose it.
    let settle = || {
        slint::platform::update_timers_and_animations();
        window.request_redraw();
        let size = window.size();
        let mut pixels =
            vec![PremultipliedRgbaColor::default(); (size.width * size.height) as usize];
        window.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, size.width as usize);
        });
    };
    let click_at = |(x, y): (f32, f32)| {
        let position = LogicalPosition::new(x, y);
        let button = PointerEventButton::Left;
        window.dispatch_event(WindowEvent::PointerPressed { position, button });
        window.dispatch_event(WindowEvent::PointerReleased { position, button });
        settle();
    };
    let key = |text: SharedString| {
        window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        window.dispatch_event(WindowEvent::KeyReleased { text });
    };
    let click = |role: AccessibleRole, label: &str| click_at(center(&ui, role, label));
    let retype = |label: &str, text: &str| {
        click(AccessibleRole::TextInput, label);
        key(Key::End.into());
        for _ in 0..16 {
            key(Key::Backspace.into());
        }
        text.chars().for_each(|c| key(c.into()));
        settle();
    };
    let (general, bar, appearance, sensors, popups, applications) = (
        "general",
        "bar",
        "appearance",
        "sensors",
        "popups",
        "applications",
    );
    let nav = |page: &str| {
        click(
            AccessibleRole::ListItem,
            &t(&format!("settings.nav.{page}")),
        )
    };

    // The navigation takes the first Tab; the arrow keys, Home and End move between pages.
    key(Key::Tab.into());
    key(Key::DownArrow.into());
    settle();
    assert_eq!(state.get_page(), 1);
    key(Key::End.into());
    settle();
    assert_eq!(state.get_page(), 7);
    key(Key::Home.into());
    key(Key::UpArrow.into());
    settle();
    assert_eq!(state.get_page(), 0);

    // A sensor moved with the keyboard keeps the focus on its button, so it moves on.
    nav(sensors);
    key(Key::Tab.into()); // the left zone's Add
    key(Key::Tab.into()); // cpu-graph's Down (its Up is disabled)
    for _ in 0..2 {
        key(' '.into());
        // In the app the store's listener shows the change.
        with_open(|open| super::layout::fill(open, &app, &app.store.get().layout));
        settle();
    }
    let left = || -> Vec<String> {
        let layout = app.store.get().layout;
        layout.left.iter().map(|s| s.kind.clone()).collect()
    };
    assert_eq!(left(), ["network-graph", "memory-meter", "cpu-graph"]);
    nav(general);

    // Appearance: a click on a gallery tile chooses its skin, and once the gallery has the
    // focus, the arrow keys move on.
    let refill = || {
        with_open_mut(|open| fill(open, &app, &app.store.get().settings));
        settle();
    };
    let skin = || app.store.get().settings.classic.skin;
    nav(appearance);
    state.invoke_set_choice("theme".into(), 1);
    window.set_size(PhysicalSize::new(width, 1000));
    refill();
    click(AccessibleRole::ListItem, "Carbon");
    assert_eq!(skin(), "carbon");
    refill();
    key(Key::RightArrow.into());
    assert_eq!(skin(), "glass");
    // A pack installed shows in the gallery and the list at once.
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/skins/sunset.zip");
    crate::look::skins::install::install(&app.skins, &fixture).expect("installed");
    super::appearance::packs_changed(&app);
    use slint::Model;
    assert!(state.get_skins().iter().any(|t| t.name == "Sunset"));
    assert!(state.get_skin_packs().iter().any(|p| p.id == "Sunset"));
    state.invoke_set_choice("theme".into(), 0);
    window.set_size(PhysicalSize::new(width, height));
    refill();

    // Leaving General ends recording the hotkey.
    state.set_recording_hotkey(true);
    nav(bar);
    assert_eq!(state.get_page(), 1);
    assert!(!state.get_recording_hotkey());

    // A dragged slider snaps to its step (10 px for the width) when released, so it shows
    // what is saved.
    state.set_mode(1);
    settle();
    let before = state.get_floating_width();
    let (x, y, _, _) = bounds(&ui, AccessibleRole::Slider, &t("settings.bar.width"));
    let (from, to) = (x + 15.0, x + 56.0);
    pointer(
        |position| WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        },
        from,
        y + 10.0,
    );
    pointer(
        |position| WindowEvent::PointerMoved { position },
        to,
        y + 10.0,
    );
    pointer(
        |position| WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        },
        to,
        y + 10.0,
    );
    settle();
    let width = state.get_floating_width();
    assert_ne!(width, before, "the drag moved the slider");
    assert_eq!(width % 10.0, 0.0, "{width}");
    let floating = app.store.get().settings.bar.floating;
    assert_eq!(floating.map(|f| f.width as f32), Some(width));
    state.set_mode(0);

    // A colour is saved as soon as it is valid; one left incomplete is not shown again.
    nav(popups);
    let popup_background = t("settings.popups.background");
    retype(&popup_background, "#20b7ff");
    let background = || app.store.get().settings.popups.background;
    assert_eq!(background().as_deref(), Some("#20B7FF"));
    key("f".into());
    nav(general);
    nav(popups);
    assert_eq!(state.get_popup_background(), "#20B7FF");
    assert_eq!(background().as_deref(), Some("#20B7FF"));
    state.invoke_reset("popups.background".into());
    // Leaving the colour box and clicking the slider without changing them keep the theme
    // defaults (SPEC §11.2).
    nav(general);
    nav(popups);
    click(AccessibleRole::TextInput, &popup_background);
    click(AccessibleRole::TextInput, &t("settings.popups.textColor"));
    let opacity = state.get_popup_opacity() / 100.0;
    click_at(thumb(&ui, &t("settings.popups.opacity"), opacity));
    let popups_settings = app.store.get().settings.popups;
    assert_eq!(popups_settings.background, None);
    assert_eq!(popups_settings.opacity, None);

    // Start with Windows is saved at once (nothing starts at sign-in on Linux).
    nav(general);
    let autostart = || app.store.get().settings.autostart;
    let autostart_switch = t("settings.general.autostart");
    click(AccessibleRole::Switch, &autostart_switch);
    assert!(autostart());
    click(AccessibleRole::Switch, &autostart_switch);
    assert!(!autostart());
    // The calendar app is saved as it is typed (SPEC §10.6).
    nav(applications);
    retype(&t("settings.apps.calendar"), "outlookcal:");
    assert_eq!(app.store.get().settings.apps.calendar, "outlookcal:");

    // Properties (SPEC §6.7): the logo's, then with a custom colour; a new Text sensor's.
    super::properties::open_form(&app, &logo, false);
    render("settings-properties-image");
    state.invoke_prop_color_mode("color".into(), 1, "#20B7FF".into());
    render("settings-properties-image-custom");
    // The form's boxes are named by their rows.
    let row = |key: &str| -> String {
        let rows = state.get_props();
        let found = rows
            .iter()
            .find(|r| r.key == key)
            .map(|r| r.label.to_string());
        found.unwrap_or_else(|| panic!("a row {key:?}"))
    };
    // An incomplete colour shows its error on the row and disables Save (SPEC §6.7).
    retype(&row("color"), "#12345");
    assert!(!state.get_props_valid());
    render("settings-properties-invalid");
    // A typed path and colour are what Save stores.
    retype(&row("color"), "#123abc");
    assert!(state.get_props_valid());
    retype(&row("image"), r"C:\Pictures\logo.png");
    // A path picked with Browse replaces the typed one, and typing goes on from it.
    super::properties::set_path(&app, "image", r"C:\Pictures\other.png".into());
    settle();
    click(AccessibleRole::TextInput, &row("image"));
    key(Key::End.into());
    (0..3).for_each(|_| key(Key::Backspace.into()));
    "svg".chars().for_each(|c| key(c.into()));
    click(AccessibleRole::Button, &t("settings.props.save"));
    assert_eq!(state.get_page(), super::model::SENSORS_PAGE as i32);
    let config = app.store.get();
    let props = &config
        .layout
        .all()
        .find(|s| s.id == logo)
        .expect("logo")
        .props;
    assert_eq!(props.get("image"), Some(&json!(r"C:\Pictures\other.svg")));
    let color = json!({ "mode": "custom", "color": "#123ABC" });
    assert_eq!(props.get("color"), Some(&color));
    let text = app
        .add_sensor(
            "text",
            crate::config::model::Zone::Left,
            0,
            &crate::sensors::Props::new(),
        )
        .expect("added");
    super::properties::open_form(&app, &text, true);
    render("settings-properties-text");
    // Cancel removes the new sensor again.
    state.invoke_props_cancel();
    assert!(app.store.get().layout.find(&text).is_none());
    assert_eq!(state.get_page(), super::model::SENSORS_PAGE as i32);
    // A drive field lists this computer's volumes (none on Linux: only the stored drive).
    let disk = app
        .add_sensor(
            "disk-meter",
            crate::config::model::Zone::Left,
            0,
            &crate::sensors::Props::new(),
        )
        .expect("added");
    super::properties::open_form(&app, &disk, true);
    super::properties::volumes_changed(app.platform.volumes());
    render("settings-properties-disk");
    state.invoke_props_cancel();
    assert!(app.store.get().layout.find(&disk).is_none());
    // The Shortcut's form: paths, parameters, a web address and how it runs (SPEC §9.17).
    let shortcut = app
        .add_sensor(
            "shortcut",
            crate::config::model::Zone::Left,
            0,
            &crate::sensors::Props::new(),
        )
        .expect("added");
    super::properties::open_form(&app, &shortcut, true);
    // Without a target or a web address it cannot be saved (SPEC §8.2).
    assert!(!state.get_props_valid());
    render_sized("settings-properties-shortcut", 900);
    state.invoke_prop_text("url".into(), "https://example.com".into());
    assert!(state.get_props_valid());
    state.invoke_props_cancel();
    assert!(app.store.get().layout.find(&shortcut).is_none());
    // Power: the modes this computer offers (none on Linux: only the stored one).
    let power = app
        .add_sensor(
            "power",
            crate::config::model::Zone::Left,
            0,
            &crate::sensors::Props::new(),
        )
        .expect("added");
    super::properties::open_form(&app, &power, true);
    render("settings-properties-power");
    state.invoke_props_cancel();
    // The Disk launcher: every drive, optical ones too (none on Linux: only the stored one).
    let disk = app
        .add_sensor(
            "disk",
            crate::config::model::Zone::Left,
            0,
            &crate::sensors::Props::new(),
        )
        .expect("added");
    super::properties::open_form(&app, &disk, true);
    // The drives come from the sampler, which this test does not run.
    super::properties::volumes_changed(app.platform.volumes());
    render("settings-properties-disk-launcher");
    state.invoke_props_cancel();
    // Battery: this computer's batteries (none on Linux: only the stored one, marked).
    let battery = app
        .add_sensor(
            "battery",
            crate::config::model::Zone::Left,
            0,
            &crate::sensors::Props::new(),
        )
        .expect("added");
    super::properties::open_form(&app, &battery, true);
    let status = app.platform.power_status().unwrap_or_default();
    super::properties::batteries_changed(crate::providers::battery_names(&status));
    render_sized("settings-properties-battery", 900);
    state.invoke_props_cancel();
    // Folder Menu: the folder, title, image, colour, opacity and hidden files (its list size
    // is not shown).
    let folder = app
        .add_sensor(
            "folder-menu",
            crate::config::model::Zone::Left,
            0,
            &crate::sensors::Props::new(),
        )
        .expect("added");
    super::properties::open_form(&app, &folder, true);
    // Its folder is required: the row says so and Save is disabled until one is chosen.
    assert!(!state.get_props_valid());
    render("settings-properties-folder-menu");
    super::properties::set_path(&app, "folder", r"C:\Users\Public".into());
    assert!(state.get_props_valid());
    render("settings-properties-folder-menu-set");
    state.invoke_props_cancel();
    // Start Menu: title, image, colour and opacity; no folder.
    let start_menu = app
        .add_sensor(
            "start-menu",
            crate::config::model::Zone::Left,
            0,
            &crate::sensors::Props::new(),
        )
        .expect("added");
    super::properties::open_form(&app, &start_menu, true);
    render("settings-properties-start-menu");
    state.invoke_props_cancel();
    // The first-run network graph: Automatic and this computer's connections.
    let network = app
        .store
        .get()
        .layout
        .all()
        .find(|s| s.kind == "network-graph")
        .map(|s| s.id.clone())
        .expect("the first-run network graph");
    super::properties::open_form(&app, &network, false);
    render_sized("settings-properties-network", 1000);
    state.invoke_props_cancel();

    // The picker (SPEC §6.6): next to the logo, with Text selected; then a search.
    let place = super::picker::Place::Next(logo.clone());
    super::picker::open_picker(&app, place);
    state.set_picker_selected(2);
    render("settings-picker");
    state.set_picker_search("zzz".into());
    state.invoke_picker_searched("zzz".into());
    assert_eq!(state.get_picker_selected(), -1);
    render("settings-picker-search");
    // Add opens the new sensor's properties, after the logo.
    state.invoke_picker_searched(SharedString::new());
    state.set_picker_selected(2);
    state.invoke_picker_add();
    assert_eq!(state.get_page(), super::properties::PAGE);
    let layout = app.store.get().layout;
    assert_eq!(layout.center[1].kind, "text");
    state.invoke_props_cancel();
    assert_eq!(app.store.get().layout.center.len(), 1);
    // From the keyboard: typing selects the first match, Up and Down in the search box move
    // the selection, and Enter adds it.
    super::picker::open_picker(&app, super::picker::Place::Next(logo.clone()));
    settle();
    key('e'.into());
    assert_eq!(state.get_picker_selected(), 0, "Image");
    // The types listed depend on the platform (e.g. the CPU dual graph needs per-processor
    // counters), so the last match is counted from the picker's own entries.
    let entries =
        super::picker::entries(crate::sensors::registry::registry(), app.platform.as_ref());
    let matches: Vec<usize> = super::picker::lines(&entries, "e")
        .into_iter()
        .filter_map(|line| match line {
            super::picker::Line::Entry(i) => Some(i),
            super::picker::Line::Heading(_) => None,
        })
        .collect();
    let last = *matches.last().expect("matches") as i32;
    (0..=matches.len()).for_each(|_| key(Key::DownArrow.into()));
    assert_eq!(state.get_picker_selected(), last, "the last, and it stays");
    // The list scrolled to show it.
    settle();
    render("settings-picker-end");
    let date = matches
        .iter()
        .position(|i| entries[*i].kind == "date")
        .expect("Date matches");
    (date + 1..matches.len()).for_each(|_| key(Key::UpArrow.into()));
    assert_eq!(state.get_picker_selected(), matches[date] as i32, "Date");
    key(Key::Return.into());
    assert_eq!(state.get_page(), super::properties::PAGE);
    assert_eq!(app.store.get().layout.center[1].kind, "date");
    state.invoke_props_cancel();
    assert_eq!(app.store.get().layout.center.len(), 1);

    // The welcome page (SPEC §11.5), as the first run opens it.
    let show_welcome = || {
        with_open_mut(|open| super::welcome::show(open, &app));
        settle();
    };
    let first_run_done = || app.store.get().meta.first_run_done;
    show_welcome();
    assert_eq!(state.get_page(), super::welcome::PAGE);
    render_sized("settings-welcome", 900);
    // A click on a theme's tile chooses it.
    window.set_size(PhysicalSize::new(760, 900));
    click(
        AccessibleRole::ListItem,
        &t("settings.appearance.theme.classic"),
    );
    assert_eq!(
        app.store.get().settings.theme,
        crate::config::model::ThemeId::Classic
    );
    refill();
    render_sized("settings-welcome-classic", 900);
    click(
        AccessibleRole::ListItem,
        &t("settings.appearance.theme.modern"),
    );
    assert_eq!(
        app.store.get().settings.theme,
        crate::config::model::ThemeId::Modern
    );
    // Leaving the page ends the first run.
    assert!(!first_run_done());
    nav(general);
    assert!(first_run_done());
    // So does *Get started*, which closes the window.
    app.store
        .update(|c| {
            c.meta.first_run_done = false;
            Ok(())
        })
        .expect("saved");
    show_welcome();
    state.invoke_welcome_done();
    assert!(first_run_done());
    assert!(!ui.window().is_visible());
    set_window(None);
    app.store.shutdown();
}
