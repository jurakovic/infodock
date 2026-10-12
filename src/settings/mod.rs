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
    /// The installed font families, read once per window.
    families: Vec<String>,
    /// The font list as shown; index 0 is the theme default.
    fonts: Vec<String>,
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
    let page = match model::route(route) {
        model::Route::Page(index) => index as i32,
        // The picker and the properties form arrive in later steps of M1 S9.
        model::Route::New { .. } | model::Route::Sensor(_) => 2,
    };
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
    app.store.on_change(Box::new(move |_, new| {
        let new = new.settings.clone();
        let a = Arc::clone(&a);
        let _ = slint::invoke_from_event_loop(move || {
            WINDOW.with(|w| {
                if let Some(open) = w.borrow_mut().as_mut() {
                    fill(open, &a, &new);
                }
            });
        });
    }));
}

fn create(app: &Arc<App>) -> Result<ui::SettingsWindow, slint::PlatformError> {
    let window = ui::SettingsWindow::new()?;
    crate::install_i18n!(window);
    connect(&window, app);
    record_hotkey(&window, app);

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
        families: app.platform.font_families(),
        fonts: Vec::new(),
    };
    fill(&mut open, app, &app.store.get().settings);
    WINDOW.with(|w| *w.borrow_mut() = Some(open));
    Ok(window)
}

