use super::*;

#[test]
fn subscriptions_change_only_where_topics_differ() {
    use crate::sampler::Topic;
    let uptime = |secs| TopicRequest {
        topic: Topic::Uptime,
        interval: Duration::from_secs(secs),
    };
    let map = |list: &[(&str, u64)]| -> HashMap<String, Vec<TopicRequest>> {
        list.iter()
            .map(|(id, secs)| ((*id).to_owned(), vec![uptime(*secs)]))
            .collect()
    };
    let old = map(&[("a", 60), ("b", 60), ("c", 60)]);
    let new = map(&[("a", 60), ("b", 30), ("d", 60)]);
    assert_eq!(
        subscription_changes(&old, &new, &[]),
        [
            ("b".to_owned(), vec![uptime(30)]),
            ("c".to_owned(), Vec::new()),
            ("d".to_owned(), vec![uptime(60)]),
        ]
    );
    assert!(subscription_changes(&new, &new, &[]).is_empty());
    // A re-created sensor subscribes again, to be sent the newest samples.
    assert_eq!(
        subscription_changes(&new, &new, &["a".to_owned(), "c".to_owned()]),
        [("a".to_owned(), vec![uptime(60)])]
    );
}

#[test]
fn a_sensors_new_state_is_stored() {
    use crate::sampler::{Delivery, Sample, Topic, UptimeSample};
    let (mut logic, window, host, mut config) = bar(Settings::default());
    let uptime = SensorInstance {
        id: "s_up".into(),
        kind: "uptime".into(),
        props: crate::sensors::Props::new(),
    };
    config.layout.left.push(uptime);
    logic.apply_config(&window, config);
    logic.sync_sensors(&window);
    let sample = Sample::Uptime(UptimeSample {
        boot_unix_ms: 0,
        uptime_secs: 700,
        read_at: Instant::now(),
    });
    logic.deliver(
        &window,
        &Delivery {
            topic: Topic::Uptime,
            sample: Arc::new(sample),
            to: None,
            pushed: false,
        },
    );
    assert!(
        host.calls()
            .contains(&r#"state s_up {"longestSecs":700}"#.to_owned()),
        "{:?}",
        host.calls()
    );
}

#[test]
fn a_resume_reaches_the_sampler() {
    let (mut logic, window, host, _) = bar(Settings::default());
    logic.on_event(&window, UiEvent::TimeChanged);
    assert!(!host.calls().contains(&"resumed".to_owned()));
    logic.on_event(&window, UiEvent::Resumed);
    assert!(host.calls().contains(&"resumed".to_owned()));
}

#[test]
fn sensors_needing_a_capability_wait_for_the_probe_and_the_others_do_not() {
    let (mut logic, window, _, mut config) = bar(Settings::default());
    // A system whose capability probe answers only when told to.
    let probe = Arc::new(
        FakePlatform::default()
            .with_capabilities(Capabilities {
                battery: true,
                ..Default::default()
            })
            .with_slow_probe(),
    );
    logic.platform = Arc::clone(&probe) as Arc<dyn Platform>;
    // A Battery in the logo's place: it needs the `battery` capability.
    config.layout.center[0].kind = "battery".into();
    let battery = config.layout.center[0].id.clone();
    logic.apply_config(&window, config);
    logic.sync_sensors(&window);
    assert_eq!(
        logic.sensors.find(&battery),
        None,
        "not shown while probing"
    );
    let cpu = logic.layout.left[0].id.clone();
    assert!(logic.sensors.find(&cpu).is_some(), "the others are");
    assert_eq!(window.probe_timer.get(), Some(PROBE_POLL));
    // Still probing: it checks again.
    window.probe_timer.set(None);
    logic.on_event(&window, UiEvent::Timer(TimerId::Probe));
    assert_eq!(window.probe_timer.get(), Some(PROBE_POLL));
    assert_eq!(logic.sensors.find(&battery), None);
    // Answered: the Battery joins, and no more checks.
    probe.answer_probe();
    window.probe_timer.set(None);
    logic.on_event(&window, UiEvent::Timer(TimerId::Probe));
    assert_eq!(logic.sensors.find(&battery), Some((Zone::Center, 0)));
    assert_eq!(window.probe_timer.get(), None);
}

#[test]
fn a_new_locale_reaches_every_sensor_on_its_context() {
    let (mut logic, window, _host, _) = bar(Settings::default());
    let platform = Arc::new(FakePlatform::default());
    platform.set_time(9, 5, 0);
    logic.platform = Arc::clone(&platform) as Arc<dyn Platform>;
    // The clocks read the time; the Date shows the month in lower case beside the day.
    logic.on_event(&window, UiEvent::TimeChanged);
    let drawn = |logic: &mut BarLogic| {
        let mut canvas = RecordingCanvas::new(1.0);
        logic.paint(&mut canvas, SizeF::new(WIDTH, 28.0));
        canvas.dump()
    };
    assert!(drawn(&mut logic).contains("\"october\""));
    // The OS regional format changes: the bar reads the locale once and measures again, and
    // the Date draws the new month name without reading the locale itself.
    let mut locale = crate::platform::Locale::en_us();
    locale.months[9] = "Oktober".into();
    platform.set_locale(locale);
    logic.on_event(&window, UiEvent::LocaleChanged);
    let after = drawn(&mut logic);
    assert!(after.contains("\"oktober\""), "{after}");
    assert!(!after.contains("\"october\""));
}
