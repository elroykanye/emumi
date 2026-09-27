# Contributing to EmuMi

Thank you for helping improve EmuMi.

## Development setup

1. Install Rust stable, Node.js 24, the Android SDK command-line tools and the
   Linux libraries listed in the README.
2. Install frontend dependencies with `npm ci` inside `frontend/`.
3. Build the frontend with `npm run build`.
4. Run the desktop application with `cargo run`.

## Before opening a pull request

Run the same checks used by CI:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cd frontend
npm ci
npm run typecheck
npm run build
```

Keep changes focused, explain user-visible behavior, and include tests for
logic that can be exercised without launching an emulator. Do not commit SDKs,
AVD data, account information, local configuration or generated release files.

By contributing, you agree that your contribution is licensed under the MIT
License.
