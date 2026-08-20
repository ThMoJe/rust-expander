use serde::{Deserialize, Serialize};
use std::env;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// The main application configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppConfig {
    /// UI language ("en" or "da")
    #[serde(default = "default_language")]
    pub language: String,
    /// Hotkey configuration for triggering expansions.
    pub hotkey: HotkeyConfig,
    /// Size of the typing buffer (clamped to 1..=256 at load time).
    #[serde(default = "default_buffer_size")]
    pub buffer_size: usize,
    /// How long (in milliseconds) to wait after simulating Ctrl+V before restoring
    /// the original clipboard content. `WinUI` 3 / XAML apps have asynchronous paste
    /// pipelines that need time to read the clipboard before we overwrite it.
    /// Increase this on slow machines or under Prism emulation if clipboard restore
    /// happens too early and cuts off the pasted text.
    #[serde(default = "default_clipboard_restore_delay_ms")]
    pub clipboard_restore_delay_ms: u64,
    /// List of user-defined snippets.
    #[serde(default)]
    pub snippets: Vec<Snippet>,
}

fn default_language() -> String {
    "en".to_string()
}

fn default_buffer_size() -> usize {
    10
}

fn default_clipboard_restore_delay_ms() -> u64 {
    150
}

/// Hotkey configuration for the application.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HotkeyConfig {
    /// Win32 modifier flags (`MOD_ALT=1`, `MOD_CONTROL=2`, `MOD_SHIFT=4`, `MOD_WIN=8`)
    pub modifiers: u32,
    /// Win32 virtual key code (e.g., 0x58 for 'X')
    pub virtual_key: u32,
}

/// A snippet definition containing a trigger, replacement, and expansion mode.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Snippet {
    /// The string that triggers the expansion.
    pub trigger: String,
    /// The replacement string.
    pub replacement: String,
    /// The mode of expansion.
    pub mode: ExpansionMode,
}

/// Defines when a snippet should be expanded.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ExpansionMode {
    /// Expands immediately upon typing the trigger.
    Immediate,
    /// Expands only when the hotkey is pressed.
    Hotkey,
}

/// Errors that can occur during configuration operations.
#[derive(Debug)]
pub enum ConfigError {
    /// Standard I/O error.
    Io(io::Error),
    /// Error deserializing TOML.
    Toml(toml::de::Error),
    /// Error serializing TOML.
    Serialize(toml::ser::Error),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Io(err) => write!(f, "I/O error: {err}"),
            ConfigError::Toml(err) => write!(f, "TOML parsing error: {err}"),
            ConfigError::Serialize(err) => write!(f, "TOML serialization error: {err}"),
        }
    }
}

impl std::error::Error for ConfigError {}

impl From<io::Error> for ConfigError {
    fn from(err: io::Error) -> Self {
        ConfigError::Io(err)
    }
}

impl From<toml::de::Error> for ConfigError {
    fn from(err: toml::de::Error) -> Self {
        ConfigError::Toml(err)
    }
}

impl From<toml::ser::Error> for ConfigError {
    fn from(err: toml::ser::Error) -> Self {
        ConfigError::Serialize(err)
    }
}

/// Returns the path to the configuration directory, creating it if it doesn't exist.
/// Logs a warning to stderr if the directory cannot be created.
#[must_use]
pub fn config_dir() -> PathBuf {
    let appdata = env::var("APPDATA").unwrap_or_else(|_| ".".to_string());
    let mut path = PathBuf::from(appdata);
    path.push("Rust-Expander");

    if let Err(e) = fs::create_dir_all(&path) {
        // Only warn if the directory genuinely doesn't exist — EEXIST is fine.
        if !path.exists() {
            eprintln!("[rust-expander] WARNING: Could not create config dir {}: {e}", path.display());
        }
    }

    path
}

/// Returns the path to the configuration file.
#[must_use]
pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

