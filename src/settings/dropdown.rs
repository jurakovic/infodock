//! The Settings window's dropdowns (`ui/dropdown.slint`) as the user drives them: the list opens
//! with the selected option over the control and the options before it above, and the mouse
//! and the keys choose and save as they go (INTERNALS.md, Settings window).

use std::sync::Arc;

use i_slint_backend_testing::{AccessibleRole, ElementHandle};
use i_slint_core::window::{PopupWindowLocation, WindowInner};
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, LogicalSize, SharedString};

use crate::app::App;
use crate::config::ConfigStore;
use crate::config::model::{FontStyle, ZOrder};
use crate::i18n::t;
use crate::ui;

/// A row of the list and the padding above the first, logical px: the Fluent style's list
/// items.
const ROW: f32 = 40.0;
const PADDING: f32 = 4.0;

/// The shown dropdown named `label`.
fn dropdown(window: &ui::SettingsWindow, label: &str) -> ElementHandle {
    ElementHandle::find_by_accessible_label(window, label)
        .find(|e| e.accessible_role() == Some(AccessibleRole::Combobox) && e.size().width > 0.0)
        .unwrap_or_else(|| panic!("a dropdown named {label:?}"))
}

/// The control's left edge and its centre's height in the window.
fn place(control: &ElementHandle) -> (f32, f32) {
    let (at, size) = (control.absolute_position(), control.size());
    (at.x, at.y + size.height / 2.0)
}

/// Where the open list's top left corner is in the window; `None` while no list is open.
fn list(window: &slint::Window) -> Option<(f32, f32)> {
    let inner = WindowInner::from_pub(window);
    let popups = inner.active_popups();
    match popups.first().map(|p| &p.location) {
        Some(PopupWindowLocation::ChildWindow(at)) => Some((at.x, at.y)),
        _ => None,
    }
}

/// The centre's height of the list's `row`-th row shown, for a list whose top is at `top`.
fn row_centre(top: f32, row: usize) -> f32 {
    top + PADDING + ROW * row as f32 + ROW / 2.0
}

fn settle() {
    slint::platform::update_timers_and_animations();
}

fn click(window: &slint::Window, (x, y): (f32, f32)) {
    let position = LogicalPosition::new(x, y);
    let button = PointerEventButton::Left;
    window.dispatch_event(WindowEvent::PointerPressed { position, button });
    window.dispatch_event(WindowEvent::PointerReleased { position, button });
    settle();
}

fn key(window: &slint::Window, text: SharedString) {
    window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window.dispatch_event(WindowEvent::KeyReleased { text });
    settle();
}

