//! Streaming playlist parsing, validation and fingerprinting.
//!
//! A playlist is read line by line so the application can handle files larger
//! than its fixed buffers. Each accepted line becomes one tag entry, in file
//! order, and duplicate URLs are preserved as independent entries.

use crate::MAX_URL_LEN;
use crate::errors::PlaylistError;
use crate::url;

/// A source of playlist lines without their line terminators.
pub trait LineSource {
    /// Returns the next line length written into `buf`, or `None` at end of file.
    ///
    /// Implementations must strip the trailing `\n` and, if present, the
    /// trailing `\r`. They must not trim other whitespace.
    fn next_line(&mut self, buf: &mut [u8]) -> Result<Option<usize>, LineReadError>;
}

/// Failure while reading a playlist line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineReadError {
    Io,
    LineTooLong,
}

/// In-memory [`LineSource`], used by tests and by host-side tooling.
pub struct SliceLineSource<'a> {
    data: &'a [u8],
    position: usize,
}

impl<'a> SliceLineSource<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, position: 0 }
    }
}

impl LineSource for SliceLineSource<'_> {
    fn next_line(&mut self, buf: &mut [u8]) -> Result<Option<usize>, LineReadError> {
        if self.position >= self.data.len() {
            return Ok(None);
        }

        let remaining = &self.data[self.position..];
        let end = remaining
            .iter()
            .position(|&byte| byte == b'\n')
            .map(|offset| self.position + offset + 1)
            .unwrap_or(self.data.len());
        let mut line = &self.data[self.position..end];
        self.position = end;

        if line.last() == Some(&b'\n') {
            line = &line[..line.len() - 1];
        }
        if line.last() == Some(&b'\r') {
            line = &line[..line.len() - 1];
        }

        if line.len() > buf.len() {
            return Err(LineReadError::LineTooLong);
        }
        buf[..line.len()].copy_from_slice(line);
        Ok(Some(line.len()))
    }
}

/// Result of scanning a complete playlist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaylistStats {
    /// Number of accepted URL entries.
    pub total: u32,
    /// Semantic fingerprint of the ordered URL sequence.
    pub fingerprint: u64,
}

/// One classified playlist line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Line<'a> {
    Ignored,
    Url(&'a [u8]),
}

/// Strips an optional UTF-8 byte-order mark from the first line.
pub fn strip_bom(line: &[u8], first_line: bool) -> &[u8] {
    if first_line && line.starts_with(&[0xEF, 0xBB, 0xBF]) {
        &line[3..]
    } else {
        line
    }
}

/// Classifies a line as an entry, a comment, or blank.
///
/// The CR of a CRLF line ending is ignored even if the source left it in
/// place. A URL line is returned verbatim; it is not trimmed.
pub fn classify_line(line: &[u8], first_line: bool) -> Line<'_> {
    let line = strip_bom(line, first_line);
    let line = line.strip_suffix(b"\r").unwrap_or(line);

    let mut index = 0;
    while index < line.len() && (line[index] == b' ' || line[index] == b'\t') {
        index += 1;
    }
    if index == line.len() {
        return Line::Ignored;
    }
    if line[index] == b'#' {
        return Line::Ignored;
    }
    Line::Url(line)
}

/// Scans and validates every entry, returning the count and fingerprint.
pub fn scan<S: LineSource>(
    source: &mut S,
    line_buf: &mut [u8],
) -> Result<PlaylistStats, PlaylistError> {
    let mut hash = FNV_OFFSET_BASIS;
    let mut total = 0u32;
    let mut line_number = 0u32;
    let mut first_line = true;

    loop {
        let read = source.next_line(line_buf).map_err(map_line_error)?;
        let Some(length) = read else {
            break;
        };
        line_number += 1;
        let line = &line_buf[..length];
        match classify_line(line, first_line) {
            Line::Ignored => {}
            Line::Url(entry) => {
                url::validate(entry).map_err(|reason| PlaylistError::InvalidUrl {
                    line: line_number,
                    reason,
                })?;
                hash = fingerprint_entry(hash, entry);
                total += 1;
            }
        }
        first_line = false;
    }

    if total == 0 {
        return Err(PlaylistError::Empty);
    }

    Ok(PlaylistStats {
        total,
        fingerprint: hash,
    })
}