/// Returns the path to the backup configuration file.
fn backup_path() -> PathBuf {
    config_dir().join("config.toml.bak")
}

/// Deletes the entire configuration directory and all of its contents
/// (config.toml, debug.log, and any other files written there).
/// Called during the self-destruct / uninstall flow.
pub fn delete_config_dir() {
    let dir = config_dir();
    if dir.exists()
        && let Err(e) = fs::remove_dir_all(&dir) {
            // Best-effort — log to stderr since the log file itself may be gone
            eprintln!("uninstall: failed to remove config dir {}: {e}", dir.display());
        }
}

// ---------------------------------------------------------------------------
// Buffered debug logging
// ---------------------------------------------------------------------------
//
// Opens debug.log once and keeps it open for the lifetime of the process.
// This avoids the overhead of open/write/close on every keystroke, which was
// previously happening inside the WH_KEYBOARD_LL hook callback.
// The file is flushed on every write so log entries appear immediately.
//
// In release builds the function is a no-op unless the RUST_EXPANDER_LOG
// environment variable is set, keeping hot-path overhead at zero.

static LOG_FILE: OnceLock<Mutex<Option<fs::File>>> = OnceLock::new();

/// Writes a diagnostic message to debug.log in the config directory.
///
/// The file is opened lazily on first call and kept open. In release builds
/// this is a no-op unless the `RUST_EXPANDER_LOG` env-var is set.
pub fn log_debug(msg: &str) {
    // In release builds, skip logging unless explicitly enabled.
    #[cfg(not(debug_assertions))]
    if env::var("RUST_EXPANDER_LOG").is_err() {
        return;
    }

    let guard = LOG_FILE.get_or_init(|| {
        let path = config_dir().join("debug.log");
        let file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .ok();
        Mutex::new(file)
    });

    if let Ok(mut lock) = guard.lock()
        && let Some(ref mut file) = *lock {
            let _ = writeln!(file, "{msg}");
            // Flush immediately so entries are visible even if the process dies.
            let _ = file.flush();
        }
}

/// Returns the default application configuration.
#[must_use]
pub fn default_config() -> AppConfig {
    AppConfig {
        language: "en".to_string(),
        hotkey: HotkeyConfig {
            modifiers: 6, // CTRL(2) | SHIFT(4)
            virtual_key: 0x54, // 'T'
        },
        buffer_size: 10,
        clipboard_restore_delay_ms: default_clipboard_restore_delay_ms(),
        snippets: vec![
            Snippet {
                trigger: ".sig".to_string(),
                replacement: "Regards,\nJohn Doe".to_string(),
                mode: ExpansionMode::Immediate,
            },
            Snippet {
                trigger: ".em".to_string(),
                replacement: "john.doe@example.com".to_string(),
                mode: ExpansionMode::Immediate,
            },
            Snippet {
                trigger: "jd".to_string(),
                replacement: "With kind regards and sincerely - John Doe".to_string(),
                mode: ExpansionMode::Hotkey,
            },
        ],
    }
}

/// Loads the configuration from the config file.
///
/// Recovery strategy:
/// 1. If `config.toml` does not exist, create it from defaults.
/// 2. If `config.toml` exists but is corrupt/invalid, automatically fall back
///    to `config.toml.bak` (written after every successful save).
/// 3. `buffer_size` is clamped to 1..=256 regardless of what is in the file.
pub fn load() -> Result<AppConfig, ConfigError> {
    let path = config_path();

    if !path.exists() {
        let config = default_config();
        save(&config)?;
        return Ok(config);
    }

    let contents = fs::read_to_string(&path)?;
    match toml::from_str::<AppConfig>(&contents) {
        Ok(mut config) => {
            // Clamp buffer_size to a safe range — prevents both the
            // buffer_size=0 crash (usize underflow in pop/ends_with) and
            // absurdly large allocations from hand-edited config files.
            config.buffer_size = config.buffer_size.clamp(1, 256);
            Ok(config)
        }
        Err(primary_err) => {
            // config.toml is corrupt — try the backup before giving up.
            let bak = backup_path();
            if bak.exists() {
                eprintln!(
                    "[rust-expander] WARNING: config.toml is corrupt ({primary_err}). \
                     Trying config.toml.bak."
                );
                let bak_contents = fs::read_to_string(&bak)?;
                match toml::from_str::<AppConfig>(&bak_contents) {
                    Ok(mut config) => {
                        config.buffer_size = config.buffer_size.clamp(1, 256);
                        eprintln!("[rust-expander] INFO: Recovered config from backup.");
                        Ok(config)
                    }
                    Err(_) => Err(ConfigError::Toml(primary_err)),
                }
            } else {
                Err(ConfigError::Toml(primary_err))
            }
        }
    }
}

