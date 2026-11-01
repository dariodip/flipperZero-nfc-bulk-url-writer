# Tag validation

Only NTAG213, NTAG215 and NTAG216 are supported. The application identifies the
model from the GET_VERSION (`0x60`) response, never from the UID.

## Model identification

GET_VERSION returns eight bytes. Byte 6 is the storage-size code:

| Storage size | Model | Pages | User memory | CC byte 2 | NDEF memory |
|---:|---|---:|---:|---:|---:|
| `0x0F` | NTAG213 | 45 | 144 | `0x12` | 144 |
| `0x11` | NTAG215 | 135 | 504 | `0x3E` | 496 |
| `0x13` | NTAG216 | 231 | 888 | `0x6D` | 872 |

Any other storage-size byte, a missing GET_VERSION response, or a failed read
means the tag is unsupported. UIDs are used only to distinguish physical tags
for the removal guard.

## Capability Container

Page 3 must be a valid Type 2 Tag Capability Container:

| Byte | Required value |
|---|---|
| 0 | `0xE1` (NDEF magic) |
| 1 | `0x10` (version 1.0) |
| 2 | model-specific size byte from the table above |
| 3 | `0x00` (free read and free write) |

A wrong magic or version is reported as an invalid Capability Container. A
size byte that does not match the identified model is a mismatch. A non-zero
access byte is treated as read-only.

The Capability Container is never written.

## Lock bits

Before writing, the application rejects a tag when:

- page 2 bytes 2 or 3 (static lock bytes) are non-zero;
- the dynamic-lock page contains a non-zero value in any of its first three
  bytes. The dynamic-lock page is the configuration page minus one: page 40
  for NTAG213, 130 for NTAG215, 226 for NTAG216.

Lock pages are never modified.

## Protection

The configuration pages start at page 41 (NTAG213), 131 (NTAG215), 227
(NTAG216). The application reads:

- `AUTH0`, the fourth byte of the first configuration page;
- the `ACCESS` byte, the first byte of the second configuration page.

It rejects the tag when:

- `PROT` (bit 7 of `ACCESS`) is set, which makes user memory read-only;
- `AUTH0` is at or below the last user page, which means the user area requires
  password authentication. A factory tag uses `AUTH0 = 0xFF` (disabled).

The application never attempts a password, never uses the default password,
never changes `AUTH0`, `PROT`, `PWD`, `PACK`, or any configuration page, and
never formats a tag. A protected tag is reported and left untouched.

## Capacity

The URL's exact encoded TLV size is compared with the NDEF memory derived from
the Capability Container (see `docs/ndef.md`). The nominal user-memory figures
504 and 888 are not used for the capacity check.

## Write range

Programming starts at page 4 and covers the tag's declared NDEF area. It
writes the encoded TLV followed by zeros, removing stale bytes from earlier
messages so simple phone readers cannot become confused. Pages 0-3 (UID, static
locks, Capability Container), the dynamic-lock page, the configuration pages,
PWD/PACK, and user memory outside the NDEF area are never written.

## Read-back

A tag is considered programmed only after its NDEF area is read again and the
decoded URI equals the playlist entry byte-for-byte. A write that completed but
whose read-back failed is a failure, and the session does not advance.

## Reuse guard

After a verified write and explicit confirmation, the programmed UID is
remembered. Presenting the same UID again shows a "remove the tag" screen and
does not advance. Removing the tag or presenting a different UID allows the
next entry.
