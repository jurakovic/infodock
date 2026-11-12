# Code and architecture review, November 2026

A whole-tree review at `136a3d3` (2026-11-12), asked for outside the plan: defects first, then structure (SOLID, patterns, module boundaries). One reviewer, reading the code; the commit gate was run first and is green on Linux (`cargo fmt --check`, `cargo lint`, `cargo test`: 595 passed, `cargo check-win`).

`review-architecture.md` (at `f3297a9`, three weeks earlier) already lists the structural findings and what was left for after 1.0. This review does not repeat them: it checks that its *before 1.0* items landed (they did, S14b–S14i), reports what is new, and says which of its deferred items the new findings lean on. Windows-only behaviour was read, not run; findings that rest on it say so.

Each finding gives severity (High: wrong behaviour users will meet; Medium: wrong behaviour in a reachable corner, or a hole in a rule the design relies on; Low: polish or hardening), effort (S under half a day, M about a day) and a proposal. Nothing here is a plan item yet; nothing in the code was changed.

## Summary

The code is in good shape for a release: no `unwrap`/`expect`/`panic!` on runtime paths outside one compile-time catalog check, `unsafe` and `windows::` only under `platform/windows`, pure logic separated and tested, re-entrancy on both UI threads handled deliberately (`WindowState::dispatch`'s deferred queue, `settings/window.rs`'s borrow policy). No High finding.

Worth fixing before 1.0:

- **R1** — config listeners can run out of order, leaving the bar and Settings on a stale config (reproduced with a test, see R1).
- **R2** — an open popup's accessibility description costs O(n²) per frame in the folder's size once any UI Automation client has asked.
- **R3** — a broken `config.json` that cannot be moved aside overwrites the good backup.
- **R4** — SPEC §14.2's "a panic on the bar thread … restarts the bar thread" is not what the code does, and the panic boundary differs between entry points.

The rest is Low. On structure, the two new points are the `providers` ↔ `sampler` cycle (A1) and the config store's notification model (A2), which R1 is a symptom of.

## Defects

### R1 Medium — config listeners can run out of order; the bar and Settings can end on a stale config

Effort S. `config/store.rs:120–141`, `bar/mod.rs:154–166`, `settings/mod.rs:166–175`.

