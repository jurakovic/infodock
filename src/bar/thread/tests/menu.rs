use super::*;

#[test]
fn sensor_menu_commands() {
    let (mut logic, window, host, config) = bar(Settings::default());
    let logo = &config.layout.center[0].id;
    right_click(&mut logic, &window, LOGO, MenuCommand::Properties);
    right_click(&mut logic, &window, LOGO, MenuCommand::NewSensor);
    right_click(&mut logic, &window, LOGO, MenuCommand::Remove);
    assert_eq!(
        host.calls(),
        [
            format!("open sensor/{logo}"),
            format!("open new?target={logo}"),
            format!("remove {logo}"),
        ]
    );
    assert_eq!(*window.confirmed.borrow(), ["Remove sensor “Image”?"]);
    // The sensor menu has Properties; the logo has some, so it is enabled.
    let menus = window.menus.borrow();
    assert!(menus[0].iter().any(|e| matches!(
        e,
        MenuEntry::Item { id, enabled: true, .. } if *id == MenuCommand::Properties.id()
    )));
}

#[test]
fn tools_import_and_export_through_the_host() {
    let (mut logic, window, host, _) = bar(Settings::default());
    let empty = PointF::new(300.0, 14.0);
    right_click(&mut logic, &window, empty, MenuCommand::ImportLayout);
    right_click(&mut logic, &window, LOGO, MenuCommand::ExportLayout);
    right_click(&mut logic, &window, empty, MenuCommand::ImportLegacy);
    right_click(&mut logic, &window, LOGO, MenuCommand::Help);
    assert_eq!(
        host.calls(),
        ["import Json", "export", "import Legacy", "open help"]
    );
}

#[test]
fn bar_menu_inserts_by_zone() {
    let (mut logic, window, host, _) = bar(Settings::default());
    // Left third, right of the three placeholders: appended.
    right_click(
        &mut logic,
        &window,
        PointF::new(350.0, 14.0),
        MenuCommand::NewSensor,
    );
    // Right third, left of Date and Time: first in the zone.
    right_click(
        &mut logic,
        &window,
        PointF::new(900.0, 14.0),
        MenuCommand::NewSensor,
    );
    right_click(
        &mut logic,
        &window,
        PointF::new(900.0, 14.0),
        MenuCommand::Settings,
    );
    right_click(
        &mut logic,
        &window,
        PointF::new(900.0, 14.0),
        MenuCommand::ToggleLock,
    );
    assert_eq!(
        host.calls(),
        [
            "open new?zone=left&index=3",
            "open new?zone=right&index=0",
            "open general",
            "change settings",
        ]
    );
}

#[test]
fn submenu_adds_the_type_and_opens_its_properties() {
    let settings = Settings {
        new_sensor_menu: crate::config::model::NewSensorMenu::Submenu,
        ..Settings::default()
    };
    let (mut logic, window, host, _) = bar(settings);
    let types = menus::addable_types(registry(), &logic.platform.capabilities());
    let text = types
        .iter()
        .position(|t| t.id() == "text")
        .expect("text type");
    right_click(&mut logic, &window, LOGO, MenuCommand::Add(text));
    assert_eq!(host.calls(), ["add text center 1", "open sensor/s_new"]);
}
