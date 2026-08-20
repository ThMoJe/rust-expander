//! Internationalization (i18n) support.
//! Currently supports English ("en") and Danish ("da").

/// All translatable UI strings for the application.
#[derive(Debug, Clone)]
pub struct Strings {
    // Window
    pub window_title: &'static str,
    pub header: &'static str,

    // Hotkey section
    pub hotkey_label: &'static str,
    pub hotkey_save: &'static str,
    pub hotkey_prompt: &'static str,

    // Buffer
    pub buffer_label: &'static str,
    pub buffer_empty: &'static str,

    // Snippet table
    pub col_trigger: &'static str,
    pub col_trigger_tooltip: &'static str,
    pub col_replacement: &'static str,
    pub col_mode: &'static str,
    pub mode_immediate: &'static str,
    pub mode_hotkey: &'static str,
    pub btn_delete: &'static str,
    pub btn_add: &'static str,

    // Bottom buttons
    pub btn_quit: &'static str,
    pub btn_pause: &'static str,
    pub btn_resume: &'static str,
    pub btn_pause_tooltip: &'static str,
    pub btn_cancel: &'static str,
    pub btn_save: &'static str,

    // Tray
    pub tray_tooltip: &'static str,
    pub tray_open: &'static str,
    pub tray_quit: &'static str,

    // Validation errors
    pub err_needs_mod: &'static str,
    pub err_ctrl_reserved: &'static str,
    pub err_sys_reserved: &'static str,
    pub err_win_reserved: &'static str,
    pub err_conflict: &'static str,

    // Uninstall confirmation dialog
    pub uninstall_btn: &'static str,
    pub uninstall_tooltip: &'static str,
    pub uninstall_title: &'static str,
    pub uninstall_body: &'static str,

    // Cancel button tooltip
    pub btn_cancel_tooltip: &'static str,
}

/// Returns the UI strings for the given language code.
/// Falls back to English for unknown languages.
#[must_use]
pub fn get_strings(lang: &str) -> Strings {
    match lang {
        "da" => STRINGS_DA,
        _ => STRINGS_EN,
    }
}

const STRINGS_EN: Strings = Strings {
    window_title: "Rust-Expander - Settings",
    header: "Settings",
    hotkey_label: "Global hotkey:",
    hotkey_save: "Set",
    hotkey_prompt: "Press new key combination",
    buffer_label: "Buffer:",
    buffer_empty: "(empty)",
    col_trigger: "Trigger (max 10)",
    col_trigger_tooltip: "When you type any of these character sequences they will be replaced either immediately or when you press the set Hotkey",
    col_replacement: "Replacement (trailing space?)",
    col_mode: "Mode",
    mode_immediate: "⚡ Immediate",
    mode_hotkey: "⌨ Hotkey",
    btn_delete: "Delete",
    btn_add: "+ Add new",
    btn_quit: "Quit",
    btn_pause: "Pause",
    btn_resume: "Resume",
    btn_pause_tooltip: "Temporarily pause replacing text you type",
    btn_cancel: "Cancel",
    btn_save: "Save",
    tray_tooltip: "Rust-Expander\nClick for settings",
    tray_open: "Open settings",
    tray_quit: "Quit",
    err_needs_mod: "Invalid: Requires Ctrl/Alt/Shift/Win",
    err_ctrl_reserved: "Invalid: Single Ctrl is reserved",
    err_sys_reserved: "Invalid: System-reserved shortcut",
    err_win_reserved: "Invalid: Windows-reserved shortcut",
    err_conflict: "Conflict: Already in use by another app",
    uninstall_btn: "\u{2620}",
    uninstall_tooltip: "Uninstall and delete app and its files",
    uninstall_title: "Uninstall Rust-Expander",
    uninstall_body: "This will permanently delete:\r\n\r\n  \u{2022} All settings and snippets\r\n  \u{2022} The application .exe file\r\n  \u{2022} The debug log\r\n\r\nThe app closes immediately. The .exe is removed a moment later.\r\n\r\nThis cannot be undone. Proceed?",
    btn_cancel_tooltip: "Undo all changes since last save",
};

const STRINGS_DA: Strings = Strings {
    window_title: "Rust-Expander - Indstillinger",
    header: "Indstillinger",
    hotkey_label: "Global genvejstast:",
    hotkey_save: "Sæt",
    hotkey_prompt: "Tast ny taste-kombination",
    buffer_label: "Buffer:",
    buffer_empty: "(tom)",
    col_trigger: "Sekvens (max 10)",
    col_trigger_tooltip: "Når du taster en af disse tegnsekvenser, erstattes de enten med det samme eller når du trykker på den valgte genvejstast",
    col_replacement: "Erstatning (mellemrum til sidst?)",
    col_mode: "Mode",
    mode_immediate: "⚡ Omgående",
    mode_hotkey: "⌨ Genvej",
    btn_delete: "Slet",
    btn_add: "+ Tilføj ny",
    btn_quit: "Afslut",
    btn_pause: "Pause",
    btn_resume: "Genoptag",
    btn_pause_tooltip: "Sæt teksterstatning midlertidigt på pause",
    btn_cancel: "Annuller",
    btn_save: "Gem",
    tray_tooltip: "Rust-Expander\nKlik for indstillinger",
    tray_open: "Åbn indstillinger",
    tray_quit: "Afslut",
    err_needs_mod: "Ugyldig: Kræver Ctrl/Alt/Shift/Win",
    err_ctrl_reserved: "Ugyldig: Enkelt Ctrl er reserveret",
    err_sys_reserved: "Ugyldig: System-reserveret genvej",
    err_win_reserved: "Ugyldig: Windows-reserveret genvej",
    err_conflict: "Konflikt: Allerede i brug af anden app",
    uninstall_btn: "\u{2620}",
    uninstall_tooltip: "Afinstaller og slet app og dens filer",
    uninstall_title: "Afinstaller Rust-Expander",
    uninstall_body: "Dette vil permanent slette:\r\n\r\n  \u{2022} Alle indstillinger og genvejstekster\r\n  \u{2022} Applikationens .exe-fil\r\n  \u{2022} Debug-loggen\r\n\r\nAppen lukker med det samme. .exe-filen fjernes kort efter.\r\n\r\nDette kan ikke fortrydes. Forts\u{00E6}t?",
    btn_cancel_tooltip: "Fortryd alle ændringer siden sidste gem",
};
