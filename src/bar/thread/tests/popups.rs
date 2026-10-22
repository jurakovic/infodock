use super::*;

#[test]
fn a_popup_opens_next_to_its_sensor_and_takes_input() {
    let (mut logic, window, _, config) = bar(Settings::default());
    let logo = config.layout.center[0].id.clone();
    let (probe, log, reply) = new_probe();
    logic.open_popup(&window, &logo, Box::new(probe));
    // Centred above the logo (its centre at 596): 100 × 50 content, 4 px padding, the 7 px
    // pointer and a 6 px shadow around it.
    let (rect, dump) = window.popup.borrow().clone().expect("shown");
    assert_eq!(rect, Rect::new(536, 981, 656, 1058));
    assert!(dump.contains("fill rect 10,10 100×50 #010203"), "{dump}");
    assert_eq!(
        log.borrow()[0],
        "layout in 1178×1024",
        "the room above the bar"
    );
    // The logo shows its active state, in the text colour of the OS theme.
    let active = |logic: &mut BarLogic| {
        let mut canvas = RecordingCanvas::new(1.0);
        logic.paint(&mut canvas, SizeF::new(WIDTH, 28.0));
        canvas
            .dump()
            .lines()
            .any(|l| l.starts_with("fill rounded") && l.ends_with("/16%"))
    };
    assert!(active(&mut logic));

    // Input arrives from the content's corner; a redraw keeps the place.
    reply.set(PopupReply::Redraw);
    let draws = window.popup_draws.get();
    logic.on_event(
        &window,
        UiEvent::Popup(PopupInput::PointerMove(PointF::new(20.0, 30.0))),
    );
    assert_eq!(
        log.borrow().last().map(String::as_str),
        Some("PointerMove(PointF { x: 10.0, y: 20.0 })")
    );
    assert_eq!(window.popup_draws.get(), draws + 1);
    // Work arriving: polled.
    logic.on_event(&window, UiEvent::Wake);
    assert_eq!(log.borrow().last().map(String::as_str), Some("poll"));

    // Esc closes it, without reaching the popup.
    let input = |key| {
        UiEvent::Popup(PopupInput::Key {
            key,
            modifiers: Modifiers::default(),
        })
    };
    logic.on_event(&window, input(Key::Escape));
    assert!(window.popup.borrow().is_none());
    assert!(!log.borrow().iter().any(|l| l.contains("Escape")));
    assert!(!active(&mut logic), "no longer active");
}

