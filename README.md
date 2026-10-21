# Infodock

A compact system-information and launcher bar for the Windows desktop: a thin bar docked to the top or bottom of the screen with small live sensors (CPU, memory, network, date, time, battery…) and launchers.

Status: early development. See [SPEC.md](SPEC.md) for the full specification and [docs/plan.md](docs/plan.md) for progress.

## Download

Every push to `master` builds Windows downloads. Open the latest successful [CI run](https://github.com/jurakovic/infodock/actions/workflows/ci.yml) and download:

- `infodock-windows-installer`: contains `infodock_<version>_x64-setup.exe`, a per-user installer that does not need admin rights;
- `infodock-windows-portable`: contains `infodock.exe`, which runs as is, without installing.

Neither is code-signed yet, so SmartScreen may warn.

## Accessibility

- **Keyboard**: the hotkey (default Ctrl+Alt+Shift+I) focuses the bar. Tab and the arrow keys move a ring between sensors, Enter or Space clicks the ringed one, Shift+F10 or the Menu key opens its menu, Esc returns to the window you were in. Settings works entirely from the keyboard.
- **Screen readers**: the bar is a UI Automation provider, so Narrator and other screen readers read each sensor's name and value, click it and open its popup; Settings exposes every control with a name.
- **Not yet followed**: Windows' high-contrast themes and the separate *Text size* setting apply neither to the bar nor to Settings (Slint 1.18, which draws Settings, follows neither); light and dark mode and the display scale are followed.

## Development

Requirements: Rust (stable). On Linux, `libfontconfig-dev` for the UI toolkit. The bar only appears on Windows; on other systems the app opens Settings only.

```sh
cargo run    # run the app
cargo test   # tests
cargo lint   # clippy
```

## Licence

MIT, see [LICENSE](LICENSE).

The Settings window is built with [Slint](https://slint.dev), used under the Slint Royalty-free License 2.0:

[![Made with Slint](https://raw.githubusercontent.com/slint-ui/slint/master/logo/MadeWithSlint-logo-whitebg.png)](https://slint.dev)
