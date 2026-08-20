use std::time::Duration;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    VIRTUAL_KEY,
};
use windows::Win32::System::DataExchange::{
    OpenClipboard, CloseClipboard, EmptyClipboard, SetClipboardData, GetClipboardData,
    CountClipboardFormats, EnumClipboardFormats,
};
use windows::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalUnlock, GlobalSize, GLOBAL_ALLOC_FLAGS,
};
use windows::Win32::Foundation::{HANDLE, HGLOBAL};

/// Magic tag to identify our own synthetic keystrokes in the keyboard hook callback.
/// ASCII representation of 'REXP'. Checked in `low_level_keyboard_proc` to skip
/// our own injected events without re-triggering expansion.
pub const EXPANDER_TAG: usize = 0x52455850;

/// CF_UNICODETEXT clipboard format constant (Win32 predefined format 13).
const CF_UNICODETEXT: u32 = 13;

/// GMEM_MOVEABLE flag for GlobalAlloc: allocates moveable memory.
/// Required by SetClipboardData — the clipboard takes ownership of the handle.
const GMEM_MOVEABLE: u32 = 0x0002;

/// A full snapshot of the clipboard's raw binary data for a single format.
/// We capture this before pasting so we can restore it precisely afterward.
struct ClipboardEntry {
    format: u32,
    data: Vec<u8>,
}

/// The text replacement engine.
///
/// Uses clipboard-based injection (Ctrl+V) for reliable text insertion across all
/// Windows apps, including WinUI 3 / XAML apps like the new Windows 11 Notepad.
/// Per-character `SendInput` (`KEYEVENTF_UNICODE`) drops or duplicates characters
/// in those apps due to their asynchronous XAML input pipeline.
pub struct Replacer;

impl Replacer {
    /// Replaces the trigger text for Immediate mode snippets.
    ///
    /// The last character of the trigger was consumed by the hook (`LRESULT(1)`) and
    /// never reached the target application, so we send `trigger_len - 1` backspaces.
    pub fn replace_immediate(trigger_len: usize, replacement: &str, restore_delay_ms: u64) {
        Self::replace_internal(trigger_len.saturating_sub(1), replacement, restore_delay_ms);
    }

    /// Replaces the trigger text for Hotkey mode snippets.
    ///
    /// All trigger characters reached the target application before the hotkey was
    /// pressed, so we send the full `trigger_len` backspaces.
    pub fn replace_hotkey(trigger_len: usize, replacement: &str, restore_delay_ms: u64) {
        Self::replace_internal(trigger_len, replacement, restore_delay_ms);
    }

    fn replace_internal(backspace_count: usize, replacement: &str, restore_delay_ms: u64) {
        crate::config::log_debug(&format!(
            "Replacer: backspaces={}, replacement='{}', restore_delay={}ms",
            backspace_count, replacement, restore_delay_ms
        ));

        // Set inhibit flag before any SendInput call so the hook callback skips our
        // own synthetic events. This must be done before ANY SendInput call because
        // SendInput re-enters the hook SYNCHRONOUSLY on the same thread.
        crate::hook::set_inhibit(true);

        // Release any held modifier keys before sending backspaces. Without this,
        // backspaces arrive as Ctrl+Backspace (delete-word) or Alt+Backspace (undo)
        // in most applications, causing the wrong amount of text to be deleted.
        Self::release_modifiers();
        std::thread::sleep(Duration::from_millis(10));

        // Delete trigger text with backspaces
        if backspace_count > 0 {
            Self::send_backspaces(backspace_count);
            std::thread::sleep(Duration::from_millis(30));
        }

        // Use clipboard-based injection for the replacement text.
        Self::paste_via_clipboard(replacement, restore_delay_ms);

        // Release inhibit after all operations are complete
        crate::hook::set_inhibit(false);
    }

