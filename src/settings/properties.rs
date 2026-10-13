//! The properties form in the Settings window (SPEC §6.7): opened for a sensor on the route
//! `sensor/<id>`, generated from its type's schema (`form.rs`). Edits preview live on the bar
//! (150 ms after the last one); Save stores them and Cancel, leaving the page or closing the
//! window drops them, which also removes a sensor the picker has just added (§6.6).

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use slint::{ComponentHandle, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};

use super::form::{Form, FormRow, RowKind, extensions};
use super::{Open, WINDOW};
use crate::app::App;
use crate::i18n::{t, t_args};
use crate::platform::PathRequest;
use crate::sensors::registry::registry;
use crate::ui;

/// The page index of the form in the window.
pub(super) const PAGE: i32 = 4;
/// Edits are previewed this long after the last one (SPEC §6.7).
const PREVIEW_DELAY: Duration = Duration::from_millis(150);

/// An open form.
pub(super) struct FormState {
    form: Form,
    /// The sensor's type, for the file types of its file fields.
    kind: String,
    /// The rows as last shown.
    rows: Vec<FormRow>,
    preview: Timer,
}

fn strings(items: &[String]) -> ModelRc<SharedString> {
    let items: Vec<SharedString> = items.iter().map(SharedString::from).collect();
    ModelRc::new(VecModel::from(items))
}

fn prop_row(row: &FormRow) -> ui::PropRow {
    ui::PropRow {
        key: row.key.into(),
        kind: row.kind.code(),
        label: row.label.as_str().into(),
        help: row.help.as_str().into(),
        visible: row.visible,
        text: row.text.as_str().into(),
        checked: row.checked,
        number: row.number,
        minimum: row.minimum,
        maximum: row.maximum,
        step: row.step,
        unit: row.unit.into(),
        options: strings(&row.options),
        index: row.index,
        color: row.color.as_str().into(),
        error: row.error.as_str().into(),
    }
}

/// Opens sensor `id`'s properties; `new` when the picker has just added it.
pub(super) fn open_form(app: &Arc<App>, id: &str, new: bool) {
    WINDOW.with(|w| {
        let Ok(mut w) = w.try_borrow_mut() else {
            return;
        };
        let Some(open) = w.as_mut() else {
            return;
        };
        cancel(open, app);
        let state = open.window.global::<ui::SettingsState>();
        let config = app.store.get();
        let instance = config.layout.all().find(|s| s.id == id);
        let Some((form, kind)) = instance
            .and_then(|i| Form::open(registry(), i, new).map(|f| (f, i.kind.clone())))
            .map(|(mut form, kind)| {
                form.set_processors(app.platform.logical_processors());
                if form.has_drive_field() {
                    form.set_volumes(app.platform.volumes());
                }
                (form, kind)
            })
        else {
            state.set_save_error(t("settings.props.missing").into());
            state.set_page(2);
            return;
        };
        let rows = form.rows(&open.families);
        let shown: Vec<ui::PropRow> = rows.iter().map(prop_row).collect();
        state.set_props(ModelRc::new(VecModel::from(shown)));
        state.set_props_title(t_args("settings.props.title", &[("name", &form.title)]).into());
        state.set_props_error(SharedString::new());
        state.set_props_valid(true);
        state.set_save_error(SharedString::new());
        open.form = Some(FormState {
            form,
            kind,
            rows,
            preview: Timer::default(),
        });
        state.set_page(PAGE);
    });
}

/// Drops an open form: ends its preview and removes a sensor the picker has just added.
pub(super) fn cancel(open: &mut Open, app: &App) {
    if let Some(state) = open.form.take() {
        state.preview.stop();
        app.end_preview(&state.form.id);
        if state.form.new
            && let Err(e) = app.remove_sensor(&state.form.id)
        {
            tracing::warn!("could not remove the new sensor: {e}");
        }
    }
}

/// Applies an edit to the open form, refreshes the rows it affects and schedules the preview.
/// An invalid value shows an error on its row and disables Save (SPEC §6.7).
fn edit(app: &Arc<App>, change: impl FnOnce(&mut Form, &[String]) -> bool) {
    WINDOW.with(|w| {
        let Ok(mut w) = w.try_borrow_mut() else {
            return;
        };
        let Some(open) = w.as_mut() else {
            return;
        };
        let families = open.families.clone();
        let Some(state) = open.form.as_mut() else {
            return;
        };
        let valid = change(&mut state.form, &families);
        let window = open.window.global::<ui::SettingsState>();
        // An error of an earlier Save no longer applies.
        window.set_props_error(SharedString::new());
        window.set_props_valid(state.form.valid());
        let rows = state.form.rows(&families);
        let model = window.get_props();
        for (i, (new, old)) in rows.iter().zip(&state.rows).enumerate() {
            // A text field shows what is typed; refreshing it would move the cursor.
            let mut compare = new.clone();
            if matches!(new.kind, RowKind::Text | RowKind::File | RowKind::Folder) {
                compare.text.clone_from(&old.text);
            }
            if compare != *old {
                model.set_row_data(i, prop_row(new));
            }
        }
        state.rows = rows;
        if valid {
            let a = Arc::clone(app);
            state
                .preview
                .start(TimerMode::SingleShot, PREVIEW_DELAY, move || preview(&a));
        }
    });
}

