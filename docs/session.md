# Session format and recovery

Progress is stored in one versioned, checksummed text file:

```
/ext/apps_data/nfc-bulk-url-writer/session.txt
```

A backup copy is written as:

```
/ext/apps_data/nfc-bulk-url-writer/session.bak
```

## Format

```
NFC-BULK-URL-WRITER-SESSION 1
playlist=/ext/apps_data/nfc-bulk-url-writer/playlists/batch.txt
fingerprint=0123456789abcdef
total=42
next=7
completed=5
skipped=2
failed=1
complete=0
checksum=fedcba9876543210
```

- `1` is the format version. An unknown version is not resumed.
- `playlist` is the absolute path of the playlist.
- `fingerprint` is the 64-bit semantic playlist fingerprint, as 16 hex digits.
- `total` is the number of accepted URLs.
- `next` is the zero-based index of the current URL.
- `completed` counts verified writes.
- `skipped` counts explicit skips.
- `failed` counts failed tag attempts; it does not advance the index.
- `complete` is `1` only when `next == total`.
- `checksum` is FNV-1a 64 over every byte before the `checksum=` line.

The counters must satisfy:

```
next == completed + skipped
next <= total
complete == (next == total)
```

A file that violates them, fails the checksum, is truncated, or uses an unknown
version is treated as corrupt and is not resumed.

## When progress is saved

Progress changes only after:

- a successful read-back verification **and** the user presses OK on the success
  screen; or
- the user explicitly chooses Skip on a tag error.

Write success alone never advances the session. A failed write, failed
read-back, or verification mismatch leaves the current index unchanged and does
not write the session file; the in-memory failed-attempt counter is saved with
the next verified/skip/abort save. An abort saves the current counters so the
session can be resumed later; it is not marked complete.

## Resume rules

On startup:

- no session file: start a new session;
- valid, incomplete session and matching playlist: offer Resume and Start New;
- completed session: show the summary with a New option; do not offer Resume.
  Pressing Done clears the completed session and returns to startup;
- missing playlist: report it; do not resume;
- changed fingerprint or URL count: report that the playlist changed; do not
  silently resume;
- corrupt or unsupported session: report it; do not infer any index from it.

The application never resumes at an index beyond the last verified or
explicitly skipped URL.

## Power-loss behavior

If power is lost after a tag was written and verified but before the success
confirmation was persisted, the same URL is presented again on the next run.
This is intentional: programming a tag twice is safe and visible, while
skipping an unverified entry is not.

If a save is interrupted, the previous file remains the recovery point. The
adapter writes a temporary file, syncs it, rotates the previous valid file to
`session.bak`, and moves the new file into place. The loader uses the newest
valid record; if neither the primary nor the backup is valid, it does not
resume.

A session write failure is reported as a storage error and is not presented as
saved progress.