/// Handles the window's callbacks: saving changes and converting colours.
fn connect(window: &ui::SettingsWindow, app: &Arc<App>) {
    let state = window.global::<ui::SettingsState>();
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
    state.on_to_color(|text| {
        crate::platform::Color::from_hex(text.trim()).map_or(slint::Color::default(), |c| {
            slint::Color::from_argb_f32(1.0, c.r, c.g, c.b)
        })
    });
    let a = Arc::clone(app);
    state.on_set_color(move |path, text| {
        match crate::config::model::normalize_color(text.trim().to_owned()) {
            Some(color) => save(&a, model::patch(&path, Value::String(color))),
            None => WINDOW.with(|w| {
                if let Some(open) = w.borrow_mut().as_mut() {
                    let state = open.window.global::<ui::SettingsState>();
                    state.set_save_error(crate::i18n::t("settings.color.invalid").into());
                    // Back to the saved colour.
                    let settings = a.store.get().settings;
                    fill(open, &a, &settings);
                }
            }),
        }
    });
    let a = Arc::clone(app);
    state.on_set_font(move |path, index| {
        let family = WINDOW.with(|w| {
            let w = w.borrow();
            let open = w.as_ref()?;
            let index = usize::try_from(index).ok().filter(|i| *i > 0)?;
            open.fonts.get(index).cloned()
        });
        save(
            &a,
            model::patch(&path, family.map_or(Value::Null, Value::String)),
        );
    });
    let a = Arc::clone(app);
    state.on_set_font_style(move |path, index| {
        let style = usize::try_from(index)
            .ok()
            .and_then(|i| i.checked_sub(1))
            .and_then(|i| model::FONT_STYLES.get(i));
        save(
            &a,
            model::patch(&path, style.map_or(Value::Null, |s| Value::from(*s))),
        );
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
}

/// Copies `settings` into the window.
fn fill(open: &mut Open, app: &App, settings: &Settings) {
    let state = open.window.global::<ui::SettingsState>();
    let json = serde_json::to_value(settings).unwrap_or_default();
    let choice = |path: &str| model::choice(path).map_or(0, |c| model::choice_index(&json, c));

    state.set_hotkey(settings.hotkey.as_str().into());
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

    let defaults = crate::config::model::ThemeDefaults::of(settings.theme);
    let strings = |items: &[String]| {
        let items: Vec<SharedString> = items.iter().map(SharedString::from).collect();
        ModelRc::new(VecModel::from(items))
    };
    let style_id = |style| {
        serde_json::to_value(style)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default()
    };

    // Sensors (SPEC §10.4)
    let sensors = &settings.sensors;
    state.set_sensor_text_color(sensors.text_color.as_str().into());
    // Both font settings share one list; a configured family that is not installed is added.
    open.fonts = model::font_labels(
        &defaults.sensor_font,
        &open.families,
        sensors.font.as_deref().or(settings.popups.font.as_deref()),
    );
    if let Some(popup_font) = settings.popups.font.as_deref()
        && !open.fonts.iter().any(|f| f == popup_font)
    {
        open.fonts.push(popup_font.to_owned());
    }
    state.set_fonts(strings(&open.fonts));
    state.set_sensor_font(model::font_index(&open.fonts, sensors.font.as_deref()));
    state.set_sensor_font_styles(strings(&model::font_style_labels(&style_id(
        defaults.sensor_font_style,
    ))));
    let current_style = |style: Option<crate::config::model::FontStyle>| {
        model::font_style_index(style.map(style_id).as_deref())
    };
    state.set_sensor_font_style(current_style(sensors.font_style));
    state.set_inactive_mode(choice("sensors.inactiveMode"));
    state.set_inactive_color(sensors.inactive_color.as_str().into());
    state.set_inactive_opacity(sensors.inactive_opacity as f32);

    // Popups (SPEC §10.5)
    let popups = &settings.popups;
    let background = popups
        .background
        .as_deref()
        .unwrap_or(&defaults.popup_background);
    state.set_popup_background(background.into());
    state.set_popup_background_customised(popups.background.is_some());
    state.set_popup_background_default(defaults.popup_background.as_str().into());
    state.set_popup_opacity(popups.opacity.unwrap_or(defaults.popup_opacity) as f32);
    state.set_popup_opacity_customised(popups.opacity.is_some());
    state.set_popup_opacity_default(defaults.popup_opacity as i32);
    state.set_popup_text_color(popups.text_color.as_str().into());
    state.set_popup_strong_color(popups.strong_text_color.as_str().into());
    state.set_popup_font(model::font_index(&open.fonts, popups.font.as_deref()));
    state.set_popup_font_styles(strings(&model::font_style_labels(&style_id(
        defaults.popup_font_style,
    ))));
    state.set_popup_font_style(current_style(popups.font_style));
    state.set_popup_font_size(popups.font_size as f32);
    state.set_popup_shadow(popups.shadow);
    state.set_show_delay(popups.show_delay_ms as f32);
    state.set_hide_delay(popups.hide_delay_ms as f32);
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
            // Put the controls back to what is saved.
            let settings = app.store.get().settings;
            fill(open, app, &settings);
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

/// While `recording-hotkey` is set, the next key pressed with its modifiers becomes the hotkey
/// (SPEC §6.5); Esc or leaving the window cancels. The keys are read from winit as physical keys,
/// so the keyboard layout does not matter (Ctrl+Alt is AltGr on many layouts).
fn record_hotkey(window: &ui::SettingsWindow, app: &Arc<App>) {
    use slint::winit_030::winit::event::{ElementState, WindowEvent};
    use slint::winit_030::winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
    use slint::winit_030::{EventResult, WinitWindowAccessor};

    let app = Arc::clone(app);
    let weak = window.as_weak();
    let mut modifiers = ModifiersState::empty();
    window.window().on_winit_window_event(move |_, event| {
        let Some(window) = weak.upgrade() else {
            return EventResult::Propagate;
        };
        let state = window.global::<ui::SettingsState>();
        let stop = || {
            state.set_recording_hotkey(false);
            state.set_hotkey_error(hotkey::error().unwrap_or_default().into());
        };
        match event {
            WindowEvent::ModifiersChanged(m) => modifiers = m.state(),
            WindowEvent::Focused(false) if state.get_recording_hotkey() => stop(),
            WindowEvent::KeyboardInput {
                event,
                is_synthetic: false,
                ..
            } if state.get_recording_hotkey() => {
                let pressed = event.state == ElementState::Pressed && !event.repeat;
                match event.physical_key {
                    PhysicalKey::Code(_) if !pressed => {}
                    PhysicalKey::Code(KeyCode::Escape) if modifiers.is_empty() => stop(),
                    // A modifier on its own: wait for the key.
                    PhysicalKey::Code(
                        KeyCode::ControlLeft
                        | KeyCode::ControlRight
                        | KeyCode::AltLeft
                        | KeyCode::AltRight
                        | KeyCode::ShiftLeft
                        | KeyCode::ShiftRight
                        | KeyCode::SuperLeft
                        | KeyCode::SuperRight
                        | KeyCode::Meta,
                    ) => {}
                    PhysicalKey::Code(code) => {
                        let held = hotkey::Modifiers {
                            ctrl: modifiers.control_key(),
                            alt: modifiers.alt_key(),
                            shift: modifiers.shift_key(),
                            win: modifiers.super_key(),
                        };
                        // A combination that is not allowed is explained; recording goes on.
                        match hotkey::accelerator(held, &format!("{code:?}")) {
                            Ok(accelerator) => {
                                state.set_recording_hotkey(false);
                                apply_hotkey(&app, &accelerator);
                            }
                            Err(e) => state.set_hotkey_error(e.into()),
                        }
                    }
                    PhysicalKey::Unidentified(_) => {}
                }
                // Keys pressed while recording are not typing or navigation.
                return EventResult::PreventDefault;
            }
            _ => {}
        }
        EventResult::Propagate
    });
}

/// Registers a recorded hotkey, then saves it; if it cannot be registered, nothing is saved and
/// the previous hotkey stays (SPEC §6.5). Registering first also retries a saved hotkey that
/// failed at start-up.
fn apply_hotkey(app: &Arc<App>, accelerator: &str) {
    let result = hotkey::set(accelerator);
    if result.is_ok() && app.store.get().settings.hotkey != accelerator {
        save(
            app,
            model::patch("hotkey", Value::String(accelerator.to_owned())),
        );
        return;
    }
    WINDOW.with(|w| {
        if let Some(open) = w.borrow().as_ref() {
            let error = result.err().unwrap_or_default();
            let state = open.window.global::<ui::SettingsState>();
            state.set_hotkey_error(error.into());
        }
    });
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
    use crate::test_util::{screenshot_path, write_png};
    use crate::ui;

    struct Offscreen(Rc<MinimalSoftwareWindow>);

    impl Platform for Offscreen {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
            Ok(self.0.clone())
        }
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
        super::connect(&ui, &app);
        let mut open = Open {
            window: ui.clone_strong(),
            monitor_keys: Vec::new(),
            families: vec!["Arial".into(), "Consolas".into(), "Segoe UI".into()],
            fonts: Vec::new(),
        };
        fill(&mut open, &app, &app.store.get().settings);
        ui.show().expect("shown");
        window.set_size(PhysicalSize::new(width, height));

        let render = |name: &str| {
            window.request_redraw();
            let mut pixels = vec![PremultipliedRgbaColor::default(); (width * height) as usize];
            window.draw_if_needed(|renderer| {
                renderer.render(&mut pixels, width as usize);
            });
            // The window is opaque, so premultiplied and straight alpha agree.
            let rgba: Vec<u8> = pixels
                .iter()
                .flat_map(|p| [p.red, p.green, p.blue, p.alpha])
                .collect();
            write_png(&screenshot_path(name), width, height, &rgba);
        };
        let state = ui.global::<ui::SettingsState>();
        for (index, page) in super::model::PAGES.iter().enumerate() {
            state.set_page(index as i32);
            render(&format!("settings-{page}"));
        }

        // The hotkey button while it waits for keys, with a refused combination.
        state.set_page(0);
        state.set_recording_hotkey(true);
        let taken = crate::i18n::t_args("settings.general.hotkey.taken", &[("hotkey", "Ctrl+K")]);
        state.set_hotkey_error(taken.into());
        render("settings-general-recording");
        app.store.shutdown();
    }
}
