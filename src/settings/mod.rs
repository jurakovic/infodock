//! The Settings window (SPEC §10.7), a Slint window on the main thread. It is created when opened
//! and dropped when closed, so a closed Settings window costs no memory.
//!
//! Controls bind two-way to the `SettingsState` global. Each change is saved as a settings patch
//! through the config store; the store's change listener then refreshes the window, so changes
//! made elsewhere (tray, hotkey) show up too.

mod appearance;
mod form;
mod layout;
mod model;
mod picker;
mod properties;

use std::cell::RefCell;
use std::sync::Arc;

use serde_json::Value;
use slint::{CloseRequestResponse, ComponentHandle, ModelRc, SharedString, VecModel};

use crate::app::App;
use crate::config::model::Settings;
use crate::config::patch::apply_settings_patch;
use crate::config::{Config, ConfigError};
use crate::platform::geometry::DEFAULT_FLOATING_WIDTH;
use crate::{hotkey, ui};

/// The page with the hotkey.
const GENERAL_PAGE: i32 = 0;

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
    /// A sensor's properties being edited.
    form: Option<properties::FormState>,
    /// The sensor picker, while it is shown.
    picker: Option<picker::PickerState>,
    /// The skin gallery on the Appearance page.
    gallery: appearance::Gallery,
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
    let route = model::route(route);
    let page = match &route {
        model::Route::Page(index) => *index as i32,
        model::Route::New { .. } => picker::PAGE,
        model::Route::Sensor(_) => properties::PAGE,
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
    match route {
        model::Route::Sensor(id) => properties::open_form(app, &id, false),
        model::Route::New {
            zone,
            index,
            target,
        } => {
            let place = picker::place(&app.store.get().layout, zone, index, target);
            picker::open_picker(app, place);
        }
        model::Route::Page(_) => window.global::<ui::SettingsState>().set_page(page),
    }
    if let Err(e) = window.show() {
        tracing::warn!("could not show settings: {e}");
        return;
    }
    focus(&window);
}

/// The drive list changed (sampler thread): an open form's drive choices follow it.
pub fn volumes_changed(volumes: Vec<crate::platform::Volume>) {
    let queued = slint::invoke_from_event_loop(move || properties::volumes_changed(volumes));
    if queued.is_err() {
        tracing::debug!("the drive list arrived after the event loop ended");
    }
}

