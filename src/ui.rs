use std::rc::Rc;
use std::sync::{Arc, Mutex};
use arc_swap::ArcSwap;
use slint::{Model, Timer, TimerMode, VecModel, SharedString, ComponentHandle, LogicalSize, LogicalPosition};
use windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW;
use windows::Win32::Foundation::{WPARAM, LPARAM};

use crate::config::{self, AppConfig, Snippet, ExpansionMode, HotkeyConfig};
use crate::hook::WM_REHOOK;

slint::include_modules!();

// ---------------------------------------------------------------------------
// Win32 virtual key code constants
// ---------------------------------------------------------------------------
// Defined once here and shared between hotkey_display_string() and
// parse_key_event() to eliminate duplicated magic hex literals.
const VK_TAB:    u32 = 0x09;
const VK_RETURN: u32 = 0x0D;
const VK_ESCAPE: u32 = 0x1B;
const VK_SPACE:  u32 = 0x20;
const VK_BACK:   u32 = 0x08;
const VK_F1:  u32 = 0x70; const VK_F2:  u32 = 0x71; const VK_F3:  u32 = 0x72;
const VK_F4:  u32 = 0x73; const VK_F5:  u32 = 0x74; const VK_F6:  u32 = 0x75;
const VK_F7:  u32 = 0x76; const VK_F8:  u32 = 0x77; const VK_F9:  u32 = 0x78;
const VK_F10: u32 = 0x79; const VK_F11: u32 = 0x7A; const VK_F12: u32 = 0x7B;
const VK_HOME:   u32 = 0x24; const VK_END:    u32 = 0x23;
const VK_PRIOR:  u32 = 0x21; const VK_NEXT:   u32 = 0x22;
const VK_DELETE: u32 = 0x2E;
const VK_LEFT:   u32 = 0x25; const VK_RIGHT:  u32 = 0x27;
const VK_UP:     u32 = 0x26; const VK_DOWN:   u32 = 0x28;
// Slint Private-Use-Area codes for function/arrow/nav keys
const SLINT_F1:    u32 = 0xF704; const SLINT_F2:    u32 = 0xF705;
const SLINT_F3:    u32 = 0xF706; const SLINT_F4:    u32 = 0xF707;
const SLINT_F5:    u32 = 0xF708; const SLINT_F6:    u32 = 0xF709;
const SLINT_F7:    u32 = 0xF70A; const SLINT_F8:    u32 = 0xF70B;
const SLINT_F9:    u32 = 0xF70C; const SLINT_F10:   u32 = 0xF70D;
const SLINT_F11:   u32 = 0xF70E; const SLINT_F12:   u32 = 0xF70F;
const SLINT_UP:    u32 = 0xF700; const SLINT_DOWN:  u32 = 0xF701;
const SLINT_LEFT:  u32 = 0xF702; const SLINT_RIGHT: u32 = 0xF703;
const SLINT_HOME:  u32 = 0xF729; const SLINT_END:   u32 = 0xF72B;
const SLINT_PGUP:  u32 = 0xF72C; const SLINT_PGDN:  u32 = 0xF72D;
const SLINT_DEL:   u32 = 0xF728;
// Slint modifier PUA range — filter these out during capture
#[allow(dead_code)] const SLINT_MOD_LOW: u32 = 0xF720;
#[allow(dead_code)] const SLINT_MOD_HIGH: u32 = 0xF72F;

/// Terminates the application cleanly from any callback context.
///
/// Stops the hook message loop (via WM_QUIT) and exits the Slint event loop.
fn graceful_shutdown(hook_thread_id: u32) {
    crate::hook::set_recording_hotkey(false);
    unsafe {
        let _ = PostThreadMessageW(
            hook_thread_id,
            windows::Win32::UI::WindowsAndMessaging::WM_QUIT,
            WPARAM(0),
            LPARAM(0),
        );
    }
    let _ = slint::quit_event_loop();
}
/// Converts a Rust `AppConfig` to a vector of `SnippetModel` for Slint.
fn config_to_snippet_models(config: &AppConfig) -> Vec<SnippetModel> {
    config.snippets.iter().map(|s| SnippetModel {
        trigger: SharedString::from(&*s.trigger),
        replacement: SharedString::from(&*s.replacement),
        mode: SharedString::from(match s.mode {
            ExpansionMode::Immediate => "immediate",
            ExpansionMode::Hotkey => "hotkey",
        }),
    }).collect()
}

