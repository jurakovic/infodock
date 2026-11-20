//! The bar (INTERNALS.md, The bar): a native window on its own thread. [`BarController`] is the
//! handle the rest of the app uses; the logic in `thread/` runs on the bar thread.

mod drop;
mod focus;
mod hint;
mod icons;
mod interact;
mod layout;
mod menus;
mod paint;
mod placement;
mod popup;
mod resize;
mod sensors;
mod shadow;
mod thread;
mod tray;

pub use paint::skin_preview;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};

use crate::config::Config;
use crate::config::ConfigStore;
use crate::config::exchange::ImportFormat;
use crate::config::model::{SensorInstance, Settings, Zone};
use crate::config::store::{Newest, Revision};
use crate::look::anim::slides_out;
use crate::platform::{
    self, AppBar, DockRequest, OsAppearance, Platform, PlatformError, TrayIcon, UiWaker,
};
use crate::sampler::{Change, Delivery, TopicRequest};
use thread::BarLogic;

/// Where and how the bar should sit, from the settings.
fn dock_request(s: &Settings, visible: bool) -> DockRequest {
    DockRequest {
        visible,
        edge: s.bar.edge,
        mode: s.bar.mode,
        monitor: s.bar.monitor.clone(),
        height: s.bar_height(),
        reserve_space: s.bar.reserve_space,
        z_order: s.bar.z_order,
        hide_on_fullscreen: s.bar.hide_on_fullscreen,
        over_desktop: s.bar.over_desktop,
        floating: s.bar.floating,
    }
}

