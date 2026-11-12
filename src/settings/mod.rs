//! The Settings window (SPEC §10.7), a Slint window on the main thread. It is created when opened
//! and dropped when closed, so a closed Settings window costs no memory.
//!
//! Controls bind two-way to the `SettingsState` global. Each change is saved as a settings patch
//! through the config store; the store's change listener then refreshes the window, so changes
//! made elsewhere (tray, hotkey) show up too.

mod about;
#[cfg(test)]
mod accessibility;
mod appearance;
mod applications;
#[cfg(test)]
mod contract;
mod form;
mod layout;
mod model;
mod picker;
mod properties;
#[cfg(test)]
mod screenshots;
mod welcome;
mod window;

use std::cell::Cell;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use slint::{CloseRequestResponse, ComponentHandle, ModelRc, SharedString, VecModel};

use crate::app::App;
use crate::config::model::{Settings, WindowPlace};
use crate::config::patch::apply_settings_patch;
use crate::config::{Config, ConfigError};
use crate::platform::geometry::DEFAULT_FLOATING_WIDTH;
use crate::{hotkey, ui};
pub use window::dialogs;
use window::{pick, with_open, with_open_mut};

/// The page with the hotkey.
const GENERAL_PAGE: i32 = 0;
/// How long after Settings opens or closes the app logs its memory: as long as CI's memory
/// check waits before it measures (SPEC §14.1).
const MEMORY_LOG_DELAY: Duration = Duration::from_secs(10);

thread_local! {
    /// Counts Settings' openings and closings, so a memory log planned after one is dropped
    /// when another came first.
    static OPENED_OR_CLOSED: Cell<u64> = const { Cell::new(0) };
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
    /// The colour scheme the native window was given (SPEC §10.3).
    scheme: Option<crate::config::model::ColorScheme>,
    /// The welcome page (SPEC §11.5).
    welcome: welcome::Welcome,
    /// Where the window is when neither maximized nor minimized, kept as it moves (SPEC §10.7).
    place: Option<WindowPlace>,
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
        model::Route::Welcome => welcome::PAGE,
    };
    let window = match window::handle() {
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
        model::Route::Welcome => {
            with_open_mut(|open| welcome::show(open, app));
        }
    }
    if let Err(e) = window.show() {
        tracing::warn!("could not show settings: {e}");
        return;
    }
    // The native window exists once shown: its title bar takes the colour scheme, and the
    // welcome page's previews its scale.
    let settings = app.store.get().settings;
    with_open_mut(|open| {
        appearance::apply_scheme(open, settings.color_scheme);
        welcome::fill(open, app, &settings);
    });
    focus(&window);
}

/// A second launch (SPEC §4.1): brings an open Settings window to the front on the page it shows,
/// or opens Settings on General. Leaving the page would end a first run shown on the welcome
/// page (§11.5). Main thread only.
pub fn activate(app: &Arc<App>) {
    match window::handle() {
        Some(window) => {
            if let Err(e) = window.show() {
                tracing::warn!("could not show settings: {e}");
                return;
            }
            focus(&window);
        }
        None => open(app, "general"),
    }
}

/// The drive list arrived or changed (sampler thread): an open form's drive choices follow it.
pub fn volumes_changed(volumes: Vec<crate::platform::Volume>) {
    let queued = slint::invoke_from_event_loop(move || properties::volumes_changed(volumes));
    if queued.is_err() {
        tracing::debug!("the drive list arrived after the event loop ended");
    }
}

/// The battery list arrived or changed (sampler thread): an open form's battery choices follow
/// it.
pub fn batteries_changed(names: Vec<String>) {
    let queued = slint::invoke_from_event_loop(move || properties::batteries_changed(names));
    if queued.is_err() {
        tracing::debug!("the battery list arrived after the event loop ended");
    }
}

/// Refreshes the open window whenever the config changes (from any thread).
pub fn watch(app: &Arc<App>) {
    let a = Arc::clone(app);
    app.store.on_change(Box::new(move |_, new| {
        let (settings, sensors) = (new.settings.clone(), new.layout.clone());
        let a = Arc::clone(&a);
        let _ = slint::invoke_from_event_loop(move || {
            with_open_mut(|open| {
                fill(open, &a, &settings);
                layout::fill(open, &a, &sensors);
            });
        });
    }));
}

