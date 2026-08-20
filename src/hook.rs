/// The Windows low-level keyboard hook manager.
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

use arc_swap::ArcSwap;
use windows::Win32::Foundation::{HMODULE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyboardState, RegisterHotKey, ToUnicode, UnregisterHotKey,
    VIRTUAL_KEY, VK_BACK, VK_CAPITAL, VK_CONTROL, VK_LCONTROL, VK_RCONTROL,
    VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_LEFT, VK_LMENU, VK_LSHIFT,
    VK_MENU, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_RMENU, VK_RSHIFT, VK_SHIFT,
    VK_TAB, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, KBDLLHOOKSTRUCT, PostThreadMessageW,
    SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, MSG, WH_KEYBOARD_LL,
    WM_HOTKEY, WM_KEYDOWN, WM_QUIT, WM_SYSKEYDOWN,
};

use crate::buffer::KeyBuffer;
use crate::config::{AppConfig, ExpansionMode, HotkeyConfig};
use crate::replacer::Replacer;

const EXPANDER_TAG: usize = 0x52455850; // ASCII 'REXP'
const HOTKEY_ID: i32 = 1;
pub const WM_REHOOK: u32 = 0x0400 + 100; // WM_USER + 100 - signal to re-register hotkey

static RECORDING_HOTKEY: AtomicBool = AtomicBool::new(false);
static PAUSED: AtomicBool = AtomicBool::new(false);

type HotkeyCaptureCallback = Box<dyn Fn(HotkeyConfig) + Send + 'static>;
type ModDisplayCallback = Box<dyn Fn(String) + Send + 'static>;

static CAPTURE_CALLBACK: Mutex<Option<HotkeyCaptureCallback>> = Mutex::new(None);
static MOD_DISPLAY_CALLBACK: Mutex<Option<ModDisplayCallback>> = Mutex::new(None);

/// Registers a callback to be invoked directly when a hotkey is recorded.
pub fn set_on_hotkey_captured<F: Fn(HotkeyConfig) + Send + 'static>(f: F) {
    *CAPTURE_CALLBACK.lock().unwrap() = Some(Box::new(f));
}

/// Registers a callback to be invoked directly when modifier keys are pressed during recording.
pub fn set_on_mod_display<F: Fn(String) + Send + 'static>(f: F) {
    *MOD_DISPLAY_CALLBACK.lock().unwrap() = Some(Box::new(f));
}

/// Enables or disables global hotkey capture mode.
pub fn set_recording_hotkey(val: bool) {
    RECORDING_HOTKEY.store(val, Ordering::SeqCst);
}

/// Sets whether text expansion is paused.
pub fn set_paused(val: bool) {
    PAUSED.store(val, Ordering::SeqCst);
    crate::config::log_debug(&format!("Hook: pause state changed to {}", val));
}

/// Returns true if text expansion is currently paused.
pub fn is_paused() -> bool {
    PAUSED.load(Ordering::Relaxed)
}

thread_local! {
    static HOOK_STATE: RefCell<Option<HookState>> = const { RefCell::new(None) };

    // MUST-3 / NEXT-2: Re-entrancy guard explanation
    //
    // When we call SendInput() from inside the keyboard hook callback to inject
    // backspaces or Ctrl+V, the Win32 low-level keyboard hook (WH_KEYBOARD_LL)
    // is SYNCHRONOUSLY re-entered on the SAME THREAD before SendInput returns.
    // This is documented Win32 behaviour: low-level hooks are called inline in the
    // thread that installed them, regardless of whether that thread is already
    // inside a hook callback.
    //
    // A thread-local AtomicBool is the correct tool here:
    //   - Zero overhead: no heap allocation, no cross-thread synchronisation.
    //   - No false sharing: each thread has its own independent copy.
    //   - Accessible from the hook proc without any pointer indirection.
    //
    // An Arc<Mutex<bool>> would be WRONG: the hook thread already holds execution
    // context in the hook proc, so a Mutex::lock() on the same thread would either
    // deadlock (non-reentrant mutex) or silently succeed and double-expand
    // (reentrant mutex). The thread_local pattern avoids both failure modes.
    static INHIBIT_HOOK: AtomicBool = const { AtomicBool::new(false) };
}

/// Called by Replacer before/after SendInput to suppress hook re-entrancy.
pub fn set_inhibit(val: bool) {
    INHIBIT_HOOK.with(|f| f.store(val, Ordering::Relaxed));
}

