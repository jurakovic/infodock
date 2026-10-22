use super::*;

fn placer_drawn(logic: &mut BarLogic) -> bool {
    let mut canvas = RecordingCanvas::new(1.0);
    logic.paint(&mut canvas, SizeF::new(WIDTH, 28.0));
    canvas.dump().lines().any(|l| l.ends_with("#0078D4"))
}

#[test]
fn dropping_files_and_links_onto_the_bar_adds_sensors() {
    use crate::platform::{DragItem, DropEffect, DropEffects};
    let (mut logic, window, host, _) = bar(Settings::default());
    let dir = tempfile::tempdir().expect("a temporary folder");
    let file = dir.path().join("notes.txt");
    std::fs::write(&file, b"x").expect("a file");
    let path = |p: &std::path::Path| p.to_string_lossy().into_owned();
    let items = vec![
        DragItem::Path(path(&file)),
        DragItem::Path(path(&dir.path().join("gone"))),
        DragItem::Path(path(dir.path())),
        DragItem::Url {
            url: "https://example.com/".into(),
            title: Some("Example".into()),
        },
        DragItem::Path("E:\\".into()),
    ];
    let at = PointF::new(1000.0, 14.0);
    let enter = DragEvent::Enter {
        items,
        at,
        modifiers: Modifiers::default(),
        allowed: DropEffects::ALL,
    };
    assert_eq!(logic.on_drag(&window, enter), DropEffect::Link);
    assert!(placer_drawn(&mut logic), "where the sensors would go");
    let over = DragEvent::Over {
        at: PointF::new(850.0, 14.0),
        modifiers: Modifiers::default(),
        allowed: DropEffects::ALL,
    };
    assert_eq!(logic.on_drag(&window, over), DropEffect::Link);
    let drop = DragEvent::Drop {
        at: PointF::new(850.0, 14.0),
        modifiers: Modifiers::default(),
        allowed: DropEffects::ALL,
    };
    assert_eq!(logic.on_drag(&window, drop), DropEffect::Link);
    assert!(!placer_drawn(&mut logic));
    // The right zone's first place: before its first sensor, one sensor per item that
    // exists, in order.
    assert_eq!(
        host.calls(),
        [
            format!(
                "add shortcut right 0 {}",
                serde_json::json!({ "target": path(&file) })
            ),
            format!(
                "add folder-menu right 1 {}",
                serde_json::json!({ "folder": path(dir.path()) })
            ),
            format!(
                "add shortcut right 2 {}",
                serde_json::json!({ "url": "https://example.com/", "title": "Example" })
            ),
            format!(
                "add disk right 3 {}",
                serde_json::json!({ "drive": "E:\\" })
            ),
        ]
    );
}

