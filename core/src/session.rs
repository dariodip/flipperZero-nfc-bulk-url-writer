//! Versioned, checksummed batch session persistence.
//!
//! The on-card format is line-oriented text so it can be inspected and
//! recovered with ordinary tools. The checksum covers every byte before the
//! checksum line, and an invalid record is never used to resume.

use crate::errors::SessionError;

/// Current session format version.
pub const SESSION_VERSION: u32 = 1;
/// Maximum playlist path stored in a session.
pub const MAX_PATH_LEN: usize = 256;
/// Maximum serialized session size.
pub const MAX_SESSION_LEN: usize = 1024;

const HEADER_PREFIX: &[u8] = b"NFC-BULK-URL-WRITER-SESSION ";
const CHECKSUM_PREFIX: &[u8] = b"checksum=";

const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Fixed-capacity playlist path.
#[derive(Clone, Copy, Debug)]
pub struct FixedPath {
    bytes: [u8; MAX_PATH_LEN],
    len: u16,
}

impl FixedPath {
    pub fn from_bytes(path: &[u8]) -> Result<Self, SessionError> {
        if path.len() > MAX_PATH_LEN {
            return Err(SessionError::PathTooLong);
        }
        if path.contains(&b'\n') || path.contains(&b'\r') {
            return Err(SessionError::Corrupt);
        }
        let mut bytes = [0u8; MAX_PATH_LEN];
        bytes[..path.len()].copy_from_slice(path);
        Ok(Self {
            bytes,
            len: path.len() as u16,
        })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }

    pub fn as_str(&self) -> Option<&str> {
        core::str::from_utf8(self.as_bytes()).ok()
    }
}

impl Default for FixedPath {
    fn default() -> Self {
        Self {
            bytes: [0u8; MAX_PATH_LEN],
            len: 0,
        }
    }
}

/// Persisted progress for one playlist.
#[derive(Clone, Debug)]
pub struct Session {
    pub fingerprint: u64,
    pub total: u32,
    pub next: u32,
    pub completed: u32,
    pub skipped: u32,
    pub failed: u32,
    pub complete: bool,
    path: FixedPath,
}

impl Session {
    /// Creates a fresh session at the start of a playlist.
    pub fn new(fingerprint: u64, total: u32, path: &[u8]) -> Result<Self, SessionError> {
        if total == 0 {
            return Err(SessionError::Invariant);
        }
        Ok(Self {
            fingerprint,
            total,
            next: 0,
            completed: 0,
            skipped: 0,
            failed: 0,
            complete: false,
            path: FixedPath::from_bytes(path)?,
        })
    }

    pub fn path(&self) -> &[u8] {
        self.path.as_bytes()
    }

    pub fn path_str(&self) -> Option<&str> {
        self.path.as_str()
    }

    /// Returns true when every URL has been verified or skipped.
    pub fn is_complete(&self) -> bool {
        self.complete
    }

    /// Remaining entries, including the current one.
    pub fn remaining(&self) -> u32 {
        self.total.saturating_sub(self.next)
    }

    /// Validates all counter invariants.
    pub fn validate(&self) -> Result<(), SessionError> {
        if self.total == 0
            || self.next > self.total
            || self.completed.checked_add(self.skipped) != Some(self.next)
            || self.next.checked_add(self.failed).is_none()
            || self.complete != (self.next == self.total)
        {
            return Err(SessionError::Invariant);
        }
        Ok(())
    }

    /// Counts a verified write and advances to the next entry.
    pub fn record_verified(&mut self) -> Result<(), SessionError> {
        if self.next >= self.total {
            return Err(SessionError::Completed);
        }
        self.completed += 1;
        self.next += 1;
        self.complete = self.next == self.total;
        Ok(())
    }

    /// Counts an explicit skip and advances to the next entry.
    pub fn record_skipped(&mut self) -> Result<(), SessionError> {
        if self.next >= self.total {
            return Err(SessionError::Completed);
        }
        self.skipped += 1;
        self.next += 1;
        self.complete = self.next == self.total;
        Ok(())
    }

    /// Counts a failed tag attempt without advancing.
    pub fn record_failed_attempt(&mut self) {
        self.failed = self.failed.saturating_add(1);
    }

