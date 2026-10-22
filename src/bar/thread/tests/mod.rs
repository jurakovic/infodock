//! The bar logic driven end to end with a fake window and host, on every OS.

use std::cell::{Cell, RefCell};
use std::sync::Mutex;
use std::sync::mpsc;

use super::*;
use crate::config::model::ClicksWhenLocked;
use crate::platform::recording::RecordingCanvas;
use crate::platform::{Capabilities, MenuEntry, PointerButton};
use crate::test_util::FakePlatform;

mod external_drop;
mod frame;
mod hover;
mod keyboard;
mod menu;
mod pointer;
mod popups;
mod sensors;

/// Records what the bar asked of the window; the menu returns `choose` once.
#[derive(Default)]
struct FakeWindow {
    choose: Cell<Option<u32>>,
    menus: RefCell<Vec<Vec<MenuEntry>>>,
    /// Where each menu was shown: at the pointer (`None`) or beside a rect.
    menus_near: RefCell<Vec<Option<Rect>>>,
    /// The bar took the keyboard focus, and gave it back this often.
    focus_taken: Cell<u32>,
    focus_restored: Cell<u32>,
    /// The window cannot take the focus.
    no_focus: Cell<bool>,
    captured: Cell<bool>,
    confirmed: RefCell<Vec<String>>,
    hint_timer: Cell<Option<std::time::Duration>>,
    /// Frame timers requested.
    frames: Cell<u32>,
    /// Where a floating bar is; `None` when docked.
    position: Cell<Option<Rect>>,
    /// The rise animation's offsets.
    slides: RefCell<Vec<i32>>,
    escape: Cell<bool>,
    /// Answer confirmations with Cancel.
    decline: Cell<bool>,
    /// The shown hint: where, and what it draws; and its title, for screen readers.
    hint: RefCell<Option<(Rect, String)>>,
    hint_name: RefCell<String>,
    popup_name: RefCell<String>,
    /// The shown popup: where, and what it draws; and how often it was drawn.
    popup: RefCell<Option<(Rect, String)>>,
    popup_draws: Cell<u32>,
    popup_timer: Cell<Option<std::time::Duration>>,
    probe_timer: Cell<Option<std::time::Duration>>,
    popup_cursor: Cell<crate::platform::PopupCursor>,
    cursor: Cell<PopupCursor>,
    popup_captured: Cell<bool>,
}

