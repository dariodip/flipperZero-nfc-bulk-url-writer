//! Small conversions for the target firmware's MfUltralight data structures.

use flipperzero_sys as sys;
use nfc_bulk_url_writer_core::tag::VERSION_LEN;

/// Converts the firmware's GET_VERSION structure to raw bytes.
pub fn version_to_bytes(version: &sys::MfUltralightVersion) -> [u8; VERSION_LEN] {
    [
        version.header,
        version.vendor_id,
        version.prod_type,
        version.prod_subtype,
        version.prod_ver_major,
        version.prod_ver_minor,
        version.storage_size,
        version.protocol_type,
    ]
}
