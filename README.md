<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="icons/logo-dark.svg">
    <img src="icons/logo-light.svg" alt="Infodock" width="280">
  </picture>
</p>

A compact system-information and launcher bar for the Windows desktop: a thin bar docked to the top or bottom of the screen with small live sensors (CPU, memory, network, date, time, battery…) and launchers.

Status: early development. See [SPEC.md](SPEC.md) for the full specification and [docs/plan.md](docs/plan.md) for progress.

## Download

Download from the [releases](https://github.com/jurakovic/infodock/releases):

- `Infodock_v<version>_win-x64-setup.exe`: a per-user installer that does not need admin rights;
- `Infodock_v<version>_win-x64.zip`: `infodock.exe`, which runs as is, without installing.

Releases marked as pre-release are previews of the next version. `checksums.txt` in each release has the SHA-256 checksums of both files. Neither is code-signed yet, so SmartScreen may warn.

Every push to `master` that changes the app also builds both: open the latest successful [CI run](https://github.com/jurakovic/infodock/actions/workflows/ci.yml) and download `Infodock_v<version>_win-x64-setup.exe` or `Infodock_v<version>_win-x64` (the zipped `infodock.exe`) under *Artifacts*. Their version is the next release with the run number, e.g. `1.0.0-build.150`.

## Accessibility

- **Keyboard**: the hotkey (default Ctrl+Alt+Shift+I) focuses the bar. Tab and the arrow keys move a ring between sensors, Enter or Space clicks the ringed one, Shift+F10 or the Menu key opens its menu, Esc returns to the window you were in. Settings works entirely from the keyboard.
- **Screen readers**: the bar is a UI Automation provider, so Narrator and other screen readers read each sensor's name and value, click it and open its popup; Settings exposes every control with a name.
- **Not followed**: Windows' high-contrast themes and the separate *Text size* setting apply neither to the bar nor to Settings (Slint 1.18, which draws Settings, follows neither); light and dark mode and the display scale are followed.

## Development

Requirements: Rust (stable). On Linux, `libfontconfig-dev` for the UI toolkit. The bar only appears on Windows; on other systems the app opens Settings only.

```sh
cargo run    # run the app
cargo test   # tests
cargo lint   # clippy
```

Local builds are version `0.0.0`; CI builds and releases carry theirs.

### Release

Releases are made with the [Publish](.github/workflows/publish.yml) workflow (*Actions* → *Publish* → *Run workflow*), which runs the steps of `publish.sh`:

1. **Build**: runs the tests and builds `infodock.exe` and the installer with the version
2. **Package**: writes the installer, `infodock.exe` zipped and `checksums.txt` to `release/`
3. **Release**: tags the built commit `v<version>` and creates the GitHub release

The version is computed from the latest tag: `master` gives the next release (`1.0.0` first, then `1.0.1`), other branches the next preview (`1.0.1-preview.1`). *Bump Type* chooses the part that is increased; *Version* sets it outright. `publish.sh` also runs locally in Git Bash on Windows, see `./publish.sh --help`.

## Licence

MIT, see [LICENSE](LICENSE).

The Settings window is built with [Slint](https://slint.dev), used under the Slint Royalty-free License 2.0:

<a href="https://slint.dev"><img src="https://raw.githubusercontent.com/slint-ui/slint/master/logo/MadeWithSlint-logo-whitebg.png" alt="Made with Slint" height="50"></a>