fn create(app: &Arc<App>) -> Result<ui::SettingsWindow, slint::PlatformError> {
    let window = ui::SettingsWindow::new()?;
    crate::install_i18n!(window);
    connect(&window, app);
    watch_window_events(&window, app);

    let a = Arc::clone(app);
    window.window().on_close_requested(move || {
        closing(&a);
        CloseRequestResponse::HideWindow
    });

    let config = app.store.get();
    let place = config.settings.settings_window;
    if let Some(place) = &place {
        restore_place(&window, place, &app.platform.monitors());
    }
    let mut open = Open {
        window: window.clone_strong(),
        monitor_keys: Vec::new(),
        families: app.platform.font_families(),
        fonts: Vec::new(),
        form: None,
        picker: None,
        gallery: appearance::Gallery::default(),
        scheme: None,
        welcome: welcome::Welcome::default(),
        place,
    };
    fill(&mut open, app, &config.settings);
    layout::fill(&open, app, &config.layout);
    window::set(Some(open));
    log_memory_later(app, true);
    Ok(window)
}

/// Puts a new window where Settings was closed (SPEC §10.7), before it is first shown: its size
/// and maximized state, and its place while a monitor still shows its title bar.
fn restore_place(
    window: &ui::SettingsWindow,
    place: &WindowPlace,
    monitors: &[crate::platform::MonitorInfo],
) {
    let w = window.window();
    w.set_size(slint::LogicalSize::new(
        place.width as f32,
        place.height as f32,
    ));
    if model::place_reachable(place, monitors) {
        w.set_position(slint::PhysicalPosition::new(place.x, place.y));
    }
    w.set_maximized(place.maximized);
}

/// The window's place while it is neither maximized nor minimized (SPEC §10.7).
fn normal_place(w: &slint::winit_030::winit::window::Window) -> Option<WindowPlace> {
    if w.is_maximized() || w.is_minimized().unwrap_or(false) {
        return None;
    }
    let position = w.outer_position().ok()?;
    let size = w.inner_size().to_logical::<f64>(w.scale_factor());
    Some(WindowPlace {
        x: position.x,
        y: position.y,
        width: size.width.round() as u32,
        height: size.height.round() as u32,
        maximized: false,
    })
}

/// Saves where the open window is (SPEC §10.7): the place it has, or last had, when neither
/// maximized nor minimized, and whether it is maximized (as it was before being minimized).
fn remember_place(app: &Arc<App>) {
    use slint::winit_030::WinitWindowAccessor;
    let place = with_open(|open| {
        let w = open.window.window();
        let normal = w.with_winit_window(normal_place).flatten().or(open.place);
        normal.map(|p| WindowPlace {
            maximized: w.is_maximized(),
            ..p
        })
    })
    .flatten();
    let Some(place) = place else {
        return;
    };
    if app.store.get().settings.settings_window == Some(place) {
        return;
    }
    if let Ok(value) = serde_json::to_value(place) {
        save(app, model::patch("settingsWindow", value));
    }
}

/// The app is quitting (main thread): an open window's place is kept, as closing it would.
pub fn quitting(app: &Arc<App>) {
    remember_place(app);
}

/// Closing saves where the window is (SPEC §10.7), drops an open properties form (§6.7) and
/// leaves the welcome page (§11.5), then drops the window once it is hidden, which frees what
/// it allocated.
fn closing(app: &Arc<App>) {
    remember_place(app);
    with_open_mut(|open| {
        properties::cancel(open, app);
        welcome::left(open, app);
    });
    let _ = slint::invoke_from_event_loop(|| window::set(None));
    log_memory_later(app, false);
}

/// Logs the process's memory [`MEMORY_LOG_DELAY`] after Settings opened (`opened`) or closed,
/// unless it was closed or opened again meanwhile (SPEC §14.1): a log from real hardware shows
/// what Settings costs there, and whether closing it gives the memory back.
fn log_memory_later(app: &Arc<App>, opened: bool) {
    let planned = OPENED_OR_CLOSED.with(|count| {
        count.set(count.get() + 1);
        count.get()
    });
    let platform = Arc::clone(&app.platform);
    slint::Timer::single_shot(MEMORY_LOG_DELAY, move || {
        if OPENED_OR_CLOSED.with(Cell::get) != planned {
            return;
        }
        let Some(memory) = platform.process_memory() else {
            return;
        };
        let secs = MEMORY_LOG_DELAY.as_secs();
        if opened {
            let welcome = with_open(|open| open.welcome.shown).unwrap_or(false);
            let page = if welcome { " on the welcome page" } else { "" };
            tracing::info!("memory {secs} s after Settings opened{page}: {memory}");
        } else {
            tracing::info!("memory {secs} s after Settings closed: {memory}");
        }
    });
}

/// Closes the window, as its close button does.
fn close(app: &Arc<App>) {
    let window = window::handle();
    closing(app);
    if let Some(window) = window
        && let Err(e) = window.hide()
    {
        tracing::warn!("could not close settings: {e}");
    }
}

