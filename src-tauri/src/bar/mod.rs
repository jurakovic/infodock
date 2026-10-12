//! The bar (SPEC §4.1, §5): a native window on its own thread. [`BarController`] is the handle
//! the rest of the app uses; the logic in `thread.rs` runs on the bar thread.

pub mod theme;
mod thread;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};

use tauri::{AppHandle, Manager};

use crate::config::ConfigStore;
use crate::config::model::Settings;
use crate::platform::{self, AppBar, DockRequest, Platform, UiWaker};
use thread::BarLogic;

/// Work for the bar thread, queued by [`BarController`].
enum Command {
    Settings(Box<Settings>),
}

/// Cross-thread handle to the bar. Managed as Tauri state.
pub struct BarController {
    appbar: Arc<dyn AppBar>,
    waker: UiWaker,
    commands: Sender<Command>,
    visible: AtomicBool,
    app: AppHandle,
}

impl BarController {
    /// Starts the bar thread and docks the bar. Call after the [`ConfigStore`] and the platform
    /// are managed.
    pub fn create(app: &AppHandle) -> Result<Arc<Self>, Box<dyn std::error::Error>> {
        let store = app.state::<ConfigStore>();
        let settings = store.get().settings;
        let platform = Arc::clone(app.state::<Arc<dyn Platform>>().inner());
        let (commands, queue) = mpsc::channel();
        let initial = settings.clone();
        let handle = app.clone();
        let thread = platform::start_bar_thread(Box::new(move || {
            Box::new(BarLogic::new(initial, platform, queue, handle))
        }))?;

        let controller = Arc::new(Self {
            appbar: thread.appbar,
            waker: thread.waker,
            commands,
            visible: AtomicBool::new(true),
            app: app.clone(),
        });
        controller.apply(&settings);

        let weak = Arc::downgrade(&controller);
        store.on_change(Box::new(move |old, new| {
            let Some(c) = weak.upgrade() else {
                return;
            };
            if DockRequest::from_settings(&old.settings, true)
                != DockRequest::from_settings(&new.settings, true)
            {
                c.apply(&new.settings);
            }
            c.send(Command::Settings(Box::new(new.settings.clone())));
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
            self.apply(&self.app.state::<ConfigStore>().get().settings);
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

    /// Releases the reserved screen space. Call on exit.
    pub fn release(&self) {
        self.appbar.release();
    }
}