`ConfigStore::update` swaps the config under its lock, releases the lock, and then calls the listeners with `(old, new)` snapshots on the calling thread. Two threads that update at the same time (the bar thread: a sensor's state or props, a tray toggle, a moved floating bar, a reorder; the main thread: any Settings change, the welcome page) can therefore notify in the opposite order of their swaps. Every consumer keeps the snapshot it was handed last as the truth:

- `BarController`'s listener stores `new.settings` for later re-docking and queues `Command::Config(new)`; the bar thread replaces its settings, layout and state with it.
- Settings' listener posts a refill from `new.settings` and `new.layout`.

So after `A: v0→v1` and `B: v1→v2` notified as B, A, the bar runs on v1 while the store holds v2: B's change (say, the theme the user just picked) is not shown until the next config change, and `BarController.settings` docks from v1 at the next show/hide. Nothing is lost on disk, and the next change heals it.

Reproduced on Linux with a throwaway test in `store.rs` (not committed): a listener parked on thread A while thread B updates and notifies; the consumer's last snapshot is then `(locked: true, edge: Bottom)` against the store's `(true, Top)`.

The window is a few instructions wide, so this is rare in practice (it needs a bar-thread save while Settings saves). It is listed as Medium because §4.4's "one source of truth" rests on it, and the store's doc comment promises nothing about order.

**Proposal:** give each swap a revision under the lock (`Shared { config: Mutex<(u64, Config)> }`) and pass it to the listeners; `BarController` and the bar thread drop a config older than the one they hold, and Settings' posted refill reads `store.get()` when it runs instead of the captured snapshot. Alternatively hold a second, ordering mutex from the swap to the end of the listener calls: no listener today waits for another thread (`AppBar::configure` posts, Settings uses `invoke_from_event_loop`, autostart writes the registry itself), so the comment at `store.rs:133–134` ("listeners may block on the main thread (tray updates)") is out of date and should go either way.

### R2 Medium — an open popup's accessibility description is O(n²) per frame in the number of rows

Effort S. `platform/windows/uia_popup.rs:82–94` and `153–162`, `platform/accessible.rs:177–182`, `sensors/common/folder_browser.rs:1001–1018`, `platform/windows/popup.rs:431–433`. Read, not measured (Windows only).

Once the popup window has answered one `WM_GETOBJECT` (any UI Automation client: a screen reader, but also Windows' own text-input and touch-keyboard hosts, which ask windows that take the focus, as the popup does), every later popup frame:

1. builds an `AccessibleRow` for **every** item of the folder, on screen or not, cloning its path and name (`folder_browser.rs:1005–1018`);
2. clones every row id of the old and the new description into two `Vec<String>` to compare them (`accessible.rs:179–180`);
3. runs `Tree::number`, whose `retain(|id, _| popup.rows.iter().any(|r| r.id == *id))` scans the rows once per known id: n²/2 string comparisons when nothing changed.

Only then is `UiaClientsAreListening()` asked (`uia_popup.rs:164`). A frame is drawn for every row the pointer crosses, every scroll step and every slide step. §14.1's Folder Menu budget is measured with 500 items and no client (about 125,000 comparisons a frame would still pass); a folder of 5,000 items is 12.5 million a frame, 20,000 (a `WinSxS`, a photo dump) 200 million, on the bar thread.

**Proposal:** in `number`, collect the new ids into a `HashSet<&str>` once; compare ids with `iter().map(|r| &r.id).eq(..)` instead of cloning; and skip steps 1–3 while `UiaClientsAreListening()` is false, keeping only a "stale" flag so the description is built when a provider is next asked. A Windows test beside `budget_folder_menu` with 5,000 rows and the provider attached would keep it.

### R3 Medium — a broken config that cannot be moved aside overwrites the good backup

Effort S. `config/store.rs:242–254` (`load`), `85–89` (`open`), `214–228` (`write_atomic`). By reading; needs a file another process holds open, so not reproduced on Linux.

When `config.json` does not parse, `load` renames it to `config.broken-<ms>.json` and loads `config.json.bak`. If the rename fails (on Windows: another process has the file open without delete sharing, e.g. an editor or a backup tool), the error is logged and loading goes on from the backup, with `recovery` set. `open` then writes at once (`must_write`), and `write_atomic` copies the current `config.json`, still the broken one, over `config.json.bak` before renaming the new file into place; that rename fails for the same reason. Result: the broken file where the config should be, a copy of it as the backup, and the only good copy gone. The next start falls back to the first-run defaults. The notification also names a `config.broken-…` file that does not exist.

**Proposal:** `write_atomic` takes whether the file on disk is known good and skips the backup copy otherwise (after a recovery, the backup that was just loaded *is* the previous version); when the move aside failed, report the file as left in place.

### R4 Medium — the panic boundary is not what SPEC §14.2 says, and differs between entry points

Effort S for the spec and the missing guards. `SPEC.md:1594`, `platform/windows/window.rs:881–886`, `platform/windows/popup.rs:301`, `platform/windows/droptarget.rs:86–98`, `logging.rs:86–103`.

§14.2: "A panic on the bar thread is logged, releases the AppBar (panic hook) and restarts the bar thread with the current config." The code does neither: the bar's and the popup's window procedures catch the panic and carry on with the same `BarLogic` (whose state may be half-updated; `AssertUnwindSafe` asserts otherwise), and the hook's fatal cleanup runs only for the main thread. That is a defensible policy, but it is not the documented one, and `CLAUDE.md` makes the spec the source of truth.

The boundary is also uneven. Bar logic reached through the window procedure is guarded; the same logic reached through `IDropTarget` (`BarDropTarget::answer` → `WindowState::drag` → `BarLogic::on_drag` → a sensor's `drop_action`/`drop_items`) is not, nor are the hint and band procedures or the hooks (`appbar.rs:655`, `hotkeys.rs:66`, whose listener runs Settings code). A panic there unwinds into an `extern "system"` frame, which aborts the process: no log flush, no `ABM_REMOVE` (Windows then removes the appbar with the window, §5.2 step 6, so the work area does recover).

**Proposal:** decide the policy and write it into §14.2 (keep going after a caught panic is the cheaper one; a restart needs `BarLogic` rebuilt from the store). One `guard(default, || …)` helper in `platform/windows` used by every callback the OS calls, so the policy is in one place; the drop target answers `DROPEFFECT_NONE` after a caught panic.

### R5 Low — every shell icon and image waits behind one worker

Effort M. `bar/icons.rs:70–96`.

`IconLoader` reads all shell icons and image files on one thread, in request order. One path on a share that does not answer holds every later icon, the bar's and the open popup's, for the SMB timeout; the cancel set only skips requests not yet started. The providers solved the same problem with a worker per key (`providers/worker.rs`), and SPEC §4.1 gives the unreachable share as the reason the loader exists. **Proposal:** after 1.0, with `review-architecture.md` S-x2/P7 (one shared way to run a slow call off-thread): a small pool, or a second worker for paths `path_root` says are remote.

### R6 Low — forgetting one battery topic drops the reader the others share

Effort S. `providers/mod.rs:107–111`.

`Topic::Battery(i)` for every `i` and `Topic::Batteries` read through one `SlowReader` key. When any one of them loses its last subscriber (Settings closes a form with a battery list; one of two Battery sensors is removed), `forget` drops that key: its worker ends and the newest reading goes, though the other topics still use it. They recover at their next tick (a new thread, a new 200 ms wait), so nothing wrong is shown; with slow battery firmware one sample is skipped. **Proposal:** `Providers::forget` takes whether another topic of the same reader is still subscribed (the scheduler knows), or the sampler asks `scheduler` before forgetting.

### R7 Low — `t_args` substitutes into text it has already substituted

Effort S. `i18n.rs:45–47`, `59–62`.

Placeholders are replaced one argument after another on the growing string, so a value that itself contains a later argument's `{name}` is rewritten: a file called `{error}.json` in "Could not read {file}: {error}", a sensor text containing `{count}`. Cosmetic, but values come from file names and imported layouts. **Proposal:** one pass over the template that copies text and looks each `{name}` up.

### R8 Low — the release script and workflow could fail more loudly and trust less

Effort S. `publish.sh`, `.github/workflows/publish.yml:99–207`.

- `publish.sh` has no `set -euo pipefail` and checks commands one by one, so some are unchecked: `sha256=$(sha256sum … | cut …)` at line 148 writes a line with an empty hash into `checksums.txt` if `sha256sum` fails, and `cp Cargo.toml`, `mkdir -p` and `set_version`'s `sed` are not checked either. `set -euo pipefail` at the top, with the existing `if [[ ! $? -eq 0 ]]` lines dropped, covers them all.
- The job that builds the released, unsigned binaries runs third-party actions pinned by tag (`dtolnay/rust-toolchain@stable`, `Swatinem/rust-cache@v2`, `taiki-e/install-action@v2`); a moved tag would run in the release build. Pinning them by commit SHA in `publish.yml` (CI can stay on tags) is the usual hardening. Inputs are passed through `env`, permissions are `contents: read` except the release job, and the version is validated: those are right.

### Checked and fine

- Re-entrancy: handler calls during a menu's or dialog's modal loop are queued (`window.rs:405–433`); a paint meanwhile is postponed.
- `Scheduler`, `SlowReader`, `layout::fit`/`place`, `ops`: pure, with tests that match the spec's examples; no arithmetic that can underflow (`saturating_*`, `checked_sub`, clamped indices).
- `material.rs`'s undocumented `SetWindowCompositionAttribute`: the struct layouts and the `transmute_copy` of the export are right; failure is reported, not assumed.
- Quitting: `request_quit`'s watchdog covers an event loop stuck in a native modal dialog.

## Architecture

### What the previous review's items look like now

All eight *before 1.0* items (S14b–S14i) are in and hold: `platform` no longer imports `sensors`, `look/` is the neutral home of the theme and skins, the contract tests pin the Rust–Slint numbers and paths, `ConfigError` has codes, machine defaults are asked before the store's lock. The *after 1.0* list in `review-architecture.md` is still accurate; three of its items now have numbers or symptoms:

- **B1 stage 2** is the largest structural debt left. `bar/thread.rs` became nine files, but `BarLogic` is still one object with 42 fields and about 68 methods; the children share every field through `use super::*`, so the split shows where code is, not what it depends on. `sensor_env!` and `layout_ctx!` exist only because a method would borrow all of `self`: the state wants to be sub-structs (`Gesture`, `Motion`, and the shared `settings`/`theme`/`locale`/`platform` group).
- **P1** (the 40-method `Platform`), measured from the callers: sensor code uses 11 of the 40 at run time (`open` 18 times, `launch`, `reveal`, `eject`, `power`, `notify`, `play_sound`, `local_time`, `known_folder`, `volume_of`, `create_url_file`) plus four for machine defaults, and three fields of `Settings` (`locked`, `clicksWhenLocked`, `apps`); `SensorCtx` hands every sensor all of both. The split P1 proposes is the one the callers already draw.
- **S-x2/P7** (hand-rolled workers): there are now five ways to run a slow call off a UI thread (`SlowReader` per key, `IconLoader`'s single queue with a cancel set, a thread per folder listing, the eject thread, the shell's spawn-and-enter-COM). R5 is what the difference between two of them costs.

### A1 Medium — `providers` and `sampler` depend on each other, and the sampler loop cannot be tested without the real OS

Effort S–M; after 1.0, or before if a topic is added. `sampler/mod.rs:20`, `providers/*.rs` (8 files import `crate::sampler`).

The topic vocabulary (`Topic`, `Sample` and its payloads, `Change`, `Slow`, `ChangeSink`) lives in `sampler/topics.rs`; `providers` imports it to build samples, and `sampler::run` takes the concrete `Providers`. So the reader depends on the scheduler's module for its data types, and the scheduler's loop on the concrete reader: a cycle between two modules that SPEC §4.2 draws as layers, and the reason the loop's own tests (`sampler/mod.rs:200–304`) run the real platform with waits of seconds (P8 noted the waits, not the cause).

**Proposal:** `sampler` declares what it needs, `trait Source { fn sample(&mut self, &Topic) -> Result<Option<Sample>, String>; fn forget(&mut self, &Topic); fn resume(&mut self); }`, and `Providers` implements it; `Pending::start` takes `impl Source`. The loop is then tested with a scripted source and a channel, in milliseconds. If the cycle itself should go too, `topics.rs` moves to a module both import (`crate::topics`), which is a rename for every other user.

### A2 Medium — the config store pushes whole snapshots to every listener, unordered and unfiltered

Effort M; the ordering part (R1) before 1.0, the rest after. `config/store.rs:104–141`.

The store is the app's Observer, and it is the simplest one that could work: every change, whatever it touched, calls every listener with two full `Config` values on whichever thread made the change. What follows from that:

- R1 (order is the listeners' problem, and none handles it).
- A sensor's state write costs three deep clones of the config (two in `update`, one for `Command::Config`), comes back to the bar thread that made it, compares the whole layout and state, and runs a sensor sync; Settings' listener clones the settings and the layout and wakes the main loop for it even while Settings is closed (`settings/mod.rs:166–175`; C11 covers the refill, not the closed window).
- Each listener works out for itself what changed (`dock_request(old) != dock_request(new)`, `enabled != old.settings.autostart`, the bar's `layout_changed`).

**Proposal (after 1.0, with C14's `Arc<Config>`):** the store hands out `Arc<Config>` plus a revision and a small `Changed { settings, layout, state }` set computed once under the lock; listeners take what concerns them, and consumers that keep a copy keep the `Arc` and the revision. That removes the clones, makes R1's fix structural, and lets Settings ignore state writes.

### A3 Low — `platform` imports the config model

Effort S; a decision more than a change. `platform/mod.rs:32`, `platform/geometry.rs:6`, `platform/text.rs:10`, `platform/windows/{appbar,locale,mod,window}.rs`.

The OS layer takes `Edge`, `BarMode`, `ZOrder`, `FloatingRect`, `Material`, `DateLocale` and `FontStyle` from `config::model`, the serde model of `config.json`. They are plain enums and work well as shared vocabulary, but the dependency points up (the lowest layer on the persistence format), and S14c removed the same shape for `platform → sensors`. **Proposal:** either say so in SPEC §4.2 (these seven types are the vocabulary `platform` shares with the config), or move them to a small module below both; no behaviour changes either way.

## SOLID and patterns, as the code stands

| Principle | Where it holds | Where it does not (finding) |
|---|---|---|
| Single responsibility | The pure modules (`layout`, `placement`, `interact`, `focus`, `resize`, `scheduler`, `ops`, `schema`, `format`, `dates`), one job each, tested alone | `BarLogic` (B1), `Platform` and `platform/mod.rs` (P1, P3), `settings::fill` and `ui/settings.slint` (C1, C8) |
| Open/closed | Sensor types: a module, a registry line, messages; forms, picker, validation and storage follow from the schema | A new topic touches about ten places (P8); Settings pages are numbers shared by Rust and Slint (C3); `FieldKind`'s live lists (S12) |
| Liskov | `Placeholder` stands in for any sensor; fakes and the recording canvas for the OS | `Platform`'s defaults double as the stub, so a method Windows forgets to override compiles and does nothing (P1); `has_popup` must agree with `click` by convention (S10) |
| Interface segregation | `Canvas`, `TextEngine`, `IconSource`, `Dialogs`, `AppBar`, `TrayIcon` are small and whole | `Platform` 40 methods, `Sensor` 24, `BarWindow` 20, `BarHost` 18; callers use narrow slices (see P1 above) |
| Dependency inversion | The platform is injected everywhere (`platform::current()` once); the bar sees the app only as `BarHost`; `transfer` talks to `Dialogs`; sensors to `IconSource` | `sampler → Providers` (A1); `app ↔ settings`, `transfer`, `hotkey` (C7); the global `registry()` and `i18n::t` (accepted, S-x1) |

Patterns in use, all of them fitting:

- **Command with a queue and a waker** (`BarController` → `Command` → `BarLogic::drain`): the one cross-thread path into the bar, and it batches.
- **Functional core, imperative shell**: `hint::Hover` returns `Step`s, sensors return `ClickResult`/`Dirty` and are polled for state (`take_state`, `take_props`), `ops` are functions on a `Config`. This is the design's best decision; B1 stage 2 is the same idea applied to gestures and motion.
- **Ports and adapters**: `Platform`, `Canvas`, `TextEngine`, `BarWindow` with a Windows adapter, a stub and test doubles; drawing is tested on Linux because of it.
- **Registry with abstract factory** (`SensorType::create`) and **template method** (the `Sensor` trait's defaults).
- **Observer** (`ConfigStore::on_change`, `ChangeSink`): see A2.
- **Null object** (`Placeholder`, `NoIcons`, `ChangeSink::NONE`), **caches keyed by what was drawn** (`SkinCache`, `ShadowCache`, `IconLoader`).

One pattern is missing where it would pay: a **single owner for background work** (R5, P7, S-x2).

## Suggested order

Before 1.0, each small and independent: R1 (revision or ordering lock, with a regression test built like the reproduction), R2 (set lookup, no clones, skip while nobody listens), R3 (no backup copy of an unparsed file), R4 (spec text and one `guard` helper), R7, R8. R6 with the next change to `providers`.

After 1.0: A1 (before the next topic), A2 with C14 and C11, R5 with P7 and S-x2, A3 as a spec sentence, then `review-architecture.md`'s list, B1 stage 2 and P1 first.