/// Handles the window's callbacks: saving changes and converting colours.
fn connect(window: &ui::SettingsWindow, app: &Arc<App>) {
    properties::connect(window, app);
    picker::connect(window, app);
    layout::connect(window, app);
    appearance::connect(window, app);
    applications::connect(window, app);
    about::connect(window, app);
    welcome::connect(window, app);
    let state = window.global::<ui::SettingsState>();
    let a = Arc::clone(app);
    state.on_page_changed(move |page| {
        // Leaving the form or the picker by the navigation drops it, and a page's question
        // whether to restore its defaults.
        with_open_mut(|open| {
            open.window
                .global::<ui::SettingsState>()
                .set_restore_confirm(-1);
            if page != properties::PAGE {
                properties::cancel(open, &a);
            }
            if page != picker::PAGE {
                open.picker = None;
            }
            if page != welcome::PAGE {
                welcome::left(open, &a);
            }
            // Packs installed or removed meanwhile show in the gallery.
            if page == appearance::PAGE {
                open.gallery.refresh(&a);
            }
            // Keys typed on another page are not a hotkey: recording ends with the page.
            let state = open.window.global::<ui::SettingsState>();
            state.set_recording_hotkey(false);
            state.set_skin_install_message(SharedString::new());
            // Text that was typed but not saved (an incomplete colour) is not shown again.
            let settings = a.store.get().settings;
            fill(open, &a, &settings);
        });
    });
    let a = Arc::clone(app);
    state.on_set_flag(move |path, value| save(&a, model::patch(&path, Value::Bool(value))));
    let a = Arc::clone(app);
    state.on_set_number(move |path, value| save_change(&a, &path, Value::from(value)));
    let a = Arc::clone(app);
    state.on_reset(move |path| save(&a, model::patch(&path, Value::Null)));
    let a = Arc::clone(app);
    state.on_restore_defaults(move |page| {
        if let Ok(page) = usize::try_from(page) {
            restore_defaults(&a, page);
        }
    });
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
    // Windows' colour dialog from a colour box's swatch, opened on the box's colour (white for
    // text that is not one yet): the colour chosen, or nothing for Cancel.
    state.on_pick_color(|text| {
        use crate::platform::{Color, Dialogs};
        let initial = Color::from_hex(text.trim()).unwrap_or(Color::rgba(255, 255, 255, 1.0));
        window::dialogs()
            .pick_color(initial)
            .map(Color::to_hex)
            .unwrap_or_default()
            .into()
    });
    state.on_set_color(move |path, text| {
        match crate::config::model::normalize_color(text.trim().to_owned()) {
            Some(color) => save_change(&a, &path, Value::String(color)),
            None => {
                with_open_mut(|open| {
                    let state = open.window.global::<ui::SettingsState>();
                    state.set_save_error(crate::i18n::t("settings.color.invalid").into());
                    // Back to the saved colour.
                    let settings = a.store.get().settings;
                    fill(open, &a, &settings);
                });
            }
        }
    });
    let a = Arc::clone(app);
    state.on_set_font(move |path, index| {
        let family = with_open(|open| {
            let index = usize::try_from(index).ok().filter(|i| *i > 0)?;
            open.fonts.get(index).cloned()
        })
        .flatten();
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
        let key = with_open(|open| {
            usize::try_from(index)
                .ok()
                .and_then(|i| open.monitor_keys.get(i).cloned())
        })
        .flatten();
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

    state.set_autostart(settings.autostart);
    state.set_autostart_error(crate::autostart::error().unwrap_or_default().into());
    state.set_hotkey(settings.hotkey.as_str().into());
    state.set_hotkey_error(hotkey::error().unwrap_or_default().into());
    state.set_hotkey_action(choice("hotkeyAction"));
    state.set_bar_focus_ring(settings.bar_focus_ring);
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
    state.set_shadow(settings.bar.shadow);
    state.set_animations(choice("bar.animations"));
    state.set_height(settings.bar_height() as f32);
    state.set_sensor_padding(settings.bar.sensor_padding as f32);
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

    // Applications (SPEC §10.6)
    state.set_calendar_app(settings.apps.calendar.as_str().into());

    appearance::fill(open, app, settings);
    welcome::fill(open, app, settings);
}

/// Saves a settings patch and shows the outcome. A changed hotkey is registered first; if that
/// fails, nothing is saved (SPEC §6.5).
fn save(app: &Arc<App>, patch: Value) {
    let result = apply(app, &patch);
    report(app, &result);
}

/// Puts `page`'s settings back to their defaults (SPEC §10.7). A default hotkey that cannot be
/// registered (another app holds it) stays as it is, with the reason under it, and the page's
/// other settings are restored all the same.
fn restore_defaults(app: &Arc<App>, page: usize) {
    let mut patch = model::page_defaults(page);
    let mut result = apply(app, &patch);
    if matches!(result, Err(SaveError::Hotkey(_)))
        && let Some(settings) = patch.as_object_mut()
    {
        settings.remove("hotkey");
        report(app, &result);
        result = apply(app, &patch);
    }
    report(app, &result);
}

/// Shows what saving a change came to: its error, or none.
fn report(app: &Arc<App>, result: &Result<Config, SaveError>) {
    with_open_mut(|open| {
        let state = open.window.global::<ui::SettingsState>();
        match result {
            Ok(_) => state.set_save_error(SharedString::new()),
            Err(SaveError::Hotkey(e)) => state.set_hotkey_error(e.as_str().into()),
            Err(SaveError::Config(e)) => {
                tracing::warn!(code = e.code(), "could not save a settings change: {e}");
                state.set_save_error(e.message().into());
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
    let os_dark = app.platform.appearance().dark;
    if model::changes(&app.store.get().settings, os_dark, path, &value) {
        save(app, model::patch(path, value));
        return;
    }
    // Nothing to save; an error from an earlier attempt no longer applies.
    with_open(|open| {
        let state = open.window.global::<ui::SettingsState>();
        state.set_save_error(SharedString::new());
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

/// Handles the native window's events: while `recording-hotkey` is set, the next key pressed
/// with its modifiers becomes the hotkey (SPEC §6.5); Esc, leaving the window or the General
/// page cancels. The keys are read from winit as physical keys, so the keyboard layout does not
/// matter (Ctrl+Alt is AltGr on many layouts). A combination another app holds never reaches
/// the window, so the platform watches for those meanwhile ([`taken_hotkey`]). When the OS
/// switches between light and dark, what follows it shows it (the Fluent palette follows by
/// itself, SPEC §10.3).
fn watch_window_events(window: &ui::SettingsWindow, app: &Arc<App>) {
    use slint::winit_030::winit::event::{ElementState, WindowEvent};
    use slint::winit_030::winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
    use slint::winit_030::{EventResult, WinitWindowAccessor};

    let app = Arc::clone(app);
    let weak = window.as_weak();
    let mut modifiers = ModifiersState::empty();
    let mut watch: Option<crate::platform::HotkeyWatch> = None;
    window.window().on_winit_window_event(move |_, event| {
        let Some(window) = weak.upgrade() else {
            return EventResult::Propagate;
        };
        let state = window.global::<ui::SettingsState>();
        // The watch runs exactly while recording; the window's events come often enough (the
        // modifiers' own presses among them) to start it before a combination is complete.
        let recording = state.get_recording_hotkey() && state.get_page() == GENERAL_PAGE;
        if recording != watch.is_some() {
            watch = if recording {
                let (app, weak) = (Arc::clone(&app), weak.clone());
                crate::platform::watch_taken_hotkeys(Box::new(move |combo| {
                    taken_hotkey(&app, &weak, combo);
                }))
            } else {
                None
            };
        }
        let stop = || {
            state.set_recording_hotkey(false);
            state.set_hotkey_error(hotkey::error().unwrap_or_default().into());
        };
        match event {
            WindowEvent::ThemeChanged(_) => {
                with_open_mut(|open| {
                    let settings = app.store.get().settings;
                    fill(open, &app, &settings);
                });
            }
            // The window's place is kept for when it closes (SPEC §10.7); a maximized or
            // minimized window keeps the one it had before.
            WindowEvent::Moved(_) | WindowEvent::Resized(_) => {
                let place = window.window().with_winit_window(normal_place).flatten();
                if place.is_some() {
                    with_open_mut(|open| open.place = place);
                }
            }
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

/// A combination someone holds was pressed while recording (SPEC §6.5): it ends the recording
/// as a combination the window saw would, so registering it fails with *Another app already
/// uses …* and the previous hotkey stays (or, for Infodock's own hotkey, it is kept). Runs
/// from the keyboard hook, so the work waits for the event loop.
fn taken_hotkey(
    app: &Arc<App>,
    window: &slint::Weak<ui::SettingsWindow>,
    combo: crate::platform::keys::KeyCombo,
) {
    let (app, window) = (Arc::clone(app), window.clone());
    slint::Timer::single_shot(Duration::ZERO, move || {
        let Some(window) = window.upgrade() else {
            return;
        };
        let state = window.global::<ui::SettingsState>();
        if !state.get_recording_hotkey() {
            return;
        }
        let held = hotkey::Modifiers {
            ctrl: combo.ctrl,
            alt: combo.alt,
            shift: combo.shift,
            win: combo.win,
        };
        match hotkey::accelerator(held, combo.code) {
            Ok(accelerator) => {
                state.set_recording_hotkey(false);
                apply_hotkey(&app, &accelerator);
            }
            Err(e) => state.set_hotkey_error(e.into()),
        }
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
    with_open(|open| {
        let error = result.err().unwrap_or_default();
        let state = open.window.global::<ui::SettingsState>();
        state.set_hotkey_error(error.into());
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
