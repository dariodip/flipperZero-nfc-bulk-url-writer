# NDEF URI encoding

The application writes exactly one NFC Forum Well-Known URI record inside one
Type 2 Tag NDEF Message TLV, followed by the mandatory `0xFE` terminator.

## Layout

For a URL split into a URI Identifier Code and a remainder:

```
03                     Type 2 Tag NDEF Message TLV
<length>               message length: 1 byte, or FF + 2 bytes big-endian
D1                     MB | ME | SR | TNF=1   (short record)
01                     Type Length (always 1)
<payload length>       1 byte (short record) or 4 bytes big-endian (long record)
55                     record type 'U'
<prefix code>          1 byte from the NFC Forum URI Identifier Code table
<remainder>            URL bytes after the matched prefix
FE                     terminator
```

A short record is used when the payload (prefix code plus remainder) is at most
255 bytes. A long record is used when it is 256 bytes or more.

The TLV length is one byte when the NDEF message is at most 254 bytes. At 255
bytes and above, the `0xFF` marker and a two-byte big-endian length are used.

## URI prefix compression

The longest matching prefix from the NFC Forum table is used. The table
includes, among others:

| Code | Prefix |
|---|---|
| `0x00` | *(no prefix; full URL stored)* |
| `0x01` | `http://www.` |
| `0x02` | `https://www.` |
| `0x03` | `http://` |
| `0x04` | `https://` |

Matching is case-sensitive. `HTTPS://example.test/` does not match the
lowercase table entry, so it is stored with code `0x00`. The URL itself is
never changed.

## Exact capacity calculation

Let:

- `R` = URL length minus matched prefix length (the remainder);
- `P` = `1 + R` (payload, including the prefix code byte);
- `M` = NDEF message length;
- `T` = encoded TLV length.

For a short record (`P <= 255`):

```
M = 1 (header) + 1 (type length) + 1 (payload length) + 1 (type) + P
```

For a long record (`P >= 256`):

```
M = 1 (header) + 1 (type length) + 4 (payload length) + 1 (type) + P
```

The TLV length is:

```
M < 255:   T = 1 (tag) + 1 (length) + M + 1 (terminator)
M >= 255:  T = 1 (tag) + 3 (FF + length) + M + 1 (terminator)
```

The URL is accepted only when `T <= NDEF memory`. There is no string-length
shortcut and no rounding.

## NDEF memory versus user memory

The capacity is derived from the tag's Capability Container byte 2, multiplied
by eight:

| Tag | CC byte 2 | NDEF memory | Nominal user memory |
|---|---|---:|---:|
| NTAG213 | `0x12` | 144 | 144 |
| NTAG215 | `0x3E` | 496 | 504 |
| NTAG216 | `0x6D` | 872 | 888 |

The larger nominal figures must not be used for the capacity check: the last
eight bytes of NTAG215 user memory and the last sixteen bytes of NTAG216 user
memory are outside the declared NDEF area.

Example with the `https://` prefix (8 bytes):

| NDEF memory | Maximum remainder | Maximum URL length | Encoded TLV |
|---:|---:|---:|---:|
| 144 | 136 | 144 | 144 |
| 496 | 483 | 491 | 496 |
| 872 | 859 | 867 | 872 |

One byte more is rejected.

## Verification

After writing, the application reads the NDEF area and decodes it strictly:

- the NDEF Message TLV must be present (leading NULL TLVs are ignored);
- the message must contain exactly one complete record;
- the record must be Well-Known (`TNF=1`), type `U`, with no ID and no
  chunking;
- the URI Identifier Code must be defined;
- a `0xFE` terminator must follow the message.

The reconstructed URL is compared byte-for-byte with the playlist entry. Case,
percent escapes, query strings, fragments and trailing slashes are not
normalised, so any difference is a verification failure.
