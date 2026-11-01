# NFC Bulk URL Writer

A Flipper Zero FAP that programs batches of NTAG213/215/216 tags with unique
HTTP/HTTPS URLs from a microSD playlist. Every write is verified by reading the
URL back, and interrupted batches can be resumed.

## Build

```sh
make release
```

or directly:

```sh
cargo build --release
```

The FAP is written to:

```
target/thumbv7em-none-eabihf/debug/nfc-bulk-url-writer.fap
```

Release:

```sh
cargo build --release
```

## Test, format and check

```sh
make test
make fmt
make check
```

`make test` detects the host target automatically. The protocol and workflow
logic is a `no_std`, allocation-free crate that runs on the desktop.

## Deploy

```sh
make deploy
```

or, with an explicit port:

```sh
make deploy FLIPPER_PORT=/dev/cu.usbmodemflip_XXXX
```

`storage` and `run-fap` auto-detect the Flipper when no port is given.

Create playlists under `/ext/apps_data/nfc-bulk-url-writer/playlists/`; one URL per
line. See [docs/user-guide.md](docs/user-guide.md).

## Documentation

- [User guide](docs/user-guide.md)
- [Playlist format](docs/playlists.md)
- [NDEF encoding and capacity](docs/ndef.md)
- [Tag validation](docs/tags.md)
- [Session recovery](docs/session.md)
- [Workflow state machine](docs/workflow.md)
- [Storage layout](docs/storage.md)
- [UI screens](docs/ui.md)
- [NFC worker](docs/nfc-worker.md)
- [NFC ABI and hardware spike](docs/nfc-abi.md)
- [Validation](docs/validation.md)
- [Development](docs/development.md)

## License

MIT.