/// Formats the global hotkey as a human-readable string.
fn hotkey_display_string(hotkey: &HotkeyConfig) -> SharedString {
    let mut parts = Vec::new();
    let m = hotkey.modifiers;

    // MOD_ALT=1, MOD_CONTROL=2, MOD_SHIFT=4, MOD_WIN=8
    if m & 2 != 0 { parts.push("CTRL"); }
    if m & 1 != 0 { parts.push("ALT"); }
    if m & 4 != 0 { parts.push("SHIFT"); }
    if m & 8 != 0 { parts.push("WIN"); }

    // Map common virtual key codes to names
    let key_name = match hotkey.virtual_key {
        0x41..=0x5A => {
            let ch = (hotkey.virtual_key as u8) as char;
            String::from(ch)
        }
        0x30..=0x39 => {
            let ch = (hotkey.virtual_key as u8) as char;
            String::from(ch)
        }
        VK_F1  => "F1".into(),
        VK_F2  => "F2".into(),
        VK_F3  => "F3".into(),
        VK_F4  => "F4".into(),
        VK_F5  => "F5".into(),
        VK_F6  => "F6".into(),
        VK_F7  => "F7".into(),
        VK_F8  => "F8".into(),
        VK_F9  => "F9".into(),
        VK_F10 => "F10".into(),
        VK_F11 => "F11".into(),
        VK_F12 => "F12".into(),
        _ => format!("0x{:02X}", hotkey.virtual_key),
    };
    parts.push(&key_name);

    SharedString::from(parts.join("+"))
}

/// Parses a key event from Slint into a Win32 HotkeyConfig.
/// Returns None for modifier-only keys (Ctrl, Shift, Alt, Win) so the caller
/// can show intermediate modifier display instead of completing the capture.
fn parse_key_event(text: &str, mut ctrl: bool, alt: bool, shift: bool, win: bool) -> Option<HotkeyConfig> {
    let ch = text.chars().next()?;
    let code = ch as u32;

    // Filter out modifier-only key events.
    // Slint sends these as their Win32 VK codes (which happen to be ASCII control chars):
    //   Shift = 0x10 (16), Ctrl = 0x11 (17), Alt/Menu = 0x12 (18)
    //   CapsLock = 0x14, LWin = 0x5B, RWin = 0x5C
    // Also filter Slint Private Use Area modifier codes (0xF720..0xF72F).
    match code {
        0x10 | 0xA0 | 0xA1 => return None, // Shift, LShift, RShift
        0x11 | 0xA2 | 0xA3 => return None, // Control, LControl, RControl
        0x12 | 0xA4 | 0xA5 => return None, // Alt/Menu, LAlt, RAlt
        0x14 => return None,                 // CapsLock
        0x5B | 0x5C => return None,          // LWin, RWin
        0xF720..=0xF72F => return None,      // Slint PUA modifier keys
        _ => {}
    }

    let vk = match code {
        VK_TAB                => VK_TAB,    // Tab
        0x0D | 0x0A           => VK_RETURN, // Enter / LF
        VK_ESCAPE             => VK_ESCAPE, // Escape
        VK_SPACE              => VK_SPACE,  // Space
        VK_BACK               => VK_BACK,   // Backspace
        1..=26 => {
            // Remaining ASCII control characters = Ctrl+letter
            // (modifier codes 0x10/0x11/0x12 already filtered above)
            // e.g. Ctrl+O → 0x0F (15), Ctrl+A → 0x01 (1)
            ctrl = true;
            0x41 + (code - 1)
        }
        0x41..=0x5A => code,        // 'A' .. 'Z'
        0x61..=0x7A => code - 0x20, // 'a' .. 'z' -> 'A' .. 'Z'
        0x30..=0x39 => code,        // '0' .. '9'
        // Slint Function keys (PUA) → Win32 VK codes
        SLINT_F1  => VK_F1,  SLINT_F2  => VK_F2,  SLINT_F3  => VK_F3,
        SLINT_F4  => VK_F4,  SLINT_F5  => VK_F5,  SLINT_F6  => VK_F6,
        SLINT_F7  => VK_F7,  SLINT_F8  => VK_F8,  SLINT_F9  => VK_F9,
        SLINT_F10 => VK_F10, SLINT_F11 => VK_F11, SLINT_F12 => VK_F12,
        // Slint Arrow / Navigation keys
        SLINT_UP    => VK_UP,    SLINT_DOWN  => VK_DOWN,
        SLINT_LEFT  => VK_LEFT,  SLINT_RIGHT => VK_RIGHT,
        SLINT_HOME  => VK_HOME,  SLINT_END   => VK_END,
        SLINT_PGUP  => VK_PRIOR, SLINT_PGDN  => VK_NEXT,
        SLINT_DEL   => VK_DELETE,
        _ => {
            if ch.is_ascii_graphic() {
                ch.to_ascii_uppercase() as u32
            } else {
                return None;
            }
        }
    };

    let mut modifiers = 0u32;
    if alt { modifiers |= 1; }
    if ctrl { modifiers |= 2; }
    if shift { modifiers |= 4; }
    if win { modifiers |= 8; }

    Some(HotkeyConfig {
        modifiers,
        virtual_key: vk,
    })
}

