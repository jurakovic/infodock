use super::*;

/// The x where the focus ring starts.
fn ring_left(logic: &mut BarLogic) -> Option<f32> {
    let ring = ring(logic)?;
    let rect = ring.split_whitespace().nth(2)?;
    rect.split(',').next()?.parse().ok()
}

/// The x where the ring around sensor `id` starts.
fn ring_left_of(logic: &BarLogic, id: &str) -> f32 {
    let (x, _) = logic.sensors.place_of(id).expect("placed");
    (x - (logic.theme.gap / 2.0).min(3.0)).max(1.5)
}

#[test]
fn the_hotkey_focuses_the_bar_and_tab_moves_the_ring_in_visual_order() {
    let (mut logic, window, _, config) = bar(Settings::default());
    assert_eq!(ring(&mut logic), None, "no ring before the hotkey");
    // Keys reach only a focused bar.
    key(&mut logic, &window, Key::Tab, false);
    assert_eq!(ring(&mut logic), None);
    logic.focus_bar(&window);
    assert_eq!(window.focus_taken.get(), 1);
    assert_eq!(ring_left(&mut logic), Some(1.5), "around the bar itself");
    // Left, centre, right, each zone in its order; then around again.
    let order: Vec<String> = [
        &config.layout.left,
        &config.layout.center,
        &config.layout.right,
    ]
    .into_iter()
    .flatten()
    .map(|s| s.id.clone())
    .collect();
    assert_eq!(logic.sensors.focus_order(), order);
    for id in order.iter().chain(&order[..1]) {
        key(&mut logic, &window, Key::Tab, false);
        assert_eq!(logic.focused_sensor().as_ref(), Some(id));
        let expected = ring_left_of(&logic, id);
        assert_eq!(ring_left(&mut logic), Some(expected));
    }
    // Shift+Tab before the first wraps to the last; Home and End; the arrows.
    key(&mut logic, &window, Key::Tab, true);
    assert_eq!(logic.focused_sensor().as_ref(), order.last());
    key(&mut logic, &window, Key::Home, false);
    assert_eq!(logic.focused_sensor().as_ref(), order.first());
    key(&mut logic, &window, Key::Right, false);
    assert_eq!(logic.focused_sensor().as_ref(), order.get(1));
    key(&mut logic, &window, Key::Left, false);
    key(&mut logic, &window, Key::End, false);
    assert_eq!(logic.focused_sensor().as_ref(), order.last());
    // The hotkey again keeps the ring where it is.
    logic.focus_bar(&window);
    assert_eq!(logic.focused_sensor().as_ref(), order.last());
    // A window that cannot take the focus: no keyboard on the bar.
    let (mut logic, window, _, _) = bar(Settings::default());
    window.no_focus.set(true);
    logic.focus_bar(&window);
    assert_eq!(ring(&mut logic), None);
}

#[test]
fn screen_readers_get_the_sensors_in_focus_order() {
    let (mut logic, window, _, config) = bar(Settings::default());
    let bar = logic.accessible();
    let ids: Vec<&str> = bar.items.iter().map(|i| i.id.as_str()).collect();
    let order = logic.sensors.focus_order();
    assert_eq!(ids, order.iter().map(String::as_str).collect::<Vec<_>>());
    let logo = &bar.items[3];
    assert_eq!(logo.id, config.layout.center[0].id);
    assert_eq!(logo.name, "Image");
    assert!(logo.value.starts_with("Infodock "), "{}", logo.value);
    let (x, width) = logic.sensors.place_of(&logo.id).expect("placed");
    assert_eq!(logo.rect, RectF::new(x, 0.0, x + width, 28.0));
    assert_eq!(bar.items[0].name, "CPU Graph");
    assert_eq!((bar.focused, bar.focus), (false, None));
    // The keyboard: the bar, then the first sensor.
    logic.focus_bar(&window);
    assert_eq!(
        (logic.accessible().focused, logic.accessible().focus),
        (true, None)
    );
    key(&mut logic, &window, Key::Tab, false);
    assert_eq!(
        logic.accessible().focused_item().map(|i| i.name.as_str()),
        Some("CPU Graph")
    );
    // A Spacer is not listed.
    let mut with_spacer = config.clone();
    with_spacer.layout.left.insert(
        0,
        SensorInstance {
            id: "s_spacer".into(),
            kind: "spacer".into(),
            props: crate::sensors::Props::new(),
        },
    );
    logic.apply_config(&window, with_spacer);
    logic.sync_sensors(&window);
    logic.paint(&mut RecordingCanvas::new(1.0), SizeF::new(WIDTH, 28.0));
    assert_eq!(logic.accessible().items.len(), 6);
    assert_eq!(logic.accessible().focus, Some(0), "still on the CPU Graph");
}

