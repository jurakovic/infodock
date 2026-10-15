//! The running app: state shared by all threads, bar visibility, and quitting.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use crate::bar::{BarController, BarHost};
use crate::config::model::{Settings, Zone};
use crate::config::{ConfigError, ConfigStore};
use crate::platform::Platform;
use crate::providers::Providers;
use crate::sampler::{Delivery, Pending, Sample, Sampler, Topic, TopicRequest, VOLUMES_EVERY};
use crate::sensors::registry::registry;
use crate::sensors::{DefaultsCtx, Props, ops};
use crate::settings;

/// How long a requested quit may take before the process ends anyway.
const QUIT_TIMEOUT: Duration = Duration::from_secs(5);
/// The sampler subscriber id of the Settings window.
const SETTINGS: &str = "settings";

pub struct App {
    pub store: ConfigStore,
    pub platform: Arc<dyn Platform>,
    bar: OnceLock<Arc<BarController>>,
    sampler: Sampler,
    /// The sampler thread until [`App::start_sampler`] starts it.
    pending: Mutex<Option<Pending>>,
    shut_down: AtomicBool,
}

impl App {
    pub fn new(store: ConfigStore, platform: Arc<dyn Platform>) -> Self {
        let (sampler, pending) = Sampler::new();
        Self {
            store,
            platform,
            bar: OnceLock::new(),
            sampler,
            pending: Mutex::new(Some(pending)),
            shut_down: AtomicBool::new(false),
        }
    }

    pub fn set_bar(&self, bar: Arc<BarController>) {
        let _ = self.bar.set(bar);
    }

    /// Starts the sampler thread, which delivers to the bar (SPEC §4.4), and the drive list to
    /// Settings while it watches it. Subscriptions made before it starts are kept.
    pub fn start_sampler(&self) {
        let pending = self
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        let (Some(pending), Some(bar)) = (pending, self.bar.get()) else {
            return;
        };
        let bar = Arc::downgrade(bar);
        let deliver = Box::new(move |delivery: Delivery| {
            let for_settings = delivery.to.as_deref().is_none_or(|to| to == SETTINGS);
            if for_settings && let Sample::Volumes(volumes) = delivery.sample.as_ref() {
                settings::volumes_changed(volumes.clone());
            }
            if let Some(bar) = bar.upgrade() {
                bar.deliver(delivery);
            }
        });
        if let Err(e) = pending.start(Providers::new(Arc::clone(&self.platform)), deliver) {
            tracing::error!("the sampler could not start: {e}");
        }
    }

    /// Settings receives the drive list while `watch` (a form with a drive field is open), so
    /// it follows drives that arrive or go (SPEC §9.12).
    pub fn watch_volumes(&self, watch: bool) {
        let topics = if watch {
            vec![TopicRequest {
                topic: Topic::Volumes,
                interval: VOLUMES_EVERY,
            }]
        } else {
            Vec::new()
        };
        self.sampler.subscribe(SETTINGS, topics);
    }

    pub fn bar_visible(&self) -> bool {
        self.bar.get().is_none_or(|b| b.is_visible())
    }

    /// Shows or hides the bar (tray click, hotkey).
    pub fn toggle_bar(&self) {
        if let Some(bar) = self.bar.get() {
            bar.toggle_visible();
        }
    }

    /// Shows the bar if hidden and brings it to the front (hotkey).
    pub fn bring_bar_to_front(&self) {
        if let Some(bar) = self.bar.get() {
            bar.bring_to_front();
        }
    }

    /// Ends the event loop from any thread; the main thread then cleans up. If the loop does not
    /// end in time (or is gone), the process ends here after the same cleanup.
    pub fn request_quit(self: &Arc<Self>) {
        let queued = slint::invoke_from_event_loop(|| {
            let _ = slint::quit_event_loop();
        })
        .is_ok();
        let app = Arc::clone(self);
        std::thread::spawn(move || {
            if queued {
                std::thread::sleep(QUIT_TIMEOUT);
                tracing::warn!("the event loop did not end; quitting anyway");
            }
            app.shutdown();
            std::process::exit(0);
        });
    }

    /// Releases the reserved screen space and the tray icon, writes pending config changes and
    /// closes the log.
    /// Runs once, whichever path ends the app.
    pub fn shutdown(&self) {
        if self.shut_down.swap(true, Ordering::SeqCst) {
            return;
        }
        if let Some(bar) = self.bar.get() {
            bar.release();
        }
        self.sampler.shutdown();
        self.store.shutdown();
        tracing::info!("Infodock exiting");
        crate::logging::shutdown();
    }
}

/// Sensor operations (SPEC §4.5). Each saves through the config store, which updates the bar
/// and Settings.
impl App {
    /// Adds a sensor; see [`ops::add`]. Returns its id.
    pub fn add_sensor(
        &self,
        kind: &str,
        zone: Zone,
        index: usize,
        props: &Props,
    ) -> Result<String, ConfigError> {
        let (id, _) = self
            .store
            .update(|c| ops::add(c, registry(), &self.defaults(), kind, zone, index, props))?;
        Ok(id)
    }

