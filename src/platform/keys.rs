//! Keys as Settings records a hotkey (SPEC §6.5), and the Windows virtual keys behind them: the
//! pure part of noticing a combination another app holds, which never reaches the window.

/// A key pressed with modifiers. `code` names the key as `KeyboardEvent.code` does (`KeyK`,
/// `Digit1`, `F5`, `ArrowUp`), as `hotkey::accelerator` takes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyCombo {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    pub code: &'static str,
}

impl KeyCombo {
    /// A combination a hotkey could be: with Ctrl, Alt or Win, or a function key alone (the
    /// rule of `hotkey::accelerator`).
    pub fn could_be_hotkey(&self) -> bool {
        self.ctrl || self.alt || self.win || self.code.starts_with('F') && self.code.len() > 1
    }
}

const LETTERS: [&str; 26] = [
    "KeyA", "KeyB", "KeyC", "KeyD", "KeyE", "KeyF", "KeyG", "KeyH", "KeyI", "KeyJ", "KeyK", "KeyL",
    "KeyM", "KeyN", "KeyO", "KeyP", "KeyQ", "KeyR", "KeyS", "KeyT", "KeyU", "KeyV", "KeyW", "KeyX",
    "KeyY", "KeyZ",
];
const DIGITS: [&str; 10] = [
    "Digit0", "Digit1", "Digit2", "Digit3", "Digit4", "Digit5", "Digit6", "Digit7", "Digit8",
    "Digit9",
];
const NUMPAD: [&str; 10] = [
    "Numpad0", "Numpad1", "Numpad2", "Numpad3", "Numpad4", "Numpad5", "Numpad6", "Numpad7",
    "Numpad8", "Numpad9",
];
const FUNCTION: [&str; 24] = [
    "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12", "F13", "F14", "F15",
    "F16", "F17", "F18", "F19", "F20", "F21", "F22", "F23", "F24",
];

/// The `KeyboardEvent.code` name of the Windows virtual key `vk`, as the hotkey registration
/// maps names back to virtual keys; `None` for modifiers and keys a hotkey here does not use.
pub fn code_of_vk(vk: u32) -> Option<&'static str> {
    let index = |first: u32| (vk - first) as usize;
    Some(match vk {
        0x41..=0x5A => LETTERS[index(0x41)],
        0x30..=0x39 => DIGITS[index(0x30)],
        0x60..=0x69 => NUMPAD[index(0x60)],
        0x70..=0x87 => FUNCTION[index(0x70)],
        0x08 => "Backspace",
        0x09 => "Tab",
        0x0D => "Enter",
        0x13 => "Pause",
        0x20 => "Space",
        0x21 => "PageUp",
        0x22 => "PageDown",
        0x23 => "End",
        0x24 => "Home",
        0x25 => "ArrowLeft",
        0x26 => "ArrowUp",
        0x27 => "ArrowRight",
        0x28 => "ArrowDown",
        0x2C => "PrintScreen",
        0x2D => "Insert",
        0x2E => "Delete",
        0xBA => "Semicolon",
        0xBB => "Equal",
        0xBC => "Comma",
        0xBD => "Minus",
        0xBE => "Period",
        0xBF => "Slash",
        0xC0 => "Backquote",
        0xDB => "BracketLeft",
        0xDC => "Backslash",
        0xDD => "BracketRight",
        0xDE => "Quote",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_keys_have_the_names_settings_records() {
        assert_eq!(code_of_vk(0x4B), Some("KeyK"));
        assert_eq!(code_of_vk(0x41), Some("KeyA"));
        assert_eq!(code_of_vk(0x5A), Some("KeyZ"));
        assert_eq!(code_of_vk(0x31), Some("Digit1"));
        assert_eq!(code_of_vk(0x70), Some("F1"));
        assert_eq!(code_of_vk(0x87), Some("F24"));
        assert_eq!(code_of_vk(0x26), Some("ArrowUp"));
        assert_eq!(code_of_vk(0x65), Some("Numpad5"));
        assert_eq!(code_of_vk(0xC0), Some("Backquote"));
        // Modifiers and keys without a hotkey name.
        for vk in [0x10, 0x11, 0x12, 0x5B, 0x5C, 0xA0, 0xA2, 0xA4, 0x1B, 0xFF] {
            assert_eq!(code_of_vk(vk), None, "{vk:#x}");
        }
    }

    #[test]
    fn a_hotkey_needs_a_modifier_or_a_function_key() {
        let combo = |ctrl, code| KeyCombo {
            ctrl,
            alt: false,
            shift: false,
            win: false,
            code,
        };
        assert!(combo(true, "KeyK").could_be_hotkey());
        assert!(combo(false, "F5").could_be_hotkey());
        assert!(!combo(false, "KeyK").could_be_hotkey());
        assert!(
            !KeyCombo {
                shift: true,
                ..combo(false, "KeyF")
            }
            .could_be_hotkey(),
            "Shift is not enough"
        );
    }
}