#[test]
fn screen_readers_click_expand_collapse_and_focus_sensors() {
    let (mut logic, window, _, mut config) = bar(Settings {
        locked: true,
        clicks_when_locked: ClicksWhenLocked::AltOnly,
        ..Settings::default()
    });
    // Invoke clicks, whatever the Lock rules say for the pointer.
    with_power_sensor(&mut logic, &window);
    window.decline.set(true);
    let power = logic.layout.center[0].id.clone();
    logic.on_event(
        &window,
        UiEvent::Accessible(AccessibleAction::Invoke(power.clone())),
    );
    assert_eq!(window.confirmed.borrow().len(), 1);
    assert_eq!(
        logic.accessible().items[3].popup,
        None,
        "no popup to expand"
    );
    // An unknown sensor: nothing.
    logic.on_event(
        &window,
        UiEvent::Accessible(AccessibleAction::Invoke("gone".into())),
    );
    assert_eq!(window.confirmed.borrow().len(), 1);

    // A Folder Menu in the logo's place expands and collapses.
    let dir = crate::sensors::common::folder_browser::tests::sample_folder();
    let logo = &mut config.layout.center[0];
    logo.kind = "folder-menu".into();
    logo.props = serde_json::json!({ "folder": dir.path().to_string_lossy() })
        .as_object()
        .cloned()
        .unwrap_or_default();
    let id = logo.id.clone();
    logic.apply_config(&window, config);
    logic.sync_sensors(&window);
    logic.paint(&mut RecordingCanvas::new(1.0), SizeF::new(WIDTH, 28.0));
    assert_eq!(logic.accessible().items[3].popup, Some(false));
    let act = |logic: &mut BarLogic, action| logic.on_event(&window, UiEvent::Accessible(action));
    act(&mut logic, AccessibleAction::Expand(id.clone()));
    assert!(window.popup.borrow().is_some());
    assert_eq!(logic.accessible().items[3].popup, Some(true));
    // The popup window is named by the folder it shows.
    let folder = dir
        .path()
        .file_name()
        .map(|n| n.to_string_lossy().into_owned());
    assert_eq!(Some(window.popup_name.borrow().clone()), folder);
    // Expanding an open one keeps it open; collapsing closes it.
    let draws = window.popup_draws.get();
    act(&mut logic, AccessibleAction::Expand(id.clone()));
    assert_eq!(window.popup_draws.get(), draws, "not opened again");
    act(&mut logic, AccessibleAction::Collapse(id.clone()));
    assert!(window.popup.borrow().is_none());
    assert_eq!(logic.accessible().items[3].popup, Some(false));
    act(&mut logic, AccessibleAction::Collapse(id.clone()));

    // Focus: the bar takes the keyboard with the ring on that sensor.
    act(&mut logic, AccessibleAction::Focus(id.clone()));
    assert_eq!(window.focus_taken.get(), 1);
    assert_eq!(logic.focused_sensor(), Some(id.clone()));
    assert_eq!(
        logic.accessible().focused_item().map(|i| i.id.clone()),
        Some(id)
    );
}

#[test]
fn a_spacer_is_passed_over() {
    let (mut logic, window, _, mut config) = bar(Settings::default());
    let spacer = SensorInstance {
        id: "s_spacer".into(),
        kind: "spacer".into(),
        props: crate::sensors::Props::new(),
    };
    config.layout.center.push(spacer);
    logic.apply_config(&window, config);
    logic.sync_sensors(&window);
    assert!(!logic.sensors.focus_order().contains(&"s_spacer".to_owned()));
    assert_eq!(logic.sensors.focus_order().len(), 6);
}

