//! NFC Forum Well-Known URI record and Type 2 Tag TLV encoding.
//!
//! The encoder produces exactly one URI record inside one NDEF Message TLV,
//! followed by the mandatory `0xFE` terminator. The decoder is strict: it
//! accepts only that shape and reproduces the original URL byte-for-byte.

use crate::MAX_TLV_LEN;
use crate::errors::NdefError;
use crate::url;

/// NFC Forum URI Identifier Codes, indexed by code value.
const URI_PREFIXES: [&[u8]; 0x24] = [
    b"",
    b"http://www.",
    b"https://www.",
    b"http://",
    b"https://",
    b"tel:",
    b"mailto:",
    b"ftp://anonymous:anonymous@",
    b"ftp://ftp.",
    b"ftps://",
    b"sftp://",
    b"smb://",
    b"nfs://",
    b"ftp://",
    b"dav://",
    b"news:",
    b"telnet://",
    b"imap:",
    b"rtsp://",
    b"urn:",
    b"pop:",
    b"sip:",
    b"sips:",
    b"tftp:",
    b"btspp://",
    b"btl2cap://",
    b"btgoep://",
    b"tcpobex://",
    b"irdaobex://",
    b"file://",
    b"urn:epc:id:",
    b"urn:epc:tag:",
    b"urn:epc:pat:",
    b"urn:epc:raw:",
    b"urn:epc:",
    b"urn:nfc:",
];

/// NDEF record type byte for a URI record.
const RECORD_TYPE_URI: u8 = 0x55;
/// Message Begin flag.
const FLAG_MB: u8 = 0x80;
/// Message End flag.
const FLAG_ME: u8 = 0x40;
/// Short Record flag.
const FLAG_SR: u8 = 0x10;
/// Type Name Format: NFC Forum well-known type.
const TNF_WELL_KNOWN: u8 = 0x01;
/// Type 2 Tag NDEF Message TLV.
const TLV_NDEF_MESSAGE: u8 = 0x03;
/// Type 2 Tag Terminator TLV.
const TLV_TERMINATOR: u8 = 0xFE;

/// Metadata returned by a successful encode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Encoded {
    /// Total TLV length, including the terminator.
    pub tlv_len: usize,
    /// NDEF message length inside the TLV.
    pub message_len: usize,
    /// URI Identifier Code that was used.
    pub prefix_code: u8,
}

/// Selects the longest matching URI prefix from the NFC Forum table.
///
/// Matching is case-sensitive; an uppercase scheme does not match and uses
/// code `0x00` with the complete URI.
pub fn select_prefix(uri: &[u8]) -> (u8, usize) {
    let mut best_code = 0u8;
    let mut best_len = 0usize;
    for (code, prefix) in URI_PREFIXES.iter().enumerate().skip(1) {
        if prefix.len() > best_len && uri.starts_with(prefix) {
            best_code = code as u8;
            best_len = prefix.len();
        }
    }
    (best_code, best_len)
}

/// Calculates the exact TLV size for a URL.
pub fn encoded_size(uri: &[u8]) -> Result<usize, NdefError> {
    url::validate(uri).map_err(NdefError::Url)?;
    Ok(size_for_valid_uri(uri))
}

fn size_for_valid_uri(uri: &[u8]) -> usize {
    let (_, prefix_len) = select_prefix(uri);
    let payload_len = 1 + (uri.len() - prefix_len);
    tlv_size(record_length(payload_len))
}

fn length_field_size(payload_len: usize) -> usize {
    if payload_len <= 255 { 1 } else { 4 }
}

fn record_length(payload_len: usize) -> usize {
    // header + type length + payload length + type 'U' + payload
    1 + 1 + length_field_size(payload_len) + 1 + payload_len
}

fn tlv_size(message_len: usize) -> usize {
    let tlv_length_field = if message_len < 255 { 1 } else { 3 };
    1 + tlv_length_field + message_len + 1
}

