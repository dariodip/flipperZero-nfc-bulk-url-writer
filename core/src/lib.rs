//! Host-testable core for the NFC Bulk URL Writer.
//!
//! The crate is `no_std` and allocation-free so it can run on the Flipper Zero
//! as well as on the desktop. It contains no Flipper SDK dependencies.

#![no_std]
#![forbid(unsafe_code)]

#[cfg(test)]
extern crate std;

pub mod errors;
pub mod name;
pub mod ndef;
pub mod playlist;
pub mod session;
pub mod tag;
pub mod url;
pub mod workflow;

pub use errors::{NdefError, PlaylistError, SessionError, TagError, UrlError};

/// Largest URL accepted by the application, in bytes.
///
/// This is deliberately larger than the biggest supported NDEF area (872
/// bytes) so that an over-capacity URL is rejected with a capacity error rather
/// than a parse error.
pub const MAX_URL_LEN: usize = 1024;

/// Largest encoded Type 2 Tag TLV accepted by the application, in bytes.
pub const MAX_TLV_LEN: usize = MAX_URL_LEN + 16;
