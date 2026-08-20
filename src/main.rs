#![windows_subsystem = "windows"]

mod buffer;
mod config;
mod hook;
mod hotkey;
mod i18n;
mod replacer;
mod text_utils;
mod ui;
mod uninstall;

use std::sync::Arc;
use arc_swap::ArcSwap;

fn main() {
    // ---------------------------------------------------------------------------
    // Singleton guard — prevent multiple instances running simultaneously.
    //
    // If two instances run at the same time they each install a WH_KEYBOARD_LL
    // hook, causing every expansion to fire twice, and they race to write
    // config.toml. A named Win32 Mutex is the standard Windows approach.
    // The OS releases the mutex automatically when the process exits.
    // ---------------------------------------------------------------------------
    let _singleton_mutex = {
        use windows::Win32::Foundation::ERROR_ALREADY_EXISTS;
        use windows::Win32::System::Threading::CreateMutexW;
        use windows::core::PCWSTR;

        let name: Vec<u16> = "Global\\RustExpanderSingleton\0"
            .encode_utf16()
            .collect();

        let handle = unsafe {
            CreateMutexW(None, true, PCWSTR(name.as_ptr()))
        };

        match handle {
            Ok(h) => {
                let last_err = unsafe { windows::Win32::Foundation::GetLastError() };
                if last_err == ERROR_ALREADY_EXISTS {
                    // Another instance is already running — show a brief message and exit.
                    unsafe {
                        use windows::Win32::UI::WindowsAndMessaging::{
                            MessageBoxW, MB_ICONINFORMATION, MB_OK,
                        };
                        let title: Vec<u16> = "Rust-Expander\0".encode_utf16().collect();
                        let text: Vec<u16> =
                            "Rust-Expander is already running.\n\nCheck the system tray.\0"
                                .encode_utf16()
                                .collect();
                        let _ = MessageBoxW(
                            None,
                            PCWSTR(text.as_ptr()),
                            PCWSTR(title.as_ptr()),
                            MB_ICONINFORMATION | MB_OK,
                        );
                    }
                    return;
                }
                // Hold the handle for the process lifetime — drop closes the mutex.
                Some(h)
            }
            Err(_) => {
                // Failed to create mutex (very unusual). Continue anyway rather than
                // silently refusing to start.
                None
            }
        }
    };


    config::log_debug("RustExpander starting...");

    // Load or create default configuration
    let app_config = match config::load() {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("Failed to load config: {}. Using defaults.", e);
            config::log_debug(&msg);
            eprintln!("[rust-expander] {}", msg);
            config::default_config()
        }
    };

    config::log_debug(&format!(
        "Loaded config: {} snippets, buffer_size={}",
        app_config.snippets.len(),
        app_config.buffer_size
    ));

    // Shared config via lock-free ArcSwap
    let shared_config = Arc::new(ArcSwap::new(Arc::new(app_config)));

    // Start the keyboard hook on a dedicated thread.
    // HookManager::start() returns Err if SetWindowsHookExW fails (e.g. the process
    // is blocked by an antivirus). We surface this with a native Win32 MessageBox so
    // the user gets an actionable error rather than a silent panic or crash.
    let mut hook_manager = match hook::HookManager::start(shared_config.clone()) {
        Ok(mgr) => mgr,
        Err(e) => {
            let msg = format!("FATAL: hook install failed: {}", e);
            config::log_debug(&msg);
            eprintln!("[rust-expander] {}", msg);

            // Show a native error dialog — the app cannot function without the hook.
            unsafe {
                use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
                use windows::core::PCWSTR;

                let title: Vec<u16> = "Rust-Expander — Fatal Error\0"
                    .encode_utf16().collect();
                let text: Vec<u16> = format!(
                    "Could not install keyboard hook:\n\n{}\n\nPlease check your antivirus settings \
                     and try running as Administrator.",
                    e
                ).encode_utf16().chain(std::iter::once(0)).collect();

                let _ = MessageBoxW(
                    None,
                    PCWSTR(text.as_ptr()),
                    PCWSTR(title.as_ptr()),
                    MB_ICONERROR | MB_OK,
                );
            }
            return;
        }
    };

    let buffer_debug = hook_manager.buffer_debug();
    config::log_debug(&format!("Hook thread started (tid={})", hook_manager.thread_id()));

    let show_settings = std::env::args().any(|arg| arg == "--settings" || arg == "--show");

    // Run the Slint UI event loop on the main thread (blocks until quit)
    match ui::setup_and_run(shared_config.clone(), hook_manager.thread_id(), buffer_debug, show_settings) {
        Ok(()) => config::log_debug("UI event loop exited normally"),
        Err(e) => {
            let msg = format!("UI error: {}", e);
            config::log_debug(&msg);
            eprintln!("[rust-expander] {}", msg);
        }
    }

    // Graceful shutdown: stop the hook thread
    hook_manager.stop();
    config::log_debug("RustExpander shutdown complete.");
}
