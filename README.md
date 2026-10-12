# Infodock

A compact system-information and launcher bar for the Windows desktop: a thin bar docked to the top or bottom of the screen with small live sensors (CPU, memory, network, date, time, battery…) and launchers.

Status: early development. See [SPEC.md](SPEC.md) for the full specification and [docs/plan.md](docs/plan.md) for progress.

## Download

Every push to `master` builds Windows downloads. Open the latest successful [CI run](https://github.com/jurakovic/infodock/actions/workflows/ci.yml) and download:

- `infodock-windows-installer`: contains `infodock_<version>_x64-setup.exe`, a per-user installer that does not need admin rights;
- `infodock-windows-portable`: contains `infodock.exe`, which runs as is, without installing.

Neither is code-signed yet, so SmartScreen may warn.

## Development

Requirements: Rust (stable). On Linux, `libfontconfig-dev` for the UI toolkit. The bar only appears on Windows; on other systems the app opens Settings only.

```sh
cargo run --manifest-path src-tauri/Cargo.toml   # run the app
cargo test --manifest-path src-tauri/Cargo.toml  # tests
cargo lint                                       # clippy
```

## Licence

MIT, see [LICENSE](LICENSE).

The Settings window and the tray icon are built with [Slint](https://slint.dev), used under the Slint Royalty-free License 2.0:

[![Made with Slint](https://raw.githubusercontent.com/slint-ui/slint/master/logo/MadeWithSlint-logo-whitebg.png)](https://slint.dev)