/// Refreshes the open window whenever the config changes (from any thread).
pub fn watch(app: &Arc<App>) {
    let a = Arc::clone(app);
    app.store.on_change(Box::new(move |_, new| {
        let (settings, sensors) = (new.settings.clone(), new.layout.clone());
        let a = Arc::clone(&a);
        let _ = slint::invoke_from_event_loop(move || {
            WINDOW.with(|w| {
                if let Some(open) = w.borrow_mut().as_mut() {
                    fill(open, &a, &settings);
                    layout::fill(open, &a, &sensors);
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

    let a = Arc::clone(app);
    window.window().on_close_requested(move || {
        // Closing drops an open properties form (SPEC §6.7), then the window once it is hidden,
        // which frees what it allocated.
        WINDOW.with(|w| {
            if let Ok(mut w) = w.try_borrow_mut()
                && let Some(open) = w.as_mut()
            {
                properties::cancel(open, &a);
            }
        });
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
        form: None,
        picker: None,
        gallery: appearance::Gallery::default(),
    };
    let config = app.store.get();
    fill(&mut open, app, &config.settings);
    layout::fill(&open, app, &config.layout);
    WINDOW.with(|w| *w.borrow_mut() = Some(open));
    Ok(window)
}

/// Handles the window's callbacks: saving changes and converting colours.
fn connect(window: &ui::SettingsWindow, app: &Arc<App>) {
    properties::connect(window, app);
    picker::connect(window, app);
    layout::connect(window, app);
    appearance::connect(window, app);
    let state = window.global::<ui::SettingsState>();
    let a = Arc::clone(app);
    state.on_page_changed(move |page| {
        // Leaving the form or the picker by the navigation drops it.
        WINDOW.with(|w| {
            if let Ok(mut w) = w.try_borrow_mut()
                && let Some(open) = w.as_mut()
            {
                if page != properties::PAGE {
                    properties::cancel(open, &a);
                }
                if page != picker::PAGE {
                    open.picker = None;
                }
                // Packs installed or removed meanwhile show in the gallery.
                if page == appearance::PAGE {
                    open.gallery.refresh(&a);
                }
                // Keys typed on another page are not a hotkey: recording ends with the page.
                let state = open.window.global::<ui::SettingsState>();
                state.set_recording_hotkey(false);
                // Text that was typed but not saved (an incomplete colour) is not shown again.
                let settings = a.store.get().settings;
                fill(open, &a, &settings);
            }
        });
    });
    let a = Arc::clone(app);
    state.on_set_flag(move |path, value| save(&a, model::patch(&path, Value::Bool(value))));
    let a = Arc::clone(app);
    state.on_set_number(move |path, value| save_change(&a, &path, Value::from(value)));
    let a = Arc::clone(app);
    state.on_reset(move |path| save(&a, model::patch(&path, Value::Null)));
    let a = Arc::clone(app);
    state.on_set_text(move |path, text| {
        save_change(&a, &path, Value::String(text.as_str().to_owned()));
    });
    let a = Arc::clone(app);
    state.on_set_floating_width(move |width| {
        let width = u32::try_from(width).unwrap_or(DEFAULT_FLOATING_WIDTH);
        let settings = a.store.get().settings;
        let floating = model::floating_with_width(&settings, &a.platform.monitors(), width);
        if let Ok(value) = serde_json::to_value(floating) {
            save(&a, model::patch("bar.floating", value));
        }
    });
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
    state.on_is_color(|text| {
        crate::config::model::normalize_color(text.trim().to_owned()).is_some()
    });
    let a = Arc::clone(app);
    state.on_set_color(move |path, text| {
        match crate::config::model::normalize_color(text.trim().to_owned()) {
            Some(color) => save_change(&a, &path, Value::String(color)),
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
    state.set_new_sensor_menu(choice("newSensorMenu"));
    state.set_sensor_spacing(settings.sensor_spacing as f32);

    state.set_mode(choice("bar.mode"));
    state.set_floating_width(
        settings
            .bar
            .floating
            .map_or(DEFAULT_FLOATING_WIDTH, |f| f.width) as f32,
    );
    state.set_edge(choice("bar.edge"));
    let monitors = model::monitor_choices(&settings.bar.monitor, &app.platform.monitors());
    let labels: Vec<SharedString> = monitors.labels.iter().map(SharedString::from).collect();
    state.set_monitors(ModelRc::new(VecModel::from(labels)));
    state.set_monitor(i32::try_from(monitors.current).unwrap_or(0));
    open.monitor_keys = monitors.keys;
    state.set_reserve_space(settings.bar.reserve_space);
    state.set_z_order(choice("bar.zOrder"));
    state.set_hide_on_fullscreen(settings.bar.hide_on_fullscreen);
    state.set_highlight_on_hover(settings.bar.highlight_on_hover);
    state.set_animations(choice("bar.animations"));
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

    appearance::fill(open, app, settings);
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

/// Saves `value` at `path` if that changes what is in effect, so a colour box losing the focus
/// or a slider clicked without moving leaves a theme default `null` (SPEC §11.2).
fn save_change(app: &Arc<App>, path: &str, value: Value) {
    if model::changes(&app.store.get().settings, path, &value) {
        save(app, model::patch(path, value));
        return;
    }
    // Nothing to save; an error from an earlier attempt no longer applies.
    WINDOW.with(|w| {
        if let Some(open) = w.borrow().as_ref() {
            let state = open.window.global::<ui::SettingsState>();
            state.set_save_error(SharedString::new());
        }
    });
}

fn apply(app: &App, patch: &Value) -> Result<Config, SaveError> {
    let current = app.store.get().settings;
    let next = apply_settings_patch(&current, patch).map_err(SaveError::Config)?;
    if next.hotkey != current.hotkey {
        hotkey::set(&next.hotkey, false).map_err(SaveError::Hotkey)?;
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
/// (SPEC §6.5); Esc, leaving the window or the General page cancels. The keys are read from
/// winit as physical keys, so the keyboard layout does not matter (Ctrl+Alt is AltGr on many
/// layouts).
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
            } if state.get_recording_hotkey() && state.get_page() == GENERAL_PAGE => {
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
    let configured = app.store.get().settings.hotkey == accelerator;
    let result = hotkey::set(accelerator, configured);
    if result.is_ok() && !configured {
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

    use serde_json::json;
    use slint::platform::software_renderer::{
        MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
    };
    use slint::platform::{Key, Platform, PointerEventButton, WindowAdapter, WindowEvent};
    use slint::{ComponentHandle, LogicalPosition, PhysicalSize, SharedString};

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
        let app = Arc::new(App::new(
            store,
            crate::platform::current(),
            dir.path().join("skins"),
        ));

        let ui = ui::SettingsWindow::new().expect("the window builds");
        crate::install_i18n!(ui);
        super::connect(&ui, &app);
        let mut open = Open {
            window: ui.clone_strong(),
            monitor_keys: Vec::new(),
            families: vec!["Arial".into(), "Consolas".into(), "Segoe UI".into()],
            fonts: Vec::new(),
            form: None,
            picker: None,
            gallery: super::appearance::Gallery::default(),
        };
        fill(&mut open, &app, &app.store.get().settings);
        super::layout::fill(&open, &app, &app.store.get().layout);
        ui.show().expect("shown");

        let render_sized = |name: &str, height: u32| {
            window.set_size(PhysicalSize::new(width, height));
            // Let state animations (e.g. a button becoming disabled) finish.
            std::thread::sleep(std::time::Duration::from_millis(200));
            slint::platform::update_timers_and_animations();
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
        let render = |name: &str| render_sized(name, height);
        let state = ui.global::<ui::SettingsState>();
        for (index, page) in super::model::PAGES.iter().enumerate() {
            state.set_page(index as i32);
            render(&format!("settings-{page}"));
        }
        // The whole Bar page.
        state.set_page(1);
        render_sized("settings-bar-full", 1000);
        state.set_mode(1);
        render("settings-bar-floating");
        state.set_mode(0);
        // Appearance with the classic theme: the skin gallery with a pack made here (a blue
        // gradient; a swatch where there is no renderer), the custom image and a missing skin.
        let pack = app.skins.join("blue-steel");
        std::fs::create_dir_all(&pack).expect("pack folder");
        std::fs::write(
            pack.join("skin.json"),
            r#"{ "name": "Blue steel", "image": "tile.png" }"#,
        )
        .expect("skin.json");
        let tile: Vec<u8> = (0..25u8)
            .flat_map(|y| [40 + y * 4, 80 + y * 3, 160 + y * 3, 255].repeat(8))
            .collect();
        write_png(&pack.join("tile.png"), 8, 25, &tile);
        let set = |open: &mut Open, change: &dyn Fn(&mut crate::config::model::Settings)| {
            app.store
                .update(|c| {
                    change(&mut c.settings);
                    Ok(())
                })
                .expect("saved");
            fill(open, &app, &app.store.get().settings);
        };
        state.set_page(super::appearance::PAGE);
        set(&mut open, &|s| {
            s.theme = crate::config::model::Theme::Classic
        });
        render_sized("settings-appearance-classic", 1000);
        set(&mut open, &|s| {
            s.classic.skin = "custom".into();
            s.classic.custom_skin = "wood.png".into();
        });
        render_sized("settings-appearance-custom", 1000);
        set(&mut open, &|s| s.classic.skin = "gone".into());
        render("settings-appearance-missing");
        set(&mut open, &|s| {
            *s = crate::config::model::Settings::default();
        });
        // The whole Sensors page, with the Layout manager; then a Remove waiting to be confirmed.
        state.set_page(super::model::SENSORS_PAGE as i32);
        render_sized("settings-sensors-layout", 1000);
        let logo = app.store.get().layout.center[0].id.clone();
        state.set_layout_confirm(logo.as_str().into());
        render_sized("settings-sensors-remove", 1000);
        state.set_layout_confirm(SharedString::new());
        // Drag memory-meter (lines: # Left 89, cpu 133, memory 177, network 221, # Centre 265)
        // onto the lower half of network-graph: it moves to the end of the left zone.
        let pointer = |event: fn(LogicalPosition) -> WindowEvent, x: f32, y: f32| {
            window.dispatch_event(event(LogicalPosition::new(x, y)));
        };
        pointer(
            |position| WindowEvent::PointerPressed {
                position,
                button: PointerEventButton::Left,
            },
            300.0,
            199.0,
        );
        for y in [210.0, 235.0, 250.0] {
            pointer(|position| WindowEvent::PointerMoved { position }, 300.0, y);
        }
        render_sized("settings-sensors-drag", 1000);
        pointer(
            |position| WindowEvent::PointerReleased {
                position,
                button: PointerEventButton::Left,
            },
            300.0,
            250.0,
        );
        let kinds: Vec<_> = app
            .store
            .get()
            .layout
            .left
            .iter()
            .map(|s| s.kind.clone())
            .collect();
        assert_eq!(kinds, ["cpu-graph", "network-graph", "memory-meter"]);

        // The hotkey button while it waits for keys, with a refused combination.
        state.set_page(0);
        state.set_recording_hotkey(true);
        let taken = crate::i18n::t_args("settings.general.hotkey.taken", &[("hotkey", "Ctrl+K")]);
        state.set_hotkey_error(taken.into());
        render("settings-general-recording");
        state.set_recording_hotkey(false);
        state.set_hotkey_error(SharedString::new());

        // From here on the callbacks find the window, as in the app.
        super::WINDOW.with(|w| *w.borrow_mut() = Some(open));
        // Clicks and keys as the user gives them. Neither the navigation nor a button takes the
        // focus, so no input may wait for its field to lose it.
        let settle = || {
            slint::platform::update_timers_and_animations();
            window.request_redraw();
            let size = window.size();
            let mut pixels =
                vec![PremultipliedRgbaColor::default(); (size.width * size.height) as usize];
            window.draw_if_needed(|renderer| {
                renderer.render(&mut pixels, size.width as usize);
            });
        };
        let click = |x: f32, y: f32| {
            let position = LogicalPosition::new(x, y);
            let button = PointerEventButton::Left;
            window.dispatch_event(WindowEvent::PointerPressed { position, button });
            window.dispatch_event(WindowEvent::PointerReleased { position, button });
            settle();
        };
        let key = |text: SharedString| {
            window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
            window.dispatch_event(WindowEvent::KeyReleased { text });
        };
        let retype = |x: f32, y: f32, text: &str| {
            click(x, y);
            key(Key::End.into());
            for _ in 0..16 {
                key(Key::Backspace.into());
            }
            text.chars().for_each(|c| key(c.into()));
            settle();
        };
        let (general, bar, appearance, sensors, popups) = (27.0, 67.0, 107.0, 147.0, 187.0);
        let nav = |y: f32| click(49.0, y);

        // Ctrl+Space in the navigation unselects its item; the page stays.
        key(Key::Tab.into());
        window.dispatch_event(WindowEvent::KeyPressed {
            text: Key::Control.into(),
        });
        key(' '.into());
        window.dispatch_event(WindowEvent::KeyReleased {
            text: Key::Control.into(),
        });
        settle();
        assert_eq!(state.get_page(), 0);

        // A sensor moved with the keyboard keeps the focus on its button, so it moves on.
        nav(sensors);
        key(Key::Tab.into()); // the left zone's Add
        key(Key::Tab.into()); // cpu-graph's Down (its Up is disabled)
        for _ in 0..2 {
            key(' '.into());
            // In the app the store's listener shows the change.
            super::WINDOW.with(|w| {
                if let Some(open) = w.borrow().as_ref() {
                    super::layout::fill(open, &app, &app.store.get().layout);
                }
            });
            settle();
        }
        let left = || -> Vec<String> {
            let layout = app.store.get().layout;
            layout.left.iter().map(|s| s.kind.clone()).collect()
        };
        assert_eq!(left(), ["network-graph", "memory-meter", "cpu-graph"]);
        nav(general);

        // Appearance: a click on a gallery tile chooses its skin, and once the gallery has the
        // focus, the arrow keys move on.
        let refill = || {
            super::WINDOW.with(|w| {
                if let Some(open) = w.borrow_mut().as_mut() {
                    fill(open, &app, &app.store.get().settings);
                }
            });
            settle();
        };
        let skin = || app.store.get().settings.classic.skin;
        nav(appearance);
        state.invoke_set_choice("theme".into(), 1);
        window.set_size(PhysicalSize::new(width, 1000));
        refill();
        // Carbon, the first tile of the second row.
        click(301.0, 634.0);
        assert_eq!(skin(), "carbon");
        refill();
        key(Key::RightArrow.into());
        assert_eq!(skin(), "glass");
        state.invoke_set_choice("theme".into(), 0);
        window.set_size(PhysicalSize::new(width, height));
        refill();

        // Leaving General ends recording the hotkey.
        state.set_recording_hotkey(true);
        nav(bar);
        assert_eq!(state.get_page(), 1);
        assert!(!state.get_recording_hotkey());

        // A dragged slider snaps to its step (10 px for the width) when released, so it shows
        // what is saved.
        state.set_mode(1);
        settle();
        pointer(
            |position| WindowEvent::PointerPressed {
                position,
                button: PointerEventButton::Left,
            },
            532.0,
            143.0,
        );
        pointer(
            |position| WindowEvent::PointerMoved { position },
            573.0,
            143.0,
        );
        pointer(
            |position| WindowEvent::PointerReleased {
                position,
                button: PointerEventButton::Left,
            },
            573.0,
            143.0,
        );
        settle();
        let width = state.get_floating_width();
        assert_eq!(width % 10.0, 0.0, "{width}");
        let floating = app.store.get().settings.bar.floating;
        assert_eq!(floating.map(|f| f.width as f32), Some(width));
        state.set_mode(0);

        // A colour is saved as soon as it is valid; one left incomplete is not shown again.
        nav(popups);
        retype(690.0, 76.0, "#20b7ff");
        let background = || app.store.get().settings.popups.background;
        assert_eq!(background().as_deref(), Some("#20B7FF"));
        key("f".into());
        nav(general);
        nav(popups);
        assert_eq!(state.get_popup_background(), "#20B7FF");
        assert_eq!(background().as_deref(), Some("#20B7FF"));
        state.invoke_reset("popups.background".into());
        // Leaving the colour box and clicking the slider without changing them keep the theme
        // defaults (SPEC §11.2).
        nav(general);
        nav(popups);
        click(690.0, 76.0);
        click(690.0, 258.0);
        click(652.0, 168.0);
        let popups_settings = app.store.get().settings.popups;
        assert_eq!(popups_settings.background, None);
        assert_eq!(popups_settings.opacity, None);

        // Properties (SPEC §6.7): the logo's, then with a custom colour; a new Text sensor's.
        super::properties::open_form(&app, &logo, false);
        render("settings-properties-image");
        state.invoke_prop_color_mode("color".into(), 1, "#20B7FF".into());
        render("settings-properties-image-custom");
        // An incomplete colour shows its error on the row and disables Save (SPEC §6.7).
        retype(690.0, 203.0, "#12345");
        assert!(!state.get_props_valid());
        render("settings-properties-invalid");
        // A typed path and colour are what Save stores.
        retype(690.0, 203.0, "#123abc");
        assert!(state.get_props_valid());
        retype(525.0, 92.0, r"C:\Pictures\logo.png");
        // A path picked with Browse replaces the typed one, and typing goes on from it.
        super::properties::set_path(&app, "image", r"C:\Pictures\other.png".into());
        settle();
        click(525.0, 92.0);
        key(Key::End.into());
        (0..3).for_each(|_| key(Key::Backspace.into()));
        "svg".chars().for_each(|c| key(c.into()));
        click(723.0, 380.0);
        assert_eq!(state.get_page(), super::model::SENSORS_PAGE as i32);
        let config = app.store.get();
        let props = &config
            .layout
            .all()
            .find(|s| s.id == logo)
            .expect("logo")
            .props;
        assert_eq!(props.get("image"), Some(&json!(r"C:\Pictures\other.svg")));
        let color = json!({ "mode": "custom", "color": "#123ABC" });
        assert_eq!(props.get("color"), Some(&color));
        let text = app
            .add_sensor(
                "text",
                crate::config::model::Zone::Left,
                0,
                &crate::sensors::Props::new(),
            )
            .expect("added");
        super::properties::open_form(&app, &text, true);
        render("settings-properties-text");
        // Cancel removes the new sensor again.
        state.invoke_props_cancel();
        assert!(app.store.get().layout.find(&text).is_none());
        assert_eq!(state.get_page(), super::model::SENSORS_PAGE as i32);
        // A drive field lists this computer's volumes (none on Linux: only the stored drive).
        let disk = app
            .add_sensor(
                "disk-meter",
                crate::config::model::Zone::Left,
                0,
                &crate::sensors::Props::new(),
            )
            .expect("added");
        super::properties::open_form(&app, &disk, true);
        render("settings-properties-disk");
        state.invoke_props_cancel();
        assert!(app.store.get().layout.find(&disk).is_none());
        // The Shortcut's form: paths, parameters, a web address and how it runs (SPEC §9.17).
        let shortcut = app
            .add_sensor(
                "shortcut",
                crate::config::model::Zone::Left,
                0,
                &crate::sensors::Props::new(),
            )
            .expect("added");
        super::properties::open_form(&app, &shortcut, true);
        render_sized("settings-properties-shortcut", 900);
        state.invoke_props_cancel();
        assert!(app.store.get().layout.find(&shortcut).is_none());
        // Power: the modes this computer offers (none on Linux: only the stored one).
        let power = app
            .add_sensor(
                "power",
                crate::config::model::Zone::Left,
                0,
                &crate::sensors::Props::new(),
            )
            .expect("added");
        super::properties::open_form(&app, &power, true);
        render("settings-properties-power");
        state.invoke_props_cancel();
        // The Disk launcher: every drive, optical ones too (none on Linux: only the stored one).
        let disk = app
            .add_sensor(
                "disk",
                crate::config::model::Zone::Left,
                0,
                &crate::sensors::Props::new(),
            )
            .expect("added");
        super::properties::open_form(&app, &disk, true);
        render("settings-properties-disk-launcher");
        state.invoke_props_cancel();
        // Battery: this computer's batteries (none on Linux: only the stored one, marked).
        let battery = app
            .add_sensor(
                "battery",
                crate::config::model::Zone::Left,
                0,
                &crate::sensors::Props::new(),
            )
            .expect("added");
        super::properties::open_form(&app, &battery, true);
        render_sized("settings-properties-battery", 900);
        state.invoke_props_cancel();
        // Folder Menu: the folder, title, image, colour, opacity and hidden files (its list size
        // is not shown).
        let folder = app
            .add_sensor(
                "folder-menu",
                crate::config::model::Zone::Left,
                0,
                &crate::sensors::Props::new(),
            )
            .expect("added");
        super::properties::open_form(&app, &folder, true);
        render("settings-properties-folder-menu");
        state.invoke_props_cancel();
        // Start Menu: title, image, colour and opacity; no folder.
        let start_menu = app
            .add_sensor(
                "start-menu",
                crate::config::model::Zone::Left,
                0,
                &crate::sensors::Props::new(),
            )
            .expect("added");
        super::properties::open_form(&app, &start_menu, true);
        render("settings-properties-start-menu");
        state.invoke_props_cancel();
        // The first-run network graph: Automatic and this computer's connections.
        let network = app
            .store
            .get()
            .layout
            .all()
            .find(|s| s.kind == "network-graph")
            .map(|s| s.id.clone())
            .expect("the first-run network graph");
        super::properties::open_form(&app, &network, false);
        render_sized("settings-properties-network", 1000);
        state.invoke_props_cancel();

        // The picker (SPEC §6.6): next to the logo, with Text selected; then a search.
        let place = super::picker::Place::Next(logo.clone());
        super::picker::open_picker(&app, place);
        state.set_picker_selected(2);
        render("settings-picker");
        state.set_picker_search("zzz".into());
        state.invoke_picker_searched("zzz".into());
        assert_eq!(state.get_picker_selected(), -1);
        render("settings-picker-search");
        // Add opens the new sensor's properties, after the logo.
        state.invoke_picker_searched(SharedString::new());
        state.set_picker_selected(2);
        state.invoke_picker_add();
        assert_eq!(state.get_page(), super::properties::PAGE);
        let layout = app.store.get().layout;
        assert_eq!(layout.center[1].kind, "text");
        state.invoke_props_cancel();
        assert_eq!(app.store.get().layout.center.len(), 1);
        // From the keyboard: typing selects the first match, Up and Down in the search box move
        // the selection, and Enter adds it.
        super::picker::open_picker(&app, super::picker::Place::Next(logo.clone()));
        settle();
        key('e'.into());
        assert_eq!(state.get_picker_selected(), 0, "Image");
        // The types listed depend on the platform (e.g. the CPU dual graph needs per-processor
        // counters), so the last match is counted from the picker's own entries.
        let entries =
            super::picker::entries(crate::sensors::registry::registry(), app.platform.as_ref());
        let matches: Vec<usize> = super::picker::lines(&entries, "e")
            .into_iter()
            .filter_map(|line| match line {
                super::picker::Line::Entry(i) => Some(i),
                super::picker::Line::Heading(_) => None,
            })
            .collect();
        let last = *matches.last().expect("matches") as i32;
        (0..=matches.len()).for_each(|_| key(Key::DownArrow.into()));
        assert_eq!(state.get_picker_selected(), last, "the last, and it stays");
        // The list scrolled to show it.
        settle();
        render("settings-picker-end");
        let date = matches
            .iter()
            .position(|i| entries[*i].kind == "date")
            .expect("Date matches");
        (date + 1..matches.len()).for_each(|_| key(Key::UpArrow.into()));
        assert_eq!(state.get_picker_selected(), matches[date] as i32, "Date");
        key(Key::Return.into());
        assert_eq!(state.get_page(), super::properties::PAGE);
        assert_eq!(app.store.get().layout.center[1].kind, "date");
        state.invoke_props_cancel();
        assert_eq!(app.store.get().layout.center.len(), 1);
        super::WINDOW.with(|w| w.borrow_mut().take());
        app.store.shutdown();
    }
}