#[test]
fn a_dropdown_opens_on_its_selection_and_chooses_with_the_mouse_and_the_keys() {
    i_slint_backend_testing::init_no_event_loop();
    let dir = tempfile::tempdir().expect("temp dir");
    let (store, _) = ConfigStore::open(dir.path(), "test");
    let app = Arc::new(App::new(
        store,
        crate::platform::current(),
        dir.path().join("skins"),
    ));
    super::open(&app, "bar");
    let ui = super::window::handle().expect("Settings is open");
    let window = ui.window();
    // Tall enough that the pages do not scroll and no list reaches the window's edge.
    window.set_size(LogicalSize::new(760.0, 3000.0));
    settle();

    // Bar's *Z-order*: Topmost, Normal and Bottom, the last chosen by default. A click opens the
    // list with Bottom over the control, under the pointer, and the other two above it.
    let z_order = || app.store.get().settings.bar.z_order;
    assert_eq!(z_order(), ZOrder::Bottom);
    let label = t("settings.bar.zOrder");
    let control = dropdown(&ui, &label);
    let (left, centre) = place(&control);
    let x = left + 40.0;
    assert_eq!(control.accessible_expanded(), Some(false));
    click(window, (x, centre));
    let (list_x, top) = list(window).expect("the list is open");
    assert_eq!(list_x, left);
    assert_eq!(row_centre(top, 2), centre, "Bottom over the control");
    assert_eq!(control.accessible_expanded(), Some(true));
    // A click where it opened keeps the choice; one a row higher takes Normal.
    click(window, (x, centre));
    assert!(
        list(window).is_none(),
        "a click on an option closes the list"
    );
    assert_eq!(z_order(), ZOrder::Bottom);
    assert_eq!(control.accessible_expanded(), Some(false));
    click(window, (x, centre));
    click(window, (x, centre - ROW));
    assert_eq!(z_order(), ZOrder::Normal);
    assert_eq!(
        control.accessible_value().as_deref(),
        Some(t("settings.bar.zOrder.normal").as_str())
    );

    // The keys: the control kept the focus. Up and Down choose on the closed control; Enter or
    // Space opens the list on the choice, the arrows choose in it, and Enter, Space or Esc
    // closes it.
    key(window, Key::UpArrow.into());
    assert_eq!(z_order(), ZOrder::Topmost);
    key(window, Key::UpArrow.into());
    assert_eq!(z_order(), ZOrder::Topmost, "it stays on the first");
    key(window, Key::Return.into());
    let (_, top) = list(window).expect("Enter opens the list");
    assert_eq!(row_centre(top, 0), centre, "Topmost over the control");
    key(window, Key::DownArrow.into());
    assert_eq!(z_order(), ZOrder::Normal);
    assert_eq!(
        list(window),
        Some((left, top)),
        "the list stays where it opened"
    );
    key(window, Key::Escape.into());
    assert!(list(window).is_none(), "Esc closes the list");
    assert_eq!(z_order(), ZOrder::Normal, "and keeps the choice");
    key(window, ' '.into());
    let (_, top) = list(window).expect("Space opens the list");
    assert_eq!(row_centre(top, 1), centre, "Normal over the control");
    key(window, ' '.into());
    assert!(list(window).is_none(), "Space closes the list");
    key(window, Key::DownArrow.into());
    assert_eq!(z_order(), ZOrder::Bottom);
    key(window, Key::DownArrow.into());
    assert_eq!(z_order(), ZOrder::Bottom, "it stays on the last");
    key(window, Key::Return.into());
    key(window, Key::Return.into());
    assert!(list(window).is_none(), "Enter closes the list");
    // The wheel chooses too, once the control has the focus.
    let wheel = |delta_y: f32| {
        let position = LogicalPosition::new(x, centre);
        window.dispatch_event(WindowEvent::PointerScrolled {
            position,
            delta_x: 0.0,
            delta_y,
        });
        settle();
    };
    wheel(10.0);
    assert_eq!(z_order(), ZOrder::Normal);

    // Sensors' *Font style*: Theme default and six styles, so the list shows six rows and
    // scrolls. Opened on the first, Down to the last scrolls the list one row and leaves it in
    // place.
    let style = || app.store.get().settings.sensors.font_style;
    super::open(&app, "sensors");
    settle();
    let control = dropdown(&ui, &t("settings.fontStyle"));
    let (left, centre) = place(&control);
    let x = left + 40.0;
    click(window, (x, centre));
    let (_, top) = list(window).expect("the list is open");
    assert_eq!(row_centre(top, 0), centre, "Theme default over the control");
    (0..6).for_each(|_| key(window, Key::DownArrow.into()));
    assert_eq!(style(), Some(FontStyle::BoldItalic));
    assert_eq!(list(window), Some((left, top)));
    // The list scrolled to show it: its fifth row is Semibold Italic, the last but one.
    click(window, (x, row_centre(top, 4)));
    assert_eq!(style(), Some(FontStyle::SemiboldItalic));
    // Opened on the last but one, the list shows the last below it and shifts up by four rows,
    // not five: as near its middle as the options allow.
    click(window, (x, centre));
    let (_, top) = list(window).expect("the list is open");
    assert_eq!(
        row_centre(top, 4),
        centre,
        "Semibold Italic over the control"
    );
    click(window, (x, centre + ROW));
    assert_eq!(style(), Some(FontStyle::BoldItalic));
    // Up in the list goes back to the first.
    key(window, Key::Return.into());
    (0..6).for_each(|_| key(window, Key::UpArrow.into()));
    key(window, Key::Return.into());
    assert_eq!(style(), None, "Theme default");
    super::window::set(None);
    app.store.shutdown();
}