    /// Where machine-derived props come from (SPEC §11.2).
    fn defaults(&self) -> DefaultsCtx<'_> {
        DefaultsCtx {
            platform: self.platform.as_ref(),
        }
    }

    /// Saves `changes` to sensor `id` and ends its preview.
    pub fn update_sensor(&self, id: &str, changes: &Props) -> Result<(), ConfigError> {
        let result = self
            .store
            .update(|c| ops::update(c, registry(), &self.defaults(), id, changes));
        self.end_preview(id);
        result.map(|_| ())
    }

    /// Shows sensor `id` with `changes` on the bar without saving (SPEC §6.7).
    pub fn preview_sensor(&self, id: &str, changes: &Props) -> Result<(), ConfigError> {
        let shown = ops::preview(&self.store.get(), registry(), &self.defaults(), id, changes)?;
        if let Some(bar) = self.bar.get() {
            bar.preview(shown);
        }
        Ok(())
    }

    /// Shows sensor `id` with its saved props again.
    pub fn end_preview(&self, id: &str) {
        if let Some(bar) = self.bar.get() {
            bar.end_preview(id);
        }
    }

    /// Moves a sensor; `index` counts with the sensor already taken out (SPEC §6.2).
    pub fn move_sensor(&self, id: &str, zone: Zone, index: usize) -> Result<(), ConfigError> {
        self.store
            .update(|c| ops::move_to(c, id, zone, index))
            .map(|_| ())
    }

    pub fn remove_sensor(&self, id: &str) -> Result<(), ConfigError> {
        let result = self.store.update(|c| ops::remove(c, id)).map(|_| ());
        self.end_preview(id);
        result
    }
}

/// The bar's view of the app (called on the bar thread).
pub struct AppHost(pub Arc<App>);

impl BarHost for AppHost {
    fn quit(&self) {
        self.0.request_quit();
    }

    fn activate(&self) {
        if let Some(bar) = self.0.bar.get() {
            bar.set_visible(true);
        }
        self.open_settings("general");
    }

    fn bar_visible(&self) -> bool {
        self.0.bar_visible()
    }

    fn toggle_bar(&self) {
        self.0.toggle_bar();
    }

    fn open_settings(&self, route: &str) {
        let (app, route) = (Arc::clone(&self.0), route.to_owned());
        let _ = slint::invoke_from_event_loop(move || settings::open(&app, &route));
    }

    fn change_settings(&self, change: Box<dyn FnOnce(&mut Settings) + Send>) {
        let result = self.0.store.update(|c| {
            change(&mut c.settings);
            Ok(())
        });
        if let Err(e) = result {
            tracing::warn!("could not change settings: {e}");
        }
    }

    fn add_sensor(&self, kind: &str, zone: Zone, index: usize) -> Option<String> {
        self.0
            .add_sensor(kind, zone, index, &Props::new())
            .map_err(|e| tracing::warn!("could not add a {kind} sensor: {e}"))
            .ok()
    }

    fn remove_sensor(&self, id: &str) {
        if let Err(e) = self.0.remove_sensor(id) {
            tracing::warn!("could not remove sensor {id}: {e}");
        }
    }

    fn move_sensor(&self, id: &str, zone: Zone, index: usize) {
        if let Err(e) = self.0.move_sensor(id, zone, index) {
            tracing::warn!("could not move sensor {id}: {e}");
        }
    }

    fn subscribe(&self, id: &str, topics: Vec<TopicRequest>) {
        self.0.sampler.subscribe(id, topics);
    }

    fn redock(&self) {
        if let Some(bar) = self.0.bar.get() {
            bar.redock();
        }
    }

    fn resumed(&self) {
        self.0.sampler.resume();
    }

    fn devices_changed(&self) {
        self.0.sampler.devices_changed();
    }

    fn set_sensor_state(&self, id: &str, state: crate::sensors::State) {
        let result = self.0.store.update(|c| {
            // A sensor removed meanwhile keeps no state.
            if c.layout.find(id).is_some() {
                c.state.insert(id.to_owned(), state);
            }
            Ok(())
        });
        if let Err(e) = result {
            tracing::warn!("could not store the state of sensor {id}: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn sensor_operations_save_through_the_store() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (store, _) = ConfigStore::open(dir.path(), "test");
        let app = App::new(store, crate::platform::current());
        let props = json!({ "text": "Hi" })
            .as_object()
            .cloned()
            .unwrap_or_default();
        let id = app
            .add_sensor("text", Zone::Center, 1, &props)
            .expect("added");
        let saved = |app: &App| app.store.get().layout;
        assert_eq!(saved(&app).center[1].id, id);

        let bigger = json!({ "fontSize": 20 })
            .as_object()
            .cloned()
            .unwrap_or_default();
        app.preview_sensor(&id, &bigger).expect("previewed");
        assert_eq!(saved(&app).center[1].props.get("fontSize"), None);
        app.update_sensor(&id, &bigger).expect("updated");
        assert_eq!(saved(&app).center[1].props["fontSize"], 20);

        app.move_sensor(&id, Zone::Left, 0).expect("moved");
        assert_eq!(saved(&app).left[0].id, id);
        app.remove_sensor(&id).expect("removed");
        assert!(saved(&app).find(&id).is_none());
        assert!(app.remove_sensor(&id).is_err());
        app.store.shutdown();
    }
}
