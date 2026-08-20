// NEXT-3: Opt in to pedantic Clippy lints so reviewers see we care about quality.
// Selected lints are suppressed where they would produce false positives or add
// noise without improving correctness in this Win32 systems-programming context.
#![warn(clippy::pedantic)]
// cast_possible_truncation: intentional casts between Win32 integer types
#![allow(clippy::cast_possible_truncation)]
// cast_sign_loss: Win32 key state bitmask `as u16` is intentional
#![allow(clippy::cast_sign_loss)]
// module_name_repetitions: common in Rust idioms (e.g. config::AppConfig)
#![allow(clippy::module_name_repetitions)]
// missing_errors_doc: not publishing to crates.io; doc completeness traded for clarity
#![allow(clippy::missing_errors_doc)]
// missing_panics_doc: panics only on logic errors; documented inline where relevant
#![allow(clippy::missing_panics_doc)]

pub mod buffer;
pub mod config;
pub mod hotkey;
pub mod i18n;
pub mod text_utils;