/// Returns the message length for a URL. Exposed for exact-size tests.
pub fn message_target_size(uri: &[u8]) -> Result<usize, NdefError> {
    url::validate(uri).map_err(NdefError::Url)?;
    Ok(message_length_for_valid_uri(uri))
}

fn message_length_for_valid_uri(uri: &[u8]) -> usize {
    let (_, prefix_len) = select_prefix(uri);
    let payload_len = 1 + (uri.len() - prefix_len);
    record_length(payload_len)
}

/// Encodes a URL into a Type 2 Tag TLV.
///
/// Returns the encoded metadata when it fits `out`. The caller must still
/// compare [`Encoded::tlv_len`] with the tag's NDEF memory size, or use
/// [`fits`] before writing.
pub fn encode(uri: &[u8], out: &mut [u8]) -> Result<Encoded, NdefError> {
    url::validate(uri).map_err(NdefError::Url)?;
    let (prefix_code, prefix_len) = select_prefix(uri);
    let remainder = &uri[prefix_len..];
    let payload_len = 1 + remainder.len();
    let message_len = record_length(payload_len);
    let tlv_len = tlv_size(message_len);

    if out.len() < tlv_len {
        return Err(NdefError::BufferTooSmall);
    }

    let mut offset = 0;
    out[offset] = TLV_NDEF_MESSAGE;
    offset += 1;
    if message_len < 255 {
        out[offset] = message_len as u8;
        offset += 1;
    } else {
        out[offset] = 0xFF;
        out[offset + 1] = (message_len >> 8) as u8;
        out[offset + 2] = (message_len & 0xFF) as u8;
        offset += 3;
    }

    let short_record = payload_len <= 255;
    out[offset] = FLAG_MB | FLAG_ME | TNF_WELL_KNOWN | if short_record { FLAG_SR } else { 0 };
    offset += 1;
    out[offset] = 1; // Type Length
    offset += 1;
    if short_record {
        out[offset] = payload_len as u8;
        offset += 1;
    } else {
        let len = payload_len as u32;
        out[offset..offset + 4].copy_from_slice(&len.to_be_bytes());
        offset += 4;
    }
    out[offset] = RECORD_TYPE_URI;
    offset += 1;
    out[offset] = prefix_code;
    offset += 1;
    out[offset..offset + remainder.len()].copy_from_slice(remainder);
    offset += remainder.len();
    out[offset] = TLV_TERMINATOR;
    offset += 1;
    debug_assert_eq!(offset, tlv_len);

    Ok(Encoded {
        tlv_len,
        message_len,
        prefix_code,
    })
}

/// Checks that a URL fits an NDEF memory area and returns its exact TLV size.
pub fn fits(uri: &[u8], ndef_memory: usize) -> Result<usize, NdefError> {
    let size = encoded_size(uri)?;
    if size > ndef_memory {
        return Err(NdefError::InsufficientCapacity {
            required: size,
            available: ndef_memory,
        });
    }
    Ok(size)
}

/// Decodes a Type 2 Tag TLV into `out` and returns the URI length.
///
/// The decoder requires exactly one complete Well-Known URI record followed by
/// a `0xFE` terminator. Leading NULL TLVs (`0x00`) are tolerated.
pub fn decode_into(data: &[u8], out: &mut [u8]) -> Result<usize, NdefError> {
    let mut offset = 0;
    while offset < data.len() && data[offset] == 0x00 {
        offset += 1;
    }
    if offset >= data.len() {
        return Err(NdefError::Truncated);
    }
    if data[offset] != TLV_NDEF_MESSAGE {
        return Err(NdefError::BadTlvTag);
    }
    offset += 1;

    if offset >= data.len() {
        return Err(NdefError::Truncated);
    }
    let message_len = if data[offset] < 0xFF {
        let length = data[offset] as usize;
        offset += 1;
        length
    } else {
        if offset + 3 > data.len() {
            return Err(NdefError::BadTlvLength);
        }
        let length = ((data[offset + 1] as usize) << 8) | data[offset + 2] as usize;
        offset += 3;
        length
    };

    let message_end = offset
        .checked_add(message_len)
        .ok_or(NdefError::Truncated)?;
    if message_end > data.len() {
        return Err(NdefError::Truncated);
    }
    let message = &data[offset..message_end];
    if message_end >= data.len() || data[message_end] != TLV_TERMINATOR {
        return Err(NdefError::MissingTerminator);
    }

    let uri_len = decode_message(message, out)?;
    Ok(uri_len)
}