struct HookState {
    buffer: KeyBuffer,
    config: Arc<ArcSwap<AppConfig>>,
    buffer_debug: Arc<Mutex<String>>,
}

/// Manages the low-level keyboard hook thread.
pub struct HookManager {
    thread_id: u32,
    join_handle: Option<std::thread::JoinHandle<()>>,
    buffer_debug: Arc<Mutex<String>>,
}

impl HookManager {
    /// Starts the hook thread and installs the low-level keyboard hook.
    ///
    /// Returns an error string if the hook could not be installed. This happens most
    /// commonly when an overly aggressive antivirus blocks `SetWindowsHookExW`, or
    /// when the process lacks the required privilege level.
    pub fn start(config: Arc<ArcSwap<AppConfig>>) -> Result<Self, String> {
        let buffer_debug = Arc::new(Mutex::new(String::new()));
        let buffer_debug_clone = buffer_debug.clone();
        // This channel carries either the hook thread ID (success) or an error string
        // (failure) from the spawned thread back to the caller.
        let (tx, rx) = mpsc::channel::<Result<u32, String>>();

        let join_handle = thread::spawn(move || {
            let thread_id = unsafe { GetCurrentThreadId() };

            unsafe {
                let hook_result = SetWindowsHookExW(
                    WH_KEYBOARD_LL,
                    Some(low_level_keyboard_proc),
                    HMODULE::default(),
                    0,
                );

                // MUST-3: Instead of panicking with .expect(), we send the error back
                // to the caller via the channel so it can show a proper UI error dialog.
                let hook_handle = match hook_result {
                    Ok(h) => h,
                    Err(e) => {
                        let _ = tx.send(Err(format!(
                            "SetWindowsHookExW failed: {}. Your antivirus may be blocking keyboard hooks.",
                            e
                        )));
                        return;
                    }
                };

                // Signal success: send the hook thread ID back to the caller.
                let _ = tx.send(Ok(thread_id));
                
                // Read initial config for buffer size and hotkey
                let initial_config = config.load();
                let buffer_size = initial_config.buffer_size;
                
                HOOK_STATE.with(|state| {
                    *state.borrow_mut() = Some(HookState {
                        buffer: KeyBuffer::new(buffer_size),
                        config: config.clone(),
                        buffer_debug: buffer_debug_clone,
                    });
                });
                
                // Register initial hotkey
                let hk = &initial_config.hotkey;
                let modifiers = windows::Win32::UI::Input::KeyboardAndMouse::HOT_KEY_MODIFIERS(hk.modifiers);
                match RegisterHotKey(None, HOTKEY_ID, modifiers, hk.virtual_key) {
                    Ok(_) => crate::config::log_debug(&format!(
                        "Hook: initial RegisterHotKey OK (mods={}, vk=0x{:X})",
                        hk.modifiers, hk.virtual_key
                    )),
                    Err(e) => crate::config::log_debug(&format!(
                        "Hook: initial RegisterHotKey FAILED (mods={}, vk=0x{:X}): {}",
                        hk.modifiers, hk.virtual_key, e
                    )),
                }
                
                let mut msg = MSG::default();
                while GetMessageW(&mut msg, None, 0, 0).into() {
                    if msg.message == WM_QUIT {
                        break;
                    } else if msg.message == WM_REHOOK {
                        // Re-register hotkey from updated ArcSwap config
                        let _ = UnregisterHotKey(None, HOTKEY_ID);
                        let new_config = config.load();
                        let hk = &new_config.hotkey;
                        let modifiers = windows::Win32::UI::Input::KeyboardAndMouse::HOT_KEY_MODIFIERS(hk.modifiers);
                        match RegisterHotKey(None, HOTKEY_ID, modifiers, hk.virtual_key) {
                            Ok(_) => crate::config::log_debug(&format!(
                                "Hook: rehook RegisterHotKey OK (mods={}, vk=0x{:X})",
                                hk.modifiers, hk.virtual_key
                            )),
                            Err(e) => crate::config::log_debug(&format!(
                                "Hook: rehook RegisterHotKey FAILED (mods={}, vk=0x{:X}): {}",
                                hk.modifiers, hk.virtual_key, e
                            )),
                        }
                    } else if msg.message == WM_HOTKEY {
                        if is_paused() {
                            continue;
                        }
                        let mut hotkey_replacement: Option<(usize, String)> = None;
                        HOOK_STATE.with(|state| {
                            if let Some(st) = state.borrow_mut().as_mut() {
                                let conf = st.config.load();
                                let buf_content = st.buffer.content();
                                crate::config::log_debug(&format!(
                                    "Hook WM_HOTKEY: buffer='{}', checking {} snippets",
                                    buf_content, conf.snippets.len()
                                ));
                                for snippet in &conf.snippets {
                                    if snippet.mode == ExpansionMode::Hotkey {
                                        let matches = st.buffer.ends_with(&snippet.trigger);
                                        crate::config::log_debug(&format!(
                                            "  trigger='{}', len={}, ends_with={}",
                                            snippet.trigger, snippet.trigger.len(), matches
                                        ));
                                        if matches {
                                            hotkey_replacement = Some((
                                                snippet.trigger.chars().count(),
                                                snippet.replacement.clone(),
                                            ));
                                            st.buffer.clear();
                                            update_buffer_debug(st);
                                            break;
                                        }
                                    }
                                }
                            }
                        });
                        if let Some((trigger_len, replacement)) = hotkey_replacement {
                            crate::config::log_debug(&format!(
                                "Hook: replacing trigger_len={} with '{}'", trigger_len, replacement
                            ));
                            let delay_ms = {
                                let conf = config.load();
                                conf.clipboard_restore_delay_ms
                            };
                            Replacer::replace_hotkey(trigger_len, &replacement, delay_ms);
                        }
                    }
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                
                UnhookWindowsHookEx(hook_handle).expect("Failed to unhook");
                let _ = UnregisterHotKey(None, HOTKEY_ID);
            }
        });

        let thread_id = rx.recv().expect("Failed to get hook thread id")?;

        Ok(HookManager {
            thread_id,
            join_handle: Some(join_handle),
            buffer_debug,
        })
    }

    /// Returns the thread ID for IPC (PostThreadMessageW).
    pub fn thread_id(&self) -> u32 {
        self.thread_id
    }

    /// Returns a clone of the buffer debug string for UI display.
    pub fn buffer_debug(&self) -> Arc<Mutex<String>> {
        self.buffer_debug.clone()
    }


    /// Stops the hook thread gracefully.
    pub fn stop(&mut self) {
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        if let Some(handle) = self.join_handle.take() {
            let _ = handle.join();
        }
    }
}

/// Updates the shared debug string with current buffer contents.
fn update_buffer_debug(st: &HookState) {
    if let Ok(mut dbg) = st.buffer_debug.try_lock() {
        *dbg = st.buffer.content();
    }
}

unsafe extern "system" fn low_level_keyboard_proc(
    n_code: i32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    if n_code < 0 {
        return unsafe { CallNextHookEx(None, n_code, w_param, l_param) };
    }
    
    // PRIMARY re-entrancy guard: our own SendInput calls.
    if INHIBIT_HOOK.with(|f| f.load(Ordering::Relaxed)) {
        return unsafe { CallNextHookEx(None, n_code, w_param, l_param) };
    }

    // SAFETY: l_param points to a valid KBDLLHOOKSTRUCT when nCode >= 0
    let kbd = unsafe { *(l_param.0 as *const KBDLLHOOKSTRUCT) };

    // SECONDARY guard: dwExtraInfo tag (belt-and-suspenders).
    // Our synthetic keystrokes are tagged — let them pass through to the app.
    if kbd.dwExtraInfo == EXPANDER_TAG {
        return unsafe { CallNextHookEx(None, n_code, w_param, l_param) };
    }

    let message = w_param.0 as u32;
    if message != WM_KEYDOWN && message != WM_SYSKEYDOWN {
        return unsafe { CallNextHookEx(None, n_code, w_param, l_param) };
    }

    let mut ctrl = (unsafe { GetAsyncKeyState(VK_CONTROL.0 as i32) } as u16 & 0x8000) != 0;
    let mut alt = (unsafe { GetAsyncKeyState(VK_MENU.0 as i32) } as u16 & 0x8000) != 0;
    let mut shift = (unsafe { GetAsyncKeyState(VK_SHIFT.0 as i32) } as u16 & 0x8000) != 0;
    let mut win = (unsafe { GetAsyncKeyState(0x5B) } as u16 & 0x8000 != 0) || (unsafe { GetAsyncKeyState(0x5C) } as u16 & 0x8000 != 0);

    // Also account for the current vkCode itself if it's a modifier key
    match kbd.vkCode {
        0x11 | 0xA2 | 0xA3 => ctrl = true,
        0x12 | 0xA4 | 0xA5 => alt = true,
        0x10 | 0xA0 | 0xA1 => shift = true,
        0x5B | 0x5C => win = true,
        _ => {}
    }

    let mut current_modifiers = 0u32;
    if alt { current_modifiers |= 1; }
    if ctrl { current_modifiers |= 2; }
    if shift { current_modifiers |= 4; }
    if win { current_modifiers |= 8; }

    // 1. HOTKEY RECORDING MODE
    if RECORDING_HOTKEY.load(Ordering::Relaxed) {
        let is_modifier_key = match kbd.vkCode {
            0x10 | 0xA0 | 0xA1 => true, // VK_SHIFT, VK_LSHIFT, VK_RSHIFT
            0x11 | 0xA2 | 0xA3 => true, // VK_CONTROL, VK_LCONTROL, VK_RCONTROL
            0x12 | 0xA4 | 0xA5 => true, // VK_MENU, VK_LMENU, VK_RMENU
            0x5B | 0x5C => true,        // VK_LWIN, VK_RWIN
            _ => false,
        };

        crate::config::log_debug(&format!("Hook recording: vk=0x{:X}, mods={}, is_mod={}", kbd.vkCode, current_modifiers, is_modifier_key));

        if is_modifier_key {
            let mut parts = Vec::new();
            if ctrl { parts.push("CTRL"); }
            if alt { parts.push("ALT"); }
            if shift { parts.push("SHIFT"); }
            if win { parts.push("WIN"); }
            if !parts.is_empty() {
                let mod_str = parts.join(" + ");
                if let Ok(guard) = MOD_DISPLAY_CALLBACK.lock()
                    && let Some(cb) = guard.as_ref() {
                        cb(mod_str);
                    }
            }
            return LRESULT(1); // Consume modifier press so Alt doesn't activate window menu
        }

        if kbd.vkCode == 0x1B { // Escape: cancel recording
            set_recording_hotkey(false);
            return LRESULT(1);
        }

        // Non-modifier key pressed: capture the complete combination
        let recorded = HotkeyConfig {
            modifiers: current_modifiers,
            virtual_key: kbd.vkCode,
        };
        crate::config::log_debug(&format!("Hook captured hotkey: {:?}", recorded));
        RECORDING_HOTKEY.store(false, Ordering::SeqCst);

        if let Ok(guard) = CAPTURE_CALLBACK.lock()
            && let Some(cb) = guard.as_ref() {
                cb(recorded);
            }

        return LRESULT(1); // Consume key event
    }

    // 2. IF PAUSED, PASS THROUGH ALL KEYS WITHOUT RECORDING OR EXPANDING
    if is_paused() {
        return unsafe { CallNextHookEx(None, n_code, w_param, l_param) };
    }

    // 3. CHECK CONFIGURED HOTKEY EXPANSION
    let mut is_hotkey_match = false;
    let mut hotkey_replacement: Option<(usize, String)> = None;

    HOOK_STATE.with(|state| {
        let mut state_ref = state.borrow_mut();
        if let Some(st) = state_ref.as_mut() {
            let conf = st.config.load();
            let hk = &conf.hotkey;
            if current_modifiers == hk.modifiers && kbd.vkCode == hk.virtual_key {
                is_hotkey_match = true;
                for snippet in &conf.snippets {
                    if snippet.mode == ExpansionMode::Hotkey
                        && st.buffer.ends_with(&snippet.trigger) {
                            hotkey_replacement = Some((
                                snippet.trigger.chars().count(),
                                snippet.replacement.clone(),
                            ));
                            st.buffer.clear();
                            update_buffer_debug(st);
                            break;
                        }
                }
            }
        }
    });

    if is_hotkey_match {
        if let Some((trigger_len, replacement)) = hotkey_replacement {
            let delay_ms = HOOK_STATE.with(|state| {
                state.borrow().as_ref()
                    .map(|st| st.config.load().clipboard_restore_delay_ms)
                    .unwrap_or(150)
            });
            Replacer::replace_hotkey(trigger_len, &replacement, delay_ms);
        }
        return LRESULT(1); // Consume the hotkey keystroke so it doesn't leak into the app
    }

    // 3. NORMAL KEYSTROKE HANDLING
    let mut consume = false;
    let mut immediate_replacement: Option<(usize, String)> = None;

    HOOK_STATE.with(|state| {
        let mut state_ref = state.borrow_mut();
        if let Some(st) = state_ref.as_mut() {
            let vk = VIRTUAL_KEY(kbd.vkCode as u16);

            // Skip modifier-only keys entirely — they shouldn't affect the buffer.
            // This prevents the buffer from being cleared when the user presses Ctrl
            // as the first key of a hotkey combination like Ctrl+Alt+Shift+O.
            match vk {
                VK_SHIFT | VK_CONTROL | VK_MENU | VK_LSHIFT | VK_RSHIFT |
                VK_LCONTROL | VK_RCONTROL | VK_LMENU | VK_RMENU | VK_CAPITAL => {
                    // Modifier-only keys: ignore completely, don't change buffer
                    // Don't even check ctrl/alt state here
                    return;
                }
                _ => {}
            }

            // If Ctrl is held (with a non-modifier key), clear buffer for editing commands
            // like Ctrl+A, Ctrl+C, Ctrl+Z etc. But NOT if Alt is also held (could be hotkey).
            if ctrl && !alt {
                crate::config::log_debug(&format!("Hook: Ctrl+key clears buffer. vk=0x{:X}", kbd.vkCode));
                st.buffer.clear();
                update_buffer_debug(st);
                return;
            }

            match vk {
                VK_BACK => {
                    st.buffer.pop();
                    update_buffer_debug(st);
                }
                VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN | VK_HOME | VK_END | 
                VK_DELETE | VK_ESCAPE | VK_TAB | VK_RETURN | VK_PRIOR | VK_NEXT => {
                    st.buffer.clear();
                    update_buffer_debug(st);
                }
                // Modifier-only keys already handled above (returned early)
                _ => {
                    let mut keyboard_state = [0u8; 256];
                    unsafe { let _ = GetKeyboardState(&mut keyboard_state); }

                    let mut char_buf = [0u16; 4];
                    // NEXT-4: ToUnicode return values:
                    //   > 0  : the number of UTF-16 code units written to char_buf.
                    //          For BMP characters this is 1; for supplementary-plane
                    //          characters (emoji, historic scripts) it is 2 (surrogate pair).
                    //   = 0  : no character produced (e.g. a dead key was consumed
                    //          and is waiting for the next keystroke).
                    //   < 0  : a dead key was added to the key state but the buffer
                    //          also contains the base character — this is an edge case
                    //          for double-tapping a dead key. We treat it identically
                    //          to result > 0 so we don't silently skip characters.
                    let result = unsafe {
                        ToUnicode(
                            kbd.vkCode,
                            kbd.scanCode,
                            Some(&keyboard_state),
                            &mut char_buf,
                            4, // TOUNICODE_FLAG_MENU: don't modify dead-key state
                        )
                    };

                    // MUST-2: Handle result > 0 (not just == 1) to correctly track
                    // emoji and other supplementary-plane characters that produce a
                    // surrogate pair (result == 2). Skipping result == 2 would cause
                    // the internal buffer to diverge from the visible text, breaking
                    // trigger matching until the user backspaces.
                    if result != 0 {
                        let units_written = result.unsigned_abs() as usize;
                        let slice = &char_buf[..units_written.min(char_buf.len())];
                        for c in char::decode_utf16(slice.iter().copied()).flatten() {
                            if !c.is_control() {
                                crate::config::log_debug(&format!(
                                    "Hook: push '{}' (U+{:04X}), buffer='{}'",
                                    c, c as u32, st.buffer.content()
                                ));
                                st.buffer.push(c);
                                update_buffer_debug(st);

                                let conf = st.config.load();
                                for snippet in &conf.snippets {
                                    if snippet.mode == ExpansionMode::Immediate
                                        && st.buffer.ends_with(&snippet.trigger) {
                                            immediate_replacement = Some((
                                                snippet.trigger.chars().count(),
                                                snippet.replacement.clone(),
                                            ));
                                            st.buffer.clear();
                                            update_buffer_debug(st);
                                            consume = true;
                                            break;
                                        }
                                }
                            }
                        }
                    }
                }
            }
        }
    });

    if let Some((trigger_len, replacement)) = immediate_replacement {
        let delay_ms = HOOK_STATE.with(|state| {
            state.borrow().as_ref()
                .map(|st| st.config.load().clipboard_restore_delay_ms)
                .unwrap_or(150)
        });
        Replacer::replace_immediate(trigger_len, &replacement, delay_ms);
    }

    if consume {
        return LRESULT(1);
    }

    unsafe { CallNextHookEx(None, n_code, w_param, l_param) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pause_state_toggling() {
        set_paused(false);
        assert!(!is_paused());

        set_paused(true);
        assert!(is_paused());

        set_paused(false);
        assert!(!is_paused());
    }
}

