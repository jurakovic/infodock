use super::*;

/// Whether the last frame drew a dimmed sensor.
fn dimmed(logic: &mut BarLogic) -> bool {
    let mut canvas = RecordingCanvas::new(1.0);
    logic.paint(&mut canvas, SizeF::new(WIDTH, 28.0));
    canvas.dump().contains("opacity 0.6")
}

fn drag_to(logic: &mut BarLogic, window: &FakeWindow, from: PointF, to: PointF) -> bool {
    press(logic, window, from, false);
    logic.on_event(
        window,
        UiEvent::PointerMove(PointF::new(from.x + 6.0, from.y)),
    );
    logic.on_event(window, UiEvent::PointerMove(to));
    let mut canvas = RecordingCanvas::new(1.0);
    logic.paint(&mut canvas, SizeF::new(WIDTH, 28.0));
    let placer = canvas.dump().lines().any(|l| l.ends_with("#0078D4"));
    release(logic, window, to);
    placer
}

#[test]
fn a_floating_bar_moves_by_its_background_while_unlocked() {
    let mut settings = Settings::default();
    settings.bar.mode = BarMode::Floating;
    let (mut logic, window, host, _) = bar(settings.clone());
    let start = Rect::new(100, 500, 1300, 528);
    window.position.set(Some(start));
    let empty = PointF::new(300.0, 14.0);
    press(&mut logic, &window, empty, false);
    assert!(window.captured.get());
    // Within the threshold nothing moves.
    logic.on_event(&window, UiEvent::PointerMove(PointF::new(302.0, 14.0)));
    assert_eq!(window.position.get(), Some(start));
    logic.on_event(&window, UiEvent::PointerMove(PointF::new(320.0, 4.0)));
    assert_eq!(window.position.get(), Some(Rect::new(120, 490, 1320, 518)));
    // The window followed, so the pointer is back at the pressed point inside it.
    logic.on_event(&window, UiEvent::PointerMove(PointF::new(305.0, 14.0)));
    assert_eq!(window.position.get(), Some(Rect::new(125, 490, 1325, 518)));
    release(&mut logic, &window, PointF::new(300.0, 14.0));
    assert!(!window.captured.get());
    let stored = host
        .settings
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .bar
        .floating;
    assert_eq!(
        stored,
        Some(FloatingRect {
            x: 125,
            y: 490,
            width: DEFAULT_FLOATING_WIDTH
        })
    );

    // Dragged into the upper half, its shadow turns below it at once (SPEC §5.1).
    settings.bar.shadow = true;
    let (mut logic, window, _, _) = bar(settings.clone());
    window.position.set(Some(start));
    let size = SizeF::new(WIDTH, 28.0);
    logic.paint(&mut RecordingCanvas::new(1.0), size);
    assert!(logic.band().expect("a shadow").above);
    press(&mut logic, &window, empty, false);
    logic.on_event(&window, UiEvent::PointerMove(PointF::new(300.0, -300.0)));
    logic.paint(&mut RecordingCanvas::new(1.0), size);
    assert!(!logic.band().expect("a shadow").above);
    release(&mut logic, &window, PointF::new(300.0, -300.0));

    // Locked, or docked: the background does not move the bar.
    settings.locked = true;
    let (mut logic, window, _, _) = bar(settings);
    window.position.set(Some(start));
    press(&mut logic, &window, empty, false);
    logic.on_event(&window, UiEvent::PointerMove(PointF::new(350.0, 14.0)));
    assert_eq!(window.position.get(), Some(start));
    let (mut logic, window, _, _) = bar(Settings::default());
    press(&mut logic, &window, empty, false);
    assert!(!window.captured.get());
}