    /// Serializes the session into `out` and returns the used length.
    pub fn serialize(&self, out: &mut [u8]) -> Result<usize, SessionError> {
        self.validate()?;

        let mut position = 0usize;
        put_bytes(out, &mut position, HEADER_PREFIX)?;
        put_decimal(out, &mut position, SESSION_VERSION)?;
        put_byte(out, &mut position, b'\n')?;

        put_bytes(out, &mut position, b"playlist=")?;
        put_bytes(out, &mut position, self.path())?;
        put_byte(out, &mut position, b'\n')?;

        put_bytes(out, &mut position, b"fingerprint=")?;
        put_hex_u64(out, &mut position, self.fingerprint)?;
        put_byte(out, &mut position, b'\n')?;

        put_decimal_line(out, &mut position, b"total=", self.total)?;
        put_decimal_line(out, &mut position, b"next=", self.next)?;
        put_decimal_line(out, &mut position, b"completed=", self.completed)?;
        put_decimal_line(out, &mut position, b"skipped=", self.skipped)?;
        put_decimal_line(out, &mut position, b"failed=", self.failed)?;

        put_bytes(out, &mut position, b"complete=")?;
        put_byte(out, &mut position, if self.complete { b'1' } else { b'0' })?;
        put_byte(out, &mut position, b'\n')?;

        let checksum = checksum(&out[..position]);
        put_bytes(out, &mut position, CHECKSUM_PREFIX)?;
        put_hex_u64(out, &mut position, checksum)?;
        put_byte(out, &mut position, b'\n')?;

        Ok(position)
    }

    /// Parses and verifies a serialized session.
    pub fn parse(data: &[u8]) -> Result<Self, SessionError> {
        let mut cursor = 0usize;

        let (_, header) = next_line(data, &mut cursor).ok_or(SessionError::Corrupt)?;
        let version_bytes = header
            .strip_prefix(HEADER_PREFIX)
            .ok_or(SessionError::Corrupt)?;
        let version = parse_decimal(version_bytes).ok_or(SessionError::Corrupt)?;
        if version != SESSION_VERSION {
            return Err(SessionError::UnsupportedVersion);
        }

        let (_, path_line) = next_line(data, &mut cursor).ok_or(SessionError::Corrupt)?;
        let path = path_line
            .strip_prefix(b"playlist=")
            .ok_or(SessionError::Corrupt)?;

        let (_, fingerprint_line) = next_line(data, &mut cursor).ok_or(SessionError::Corrupt)?;
        let fingerprint = fingerprint_line
            .strip_prefix(b"fingerprint=")
            .and_then(parse_hex_u64)
            .ok_or(SessionError::Corrupt)?;

        let (_, total_line) = next_line(data, &mut cursor).ok_or(SessionError::Corrupt)?;
        let total = total_line
            .strip_prefix(b"total=")
            .and_then(parse_decimal)
            .ok_or(SessionError::Corrupt)?;

        let (_, next_line_bytes) = next_line(data, &mut cursor).ok_or(SessionError::Corrupt)?;
        let next = next_line_bytes
            .strip_prefix(b"next=")
            .and_then(parse_decimal)
            .ok_or(SessionError::Corrupt)?;

        let (_, completed_line) = next_line(data, &mut cursor).ok_or(SessionError::Corrupt)?;
        let completed = completed_line
            .strip_prefix(b"completed=")
            .and_then(parse_decimal)
            .ok_or(SessionError::Corrupt)?;

        let (_, skipped_line) = next_line(data, &mut cursor).ok_or(SessionError::Corrupt)?;
        let skipped = skipped_line
            .strip_prefix(b"skipped=")
            .and_then(parse_decimal)
            .ok_or(SessionError::Corrupt)?;

        let (_, failed_line) = next_line(data, &mut cursor).ok_or(SessionError::Corrupt)?;
        let failed = failed_line
            .strip_prefix(b"failed=")
            .and_then(parse_decimal)
            .ok_or(SessionError::Corrupt)?;

        let (_, complete_line) = next_line(data, &mut cursor).ok_or(SessionError::Corrupt)?;
        let complete = match complete_line.strip_prefix(b"complete=") {
            Some(b"1") => true,
            Some(b"0") => false,
            _ => return Err(SessionError::Corrupt),
        };

        let (checksum_start, checksum_line) =
            next_line(data, &mut cursor).ok_or(SessionError::Corrupt)?;
        let expected = checksum_line
            .strip_prefix(CHECKSUM_PREFIX)
            .and_then(parse_hex_u64)
            .ok_or(SessionError::Corrupt)?;

        // Nothing but empty lines may follow the checksum.
        while let Some((_, trailing)) = next_line(data, &mut cursor) {
            if !trailing.is_empty() {
                return Err(SessionError::Corrupt);
            }
        }

        if checksum(&data[..checksum_start]) != expected {
            return Err(SessionError::Corrupt);
        }

        let session = Self {
            fingerprint,
            total,
            next,
            completed,
            skipped,
            failed,
            complete,
            path: FixedPath::from_bytes(path)?,
        };
        session.validate()?;
        Ok(session)
    }
}

