# Playlists

A playlist is a plain-text file with one URL per line. Each accepted line is
one tag to program, in file order.

## Location and naming

The default playlist folder is:

```
/ext/apps_data/nfc-bulk-url-writer/playlists/
```

Only files ending in `.txt` are listed. The list is sorted ascending by display
name. Subdirectories are not scanned.

You can choose a different folder from the app: **Load batch from storage** →
navigate to the folder → **Use this folder**. The choice is remembered in
`/ext/apps_data/nfc-bulk-url-writer/settings.txt` as `playlist_dir=<path>`. The
default folder is used when no choice has been saved.

## Example

```text
# Monday batch
https://example.test/contact/anna

http://example.test/contact/bruno
# The same URL may appear twice on purpose
https://example.test/contact/anna
```

This playlist has three entries, not four: the comment lines and the blank line
are ignored.

## Rules

- Empty lines are ignored.
- A line whose first non-whitespace character is `#` is a comment and is ignored.
- Every other line is a URL. The line is **not** trimmed: leading or trailing
  spaces make the URL invalid.
- CRLF line endings and a UTF-8 byte-order mark at the start of the file are
  accepted.
- Order is preserved. Duplicate URLs are independent entries and are programmed
  on separate tags.
- Playlist files are never modified by the application.

## Accepted URLs

An entry must be:

- an `http://` or `https://` URL, with the scheme compared case-insensitively;
- non-empty after the host marker, with a non-empty host;
- composed only of printable ASCII (0x21-0x7E), so spaces, tabs, control
  characters and non-ASCII characters are rejected;
- free of incomplete percent escapes; every `%` must be followed by two
  hexadecimal digits.

Accepted URLs are stored exactly as written. The application never normalises
case, percent escapes, paths, query strings, fragments, or trailing slashes.

Unsupported schemes (`ftp://`, `mailto:`, and so on) are rejected with the line
number. An invalid URL stops playlist loading; nothing is programmed until the
file is fixed.

An empty playlist (only comments and blank lines) is rejected.

## Fingerprint

The session stores a 64-bit semantic fingerprint of the ordered URL sequence.
It uses FNV-1a with an explicit length prefix per URL. As a result:

- changing comments, blank lines, CRLF style, or the byte-order mark does not
  change the fingerprint;
- editing, adding, removing, or reordering a URL changes it;
- the order of duplicate URLs matters.

The fingerprint makes a resumed session refuse to run against a different
playlist instead of silently continuing at the wrong index.

## Limits

The application reads a playlist line by line and does not load the whole file
into memory. One line is limited to 1024 URL bytes plus the line terminator.
Playlists may be much larger than the tag capacity because only one URL is
programmed at a time.