/// Shows the form's edits on the bar.
fn preview(app: &App) {
    let pending = WINDOW.with(|w| {
        let w = w.try_borrow().ok()?;
        let state = w.as_ref()?.form.as_ref()?;
        Some((state.form.id.clone(), state.form.changes().clone()))
    });
    if let Some((id, changes)) = pending
        && let Err(e) = app.preview_sensor(&id, &changes)
    {
        tracing::warn!("could not preview sensor {id}: {e}");
    }
}

/// Saves the edits and returns to the Sensors page; on failure the form stays with the error.
fn save(app: &App) {
    let pending = WINDOW.with(|w| {
        let w = w.try_borrow().ok()?;
        let state = w.as_ref()?.form.as_ref()?;
        // Save is disabled while a value is invalid (SPEC §6.7).
        let valid = state.form.valid();
        valid.then(|| (state.form.id.clone(), state.form.changes().clone()))
    });
    let Some((id, changes)) = pending else {
        return;
    };
    let result = app.update_sensor(&id, &changes);
    WINDOW.with(|w| {
        let Ok(mut w) = w.try_borrow_mut() else {
            return;
        };
        let Some(open) = w.as_mut() else {
            return;
        };
        let window = open.window.global::<ui::SettingsState>();
        match result {
            Ok(()) => {
                if let Some(state) = open.form.take() {
                    state.preview.stop();
                }
                window.set_page(2);
            }
            Err(e) => window.set_props_error(e.to_string().into()),
        }
    });
}

/// Lets the user pick the path of a file or folder field.
fn browse(app: &Arc<App>, key: &str) {
    let request = WINDOW.with(|w| {
        let w = w.try_borrow().ok()?;
        let open = w.as_ref()?;
        let state = open.form.as_ref()?;
        let row = state.rows.iter().find(|r| r.key == key)?;
        let folder = row.kind == RowKind::Folder;
        Some((folder, state.kind.clone(), owner(&open.window)))
    });
    let Some((folder, kind, owner)) = request else {
        return;
    };
    let title = t(if folder {
        "settings.props.pickFolder"
    } else {
        "settings.props.pickFile"
    });
    let (types_name, all_files) = (
        t("settings.props.supportedFiles"),
        t("settings.props.allFiles"),
    );
    // The picker runs its own modal loop; no window state is borrowed meanwhile.
    let picked = crate::platform::pick_path(
        owner,
        &PathRequest {
            title: &title,
            folder,
            extensions: extensions(registry(), &kind, key),
            types_name: &types_name,
            all_files: &all_files,
        },
    );
    if let Some(path) = picked {
        set_path(app, key, path);
    }
}

/// Sets the path field `key` to a picked `path` and shows it.
pub(super) fn set_path(app: &Arc<App>, key: &str, path: String) {
    edit(app, |form, _| form.set(key, Value::String(path)));
    // Typing into the box replaced its binding to the row, so updating the row would not show
    // the path: the row is replaced instead, which builds a new box.
    WINDOW.with(|w| {
        let Ok(w) = w.try_borrow() else {
            return;
        };
        if let Some(open) = w.as_ref()
            && let Some(state) = open.form.as_ref()
            && let Some(i) = state.rows.iter().position(|r| r.key == key)
        {
            let model = open.window.global::<ui::SettingsState>().get_props();
            if let Some(rows) = model.as_any().downcast_ref::<VecModel<ui::PropRow>>() {
                rows.remove(i);
                rows.insert(i, prop_row(&state.rows[i]));
            }
        }
    });
}

/// The window's native handle, to make the picker modal to it.
#[cfg(windows)]
fn owner(window: &ui::SettingsWindow) -> Option<isize> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use slint::winit_030::WinitWindowAccessor;
    window
        .window()
        .with_winit_window(|w| match w.window_handle().ok()?.as_raw() {
            RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
            _ => None,
        })
        .flatten()
}

#[cfg(not(windows))]
fn owner(_window: &ui::SettingsWindow) -> Option<isize> {
    None
}

/// Handles the form's callbacks.
pub(super) fn connect(window: &ui::SettingsWindow, app: &Arc<App>) {
    let state = window.global::<ui::SettingsState>();
    let a = Arc::clone(app);
    state.on_prop_text(move |key, text| {
        edit(&a, |form, _| {
            form.set(&key, Value::String(text.trim().to_owned()))
        });
    });
    let a = Arc::clone(app);
    state.on_prop_flag(move |key, value| edit(&a, |form, _| form.set(&key, Value::Bool(value))));
    let a = Arc::clone(app);
    state.on_prop_number(move |key, value| {
        edit(&a, |form, _| form.set(&key, Value::from(f64::from(value))));
    });
    let a = Arc::clone(app);
    state.on_prop_choice(move |key, index| {
        edit(&a, |form, fonts| {
            usize::try_from(index).is_ok_and(|i| form.choose(&key, i, fonts))
        });
    });
    let a = Arc::clone(app);
    state.on_prop_color_mode(move |key, index, color| {
        edit(&a, |form, _| {
            usize::try_from(index).is_ok_and(|i| form.color_mode(&key, i, &color))
        });
    });
    let a = Arc::clone(app);
    state.on_prop_browse(move |key| browse(&a, &key));
    let a = Arc::clone(app);
    state.on_props_save(move || save(&a));
    let a = Arc::clone(app);
    state.on_props_cancel(move || {
        WINDOW.with(|w| {
            if let Ok(mut w) = w.try_borrow_mut()
                && let Some(open) = w.as_mut()
            {
                cancel(open, &a);
                open.window.global::<ui::SettingsState>().set_page(2);
            }
        });
    });
}