/// What the bar asks of the rest of the app. Called on the bar thread.
pub trait BarHost: Send + Sync {
    /// The OS asked the app to close (graceful `taskkill`, Restart Manager).
    fn quit(&self);
    /// A second launch asked this instance to show itself.
    fn activate(&self);
    fn bar_visible(&self) -> bool;
    /// Shows or hides the bar (tray click, tray menu).
    fn toggle_bar(&self);
    /// Opens Settings on `route`, e.g. `general` or `sensor/<id>`.
    fn open_settings(&self, route: &str);
    /// Changes and saves settings (tray menu: Lock, Popups).
    fn change_settings(&self, change: Box<dyn FnOnce(&mut Settings) + Send>);
    /// Adds a sensor of type `kind` with its defaults; returns its id.
    fn add_sensor(&self, kind: &str, zone: Zone, index: usize) -> Option<String>;
    /// Adds sensors of the given types and props in this order at `index` of `zone`, without
    /// opening their Properties (a drop).
    fn add_sensors(
        &self,
        zone: Zone,
        index: usize,
        sensors: Vec<(&'static str, crate::sensors::Props)>,
    );
    fn remove_sensor(&self, id: &str);
    /// Moves a sensor; `index` counts with it already taken out.
    fn move_sensor(&self, id: &str, zone: Zone, index: usize);
    /// Replaces the topics sensor `id` receives samples of; empty to stop.
    fn subscribe(&self, id: &str, topics: Vec<TopicRequest>);
    /// Docks the bar as the settings say now; after a docked bar's edge changed, the bar
    /// thread calls this once it has slid out.
    fn redock(&self);
    /// The system resumed from sleep: the sampler reads every topic again.
    fn resumed(&self);
    /// Something changed, e.g. a drive arrived or went: the sampler reads the topics that
    /// follow it again. Called from any thread.
    fn changed(&self, change: Change);
    /// Stores sensor `id`'s runtime state in the config, e.g. the uptime record.
    fn set_sensor_state(&self, id: &str, state: crate::sensors::State);
    /// Saves props sensor `id` changed itself (`changes`: the changed keys), e.g. what a
    /// Battery shows after a click.
    fn update_sensor(&self, id: &str, changes: crate::sensors::Props);
    /// Imports a layout file as the user picks it (Tools menu): the picker,
    /// the *Replace* / *Merge* question and the report, on the main thread.
    fn import_layout(&self, format: ImportFormat);
    /// Exports the layout or the whole config to a file the user picks.
    fn export_layout(&self);
    /// The OS appearance changed (light or dark, accent colour, transparency effects): Settings
    /// shows what depends on it, e.g. whether a material can be chosen.
    fn appearance_changed(&self, appearance: OsAppearance);
}

/// Work for the bar thread, queued by [`BarController`].
enum Command {
    /// The config changed: settings, layout or sensor state. One older than the bar's is dropped.
    Config(Revision, Box<Config>),
    /// Show a sensor with other props until the preview ends.
    Preview(Box<SensorInstance>),
    EndPreview(String),
    /// A sample from the sampler thread.
    Sample(Delivery),
    /// The bar was hidden (tray, hotkey): its hint and popup close.
    Hidden,
    /// The hotkey focuses the bar.
    Focus,
    /// A skin pack was installed or removed: the skin is drawn again.
    SkinsChanged,
}

/// Cross-thread handle to the bar.
pub struct BarController {
    appbar: Arc<dyn AppBar>,
    tray: Arc<dyn TrayIcon>,
    waker: UiWaker,
    commands: Sender<Command>,
    visible: AtomicBool,
    /// The latest settings, for re-docking when visibility changes.
    settings: Newest<Settings>,
}

impl BarController {
    /// Starts the bar thread and docks the bar; follows `store` from then on. Skin packs come
    /// from `skins`.
    pub fn create(
        store: &ConfigStore,
        platform: Arc<dyn Platform>,
        host: Arc<dyn BarHost>,
        skins: PathBuf,
    ) -> Result<Arc<Self>, PlatformError> {
        let (config, revision) = store.get_revised();
        let settings = config.settings.clone();
        let (commands, queue) = mpsc::channel();
        let thread = platform::start_bar_thread(Box::new(move |waker| {
            Box::new(BarLogic::new(
                (config, revision),
                platform,
                queue,
                host,
                waker,
                skins,
            ))
        }))?;

        let controller = Arc::new(Self {
            appbar: thread.appbar,
            tray: thread.tray,
            waker: thread.waker,
            commands,
            visible: AtomicBool::new(true),
            settings: Newest::new(settings.clone(), revision),
        });
        controller.apply(&settings);

        let weak = Arc::downgrade(&controller);
        store.on_change(Box::new(move |_, new, revision| {
            let Some(c) = weak.upgrade() else {
                return;
            };
            // Against the settings applied last, which a change notified late is older than.
            c.settings
                .offer(revision, new.settings.clone(), |old, settings| {
                    // A docked bar slides out before it changes edges; the bar thread redocks it
                    // then.
                    if !slides_out(&old.bar, &settings.bar)
                        && dock_request(old, true) != dock_request(settings, true)
                    {
                        c.apply(settings);
                    }
                    c.send(Command::Config(revision, Box::new(new.clone())));
                });
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
        self.appbar
            .configure(dock_request(settings, self.visible.load(Ordering::SeqCst)));
    }

    pub fn is_visible(&self) -> bool {
        self.visible.load(Ordering::SeqCst)
    }

    /// Shows or hides the bar; a hidden bar reserves no space.
    pub fn set_visible(&self, visible: bool) {
        if self.visible.swap(visible, Ordering::SeqCst) != visible {
            self.apply(&self.settings.get());
            if !visible {
                self.send(Command::Hidden);
            }
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

    /// Gives the shown bar the keyboard focus (hotkey).
    pub fn focus(&self) {
        if self.is_visible() {
            self.send(Command::Focus);
        }
    }

    /// Shows `sensor` (matched by id) instead of its saved entry until [`Self::end_preview`].
    pub fn preview(&self, sensor: SensorInstance) {
        self.send(Command::Preview(Box::new(sensor)));
    }

    pub fn end_preview(&self, id: &str) {
        self.send(Command::EndPreview(id.to_owned()));
    }

    /// A skin pack was installed or removed: the bar reads its skin again.
    pub fn skins_changed(&self) {
        self.send(Command::SkinsChanged);
    }

    /// Passes a sample to the sensors on the bar (sampler thread).
    pub fn deliver(&self, delivery: Delivery) {
        self.send(Command::Sample(delivery));
    }

    /// Docks the bar as the latest settings say.
    pub fn redock(&self) {
        self.apply(&self.settings.get());
    }

    /// Releases the reserved screen space. Call on exit.
    pub fn release(&self) {
        self.appbar.release();
        self.tray.remove();
    }
}