#[test]
fn a_floating_bar_resizes_by_its_ends_while_unlocked() {
    let mut settings = Settings::default();
    settings.bar.mode = BarMode::Floating;
    let place = |logic: &mut BarLogic, window: &FakeWindow, rect: Rect| {
        window.position.set(Some(rect));
        logic.on_event(
            window,
            UiEvent::Placed(BarGeometry {
                rect,
                edge: crate::config::model::Edge::Bottom,
                monitor: crate::platform::MonitorInfo::new(
                    r"\\.\DISPLAY1".into(),
                    "Test".into(),
                    Rect::new(0, 0, 1200, 1080),
                    Rect::new(0, 0, 1200, 1052),
                    1.0,
                    true,
                ),
                scale: 1.0,
            }),
        );
    };
    let stored = |host: &FakeHost| {
        host.settings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .bar
            .floating
    };
    let (mut logic, window, host, _) = bar(settings.clone());
    place(&mut logic, &window, Rect::new(200, 1000, 1000, 1028));
    // The resize cursor within 6 px of either end.
    for (x, cursor) in [
        (3.0, PopupCursor::ResizeWidth),
        (400.0, PopupCursor::Arrow),
        (797.0, PopupCursor::ResizeWidth),
    ] {
        hover(&mut logic, &window, PointF::new(x, 14.0));
        assert_eq!(window.cursor.get(), cursor, "at {x}");
    }
    logic.on_event(&window, UiEvent::PointerLeave);
    assert_eq!(window.cursor.get(), PopupCursor::Arrow);

    // The right end, pressed 3 px inside it, follows the pointer at once.
    press(&mut logic, &window, PointF::new(797.0, 14.0), false);
    assert!(window.captured.get());
    logic.on_event(&window, UiEvent::PointerMove(PointF::new(897.0, 14.0)));
    assert_eq!(
        window.position.get(),
        Some(Rect::new(200, 1000, 1100, 1028))
    );
    assert_eq!(window.cursor.get(), PopupCursor::ResizeWidth);
    // Not past the work area.
    logic.on_event(&window, UiEvent::PointerMove(PointF::new(2000.0, 14.0)));
    assert_eq!(
        window.position.get(),
        Some(Rect::new(200, 1000, 1200, 1028))
    );
    release(&mut logic, &window, PointF::new(2000.0, 14.0));
    assert!(!window.captured.get());
    assert_eq!(
        stored(&host),
        Some(FloatingRect {
            x: 200,
            y: 1000,
            width: 1000
        })
    );

    // The left end: the window moves with it, so each move is read from where it is.
    press(&mut logic, &window, PointF::new(2.0, 14.0), false);
    logic.on_event(&window, UiEvent::PointerMove(PointF::new(-98.0, 14.0)));
    assert_eq!(
        window.position.get(),
        Some(Rect::new(100, 1000, 1200, 1028))
    );
    logic.on_event(&window, UiEvent::PointerMove(PointF::new(-48.0, 14.0)));
    assert_eq!(window.position.get(), Some(Rect::new(50, 1000, 1200, 1028)));
    // Not narrower than 200 px.
    logic.on_event(&window, UiEvent::PointerMove(PointF::new(1100.0, 14.0)));
    assert_eq!(
        window.position.get(),
        Some(Rect::new(1000, 1000, 1200, 1028))
    );
    // A lost capture stores it too.
    logic.on_event(&window, UiEvent::CaptureLost);
    assert_eq!(
        stored(&host),
        Some(FloatingRect {
            x: 1000,
            y: 1000,
            width: 200
        })
    );

    // A press without a move stores nothing.
    let (mut logic, window, host, _) = bar(settings.clone());
    place(&mut logic, &window, Rect::new(200, 1000, 1000, 1028));
    press(&mut logic, &window, PointF::new(797.0, 14.0), false);
    release(&mut logic, &window, PointF::new(797.0, 14.0));
    assert_eq!(stored(&host), None);

    // Locked, or docked: the ends do nothing.
    settings.locked = true;
    let (mut logic, window, _, _) = bar(settings);
    place(&mut logic, &window, Rect::new(200, 1000, 1000, 1028));
    hover(&mut logic, &window, PointF::new(3.0, 14.0));
    assert_eq!(window.cursor.get(), PopupCursor::Arrow);
    press(&mut logic, &window, PointF::new(3.0, 14.0), false);
    logic.on_event(&window, UiEvent::PointerMove(PointF::new(-97.0, 14.0)));
    assert_eq!(
        window.position.get(),
        Some(Rect::new(200, 1000, 1000, 1028))
    );
    let (mut logic, window, _, _) = bar(Settings::default());
    hover(&mut logic, &window, PointF::new(3.0, 14.0));
    assert_eq!(window.cursor.get(), PopupCursor::Arrow);
    press(&mut logic, &window, PointF::new(3.0, 14.0), false);
    assert!(!window.captured.get());
}

