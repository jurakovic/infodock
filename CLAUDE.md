# Infodock — working notes for Claude

`SPEC.md` is the source of truth. Read the sections relevant to your task before writing code, and cite them (`§5.2`) in code comments only where the reason is not obvious. If the spec is wrong or ambiguous, fix the spec in the same commit and say so in the commit message.

`docs/plan.md` tracks milestones and slices. Tick items off as they land and add decisions to its log.

## Layout

The repository root is the Rust crate `infodock` (lib `infodock_lib`).

```
src/                 Rust sources
  bar/               the native bar: BarController (any thread) and its logic on the bar thread
  settings/          the Settings window (Slint) and its glue to the config
  platform/          OS layer behind traits; Windows code in platform/windows, other OSes get a stub
ui/                  Slint UI: the Settings window (`.slint`, compiled by build.rs)
i18n/                message catalogs (`en.json`)
icons/               app icons
resources/           Windows application manifest
.github/scripts/     CI helpers (memory check)
docs/                plan.md, manual-test.md
tests/fixtures/      shared test fixtures (legacy XML)
```

## Commands

| What | Command |
|---|---|
| Format | `cargo fmt` |
| Lint (clippy, warnings are errors) | `cargo lint` |
| Tests | `cargo test` |
| Render the Settings pages to `target/screenshots/*.png` | `cargo test render_settings_pages -- --ignored` |
| Type-check the Windows code from Linux | `cargo check-win` |
| Run the app | `cargo run` (Windows; elsewhere only Settings opens) |
| Build the installer (Windows) | `cargo build --release` then `cargo packager --release` |

Cloud sessions run on Linux. `check-win` compiles every `#[cfg(windows)]` path without linking; it needs the `x86_64-pc-windows-msvc` target and `llvm-rc` (installed by the SessionStart hook, together with `libfontconfig-dev` for Slint on Linux). To look at UI changes, render the Settings pages and open the PNGs. Real Windows behaviour is verified by CI (`.github/workflows/ci.yml`: clippy, tests, `--ignored` integration tests, Settings screenshots, NSIS installer and portable exe, memory check) and by the manual checklist in `docs/manual-test.md`.

## Gate before every commit

`cargo fmt --check`, `cargo lint`, `cargo test` and `cargo check-win`.

## Workflow

- Commit straight to `master`, one logical change per commit, and **push after every commit**.
- After pushing, check the CI run; if it is red, fixing it comes before anything else. CI runs only when build inputs change (paths in `ci.yml`); docs-only commits do not trigger it.
- End commit messages with the `Co-Authored-By` trailer the session provides.

## Conventions

- Rust: edition 2024, no `unwrap()`/`expect()` on runtime paths (setup code that cannot continue may `expect`). OS calls only inside `platform/`; everything else talks to the `Platform` trait.
- Pure logic (geometry, config merging, formatting, settings mapping) lives in plain functions with unit tests and no OS calls.
- Every user-visible string goes through `i18n::t()` in Rust or `I18n.tr()` in `.slint`; keys live in `i18n/en.json`.
- Config `props` store only values that differ from static defaults (SPEC §11.2).
- Coordinates are logical (DIP) in layout and drawing; platform code converts to physical pixels with the monitor scale.
- Threads: the bar thread owns the bar, hint and popup windows, the tray icon and native menus; other threads talk to it through `BarController`, which queues a command and wakes the thread. The main thread runs Slint's event loop and owns the Settings window and the hotkey; other threads reach it with `slint::invoke_from_event_loop`. Never touch either thread's UI from another thread.
- Slint: Fluent style; controls bind two-way to a global (`SettingsState`), and Rust saves changes through settings patches. Every component instance needs `install_i18n!`.
- Memory budget: ≤ 30 MB with Settings closed (SPEC §14.1). CI's memory check reports it on every Windows build; keep an eye on it when adding dependencies or caches.
