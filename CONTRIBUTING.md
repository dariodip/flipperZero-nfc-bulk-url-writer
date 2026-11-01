# Contributing to NFC Bulk URL Writer

Thank you for your interest in contributing to NFC Bulk URL Writer!

Contributions are welcome, whether they involve fixing bugs, improving documentation, adding features, or testing the application.

This document explains how to contribute to the project.

## Getting Started

Before making changes:

1. Read the project's README and existing documentation.
2. Check open issues and pull requests to avoid duplicating existing work.
3. For significant changes, open an issue first to discuss the proposed solution.
4. Fork the repository and create a dedicated branch for your changes.

Keep contributions focused and easy to review.

## Development Environment

NFC Bulk URL Writer is developed primarily in Rust, with a C adapter for integration with the Flipper Zero firmware APIs.

The development environment requires:

- Rust toolchain compatible with the project.
- Cargo.
- Flipper Zero firmware development tools (FBT), where required.
- A Flipper Zero device for hardware testing.
- Git.

Follow the installation and build instructions in the project README.

Do not introduce new dependencies without a clear technical reason. Prefer small, well-maintained dependencies.

## Code Guidelines

### Rust

- Follow standard Rust conventions and idioms.
- Run `cargo fmt` before submitting changes.
- Run `cargo clippy` where supported by the project configuration.
- Prefer explicit error handling over panics.
- Keep modules focused and responsibilities well-defined.
- Document public APIs and non-obvious implementation details.
- Avoid `unsafe` unless strictly necessary. If used, document its safety requirements.

### C Adapter

- Keep the C adapter as small as possible.
- Use it only for functionality that cannot reasonably be accessed through the Rust bindings.
- Keep the Rust/C interface explicit and documented.
- Respect Flipper Zero firmware API and memory-management conventions.

### General

- Keep changes consistent with the existing architecture.
- Avoid unrelated refactoring in feature-specific pull requests.
- Update documentation whenever behavior or user-facing functionality changes.
- Do not commit generated binaries, temporary files, or local development configuration.

## NFC Safety

NFC Bulk URL Writer interacts with physical NFC tags. Changes affecting NFC operations must be implemented carefully.

In particular:

- Never write to tag manufacturer data or UID pages.
- Never modify lock bits, password pages, PACK, or configuration pages unless explicitly required by an approved feature.
- Do not attempt to bypass tag protection mechanisms.
- Verify write operations by reading back the written data.
- Handle unsupported, read-only, protected, and insufficient-capacity tags safely.
- Never advance to the next URL when the current write has not been successfully verified.
- Do not silently overwrite existing tag data.

Changes involving NFC memory layout, NDEF encoding, tag detection, or write operations should include automated tests whenever practical.

Hardware testing with a Flipper Zero is strongly encouraged for changes affecting NFC behavior.

## Tests and Validation

Before submitting a pull request, run the checks relevant to your changes.

Where supported by the project configuration:

```bash
cargo fmt --all -- --check
cargo test
cargo clippy --all-targets --all-features
```

Some checks may require the Flipper Zero target or firmware build environment. Follow the repository README for the correct build procedure.

If you cannot run a particular check, explicitly mention this in your pull request.

Never report tests as passing unless you have actually run them.

## Commit Messages

Use clear, descriptive commit messages.

The following conventional prefixes are encouraged:

- `feat:` for new features.
- `fix:` for bug fixes.
- `docs:` for documentation changes.
- `test:` for tests.
- `refactor:` for code restructuring.
- `chore:` for maintenance tasks.

Examples:

```text
feat: add NTAG216 support
fix: handle read-only NFC tags
docs: improve installation instructions
test: add NDEF URI encoding tests
```

Keep commits focused and avoid mixing unrelated changes.

## Pull Requests

When opening a pull request:

1. Explain the problem or motivation.
2. Describe the proposed solution.
3. Summarize relevant implementation details.
4. Include tests and manual validation performed.
5. Mention limitations or known issues.
6. Link related issues, if applicable.

For changes affecting NFC behavior, also include:

- NFC tag models tested (e.g. NTAG213, NTAG215, NTAG216).
- Whether testing was performed on physical hardware or only through automated tests.
- Relevant test results and observed behavior.

Pull requests should be focused, reviewable, and based on the latest version of the default branch.

## Bug Reports and Feature Requests

When reporting a bug, provide:

- A clear description of the problem.
- Steps to reproduce it.
- Expected behavior.
- Actual behavior.
- Relevant logs or error messages.
- Flipper Zero firmware version and hardware details, where relevant.

For feature requests, explain the use case and why the proposed feature would be useful.

Do not include sensitive information, personal data, or private NFC tag contents in public issues.

## Security Issues

Please do not disclose security vulnerabilities or potentially harmful NFC behaviors in public issues before they have been assessed.

Use the repository's private vulnerability reporting mechanism, if available. Otherwise, contact the project maintainer privately.

## License

By contributing to NFC Bulk URL Writer, you agree that your contributions will be released under the MIT License.

You retain copyright ownership of your contributions while granting the permissions described by that license.

## Code of Conduct

Be respectful, constructive, and inclusive.

Technical disagreements are welcome. Personal attacks, harassment, and disrespectful behavior are not.

We aim to maintain a collaborative environment where contributors can exchange ideas and improve the project together.

Thank you for helping make NFC Bulk URL Writer better!