#[test]
fn a_folder_menu_takes_dropped_files_with_a_hint() {
    use crate::platform::{DragItem, DropEffect, DropEffects};
    let (mut logic, window, host, mut config) = bar(Settings::default());
    // Knows no volumes, so items are copied on every OS.
    let files = Arc::new(FakePlatform::default().without_volumes());
    logic.platform = Arc::clone(&files) as Arc<dyn Platform>;
    let target = tempfile::tempdir().expect("a temporary folder");
    let source = tempfile::tempdir().expect("a temporary folder");
    let file = source.path().join("report.pdf");
    std::fs::write(&file, b"x").expect("a file");
    // A Folder Menu in the logo's place.
    let logo = &mut config.layout.center[0];
    logo.kind = "folder-menu".into();
    logo.props = serde_json::json!({ "folder": target.path().to_string_lossy(), "title": "Inbox" })
        .as_object()
        .cloned()
        .unwrap_or_default();
    logic.apply_config(&window, config);
    logic.sync_sensors(&window);
    logic.paint(&mut RecordingCanvas::new(1.0), SizeF::new(WIDTH, 28.0));
    let enter = DragEvent::Enter {
        items: vec![DragItem::Path(file.to_string_lossy().into_owned())],
        at: LOGO,
        modifiers: Modifiers::default(),
        allowed: DropEffects::ALL,
    };
    // No volumes known: another volume, so a copy.
    assert_eq!(logic.on_drag(&window, enter), DropEffect::Copy);
    let (_, hint) = window.hint.borrow().clone().expect("the drop hint");
    assert!(hint.contains("\"Copy 'report.pdf' to 'Inbox'\""), "{hint}");
    let mut canvas = RecordingCanvas::new(1.0);
    logic.paint(&mut canvas, SizeF::new(WIDTH, 28.0));
    let dump = canvas.dump();
    assert!(dump.contains("#000000/78%"), "darkened: {dump}");
    assert!(!dump.lines().any(|l| l.ends_with("#0078D4")), "no placer");
    // Ctrl+Shift: a shortcut; the hint follows the keys.
    let over = DragEvent::Over {
        at: LOGO,
        modifiers: Modifiers {
            ctrl: true,
            shift: true,
            alt: false,
        },
        allowed: DropEffects::ALL,
    };
    assert_eq!(logic.on_drag(&window, over), DropEffect::Link);
    let (_, hint) = window.hint.borrow().clone().expect("the drop hint");
    assert!(
        hint.contains("Create shortcut to 'report.pdf' in 'Inbox'"),
        "{hint}"
    );
    // Off the sensor: the placer again, no hint.
    let away = DragEvent::Over {
        at: PointF::new(850.0, 14.0),
        modifiers: Modifiers::default(),
        allowed: DropEffects::ALL,
    };
    assert_eq!(logic.on_drag(&window, away), DropEffect::Link);
    assert!(window.hint.borrow().is_none());
    assert!(placer_drawn(&mut logic));
    let drop = DragEvent::Drop {
        at: LOGO,
        modifiers: Modifiers::default(),
        allowed: DropEffects::ALL,
    };
    assert_eq!(logic.on_drag(&window, drop), DropEffect::Copy);
    assert!(window.hint.borrow().is_none());
    assert!(host.calls().is_empty(), "no new sensors");
    let operations = files.log();
    assert_eq!(operations.len(), 1, "{operations:?}");
    assert!(operations[0].starts_with("Copy "), "{operations:?}");
    let mut canvas = RecordingCanvas::new(1.0);
    logic.paint(&mut canvas, SizeF::new(WIDTH, 28.0));
    assert!(!canvas.dump().contains("#000000/78%"));
}

#[test]
fn what_the_bar_cannot_take_is_refused() {
    use crate::platform::{DragItem, DropEffect, DropEffects};
    let (mut logic, window, host, _) = bar(Settings::default());
    let mail = DragEvent::Enter {
        items: vec![DragItem::Url {
            url: "mailto:me@example.com".into(),
            title: None,
        }],
        at: LOGO,
        modifiers: Modifiers::default(),
        allowed: DropEffects::ALL,
    };
    assert_eq!(logic.on_drag(&window, mail), DropEffect::None);
    assert!(!placer_drawn(&mut logic));
    let drop = DragEvent::Drop {
        at: LOGO,
        modifiers: Modifiers::default(),
        allowed: DropEffects::ALL,
    };
    assert_eq!(logic.on_drag(&window, drop), DropEffect::None);
    // A source that allows only moving: nothing to create links or copies with.
    let move_only = DropEffects {
        moves: true,
        ..DropEffects::default()
    };
    let enter = DragEvent::Enter {
        items: vec![DragItem::Path("E:\\".into())],
        at: LOGO,
        modifiers: Modifiers::default(),
        allowed: move_only,
    };
    assert_eq!(logic.on_drag(&window, enter), DropEffect::None);
    assert_eq!(logic.on_drag(&window, DragEvent::Leave), DropEffect::None);
    assert!(host.calls().is_empty());
}
