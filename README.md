# Infodock

A compact system-information and launcher bar for the Windows desktop: a thin bar docked to the top or bottom of the screen with small live sensors (CPU, memory, network, date, time, battery…) and launchers.

Status: early development. See [SPEC.md](SPEC.md) for the full specification and [docs/plan.md](docs/plan.md) for progress.

## Download

Every push to `master` builds a Windows installer. Open the latest successful [CI run](https://github.com/jurakovic/infodock/actions/workflows/ci.yml) and download the `infodock-windows-installer` artifact. The installer is per-user and does not need admin rights. For a build without installing, download `infodock-windows-portable` instead: it contains `infodock.exe`, which runs as is (it needs the WebView2 runtime, which Windows 10/11 normally has). Neither is code-signed yet, so SmartScreen may warn.

## Development

Requirements: [Bun](https://bun.sh), Rust (stable), and on Windows the WebView2 runtime (preinstalled on Windows 11).

```sh
bun install
bun run tauri dev      # run the app
bun run test           # UI tests
bun run lint           # eslint, svelte-check, rustfmt, clippy
cargo test --manifest-path src-tauri/Cargo.toml
```

## Licence

MIT, see [LICENSE](LICENSE).
