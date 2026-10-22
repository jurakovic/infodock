//! The running app: state shared by all threads, bar visibility, and quitting.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use crate::bar::{BarController, BarHost};
use crate::config::exchange::{
    self, ExportScope, ImportFormat, ImportMode, ImportReport, Incoming,
};
use crate::config::legacy::LegacySensor;
use crate::config::model::{Settings, Zone};
use crate::config::{ConfigError, ConfigStore};
use crate::platform::Platform;
use crate::providers::Providers;
use crate::sampler::{
    BATTERY_EVERY, Change, ChangeSink, Delivery, Pending, Sample, Sampler, Subscriber, Topic,
    TopicRequest, VOLUMES_EVERY,
};
use crate::sensors::registry::registry;
use crate::sensors::{DefaultsCtx, Props, import, legacy, ops};
use crate::{settings, transfer};

/// How long a requested quit may take before the process ends anyway.
const QUIT_TIMEOUT: Duration = Duration::from_secs(5);

pub struct App {
    pub store: ConfigStore,
    pub platform: Arc<dyn Platform>,
    /// The skins folder (SPEC §11.1, §12.4).
    pub skins: PathBuf,
    bar: OnceLock<Arc<BarController>>,
    sampler: Sampler,
    /// The sampler thread until [`App::start_sampler`] starts it.
    pending: Mutex<Option<Pending>>,
    shut_down: AtomicBool,
}

impl App {
    pub fn new(store: ConfigStore, platform: Arc<dyn Platform>, skins: PathBuf) -> Self {
        let (sampler, pending) = Sampler::new();
        Self {
            store,
            platform,
            skins,
            bar: OnceLock::new(),
            sampler,
            pending: Mutex::new(Some(pending)),
            shut_down: AtomicBool::new(false),
        }
    }

    pub fn set_bar(&self, bar: Arc<BarController>) {
        let _ = self.bar.set(bar);
    }