#[test]
fn the_focused_sensors_hint_shows_after_the_delay() {
    let (mut logic, window, _, _) = bar(Settings::default());
    logic.focus_bar(&window);
    key(&mut logic, &window, Key::Home, false);
    key(&mut logic, &window, Key::End, false);
    assert_eq!(window.hint_timer.get(), Some(Duration::from_millis(600)));
    hint_timer(&mut logic, &window);
    assert!(window.hint.borrow().is_some(), "the Time's hint");
    // Esc: the hint goes with the ring.
    key(&mut logic, &window, Key::Escape, false);
    assert!(window.hint.borrow().is_none());
}

#[test]
fn enter_and_space_click_the_focused_sensor() {
    let (mut logic, window, _, config) = bar(Settings {
        locked: true,
        clicks_when_locked: ClicksWhenLocked::AltOnly,
        ..Settings::default()
    });
    with_power_sensor(&mut logic, &window);
    window.decline.set(true);
    logic.focus_bar(&window);
    // Nothing focused: nothing to click.
    key(&mut logic, &window, Key::Enter, false);
    assert!(window.confirmed.borrow().is_empty());
    logic.keys.as_mut().expect("keys").sensor = Some(config.layout.center[0].id.clone());
    // A deliberate key: even a locked Alt-only bar clicks.
    key(&mut logic, &window, Key::Enter, false);
    key(&mut logic, &window, Key::Space, false);
    assert_eq!(window.confirmed.borrow().len(), 2);
}

#[test]
fn shift_f10_and_the_menu_key_open_the_menu_beside_the_focus() {
    let (mut logic, window, host, config) = bar(Settings::default());
    logic.focus_bar(&window);
    // No sensor focused: the bar menu beside the bar, new sensors at the end of the left zone.
    window.choose.set(Some(MenuCommand::NewSensor.id()));
    key(&mut logic, &window, Key::F10, true);
    assert_eq!(host.calls(), ["open new?zone=left&index=3"]);
    assert_eq!(
        window.menus_near.borrow().last(),
        Some(&Some(Rect::new(0, 1052, 1200, 1080)))
    );
    // F10 alone opens nothing.
    key(&mut logic, &window, Key::F10, false);
    assert_eq!(window.menus.borrow().len(), 1);
    // The logo focused: its menu, beside it.
    let logo = config.layout.center[0].id.clone();
    logic.keys.as_mut().expect("keys").sensor = Some(logo.clone());
    window.choose.set(Some(MenuCommand::Properties.id()));
    key(&mut logic, &window, Key::Menu, false);
    assert_eq!(host.calls().last(), Some(&format!("open sensor/{logo}")));
    let (x, width) = logic.sensors.place_of(&logo).expect("placed");
    let near = Rect::new(x as i32, 1052, (x + width) as i32, 1080);
    assert_eq!(window.menus_near.borrow().last(), Some(&Some(near)));
    // The pointer's menus stay at the pointer.
    right_click(&mut logic, &window, LOGO, MenuCommand::Settings);
    assert_eq!(window.menus_near.borrow().last(), Some(&None));
}

