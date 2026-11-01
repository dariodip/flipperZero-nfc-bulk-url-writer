//! Strict HTTP/HTTPS URL validation.
//!
//! The application only needs to recognise a URL, preserve it byte-for-byte and
//! reject input that cannot be stored safely on a tag. It intentionally does
//! not implement full RFC 3986 parsing or normalisation.

use crate::errors::UrlError;

/// A validated URL and the byte offsets of its interesting parts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UrlParts<'a> {
    /// The complete URL, exactly as supplied.
    pub bytes: &'a [u8],
    /// Offset at which the scheme starts (always zero).
    pub scheme_start: usize,
    /// Offset of `://`.
    pub scheme_end: usize,
    /// Offset where the host starts.
    pub host_start: usize,
    /// Offset one past the last host byte.
    pub host_end: usize,
}

impl<'a> UrlParts<'a> {
    pub fn scheme(&self) -> &'a [u8] {
        &self.bytes[self.scheme_start..self.scheme_end]
    }

    pub fn host(&self) -> &'a [u8] {
        &self.bytes[self.host_start..self.host_end]
    }
}

/// Validates an HTTP or HTTPS URL and returns its component offsets.
///
/// Rules:
/// - the scheme is `http` or `https`, compared case-insensitively;
/// - `://` is required;
/// - the host is non-empty;
/// - every byte is printable ASCII (0x21..=0x7E), which excludes spaces,
///   controls and non-ASCII input;
/// - every `%` is followed by two hexadecimal digits.
///
/// The input is never trimmed or normalised.
pub fn validate(input: &[u8]) -> Result<UrlParts<'_>, UrlError> {
    if input.is_empty() {
        return Err(UrlError::Empty);
    }

    for &byte in input {
        if !(0x21..=0x7E).contains(&byte) {
            return Err(UrlError::IllegalCharacter);
        }
    }

    let separator = find(input, b"://").ok_or(UrlError::UnsupportedScheme)?;
    let scheme = &input[..separator];
    let is_http = eq_ignore_ascii_case(scheme, b"http");
    let is_https = eq_ignore_ascii_case(scheme, b"https");
    if !is_http && !is_https {
        return Err(UrlError::UnsupportedScheme);
    }

    let host_start = separator + 3;
    let mut host_end = input.len();
    for (offset, &byte) in input[host_start..].iter().enumerate() {
        if byte == b'/' || byte == b'?' || byte == b'#' {
            host_end = host_start + offset;
            break;
        }
    }
    if host_end == host_start {
        return Err(UrlError::MissingHost);
    }

    validate_percent_escapes(input)?;

    Ok(UrlParts {
        bytes: input,
        scheme_start: 0,
        scheme_end: separator,
        host_start,
        host_end,
    })
}

/// Returns true when the input is a valid HTTP/HTTPS URL.
pub fn is_valid(input: &[u8]) -> bool {
    validate(input).is_ok()
}

/// Validates every percent escape in the input.
pub fn validate_percent_escapes(input: &[u8]) -> Result<(), UrlError> {
    let mut index = 0;
    while index < input.len() {
        if input[index] == b'%' {
            let high = input.get(index + 1).copied();
            let low = input.get(index + 2).copied();
            match (high, low) {
                (Some(high), Some(low)) if is_hex(high) && is_hex(low) => {
                    index += 3;
                    continue;
                }
                _ => return Err(UrlError::InvalidPercentEscape),
            }
        }
        index += 1;
    }
    Ok(())
}

fn is_hex(byte: u8) -> bool {
    byte.is_ascii_hexdigit()
}

fn eq_ignore_ascii_case(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_http_and_https() {
        assert!(is_valid(b"http://example.test/contact/1"));
        assert!(is_valid(b"https://example.test/contact/2"));
        assert!(is_valid(b"HTTPS://example.test/Contact/ABC"));
    }

    #[test]
    fn preserves_original_bytes() {
        let parts = validate(b"HTTPS://Example.Test/A?x=1#frag").unwrap();
        assert_eq!(parts.bytes, b"HTTPS://Example.Test/A?x=1#frag");
        assert_eq!(parts.scheme(), b"HTTPS");
        assert_eq!(parts.host(), b"Example.Test");
    }

    #[test]
    fn rejects_unsupported_scheme() {
        assert_eq!(
            validate(b"ftp://example.test/file"),
            Err(UrlError::UnsupportedScheme)
        );
        assert_eq!(
            validate(b"mailto:a@example.test"),
            Err(UrlError::UnsupportedScheme)
        );
    }

    #[test]
    fn rejects_missing_host() {
        assert_eq!(validate(b"https:///contact"), Err(UrlError::MissingHost));
        assert_eq!(validate(b"https://"), Err(UrlError::MissingHost));
        assert_eq!(validate(b"https://?x=1"), Err(UrlError::MissingHost));
    }

    #[test]
    fn rejects_spaces_controls_and_non_ascii() {
        assert_eq!(
            validate(b"https://example.test/a b"),
            Err(UrlError::IllegalCharacter)
        );
        assert_eq!(
            validate(b" https://example.test/"),
            Err(UrlError::IllegalCharacter)
        );
        assert_eq!(
            validate(b"https://example.test/\x01"),
            Err(UrlError::IllegalCharacter)
        );
        assert_eq!(
            validate(b"https://example.test/\xc3\xa9"),
            Err(UrlError::IllegalCharacter)
        );
    }

    #[test]
    fn rejects_incomplete_percent_escape() {
        assert_eq!(
            validate(b"https://example.test/%2"),
            Err(UrlError::InvalidPercentEscape)
        );
        assert_eq!(
            validate(b"https://example.test/%zz"),
            Err(UrlError::InvalidPercentEscape)
        );
        assert!(is_valid(b"https://example.test/%20"));
    }

    #[test]
    fn accepts_host_only_and_query_without_path() {
        assert!(is_valid(b"https://example.test"));
        assert!(is_valid(b"https://example.test?x=1"));
        assert!(is_valid(b"https://example.test#top"));
    }
}