    /// Starts the sampler thread, which delivers to the bar (SPEC §4.4), and the drive and
    /// battery lists to Settings while it watches them. Subscriptions made before it starts are
    /// kept.
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
            let for_settings = delivery
                .to
                .as_ref()
                .is_none_or(|to| *to == Subscriber::Settings);
            if for_settings {
                match delivery.sample.as_ref() {
                    Sample::Volumes(volumes) => settings::volumes_changed(volumes.clone()),
                    Sample::Batteries(names) => settings::batteries_changed(names.clone()),
                    _ => {}
                }
            }
            if let Some(bar) = bar.upgrade() {
                bar.deliver(delivery);
            }
        });
        let sampler = self.sampler.clone();
        let providers = Providers::new(Arc::clone(&self.platform))
            .with_changes(ChangeSink::new(move |change| sampler.changed(change)));
        if let Err(e) = pending.start(providers, deliver) {
            tracing::error!("the sampler could not start: {e}");
        }
    }

    /// Settings receives the drive list while `volumes` and the battery list while
    /// `batteries` (a form with such a field is open) from the sampler, so the main thread never
    /// waits for a drive or a battery, and the lists follow drives that arrive or go (SPEC
    /// §9.12). The newest sample comes at once when the topic is already read for the bar.
    pub fn watch_lists(&self, volumes: bool, batteries: bool) {
        let mut topics = Vec::new();
        if volumes {
            topics.push(TopicRequest {
                topic: Topic::Volumes,
                interval: VOLUMES_EVERY,
            });
        }
        if batteries {
            topics.push(TopicRequest {
                topic: Topic::Batteries,
                interval: BATTERY_EVERY,
            });
        }
        self.sampler.subscribe(Subscriber::Settings, topics);
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

    /// Gives the bar the keyboard focus if it is shown (hotkey, SPEC §14.4).
    pub fn focus_bar(&self) {
        if let Some(bar) = self.bar.get() {
            bar.focus();
        }
    }

    /// Ends the event loop from any thread; the main thread then cleans up. If the loop does not
    /// end in time (or is gone), the process ends here after the same cleanup.
    fn request_quit(self: &Arc<Self>) {
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
/// and Settings. What the machine says for machine-derived props is asked first, outside the
/// store's lock, since it may list the drives.
impl App {
    /// Adds a sensor; see [`ops::add`]. Returns its id.
    pub fn add_sensor(
        &self,
        kind: &str,
        zone: Zone,
        index: usize,
        props: &Props,
    ) -> Result<String, ConfigError> {
        let none = Props::new();
        let machine = ops::machine_props(registry(), &self.defaults(), kind, &none, props);
        let (id, _) = self
            .store
            .update(|c| ops::add(c, registry(), &machine, kind, zone, index, props))?;
        Ok(id)
    }

    /// Adds `sensors` (types and props) in this order at `index` of `zone`, in one change;
    /// returns their ids.
    pub fn add_sensors(
        &self,
        zone: Zone,
        index: usize,
        sensors: &[(&str, Props)],
    ) -> Result<Vec<String>, ConfigError> {
        let none = Props::new();
        let machine: Vec<Props> = sensors
            .iter()
            .map(|(kind, props)| {
                ops::machine_props(registry(), &self.defaults(), kind, &none, props)
            })
            .collect();
        let (ids, _) = self.store.update(|c| {
            sensors
                .iter()
                .zip(&machine)
                .enumerate()
                .map(|(i, ((kind, props), machine))| {
                    ops::add(c, registry(), machine, kind, zone, index + i, props)
                })
                .collect::<Result<Vec<_>, _>>()
        })?;
        Ok(ids)
    }

    /// Where machine-derived props come from (SPEC §11.2).
    fn defaults(&self) -> DefaultsCtx<'_> {
        DefaultsCtx {
            platform: self.platform.as_ref(),
        }
    }

    /// Saves `changes` to sensor `id` and ends its preview.
    pub fn update_sensor(&self, id: &str, changes: &Props) -> Result<(), ConfigError> {
        let result = self.save_sensor(id, changes);
        self.end_preview(id);
        result
    }

    /// Saves `changes` to sensor `id`; see [`ops::update`].
    fn save_sensor(&self, id: &str, changes: &Props) -> Result<(), ConfigError> {
        let config = self.store.get();
        let machine = ops::machine_props_of(&config, registry(), &self.defaults(), id, changes);
        self.store
            .update(|c| ops::update(c, registry(), &machine, id, changes))
            .map(|_| ())
    }

    /// Shows sensor `id` with `changes` on the bar without saving (SPEC §6.7).
    pub fn preview_sensor(&self, id: &str, changes: &Props) -> Result<(), ConfigError> {
        let config = self.store.get();
        let machine = ops::machine_props_of(&config, registry(), &self.defaults(), id, changes);
        let shown = ops::preview(&config, registry(), &machine, id, changes)?;
        if let Some(bar) = self.bar.get() {
            bar.preview(shown);
        }
        Ok(())
    }

    /// A skin pack was installed or removed: the bar draws its skin again (SPEC §12.4).
    pub fn skins_changed(&self) {
        if let Some(bar) = self.bar.get() {
            bar.skins_changed();
        }
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

/// Layout exchange (SPEC §11.3). An import saves through the config store, which updates the bar
/// and Settings.
impl App {
    /// The config, or its layout, as the text of an exchange file.
    pub fn export(&self, scope: ExportScope) -> serde_json::Result<String> {
        exchange::export(&self.store.get(), scope)
    }

    /// What the sensors of a legacy layout bring, converted by their types (SPEC §11.4).
    pub fn legacy_incoming(&self, sensors: Vec<LegacySensor>) -> Incoming {
        legacy::incoming(registry(), &self.defaults(), sensors)
    }

    /// Adds what an exchange or legacy file brings to the layout; see [`import::import`].
    pub fn import(
        &self,
        incoming: Incoming,
        mode: ImportMode,
    ) -> Result<ImportReport, ConfigError> {
        // Checked before the store is locked: the machine may be asked for props (SPEC §4.5).
        let prepared = import::prepare(registry(), &self.defaults(), incoming);
        let (report, _) = self
            .store
            .update(|c| Ok(import::import(c, prepared, mode)))?;
        Ok(report)
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
        let app = Arc::clone(&self.0);
        let _ = slint::invoke_from_event_loop(move || settings::activate(&app));
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

    fn add_sensors(&self, zone: Zone, index: usize, sensors: Vec<(&'static str, Props)>) {
        if let Err(e) = self.0.add_sensors(zone, index, &sensors) {
            tracing::warn!("could not add the dropped sensors: {e}");
        }
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
        self.0
            .sampler
            .subscribe(Subscriber::Sensor(id.to_owned()), topics);
    }

    fn redock(&self) {
        if let Some(bar) = self.0.bar.get() {
            bar.redock();
        }
    }

    fn resumed(&self) {
        self.0.sampler.resume();
    }

    fn changed(&self, change: Change) {
        self.0.sampler.changed(change);
    }

    /// Saved without ending a preview, which goes on showing what Settings edits.
    fn update_sensor(&self, id: &str, changes: Props) {
        if let Err(e) = self.0.save_sensor(id, &changes) {
            tracing::warn!("sensor {id} could not save {changes:?}: {e}");
        }
    }

    fn import_layout(&self, format: ImportFormat) {
        let app = Arc::clone(&self.0);
        let _ = slint::invoke_from_event_loop(move || {
            transfer::import(&app, &settings::dialogs(), format);
        });
    }

    fn export_layout(&self) {
        let app = Arc::clone(&self.0);
        let _ = slint::invoke_from_event_loop(move || {
            transfer::export(&app, &settings::dialogs());
        });
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
        let app = App::new(store, crate::platform::current(), dir.path().join("skins"));
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

    #[test]
    fn imports_save_through_the_store() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (store, _) = ConfigStore::open(dir.path(), "test");
        let app = App::new(store, crate::platform::current(), dir.path().join("skins"));
        let layout = app.export(ExportScope::Layout).expect("exported");
        let incoming = exchange::parse(&layout).expect("parsed");
        let report = app.import(incoming, ImportMode::Merge).expect("imported");
        assert_eq!(report.added, 6);
        assert_eq!(app.store.get().layout.left.len(), 6);
        let incoming = exchange::parse(&layout).expect("parsed");
        app.import(incoming, ImportMode::Replace).expect("imported");
        assert_eq!(app.store.get().layout.left.len(), 3);
        app.store.shutdown();
    }
}
