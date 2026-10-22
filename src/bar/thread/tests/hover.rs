use super::*;

#[test]
fn hint_shows_after_the_delay_above_the_sensor() {
    let (mut logic, window, _, _) = bar(Settings::default());
    hover(&mut logic, &window, LOGO);
    assert_eq!(
        window.hint_timer.get(),
        Some(Duration::from_millis(600)),
        "the show delay runs"
    );
    assert!(window.hint.borrow().is_none());
    hint_timer(&mut logic, &window);
    let hint = window.hint.borrow().clone().expect("shown");
    // The logo's popup text, above the bar, its pointer under the bubble.
    assert!(
        hint.1.contains(&format!("\"Infodock {}\"", crate::VERSION)),
        "{}",
        hint.1
    );
    assert!(hint.0.bottom <= 1052 + 6, "above the bar: {:?}", hint.0);
    // Screen readers read its text as the window's title.
    assert_eq!(
        *window.hint_name.borrow(),
        format!("Infodock {}", crate::VERSION)
    );
    assert!(
        hint.0.left < 600 && hint.0.right > 600,
        "centred on the logo: {:?}",
        hint.0
    );

    // Leaving: hidden after the hide delay.
    logic.on_event(&window, UiEvent::PointerLeave);
    assert_eq!(window.hint_timer.get(), Some(Duration::from_millis(100)));
    assert!(window.hint.borrow().is_some());
    hint_timer(&mut logic, &window);
    assert!(window.hint.borrow().is_none());
}

#[test]
fn hint_closes_on_a_press_and_stays_closed() {
    let (mut logic, window, _, _) = bar(Settings::default());
    hover(&mut logic, &window, LOGO);
    hint_timer(&mut logic, &window);
    press(&mut logic, &window, LOGO, false);
    assert!(window.hint.borrow().is_none());
    release(&mut logic, &window, LOGO);
    hover(&mut logic, &window, PointF::new(LOGO.x + 1.0, 14.0));
    hint_timer(&mut logic, &window);
    assert!(
        window.hint.borrow().is_none(),
        "not again over the same sensor"
    );
}

#[test]
fn no_hints_while_popups_are_off() {
    let settings = Settings {
        popups_enabled: false,
        ..Settings::default()
    };
    let (mut logic, window, _, _) = bar(settings);
    hover(&mut logic, &window, LOGO);
    hint_timer(&mut logic, &window);
    assert!(window.hint.borrow().is_none());
}

#[test]
fn placeholders_explain_themselves() {
    let (mut logic, window, _, mut config) = bar(Settings::default());
    // A sensor of a type from a newer version, first on the left.
    let unknown = SensorInstance {
        id: "s_mail".into(),
        kind: "mail".into(),
        props: Default::default(),
    };
    config.layout.left.insert(0, unknown);
    logic.apply_config(&window, config);
    logic.sync_sensors(&window);
    logic.paint(&mut RecordingCanvas::new(1.0), SizeF::new(WIDTH, 28.0));
    hover(&mut logic, &window, PointF::new(10.0, 14.0));
    hint_timer(&mut logic, &window);
    let hint = window.hint.borrow().clone().expect("shown");
    assert!(
        hint.1.contains("not available in this version"),
        "{}",
        hint.1
    );
    // Kept on screen: 7 px from the monitor's left edge, less the shadow margin.
    assert_eq!(hint.0.left, 1);
}

#[test]
fn a_hint_is_rebuilt_only_when_its_sensor_changes() {
    use crate::sampler::{CpuSample, Sample, Topic};
    let (mut logic, window, _, _) = bar(Settings::default());
    hover(&mut logic, &window, LOGO);
    hint_timer(&mut logic, &window);
    assert!(window.hint.borrow().is_some(), "the logo's hint");
    *window.hint.borrow_mut() = None;
    // The CPU graph changes; the logo's hint is left as it is.
    let cpu = Sample::Cpu(CpuSample {
        total: 50.0,
        ..Default::default()
    });
    logic.deliver(
        &window,
        &Delivery {
            topic: Topic::Cpu,
            sample: Arc::new(cpu),
            to: None,
            pushed: false,
        },
    );
    assert!(window.hint.borrow().is_none(), "not shown again");
}

#[test]
fn a_live_hint_follows_its_sensor() {
    let (mut logic, window, _, config) = bar(Settings::default());
    hover(&mut logic, &window, LOGO);
    hint_timer(&mut logic, &window);
    let before = window.hint.borrow().clone().expect("shown").0;
    // A sensor added in front of the logo moves it to the right.
    let mut config = config;
    let text = SensorInstance {
        id: "s_text".into(),
        kind: "text".into(),
        props: serde_json::json!({ "text": "Hello" })
            .as_object()
            .cloned()
            .unwrap_or_default(),
    };
    config.layout.center.insert(0, text);
    logic.apply_config(&window, config);
    logic.sync_sensors(&window);
    let after = window.hint.borrow().clone().expect("still shown").0;
    assert!(after.left > before.left, "{before:?} → {after:?}");
}