/// Result of comparing a stored session with the current playlist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeStatus {
    Resume,
    Completed,
    PlaylistChanged,
    PlaylistMissing,
    NoSession,
}

/// Decides whether a stored session may be resumed.
pub fn resume_status(
    session: Option<&Session>,
    playlist_present: bool,
    fingerprint: u64,
    total: u32,
) -> ResumeStatus {
    let Some(session) = session else {
        return ResumeStatus::NoSession;
    };
    if !playlist_present {
        return ResumeStatus::PlaylistMissing;
    }
    if session.fingerprint != fingerprint || session.total != total {
        return ResumeStatus::PlaylistChanged;
    }
    if session.complete || session.next >= session.total {
        return ResumeStatus::Completed;
    }
    ResumeStatus::Resume
}

fn checksum(data: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET_BASIS;
    for &byte in data {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

fn next_line<'a>(data: &'a [u8], cursor: &mut usize) -> Option<(usize, &'a [u8])> {
    if *cursor >= data.len() {
        return None;
    }
    let start = *cursor;
    let relative_end = data[start..].iter().position(|&byte| byte == b'\n');
    let (end, next) = match relative_end {
        Some(offset) => (start + offset, start + offset + 1),
        None => (data.len(), data.len()),
    };
    *cursor = next;
    let mut line = &data[start..end];
    if line.last() == Some(&b'\r') {
        line = &line[..line.len() - 1];
    }
    Some((start, line))
}

fn parse_decimal(bytes: &[u8]) -> Option<u32> {
    if bytes.is_empty() {
        return None;
    }
    let mut value = 0u32;
    for &byte in bytes {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?.checked_add((byte - b'0') as u32)?;
    }
    Some(value)
}

fn parse_hex_u64(bytes: &[u8]) -> Option<u64> {
    if bytes.is_empty() || bytes.len() > 16 {
        return None;
    }
    let mut value = 0u64;
    for &byte in bytes {
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            _ => return None,
        };
        value = value.checked_mul(16)?.checked_add(digit as u64)?;
    }
    Some(value)
}

fn put_byte(out: &mut [u8], position: &mut usize, byte: u8) -> Result<(), SessionError> {
    if *position >= out.len() {
        return Err(SessionError::BufferTooSmall);
    }
    out[*position] = byte;
    *position += 1;
    Ok(())
}

fn put_bytes(out: &mut [u8], position: &mut usize, bytes: &[u8]) -> Result<(), SessionError> {
    if *position + bytes.len() > out.len() {
        return Err(SessionError::BufferTooSmall);
    }
    out[*position..*position + bytes.len()].copy_from_slice(bytes);
    *position += bytes.len();
    Ok(())
}

fn put_decimal(out: &mut [u8], position: &mut usize, mut value: u32) -> Result<(), SessionError> {
    let mut digits = [0u8; 10];
    let mut count = 0;
    if value == 0 {
        return put_byte(out, position, b'0');
    }
    while value > 0 {
        digits[count] = b'0' + (value % 10) as u8;
        value /= 10;
        count += 1;
    }
    while count > 0 {
        count -= 1;
        put_byte(out, position, digits[count])?;
    }
    Ok(())
}

fn put_decimal_line(
    out: &mut [u8],
    position: &mut usize,
    prefix: &[u8],
    value: u32,
) -> Result<(), SessionError> {
    put_bytes(out, position, prefix)?;
    put_decimal(out, position, value)?;
    put_byte(out, position, b'\n')
}

