//! The global hotkey (SPEC §6.5): brings the bar to the front or toggles it.
//!
//! The hotkey manager lives on the main thread, whose event loop delivers its messages. Errors are
//! sentences for Settings to show as they are.

use std::cell::RefCell;
use std::str::FromStr;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

use crate::app::App;
use crate::config::model::HotkeyAction;
use crate::i18n::{t, t_args};

struct State {
    current: Option<HotKey>,
    /// Why the configured hotkey is not registered, e.g. another app owns it.
    error: Option<String>,
}

static STATE: Mutex<State> = Mutex::new(State {
    current: None,
    error: None,
});

thread_local! {
    static MANAGER: RefCell<Option<GlobalHotKeyManager>> = const { RefCell::new(None) };
}

fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Creates the hotkey manager and registers the configured hotkey. A taken hotkey does not stop
/// the app; Settings shows the error (SPEC §6.5). Main thread only.
pub fn init(app: &Arc<App>) {
    match GlobalHotKeyManager::new() {
        Ok(manager) => MANAGER.with(|m| *m.borrow_mut() = Some(manager)),
        Err(e) => {
            tracing::warn!("global hotkeys unavailable: {e}");
            state().error = Some(t("settings.general.hotkey.unavailable"));
            return;
        }
    }
    let handler_app = Arc::clone(app);
    GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
        let ours = state().current.is_some_and(|h| h.id() == event.id);
        if ours && event.state == HotKeyState::Pressed {
            on_pressed(&handler_app);
        }
    }));
    let _ = set(&app.store.get().settings.hotkey, true);
}

fn on_pressed(app: &Arc<App>) {
    match app.store.get().settings.hotkey_action {
        HotkeyAction::BringToFront => app.bring_bar_to_front(),
        HotkeyAction::ToggleVisibility => app.toggle_bar(),
    }
}

/// Parses an accelerator such as `Ctrl+Alt+Shift+I` or `Win+Ctrl+1`.
pub fn parse(accelerator: &str) -> Result<HotKey, String> {
    // global-hotkey calls the Windows key Super.
    let tokens: Vec<&str> = accelerator
        .trim()
        .split('+')
        .map(|token| {
            if token.trim().eq_ignore_ascii_case("win") {
                "Super"
            } else {
                token
            }
        })
        .collect();
    HotKey::from_str(&tokens.join("+")).map_err(|e| e.to_string())
}

/// Modifier keys held with a recorded key.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
}

/// The accelerator for a combination recorded in Settings (SPEC §6.5), e.g. `Ctrl+Alt+K`. `code`
/// names the physical key as `KeyboardEvent.code` does (`KeyK`, `Digit1`, `F5`, `ArrowUp`).
/// Without Ctrl, Alt or Win only a function key may stand alone, so that a hotkey never takes a
/// key away from typing.
pub fn accelerator(modifiers: Modifiers, code: &str) -> Result<String, String> {
    let short = |prefix: &str| code.strip_prefix(prefix).filter(|rest| rest.len() == 1);
    let key = short("Key").or_else(|| short("Digit")).unwrap_or(code);
    let function_key = key
        .strip_prefix('F')
        .and_then(|n| n.parse::<u8>().ok())
        .is_some_and(|n| (1..=24).contains(&n));
    let Modifiers {
        ctrl,
        alt,
        shift,
        win,
    } = modifiers;
    if !(ctrl || alt || win || function_key) {
        return Err(t("settings.general.hotkey.needsModifier"));
    }
    let names = [(win, "Win"), (ctrl, "Ctrl"), (alt, "Alt"), (shift, "Shift")];
    let mut parts: Vec<&str> = names
        .into_iter()
        .filter_map(|(held, name)| held.then_some(name))
        .collect();
    parts.push(key);
    let accelerator = parts.join("+");
    parse(&accelerator)
        .map_err(|_| t_args("settings.general.hotkey.unsupported", &[("key", key)]))?;
    Ok(accelerator)
}

/// Registers `accelerator` in place of the current hotkey. On failure the previous hotkey stays
/// registered and the error is returned. It is kept for [`error`] only when `accelerator` is the
/// `configured` hotkey: a refused new combination leaves the configured one as it was. Main
/// thread only.
pub fn set(accelerator: &str, configured: bool) -> Result<(), String> {
    let result = register(accelerator);
    match &result {
        Ok(()) => state().error = None,
        Err(e) => {
            tracing::warn!("hotkey {accelerator:?} not registered: {e}");
            if configured {
                state().error = Some(e.clone());
            }
        }
    }
    result
}