    /// Pastes text via the clipboard using a full backup/restore cycle.
    ///
    /// Strategy:
    /// 1. Save a snapshot of every current clipboard format (not just CF_UNICODETEXT).
    ///    This is critical: if the user previously copied an image, a file, or a
    ///    rich-text object, we must restore ALL those formats — not just text.
    ///    Simply skipping non-text clipboards would silently destroy the user's data.
    /// 2. Set our replacement text as CF_UNICODETEXT.
    /// 3. Simulate Ctrl+V.
    /// 4. Wait `restore_delay_ms` for the target app to finish reading the clipboard
    ///    (WinUI 3 / XAML paste pipelines are asynchronous).
    /// 5. Restore every previously saved format verbatim.
    fn paste_via_clipboard(text: &str, restore_delay_ms: u64) {
        // --- Step 1: Capture full clipboard snapshot ---
        let saved = Self::backup_all_clipboard_formats();
        crate::config::log_debug(&format!(
            "Replacer: clipboard backed up ({} formats)",
            saved.len()
        ));

        // --- Step 2: Write our text to clipboard ---
        if !Self::set_clipboard_text(text) {
            crate::config::log_debug(
                "Replacer: failed to set clipboard, falling back to SendInput",
            );
            Self::send_unicode_string_with_delay(text, 5);
            return;
        }
        crate::config::log_debug("Replacer: clipboard set, sending Ctrl+V");

        // --- Step 3: Simulate Ctrl+V ---
        std::thread::sleep(Duration::from_millis(10));
        Self::send_ctrl_v();

        // --- Steps 4 & 5: Wait, then restore ---
        // The delay must be long enough for the target application's paste handler to
        // finish reading the clipboard. WinUI 3 apps dispatch paste asynchronously
        // on the XAML thread, so a short delay is required. The value is configurable
        // in config.toml as `clipboard_restore_delay_ms` (default 150 ms).
        if !saved.is_empty() {
            std::thread::sleep(Duration::from_millis(restore_delay_ms));
            crate::config::log_debug("Replacer: restoring clipboard");
            Self::restore_clipboard_formats(saved);
        }
    }

    // -------------------------------------------------------------------------
    // Clipboard helpers — backup / restore ALL formats
    // -------------------------------------------------------------------------

    /// Captures raw binary data for every format currently on the clipboard.
    ///
    /// Returns a list of `ClipboardEntry` values; returns an empty `Vec` on any
    /// failure (e.g. another process holds the clipboard open).
    ///
    /// Only formats backed by HGLOBAL memory are captured. GDI handle formats
    /// (CF_BITMAP, CF_METAFILEPICT, CF_PALETTE) cannot be duplicated via
    /// `GlobalLock` and are skipped; those formats are rare in everyday usage.
    fn backup_all_clipboard_formats() -> Vec<ClipboardEntry> {
        let mut entries = Vec::new();

        unsafe {
            if OpenClipboard(None).is_err() {
                crate::config::log_debug("Replacer: OpenClipboard failed (backup)");
                return entries;
            }

            let format_count = CountClipboardFormats();
            if format_count == 0 {
                let _ = CloseClipboard();
                return entries;
            }

            // Walk every format available on the clipboard.
            let mut fmt = 0u32;
            loop {
                fmt = EnumClipboardFormats(fmt);
                if fmt == 0 {
                    break; // No more formats.
                }

                let handle_result = GetClipboardData(fmt);
                let handle = match handle_result {
                    Ok(h) => h,
                    Err(_) => continue,
                };

                let hglobal = HGLOBAL(handle.0);
                let size = GlobalSize(hglobal);
                if size == 0 {
                    continue;
                }

                let ptr = GlobalLock(hglobal);
                if ptr.is_null() {
                    continue;
                }

                let bytes = std::slice::from_raw_parts(ptr as *const u8, size);
                let data = bytes.to_vec();
                let _ = GlobalUnlock(hglobal);

                entries.push(ClipboardEntry { format: fmt, data });
            }

            let _ = CloseClipboard();
        }

        entries
    }