impl BarWindow for FakeWindow {
    fn invalidate(&self) {}
    fn set_material(&self, _material: Material) {}
    fn set_tray_icon(&self, _tooltip: Option<&str>) {}
    fn show_menu(&self, entries: &[MenuEntry], near: Option<Rect>) -> Option<u32> {
        self.menus.borrow_mut().push(entries.to_vec());
        self.menus_near.borrow_mut().push(near);
        self.choose.take()
    }
    fn take_focus(&self) -> bool {
        self.focus_taken.set(self.focus_taken.get() + 1);
        !self.no_focus.get()
    }
    fn restore_focus(&self) {
        self.focus_restored.set(self.focus_restored.get() + 1);
    }
    fn set_timer(&self, id: TimerId, delay: Option<std::time::Duration>) {
        match id {
            TimerId::Hint => self.hint_timer.set(delay),
            TimerId::Frame => self.frames.set(self.frames.get() + 1),
            TimerId::Popup => self.popup_timer.set(delay),
            TimerId::Probe => self.probe_timer.set(delay),
            _ => {}
        }
    }
    fn show_hint(
        &self,
        rect: Rect,
        scale: f32,
        name: &str,
        paint: &mut dyn FnMut(&mut dyn Canvas),
    ) {
        let mut canvas = RecordingCanvas::new(scale);
        paint(&mut canvas);
        *self.hint.borrow_mut() = Some((rect, canvas.dump()));
        *self.hint_name.borrow_mut() = name.to_owned();
    }
    fn show_popup(
        &self,
        rect: Rect,
        scale: f32,
        name: &str,
        paint: &mut dyn FnMut(&mut dyn Canvas),
    ) {
        let mut canvas = RecordingCanvas::new(scale);
        paint(&mut canvas);
        *self.popup.borrow_mut() = Some((rect, canvas.dump()));
        *self.popup_name.borrow_mut() = name.to_owned();
        self.popup_draws.set(self.popup_draws.get() + 1);
    }
    fn hide_popup(&self) {
        *self.popup.borrow_mut() = None;
    }
    fn set_popup_cursor(&self, cursor: crate::platform::PopupCursor) {
        self.popup_cursor.set(cursor);
    }
    fn capture_popup_pointer(&self, capture: bool) {
        self.popup_captured.set(capture);
    }
    fn hide_hint(&self) {
        *self.hint.borrow_mut() = None;
    }
    fn capture_pointer(&self, capture: bool) {
        self.captured.set(capture);
    }
    fn slide(&self, dy: i32) {
        self.slides.borrow_mut().push(dy);
    }
    fn resize_to(&self, rect: Rect, _width: u32) -> Option<Rect> {
        self.position.get()?;
        self.position.set(Some(rect));
        Some(rect)
    }
    fn set_cursor(&self, cursor: PopupCursor) {
        self.cursor.set(cursor);
    }
    fn move_by(&self, dx: i32, dy: i32) -> Option<Rect> {
        let mut rect = self.position.get()?;
        rect = Rect::new(
            rect.left + dx,
            rect.top + dy,
            rect.right + dx,
            rect.bottom + dy,
        );
        self.position.set(Some(rect));
        Some(rect)
    }
    fn escape_down(&self) -> bool {
        self.escape.get()
    }
    fn confirm(&self, message: &str, _action: &str) -> bool {
        self.confirmed.borrow_mut().push(message.to_owned());
        !self.decline.get()
    }
}

#[derive(Default)]
struct FakeHost {
    calls: Mutex<Vec<String>>,
    /// What `change_settings` changes.
    settings: Mutex<Settings>,
}

