# Rust-Expander

A high-performance, native Windows text expander written in **Rust** and **Slint UI**, supporting both **Windows 11 ARM64** and **Windows 10/11 x86_64 (Intel/AMD)**.

Designed from the ground up for speed, low resource consumption, and rock-solid compatibility with modern Windows 11 apps (including WinUI 3, Windows 11 Notepad, and Chromium-based browsers).

---

## Features

- 🚀 **Native Dual-Architecture Support**: Pre-built native releases for both ARM64 (Snapdragon X / Surface Pro) and x86_64 (Intel / AMD).
- ⚡ **Ultra-Fast Matching**: Zero-allocation circular buffer matching (~1.04 ns per keystroke).
- 🪟 **Modern Windows 11 / WinUI 3 Compatible**: Flawless text replacement that avoids asynchronous input drops and duplicated characters.
- ⌨️ **Dual Expansion Modes**:
  - **Immediate**: Expands automatically the moment you finish typing the trigger string (e.g. `:email` &rarr; `user@example.com`).
  - **Hotkey**: Expands when your configured hotkey is pressed (e.g. typing a short code followed by for example `Alt + Shift + X`).
- 🌐 **Multilingual Support (i18n)**: Native English and Danish UI translations.
- 🖥️ **Lightweight Slint UI & System Tray**: Minimalist settings window and tray integration with software rendering for GPU-independent stability.

---

## Technical Architecture & Design

### 1. Zero-Allocation Reverse Buffer Matching (1.04 ns / call)
The keyboard hook maintains a fixed-size circular ring buffer (`KeyBuffer`) tracking recent keystrokes without heap allocation. When evaluating snippet triggers, matching iterates backwards using a double-ended iterator directly on UTF-8 streams. This achieves immediate early-exit on the first mismatched character with zero `malloc`/heap overhead during active typing.

### 2. WinUI 3 Reliable Text Injection Engine
Per-character `SendInput` (`KEYEVENTF_UNICODE`) frequently drops or duplicates characters in asynchronous XAML/WinUI 3 applications. Rust-Expander employs a dedicated clipboard injection technique:
1. Releases active modifier keys (Ctrl, Shift, Alt) to prevent unwanted shortcuts.
2. Backspaces the trigger length.
3. Temporarily injects the replacement text via the Win32 Clipboard (`CF_UNICODETEXT`) and simulates `Ctrl + V`.
4. Asynchronously restores the original clipboard content after an intentional 150 ms window.

### 3. Slint Software Rendering
Uses Slint with the `renderer-software` backend to ensure maximum compatibility across Qualcomm Snapdragon ARM64 chips and standard x86 GPUs without relying on GPU driver OpenGL/Vulkan quirks.

### 4. Lock-Free Config Sharing (`ArcSwap`)
Configuration state is distributed between the low-level Windows keyboard hook thread and the Slint UI thread using `ArcSwap`, guaranteeing wait-free read paths during high-frequency typing.

---

## Performance Benchmarks

Run via `cargo bench --bench buffer_benchmark`:

| Benchmark | Latency / Call | Description |
| :--- | :--- | :--- |
| **Trigger Scan (`ends_with`)** | **~1.04 ns** | Evaluated on non-matching keystroke across snippet list |
| **Exact Match Expansion** | **~3.30 ns** | Full trigger match detection |
| **Keystroke Ring Buffer Push** | **~2.68 ns** | Keystroke capture into circular buffer |

---

## Quickstart

