//! Smoke test for the shared fixtures used by unit and hardware checks.

use nfc_bulk_url_writer_core::ndef;
use nfc_bulk_url_writer_core::playlist::{self, LineSource, SliceLineSource};
use nfc_bulk_url_writer_core::tag::{self, TagModel, TagSnapshot, VERSION_LEN};
use nfc_bulk_url_writer_core::{PlaylistError, TagError};

const PLAYLIST: &[u8] = include_bytes!("fixtures/playlist.txt");
const PLAYLIST_CRLF_BOM: &[u8] = include_bytes!("fixtures/playlist_crlf_bom.txt");
const PLAYLIST_INVALID: &[u8] = include_bytes!("fixtures/playlist_invalid.txt");
const PLAYLIST_EMPTY: &[u8] = include_bytes!("fixtures/playlist_empty.txt");
const NDEF_URI: &[u8] = include_bytes!("fixtures/ndef_uri.txt");
const NDEF_URI_TLV: &str = include_str!("fixtures/ndef_uri_tlv.hex");
const NTAG213_HEX: &str = include_str!("fixtures/ntag213.hex");
const NTAG215_HEX: &str = include_str!("fixtures/ntag215.hex");
const NTAG216_HEX: &str = include_str!("fixtures/ntag216.hex");

fn parse_hex(text: &str) -> std::vec::Vec<u8> {
    text.split_whitespace()
        .map(|byte| u8::from_str_radix(byte, 16).expect("hex byte"))
        .collect()
}

fn load_snapshot(text: &str, model: TagModel) -> TagSnapshot {
    let mut snapshot = TagSnapshot::new();
    let mut count = 0u16;
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let (page, bytes) = line.split_once(':').expect("page: bytes");
        let page: u16 = page.trim().parse().expect("page number");
        let bytes = parse_hex(bytes);
        assert_eq!(bytes.len(), 4);
        assert!(snapshot.set_page(page, [bytes[0], bytes[1], bytes[2], bytes[3]]));
        count += 1;
    }
    assert_eq!(count, model.page_count());

    let mut version = [0u8; VERSION_LEN];
    version[0] = 0x00;
    version[1] = 0x04;
    version[2] = 0x04;
    version[3] = 0x02;
    version[4] = 0x01;
    version[5] = 0x00;
    version[6] = match model {
        TagModel::Ntag213 => 0x0F,
        TagModel::Ntag215 => 0x11,
        TagModel::Ntag216 => 0x13,
    };
    version[7] = 0x03;
    snapshot.set_version(version);
    snapshot.set_pages_read(model.page_count());
    snapshot
}

fn scan(text: &[u8]) -> Result<playlist::PlaylistStats, PlaylistError> {
    let mut source = SliceLineSource::new(text);
    let mut line = [0u8; playlist::LINE_BUFFER_SIZE];
    playlist::scan(&mut source, &mut line)
}

fn entry(text: &[u8], index: u32) -> std::vec::Vec<u8> {
    let mut source = SliceLineSource::new(text);
    let mut line = [0u8; playlist::LINE_BUFFER_SIZE];
    let mut out = [0u8; 1024];
    let length = playlist::entry_at(&mut source, index, &mut line, &mut out).unwrap();
    out[..length].to_vec()
}

#[test]
fn playlist_fixture_parses_in_order_with_duplicates() {
    let stats = scan(PLAYLIST).unwrap();
    assert_eq!(stats.total, 3);
    assert_eq!(entry(PLAYLIST, 0), b"https://example.test/contact/1");
    assert_eq!(entry(PLAYLIST, 1), b"http://example.test/contact/2");
    assert_eq!(entry(PLAYLIST, 2), b"https://example.test/contact/1");
    assert_eq!(
        stats.fingerprint,
        playlist::fingerprint_urls([
            &b"https://example.test/contact/1"[..],
            b"http://example.test/contact/2",
            b"https://example.test/contact/1",
        ])
    );
}

#[test]
fn crlf_bom_fixture_has_the_same_semantic_fingerprint() {
    let stats = scan(PLAYLIST_CRLF_BOM).unwrap();
    assert_eq!(stats.total, 2);
    assert_eq!(
        entry(PLAYLIST_CRLF_BOM, 0),
        b"https://example.test/contact/1"
    );
    assert_eq!(
        stats.fingerprint,
        playlist::fingerprint_urls([
            &b"https://example.test/contact/1"[..],
            b"https://example.test/contact/2",
        ])
    );
}

#[test]
fn invalid_and_empty_fixtures_are_rejected() {
    assert_eq!(
        scan(PLAYLIST_INVALID),
        Err(PlaylistError::InvalidUrl {
            line: 3,
            reason: nfc_bulk_url_writer_core::UrlError::UnsupportedScheme,
        })
    );
    assert_eq!(scan(PLAYLIST_EMPTY), Err(PlaylistError::Empty));
}

#[test]
fn ndef_fixture_round_trips() {
    let uri = NDEF_URI.strip_suffix(b"\n").unwrap();
    let tlv = parse_hex(NDEF_URI_TLV);
    assert_eq!(tlv.len(), 30);
    let mut out = [0u8; 1024];
    let length = ndef::decode_into(&tlv, &mut out).unwrap();
    assert_eq!(&out[..length], uri);
    assert_eq!(ndef::encoded_size(uri).unwrap(), tlv.len());
}

#[test]
fn ntag_fixtures_validate() {
    for (hex, model, ndef_memory) in [
        (NTAG213_HEX, TagModel::Ntag213, 144),
        (NTAG215_HEX, TagModel::Ntag215, 496),
        (NTAG216_HEX, TagModel::Ntag216, 872),
    ] {
        let snapshot = load_snapshot(hex, model);
        let info = tag::validate(&snapshot, 100).expect("factory snapshot");
        assert_eq!(info.model, model);
        assert_eq!(info.ndef_memory, ndef_memory);
        assert_eq!(snapshot.model(), Some(model));
        assert_eq!(
            tag::validate(&snapshot, ndef_memory + 1),
            Err(TagError::InsufficientCapacity {
                required: ndef_memory + 1,
                available: ndef_memory,
            })
        );
    }
}

#[test]
fn line_source_reports_end_of_file() {
    let mut source = SliceLineSource::new(b"a\nb");
    let mut buffer = [0u8; 8];
    assert_eq!(source.next_line(&mut buffer).unwrap(), Some(1));
    assert_eq!(source.next_line(&mut buffer).unwrap(), Some(1));
    assert_eq!(source.next_line(&mut buffer).unwrap(), None);
}
