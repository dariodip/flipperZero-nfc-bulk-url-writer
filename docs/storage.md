# microSD storage

Everything the application reads or writes lives under
`/ext/apps_data/nfc-bulk-url-writer/`.

## Playlists

```
/ext/apps_data/nfc-bulk-url-writer/playlists/
```

- Only files ending in `.txt` are listed (the extension is matched
  case-insensitively).
- Names are sorted ascending by display name (the name without `.txt`).
- Subdirectories are ignored.
- The directory is not created by the application. If it is missing, the UI
  reports the expected path.

The playlist itself is streamed with a 256-byte read buffer and a 1025-byte line
buffer. The whole file is never loaded into memory. To read entry *N* after a
resume or a retry, the file is reopened and scanned from the start; this keeps
memory bounded and preserves order.

## Settings

```
/ext/apps_data/nfc-bulk-url-writer/settings.txt
```

Stores the chosen playlist folder:

```
playlist_dir=/ext/apps_data/nfc-bulk-url-writer/playlists
```

If the file is absent or the folder is missing, the app falls back to the
default playlist folder and reports a setup error when the folder cannot be
opened. **Load batch from storage** opens the folder picker; **Use this folder**
writes the setting.

## Session files

```
/ext/apps_data/nfc-bulk-url-writer/session.txt     primary
/ext/apps_data/nfc-bulk-url-writer/session.tmp     temporary file
/ext/apps_data/nfc-bulk-url-writer/session.bak     previous valid file
```

Saving:

1. serialize and validate the session (counters and checksum);
2. write `session.tmp` and sync it;
3. if `session.txt` exists, remove the old `session.bak` and rename
   `session.txt` to `session.bak`;
4. rename `session.tmp` to `session.txt`.

Loading tries `session.txt` first and falls back to `session.bak`. A missing
primary file with no backup means "no session". A file that fails its checksum,
version check or counter invariants is not used to resume; the UI reports a
corrupt session and offers to start a new one.

The format is documented in `docs/session.md`.

## Error mapping

| Storage condition | UI category |
|---|---|
| playlist directory missing | setup error: no playlist directory, with the expected path |
| playlist file missing at selection or resume | setup error: playlist missing / changed |
| playlist read failure | setup error: storage error |
| empty or invalid playlist | setup error: playlist error with line number |
| session read failure | setup error: session/storage error; no resume |
| session write failure | storage error: progress was not saved; no success claim |

The adapter keeps "missing", "empty", "read failure", "parse failure" and
"write failure" distinct; they are not collapsed into a generic failure.

## Debug flag

If this file exists, the startup menu also shows the NFC diagnostics and UI
preview entries:

```
/ext/apps_data/nfc-bulk-url-writer/debug
```

The flag only enables read-only diagnostics and screen previews. It never
changes the programming workflow or writes to a tag by itself.
