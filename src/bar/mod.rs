//! The bar (SPEC §4.1, §5): a native window on its own thread. [`BarController`] is the handle
//! the rest of the app uses; the logic in `thread.rs` runs on the bar thread.

pub mod anim;
mod hint;
mod icons;
mod interact;
mod layout;
mod menus;
mod paint;
mod placement;
mod sensors;
pub mod theme;
mod thread;
mod tray;

pub(crate) use interact::clicks_allowed;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, PoisonError};

use crate::config::Config;
use crate::config::ConfigStore;
use crate::config::model::{BarMode, SensorInstance, Settings, Zone};
use crate::platform::{self, AppBar, DockRequest, Platform, PlatformError, TrayIcon, UiWaker};
use crate::sampler::{Delivery, TopicRequest};
use thread::BarLogic;

/// What the bar asks of the rest of the app. Called on the bar thread.
pub trait BarHost: Send + Sync {
    /// The OS asked the app to close (graceful `taskkill`, Restart Manager).
    fn quit(&self);
    /// A second launch asked this instance to show itself (SPEC §6.5).
    fn activate(&self);
    fn bar_visible(&self) -> bool;
    /// Shows or hides the bar (tray click, tray menu).
    fn toggle_bar(&self);
    /// Opens Settings on `route`, e.g. `general` or `sensor/<id>` (SPEC §4.1).
    fn open_settings(&self, route: &str);
    /// Changes and saves settings (tray menu: Lock, Popups).
    fn change_settings(&self, change: Box<dyn FnOnce(&mut Settings) + Send>);
    /// Adds a sensor of type `kind` with its defaults; returns its id (SPEC §4.5).
    fn add_sensor(&self, kind: &str, zone: Zone, index: usize) -> Option<String>;
    fn remove_sensor(&self, id: &str);
    /// Moves a sensor; `index` counts with it already taken out (SPEC §6.2).
    fn move_sensor(&self, id: &str, zone: Zone, index: usize);
    /// Replaces the topics sensor `id` receives samples of; empty to stop (SPEC §4.4).
    fn subscribe(&self, id: &str, topics: Vec<TopicRequest>);
    /// Docks the bar as the settings say now; after a docked bar's edge changed, the bar
    /// thread calls this once it has slid out (SPEC §5.5).
    fn redock(&self);
    /// The system resumed from sleep: the sampler reads every topic again (SPEC §4.4).
    fn resumed(&self);
    /// Stores sensor `id`'s runtime state in the config (SPEC §11.2), e.g. the uptime record.
    fn set_sensor_state(&self, id: &str, state: crate::sensors::State);
}

/// Work for the bar thread, queued by [`BarController`].
enum Command {
    /// The config changed: settings, layout or sensor state.
    Config(Box<Config>),
    /// Show a sensor with other props until the preview ends (SPEC §6.7).
    Preview(Box<SensorInstance>),
    EndPreview(String),
    /// A sample from the sampler thread (SPEC §4.4).
    Sample(Delivery),
}

/// Cross-thread handle to the bar.
pub struct BarController {
    appbar: Arc<dyn AppBar>,
    tray: Arc<dyn TrayIcon>,
    waker: UiWaker,
    commands: Sender<Command>,
    visible: AtomicBool,
    /// The latest settings, for re-docking when visibility changes.
    settings: Mutex<Settings>,
}

impl BarController {
    /// Starts the bar thread and docks the bar; follows `store` from then on.
    pub fn create(
        store: &ConfigStore,
        platform: Arc<dyn Platform>,
        host: Arc<dyn BarHost>,
    ) -> Result<Arc<Self>, PlatformError> {
        let config = store.get();
        let settings = config.settings.clone();
        let (commands, queue) = mpsc::channel();
        let thread = platform::start_bar_thread(Box::new(move |waker| {
            Box::new(BarLogic::new(config, platform, queue, host, waker))
        }))?;

        let controller = Arc::new(Self {
            appbar: thread.appbar,
            tray: thread.tray,
            waker: thread.waker,
            commands,
            visible: AtomicBool::new(true),
            settings: Mutex::new(settings.clone()),
        });
        controller.apply(&settings);

        let weak = Arc::downgrade(&controller);
        store.on_change(Box::new(move |old, new| {
            let Some(c) = weak.upgrade() else {
                return;
            };
            *c.settings.lock().unwrap_or_else(PoisonError::into_inner) = new.settings.clone();
            // A docked bar slides out before it changes edges; the bar thread redocks it then.
            let (o, n) = (&old.settings.bar, &new.settings.bar);
            let slides = o.edge != n.edge && o.mode == BarMode::Docked && n.mode == BarMode::Docked;
            if !slides
                && DockRequest::from_settings(&old.settings, true)
                    != DockRequest::from_settings(&new.settings, true)
            {
                c.apply(&new.settings);
            }
            c.send(Command::Config(Box::new(new.clone())));
        }));

        let appbar = Arc::clone(&controller.appbar);
        crate::logging::on_fatal_panic(Box::new(move || appbar.release()));
        Ok(controller)
    }

    fn send(&self, command: Command) {
        if self.commands.send(command).is_ok() {
            self.waker.wake();
        }
    }

    fn apply(&self, settings: &Settings) {
        self.appbar.configure(DockRequest::from_settings(
            settings,
            self.visible.load(Ordering::SeqCst),
        ));
    }

    pub fn is_visible(&self) -> bool {
        self.visible.load(Ordering::SeqCst)
    }

    /// Shows or hides the bar; a hidden bar reserves no space.
    pub fn set_visible(&self, visible: bool) {
        if self.visible.swap(visible, Ordering::SeqCst) != visible {
            let settings = self
                .settings
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            self.apply(&settings);
        }
    }

    pub fn toggle_visible(&self) {
        self.set_visible(!self.is_visible());
    }

    /// Shows the bar if hidden and brings it to the front (hotkey).
    pub fn bring_to_front(&self) {
        self.set_visible(true);
        self.appbar.raise();
    }

    /// Shows `sensor` (matched by id) instead of its saved entry until [`Self::end_preview`].
    pub fn preview(&self, sensor: SensorInstance) {
        self.send(Command::Preview(Box::new(sensor)));
    }

    pub fn end_preview(&self, id: &str) {
        self.send(Command::EndPreview(id.to_owned()));
    }

    /// Passes a sample to the sensors on the bar (sampler thread).
    pub fn deliver(&self, delivery: Delivery) {
        self.send(Command::Sample(delivery));
    }

    /// Docks the bar as the latest settings say.
    pub fn redock(&self) {
        let settings = self
            .settings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        self.apply(&settings);
    }

    /// Releases the reserved screen space. Call on exit.
    pub fn release(&self) {
        self.appbar.release();
        self.tray.remove();
    }
}
