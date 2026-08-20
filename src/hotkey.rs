//! Hotkey validation logic.
//!
//! This module is part of the **library** crate so that `cargo test` picks up
//! the unit tests without being blocked by the `#[windows_subsystem = "windows"]`
//! attribute on the binary crate's `main.rs`.

use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS};

use crate::config::HotkeyConfig;

/// Temporary hotkey ID used during conflict-check registration (never the live ID).
const TEST_HOTKEY_ID: i32 = 999;

/// Validates whether a hotkey configuration is permitted and checks for
/// conflicts with other running applications.
///
/// Returns `Ok(())` if the hotkey is acceptable, or `Err(reason)` with a
/// human-readable message in the language specified by `lang`.
pub fn validate_hotkey(hk: &HotkeyConfig, lang: &str) -> Result<(), String> {
    let strings = crate::i18n::get_strings(lang);

    // Rule 1: Cannot have 0 modifiers unless it's a Function key (F1-F12: 0x70..=0x7B)
    if hk.modifiers == 0
        && !(0x70..=0x7B).contains(&hk.virtual_key) {
            return Err(strings.err_needs_mod.to_string());
        }

    // Rule 2: Single-modifier Ctrl (without Alt or Shift) is restricted to prevent
    // hijacking standard editing shortcuts (Ctrl+C, Ctrl+V, Ctrl+A, Ctrl+Z, etc.)
    if hk.modifiers == 2 { // MOD_CONTROL only
        match hk.virtual_key {
            0x41..=0x5A | 0x30..=0x39 | 0x20 | 0x09 | 0x0D | 0x1B | 0x25..=0x28 => {
                return Err(strings.err_ctrl_reserved.to_string());
            }
            _ => {}
        }
    }

    // Rule 3: Single-modifier Alt reserved system shortcuts
    if hk.modifiers == 1 { // MOD_ALT only
        match hk.virtual_key {
            0x09 | 0x73 | 0x20 | 0x0D | 0x1B => return Err(strings.err_sys_reserved.to_string()),
            _ => {}
        }
    }

    // Rule 4: Single-modifier Win reserved system shortcuts
    if hk.modifiers == 8 { // MOD_WIN only
        match hk.virtual_key {
            0x4C | 0x44 | 0x45 | 0x52 | 0x53 | 0x58 | 0x49 | 0x41 | 0x4E | 0x56 | 0x50 | 0x09 => {
                return Err(strings.err_win_reserved.to_string());
            }
            _ => {}
        }
    }

    // Rule 5: Test Windows Global Hotkey registration for conflicts with other apps
    let modifiers = HOT_KEY_MODIFIERS(hk.modifiers);
    let test_reg = unsafe {
        RegisterHotKey(None, TEST_HOTKEY_ID, modifiers, hk.virtual_key)
    };

    if test_reg.is_err() {
        return Err(strings.err_conflict.to_string());
    }

    unsafe {
        let _ = UnregisterHotKey(None, TEST_HOTKEY_ID);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_hotkey_rejects_ctrl_v() {
        let hk = HotkeyConfig {
            modifiers: 2, // MOD_CONTROL
            virtual_key: 0x56, // 'V'
        };
        assert!(validate_hotkey(&hk, "en").is_err());
    }

    #[test]
    fn test_validate_hotkey_rejects_ctrl_c() {
        let hk = HotkeyConfig {
            modifiers: 2, // MOD_CONTROL
            virtual_key: 0x43, // 'C'
        };
        assert!(validate_hotkey(&hk, "en").is_err());
    }

    #[test]
    fn test_validate_hotkey_rejects_alt_f4() {
        let hk = HotkeyConfig {
            modifiers: 1, // MOD_ALT
            virtual_key: 0x73, // F4
        };
        assert!(validate_hotkey(&hk, "en").is_err());
    }

    #[test]
    fn test_validate_hotkey_rejects_win_l() {
        let hk = HotkeyConfig {
            modifiers: 8, // MOD_WIN
            virtual_key: 0x4C, // 'L'
        };
        assert!(validate_hotkey(&hk, "en").is_err());
    }

    #[test]
    fn test_validate_hotkey_rejects_bare_character() {
        let hk = HotkeyConfig {
            modifiers: 0,
            virtual_key: 0x41, // 'A'
        };
        assert!(validate_hotkey(&hk, "en").is_err());
    }

    #[test]
    fn test_validate_hotkey_allows_function_key() {
        let hk = HotkeyConfig {
            modifiers: 0,
            virtual_key: 0x78, // F9
        };
        assert!(validate_hotkey(&hk, "en").is_ok());
    }

    #[test]
    fn test_validate_hotkey_allows_ctrl_shift_o() {
        let hk = HotkeyConfig {
            modifiers: 6, // MOD_CONTROL (2) | MOD_SHIFT (4)
            virtual_key: 0x4F, // 'O'
        };
        assert!(validate_hotkey(&hk, "en").is_ok());
    }

    #[test]
    fn test_validate_hotkey_allows_alt_shift_x() {
        let hk = HotkeyConfig {
            modifiers: 5, // MOD_ALT (1) | MOD_SHIFT (4)
            virtual_key: 0x58, // 'X'
        };
        assert!(validate_hotkey(&hk, "en").is_ok());
    }
}