/// Copies the URL at `index` (zero-based) into `out` and returns its length.
pub fn entry_at<S: LineSource>(
    source: &mut S,
    index: u32,
    line_buf: &mut [u8],
    out: &mut [u8],
) -> Result<usize, PlaylistError> {
    let mut entry_index = 0u32;
    let mut line_number = 0u32;
    let mut first_line = true;

    loop {
        let read = source.next_line(line_buf).map_err(map_line_error)?;
        let Some(length) = read else {
            return Err(PlaylistError::IndexOutOfRange);
        };
        line_number += 1;
        let line = &line_buf[..length];
        if let Line::Url(entry) = classify_line(line, first_line) {
            url::validate(entry).map_err(|reason| PlaylistError::InvalidUrl {
                line: line_number,
                reason,
            })?;
            if entry_index == index {
                if entry.len() > out.len() {
                    return Err(PlaylistError::UrlTooLong);
                }
                out[..entry.len()].copy_from_slice(entry);
                return Ok(entry.len());
            }
            entry_index += 1;
        }
        first_line = false;
    }
}

fn map_line_error(error: LineReadError) -> PlaylistError {
    match error {
        LineReadError::Io => PlaylistError::Source,
        LineReadError::LineTooLong => PlaylistError::LineTooLong,
    }
}