fn put_hex_u64(out: &mut [u8], position: &mut usize, value: u64) -> Result<(), SessionError> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    // Always emit 16 digits so the checksum has a stable width.
    for shift in (0..16).rev() {
        let digit = HEX[((value >> (shift * 4)) & 0xF) as usize];
        put_byte(out, position, digit)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Session {
        let mut session = Session::new(
            0x0123_4567_89AB_CDEF,
            42,
            b"/ext/apps_data/nfc-bulk-url-writer/playlists/batch.txt",
        )
        .unwrap();
        session.record_verified().unwrap();
        session.record_verified().unwrap();
        session.record_skipped().unwrap();
        session.record_failed_attempt();
        session
    }

    fn serialize(session: &Session) -> std::vec::Vec<u8> {
        let mut buffer = [0u8; MAX_SESSION_LEN];
        let length = session.serialize(&mut buffer).unwrap();
        buffer[..length].to_vec()
    }

    #[test]
    fn round_trips() {
        let session = sample();
        let bytes = serialize(&session);
        let parsed = Session::parse(&bytes).unwrap();
        assert_eq!(parsed.fingerprint, session.fingerprint);
        assert_eq!(parsed.total, 42);
        assert_eq!(parsed.next, 3);
        assert_eq!(parsed.completed, 2);
        assert_eq!(parsed.skipped, 1);
        assert_eq!(parsed.failed, 1);
        assert!(!parsed.complete);
        assert_eq!(parsed.path(), session.path());
        assert!(parsed.validate().is_ok());
    }

    #[test]
    fn rejects_unknown_version() {
        let mut bytes = serialize(&sample());
        let position = b"NFC-BULK-URL-WRITER-SESSION ".len();
        bytes[position] = b'2';
        assert_eq!(
            Session::parse(&bytes).unwrap_err(),
            SessionError::UnsupportedVersion
        );
    }

    #[test]
    fn rejects_truncated_and_checksum_mismatch() {
        let bytes = serialize(&sample());
        for cut in [5usize, 20] {
            assert_eq!(
                Session::parse(&bytes[..bytes.len() - cut]).unwrap_err(),
                SessionError::Corrupt
            );
        }

        let mut corrupted = bytes.clone();
        let marker = b"total=42";
        let position = corrupted
            .windows(marker.len())
            .position(|window| window == marker)
            .unwrap();
        corrupted[position + 6] = b'4';
        corrupted[position + 7] = b'3';
        assert_eq!(
            Session::parse(&corrupted).unwrap_err(),
            SessionError::Corrupt
        );
    }

    #[test]
    fn rejects_inconsistent_counters() {
        let mut session = sample();
        session.next = 9;
        assert_eq!(session.validate(), Err(SessionError::Invariant));
        assert_eq!(
            session.serialize(&mut [0u8; MAX_SESSION_LEN]),
            Err(SessionError::Invariant)
        );

        let mut finished = Session::new(1, 1, b"/a").unwrap();
        finished.record_verified().unwrap();
        assert!(finished.complete);
        assert!(finished.validate().is_ok());
    }

    #[test]
    fn completed_session_cannot_advance() {
        let mut session = Session::new(1, 1, b"/a").unwrap();
        session.record_verified().unwrap();
        assert_eq!(session.record_verified(), Err(SessionError::Completed));
        assert_eq!(session.record_skipped(), Err(SessionError::Completed));
    }

    #[test]
    fn resume_decisions() {
        let session = sample();
        let current = (session.fingerprint, session.total);
        assert_eq!(
            resume_status(Some(&session), true, current.0, current.1),
            ResumeStatus::Resume
        );
        assert_eq!(
            resume_status(Some(&session), false, current.0, current.1),
            ResumeStatus::PlaylistMissing
        );
        assert_eq!(
            resume_status(Some(&session), true, current.0 + 1, current.1),
            ResumeStatus::PlaylistChanged
        );
        assert_eq!(
            resume_status(Some(&session), true, current.0, current.1 + 1),
            ResumeStatus::PlaylistChanged
        );
        assert_eq!(
            resume_status(None, true, current.0, current.1),
            ResumeStatus::NoSession
        );

        let mut completed = session.clone();
        completed.next = 42;
        completed.completed = 41;
        completed.skipped = 1;
        completed.complete = true;
        assert_eq!(
            resume_status(
                Some(&completed),
                true,
                completed.fingerprint,
                completed.total
            ),
            ResumeStatus::Completed
        );
    }

    #[test]
    fn failed_attempts_do_not_advance() {
        let mut session = sample();
        let snapshot = (session.next, session.completed, session.skipped);
        session.record_failed_attempt();
        assert_eq!((session.next, session.completed, session.skipped), snapshot);
        assert_eq!(session.failed, 2);
    }

    #[test]
    fn path_length_and_newline_are_checked() {
        assert_eq!(
            Session::new(1, 1, &[b'a'; MAX_PATH_LEN + 1]).unwrap_err(),
            SessionError::PathTooLong
        );
        assert_eq!(
            Session::new(1, 1, b"/a\nb").unwrap_err(),
            SessionError::Corrupt
        );
    }

    #[test]
    fn buffer_too_small() {
        let session = sample();
        assert_eq!(
            session.serialize(&mut [0u8; 16]),
            Err(SessionError::BufferTooSmall)
        );
    }
}