#[test]
fn a_press_dims_the_sensor_until_release() {
    let (mut logic, window, _, _) = bar(Settings::default());
    press(&mut logic, &window, LOGO, false);
    assert!(window.captured.get());
    assert!(dimmed(&mut logic));
    release(&mut logic, &window, LOGO);
    assert!(!window.captured.get());
    assert!(!dimmed(&mut logic));

    // Moving starts a drag: the sensor stays dimmed until it is dropped.
    press(&mut logic, &window, LOGO, false);
    logic.on_event(
        &window,
        UiEvent::PointerMove(PointF::new(LOGO.x + 5.0, 14.0)),
    );
    assert!(dimmed(&mut logic));
    release(&mut logic, &window, LOGO);
    assert!(!dimmed(&mut logic));
}

#[test]
fn a_click_that_asks_first_waits_for_the_answer() {
    let (mut logic, window, _host, _) = bar(Settings::default());
    let power = with_power_sensor(&mut logic, &window);
    let actions = || power.log();
    // Declined: nothing happens.
    window.decline.set(true);
    press(&mut logic, &window, LOGO, false);
    release(&mut logic, &window, LOGO);
    assert_eq!(
        *window.confirmed.borrow(),
        ["Do you really want to shut down this computer?"]
    );
    assert!(actions().is_empty());
    // Confirmed: the action is taken.
    window.decline.set(false);
    press(&mut logic, &window, LOGO, false);
    release(&mut logic, &window, LOGO);
    assert_eq!(actions(), ["ShutDown force=false"]);
}

#[test]
fn locked_alt_only_bar_ignores_presses_without_alt() {
    let settings = Settings {
        locked: true,
        clicks_when_locked: ClicksWhenLocked::AltOnly,
        ..Settings::default()
    };
    let (mut logic, window, _, _) = bar(settings);
    press(&mut logic, &window, LOGO, false);
    assert!(!window.captured.get());
    assert!(!dimmed(&mut logic));
    press(&mut logic, &window, LOGO, true);
    assert!(dimmed(&mut logic));
}

#[test]
fn dragging_moves_a_sensor() {
    let (mut logic, window, host, config) = bar(Settings::default());
    let logo = config.layout.center[0].id.clone();
    // Drop the logo on the empty right part of the left third: after the left zone's last.
    assert!(
        drag_to(&mut logic, &window, LOGO, PointF::new(350.0, 14.0)),
        "placer drawn"
    );
    assert_eq!(host.calls(), [format!("move {logo} left 3")]);
    assert!(!window.captured.get());

    // The first placeholder onto the right half of the third: after it, removal counted.
    let cpu = config.layout.left[0].id.clone();
    let third_x = {
        let spans = logic.sensors.spans();
        spans[0][2].0 + spans[0][2].1 - 2.0
    };
    drag_to(
        &mut logic,
        &window,
        PointF::new(10.0, 14.0),
        PointF::new(third_x, 14.0),
    );
    assert_eq!(host.calls()[1], format!("move {cpu} left 2"));
}

#[test]
fn dropping_in_place_or_pressing_esc_moves_nothing() {
    let (mut logic, window, host, _) = bar(Settings::default());
    drag_to(&mut logic, &window, LOGO, PointF::new(LOGO.x + 8.0, 14.0));
    assert!(host.calls().is_empty());

    press(&mut logic, &window, LOGO, false);
    logic.on_event(&window, UiEvent::PointerMove(PointF::new(350.0, 14.0)));
    window.escape.set(true);
    logic.on_event(&window, UiEvent::Timer(TimerId::Drag));
    assert!(!window.captured.get(), "cancelled");
    release(&mut logic, &window, PointF::new(350.0, 14.0));
    assert!(host.calls().is_empty());
}

#[test]
fn a_locked_bar_does_not_drag() {
    let settings = Settings {
        locked: true,
        ..Settings::default()
    };
    let (mut logic, window, host, _) = bar(settings);
    assert!(!drag_to(
        &mut logic,
        &window,
        LOGO,
        PointF::new(350.0, 14.0)
    ));
    assert!(host.calls().is_empty());
}