    /// Restores all clipboard formats from a previously taken snapshot.
    fn restore_clipboard_formats(entries: Vec<ClipboardEntry>) {
        if entries.is_empty() {
            return;
        }

        unsafe {
            if OpenClipboard(None).is_err() {
                crate::config::log_debug("Replacer: OpenClipboard failed (restore)");
                return;
            }

            if EmptyClipboard().is_err() {
                let _ = CloseClipboard();
                return;
            }

            for entry in entries {
                let byte_len = entry.data.len();
                let hmem = match GlobalAlloc(GLOBAL_ALLOC_FLAGS(GMEM_MOVEABLE), byte_len) {
                    Ok(h) => h,
                    Err(_) => continue,
                };

                let ptr = GlobalLock(hmem);
                if ptr.is_null() {
                    continue;
                }
                std::ptr::copy_nonoverlapping(
                    entry.data.as_ptr(),
                    ptr as *mut u8,
                    byte_len,
                );
                let _ = GlobalUnlock(hmem);

                // SetClipboardData transfers ownership of hmem to the clipboard;
                // do NOT call GlobalFree on hmem after this call succeeds.
                if SetClipboardData(entry.format, HANDLE(hmem.0 as _)).is_err() {
                    crate::config::log_debug(&format!(
                        "Replacer: SetClipboardData failed for format {}",
                        entry.format
                    ));
                }
            }

            let _ = CloseClipboard();
        }
    }

    /// Writes a plain-text string to the clipboard as CF_UNICODETEXT.
    /// Returns `true` on success. The clipboard must NOT be open when called.
    fn set_clipboard_text(text: &str) -> bool {
        // Normalise to CRLF as required by the Win32 CF_UNICODETEXT specification.
        // See text_utils::normalise_to_crlf for details and unit tests.
        let normalised = crate::text_utils::normalise_to_crlf(text);
        let wide: Vec<u16> = normalised.encode_utf16().chain(std::iter::once(0)).collect();
        let byte_len = wide.len() * 2;

        unsafe {
            if OpenClipboard(None).is_err() {
                crate::config::log_debug("Replacer: OpenClipboard failed (set_text)");
                return false;
            }

            let success = (|| -> bool {
                if EmptyClipboard().is_err() {
                    return false;
                }

                let hmem = match GlobalAlloc(GLOBAL_ALLOC_FLAGS(GMEM_MOVEABLE), byte_len) {
                    Ok(h) => h,
                    Err(_) => return false,
                };

                let ptr = GlobalLock(hmem);
                if ptr.is_null() {
                    return false;
                }
                std::ptr::copy_nonoverlapping(
                    wide.as_ptr() as *const u8,
                    ptr as *mut u8,
                    byte_len,
                );
                let _ = GlobalUnlock(hmem);

                // SetClipboardData takes ownership of hmem — do NOT free hmem
                SetClipboardData(CF_UNICODETEXT, HANDLE(hmem.0 as _)).is_ok()
            })();

            let _ = CloseClipboard();
            success
        }
    }

    // -------------------------------------------------------------------------
    // SendInput helpers
    // -------------------------------------------------------------------------

    /// Sends Ctrl+V keystroke to trigger a paste from the clipboard.
    fn send_ctrl_v() {
        let inputs = [
            Self::make_vk_input(0xA2, KEYBD_EVENT_FLAGS(0)), // VK_LCONTROL down
            Self::make_vk_input(0x56, KEYBD_EVENT_FLAGS(0)), // VK_V down
            Self::make_vk_input(0x56, KEYEVENTF_KEYUP),      // VK_V up
            Self::make_vk_input(0xA2, KEYEVENTF_KEYUP),      // VK_LCONTROL up
        ];
        unsafe {
            SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
        }
    }