const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn fingerprint_entry(mut hash: u64, entry: &[u8]) -> u64 {
    // An explicit length prefix makes the encoding unambiguous.
    for byte in (entry.len() as u32).to_le_bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    for &byte in entry {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Computes the fingerprint of an already-validated ordered URL sequence.
pub fn fingerprint_urls<'a>(urls: impl IntoIterator<Item = &'a [u8]>) -> u64 {
    let mut hash = FNV_OFFSET_BASIS;
    for entry in urls {
        hash = fingerprint_entry(hash, entry);
    }
    hash
}

/// Recommended buffer size for one playlist line.
pub const LINE_BUFFER_SIZE: usize = MAX_URL_LEN + 1;

#[cfg(test)]
mod tests {
    use super::*;

    const LINE_BUF: usize = 64;
    const OUT_BUF: usize = 64;

    fn stats(text: &[u8]) -> Result<PlaylistStats, PlaylistError> {
        let mut source = SliceLineSource::new(text);
        let mut line = [0u8; LINE_BUF];
        scan(&mut source, &mut line)
    }

    fn entry(text: &[u8], index: u32) -> Result<std::vec::Vec<u8>, PlaylistError> {
        let mut source = SliceLineSource::new(text);
        let mut line = [0u8; LINE_BUF];
        let mut out = [0u8; OUT_BUF];
        let length = entry_at(&mut source, index, &mut line, &mut out)?;
        Ok(out[..length].to_vec())
    }

    #[test]
    fn ignores_comments_and_blank_lines() {
        let text = b"# first\n\nhttps://a.test/1\n   # indented\n\t\nhttp://b.test/2\n";
        assert_eq!(stats(text).unwrap().total, 2);
        assert_eq!(entry(text, 0).unwrap(), b"https://a.test/1");
        assert_eq!(entry(text, 1).unwrap(), b"http://b.test/2");
    }

    #[test]
    fn handles_crlf_and_bom() {
        let text = b"\xEF\xBB\xBFhttps://a.test/1\r\nhttps://b.test/2\r\n";
        assert_eq!(stats(text).unwrap().total, 2);
        assert_eq!(entry(text, 0).unwrap(), b"https://a.test/1");
        assert_eq!(entry(text, 1).unwrap(), b"https://b.test/2");
    }

    #[test]
    fn empty_playlist_is_rejected() {
        assert_eq!(stats(b"# only\n\n \n"), Err(PlaylistError::Empty));
        assert_eq!(stats(b""), Err(PlaylistError::Empty));
    }

    #[test]
    fn preserves_order_and_duplicates() {
        let text = b"https://a.test/1\nhttps://a.test/1\nhttps://b.test/2\n";
        assert_eq!(stats(text).unwrap().total, 3);
        assert_eq!(entry(text, 0).unwrap(), b"https://a.test/1");
        assert_eq!(entry(text, 1).unwrap(), b"https://a.test/1");
        assert_eq!(entry(text, 2).unwrap(), b"https://b.test/2");
    }

    #[test]
    fn rejects_whitespace_around_url_instead_of_trimming() {
        let text = b" https://a.test/1\n";
        assert_eq!(
            stats(text),
            Err(PlaylistError::InvalidUrl {
                line: 1,
                reason: crate::errors::UrlError::IllegalCharacter
            })
        );
    }

    #[test]
    fn reports_line_numbers() {
        let text = b"# comment\nhttps://ok.test/\nftp://bad.test/\n";
        assert_eq!(
            stats(text),
            Err(PlaylistError::InvalidUrl {
                line: 3,
                reason: crate::errors::UrlError::UnsupportedScheme
            })
        );
    }

    #[test]
    fn index_out_of_range() {
        assert_eq!(
            entry(b"https://a.test/1\n", 1),
            Err(PlaylistError::IndexOutOfRange)
        );
    }

    #[test]
    fn fingerprint_ignores_comments_blank_lines_crlf_and_bom() {
        let a = stats(b"# a\nhttps://x.test/1\n\nhttps://y.test/2\n").unwrap();
        let b = stats(b"\xEF\xBB\xBFhttps://x.test/1\r\n# b\r\nhttps://y.test/2\r\n").unwrap();
        assert_eq!(a.fingerprint, b.fingerprint);
        assert_eq!(a.total, b.total);
    }

    #[test]
    fn fingerprint_changes_with_content_and_order() {
        let a = stats(b"https://x.test/1\nhttps://y.test/2\n").unwrap();
        let b = stats(b"https://y.test/2\nhttps://x.test/1\n").unwrap();
        let c = stats(b"https://x.test/1\nhttps://y.test/3\n").unwrap();
        let d = stats(b"https://x.test/1\n").unwrap();
        let e = stats(b"https://x.test/1\nhttps://y.test/2\nhttps://z.test/3\n").unwrap();
        assert_ne!(a.fingerprint, b.fingerprint);
        assert_ne!(a.fingerprint, c.fingerprint);
        assert_ne!(a.fingerprint, d.fingerprint);
        assert_ne!(a.fingerprint, e.fingerprint);
    }

    #[test]
    fn duplicate_order_matters() {
        let a = stats(b"https://x.test/1\nhttps://x.test/1\nhttps://y.test/2\n").unwrap();
        let b = stats(b"https://x.test/1\nhttps://y.test/2\nhttps://x.test/1\n").unwrap();
        assert_ne!(a.fingerprint, b.fingerprint);
    }

    #[test]
    fn line_too_long_is_reported() {
        let mut source = SliceLineSource::new(b"https://a.test/very-long-path\n");
        let mut line = [0u8; 8];
        assert_eq!(
            scan(&mut source, &mut line),
            Err(PlaylistError::LineTooLong)
        );
    }

    struct FailingSource;

    impl LineSource for FailingSource {
        fn next_line(&mut self, _buf: &mut [u8]) -> Result<Option<usize>, LineReadError> {
            Err(LineReadError::Io)
        }
    }

    #[test]
    fn source_failure_is_reported() {
        let mut source = FailingSource;
        let mut line = [0u8; LINE_BUF];
        assert_eq!(scan(&mut source, &mut line), Err(PlaylistError::Source));
        assert_eq!(
            entry_at(&mut source, 0, &mut line, &mut [0u8; OUT_BUF]),
            Err(PlaylistError::Source)
        );
    }

    #[test]
    fn missing_entry_is_out_of_range() {
        assert_eq!(entry(b"", 0), Err(PlaylistError::IndexOutOfRange));
        assert_eq!(
            entry(b"# only comments\n", 0),
            Err(PlaylistError::IndexOutOfRange)
        );
    }

    #[test]
    fn documented_example_parses() {
        let example = b"# Monday batch\n\
https://example.test/contact/anna\n\
\n\
http://example.test/contact/bruno\n\
# The same URL may appear twice on purpose\n\
https://example.test/contact/anna\n";
        let stats = stats(example).unwrap();
        assert_eq!(stats.total, 3);
        assert_eq!(
            entry(example, 0).unwrap(),
            b"https://example.test/contact/anna"
        );
        assert_eq!(
            entry(example, 1).unwrap(),
            b"http://example.test/contact/bruno"
        );
        assert_eq!(
            entry(example, 2).unwrap(),
            b"https://example.test/contact/anna"
        );
    }
}
