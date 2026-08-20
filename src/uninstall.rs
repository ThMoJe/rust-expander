//! Self-destruct / uninstall support.
//!
//! A running Windows process cannot delete its own `.exe` file because the OS
//! holds a file lock on every loaded image. The standard portable-app workaround
//! is to hand the deletion off to a separate, short-lived `cmd.exe` process that
//! sleeps briefly (using `ping` as a delay primitive) then deletes the file.
//!
//! The spawned `cmd.exe` is created with `CREATE_NO_WINDOW | DETACHED_PROCESS`
//! so it is completely invisible and survives after the parent exits.

use std::os::windows::process::CommandExt;
use windows::Win32::System::Threading::{CREATE_NO_WINDOW, DETACHED_PROCESS};

/// Executes the full self-destruct sequence:
///
/// 1. Resolves the current `.exe` path.
/// 2. Deletes the entire config directory (settings, log, etc.).
/// 3. Spawns a hidden, detached `cmd.exe` process that waits ~3 s then
///    deletes the `.exe` file. The 3-second window gives the app enough time
///    to finish its own shutdown before the file is removed.
/// 4. Posts `WM_QUIT` to the hook thread so its message loop terminates cleanly.
/// 5. Calls `slint::quit_event_loop()` so the main thread unwinds.
///
/// Returns `Err(String)` if the `.exe` path cannot be resolved or if spawning
/// the delete process fails. Config-dir deletion failures are best-effort.
pub fn self_destruct(hook_thread_id: u32) -> Result<(), String> {
    // --- Step 1: Resolve own exe path ---
    let exe_path = std::env::current_exe()
        .map_err(|e| format!("Could not determine exe path: {}", e))?;

    let exe_str = exe_path
        .to_str()
        .ok_or_else(|| "Exe path contains non-UTF-8 characters".to_string())?
        .to_owned();

    // --- Step 2: Wipe config directory (best-effort) ---
    crate::config::delete_config_dir();

    // --- Step 3: Spawn hidden delayed-delete process ---
    //
    // `ping -n 4 127.0.0.1` busy-waits ~3 seconds (4 ICMP echo requests at
    // ~1 s intervals, first one immediate). That is more than enough for the
    // Slint event loop and hook thread to unwind completely.
    //
    // CREATE_NO_WINDOW : no console window flashes to the user.
    // DETACHED_PROCESS : child survives after the parent process exits.
    let delete_cmd = format!("ping -n 4 127.0.0.1 >nul & del /f /q \"{}\"", exe_str);
    let creation_flags = CREATE_NO_WINDOW.0 | DETACHED_PROCESS.0;

    std::process::Command::new("cmd")
        .args(["/c", &delete_cmd])
        .creation_flags(creation_flags)
        .spawn()
        .map_err(|e| format!("Failed to spawn delete process: {}", e))?;

    // --- Step 4: Shut down the hook thread ---
    unsafe {
        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};
        let _ = PostThreadMessageW(hook_thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
    }

    // --- Step 5: Terminate the Slint event loop ---
    // Called from the Slint main thread (inside a UI callback), so this is safe.
    let _ = slint::quit_event_loop();

    Ok(())
}
