# Development

The project builds a Flipper Zero FAP in Rust with a host-testable `no_std`
core. The generated target and linker configuration in `.cargo/config.toml`
is kept as-is.

## Layout

| Path | Purpose |
|---|---|
| `src/` | Flipper adapter: entry point, UI, storage, NFC worker. |
| `core/` | `nfc-bulk-url-writer-core`, a `no_std`, allocation-free crate with all protocol, playlist, session and workflow logic. |
| `core/tests/fixtures/` | Shared playlist, NDEF and synthetic NTAG page fixtures. |
| `docs/` | Development and user documentation. |

The root package is the FAP; `core` is a workspace member. The core has no
Flipper SDK dependency, so it can be tested on the desktop.

## Make targets

The repository includes a `Makefile`:

```sh
make test        # host-testable core tests
make fmt         # format all Rust code
make fmt-check   # check formatting without changing files
make clippy      # run clippy
make check       # fmt-check + clippy + test
make build       # debug FAP
make release     # release FAP
make deploy      # release + copy to the Flipper
make deploy-debug
make run         # release + run through run-fap
make clean
```

`storage` and `run-fap` auto-detect the Flipper. To select a device explicitly:

```sh
make deploy FLIPPER_PORT=/dev/cu.usbmodemflip_XXXX
```

The destination can be overridden with
`FLIPPER_FAP_PATH=/ext/apps/Examples/nfc-bulk-url-writer.fap`.

## Build the FAP

```sh
cargo build
```

Output:

```
target/thumbv7em-none-eabihf/debug/nfc-bulk-url-writer.fap
```

Release build:

```sh
cargo build --release
```

Output:

```
target/thumbv7em-none-eabihf/release/nfc-bulk-url-writer.fap
```

The `.cargo/config.toml` sets `thumbv7em-none-eabihf` as the default target;
do not remove it.

## Run the core tests on the desktop

The default target cannot run host tests, so override it explicitly:

```sh
cargo test -p nfc-bulk-url-writer-core --target aarch64-apple-darwin
```

On other hosts, substitute the local host triple (`x86_64-unknown-linux-gnu`,
`x86_64-pc-windows-msvc`, and so on).

The suite covers URL validation, playlist parsing and fingerprinting, NDEF
encoding/decoding, tag validation, session persistence and the workflow state
machine. It also loads every file in `core/tests/fixtures/`.

## Check that both packages are present

```sh
cargo metadata --no-deps --format-version 1
```

The output lists `nfc-bulk-url-writer` and `nfc-bulk-url-writer-core`.

## Deploy

With a Flipper connected and `flipperzero-tools` installed:

```sh
storage send target/thumbv7em-none-eabihf/debug/nfc-bulk-url-writer.fap \
  /ext/apps/Examples/nfc-bulk-url-writer.fap
```

Playlists live under `/ext/apps_data/nfc-bulk-url-writer/playlists/`. See
`docs/playlists.md` and `docs/storage.md`.

## Toolchain

The repository pins `nightly-2025-08-31` in `rust-toolchain.toml` and uses the
`different-binary-name` Cargo feature. Both are required for the `.fap`
extension; keep them when upgrading the SDK.
