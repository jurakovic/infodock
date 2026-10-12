//! The Settings window (SPEC §10.7), a Slint window on the main thread. It is created when opened
//! and dropped when closed, so a closed Settings window costs no memory.
//!
//! Controls bind two-way to the `SettingsState` global. Each change is saved as a settings patch
//! through the config store; the store's change listener then refreshes the window, so changes
//! made elsewhere (tray, hotkey) show up too.

mod model;

use std::cell::RefCell;
use std::sync::Arc;

use serde_json::Value;
use slint::{CloseRequestResponse, ComponentHandle, ModelRc, SharedString, VecModel};

use crate::app::App;
use crate::config::model::Settings;
use crate::config::patch::apply_settings_patch;
use crate::config::{Config, ConfigError};
use crate::{hotkey, ui};

thread_local! {
    static WINDOW: RefCell<Option<Open>> = const { RefCell::new(None) };
}

/// The open window and what its controls were filled from.
struct Open {
    window: ui::SettingsWindow,
    /// Monitor keys in the order of the monitor list.
    monitor_keys: Vec<String>,
}

/// Why a change was not saved.
enum SaveError {
    /// The hotkey could not be registered; nothing was saved (SPEC §6.5).
    Hotkey(String),
    Config(ConfigError),
}

/// Opens Settings on `route` (e.g. `bar`), or switches the open window to it and brings it to
/// the front. Main thread only.
pub fn open(app: &Arc<App>, route: &str) {
    let page = model::page_index(route);
    let existing = WINDOW.with(|w| w.borrow().as_ref().map(|open| open.window.clone_strong()));
    let window = match existing {
        Some(window) => window,
        None => match create(app) {
            Ok(window) => window,
            Err(e) => {
                tracing::warn!("could not open settings: {e}");
                return;
            }
        },
    };
    window.global::<ui::SettingsState>().set_page(page);
    if let Err(e) = window.show() {
        tracing::warn!("could not show settings: {e}");
        return;
    }
    focus(&window);
}

/// Refreshes the open window whenever the config changes (from any thread).
pub fn watch(app: &Arc<App>) {
    let a = Arc::clone(app);
    app.store.on_change(Box::new(move |old, new| {
        let (old, new) = (old.settings.clone(), new.settings.clone());
        let a = Arc::clone(&a);
        let _ = slint::invoke_from_event_loop(move || {
            WINDOW.with(|w| {
                if let Some(open) = w.borrow_mut().as_mut() {
                    fill(open, &a, &new, Some(&old));
                }
            });
        });
    }));
}

fn create(app: &Arc<App>) -> Result<ui::SettingsWindow, slint::PlatformError> {
    let window = ui::SettingsWindow::new()?;
    crate::install_i18n!(window);
    let state = window.global::<ui::SettingsState>();

    let a = Arc::clone(app);
    state.on_apply_hotkey(move |text| {
        let text = text.trim().to_owned();
        let unchanged = a.store.get().settings.hotkey == text;
        if unchanged && hotkey::error().is_none() {
            return;
        }
        save(&a, model::patch("hotkey", Value::String(text)));
    });
    let a = Arc::clone(app);
    state.on_set_flag(move |path, value| save(&a, model::patch(&path, Value::Bool(value))));
    let a = Arc::clone(app);
    state.on_set_number(move |path, value| save(&a, model::patch(&path, Value::from(value))));
    let a = Arc::clone(app);
    state.on_reset(move |path| save(&a, model::patch(&path, Value::Null)));
    let a = Arc::clone(app);
    state.on_set_choice(move |path, index| {
        let value = model::choice(&path)
            .and_then(|c| usize::try_from(index).ok().and_then(|i| c.values.get(i)));
        if let Some(value) = value {
            save(&a, model::patch(&path, Value::from(*value)));
        }
    });
    let a = Arc::clone(app);
    state.on_set_monitor(move |index| {
        let key = WINDOW.with(|w| {
            let w = w.borrow();
            let open = w.as_ref()?;
            usize::try_from(index)
                .ok()
                .and_then(|i| open.monitor_keys.get(i).cloned())
        });
        if let Some(key) = key {
            save(&a, model::patch("bar.monitor", Value::String(key)));
        }
    });

    window.window().on_close_requested(|| {
        // Drop the window once it is hidden, which frees what it allocated.
        let _ = slint::invoke_from_event_loop(|| {
            WINDOW.with(|w| w.borrow_mut().take());
        });
        CloseRequestResponse::HideWindow
    });

    let mut open = Open {
        window: window.clone_strong(),
        monitor_keys: Vec::new(),
    };
    fill(&mut open, app, &app.store.get().settings, None);
    WINDOW.with(|w| *w.borrow_mut() = Some(open));
    Ok(window)
}