fn register(accelerator: &str) -> Result<(), String> {
    let failed = |error: &str| t_args("settings.general.hotkey.error", &[("error", error)]);
    let hotkey = parse(accelerator).map_err(|e| failed(&e))?;
    if state().current == Some(hotkey) {
        return Ok(());
    }
    MANAGER.with(|m| {
        let manager = m.borrow();
        let Some(manager) = manager.as_ref() else {
            return Err(t("settings.general.hotkey.unavailable"));
        };
        manager.register(hotkey).map_err(|e| match e {
            global_hotkey::Error::AlreadyRegistered(_) => t_args(
                "settings.general.hotkey.taken",
                &[("hotkey", accelerator.trim())],
            ),
            e => failed(&e.to_string()),
        })?;
        if let Some(old) = state().current.replace(hotkey) {
            let _ = manager.unregister(old);
        }
        Ok(())
    })
}

/// Why the configured hotkey is not registered, if it is not.
pub fn error() -> Option<String> {
    state().error.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_default_and_common_accelerators() {
        for a in [
            "Ctrl+Alt+Shift+I",
            "ctrl+alt+shift+i",
            "Alt+Space",
            "Ctrl+Shift+F12",
            " Super+B ",
        ] {
            assert!(parse(a).is_ok(), "{a}: {:?}", parse(a));
        }
    }

    #[test]
    fn parses_the_windows_key() {
        let win = parse("Win+Ctrl+1").expect("parses");
        assert_eq!(Some(win), parse("Super+Ctrl+1").ok());
    }

    fn mods(ctrl: bool, alt: bool, shift: bool, win: bool) -> Modifiers {
        Modifiers {
            ctrl,
            alt,
            shift,
            win,
        }
    }

    #[test]
    fn recorded_combinations_become_accelerators() {
        let cases = [
            (mods(true, true, true, false), "KeyK", "Ctrl+Alt+Shift+K"),
            (mods(true, false, false, true), "Digit1", "Win+Ctrl+1"),
            (mods(false, true, false, false), "ArrowUp", "Alt+ArrowUp"),
            (
                mods(true, false, false, false),
                "Backquote",
                "Ctrl+Backquote",
            ),
            (mods(false, false, false, false), "F9", "F9"),
            (mods(false, false, true, false), "F24", "Shift+F24"),
        ];
        for (m, code, expected) in cases {
            let accelerator = accelerator(m, code);
            assert_eq!(accelerator.as_deref(), Ok(expected));
            assert!(parse(expected).is_ok(), "{expected} parses back");
        }
    }

    #[test]
    fn a_plain_key_needs_ctrl_alt_or_win() {
        assert!(accelerator(mods(false, false, false, false), "KeyA").is_err());
        assert!(accelerator(mods(false, false, true, false), "KeyA").is_err());
        assert!(accelerator(mods(false, false, false, false), "Space").is_err());
    }

    #[test]
    fn keys_the_os_cannot_register_are_refused() {
        assert!(accelerator(mods(true, false, false, false), "IntlBackslash").is_err());
        assert!(accelerator(mods(true, false, false, false), "F25").is_err());
    }

    #[test]
    fn keeps_errors_of_the_configured_hotkey_only() {
        // No manager on this thread, so registering fails.
        assert!(set("Ctrl+Alt+K", false).is_err());
        assert_eq!(
            error(),
            None,
            "a refused new combination is not the configured hotkey"
        );
        assert!(set("Ctrl+Alt+K", true).is_err());
        assert!(error().is_some(), "the configured hotkey is not registered");
        // Refusing another combination keeps the configured hotkey's error.
        let configured = error();
        assert!(set("Ctrl+Alt+J", false).is_err());
        assert_eq!(error(), configured);
        state().error = None;
    }

    #[test]
    fn rejects_garbage() {
        for a in ["", "Ctrl+", "Hyper+Banana", "Ctrl+Alt"] {
            assert!(parse(a).is_err(), "{a} should not parse");
        }
    }
}
