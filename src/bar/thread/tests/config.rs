//! Config changes reaching the bar keep the newest: two threads' changes can be
//! notified in the opposite order of their revisions.

use std::sync::Arc;

use super::*;
use crate::bar::BarController;
use crate::config::ConfigStore;
use crate::config::model::Edge;

#[test]
fn the_bar_thread_drops_a_config_older_than_its_own() {
    let config = Config::first_run("1.0.0");
    let (commands, queue) = mpsc::channel();
    let mut logic = BarLogic::new(
        (config.clone(), 3),
        crate::platform::current(),
        queue,
        Arc::new(FakeHost::default()) as Arc<dyn BarHost>,
        UiWaker::new(|| {}),
        PathBuf::from("no-skins"),
    );
    logic.text = Rc::new(crate::platform::recording::FixedMetrics);
    let window = FakeWindow::default();
    logic.on_event(&window, UiEvent::Started);

    let mut newer = config.clone();
    newer.settings.locked = true;
    let mut older = config.clone();
    older.settings.bar.edge = Edge::Top;
    commands
        .send(Command::Config(2, Box::new(older.clone())))
        .unwrap();
    logic.on_event(&window, UiEvent::Wake);
    assert_eq!(logic.settings, config.settings, "older than the bar's own");
    commands
        .send(Command::Config(5, Box::new(newer.clone())))
        .unwrap();
    commands.send(Command::Config(4, Box::new(older))).unwrap();
    logic.on_event(&window, UiEvent::Wake);
    assert_eq!(logic.settings, newer.settings);
}

#[test]
fn the_controller_keeps_the_newest_settings_of_two_threads_changes() {
    // Thread A's change reaches the controller only after thread B's, as when A's listeners
    // run late.
    let dir = tempfile::tempdir().unwrap();
    let (store, _) = ConfigStore::open(dir.path(), "1.0.0");
    let store = Arc::new(store);
    let (a_parked, parked) = mpsc::channel::<()>();
    let (release, a_released) = mpsc::channel::<()>();
    let a_released = Mutex::new(a_released);
    store.on_change(Box::new(move |_, new, _| {
        if new.settings.locked && new.settings.bar.edge == Edge::Bottom {
            let _ = a_parked.send(());
            let _ = a_released.lock().map(|r| r.recv());
        }
    }));
    let controller = BarController::create(
        &store,
        crate::platform::current(),
        Arc::new(FakeHost::default()),
        PathBuf::from("no-skins"),
    )
    .unwrap();

    let a = {
        let store = Arc::clone(&store);
        std::thread::spawn(move || {
            store
                .update(|c| {
                    c.settings.locked = true;
                    Ok(())
                })
                .unwrap();
        })
    };
    parked.recv().unwrap();
    store
        .update(|c| {
            c.settings.bar.edge = Edge::Top;
            Ok(())
        })
        .unwrap();
    release.send(()).unwrap();
    a.join().unwrap();

    assert_eq!(controller.settings.get(), store.get().settings);
    store.shutdown();
}