/// Applies all i18n strings to the ConfigWindow and AppTray based on the given language code.
fn apply_language(window: &ConfigWindow, tray: &AppTray, lang: &str) {
    let s = crate::i18n::get_strings(lang);
    window.set_window_title_text(SharedString::from(s.window_title));
    window.set_i18n_header(SharedString::from(s.header));
    window.set_i18n_hotkey_label(SharedString::from(s.hotkey_label));
    window.set_i18n_hotkey_save(SharedString::from(s.hotkey_save));
    window.set_i18n_hotkey_prompt(SharedString::from(s.hotkey_prompt));
    window.set_i18n_buffer_label(SharedString::from(s.buffer_label));
    window.set_i18n_buffer_empty(SharedString::from(s.buffer_empty));
    window.set_i18n_col_trigger(SharedString::from(s.col_trigger));
    window.set_i18n_col_trigger_tooltip(SharedString::from(s.col_trigger_tooltip));
    window.set_i18n_col_replacement(SharedString::from(s.col_replacement));
    window.set_i18n_col_mode(SharedString::from(s.col_mode));
    window.set_i18n_mode_immediate(SharedString::from(s.mode_immediate));
    window.set_i18n_mode_hotkey(SharedString::from(s.mode_hotkey));
    window.set_i18n_btn_delete(SharedString::from(s.btn_delete));
    window.set_i18n_btn_add(SharedString::from(s.btn_add));
    window.set_i18n_btn_quit(SharedString::from(s.btn_quit));
    window.set_i18n_btn_pause(SharedString::from(s.btn_pause));
    window.set_i18n_btn_resume(SharedString::from(s.btn_resume));
    window.set_i18n_btn_pause_tooltip(SharedString::from(s.btn_pause_tooltip));
    window.set_i18n_btn_cancel(SharedString::from(s.btn_cancel));
    window.set_i18n_btn_save(SharedString::from(s.btn_save));
    window.set_i18n_current_lang(SharedString::from(
        if lang == "da" { "Dansk" } else { "English" }
    ));
    window.set_i18n_uninstall_btn(SharedString::from(s.uninstall_btn));
    window.set_i18n_uninstall_tooltip(SharedString::from(s.uninstall_tooltip));
    window.set_i18n_btn_cancel_tooltip(SharedString::from(s.btn_cancel_tooltip));
    tray.set_tray_tooltip_text(SharedString::from(s.tray_tooltip));
    tray.set_tray_open_text(SharedString::from(s.tray_open));
    tray.set_tray_quit_text(SharedString::from(s.tray_quit));
}

