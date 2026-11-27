# Infodock — working notes for Claude

`INTERNALS.md` describes how Infodock is built: its modules and the dependency rule, the threads and windows, the data flow from the sampler to the bar, the platform layer, the sensors, the config, Settings, the theme and skins, localisation, the budgets, building, testing and CI. Read the sections relevant to your task before writing code. It describes the code as it is, and nothing else: no decisions, no history, no plans.

## Layout

The repository root is the Rust crate `infodock` (lib `infodock_lib`).

```
src/                 Rust sources
  bar/               the native bar: BarController (any thread) and its logic on the bar thread
  look/              the theme, skins and animation helpers the bar, sensors and Settings share
  settings/          the Settings window (Slint) and its glue to the config
  sensors/           sensor framework: traits, property schemas, registry, placeholders; one module per type
  sampler/           the sampler thread: topic subscriptions and scheduling
  providers/         the system data behind the topics, read through the Platform trait
  platform/          OS layer behind traits; Windows code in platform/windows, other OSes get a stub
ui/                  Slint UI: the Settings window (`.slint`, compiled by build.rs) and its icons (`ui/icons`)
i18n/                message catalogs (`en.json`)
icons/               app and tray icons, the README logo and the installer's images (`icons/installer`); SVG sources beside their renders
examples/            `icons.rs`: renders the artwork's `.ico`, PNG and BMP files from the SVGs
layouts/             ready-made layouts for Import layout… (`<name>.infodock.json`)
assets/              artwork embedded in the binary (logo, icons)
resources/           Windows application manifest
.github/scripts/     CI helpers (memory, hotkey, Show desktop, drive release and installer checks)
tests/fixtures/      shared test fixtures (legacy XML, skin packs)
INTERNALS.md         how the code is built, as it is now
```

## Commands

| What | Command |
|---|---|
| Format | `cargo fmt` |
| Lint (clippy, warnings are errors) | `cargo lint` |
| Tests | `cargo test` |
| Render the Settings pages to `target/screenshots/*.png` | `cargo test render_settings_pages -- --ignored` |
| Render the bar to `target/screenshots/bar-*.png` (Windows only) | `cargo test render_bar -- --ignored` |
| Measure the time budgets to `target/budgets/*.json` (Windows only) | `cargo test budget_ -- --ignored --test-threads=1` |
| Render the artwork's `.ico`, PNG and BMP files from their SVGs (commit the results) | `cargo run --example icons` |
| Render the tray icons and installer images to `target/screenshots/` | `cargo test --example icons -- --ignored` |
| Type-check the Windows code from Linux | `cargo check-win` |
| Run the app | `cargo run` (Windows; elsewhere only Settings opens) |
| Build the exe and installer with a version (Windows, Git Bash) | `./publish.sh build -v 1.0.0-build.1` (`-t false` skips the tests) |
| The next release's version, from the tags | `./publish.sh version` |

Cloud sessions run on Linux. `check-win` compiles every `#[cfg(windows)]` path without linking; it needs the `x86_64-pc-windows-msvc` target and `llvm-rc` (installed by the SessionStart hook, together with `libfontconfig-dev` for Slint on Linux). To look at UI changes, render the Settings pages and open the PNGs; the bar renders only on Windows, so look at it in CI's `Infodock_v<version>_screenshots` artifact (the GitHub tools give a download link). Drawing code is tested on Linux against the recording canvas (`platform/recording.rs`). Real Windows behaviour is verified by CI (`.github/workflows/ci.yml`: clippy, tests, `--ignored` integration tests, Settings and bar screenshots, NSIS installer and portable exe, budget tests and the memory and time check, the hotkey, Show desktop and drive release checks, a silent install and uninstall).

## Gate before every commit

`cargo fmt --check`, `cargo lint`, `cargo test` and `cargo check-win`. A docs-only commit needs none.

## Workflow