fn decode_message(message: &[u8], out: &mut [u8]) -> Result<usize, NdefError> {
    if message.len() < 4 {
        return Err(NdefError::BadRecordHeader);
    }
    let header = message[0];
    let mb = header & FLAG_MB != 0;
    let me = header & FLAG_ME != 0;
    let short = header & FLAG_SR != 0;
    let has_id = header & 0x08 != 0;
    let tnf = header & 0x07;

    if !mb || !me {
        return Err(NdefError::MultipleRecords);
    }
    if has_id || tnf != TNF_WELL_KNOWN {
        return Err(NdefError::BadRecordType);
    }
    // Type Length must be 1 (the 'U' type).
    if message[1] != 1 {
        return Err(NdefError::BadRecordType);
    }

    let mut offset = 2;
    let payload_len = if short {
        if offset >= message.len() {
            return Err(NdefError::Truncated);
        }
        let length = message[offset] as usize;
        offset += 1;
        length
    } else {
        if offset + 4 > message.len() {
            return Err(NdefError::Truncated);
        }
        let length = u32::from_be_bytes([
            message[offset],
            message[offset + 1],
            message[offset + 2],
            message[offset + 3],
        ]) as usize;
        offset += 4;
        length
    };

    if offset >= message.len() || message[offset] != RECORD_TYPE_URI {
        return Err(NdefError::BadRecordType);
    }
    offset += 1;

    if payload_len == 0 || offset + payload_len > message.len() {
        return Err(NdefError::BadRecordHeader);
    }
    if offset + payload_len != message.len() {
        return Err(NdefError::MultipleRecords);
    }

    let prefix_code = message[offset];
    if prefix_code as usize >= URI_PREFIXES.len() {
        return Err(NdefError::BadUriPrefix);
    }
    let prefix = URI_PREFIXES[prefix_code as usize];
    let remainder = &message[offset + 1..offset + payload_len];
    let total = prefix.len() + remainder.len();
    if total > out.len() {
        return Err(NdefError::UriTooLong);
    }
    out[..prefix.len()].copy_from_slice(prefix);
    out[prefix.len()..total].copy_from_slice(remainder);
    Ok(total)
}

/// Decodes and compares a written TLV with the expected URL.
pub fn verify(data: &[u8], expected: &[u8], scratch: &mut [u8]) -> Result<usize, NdefError> {
    url::validate(expected).map_err(NdefError::Url)?;
    let length = decode_into(data, scratch)?;
    if &scratch[..length] != expected {
        return Err(NdefError::VerificationMismatch);
    }
    Ok(length)
}

/// Reconstructs a URI from a URI Identifier Code and remainder.
pub fn build_uri(prefix_code: u8, remainder: &[u8], out: &mut [u8]) -> Result<usize, NdefError> {
    let prefix = URI_PREFIXES
        .get(prefix_code as usize)
        .ok_or(NdefError::BadUriPrefix)?;
    let total = prefix.len() + remainder.len();
    if total > out.len() {
        return Err(NdefError::UriTooLong);
    }
    out[..prefix.len()].copy_from_slice(prefix);
    out[prefix.len()..total].copy_from_slice(remainder);
    Ok(total)
}

/// Returns the static prefix for a URI Identifier Code.
pub fn prefix_for_code(code: u8) -> Option<&'static [u8]> {
    URI_PREFIXES.get(code as usize).copied()
}