/// Saves the given configuration to the config file.
///
/// After a successful write, the file is also copied to `config.toml.bak`.
/// If config.toml ever becomes corrupt (power loss, disk full, etc.), `load()`
/// will automatically recover from the backup.
pub fn save(config: &AppConfig) -> Result<(), ConfigError> {
    let path = config_path();
    let toml_str = toml::to_string_pretty(config)?;
    fs::write(&path, &toml_str)?;

    // Write backup — best-effort, never fails the save itself.
    let bak = backup_path();
    if let Err(e) = fs::copy(&path, &bak) {
        eprintln!("[rust-expander] WARNING: Could not write config backup: {e}");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_serialization_roundtrip() {
        let config = default_config();
        let toml_str = toml::to_string(&config).expect("Failed to serialize default config");
        let deserialized: AppConfig = toml::from_str(&toml_str).expect("Failed to deserialize default config");
        assert_eq!(config, deserialized);
    }

    #[test]
    fn test_default_config_values() {
        let config = default_config();
        assert_eq!(config.language, "en");
        assert_eq!(config.hotkey.modifiers, 6);
        assert_eq!(config.hotkey.virtual_key, 0x54);
        assert_eq!(config.buffer_size, 10);
        assert_eq!(config.clipboard_restore_delay_ms, 150);
        assert_eq!(config.snippets.len(), 3);
        assert_eq!(config.snippets[0].trigger, ".sig");
        assert_eq!(config.snippets[0].mode, ExpansionMode::Immediate);
        assert_eq!(config.snippets[1].trigger, ".em");
        assert_eq!(config.snippets[1].mode, ExpansionMode::Immediate);
        assert_eq!(config.snippets[2].trigger, "jd");
        assert_eq!(config.snippets[2].mode, ExpansionMode::Hotkey);
    }

    // NEXT-5: Verify that completely invalid TOML is rejected cleanly.
    #[test]
    fn test_invalid_toml_is_rejected() {
        let bad_toml = "this is not [[valid] toml = {";
        let result: Result<AppConfig, _> = toml::from_str(bad_toml);
        assert!(result.is_err(), "Expected parse failure for invalid TOML");
    }

    // NEXT-5: Verify a minimal config (only mandatory fields) gets correct defaults
    // for all optional fields, confirming backward-compatibility with old config files
    // that predate newly added optional fields like clipboard_restore_delay_ms.
    #[test]
    fn test_minimal_config_applies_defaults() {
        let minimal = r#"
            buffer_size = 5
            [hotkey]
            modifiers = 5
            virtual_key = 88
        "#;
        let config: AppConfig = toml::from_str(minimal).expect("Minimal config should parse");
        assert_eq!(config.language, "en", "language should default to 'en'");
        assert_eq!(
            config.clipboard_restore_delay_ms, 150,
            "clipboard_restore_delay_ms should default to 150"
        );
        assert!(config.snippets.is_empty(), "snippets should default to empty");
    }

    // NEXT-5: Verify that invalid hotkey modifier values are stored as-is (they are u32
    // so the parser accepts any value; validation happens at the Win32 layer).
    #[test]
    fn test_hotkey_modifier_boundary_values() {
        let toml_str = r#"
            buffer_size = 64
            [hotkey]
            modifiers = 0
            virtual_key = 112
        "#;
        let config: AppConfig = toml::from_str(toml_str).expect("Zero-modifier config should parse");
        assert_eq!(config.hotkey.modifiers, 0);
        assert_eq!(config.hotkey.virtual_key, 112); // F1 VK code
    }

    // NEXT-5: Verify that a missing [hotkey] section fails gracefully (it is required).
    #[test]
    fn test_missing_hotkey_section_fails() {
        let toml_str = "buffer_size = 10";
        let result: Result<AppConfig, _> = toml::from_str(toml_str);
        assert!(result.is_err(), "Config without [hotkey] should fail");
    }

    // NEXT-5: Verify custom clipboard_restore_delay_ms survives a round-trip.
    #[test]
    fn test_custom_clipboard_delay_roundtrip() {
        let mut config = default_config();
        config.clipboard_restore_delay_ms = 300;
        let toml_str = toml::to_string(&config).expect("Should serialize");
        let back: AppConfig = toml::from_str(&toml_str).expect("Should deserialize");
        assert_eq!(back.clipboard_restore_delay_ms, 300);
    }

    // NEXT-5: Verify that an unknown expansion mode string causes a parse error.
    #[test]
    fn test_unknown_expansion_mode_is_rejected() {
        let toml_str = r#"
            buffer_size = 10
            [hotkey]
            modifiers = 5
            virtual_key = 88
            [[snippets]]
            trigger = ".test"
            replacement = "hello"
            mode = "turbo"
        "#;
        let result: Result<AppConfig, _> = toml::from_str(toml_str);
        assert!(result.is_err(), "Unknown expansion mode should fail to parse");
    }

    #[test]
    fn test_multiline_replacement_roundtrip() {
        let mut config = default_config();
        config.snippets = vec![Snippet {
            trigger: ".sig".to_string(),
            replacement: "Line one\nLine two\nLine three".to_string(),
            mode: ExpansionMode::Immediate,
        }];
        let toml_str = toml::to_string(&config).expect("Should serialize");
        let back: AppConfig = toml::from_str(&toml_str).expect("Should deserialize");
        assert_eq!(
            back.snippets[0].replacement,
            "Line one\nLine two\nLine three",
            "Newlines in replacement should survive TOML round-trip"
        );
    }

    #[test]
    fn test_buffer_size_clamp_zero() {
        // buffer_size=0 used to cause usize underflow in KeyBuffer::pop().
        // The load() function now clamps it to 1.
        let toml_str = r#"
            buffer_size = 0
            [hotkey]
            modifiers = 6
            virtual_key = 84
        "#;
        let mut config: AppConfig = toml::from_str(toml_str).expect("Should parse");
        config.buffer_size = config.buffer_size.clamp(1, 256);
        assert_eq!(config.buffer_size, 1, "buffer_size=0 should be clamped to 1");
    }

    #[test]
    fn test_buffer_size_clamp_huge() {
        let toml_str = r#"
            buffer_size = 99999
            [hotkey]
            modifiers = 6
            virtual_key = 84
        "#;
        let mut config: AppConfig = toml::from_str(toml_str).expect("Should parse");
        config.buffer_size = config.buffer_size.clamp(1, 256);
        assert_eq!(config.buffer_size, 256, "buffer_size=99999 should be clamped to 256");
    }

    #[test]
    fn test_buffer_size_default_when_missing() {
        // buffer_size is now optional with a default of 10.
        let toml_str = r#"
            [hotkey]
            modifiers = 6
            virtual_key = 84
        "#;
        let config: AppConfig = toml::from_str(toml_str).expect("Should parse without buffer_size");
        assert_eq!(config.buffer_size, 10, "buffer_size should default to 10");
    }
}