- **Each change on a branch of its own**, from the latest `master` and named for what it does (`feat/sensor-icons`, `fix/busy-drive-icon`, `chore/…`, `docs/…`): one logical change per commit, and **push the branch after every commit** (`git push origin <branch>`).
- **A fix to a commit that isn't on `master` yet is a fixup of it**, not a commit of its own: `git commit --fixup=<sha>`, its code and docs together. A message to correct gets an `amend!` commit: `git commit --allow-empty -m "amend! <the commit's subject>" -m "<the whole new message>"`. Don't rewrite the pushed branch before it's done.
- **When the branch is done**, fold the fixups into their commits, `GIT_SEQUENCE_EDITOR=: git rebase -i --autosquash --committer-date-is-author-date <the commit the branch started from>`, check that the tree didn't change (`git diff <the branch before> HEAD` is empty), force-push the branch (`git push --force-with-lease origin <branch>`) and fast-forward `master` to it (`git push origin <branch>:master`). The branch stays. Nothing else moves `master`: no merge commits, no rebases of it, no force-push. So each commit on `master` is one whole change, its code, tests and docs together.
- A fix to a commit already on `master` is a commit of its own, on a branch of its own like any change.
- Check CI once per batch of commits; if it is red, fixing it comes before anything else. CI runs on pushes to `master` and on pull requests, not on other branches, and only when build inputs change (paths in `ci.yml`); docs-only commits do not trigger it.
- End commit messages with the `Co-Authored-By` trailer the session provides.
- Never tag, release or change the version in `Cargo.toml` (it stays `0.0.0`): the user releases by running the Publish workflow (`INTERNALS.md`, Building and releasing).

## Docs

- **`INTERNALS.md` changes with the code.** A commit that changes the code updates, in the same commit, every place it makes untrue in `INTERNALS.md`, the README, this file and the code comments: not only the section about what it changes, but every one that names or describes the code it touches (a module list, a table of topics, a budget, a CI step). Fix one that is already wrong when you find it.
- **Current state only.** `INTERNALS.md` says how the code is, never how it came to be: no decisions, alternatives, dates, history or plans. It names no work tracked elsewhere.
- **Sections by name.** Code cites it by section name (`INTERNALS.md, Data flow`), and only where a comment needs it.

## Conventions

- Rust: edition 2024, no `unwrap()`/`expect()` on runtime paths (setup code that cannot continue may `expect`). OS calls only inside `platform/`; everything else talks to the `Platform` trait.
- Pure logic (geometry, config merging, formatting, settings mapping) lives in plain functions with unit tests and no OS calls.
- Every user-visible string goes through `i18n::t()` in Rust or `I18n.tr()` in `.slint`; keys live in `i18n/en.json`.
- Config `props` store only values that differ from static defaults (`INTERNALS.md`, Config).
- Coordinates are logical (DIP) in layout and drawing; platform code converts to physical pixels with the monitor scale.
- Threads: the bar thread owns the bar, hint, popup and band windows, the tray icon and native menus; other threads talk to it through `BarController`, which queues a command and wakes the thread. The main thread runs Slint's event loop and owns the Settings window and the hotkey; other threads reach it with `slint::invoke_from_event_loop`. Never touch either thread's UI from another thread.
- Slint: Fluent style; controls bind two-way to a global (`SettingsState`), and Rust saves changes through settings patches. Every component instance needs `install_i18n!`.
- Memory budget: ≤ 30 MB with Settings closed (`INTERNALS.md`, Budgets). CI's memory check enforces it on every Windows build (the step fails when idle or after closing Settings is over); keep an eye on it when adding dependencies or caches. The same step enforces the time budgets: the cold and warm start from the app's log (its first-frame line says when each start-up step was done) and the `budget_*` tests' records; keep work off the way to the bar's first frame.

## Comments

`INTERNALS.md` says how the code is built; a code comment explains the code it sits on.

- **A module header** is one to three lines: what the module is, then the `INTERNALS.md` section that describes it, where one does and it helps.
- **A comment by the code** gives what the code cannot say: a guard's reason, an ordering constraint, a Windows quirk, a number's origin.
- **Only what is true now**: no history, no task or decision ids, no earlier implementations.