    /// Sends key-up events for any modifier key currently reported as held down.
    ///
    /// Called before sending backspaces to prevent accidental shortcuts:
    /// - Ctrl+Backspace → delete entire word (instead of single char)
    /// - Alt+Backspace → undo in some applications
    fn release_modifiers() {
        use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

        const MODIFIERS: &[(i32, u16)] = &[
            (0xA2, 0xA2), // VK_LCONTROL
            (0xA3, 0xA3), // VK_RCONTROL
            (0xA0, 0xA0), // VK_LSHIFT
            (0xA1, 0xA1), // VK_RSHIFT
            (0xA4, 0xA4), // VK_LMENU (Left Alt)
            (0xA5, 0xA5), // VK_RMENU (Right Alt / AltGr)
            (0x5B, 0x5B), // VK_LWIN
            (0x5C, 0x5C), // VK_RWIN
        ];

        // Fixed-size stack array — no heap allocation.
        // 8 slots cover all possible modifier keys exactly.
        let mut inputs = [Self::make_vk_input(0, KEYBD_EVENT_FLAGS(0)); 8];
        let mut count = 0usize;

        for &(check_vk, send_vk) in MODIFIERS {
            let state = unsafe { GetAsyncKeyState(check_vk) };
            if (state as u16 & 0x8000) != 0 {
                inputs[count] = Self::make_vk_input(send_vk, KEYEVENTF_KEYUP);
                count += 1;
            }
        }

        if count > 0 {
            crate::config::log_debug(&format!(
                "Replacer: releasing {count} modifier key(s)"
            ));
            unsafe {
                SendInput(&inputs[..count], std::mem::size_of::<INPUT>() as i32);
            }
        }
    }

    /// Sends `count` backspace key events with a small inter-key delay.
    fn send_backspaces(count: usize) {
        if count == 0 {
            return;
        }
        for _ in 0..count {
            let inputs = [
                Self::make_vk_input(0x08, KEYBD_EVENT_FLAGS(0)), // VK_BACK down
                Self::make_vk_input(0x08, KEYEVENTF_KEYUP),      // VK_BACK up
            ];
            unsafe {
                SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Fallback: sends text as Unicode synthetic keystrokes using KEYEVENTF_UNICODE.
    ///
    /// Only used when clipboard access fails entirely. Unreliable in WinUI 3 / XAML
    /// apps (drops and duplicates characters due to async pipelines), but preferable
    /// to producing no output at all.
    fn send_unicode_string_with_delay(text: &str, delay_ms: u64) {
        use windows::Win32::UI::Input::KeyboardAndMouse::KEYEVENTF_UNICODE;
        let mut buf = [0u16; 2];

        for c in text.chars() {
            if c == '\n' || c == '\r' {
                let inputs = [
                    Self::make_vk_input(0x0D, KEYBD_EVENT_FLAGS(0)),
                    Self::make_vk_input(0x0D, KEYEVENTF_KEYUP),
                ];
                unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32); }
            } else {
                let encoded = c.encode_utf16(&mut buf);
                for unit in encoded.iter() {
                    let inputs = [
                        Self::make_unicode_input(*unit, KEYEVENTF_UNICODE),
                        Self::make_unicode_input(*unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP),
                    ];
                    unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32); }
                }
            }
            if delay_ms > 0 {
                std::thread::sleep(Duration::from_millis(delay_ms));
            }
        }
    }

    /// Constructs a VK-based INPUT structure (backspace, Enter, Ctrl, etc.)
    fn make_vk_input(vk: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(vk),
                    wScan: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: EXPANDER_TAG,
                },
            },
        }
    }

    /// Constructs a Unicode INPUT structure (KEYEVENTF_UNICODE, wVk=0).
    #[allow(dead_code)]
    fn make_unicode_input(scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0),
                    wScan: scan,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: EXPANDER_TAG,
                },
            },
        }
    }
}
