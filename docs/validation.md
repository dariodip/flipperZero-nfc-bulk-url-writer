# Validation

## Host checks

| Check | Command | Result |
|---|---|---|
| Core unit tests | `cargo test -p nfc-bulk-url-writer-core --target aarch64-apple-darwin` | 68 passed |
| Fixture smoke tests | same command, `tests/fixtures.rs` | 6 passed |
| Debug FAP build | `cargo build` | `target/thumbv7em-none-eabihf/debug/nfc-bulk-url-writer.fap` |
| Release FAP build | `cargo build --release` | `target/thumbv7em-none-eabihf/release/nfc-bulk-url-writer.fap` |
| Lints | `cargo clippy` | no warnings |
| OpenSpec validation | `openspec validate add-nfc-bulk-url-writer --strict` | valid |

## Hardware environment

| Item | Value |
|---|---|
| Firmware | RogueMaster `rm-420`, build 24-10-2026 |
| API version | 88.23 |
| Hardware target | 7 (Flipper Zero) |
| Hardware UID | `FCD4970127E18000` |
| Test tag | NTAG213, 7-byte UID `53 A8 C6 C2 64 00 01`, vendor Shanghai Feiju |

Only NTAG213 tags were available. NTAG215 and NTAG216 are covered by unit
tests but were **not** tested on hardware.

## NTAG213 results (pass)

### Read diagnostics

- Model: NTAG213, 45 pages.
- UID: `53 A8 C6 C2 64 00 01` (matches the built-in NFC CLI).
- Capability Container: `E1 10 12 00`.
- GET_VERSION storage byte: `0F`.
- NDEF memory: 144 bytes.
- Static locks zero; dynamic lock user bytes zero; `AUTH0 = FF`, `PROT = 0`.

The diagnostics also read the user area and show lock/configuration pages.

### Write and read-back

- `https://example.test/contact/auto1` written, read back, and independently
  decoded from raw pages.
- `https://www.wikipedia.org/` and `https://www.google.com/` written through
  the normal UI workflow.
- Phone scan confirmed both real URLs open correctly.
- Raw record layout after the fix:
  `03 12 D1 01 0F 55 02 77 69 6B 69 70 65 64 69 61 2E 6F 72 67 2F FE`
  (`D1` header, `01` Type Length, payload length, `55` type, `02` prefix,
  remainder, terminator).
- The NDEF area is filled with the new TLV followed by zeros; no stale bytes
  from older messages remain. Metadata and user memory outside the NDEF area
  are not written.

### UI workflow

- 3-URL playlist: 3 completed, 0 skipped, 0 failed.
- 2-URL playlist: 2 completed, 0 skipped, 0 failed.
- Success screen appears only after read-back verification.
- OK advances; Back does not.
- Same tag is refused until removed; the removal screen works.
- “Waiting for next tag” stays visible after removal (no blank screen).
- Summary Done returns to the startup screen; no crash or loader error. A
  completed session is cleared, so the next start is clean.

### Capacity

- 144-byte URL (144-byte TLV) writes and verifies on the NTAG213. The raw
  layout is `03 8D D1 01 89 55 04 … FE`; the terminator is the last byte of
  page 39.
- 145-byte URL is rejected with
  `URL needs 145 bytes but the tag has 144 bytes of NDEF memory`.
- The tag is unchanged after the rejection.

### Custom playlist folder

- Chose `/ext/apps_data/nfc-bulk-url-writer/playlists/custom` with the folder
  picker; the setting was written to `settings.txt`.
- The folder picker supports `Use this folder`, `..` and subdirectory
  navigation.

### Long Back exit

- Holding Back shows `Exit app?`; Yes closes the application and No returns to
  the current screen. Short Back keeps its normal per-screen behavior.

### Recovery

- Abort at URL 2 saves `next=1, completed=1, complete=0`.
- Relaunch offers Resume; Resume starts at URL 2 without asking for the
  playlist again.
- Changing the playlist after an incomplete session produces a playlist-changed
  setup error; it is not resumed.
- A corrupt session checksum produces a session error; it is not resumed.

## Not tested / deferred

- NTAG215 and NTAG216 hardware (none available). Their 496/872-byte NDEF
  capacities are unit-tested.
- Locked, read-only, and password-protected tags.
- Unsupported non-NTAG tags.
- Tag removed during a write.
- A different physical tag for the removal guard (same-tag refusal was tested).
- Power loss after a write but before the success confirmation.
- Formal long-playlist soak/leak test.

These limitations do not indicate a known failure; they simply were not
exercised with the available hardware.

## Reproducing the hardware checks

The headless harness is enabled by files on the microSD card:

- **Read-only diagnostics:** create
  `/ext/apps_data/nfc-bulk-url-writer/auto-read`, launch the FAP, and present a tag.
  The report is appended to `/ext/apps_data/nfc-bulk-url-writer/trace.log`.
- **Write/read-back:** create `/ext/apps_data/nfc-bulk-url-writer/auto-write` and put
  one URL in `/ext/apps_data/nfc-bulk-url-writer/auto-url.txt`. The app reads the tag,
  writes, verifies, and appends the result to the trace.
- Remove the flag files after testing. They are ignored by the normal UI.

The debug menu (with `/ext/apps_data/nfc-bulk-url-writer/debug`) also exposes the
on-screen read diagnostics and UI preview.

## Bugs found during hardware validation (fixed)

1. **Missing NDEF Type Length byte.** The original record was
   `D1 55 <payload len> …`; phones interpreted `0x55` as the type length and
   ignored the tag. The correct layout is `D1 01 <payload len> 55 …`.
   Encoder, decoder, capacity math, tests and docs were corrected.
2. **Async poller use-after-free.** `nfc_poller_start_ex` is asynchronous; the
   poller was freed before the callback completed. A thread-flag completion
   handshake and a cancel flag were added.
3. **NfcCommand callback ABI.** The C enum is 4 bytes; the binding returns a
   1-byte struct. The callback now returns an explicit 32-bit command.
4. **Popup text lifetime.** The Popup stores the header/text pointer without
   copying; the buffers now live in the UI object, fixing the blank screen.
5. **UID layout.** The 7-byte NTAG UID is page 0 UID0-2 + BCC0, page 1
   UID3-6; the previous construction included BCC0 and omitted UID6.
6. **Stale NDEF bytes.** Older messages after the terminator could confuse
   readers; the writer now fills the NDEF area with the new TLV plus zeros.