/// Maximum TLV produced for [`crate::MAX_URL_LEN`] input.
pub const ENCODED_BUFFER_SIZE: usize = MAX_TLV_LEN;

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_ok(uri: &[u8]) -> (std::vec::Vec<u8>, Encoded) {
        let mut out = [0u8; ENCODED_BUFFER_SIZE];
        let encoded = encode(uri, &mut out).unwrap();
        (out[..encoded.tlv_len].to_vec(), encoded)
    }

    fn decode(uri: &[u8]) -> Result<std::vec::Vec<u8>, NdefError> {
        let (tlv, _) = encode_ok(uri);
        let mut out = [0u8; 2048];
        let length = decode_into(&tlv, &mut out)?;
        Ok(out[..length].to_vec())
    }

    #[test]
    fn prefix_selection_prefers_longest_match() {
        assert_eq!(select_prefix(b"https://www.example.test/x"), (0x02, 12));
        assert_eq!(select_prefix(b"http://www.example.test/x"), (0x01, 11));
        assert_eq!(select_prefix(b"https://example.test/x"), (0x04, 8));
        assert_eq!(select_prefix(b"http://example.test/x"), (0x03, 7));
        assert_eq!(select_prefix(b"HTTPS://example.test/x"), (0x00, 0));
    }

    #[test]
    fn record_layout_is_exact() {
        let (tlv, encoded) = encode_ok(b"https://example.test/x");
        assert_eq!(encoded.prefix_code, 0x04);
        assert_eq!(tlv[0], 0x03);
        assert_eq!(tlv[1], encoded.message_len as u8);
        assert_eq!(tlv[2], 0xD1); // MB | ME | SR | TNF well-known
        assert_eq!(tlv[3], 0x01); // Type Length
        assert_eq!(tlv[4], (encoded.message_len - 4) as u8); // payload len
        assert_eq!(tlv[5], 0x55); // type 'U'
        assert_eq!(tlv[6], 0x04);
        assert_eq!(&tlv[7..tlv.len() - 1], b"example.test/x");
        assert_eq!(*tlv.last().unwrap(), 0xFE);
    }

    #[test]
    fn long_record_boundary_at_payload_255_and_256() {
        // https:// prefix is 8 bytes; payload = 1 + remainder.
        let mut uri_255 = std::vec![b'a'; 8 + 254];
        uri_255[..8].copy_from_slice(b"https://");
        let (tlv, encoded) = encode_ok(&uri_255);
        assert_eq!(encoded.message_len, 255 + 4);
        assert_eq!(tlv[4] & FLAG_SR, FLAG_SR);
        assert_eq!(decode(&uri_255).unwrap(), uri_255);

        let mut uri_256 = std::vec![b'a'; 8 + 255];
        uri_256[..8].copy_from_slice(b"https://");
        let (tlv, encoded) = encode_ok(&uri_256);
        assert_eq!(encoded.message_len, 256 + 7);
        assert_eq!(tlv[4] & FLAG_SR, 0);
        assert_eq!(decode(&uri_256).unwrap(), uri_256);
    }

    #[test]
    fn tlv_length_boundary_at_message_254_255_256() {
        // URL length = message length + 3 for a short record (https:// prefix).
        for message_len in [254usize, 255, 256] {
            let uri_len = message_len + 3;
            let mut uri = std::vec![b'a'; uri_len];
            uri[..8].copy_from_slice(b"https://");
            let (tlv, encoded) = encode_ok(&uri);
            assert_eq!(encoded.message_len, message_len);
            if message_len < 255 {
                assert_eq!(tlv[1], message_len as u8);
            } else {
                assert_eq!(tlv[1], 0xFF);
                assert_eq!(tlv[2] as usize, message_len >> 8);
                assert_eq!(tlv[3] as usize, message_len & 0xFF);
            }
            assert_eq!(decode(&uri).unwrap(), uri);
        }
    }

    #[test]
    fn strict_decoder_rejects_bad_shapes() {
        let uri = b"https://example.test/x";
        let (tlv, encoded) = encode_ok(uri);
        let mut out = [0u8; 2048];

        let mut truncated = tlv.clone();
        truncated.truncate(tlv.len() - 2);
        assert!(decode_into(&truncated, &mut out).is_err());

        let mut missing_terminator = tlv.clone();
        *missing_terminator.last_mut().unwrap() = 0x00;
        assert_eq!(
            decode_into(&missing_terminator, &mut out),
            Err(NdefError::MissingTerminator)
        );

        let mut bad_type = tlv.clone();
        bad_type[5] = 0x54; // 'T'
        assert_eq!(
            decode_into(&bad_type, &mut out),
            Err(NdefError::BadRecordType)
        );

        let mut bad_header = tlv.clone();
        bad_header[2] &= !FLAG_ME;
        assert_eq!(
            decode_into(&bad_header, &mut out),
            Err(NdefError::MultipleRecords)
        );

        // Extend the message by two bytes just before the terminator: the
        // decoder sees a complete record followed by another record header.
        let mut two_records = tlv.clone();
        let message_len = encoded.message_len;
        assert!(message_len + 2 < 255);
        two_records[1] = (message_len + 2) as u8;
        let terminator_index = 2 + message_len;
        two_records.insert(terminator_index, 0xD1);
        two_records.insert(terminator_index + 1, 0x55);
        assert!(decode_into(&two_records, &mut out).is_err());

        let mut bad_prefix = tlv.clone();
        bad_prefix[6] = 0x40;
        assert_eq!(
            decode_into(&bad_prefix, &mut out),
            Err(NdefError::BadUriPrefix)
        );
    }

    #[test]
    fn verification_is_byte_exact() {
        let uri = b"https://example.test/a?x=1&y=2#section";
        let (tlv, _) = encode_ok(uri);
        let mut scratch = [0u8; 2048];
        assert_eq!(verify(&tlv, uri, &mut scratch), Ok(uri.len()));
        assert_eq!(
            verify(&tlv, b"https://example.test/a?x=1&y=2", &mut scratch),
            Err(NdefError::VerificationMismatch)
        );
    }

    #[test]
    fn round_trips_preserve_every_byte() {
        for uri in [
            &b"https://example.test/a?x=1&y=2#section"[..],
            b"https://example.test/Contact/ABC",
            b"https://example.test/trailing/",
            b"https://www.example.test/contact/1",
            b"HTTPS://example.test/contact/1",
            b"http://example.test/%20space",
        ] {
            assert_eq!(decode(uri).unwrap(), uri);
        }
    }

    #[test]
    fn capacity_boundaries_use_exact_tlv_size() {
        // Prefix https:// is 8 bytes.
        // TLV = 8 + remainder for short messages; 13 + remainder for long ones.
        let cases = [(144usize, 144usize), (496, 491), (872, 867)];
        for (capacity, max_url) in cases {
            let mut uri = std::vec![b'a'; max_url];
            uri[..8].copy_from_slice(b"https://");
            assert_eq!(encoded_size(&uri).unwrap(), capacity);
            assert_eq!(fits(&uri, capacity), Ok(capacity));

            let mut too_long = std::vec![b'a'; max_url + 1];
            too_long[..8].copy_from_slice(b"https://");
            assert_eq!(encoded_size(&too_long).unwrap(), capacity + 1);
            assert_eq!(
                fits(&too_long, capacity),
                Err(NdefError::InsufficientCapacity {
                    required: capacity + 1,
                    available: capacity,
                })
            );
        }
    }

    #[test]
    fn invalid_url_is_rejected() {
        assert_eq!(
            encoded_size(b"ftp://example.test/"),
            Err(NdefError::Url(crate::errors::UrlError::UnsupportedScheme))
        );
    }
}