#[test]
fn esc_or_a_click_elsewhere_ends_the_keyboard_on_the_bar() {
    let (mut logic, window, _, _) = bar(Settings::default());
    logic.focus_bar(&window);
    key(&mut logic, &window, Key::Tab, false);
    key(&mut logic, &window, Key::Escape, false);
    assert_eq!(
        window.focus_restored.get(),
        1,
        "back to the window that had it"
    );
    assert_eq!(ring(&mut logic), None);
    key(&mut logic, &window, Key::Tab, false);
    assert_eq!(ring(&mut logic), None, "keys no longer reach it");

    // Another app took the focus: the ring goes, nothing is given back.
    logic.focus_bar(&window);
    logic.on_event(&window, UiEvent::FocusLost { within_app: false });
    assert_eq!(ring(&mut logic), None);
    logic.on_event(&window, UiEvent::FocusGained);
    assert_eq!(ring(&mut logic), None, "a menu's activation brings no ring");
    assert_eq!(window.focus_restored.get(), 1);

    // A confirmation of the bar's took it: the ring waits and comes back with the focus.
    logic.focus_bar(&window);
    key(&mut logic, &window, Key::Tab, false);
    logic.on_event(&window, UiEvent::FocusLost { within_app: true });
    assert_eq!(ring(&mut logic), None);
    logic.on_event(&window, UiEvent::FocusGained);
    assert!(ring(&mut logic).is_some());
    assert!(logic.focused_sensor().is_some(), "on the same sensor");
    // Lost to its own window, then a press on the bar: what was left ends.
    logic.on_event(&window, UiEvent::FocusLost { within_app: true });
    press(&mut logic, &window, PointF::new(300.0, 14.0), false);
    logic.on_event(&window, UiEvent::FocusGained);
    assert_eq!(ring(&mut logic), None);

    // Hiding the bar gives the focus back.
    logic.focus_bar(&window);
    logic.hidden(&window);
    assert_eq!(window.focus_restored.get(), 2);
    assert_eq!(ring(&mut logic), None);
}

#[test]
fn a_removed_sensors_focus_goes_to_the_one_in_its_place() {
    let (mut logic, window, _, mut config) = bar(Settings::default());
    logic.focus_bar(&window);
    let logo = config.layout.center[0].id.clone();
    let date = config.layout.right[0].id.clone();
    logic.keys.as_mut().expect("keys").sensor = Some(logo);
    config.layout.center.clear();
    logic.apply_config(&window, config.clone());
    logic.sync_sensors(&window);
    assert_eq!(logic.focused_sensor(), Some(date));
    // The bar emptied: the ring goes around the bar itself.
    config.layout = Layout::default();
    logic.apply_config(&window, config);
    logic.sync_sensors(&window);
    assert_eq!(logic.focused_sensor(), None);
    assert_eq!(ring_left(&mut logic), Some(1.5));
    key(&mut logic, &window, Key::Tab, false);
    assert_eq!(logic.focused_sensor(), None, "nothing to move to");
}

#[test]
fn a_popup_opened_from_the_keyboard_gives_the_focus_back_to_the_bar() {
    let (mut logic, window, _, config) = bar(Settings::default());
    let logo = config.layout.center[0].id.clone();
    logic.focus_bar(&window);
    logic.keys.as_mut().expect("keys").sensor = Some(logo.clone());
    let (probe, _, _) = new_probe();
    logic.open_popup(&window, &logo, Box::new(probe));
    // The popup takes the focus, as on a click.
    logic.on_event(&window, UiEvent::FocusLost { within_app: true });
    let esc = UiEvent::Popup(PopupInput::Key {
        key: Key::Escape,
        modifiers: Modifiers::default(),
    });
    logic.on_event(&window, esc);
    assert!(window.popup.borrow().is_none());
    assert_eq!(window.focus_taken.get(), 2, "the bar took it back");
    assert_eq!(logic.focused_sensor(), Some(logo.clone()));
    assert!(ring(&mut logic).is_some());

    // Closed by itself (an item chosen): the same.
    let (probe, _, reply) = new_probe();
    logic.open_popup(&window, &logo, Box::new(probe));
    logic.on_event(&window, UiEvent::FocusLost { within_app: true });
    reply.set(PopupReply::Close);
    logic.on_event(
        &window,
        UiEvent::Popup(PopupInput::PointerMove(PointF::new(20.0, 20.0))),
    );
    assert_eq!(window.focus_taken.get(), 3);

    // Closed by a click in another app: the keyboard on the bar ends with it.
    let (probe, _, _) = new_probe();
    logic.open_popup(&window, &logo, Box::new(probe));
    logic.on_event(&window, UiEvent::FocusLost { within_app: true });
    logic.on_event(&window, UiEvent::PopupDeactivated);
    assert_eq!(window.focus_taken.get(), 3);
    assert_eq!(ring(&mut logic), None);
}
