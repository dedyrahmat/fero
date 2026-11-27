# Contributing to Fero

Use [GitHub Issues](https://github.com/dedyrahmat/fero/issues) for bugs and feature requests. See [SECURITY.md](SECURITY.md) for vulnerability reports.

1. Follow the [build instructions](docs/development.md#build-fero-locally) for your platform.
2. Run `cargo fmt --all -- --check` and `cargo test --locked --no-default-features`. For desktop changes, also run `cargo test --locked --bin fero` and exercise the changed workflow in the native app.
3. Open a pull request describing the behavior change, the platform and commands tested, and any related issue. For visible changes, include screenshots and note the window sizes and appearances you checked.