### Prerequisites
- Windows 11 / 10 on **ARM64** or **x86_64**
- [Rust toolchain](https://rustup.rs/) (edition 2024+)

### Building and Running

```powershell
# Run in development mode (defaults to your machine's native architecture)
cargo run

# Build optimized release for ARM64
cargo build --release --target aarch64-pc-windows-msvc

# Build optimized release for x86_64 (Intel/AMD)
cargo build --release --target x86_64-pc-windows-msvc

# Run unit tests
cargo test

# Run benchmark suite
cargo bench --bench buffer_benchmark
```

---

## Configuration

Settings and snippets are stored in `config.toml` (located in your app data directory or project root):

```toml
language = "en" # "en" or "da"
buffer_size = 64

[hotkey]
modifiers = 5 # Alt (1) + Shift (4)
virtual_key = 88 # 'X' key (0x58)

[[snippets]]
trigger = ":email"
replacement = "name@example.com"
mode = "immediate"

[[snippets]]
trigger = ":sig"
replacement = "Best regards,\nYour Name"
mode = "immediate"

[[snippets]]
trigger = "addr"
replacement = "123 Main Street, Suite 400"
mode = "hotkey"
```

---

## Security, Privacy & Offline Guarantee

Because Rust-Expander operates via a low-level Windows keyboard hook (`WH_KEYBOARD_LL`) to detect snippet triggers, privacy and transparency are paramount:

- 🔒 **100% Offline**: The binary contains **zero networking crates** (`reqwest`, `tokio`, sockets, etc.) and transmits **zero telemetry or analytics**.
- 🗄️ **Local Storage Only**: Snippet definitions, hotkeys, and preferences remain strictly on your local device in `config.toml`.
- 📋 **Safe Clipboard Restoration**: Injected text utilizes temporary Win32 clipboard replacement with an automatic 150 ms restore window to preserve your previous clipboard state.
- 🔍 **Auditable & Open Source**: The entire codebase is open source and can be inspected or compiled from source directly.

---

## Windows SmartScreen & Installation Notice

Because this is an independent open-source project without an enterprise Code Signing Certificate (EV Certificate), **Windows SmartScreen** may display a warning when launching the pre-compiled binary for the first time:

1. Click **"More info"** on the Windows SmartScreen dialog.
2. Click **"Run anyway"**.

> [!TIP]
> You can verify the integrity of the downloaded zip archives using the SHA-256 hashes published with every [GitHub Release](https://github.com/ThMoJe/Rust-Expander/releases) in `checksums.txt`, or compile the application yourself using `cargo build --release`.

---

## Known Limitations & Technical Considerations

- **Elevated Windows / Administrator Mode (UAC)**: Due to Windows User Interface Privilege Isolation (UIPI), standard user-mode applications cannot capture keystrokes or inject text into windows running with elevated Administrator privileges (such as an Administrator Terminal or Task Manager). If you frequently type into elevated windows, run Rust-Expander as Administrator.
- **DirectInput / Exclusive Fullscreen Games**: Games or software that read raw hardware input directly bypassing standard Win32 message queues will not trigger expansions.
- **Password Fields**: When typing into sensitive fields, use hotkey-triggered expansion mode or clear your active buffer if needed.

---

## License

This project is licensed under the [MIT License](LICENSE).

---

## Changelog

### v0.1.0 — 2026-08-19
- **Full clipboard backup/restore**: All clipboard formats are now preserved across text injection — no longer just `CF_UNICODETEXT`. Third-party clipboard managers remain unaffected.
- **Emoji / surrogate-pair tracking**: Typing emoji or other supplementary-plane characters no longer desynchronises the internal trigger buffer.
- **Configurable clipboard restore delay**: `clipboard_restore_delay_ms` in `config.toml` (default: 150 ms) — tune for slow machines or Prism-emulated environments.
- **Vertical scrollbar**: The snippet list now shows a scrollbar when rows overflow the visible area.
- **Self-destruct uninstall**: 🗑 button in Settings opens a confirmation dialog and then removes all settings, the log, and the `.exe` — complete portable uninstall in one click.
- **Graceful hook-install errors**: Instead of panicking, the app now shows a native error dialog if `SetWindowsHookExW` fails (e.g. blocked by antivirus).
- **Config-file robustness**: Missing optional fields (including `[[snippets]]`) no longer crash on load; they fall back to defaults.
- **Unit tests**: 34 tests covering buffer, config parsing, hotkey validation, and round-trip serialisation.
- **Clippy pedantic**: `#![warn(clippy::pedantic)]` enabled with targeted suppression of intentional Win32 cast patterns.

### v0.1.0 — Initial release
- Native ARM64 + x86_64 text expander
- Immediate and Hotkey expansion modes
- Slint UI with system tray, English + Danish i18n
- Clipboard-injection engine compatible with WinUI 3 / Windows 11 Notepad
