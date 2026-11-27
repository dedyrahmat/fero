# Repository Guidelines

## Project Structure

- `src/main.rs` starts the GPUI desktop app and runs transfer commands on a worker thread. `src/desktop.rs` and `src/text_field.rs` implement the interface.
- `src/lib.rs` manages the transfer library and SQLite persistence. `src/bridge.rs` defines the CXX interface to the C++17 engine in `native/`. `build.rs` compiles the bridge and links libtorrent.
- `src/tests.rs` contains local swarm tests and a manual desktop acceptance helper. `design/` holds the desktop mockup and bundled fonts. `PLAN.md` describes release requirements.

## Build and Run

Use stable Rust, a C++17 compiler, CMake, Boost, OpenSSL, and libtorrent **2.1.2**. Follow the [build instructions](docs/development.md#build-fero-locally) for your platform. On macOS and Linux, add `$PWD/.native/install/lib/pkgconfig` to `PKG_CONFIG_PATH` after installing libtorrent locally. Keep Cargo builds locked to `Cargo.lock`.

- `cargo build --locked` compiles the desktop app.
- `cargo run --locked` launches Fero locally.
- `cargo test --locked --no-default-features` runs the engine and persistence tests without GPUI.

## Coding Style

Use four-space indentation and `cargo fmt` for Rust. Use `snake_case` for functions and modules and `UpperCamelCase` for types. Match the surrounding C++ formatting and keep the declarations in `native/engine.h` aligned with the Rust bridge. Keep network and disk work on the worker thread so the interface stays responsive.

## Testing

Write built-in Rust `#[test]` cases with descriptive names such as `download_restart_and_reseed`. Use temporary directories and loopback peers for transfer tests. Cover restart, byte-for-byte completion, and reseeding when changing the transfer lifecycle.

For native-window changes, start with macOS and use the [desktop acceptance workflow](docs/development.md#run-phase-1-desktop-acceptance). Run `cargo test --locked --no-default-features desktop_acceptance -- --ignored --nocapture` to start the helper. Use its temporary `FERO_DATA_DIR` for the desktop run.

## Commits and Pull Requests

Use short, imperative commit subjects, such as `Preserve file selections across restart`. Summarize the behavior change, link any related issue, and report the commands and platforms you verified. For visible UI changes, include screenshots and note which window sizes and appearances you checked.
