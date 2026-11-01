//! Error types shared by the core modules.

use core::fmt;

/// Reason a playlist line is not an acceptable URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrlError {
    Empty,
    UnsupportedScheme,
    MissingHost,
    IllegalCharacter,
    InvalidPercentEscape,
}

impl fmt::Display for UrlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Empty => "empty URL",
            Self::UnsupportedScheme => "only http:// and https:// are supported",
            Self::MissingHost => "the URL has no host",
            Self::IllegalCharacter => "the URL contains a space, control or non-ASCII character",
            Self::InvalidPercentEscape => "the URL contains an incomplete percent escape",
        };
        f.write_str(text)
    }
}

/// Playlist discovery, parsing or validation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaylistError {
    /// The underlying line source failed.
    Source,
    /// The line did not fit in the caller's buffer.
    LineTooLong,
    /// The playlist contained only comments and blank lines.
    Empty,
    /// A specific line is not a usable URL.
    InvalidUrl { line: u32, reason: UrlError },
    /// The requested entry is past the end of the playlist.
    IndexOutOfRange,
    /// The entry did not fit in the caller's output buffer.
    UrlTooLong,
}

impl fmt::Display for PlaylistError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source => f.write_str("could not read the playlist"),
            Self::LineTooLong => f.write_str("a playlist line is too long"),
            Self::Empty => f.write_str("the playlist contains no URLs"),
            Self::InvalidUrl { line, reason } => {
                write!(f, "playlist line {line}: {reason}")
            }
            Self::IndexOutOfRange => f.write_str("the requested playlist entry does not exist"),
            Self::UrlTooLong => f.write_str("the playlist entry is too long"),
        }
    }
}

/// NDEF / Type 2 Tag encoding or verification failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NdefError {
    Url(UrlError),
    /// The supplied output buffer is too small for the encoded message.
    BufferTooSmall,
    /// The encoded TLV does not fit the tag's NDEF memory area.
    InsufficientCapacity {
        required: usize,
        available: usize,
    },
    /// The data ended before a complete TLV was available.
    Truncated,
    /// The TLV tag byte was not an NDEF Message TLV.
    BadTlvTag,
    /// A long TLV length was declared without two length bytes.
    BadTlvLength,
    /// The mandatory `0xFE` terminator was missing.
    MissingTerminator,
    /// The record header was not a single, complete, well-known URI record.
    BadRecordHeader,
    /// The record type was not `U`.
    BadRecordType,
    /// The URI identifier code is not a defined NFC Forum code.
    BadUriPrefix,
    /// The NDEF message contains more than one record.
    MultipleRecords,
    /// The decoded URI differs from the expected URI.
    VerificationMismatch,
    /// The decoded URI did not fit the caller's buffer.
    UriTooLong,
}

impl fmt::Display for NdefError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Url(reason) => write!(f, "invalid URL: {reason}"),
            Self::BufferTooSmall => f.write_str("the encoded message does not fit the buffer"),
            Self::InsufficientCapacity {
                required,
                available,
            } => write!(
                f,
                "URL needs {required} bytes but the tag has {available} bytes"
            ),
            Self::Truncated => f.write_str("the tag data is truncated"),
            Self::BadTlvTag => f.write_str("the NDEF message TLV is missing"),
            Self::BadTlvLength => f.write_str("the NDEF message length is invalid"),
            Self::MissingTerminator => f.write_str("the NDEF message terminator is missing"),
            Self::BadRecordHeader => {
                f.write_str("the tag does not contain one complete URI record")
            }
            Self::BadRecordType => f.write_str("the tag record is not a URI record"),
            Self::BadUriPrefix => f.write_str("the tag uses an unknown URI prefix"),
            Self::MultipleRecords => f.write_str("the tag contains more than one record"),
            Self::VerificationMismatch => f.write_str("the written URL does not match"),
            Self::UriTooLong => f.write_str("the written URL does not fit the buffer"),
        }
    }
}

/// Tag identification, validation or capacity failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagError {
    /// GET_VERSION did not report NTAG213/215/216 storage size.
    UnsupportedTag,
    /// The version data or page dump was incomplete.
    IncompleteDump,
    /// Page 3 is not a valid Type 2 Tag Capability Container.
    InvalidCapabilityContainer,
    /// The Capability Container size does not match the identified model.
    CapabilityContainerMismatch,
    /// The Capability Container does not permit free read and write.
    ReadOnly,
    /// Static or dynamic lock bits are set.
    Locked,
    /// The configuration indicates that user memory is write-protected or password-protected.
    PasswordProtected,
    /// The encoded URL does not fit the tag's NDEF memory area.
    InsufficientCapacity { required: usize, available: usize },
}

impl fmt::Display for TagError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedTag => f.write_str("unsupported tag (use NTAG213/215/216)"),
            Self::IncompleteDump => f.write_str("the tag could not be read completely"),
            Self::InvalidCapabilityContainer => {
                f.write_str("the tag has no valid Capability Container")
            }
            Self::CapabilityContainerMismatch => {
                f.write_str("the Capability Container does not match the tag model")
            }
            Self::ReadOnly => f.write_str("the tag is read-only"),
            Self::Locked => f.write_str("the tag is locked"),
            Self::PasswordProtected => f.write_str("the tag is password-protected or protected"),
            Self::InsufficientCapacity {
                required,
                available,
            } => write!(
                f,
                "URL needs {required} bytes but the tag has {available} bytes of NDEF memory"
            ),
        }
    }
}

/// Session persistence or resume failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionError {
    Corrupt,
    UnsupportedVersion,
    Invariant,
    PathTooLong,
    BufferTooSmall,
    PlaylistChanged,
    PlaylistMissing,
    Completed,
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Corrupt => f.write_str("the session file is corrupt"),
            Self::UnsupportedVersion => f.write_str("the session file uses an unknown version"),
            Self::Invariant => f.write_str("the session counters are inconsistent"),
            Self::PathTooLong => f.write_str("the playlist path is too long"),
            Self::BufferTooSmall => f.write_str("the session does not fit the buffer"),
            Self::PlaylistChanged => f.write_str("the playlist changed since the session started"),
            Self::PlaylistMissing => f.write_str("the session's playlist no longer exists"),
            Self::Completed => f.write_str("the session is already complete"),
        }
    }
}
