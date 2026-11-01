# NFC worker

All RF access happens on one dedicated Furi thread. The GUI thread never calls
an NFC function except `NfcWorker::cancel`, which stops the worker's current
poller.

## Threading

- The worker owns the `Nfc` handle and every poller.
- Commands go to the worker through a raw Furi message queue (2 slots).
- Results go back through a result queue (4 slots) plus a
  `view_dispatcher_send_custom_event` notification.
- The poller callback never touches views or the workflow. It only fills the
  current operation's result.
- `NfcWorker::stop` cancels the current poller, sends `Stop`, joins the thread
  and frees the `Nfc` handle. The app calls it before leaving the view
  dispatcher.

## Operations

### Read and validate

1. Poll `nfc_poller_detect` every 100 ms, checking the command queue for
   `Stop`.
2. Start an extended MfUltralight poller (`nfc_poller_start_ex`). The parent
   ISO14443-3A poller activates the tag; on its `Ready` event the worker calls
   `mf_ultralight_poller_read_version` and `mf_ultralight_poller_read_page`
   directly.
3. Read the UID, static locks, Capability Container and configuration pages
   into a core `TagSnapshot`. No authentication command is used.
4. Run `tag::validate_metadata` with the exact URL TLV size.
5. Emit `Validated` with model, NDEF capacity and UID, or `Rejected` with the
   core `TagError`.

The diagnostics path also reads the user area so the whole dump can be
inspected; the normal path reads only what validation and writing need.

The high-level MfUltralight read state machine is deliberately not used: after
a full read it probes the default password, which could consume `AUTHLIM`
attempts and violates the no-password rule.

### Write and verify

1. Encode the URL into an exact Type 2 Tag TLV before touching the tag.
2. Start a custom MfUltralight session as above and read the metadata. The
   validated model from the preceding read phase is passed with the request;
   when it is known, the session skips the GET_VERSION exchange and reproduces
   the model identity from the validated model, while fully re-reading the
   identity, lock and configuration pages. Compare the UID with the UID that
   was just validated. A mismatch aborts as `TagChanged`.
3. Validate the tag again (model, CC, locks, protection, capacity).
4. Read the pages covering the TLV so the last partial page keeps its existing
   bytes, then write only those pages, starting at page 4. No metadata page,
   lock page, password page or configuration page is written.
5. Read the written pages back in 4-page chunks and decode them with the strict
   decoder. The decoded URL must equal the playlist entry byte-for-byte.
6. Emit `Verified`, `WriteFailed`, `ReadbackFailed` or `VerificationFailed`.

The firmware's write mode is not used because it requires an authenticated
session and rewrites the whole user area. The high-level read mode is not used
because of its default-password probe.

### Removal guard

After a confirmed success the worker runs a one-shot UID read:

- if no tag is present, emit `TagCleared`;
- if a tag is present, read pages 0-1 and compare the UID with the programmed
  UID;
- the same UID emits `SameTagPresent` once and keeps polling;
- a different UID emits `TagCleared`.

A tag that is present but unreadable is treated as the same tag until the user
aborts, rather than risking reuse of the programmed tag.

This is a physical-tag guard, not an external-field detector. The app does not
use `furi_hal_nfc_field_is_present`.

## Timeouts and cancellation

The NFC HAL provides frame wait times for each command; a missing or moved tag
makes activation or a page operation fail promptly. In addition:

- `nfc_poller_detect` is polled with a 100 ms sleep and checks for `Stop`
  between attempts;
- `NfcWorker::cancel` takes the poller mutex and calls `nfc_poller_stop` on the
  active poller, interrupting a blocked start;
- the app arms a one-shot 30 s timer before a write/verify operation and
  disarms it when the result arrives. If it fires, it calls
  `NfcWorker::cancel`, so a stuck write cannot block the UI forever;
- commands time out after 100 ms if the queue is full, which is reported as a
  failed send by the caller;
- the UI's Back during waiting/removal only opens abort confirmation, so an
  in-flight operation is never abandoned silently.

## Error mapping

| Worker result | Core action |
|---|---|
| `Rejected(TagError)` | tag error: Retry/Skip/Abort |
| `NdefRejected(NdefError)` | tag/encoding error: Retry/Skip/Abort |
| `TagChanged` | tag error: Retry/Skip/Abort |
| `WriteFailed` | tag error: Retry/Skip/Abort |
| `ReadbackFailed` | tag error: Retry/Skip/Abort |
| `VerificationFailed` | tag error: Retry/Skip/Abort |
| `Verified` | success screen after verification |
| `SameTagPresent` | removal screen, no advance |
| `TagCleared` | next URL or summary |
| `InternalError` | tag error |

No worker result advances the session by itself. The workflow does that only on
explicit confirmation or skip.
