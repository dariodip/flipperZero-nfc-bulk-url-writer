# NFC Bulk URL Writer — user guide

This Flipper Zero app writes a batch of unique HTTP/HTTPS URLs to NTAG213,
NTAG215 and NTAG216 tags. Each tag receives one URL, and every write is read
back and verified before the app offers the next one.

## Before you start

1. Copy `nfc-bulk-url-writer.fap` to `/ext/apps/Examples/` on the microSD card.
2. Use the default playlist directory:
   `/ext/apps_data/nfc-bulk-url-writer/playlists/`
   or put playlists in any folder and choose it from the app with
   `Load batch from storage`. The choice is remembered.
3. Add one or more `.txt` playlists (see below).
4. Use tags that are not locked, not password-protected, and have a valid NDEF
   Capability Container. Factory-fresh NTAG tags are the intended case.
   The app never formats tags, bypasses passwords, or changes lock bits.

## Create a playlist

One URL per line:

```text
# Monday batch
https://example.test/contact/anna

http://example.test/contact/bruno
# The same URL may appear twice on purpose
https://example.test/contact/anna
```

- Blank lines and lines starting with `#` are ignored.
- Only `http://` and `https://` URLs are accepted.
- Spaces, control characters and incomplete percent escapes are rejected.
- Leading/trailing spaces are **not** trimmed, so they make the line invalid.
- Order is preserved and duplicates are programmed on separate tags.
- CRLF and a UTF-8 BOM are accepted.

The full rules are in `docs/playlists.md`.

## Program a batch

1. Open `NFC Bulk URL Writer`.
2. Choose `Resume session` if an unfinished batch is offered, or `Start new
   session`. To change folders, choose `Load batch from storage`, navigate to
   the folder, and select `Use this folder`.
3. If starting new, select a playlist from the chosen folder and confirm the URL
   count.
4. Place a tag on the Flipper. The screen shows the current index and total.
5. Wait for `Writing` / `Verifying`. The app writes only the NDEF pages and
   reads the URI back.
6. When `Success` appears, press **OK**. The tag has been verified.
7. Remove the tag. If the same tag is still detected, the app refuses to
   advance until it is removed or a different tag is presented.

The app never advances automatically after a success; OK is required. If the
tag is removed during a write, or the read-back differs, the URL stays current.

## Errors

For a tag error the app offers:

- **Retry** — try the same URL again with the same index.
- **Skip** — explicitly skip this URL and advance.
- **Abort** — save progress and stop; the session can be resumed.

Playlist, session and storage errors are setup errors. They offer `New` /
`Exit`, never a tag retry.

Common tag errors:

| Message | Meaning |
|---|---|
| unsupported tag | not an NTAG213/215/216, or GET_VERSION failed |
| Capability Container | page 3 is missing or does not match the model |
| locked | static or dynamic lock bits are set |
| password-protected | `AUTH0`/`PROT` protects the user area |
| insufficient capacity | the encoded URL does not fit the NDEF area |
| verification | the read-back URL differs from the playlist |

## Sessions

Progress is saved only after a verified write plus OK, or an explicit skip. The
session records the playlist path, a content fingerprint, the next index,
completed/skipped/failed counts, and a checksum.

On startup:

- an unfinished session is offered for resume;
- a changed playlist (different fingerprint or URL count) is not resumed;
- a missing playlist is reported;
- a corrupt or unknown-version session is not trusted;
- a completed session shows its summary instead of offering Resume;
  pressing Done clears it and returns to the startup screen.

If power is lost after a write but before the confirmation was saved, the same
URL is shown again. This is intentional: writing it twice is safe, skipping an
unverified entry is not.

## Supported tags and limits

| Tag | NDEF capacity |
|---|---:|
| NTAG213 | 144 bytes |
| NTAG215 | 496 bytes |
| NTAG216 | 872 bytes |

The capacity is the Capability Container's NDEF area, not the nominal user
memory (504/888 for NTAG215/216). URLs are compressed with the NFC Forum URI
prefix table before the capacity check.

Out of scope: other NFC families, other NDEF record types, formatting,
password management, automatic lock changes, QR/network/backend features.

## Diagnostics

If the file `/ext/apps_data/nfc-bulk-url-writer/debug` exists, the startup menu also
shows:

- `NFC read diagnostics` — reads one tag and shows model, pages, UID,
  Capability Container, GET_VERSION and NDEF capacity without writing.
- `Preview UI` — cycles through every screen with fake data.

See `docs/development.md` for building and deploying.