/// Sets up the Slint UI and runs the event loop.
/// This blocks the current thread until the application exits.
pub fn setup_and_run(
    config: Arc<ArcSwap<AppConfig>>,
    hook_thread_id: u32,
    buffer_debug: Arc<Mutex<String>>,
    show_settings_on_start: bool,
) -> Result<(), slint::PlatformError> {
    let window = ConfigWindow::new()?;
    let tray = AppTray::new()?;

    // Load initial model
    let current_config = config.load();
    let snippets = config_to_snippet_models(&current_config);
    let snippets_model = Rc::new(VecModel::from(snippets));
    
    window.set_snippets(snippets_model.clone().into());
    window.set_hotkey_display(hotkey_display_string(&current_config.hotkey));
    let cfg_path = config::config_path().to_string_lossy().to_string();
    window.set_config_file_path(SharedString::from(cfg_path));

    // Apply language from config
    apply_language(&window, &tray, &current_config.language);

    if show_settings_on_start {
        let _ = window.show();
        window.window().set_size(LogicalSize::new(722.0, 485.0));
    }

    // State for capturing hotkey
    let pending_hotkey: Arc<Mutex<Option<HotkeyConfig>>> = Arc::new(Mutex::new(None));

    // Runtime-only window geometry memory (not persisted to disk).
    // Populated on Save; restored on the next tray reopen within the same session.
    // Reverts to the default 722x485 on app restart.
    let saved_size: Arc<Mutex<Option<LogicalSize>>> = Arc::new(Mutex::new(None));
    let saved_pos:  Arc<Mutex<Option<LogicalPosition>>> = Arc::new(Mutex::new(None));

    // Register direct callbacks from the keyboard hook thread to the Slint UI thread
    let window_weak_for_capture = window.as_weak();
    let pending_hotkey_for_capture = pending_hotkey.clone();
    crate::hook::set_on_hotkey_captured(move |hk| {
        let w_weak = window_weak_for_capture.clone();
        let pending = pending_hotkey_for_capture.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(w) = w_weak.upgrade() {
                let display = hotkey_display_string(&hk);
                w.set_hotkey_display(display);
                w.set_hotkey_capturing(false);
                w.set_hotkey_conflict(false);
                w.set_hotkey_can_save(true);
                *pending.lock().unwrap() = Some(hk);
            }
        });
    });

    let window_weak_for_mod = window.as_weak();
    crate::hook::set_on_mod_display(move |mod_str| {
        let w_weak = window_weak_for_mod.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(w) = w_weak.upgrade()
                && w.get_hotkey_capturing() {
                    w.set_hotkey_display(SharedString::from(format!("{} + ...", mod_str)));
                }
        });
    });

    // Timer to poll buffer contents debug display (every 50ms)
    let buffer_timer = Timer::default();
    let window_weak = window.as_weak();
    let buffer_debug_clone = buffer_debug.clone();
    buffer_timer.start(TimerMode::Repeated, std::time::Duration::from_millis(50), move || {
        if let Some(w) = window_weak.upgrade()
            && let Ok(content) = buffer_debug_clone.try_lock() {
                w.set_buffer_content(SharedString::from(content.as_str()));
            }
    });

    // Tray callbacks
    let window_weak = window.as_weak();
    let config_clone_for_tray = config.clone();
    let pending_hotkey_for_tray = pending_hotkey.clone();
    let saved_size_for_tray = saved_size.clone();
    let saved_pos_for_tray  = saved_pos.clone();
    tray.on_open_settings(move || {
        if let Some(w) = window_weak.upgrade() {
            // Reset hotkey capture state on show
            crate::hook::set_recording_hotkey(false);
            w.set_hotkey_capturing(false);
            w.set_hotkey_conflict(false);
            w.set_hotkey_can_save(false);
            let current = config_clone_for_tray.load();
            w.set_hotkey_display(hotkey_display_string(&current.hotkey));
            *pending_hotkey_for_tray.lock().unwrap() = None;
            // Clear any stale save-error and reload snippets from current config
            w.set_save_error_message(SharedString::from(""));

            // set_position() MUST come before show(). Win32's SetWindowPos on a
            // HIDDEN window is a pure metadata update — no WM_PAINT, no surface blit.
            // If called AFTER show(), SetWindowPos performs a synchronous blit of the
            // current surface bits (which are white/empty immediately after show()) to
            // the new position. Win32 then considers those regions "clean" and suppresses
            // the pending WM_PAINT, leaving a permanently blank white window that
            // request_redraw() cannot recover because the coalesced paint covers zero
            // dirty pixels. Positioning first, while hidden, avoids the blit entirely.
            if let Some(pos) = *saved_pos_for_tray.lock().unwrap() {
                w.window().set_position(pos);
            }

            // show() MUST come before set_size(): the window needs to be visible
            // (and attached to its monitor) so that Slint/Win32 can resolve the
            // correct DPI scale factor before applying the logical size. Calling
            // set_size on a hidden window uses a stale/zero DPI factor which
            // squeezes content into the bottom-right corner on re-open.
            let _ = w.show();

            // Restore size from last Save, or use defaults.
            if let Some(sz) = *saved_size_for_tray.lock().unwrap() {
                w.window().set_size(sz);
            } else {
                w.window().set_size(LogicalSize::new(722.0, 485.0));
            }

            // Bring window to foreground and restore if minimized.
            // FindWindowW locates our HWND by title (Slint doesn't expose HWND directly).
            use windows::Win32::UI::WindowsAndMessaging::{
                SetForegroundWindow, ShowWindow, SW_RESTORE, IsIconic, FindWindowW,
            };
            use windows::core::PCWSTR;
            unsafe {
                let title_str = w.get_window_title_text();
                let wide: Vec<u16> = title_str.encode_utf16().chain(std::iter::once(0)).collect();
                let hwnd = FindWindowW(PCWSTR::null(), PCWSTR(wide.as_ptr()));
                if let Ok(hwnd) = hwnd {
                    if IsIconic(hwnd).as_bool() {
                        let _ = ShowWindow(hwnd, SW_RESTORE);
                    }
                    let _ = SetForegroundWindow(hwnd);
                }
            }
        }
    });

    let htid = hook_thread_id;
    tray.on_quit_app(move || graceful_shutdown(htid));

    // Quit button in settings window — same behavior as tray quit
    let htid = hook_thread_id;
    window.on_quit_app(move || graceful_shutdown(htid));

    // Trash icon — show a Win32 confirmation dialog, then self-destruct if confirmed
    let htid = hook_thread_id;
    let config_for_uninstall = config.clone();
    window.on_uninstall_app(move || {
        // Pick dialog strings based on current language
        let lang = config_for_uninstall.load().language.clone();
        let s = crate::i18n::get_strings(&lang);

        // Show a native Win32 Yes/No MessageBox — blocks the Slint thread until dismissed
        let confirmed = unsafe {
            use windows::Win32::UI::WindowsAndMessaging::{
                MessageBoxW, MB_ICONWARNING, MB_YESNO, MB_DEFBUTTON2, IDYES,
            };
            use windows::core::PCWSTR;

            let title: Vec<u16> = s.uninstall_title.encode_utf16()
                .chain(std::iter::once(0)).collect();
            let body: Vec<u16> = s.uninstall_body.encode_utf16()
                .chain(std::iter::once(0)).collect();

            // MB_DEFBUTTON2 makes "No" the default so Enter doesn't accidentally confirm
            let result = MessageBoxW(
                None,
                PCWSTR(body.as_ptr()),
                PCWSTR(title.as_ptr()),
                MB_ICONWARNING | MB_YESNO | MB_DEFBUTTON2,
            );
            result == IDYES
        };

        if confirmed {
            crate::config::log_debug("UI: user confirmed uninstall — starting self-destruct");
            match crate::uninstall::self_destruct(htid) {
                Ok(()) => {}
                Err(e) => {
                    // If self_destruct fails we just log — can't show UI at this point
                    crate::config::log_debug(&format!("uninstall failed: {}", e));
                    eprintln!("uninstall failed: {}", e);
                }
            }
        }
    });

    // Hotkey clicked callback — enters capture mode via Slint FocusScope only.
    // We do NOT activate the low-level hook recording (set_recording_hotkey)
    // because that consumes ALL keystrokes system-wide and breaks Windows shortcuts.
    let window_weak = window.as_weak();
    let pending_hotkey_clone = pending_hotkey.clone();
    window.on_hotkey_clicked(move || {
        if let Some(w) = window_weak.upgrade() {
            config::log_debug("UI: on_hotkey_clicked (FocusScope mode)");
            w.set_hotkey_capturing(true);
            w.set_hotkey_conflict(false);
            w.set_hotkey_can_save(false);
            w.set_hotkey_display(w.get_i18n_hotkey_prompt());
            *pending_hotkey_clone.lock().unwrap() = None;
        }
    });

    // Slint FocusScope key-press handler.
    // When only modifier keys are held, shows intermediate display ("CTRL + SHIFT + ...").
    // When a non-modifier key is pressed, validates immediately and completes the capture.
    let window_weak = window.as_weak();
    let pending_hotkey_clone = pending_hotkey.clone();
    let config_for_validate = config.clone();
    window.on_key_recorded(move |text, ctrl, alt, shift, win| {
        config::log_debug(&format!(
            "UI: key_recorded: text=U+{:04X}, ctrl={}, alt={}, shift={}, win={}",
            text.chars().next().map(|c| c as u32).unwrap_or(0), ctrl, alt, shift, win
        ));

        match parse_key_event(text.as_str(), ctrl, alt, shift, win) {
            Some(hk) => {
                // Non-modifier key pressed: validate immediately
                if let Some(w) = window_weak.upgrade() {
                    let display = hotkey_display_string(&hk);
                    config::log_debug(&format!("UI: captured hotkey={:?}, display={}", hk, display));
                    let lang = config_for_validate.load().language.clone();

                    match crate::hotkey::validate_hotkey(&hk, &lang) {
                        Ok(()) => {
                            // Valid hotkey — show it and enable save
                            w.set_hotkey_display(display);
                            w.set_hotkey_capturing(false);
                            w.set_hotkey_conflict(false);
                            w.set_hotkey_can_save(true);
                            *pending_hotkey_clone.lock().unwrap() = Some(hk);
                        }
                        Err(reason) => {
                            // Invalid hotkey — show error with red background.
                            // User can click the field again to retry.
                            let err_display = format!("{} — {}", display, reason);
                            w.set_hotkey_display(SharedString::from(err_display));
                            w.set_hotkey_capturing(false);
                            w.set_hotkey_conflict(true);
                            w.set_hotkey_can_save(false);
                            *pending_hotkey_clone.lock().unwrap() = None;
                            config::log_debug(&format!("UI: hotkey rejected: {}", reason));
                        }
                    }
                }
                true
            }
            None => {
                // Modifier-only key: show intermediate display
                if let Some(w) = window_weak.upgrade() {
                    let mut parts = Vec::new();
                    if ctrl { parts.push("CTRL"); }
                    if alt { parts.push("ALT"); }
                    if shift { parts.push("SHIFT"); }
                    if win { parts.push("WIN"); }
                    if !parts.is_empty() {
                        let display = format!("{} + ...", parts.join(" + "));
                        w.set_hotkey_display(SharedString::from(display));
                    }
                }
                true // consume the modifier key event
            }
        }
    });

    // Save hotkey callback — validation already done in on_key_recorded,
    // so this just persists the already-validated hotkey.
    let window_weak = window.as_weak();
    let config_clone = config.clone();
    let pending_hotkey_clone = pending_hotkey.clone();
    let htid = hook_thread_id;
    window.on_save_hotkey(move || {
        if let Some(w) = window_weak.upgrade() {
            let opt_hk = pending_hotkey_clone.lock().unwrap().clone();
            if let Some(hk) = opt_hk {
                // Struct-update: only override the hotkey field, inherit all others.
                let current_config = config_clone.load();
                let new_config = AppConfig {
                    hotkey: hk.clone(),
                    ..(**current_config).clone()
                };

                if let Err(e) = config::save(&new_config) {
                    config::log_debug(&format!("Failed to save hotkey config: {}", e));
                    eprintln!("[rust-expander] Failed to save hotkey config: {}", e);
                } else {
                    config_clone.store(Arc::new(new_config.clone()));
                    unsafe {
                        let _ = PostThreadMessageW(htid, WM_REHOOK, WPARAM(0), LPARAM(0));
                    }
                }

                w.set_hotkey_conflict(false);
                w.set_hotkey_can_save(false);
                w.set_hotkey_display(hotkey_display_string(&hk));
                *pending_hotkey_clone.lock().unwrap() = None;
            }
        }
    });

    // Save config callback
    let window_weak = window.as_weak();
    let config_clone = config.clone();
    let snippets_model_clone = snippets_model.clone();
    let htid = hook_thread_id;

    let saved_size_for_save = saved_size.clone();
    let saved_pos_for_save  = saved_pos.clone();
    window.on_save_config(move || {
        if let Some(w) = window_weak.upgrade() {
            let current_config = config_clone.load();

            // Collect snippets, skipping rows with empty triggers.
            // An empty trigger matches every buffer.ends_with("") call → infinite
            // expansion loop. Filter here so they are never persisted.
            let mut new_snippets = Vec::new();
            for i in 0..snippets_model_clone.row_count() {
                if let Some(model) = snippets_model_clone.row_data(i) {
                    if model.trigger.is_empty() {
                        continue; // skip incomplete rows silently
                    }
                    let mode = if model.mode.as_str() == "hotkey" {
                        ExpansionMode::Hotkey
                    } else {
                        ExpansionMode::Immediate
                    };
                    new_snippets.push(Snippet {
                        trigger: model.trigger.to_string(),
                        replacement: model.replacement.to_string(),
                        mode,
                    });
                }
            }

            // Struct-update: only override snippets, inherit all other fields.
            let new_config = AppConfig {
                snippets: new_snippets,
                ..(**current_config).clone()
            };

            match config::save(&new_config) {
                Ok(()) => {
                    config_clone.store(Arc::new(new_config));
                    unsafe {
                        let _ = PostThreadMessageW(htid, WM_REHOOK, WPARAM(0), LPARAM(0));
                    }
                    // Capture window geometry as logical coordinates so that
                    // restore works correctly regardless of DPI state on reopen.
                    let scale = w.window().scale_factor();
                    let phys_size = w.window().size();
                    *saved_size_for_save.lock().unwrap() = Some(LogicalSize::new(
                        phys_size.width as f32 / scale,
                        phys_size.height as f32 / scale,
                    ));
                    let phys_pos = w.window().position();
                    *saved_pos_for_save.lock().unwrap() = Some(LogicalPosition::new(
                        phys_pos.x as f32 / scale,
                        phys_pos.y as f32 / scale,
                    ));
                    let _ = w.hide();
                }
                Err(e) => {
                    // Show the error inside the window instead of silently closing.
                    let msg = format!("Save failed: {}", e);
                    config::log_debug(&msg);
                    eprintln!("[rust-expander] {}", msg);
                    w.set_save_error_message(SharedString::from(msg));
                    // Don't hide the window — let the user see the error.
                }
            }
        }
    });

    // Cancel config callback
    let window_weak = window.as_weak();
    let config_clone = config.clone();
    let snippets_model_clone = snippets_model.clone();
    
    window.on_cancel_config(move || {
        if let Some(w) = window_weak.upgrade() {
            let current_config = config_clone.load();
            let new_models = config_to_snippet_models(&current_config);
            snippets_model_clone.set_vec(new_models);
            
            let _ = w.hide();
        }
    });

    // Window close (X button) — hide instead of destroy so re-open renders correctly.
    // Reverts unsaved snippet edits, same as Cancel.
    let window_weak = window.as_weak();
    let config_clone_for_close = config.clone();
    let snippets_model_for_close = snippets_model.clone();
    window.window().on_close_requested(move || {
        if let Some(w) = window_weak.upgrade() {
            let current_config = config_clone_for_close.load();
            let new_models = config_to_snippet_models(&current_config);
            snippets_model_for_close.set_vec(new_models);
            let _ = w.hide();
        }
        slint::CloseRequestResponse::HideWindow
    });

    // Add snippet callback
    let snippets_model_clone = snippets_model.clone();
    window.on_add_snippet(move || {
        snippets_model_clone.push(SnippetModel {
            trigger: SharedString::from(""),
            replacement: SharedString::from(""),
            mode: SharedString::from("immediate"),
        });
    });

    // Remove snippet callback
    let snippets_model_clone = snippets_model.clone();
    window.on_remove_snippet(move |index| {
        if index >= 0 && (index as usize) < snippets_model_clone.row_count() {
            snippets_model_clone.remove(index as usize);
        }
    });

    // Language changed callback — apply immediately and save
    let window_weak = window.as_weak();
    let tray_weak = tray.as_weak();
    let config_clone = config.clone();
    window.on_language_changed(move |lang_code| {
        let lang = lang_code.to_string();
        if let (Some(w), Some(t)) = (window_weak.upgrade(), tray_weak.upgrade()) {
            apply_language(&w, &t, &lang);

            // Struct-update: only override language field.
            let current_config = config_clone.load();
            let new_config = AppConfig {
                language: lang,
                ..(**current_config).clone()
            };
            if let Err(e) = config::save(&new_config) {
                config::log_debug(&format!("Failed to save language config: {}", e));
                eprintln!("[rust-expander] Failed to save language config: {}", e);
            } else {
                config_clone.store(Arc::new(new_config));
            }
        }
    });

    // Open config folder in Windows Explorer
    window.on_open_config_folder(move || {
        let dir = config::config_dir();
        let _ = std::process::Command::new("explorer")
            .arg(&dir)
            .spawn();
    });

    // Toggle pause callback
    let window_weak = window.as_weak();
    window.on_toggle_pause(move || {
        let new_paused = !crate::hook::is_paused();
        crate::hook::set_paused(new_paused);
        if let Some(w) = window_weak.upgrade() {
            w.set_is_paused(new_paused);
        }
    });

    // Run the Slint event loop (blocks until quit)
    slint::run_event_loop()?;
    Ok(())
}