impl FakeHost {
    fn log(&self, call: String) {
        self.calls
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(call);
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl BarHost for FakeHost {
    fn quit(&self) {
        self.log("quit".into());
    }
    fn activate(&self) {}
    fn bar_visible(&self) -> bool {
        true
    }
    fn toggle_bar(&self) {}
    fn open_settings(&self, route: &str) {
        self.log(format!("open {route}"));
    }
    fn change_settings(&self, change: Box<dyn FnOnce(&mut Settings) + Send>) {
        self.log("change settings".into());
        change(&mut self.settings.lock().unwrap_or_else(|e| e.into_inner()));
    }
    fn add_sensor(&self, kind: &str, zone: Zone, index: usize) -> Option<String> {
        self.log(format!("add {kind} {} {index}", zone_id(zone)));
        Some("s_new".into())
    }
    fn add_sensors(
        &self,
        zone: Zone,
        index: usize,
        sensors: Vec<(&'static str, crate::sensors::Props)>,
    ) {
        for (i, (kind, props)) in sensors.into_iter().enumerate() {
            self.log(format!(
                "add {kind} {} {} {}",
                zone_id(zone),
                index + i,
                serde_json::Value::Object(props)
            ));
        }
    }
    fn remove_sensor(&self, id: &str) {
        self.log(format!("remove {id}"));
    }
    fn move_sensor(&self, id: &str, zone: Zone, index: usize) {
        self.log(format!("move {id} {} {index}", zone_id(zone)));
    }
    fn subscribe(&self, id: &str, topics: Vec<TopicRequest>) {
        self.log(format!("subscribe {id} {topics:?}"));
    }
    fn redock(&self) {
        self.log("redock".into());
    }
    fn resumed(&self) {
        self.log("resumed".into());
    }
    fn changed(&self, change: Change) {
        self.log(format!("changed {change:?}"));
    }
    fn set_sensor_state(&self, id: &str, state: crate::sensors::State) {
        self.log(format!("state {id} {}", serde_json::Value::Object(state)));
    }
    fn update_sensor(&self, id: &str, changes: crate::sensors::Props) {
        self.log(format!(
            "update {id} {}",
            serde_json::Value::Object(changes)
        ));
    }
    fn import_layout(&self, format: ImportFormat) {
        self.log(format!("import {format:?}"));
    }
    fn export_layout(&self) {
        self.log("export".into());
    }
}

const WIDTH: f32 = 1200.0;

/// The bar logic with the first-run layout, laid out on a 1200 px bar, in place.
fn bar(settings: Settings) -> (BarLogic, FakeWindow, Arc<FakeHost>, Config) {
    let (mut logic, window, host, config) = rising_bar(settings);
    logic.frame(&window, Instant::now() + Duration::from_secs(1));
    window.frames.set(0);
    window.slides.borrow_mut().clear();
    // The start's subscriptions (the CPU graph) are not what tests look for.
    host.calls.lock().unwrap_or_else(|e| e.into_inner()).clear();
    logic.paint(&mut RecordingCanvas::new(1.0), SizeF::new(WIDTH, 28.0));
    (logic, window, host, config)
}

/// The bar logic just placed, so it has started to rise (with animations).
fn rising_bar(settings: Settings) -> (BarLogic, FakeWindow, Arc<FakeHost>, Config) {
    let mut config = Config::first_run("1.0.0");
    config.settings = settings;
    let host = Arc::new(FakeHost::default());
    let (_tx, rx) = mpsc::channel();
    let mut logic = BarLogic::new(
        config.clone(),
        crate::platform::current(),
        rx,
        Arc::clone(&host) as Arc<dyn BarHost>,
        UiWaker::new(|| {}),
        PathBuf::from("no-skins"),
    );
    // The same text metrics on every OS, readable by the recording canvas.
    logic.text = Rc::new(crate::platform::recording::FixedMetrics);
    let window = FakeWindow::default();
    logic.on_event(&window, UiEvent::Started);
    // A 1200 × 1080 monitor with the bar at the bottom.
    let monitor = crate::platform::MonitorInfo::new(
        r"\\.\DISPLAY1".into(),
        "Test".into(),
        Rect::new(0, 0, 1200, 1080),
        Rect::new(0, 0, 1200, 1052),
        1.0,
        true,
    );
    logic.on_event(
        &window,
        UiEvent::Placed(BarGeometry {
            rect: Rect::new(0, 1052, 1200, 1080),
            edge: crate::config::model::Edge::Bottom,
            monitor,
            scale: 1.0,
        }),
    );
    logic.paint(&mut RecordingCanvas::new(1.0), SizeF::new(WIDTH, 28.0));
    (logic, window, host, config)
}

/// The middle of the logo, the only sensor in the centre.
const LOGO: PointF = PointF::new(WIDTH / 2.0, 14.0);

fn press(logic: &mut BarLogic, window: &FakeWindow, at: PointF, alt: bool) {
    let modifiers = Modifiers {
        alt,
        ..Modifiers::default()
    };
    logic.on_event(
        window,
        UiEvent::PointerDown {
            button: PointerButton::Left,
            at,
            modifiers,
        },
    );
}

fn release(logic: &mut BarLogic, window: &FakeWindow, at: PointF) {
    logic.on_event(
        window,
        UiEvent::PointerUp {
            button: PointerButton::Left,
            at,
            modifiers: Modifiers::default(),
        },
    );
}

fn right_click(logic: &mut BarLogic, window: &FakeWindow, at: PointF, choose: MenuCommand) {
    window.choose.set(Some(choose.id()));
    logic.on_event(
        window,
        UiEvent::PointerUp {
            button: PointerButton::Right,
            at,
            modifiers: Modifiers::default(),
        },
    );
}

/// Puts a Power sensor, whose click asks first, in the logo's place.
fn with_power_sensor(logic: &mut BarLogic, window: &FakeWindow) -> Arc<FakePlatform> {
    // Records power actions instead of taking them: the tests run on Windows too.
    let power = Arc::new(FakePlatform::default().with_capabilities(Capabilities {
        power_modes: vec!["shutDown".into()],
        ..Default::default()
    }));
    logic.platform = Arc::clone(&power) as Arc<dyn Platform>;
    let mut config = Config {
        settings: logic.settings.clone(),
        layout: logic.layout.clone(),
        state: logic.state.clone(),
        ..Config::default()
    };
    let logo = &mut config.layout.center[0];
    logo.kind = "power".into();
    logo.props = serde_json::json!({ "mode": "shutDown" })
        .as_object()
        .cloned()
        .unwrap_or_default();
    logic.apply_config(window, config);
    logic.sync_sensors(window);
    logic.paint(&mut RecordingCanvas::new(1.0), SizeF::new(WIDTH, 28.0));
    power
}

fn key(logic: &mut BarLogic, window: &FakeWindow, key: Key, shift: bool) {
    let modifiers = Modifiers {
        shift,
        ..Modifiers::default()
    };
    logic.on_event(window, UiEvent::Key { key, modifiers });
}

/// The focus ring the last frame drew: its rectangle, or `None`.
fn ring(logic: &mut BarLogic) -> Option<String> {
    let mut canvas = RecordingCanvas::new(1.0);
    logic.paint(&mut canvas, SizeF::new(WIDTH, 28.0));
    canvas
        .dump()
        .lines()
        .find(|l| l.starts_with("stroke rounded") && l.ends_with("w2"))
        .map(str::to_owned)
}

fn hover(logic: &mut BarLogic, window: &FakeWindow, at: PointF) {
    logic.on_event(window, UiEvent::PointerMove(at));
}

fn hint_timer(logic: &mut BarLogic, window: &FakeWindow) {
    logic.on_event(window, UiEvent::Timer(TimerId::Hint));
}

/// What reaches a [`Probe`].
type Log = Rc<RefCell<Vec<String>>>;

/// A 100 × 50 popup that logs what reaches it and answers with `reply`.
struct Probe {
    log: Log,
    reply: Rc<Cell<PopupReply>>,
    wake: Option<Instant>,
    props: Option<crate::sensors::Props>,
}

impl Popup for Probe {
    fn kind(&self) -> &'static str {
        "probe"
    }
    fn layout(&mut self, cx: &PopupCtx) -> SizeF {
        self.log
            .borrow_mut()
            .push(format!("layout in {}×{}", cx.room.width, cx.room.height));
        SizeF::new(100.0, 50.0)
    }
    fn draw(&self, canvas: &mut dyn Canvas, origin: PointF, _theme: &Theme) {
        canvas.fill_rect(
            RectF::new(origin.x, origin.y, origin.x + 100.0, origin.y + 50.0),
            crate::platform::Color::rgba(1, 2, 3, 1.0),
        );
    }
    fn input(&mut self, input: &PopupInput, _cx: &PopupCtx) -> PopupReply {
        self.log.borrow_mut().push(format!("{input:?}"));
        self.reply.get()
    }
    fn wake_at(&self) -> Option<Instant> {
        self.wake
    }
    fn on_wake(&mut self, _cx: &PopupCtx) -> PopupReply {
        self.log.borrow_mut().push("wake".into());
        self.wake = None;
        PopupReply::Close
    }
    fn poll(&mut self, _cx: &PopupCtx) -> PopupReply {
        self.log.borrow_mut().push("poll".into());
        PopupReply::Nothing
    }
    fn take_props(&mut self) -> Option<crate::sensors::Props> {
        self.props.take()
    }
}

fn new_probe() -> (Probe, Log, Rc<Cell<PopupReply>>) {
    let log = Rc::new(RefCell::new(Vec::new()));
    let reply = Rc::new(Cell::new(PopupReply::Nothing));
    let probe = Probe {
        log: Rc::clone(&log),
        reply: Rc::clone(&reply),
        wake: None,
        props: None,
    };
    (probe, log, reply)
}