#[test]
fn a_popup_closes_on_blur_on_request_and_on_a_press() {
    let (mut logic, window, host, config) = bar(Settings::default());
    let logo = config.layout.center[0].id.clone();
    let (probe, _, _) = new_probe();
    logic.open_popup(&window, &logo, Box::new(probe));
    logic.on_event(&window, UiEvent::PopupDeactivated);
    assert!(logic.popups.owner().is_none(), "closed on blur");

    // Asked to close, e.g. after opening a file; what it changed is saved.
    let (mut probe, _, reply) = new_probe();
    probe.props = Some(
        serde_json::json!({ "popupWidth": 300 })
            .as_object()
            .cloned()
            .unwrap_or_default(),
    );
    logic.open_popup(&window, &logo, Box::new(probe));
    reply.set(PopupReply::Close);
    logic.on_event(&window, UiEvent::Popup(PopupInput::Char('a')));
    assert!(window.popup.borrow().is_none());
    assert!(
        host.calls()
            .contains(&format!(r#"update {logo} {{"popupWidth":300}}"#)),
        "{:?}",
        host.calls()
    );

    // A wake-up it asked for.
    let (mut probe, log, _) = new_probe();
    probe.wake = Some(Instant::now());
    logic.open_popup(&window, &logo, Box::new(probe));
    assert_eq!(window.popup_timer.get(), Some(Duration::ZERO));
    logic.on_event(&window, UiEvent::Timer(TimerId::Popup));
    assert_eq!(log.borrow().last().map(String::as_str), Some("wake"));
    assert!(window.popup.borrow().is_none(), "it asked to close");
    assert_eq!(window.popup_timer.get(), None);
}

#[test]
fn the_press_that_closes_a_popup_does_not_reopen_it() {
    let (mut logic, window, _host, mut config) = bar(Settings::default());
    // A Power sensor (which asks first) in the logo's place shows whether a click arrives.
    let logo = &mut config.layout.center[0];
    logo.kind = "power".into();
    let id = logo.id.clone();
    logic.apply_config(&window, config);
    logic.sync_sensors(&window);
    logic.paint(&mut RecordingCanvas::new(1.0), SizeF::new(WIDTH, 28.0));
    window.decline.set(true);
    let (probe, _, _) = new_probe();
    logic.open_popup(&window, &id, Box::new(probe));
    // The press closes the popup; the click it ends is ignored.
    press(&mut logic, &window, LOGO, false);
    assert!(window.popup.borrow().is_none());
    release(&mut logic, &window, LOGO);
    assert!(window.confirmed.borrow().is_empty(), "no click");
    // No hints show while a popup is open.
    let (probe, _, _) = new_probe();
    logic.open_popup(&window, &id, Box::new(probe));
    hover(&mut logic, &window, PointF::new(LOGO.x + 1.0, 14.0));
    assert_eq!(window.hint_timer.get(), None);
}

#[test]
fn after_a_popup_no_reopening_hint_and_hiding_the_bar_closes_it() {
    let dir = crate::sensors::common::folder_browser::tests::sample_folder();
    let (mut logic, window, _host, mut config) = bar(Settings::default());
    let logo = &mut config.layout.center[0];
    logo.kind = "folder-menu".into();
    logo.props = serde_json::json!({ "folder": dir.path().to_string_lossy() })
        .as_object()
        .cloned()
        .unwrap_or_default();
    logic.apply_config(&window, config);
    logic.sync_sensors(&window);
    logic.paint(&mut RecordingCanvas::new(1.0), SizeF::new(WIDTH, 28.0));
    press(&mut logic, &window, LOGO, false);
    release(&mut logic, &window, LOGO);
    assert!(window.popup.borrow().is_some(), "opened");
    // A slow click on the sensor closes its popup and does not open it again.
    press(&mut logic, &window, LOGO, false);
    assert!(window.popup.borrow().is_none(), "closed by the press");
    std::thread::sleep(super::popup::GUARD + Duration::from_millis(50));
    release(&mut logic, &window, LOGO);
    assert!(window.popup.borrow().is_none(), "not opened again");
    // Opened again; moving over the bar meanwhile and closing it with Esc: the pointer is
    // still on the sensor, so its hint stays away.
    logic.on_event(&window, UiEvent::PointerMove(LOGO));
    press(&mut logic, &window, LOGO, false);
    release(&mut logic, &window, LOGO);
    assert!(window.popup.borrow().is_some());
    window.hint_timer.set(None);
    logic.on_event(
        &window,
        UiEvent::PointerMove(PointF::new(LOGO.x + 1.0, 14.0)),
    );
    logic.on_event(
        &window,
        UiEvent::Popup(PopupInput::Key {
            key: Key::Escape,
            modifiers: Modifiers::default(),
        }),
    );
    logic.on_event(&window, UiEvent::PointerMove(LOGO));
    assert_eq!(window.hint_timer.get(), None, "no hint on its way");
    // Hiding the bar (tray, hotkey) closes an open popup.
    std::thread::sleep(super::popup::GUARD);
    press(&mut logic, &window, LOGO, false);
    release(&mut logic, &window, LOGO);
    assert!(window.popup.borrow().is_some());
    logic.hidden(&window);
    assert!(window.popup.borrow().is_none());
}

#[test]
fn a_folder_menu_opens_its_list_closes_on_esc_and_resizes() {
    let dir = crate::sensors::common::folder_browser::tests::sample_folder();
    let (mut logic, window, host, mut config) = bar(Settings::default());
    // A Folder Menu in the logo's place.
    let logo = &mut config.layout.center[0];
    logo.kind = "folder-menu".into();
    logo.props = serde_json::json!({ "folder": dir.path().to_string_lossy() })
        .as_object()
        .cloned()
        .unwrap_or_default();
    logic.apply_config(&window, config);
    logic.sync_sensors(&window);
    logic.paint(&mut RecordingCanvas::new(1.0), SizeF::new(WIDTH, 28.0));
    press(&mut logic, &window, LOGO, false);
    release(&mut logic, &window, LOGO);
    let (rect, dump) = window.popup.borrow().clone().expect("the list is shown");
    assert!(dump.contains("\"report 10.pdf\""), "{dump}");
    // 240 px wide, its left edge (inside the shadow) at the sensor's.
    assert_eq!(rect.width(), 240 + 2 * 4 + 2 * 6);
    let key = |key| {
        UiEvent::Popup(PopupInput::Key {
            key,
            modifiers: Modifiers::default(),
        })
    };
    logic.on_event(&window, key(Key::Escape));
    assert!(window.popup.borrow().is_none());

    // Dragging its free right edge: the pointer is captured and the size saved. (Clicks
    // on the sensor wait out the guard after Esc closed it.)
    std::thread::sleep(super::popup::GUARD);
    press(&mut logic, &window, LOGO, false);
    release(&mut logic, &window, LOGO);
    assert!(window.popup.borrow().is_some());
    // The content starts 10 px into the window (shadow and padding).
    let at = |x: f32| PointF::new(10.0 + x, 110.0);
    let input = |logic: &mut BarLogic, input| logic.on_event(&window, UiEvent::Popup(input));
    input(&mut logic, PopupInput::PointerMove(at(239.0)));
    assert_eq!(
        window.popup_cursor.get(),
        crate::platform::PopupCursor::ResizeWidth
    );
    input(
        &mut logic,
        PopupInput::PointerDown {
            button: PointerButton::Left,
            at: at(239.0),
            modifiers: Modifiers::default(),
        },
    );
    assert!(window.popup_captured.get());
    input(&mut logic, PopupInput::PointerMove(at(299.0)));
    input(
        &mut logic,
        PopupInput::PointerUp {
            button: PointerButton::Left,
            at: at(299.0),
            modifiers: Modifiers::default(),
        },
    );
    assert!(!window.popup_captured.get());
    let (rect, _) = window.popup.borrow().clone().expect("still open");
    assert_eq!(rect.width(), 300 + 2 * 4 + 2 * 6, "60 px wider");
    assert!(
        host.calls()
            .iter()
            .any(|c| c.ends_with(r#"{"popupHeight":320,"popupWidth":300}"#)),
        "{:?}",
        host.calls()
    );
}
