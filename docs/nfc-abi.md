# NFC ABI notes

The FAP is compiled against `flipperzero-sys` 0.17.0, generated for API 88.2.
The tested device runs RogueMaster `rm-420` with API 88.23.

## Approach

The worker does **not** use the high-level MfUltralight read or write state
machine. It starts an `NfcPoller` with the MfUltralight protocol in extended
mode (`nfc_poller_start_ex`). When the parent ISO14443-3A poller reports
`Ready`, the worker calls the exported per-page functions directly:

- `mf_ultralight_poller_read_version`
- `mf_ultralight_poller_read_page`
- `mf_ultralight_poller_write_page`

This avoids authentication entirely. The high-level read path probes the
default password after a full read; the firmware write mode requires an
authenticated session and rewrites the whole user area. The custom session does
neither.

The compatibility module (`src/nfc/compat.rs`) only converts
`MfUltralightVersion` fields into the core's raw version bytes. No private
struct offsets, auth-context layout, or event-union assumptions are used.

## Hardware findings

### `nfc_poller_start_ex` is asynchronous

The call returns before the protocol callback runs. The worker must wait for a
completion signal and only then call `nfc_poller_stop` and
`nfc_poller_free`. The implementation uses a per-thread completion flag, with a
100 ms wait loop that checks a cancel flag. Freeing the poller immediately
caused a use-after-free and a device reset.

### `NfcCommand` return size

The C `NfcCommand` enum is 4 bytes, while the generated binding represents it as
a 1-byte struct. Returning the binding type leaves the upper 24 bits of `r0`
unspecified, turning `Stop` into an invalid command. The callback returns an
explicit 32-bit value (`0` = Continue, `2` = Stop) and the function pointer is
transmuted to the binding's callback type.

### UID layout

NTAG21x use a 7-byte UID:

- page 0: UID0, UID1, UID2, BCC0
- page 1: UID3, UID4, UID5, UID6

The earlier code included BCC0 and omitted UID6. The removal guard and
tag-change check now use the correct bytes.

### NDEF record layout

The record must start with `D1 01 <payload length> 55 <prefix> …`:
header, Type Length = 1, Payload Length, type `U`. The initial implementation
omitted Type Length, so phones saw type length `0x55` and ignored the tag. The
encoder, decoder, capacity calculations, tests and docs were corrected.

### Tag written area

The worker writes the encoded TLV followed by zeros across the tag's declared
NDEF area. This removes stale bytes from earlier messages. It never writes
pages 0-3, the dynamic-lock page, configuration pages, PWD/PACK, or user memory
outside the NDEF area.

## Hardware spike results

Tested on the NTAG213 described in `docs/validation.md`:

- GET_VERSION, model, page count, CC, UID, locks and configuration read
  correctly.
- A URL was written, read back and verified by the app.
- The raw pages decoded to the expected URI with a `0xFE` terminator.
- A phone opened the written URLs.
- A 144-byte TLV fit; a 145-byte TLV was rejected without writing.

The C adapter fallback was not needed for NTAG213. It remains a contingency for
a future firmware/ABI change.

## Hardware spike procedure

1. Build and copy the FAP.
2. Create `/ext/apps_data/nfc-bulk-url-writer/debug` for the diagnostic menu, or use
   the file-triggered auto-read mode.
3. Open `NFC Bulk URL Writer` → `NFC read diagnostics`.
4. Present each supported tag and confirm model, page count, UID, CC,
   GET_VERSION and NDEF capacity.
5. Run a one-URL batch and confirm the written URL with a phone/reader and with
   read-back.
6. Record the results in `docs/validation.md`.
