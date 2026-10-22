//! The open Settings window as the main thread holds it, and the one way to reach it.
//!
//! Callbacks reach the window through [`with_open`] and [`with_open_mut`], which borrow it only
//! for the closure. Slint and winit may call back while a callback runs (a refill can set off
//! another callback, a modal picker runs its own loop): such a nested call finds the window
//! busy, does nothing and logs it, rather than panicking on a second borrow. So a callback
//! takes what it needs, lets go, and only then saves, previews or shows a dialog.

use std::cell::RefCell;

use slint::ComponentHandle;
use slint::winit_030::WinitWindowAccessor;

use super::Open;
use crate::platform::{NativeDialogs, OwnerWindow, PathRequest};
use crate::ui;

thread_local! {
    static WINDOW: RefCell<Option<Open>> = const { RefCell::new(None) };
}

/// Runs `f` on the open window: `None` while Settings is closed, or (logged) while the window
/// is borrowed by a callback this one runs inside.
pub(super) fn with_open<R>(f: impl FnOnce(&Open) -> R) -> Option<R> {
    WINDOW.with(|w| shared(w, f))
}

/// [`with_open`] for changes to the window's state.
pub(super) fn with_open_mut<R>(f: impl FnOnce(&mut Open) -> R) -> Option<R> {
    WINDOW.with(|w| exclusive(w, f))
}

/// The borrow policy of [`with_open`], on any cell.
fn shared<T, R>(cell: &RefCell<Option<T>>, f: impl FnOnce(&T) -> R) -> Option<R> {
    match cell.try_borrow() {
        Ok(value) => value.as_ref().map(f),
        Err(_) => busy(),
    }
}

/// The borrow policy of [`with_open_mut`], on any cell.
fn exclusive<T, R>(cell: &RefCell<Option<T>>, f: impl FnOnce(&mut T) -> R) -> Option<R> {
    match cell.try_borrow_mut() {
        Ok(mut value) => value.as_mut().map(f),
        Err(_) => busy(),
    }
}

fn busy<R>() -> Option<R> {
    tracing::debug!("the Settings window is busy; a nested call does nothing");
    None
}

/// Keeps `open` as the open window, or (`None`) drops the one there is, which frees what it
/// allocated; the old one is dropped once it is no longer borrowed.
pub(super) fn set(open: Option<Open>) {
    let old = WINDOW.with(|w| match w.try_borrow_mut() {
        Ok(mut w) => std::mem::replace(&mut *w, open),
        Err(_) => {
            tracing::warn!("the Settings window is busy; it was not replaced");
            None
        }
    });
    drop(old);
}

/// A handle to the open window, e.g. to show it once no state is borrowed.
pub(super) fn handle() -> Option<ui::SettingsWindow> {
    with_open(|open| open.window.clone_strong())
}

/// The dialogs modal to the open window (unowned while Settings is closed). Main thread only;
/// a dialog runs its own modal loop, so none is shown while the window is borrowed.
pub fn dialogs() -> NativeDialogs {
    NativeDialogs {
        owner: with_open(|open| owner(&open.window)).flatten(),
    }
}

/// The OS picker for a folder, or for a file of `extensions` called `types_name`, modal to the
/// window.
pub(super) fn pick(
    title: &str,
    folder: bool,
    extensions: &[&str],
    types_name: &str,
) -> Option<String> {
    use crate::platform::Dialogs;
    let all_files = crate::i18n::t("settings.props.allFiles");
    dialogs().open_path(&PathRequest {
        title,
        folder,
        extensions,
        types_name,
        all_files: &all_files,
    })
}

/// The native window behind `window`.
fn owner(window: &ui::SettingsWindow) -> Option<OwnerWindow> {
    window
        .window()
        .with_winit_window(|w| OwnerWindow::of(w))
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_calls_do_nothing_instead_of_panicking() {
        let cell = RefCell::new(Some(1));
        // Readers nest; a writer inside a reader, or anything inside a writer, does not run.
        assert_eq!(shared(&cell, |a| shared(&cell, |b| a + b)), Some(Some(2)));
        assert_eq!(shared(&cell, |_| exclusive(&cell, |v| *v = 5)), Some(None));
        assert_eq!(exclusive(&cell, |_| shared(&cell, |v| *v)), Some(None));
        assert_eq!(exclusive(&cell, |_| exclusive(&cell, |v| *v)), Some(None));
        assert_eq!(exclusive(&cell, |v| std::mem::replace(v, 3)), Some(1));
        assert_eq!(shared(&cell, |v| *v), Some(3));
        // Closed: nothing runs.
        let closed: RefCell<Option<i32>> = RefCell::new(None);
        assert_eq!(shared(&closed, |v| *v), None);
        assert_eq!(exclusive(&closed, |v| *v), None);
    }
}
