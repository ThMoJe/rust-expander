//! Small text-processing utilities used by the replacer and elsewhere.
//!
//! Lives in the library crate so the functions can be unit-tested by `cargo test`.

/// Normalises line endings in a string to CRLF (`\r\n`), as required by the
/// Win32 `CF_UNICODETEXT` clipboard format specification.
///
/// Any existing `\r\n` sequences are first collapsed to `\n` to prevent
/// double-conversion, then every remaining `\n` is replaced with `\r\n`.
#[must_use]
pub fn normalise_to_crlf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\n', "\r\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lf_converted_to_crlf() {
        assert_eq!(normalise_to_crlf("hello\nworld"), "hello\r\nworld");
    }

    #[test]
    fn test_crlf_unchanged() {
        assert_eq!(normalise_to_crlf("hello\r\nworld"), "hello\r\nworld");
    }

    #[test]
    fn test_mixed_endings_normalised() {
        assert_eq!(
            normalise_to_crlf("a\r\nb\nc"),
            "a\r\nb\r\nc"
        );
    }

    #[test]
    fn test_no_newlines_unchanged() {
        assert_eq!(normalise_to_crlf("hello world"), "hello world");
    }

    #[test]
    fn test_empty_string() {
        assert_eq!(normalise_to_crlf(""), "");
    }
}
