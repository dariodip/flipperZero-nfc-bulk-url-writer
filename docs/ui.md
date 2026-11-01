# User interface

The app uses native Flipper components through a `ViewDispatcher`:

- `Submenu` for the startup choice and playlist list;
- `Popup` while waiting, writing or verifying;
- `DialogEx` for playlist confirmation, success, removal, errors and the
  summary.

No scene manager is used because the flow is linear.

## Screens

### Startup (`Submenu`)

Header: `NFC Bulk URL Writer`.

| Item | When | Action |
|---|---|---|
| `Resume session` | a valid, incomplete session exists | resumes at the saved index |
| `Start new session` | always | clears the old session when a new playlist is confirmed, then opens the playlist list for the remembered folder |
| `Load batch from storage` | always | opens the folder picker and remembers the chosen folder |
| `NFC read diagnostics` | debug flag exists | read-only tag dump |
| `Preview UI` | debug flag exists | cycles through every screen with fake data |

Back exits without starting NFC.

### Playlist folder picker (`Submenu`)

Header: `Folder: <name>`. Items: `Use this folder`, `..` when a parent exists,
and the subdirectories of the current folder. Selecting a subdirectory enters
it; `Use this folder` saves the folder and opens its playlist list. Back returns
to the startup screen without changing the saved folder.

### Playlist selection (`Submenu`)

Header: `Select playlist`. Lists `.txt` display names from the chosen folder in
deterministic order.
If no playlists or no directory exist, a dialog shows the expected path and an
`Exit` button.

Selecting a playlist opens a confirmation dialog with the playlist name and URL
count. `Back` returns to the list; `Start` begins the batch.

### Waiting for a tag (`Popup`)

Header `Tag <index>/<total>`, `Waiting for next tag` (or `Waiting for tag` for
the first entry), `Place tag near Flipper`, and a scrollable full current URL.
Use Up/Down to scroll long URLs; the URL wraps inside the scroll area. The
index and total stay visible. The URL view is a `Widget`; its text is copied
internally, so the screen does not go blank.

Back asks `Abort session?` with `No` / `Yes`. `No` returns to the waiting
screen without restarting the tag read.

### Tag detected / writing (`Popup`)

Header `Writing <index>/<total>`, text with the detected model and NDEF
capacity (for example `NTAG213 144B`) and `Do not move...`, then `Verifying...`.
The screen stays responsive because all NFC work happens on the worker thread.
Back is ignored during an in-flight write.

### Success (`DialogEx`)

Only reachable after read-back verification. Header `Success`, text with the
model, URL index and `Verified`, and a single `OK` button. OK advances to tag
removal. Back does nothing.

### Remove tag (`DialogEx`)

Header `Remove tag`; text asks the user to remove the tag and place the next
one. If the same UID is detected, the text explicitly says `Same tag detected`.
The screen has no action button; removal or a different tag advances
automatically.

### Error (`DialogEx`)

Header identifies the category: tag, playlist, session or storage.

- Tag errors show `Retry`, `Skip`, `Abort`.
- Setup errors show `New`, `Exit`; tag Retry is never offered.
- The text names the concrete reason (unsupported tag, locked, capacity,
  checksum, line number, and so on).

### Summary (`DialogEx`)

Header `Batch complete` or `Session aborted`; text shows done/total, skipped,
failed attempts and remaining. `Done` returns to the startup screen: a
completed session is cleared, while an aborted session is offered for Resume.
For a completed batch, `New` starts a new session; Back also returns to the
startup screen. Exit from the startup screen with Back. An aborted session is
not marked complete.

## Input conventions

- Center/OK confirms.
- Back is the documented back action; it never advances a successful write.
- Holding Back shows an `Exit app?` confirmation. Yes closes the app; No
  returns to the current screen. A short Back keeps its screen-specific action.
- Back during waiting or removal asks for abort confirmation.
- Back on the success screen is a no-op.
- There is no undocumented input that advances the workflow.

## Debug preview

With `/ext/apps_data/nfc-bulk-url-writer/debug` present, `Preview UI` shows the waiting,
writing, success, removal, tag-error and summary screens in sequence. Popup
screens advance on a 1.2 s timeout; dialogs advance with OK or Back. No tag is
read and no session is created.

## Diagnostics

`NFC read diagnostics` reads and validates one tag without writing. The first
page shows the model, page count, pages read, UID, Capability Container bytes,
GET_VERSION bytes and NDEF capacity. `Next` shows the static lock bytes, dynamic
lock bytes and the first two configuration pages. This is the read-only hardware
spike.