/// Copies `settings` into the window. `previous` is what the window was filled from before; the
/// hotkey field keeps what the user is typing unless the saved hotkey itself changed.
fn fill(open: &mut Open, app: &App, settings: &Settings, previous: Option<&Settings>) {
    let state = open.window.global::<ui::SettingsState>();
    let json = serde_json::to_value(settings).unwrap_or_default();
    let choice = |path: &str| model::choice(path).map_or(0, |c| model::choice_index(&json, c));

    if previous.is_none_or(|p| p.hotkey != settings.hotkey) {
        state.set_hotkey(settings.hotkey.as_str().into());
    }
    state.set_hotkey_error(hotkey::error().unwrap_or_default().into());
    state.set_hotkey_action(choice("hotkeyAction"));
    state.set_show_tray_icon(settings.show_tray_icon);
    state.set_locked(settings.locked);
    state.set_popups_enabled(settings.popups_enabled);
    state.set_clicks_when_locked(choice("clicksWhenLocked"));
    state.set_sensor_spacing(settings.sensor_spacing as f32);

    state.set_edge(choice("bar.edge"));
    let monitors = model::monitor_choices(&settings.bar.monitor, &app.platform.monitors());
    let labels: Vec<SharedString> = monitors.labels.iter().map(SharedString::from).collect();
    state.set_monitors(ModelRc::new(VecModel::from(labels)));
    state.set_monitor(i32::try_from(monitors.current).unwrap_or(0));
    open.monitor_keys = monitors.keys;
    state.set_reserve_space(settings.bar.reserve_space);
    state.set_z_order(choice("bar.zOrder"));
    state.set_hide_on_fullscreen(settings.bar.hide_on_fullscreen);
    state.set_height(settings.bar_height() as f32);
    state.set_theme_height(
        crate::config::model::ThemeDefaults::of(settings.theme).bar_height as i32,
    );
    state.set_height_customised(settings.bar.height.is_some());
}

/// Saves a settings patch and shows the outcome. A changed hotkey is registered first; if that
/// fails, nothing is saved (SPEC §6.5).
fn save(app: &Arc<App>, patch: Value) {
    let result = apply(app, &patch);
    WINDOW.with(|w| {
        let mut w = w.borrow_mut();
        let Some(open) = w.as_mut() else {
            return;
        };
        let state = open.window.global::<ui::SettingsState>();
        match &result {
            Ok(_) => state.set_save_error(SharedString::new()),
            Err(SaveError::Hotkey(e)) => state.set_hotkey_error(e.as_str().into()),
            Err(SaveError::Config(e)) => {
                tracing::warn!("could not save a settings change: {e}");
                state.set_save_error(e.to_string().into());
            }
        }
        if matches!(result, Err(SaveError::Config(_))) {
            // Put the controls back to what is saved; a rejected hotkey stays in its field.
            let settings = app.store.get().settings;
            fill(open, app, &settings, None);
        }
    });
}

fn apply(app: &App, patch: &Value) -> Result<Config, SaveError> {
    let current = app.store.get().settings;
    let next = apply_settings_patch(&current, patch).map_err(SaveError::Config)?;
    if next.hotkey != current.hotkey {
        hotkey::set(&next.hotkey).map_err(SaveError::Hotkey)?;
    }
    app.store
        .update(|c| {
            c.settings = apply_settings_patch(&c.settings, patch)?;
            Ok(())
        })
        .map(|(_, config)| config)
        .map_err(SaveError::Config)
}

/// Brings the window to the front, e.g. when a second launch asked for Settings.
fn focus(window: &ui::SettingsWindow) {
    use slint::winit_030::WinitWindowAccessor;
    window.window().with_winit_window(|w| {
        w.set_minimized(false);
        w.focus_window();
    });
}

#[cfg(test)]
mod screenshots {
    use std::path::Path;
    use std::rc::Rc;
    use std::sync::Arc;

    use slint::platform::software_renderer::{
        MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
    };
    use slint::platform::{Platform, WindowAdapter};
    use slint::{ComponentHandle, PhysicalSize};

    use super::{Open, fill};
    use crate::app::App;
    use crate::config::ConfigStore;
    use crate::ui;

    struct Offscreen(Rc<MinimalSoftwareWindow>);

    impl Platform for Offscreen {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
            Ok(self.0.clone())
        }
    }

    fn write_png(path: &Path, width: u32, height: u32, pixels: &[PremultipliedRgbaColor]) {
        let file = std::fs::File::create(path).expect("screenshot file");
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let bytes: Vec<u8> = pixels
            .iter()
            .flat_map(|p| [p.red, p.green, p.blue, p.alpha])
            .collect();
        encoder
            .write_header()
            .and_then(|mut w| w.write_image_data(&bytes))
            .expect("PNG written");
    }

    /// Renders each Settings page with the software renderer, for reviewing the layout without
    /// running the app. CI uploads the PNGs.
    #[test]
    #[ignore = "writes PNGs to target/screenshots; run with --ignored"]
    fn render_settings_pages() {
        let (width, height) = (760, 560);
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        slint::platform::set_platform(Box::new(Offscreen(window.clone())))
            .expect("the first Slint use in this process");
        let dir = tempfile::tempdir().expect("temp dir");
        let (store, _) = ConfigStore::open(dir.path(), "test");
        let app = Arc::new(App::new(store, crate::platform::current()));

        let ui = ui::SettingsWindow::new().expect("the window builds");
        crate::install_i18n!(ui);
        let mut open = Open {
            window: ui.clone_strong(),
            monitor_keys: Vec::new(),
        };
        fill(&mut open, &app, &app.store.get().settings, None);
        ui.show().expect("shown");
        window.set_size(PhysicalSize::new(width, height));

        let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/screenshots");
        std::fs::create_dir_all(&out).expect("screenshot dir");
        for (index, page) in super::model::PAGES.iter().enumerate() {
            ui.global::<ui::SettingsState>().set_page(index as i32);
            window.request_redraw();
            let mut pixels = vec![PremultipliedRgbaColor::default(); (width * height) as usize];
            window.draw_if_needed(|renderer| {
                renderer.render(&mut pixels, width as usize);
            });
            write_png(
                &out.join(format!("settings-{page}.png")),
                width,
                height,
                &pixels,
            );
        }
        app.store.shutdown();
    }
}
