# Internals

How Infodock is built: the modules and what each may use, the threads and the windows they own, how system data reaches the bar, the platform layer, the sensors, the config and where it is stored, Settings, the look, and how it is built, tested and released. It describes the code as it is; a change to the code updates it in the same commit.

Infodock is one Rust crate (`infodock`, lib `infodock_lib`). The bar, its hints and popups are native Win32 windows drawn with Direct2D and DirectWrite; the Settings window is Slint. No browser engine runs at any time. Windows is the only platform with an implementation; elsewhere a stub platform lets the app open Settings and run the tests.

## Principles

- **OS calls only in `platform/`.** Everything else talks to the `Platform` trait and the bar thread's window traits, so the bar logic, the sensors, the sampler and Settings run unchanged against the fake platform in tests, on any OS.
- **Pure logic in plain functions.** Layout, placement, hit-testing, focus moves, config merging and migration, settings mapping, formatting, colours and accessibility diffs are functions over plain data, with unit tests and no OS calls.
- **Each thread owns its UI.** The bar thread owns the bar and its windows, the main thread owns Settings. Other threads reach them only through `BarController` and `slint::invoke_from_event_loop`.
- **One way into the config.** Every change, from Settings, the tray, the hotkey or the bar, goes through the config store, which saves it and tells its listeners.
- **Shared readings.** A topic is read once per tick however many sensors show it.
- **Nothing slow on the bar thread.** Readings that can block (drives, network shares, batteries, folder listings, shell icons, image files) run on worker threads.
- **Small at idle.** The bar is cheap to keep running; Settings is created when opened and dropped when closed (see [Budgets](#budgets)).

## Technology

| Part | Crates and APIs |
|---|---|
| Language | Rust stable, edition 2024 |
| Bar, hints, popups | native Win32 windows (`windows` crate); Direct2D with a software render target, DirectWrite text, WIC image decoding |
| SVG | `resvg` rasterises the built-in icons and the user's SVG images to bitmaps; images referenced by path inside an SVG are not loaded |
| Settings window | Slint 1.x: winit backend, software renderer, Fluent style, AccessKit; `.slint` files compiled by `build.rs` |
| Global hotkey | `global-hotkey` |
| Tray icon and menus | native: `Shell_NotifyIconW` registered for the bar window, `TrackPopupMenuEx` menus |
| System data | PDH, `GetSystemTimes`, `GlobalMemoryStatusEx`, `NtQuerySystemInformation`, IP Helper, `wlanapi`, `starship-battery` (batteries), `sysinfo` (cross-platform fallback); no WMI (see [Windows data sources](#windows-data-sources)) |
| Serialisation | `serde`, `serde_json`; `quick-xml` for the legacy layout import |
| Logging | `tracing`, `tracing-appender` (daily files) |
| Windows resources | `embed-resource`: the exe icon, the application manifest, version information |
| Packaging | `cargo-packager`: an NSIS per-user installer; the portable exe needs nothing else |

`Cargo.toml` lists the `windows` crate's features and says what each dependency is for.

## Folders

```
src/                 the Rust sources (see Modules)
ui/                  the Settings window in Slint: app.slint, i18n.slint, settings.slint, dropdown.slint; ui/icons/ (the navigation's Fluent icons, the buttons' and the dropdown's)
i18n/                message catalogs: en.json, embedded in the binary
assets/              embedded artwork: icons/*.svg (the sensors' icons), logo.svg, sounds/chime.wav, third-party.tsv (the About page's crate list)
icons/               the app and tray icons, the README logos and installer/ images, each SVG beside its renders
examples/icons.rs    renders the .ico, PNG and BMP files of icons/ from their SVGs
resources/           the Windows application manifest
tests/fixtures/      legacy XML layouts (legacy/), a skin pack (skins/)
.github/workflows/   ci.yml, publish.yml
.github/scripts/     CI's memory, time, hotkey, Show desktop, drive release and installer checks; the third-party list
.cargo/config.toml   the `lint` and `check-win` aliases
build.rs             compiles the Slint UI and the Windows resources
publish.sh           versions, builds, packages and releases
```

## Modules

```
src/
  main.rs, lib.rs   start-up and shutdown: the capability probe, the bar thread, the sampler, the main event loop (Settings, hotkey)
  app.rs            App: the state all threads share; AppHost, the bar's BarHost; bar visibility; quitting
  hotkey.rs         the global hotkey (main thread)
  autostart.rs      starting at sign-in: the `Run` entry follows `settings.autostart`
  transfer.rs       import and export as the user runs them: pickers, questions, the report (main thread)
  logging.rs        the log file and the panic hook
  i18n.rs           message catalogs, plural rules, t()
  test_util.rs      what the tests share: the fake platform
  settings/         the Settings window's glue: mod.rs (opening, filling and saving the Slint state, the plain pages), window.rs (the open window and the dialogs modal to it), model.rs (pure mapping: pages, routes, choices, patches), appearance.rs, applications.rs, layout.rs (Layout manager), picker.rs, properties.rs and form.rs (a sensor's properties), welcome.rs, about.rs; accessibility.rs, dropdown.rs, screenshots.rs and contract.rs are tests
  config/           mod.rs (ConfigError), model.rs (the typed config), store.rs (load, save, atomic write, debounce, listeners), migrate.rs, patch.rs (settings patches), exchange.rs (export and import files), legacy.rs (legacy XML reading)
  sampler/          mod.rs (the sampler thread, ChangeSink), scheduler.rs (subscriptions, intervals, backoff), topics.rs (topics, changes, samples)
  providers/        one provider per topic kind: cpu.rs, memory.rs, swap.rs, disk.rs, net.rs, battery.rs, uptime.rs; worker.rs (readings that can stall, one worker per key)
  bar/              the bar's platform-neutral logic, on the bar thread
    mod.rs          BarController (the cross-thread handle), BarHost (what the bar asks of the app)
    thread/         BarLogic: mod.rs (state, event dispatch, config, tray) and a module per job: pointer, hover, menu, popups, sensors, external_drop, keyboard, frame; tests/ split the same way
    sensors.rs      the sensor instances: creation, samples, measuring, placing, drawing, error state
    layout.rs       zone layout, pure
    interact.rs     presses, clicks, the Lock rules, the reorder drag, pure
    focus.rs        the keyboard on the bar, pure
    resize.rs       resizing a floating bar by its ends, pure
    placement.rs    where hints and popups go, pure
    drop.rs         drops from other apps: what each item is
    hint.rs, popup.rs   the hover hint and the interactive popup host
    menus.rs, tray.rs   the context menus and the tray menu
    icons.rs        shell icons and image files, read on a worker
    paint.rs        the bar's background; the screenshot and budget tests
    shadow.rs       the bar's shadow band
  look/             how Infodock looks, shared by the bar, the sensors and Settings
    theme.rs        the resolved Theme
    skins/          mod.rs (the configured skin, drawn and tinted, or its fallback), builtin.rs, pack.rs and raster.rs (skin packs), zip.rs, install.rs
    anim.rs         fades and easing
  sensors/
    mod.rs          the SensorType and Sensor traits, SensorCtx, LayoutCtx, DrawCtx
    schema.rs       property schemas: validation, defaults, storage rules
    registry.rs     all sensor types; creating the sensor for a layout entry
    ops.rs, import.rs   the operations Settings uses; imported sensors checked by their types
    legacy.rs       legacy properties to props
    format.rs, color.rs, dates.rs   formatting, colours and the dynamic gradient, dates in the locale's patterns
    popup.rs, drop.rs   the Popup trait; the sensor a drop makes, drops onto sensors
    placeholder.rs  unknown and unsupported types
    test_support.rs the sensor tests' Harness
    common/         building blocks: two_line, meter, graph, dual_graph, history, icon, icon_label; the popups folder_browser and eject_list; eject (eject notifications)
    <type>.rs       one module per sensor type or family (see Sensors)
  platform/
    mod.rs          the Platform trait and its reading types; BarWindow, BarHandler, UiEvent, AppBar; dialogs; app folders; the single instance
    canvas.rs, text.rs, bitmap.rs   colours, shapes and the Canvas trait; fonts, TextLayout and the TextEngine trait; Bitmap, format sniffing, SVG, tint
    recording.rs    the recording canvas and fixed-metrics text engine (tests on every OS, the stub's renderer)
    geometry.rs     pure placement math: docking strips, floating rects, monitor selection
    keys.rs         hotkeys as Settings records them, and their virtual keys, pure
    locale.rs, time.rs   the locale, calendar dates and local time as plain data
    accessible.rs   the bar as screen readers see it, and the events between two snapshots, pure
    drag.rs, notify.rs   drag and drop items and effects; a notification's content
    drives.rs       the drive gate: no reading starts on a drive while Windows takes it
    guard.rs        catching panics in callbacks the OS calls
    windows/        the Windows implementation (below)
    stub.rs         the portable fallback for other OSes
```

`platform/windows/`: `mod.rs` (WindowsPlatform: capabilities, appearance, local time, sound), `window.rs` (the bar thread, its window class, message loop, input to events), `appbar.rs`, `band.rs` (the shadow band window), `hint.rs` and `popup.rs` (layered windows), `tray.rs`, `menu.rs`, `dialog.rs` (`TaskDialogIndirect`, pickers, the colour dialog), `droptarget.rs` (`IDropTarget`), `uia.rs` and `uia_popup.rs` (UI Automation providers), `render.rs` (the Direct2D canvas), `text.rs` (DirectWrite), `image.rs` (WIC), `monitors.rs`, `instance.rs` (single-instance lock, data folders), `com.rs`, `close.rs` (close requests to any of the app's windows), `shell.rs` (`ShellExecuteEx`, the Recycle Bin, pickers), `fileops.rs` (`IFileOperation`, `.lnk` and `.url`), `icons.rs` (shell icons), `autostart.rs`, `toast.rs`, `power.rs`, `eject.rs`, `device.rs` (volume devices), `drives.rs` (the drive watch), `hotkeys.rs` (the keyboard hook while a hotkey is recorded), `material.rs` (Acrylic and Mica), and the readings `cpu.rs` (PDH), `memory.rs`, `disk.rs`, `net.rs`, `battery.rs`, `wlan.rs`, `locale.rs` (NLS).

**The dependency rule.** Dependencies run one way, tests aside:

- `app`, `settings` and the other main-thread modules (`hotkey`, `transfer`) use everything below them; Settings uses the bar only for skin previews (`bar::skin_preview`).
- `bar` uses `sensors`, `look`, `sampler`, `platform` and `config`.
- `sensors` use `look`, `sampler`, `platform` and `config`.
- `sampler` and `providers` use each other (the sampler drives the providers, which fill its sample types) and `platform`.
- `look` uses `platform` and `config`.
- `platform` uses only `config`'s model types, `i18n` and `logging`.

Nothing below them names `settings` or `app`. The modules are private to the crate, so the compiler reports unused items (on Windows builds; elsewhere most of the bar is unreachable).

## Threads and windows

One process, single instance.

| Window | Thread | Kind | Purpose |
|---|---|---|---|
| `bar` | bar | `WS_POPUP`, `WS_EX_TOOLWINDOW` (no taskbar button, no Alt+Tab), `WS_EX_NOACTIVATE` (clicks do not take the focus); per-pixel alpha composited by DWM, with an optional material | the bar |
| `hint` | bar | layered (`UpdateLayeredWindow`), topmost, non-activating, click-through, tool window; created on first use and reused | hover hints, the calendar |
| `popup` | bar | layered, topmost, tool window, focusable; hides when it loses activation; created on first use and reused | the Folder Menu, Start Menu and Eject USB popups |
| `band` | bar | layered, click-through, non-activating, tool window, owned by the bar window and kept right above it; created when first needed | a skin's shadow band or the bar's shadow, beside the bar |
| `drives` | drives | `WS_POPUP`, `WS_EX_TOOLWINDOW`, never shown; top-level, so volume broadcasts reach it; created when a drive is first read | the drive watch: the local drives' device events; the Recycle Bin's readings, handed over as jobs |
| `settings` | main | Slint, a normal decorated window, opened on a route, created on demand and dropped on close | Settings, a sensor's properties, the picker, the Layout manager, the welcome page, Help, About |

The **tray icon** is registered for the bar window, on the bar thread; its menu is built from the current state each time it opens.

Threads:

- **main**: Slint's event loop, the Settings window and the global hotkey. Other threads reach it with `slint::invoke_from_event_loop`.
- **bar thread**: a COM single-threaded apartment (`OleInitialize`, for drag and drop) with its own message loop. It owns the bar, hint, popup and band windows, the tray icon, the sensor instances, layout, rendering, menus and native dialogs. Other threads send it commands through `BarController`, which queues the command and posts a wake-up message to the bar window.
- **sampler thread**: reads the providers and delivers samples to the bar thread's queue (see [Data flow](#data-flow)).
- **drives thread**: the drive watch, a COM single-threaded apartment with its own message loop, started when a drive is first read. It answers Windows when it removes, locks or dismounts a drive, at once, whatever the bar thread does, and reads the Recycle Bin for the provider's worker, as jobs it runs between its answers (see [Windows data sources](#windows-data-sources) for an answer that comes inside one).
- **workers**: slow work, with results back on the bar thread's queue: the providers' per-key workers (drives, paths, batteries, the Recycle Bin, which the drives thread reads), the bar's `icons` worker (shell icons and the sensors' image files, kept by path and size), the popups' folder listings, file operations, and the config writer. The bar thread never reads a file a user chose, so a network share that does not answer holds nothing up; sensors measure again when an icon or image arrives. The classic skin's custom image is the exception: it is read where the skin is drawn.

**Single instance**: a named mutex (`Local\app.infodock.desktop`). A second launch finds the running bar window by its class, lets it take the foreground and posts it a registered activation message, which carries no data. The running instance shows the bar and opens Settings on General, or brings an open Settings window to the front on the page it shows; the second launch exits.

**Start-up** (`lib.rs`): the capability probe starts on a thread of its own as soon as the process starts; the config is loaded, the sampler and the bar thread start, and the bar thread creates its window, the tray icon and one sensor per layout entry. The bar does not wait for the probe: until `Platform::probed_capabilities` answers, it shows only the sensors whose type needs no capability, checking every 25 ms, and the others join once the answer is there. The log records when each step was done, with the bar's first frame (see [Budgets](#budgets)).

**Quitting** (`app.rs`): the tray's and the bar menu's *Quit* and a close request end the main event loop (`App::request_quit`), once; the main thread then keeps Settings' place and runs `App::shutdown`, which releases the AppBar and the tray icon, stops the sampler, writes pending config changes and closes the log. If the loop has not ended 5 s later, the process ends after the same cleanup. A close request is a `WM_CLOSE` to the bar window, or one posted to any other window of the main, bar or drives thread: `taskkill` without `/f` posts it to one top-level window of the process, found by z-order, which with the bar kept behind other windows is a hidden one (winit's, the hotkey's, the drive watch's) or Settings. Each of those threads hooks the messages it takes from its queue (`WH_GETMESSAGE`, `platform/windows/close.rs`) and hands a posted `WM_CLOSE` to the bar window instead, so the window it was posted to gets nothing. The bar window quits on it itself (`ExitRequests::quit`, `BarHost::quit`), bypassing the bar logic, so a menu or a confirmation the bar shows does not hold it up. A window's own close button sends its `WM_CLOSE` (`SC_CLOSE`), which passes no hook: Settings' closes Settings. When the session ends (sign-out, shutdown) or Restart Manager closes the app, Windows sends `WM_ENDSESSION` and may end the process once the windows have answered: the bar window's runs `App::shutdown` before it answers, on the bar thread and whatever the bar logic is doing (`BarHost::session_ending`), then quits as above; Settings' place is not kept then.

## Data flow

1. For each sensor the bar thread asks its type for **topic requests** (`Sensor::topics`, e.g. `Topic::Cpu` every 1 s), once when it creates the sensor, and passes on only subscriptions that changed (keyed by sensor id; an empty list unsubscribes).
2. The **scheduler** (`sampler/scheduler.rs`) keeps the subscribers per topic and reads each topic at the shortest interval its subscribers ask for (at least 100 ms), on one grid counted from its first sample, so topics with whole-second intervals are read and drawn together. Providers are shared: ten CPU sensors cost one CPU reading per tick.
3. Every sample goes to the bar thread's queue. The bar hands a sample to a sensor only once the sensor's own interval has passed since the last one it took (a quarter of the interval early, at most 500 ms, still counts), so a 5 s graph keeps its time scale beside a 1 s meter on the same topic. The scheduler keeps the newest sample of each topic, and every subscribe returns it to that subscriber alone, so a sensor gets data at once when it joins a topic; a sensor re-created under the same id (its props changed, a preview) is subscribed again for that catch-up.
4. **Subscribers** are sensors by id, or the Settings window, which takes the drive and battery lists while a form needs them (`sampler::Subscriber`).
5. **Changes** (`Change`: devices, the Recycle Bin, power, Wi-Fi, an answer that came late) make the topics they concern due at once (`Scheduler::refresh`). OS notifications reach the bar window, which tells the sampler; a sensor reports what its own action changed through `SensorCtx::changes`, a `ChangeSink` it can hand to the thread that finishes the action; providers report notifications that arrive on other threads the same way. A drive arriving or going refreshes `volumes`, every `disk/…` and every `path/…`. A reading made because of a change is *pushed* and reaches every subscriber at once.
6. **Slow readings.** Disks, volumes, paths, batteries and the Recycle Bin are read on worker threads (`providers/worker.rs`), one per key, so a network drive whose server is away holds up nothing else; the Recycle Bin's worker hands the reading to the drives thread and waits for it (see [Windows data sources](#windows-data-sources)). The sampler waits 200 ms for an answer and does not wait again for a key still being read from an earlier tick; an answer that comes later reports `Change::Answered`, so its topics are read again at once.
7. **Failures** keep the previous sample and are retried at the next tick; the third in a row becomes an error sample (see [Errors and logging](#errors-and-logging)).
8. A topic nobody subscribes to any more is reported to the providers, which drop its baseline, unless a topic still subscribed shares its reader (`providers::share_reader`: every battery topic and Settings' battery list read the power status through one).
9. **Resume** (`PBT_APMRESUMEAUTOMATIC`) lets the providers take new baselines and reads every topic at once; resume and time changes (`WM_TIMECHANGE`) also reach every sensor (`Sensor::on_clock_change`), so clocks read the time again and move their wake-ups.
10. **Wake-ups.** Sensors that need no system data (Text, Image, Spacer, Date, Time) and animations ask for a wake-up time instead (`Sensor::wake_at`); the bar thread keeps one timer for the earliest of them. Clocks align their wake-ups to whole seconds and minutes of the local time (`Platform::local_time`), 20 ms past the boundary.
11. **Frames.** A sample or wake-up that changes what a sensor shows marks it dirty (`Dirty`). The bar thread measures the dirty sensors again, runs the layout again only if a width changed, and draws at most one frame per 16 ms. An open hint is rebuilt only when its sensor changed or moved.
12. **History** (graphs) lives in the sensor instance, as a ring buffer of raw values; it is not stored.
13. **Config changes** go through the store, which saves them and calls its listeners: the bar thread re-creates changed sensors (by id) and lays out again; an open Settings window fills its controls again. Each change takes the next revision under the store's lock and the listeners run after the lock, on the thread that made the change, so two threads' changes can reach a listener in the opposite order: a listener keeps the newest revision it was handed (`store::Newest`).

## Platform layer

`Platform` (`platform/mod.rs`) is the set of thread-safe OS services (`Send + Sync`), passed around as `Arc<dyn Platform>`; `platform::current()` is called once, at start-up. Its methods fall into groups:

| Group | Methods | Used by |
|---|---|---|
| Machine and session | `capabilities`, `probed_capabilities`, `monitors`, `appearance`, `locale`, `local_time`, `font_families` | all |
| Readings | `uptime`, `logical_processors`, `cpu_reader`, `memory`, `page_files`, `volumes`, `disk_space`, `net_interfaces`, `net_interface`, `power_status`, `wifi_reader`, `recycle_bin`, `path_kind`, `list_dir`, `shell_icon` | providers, workers, Settings' lists; never the bar thread |
| Shell | `open`, `launch`, `reveal`, `known_folder`, `compare_names`, `volume_of`, `file_operation`, `recycle`, `create_url_file`, `empty_recycle_bin` | sensors |
| Power, devices, notifications, sound | `power`, `eject`, `notify`, `play_sound` | sensors |
| System | `autostart`, `set_autostart`, `decode_code_page`, `process_memory` | Settings, the app, the log |

Only `capabilities`, `monitors`, `appearance` and `uptime` are required; every other method has a default, which is what other OSes get (`stub.rs` implements just the four). Windows overrides all of them except three that are portable: `open` (which calls `launch`), `volume_of` (by the path's form) and `create_url_file` (plain file I/O).

`Capabilities` says what this machine has: `appbar`, `recycle_bin`, `start_menu`, `wifi`, `battery`, `power_modes`, `eject`, `per_core_cpu`. On Windows they are probed once per run, on a thread of their own, the slow probes side by side: `recycle_bin` when the shell knows the Recycle Bin folder (`SHGetKnownFolderIDList`), `start_menu` when the per-user Programs folder exists, `wifi` when the WLAN service answers and has a wireless interface (`wlanapi.dll` is loaded only for that), `battery` unless `GetSystemPowerStatus` says there is no system battery, `eject` always, `per_core_cpu` when the per-processor `% Processor Utility` counter can be added.

Conventions, stated on the trait:

- **Blocking.** A reading may wait for slow hardware or a network drive, so none runs on the bar thread: the sampler reads the CPU, memory, page files, network and Wi-Fi itself, and volumes, disk space, paths, batteries and the Recycle Bin on its workers (the Recycle Bin's worker hands the reading to the drives thread); folder listings and shell icons are read on the bar's workers. Settings reads volumes (machine defaults), network adapters and fonts on the main thread. One exception: a drop onto the bar reads `path_kind` of each dropped item on the bar thread, once per drop and never per drag-over, since the sensors it creates depend on it. Actions return at once and work in the background (a short-lived thread with COM, or the shell's own).
- **Absence and failure.** A reading that can fail returns `Result<_, String>` with the OS's message (memory, page files, folder listings, power status, the Recycle Bin, the CPU and Wi-Fi readers). Disk space, volumes, network interfaces and `path_kind` fold a failure into absence (`None`, an empty list, `Missing`), which sensors show as missing or "n/a"; so does the process's own memory. Typed errors (`EjectError`, `ImageError`) are for callers that act on the reason.

**Bar-thread services** are not `Send` and not part of `Platform`. `start_bar_thread(make_handler, exit)` starts the bar thread and returns its `AppBar`, tray icon and waker; the bar window passes a close request and the session's end to `exit` (`ExitRequests`) itself, bypassing the handler, which may be busy with a menu or a confirmation (see [Threads and windows](#threads-and-windows)):

- `BarWindow`: the bar logic's view of the native window: invalidate, material, tray icon, native menus at the pointer or beside the focused sensor, one-shot timers by `TimerId`, the hint and popup windows (shown at a physical rect with what a closure paints, faded, updated in place), pointer capture and cursors, the keyboard focus, confirmations, moving and resizing a floating bar, and the slide of the rise animation.
- `BarHandler`: what the bar logic implements. Input arrives as platform-neutral `UiEvent`s (pointer, wheel, key, timer, popup input, focus, UI Automation requests, and the OS broadcasts the bar cares about: resume, time change, regional format, device and power changes, appearance), drags from other apps as `DragEvent`s. The handler paints the bar into a canvas and, after each frame, describes the shadow band (`band`) and the accessible tree (`accessible`).
- `AppBar`: attached to the bar window when the bar thread starts; `configure(DockRequest)` (repeating the current request is cheap), `raise`, `release` from any thread.
- `TrayIcon`: shown, updated or removed through the bar window, removable from any thread on exit.
- `Dialogs`: open and save pickers, a question with command links, a message, the colour dialog. `NativeDialogs` shows them modal to an `OwnerWindow` on the calling thread. Settings has one way to them, `settings::dialogs()`, modal to its window while it is open.

**Drawing.** `Canvas` draws in logical px at the window's scale: filled and stroked rectangles, rounded rectangles, lines and polygons, linear gradients, text layouts, bitmaps with opacity, clips and opacity layers (layers still open at the end of a frame are closed). Text is laid out by a `TextEngine`, separate from the canvas, because sensors measure outside drawing; a `TextLayout` carries its size and first baseline. A font family that is not installed falls back to Segoe UI.

On Windows the canvas is Direct2D with a **software** render target drawing into a premultiplied 32-bit bitmap, with DirectWrite text in grayscale antialiasing (the background is transparent), so no GPU driver is loaded into the process. The bar copies the bitmap to its DWM-composited window; hint and popup use `UpdateLayeredWindow`; `render_offscreen` returns a `Bitmap` (skin previews, screenshots). The recording canvas (`recording.rs`) records draw calls with a fixed-metrics text engine, so layout and drawing are tested on any OS.

**Bitmaps** are premultiplied RGBA in shared memory, so workers can decode them. PNG, JPEG, GIF, BMP and ICO are decoded with WIC's built-in decoders (`GUID_VendorMicrosoftBuiltIn`), recognised by their first bytes, so third-party codecs never see Infodock's files; SVG goes through `resvg`. Decoding to a requested height scales with high-quality filtering (an `.ico` picks its closest frame). Limits keep a small file from asking for gigabytes: files up to 32 MB; results up to 8192 px a side; raster sources up to 64 megapixels; an `.svgz` inflated to at most 32 MB; images embedded in an SVG skipped above 16 megapixels. The renderer uploads each bitmap once and keeps it while the bitmap lives. Bitmaps are drawn from whole physical px, so they stay sharp at 125 % and 150 %.

**Shell icons** (`Platform::shell_icon`, `platform/windows/icons.rs`): `IShellItemImageFactory::GetImage(SIIGBF_ICONONLY)` at the physical size, read with `GetDIBits`; a pixel whose colour exceeds its alpha marks a straight-alpha icon, and an icon without any alpha is opaque. Shell calls run in a single-threaded COM apartment entered once per thread (`com.rs`), with critical-error dialogs off. Sensors ask for icons while they measure (`LayoutCtx::icons`), get *loading* until the icon arrives on the `icons` worker, and are measured again when it does. While Windows takes the path's drive, `shell_icon` says busy rather than giving no icon (see [Windows data sources](#windows-data-sources)): the answer is not kept, and every sensor measures again at the bar's next paint, which asks for the icon again; the answer itself paints nothing, and nothing measures again because of it before that paint. The sensors' icons are kept; the popups' are an LRU of 256.

**Launching** (`Platform::open`, `Platform::launch`): `ShellExecuteExW` on a short-lived thread with COM, with target, parameters, working folder and the `runas` verb as separate fields; a declined elevation prompt is only logged.

**Autostart** (`platform/windows/autostart.rs`): the value `name` of `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` and Task Manager's mark for it under `…\Explorer\StartupApproved\Run`; a change enables (writes, clears the mark), updates (writes, keeps the mark) or removes (both). `autostart.rs` at the top keeps the value `Infodock` in step with `settings.autostart`.

**Notifications** (`platform/notify.rs`, `platform/windows/toast.rs`): a `ToastGeneric` toast with two text lines through `CreateToastNotifierWithId`, under the AppUserModelID `app.infodock.desktop`, on a short-lived thread.

**Panics in callbacks** (`platform/guard.rs`): a panic in a callback the OS calls (the window procedures, the drop target, the foreground and keyboard hooks, the close-request message hook, the Wi-Fi notification) is caught there, since unwinding into the OS would abort; the OS gets the answer it would have without us.

**The stub** (`stub.rs`): there is no bar window; the bar thread runs the handler for queued work, and the recording canvas is the renderer.

## The bar

The bar logic (`bar/thread/`) is a `BarHandler`: `BarLogic` holds the state and dispatches events to a module per job (pointer, hover, menu, popups, sensors, external drops, keyboard, frame). Its pure parts (`layout`, `interact`, `focus`, `resize`, `placement`) are plain functions with their own tests; the whole logic is tested end to end with a fake window and host (`bar/thread/tests/`).

### Placement and the AppBar

Coordinates are logical (DIP) in layout and drawing; the platform converts to physical px with the monitor's scale (see [DPI and monitors](#dpi-and-monitors)). `platform/geometry.rs` computes the docking strip, the floating rect and the monitor (`select_monitor`: by device name, falling back to the friendly name and resolution, then the primary).

- **Docked**, with `reserveSpace`: the bar window itself is registered as an appbar (`appbar.rs`: `ABM_NEW` with a private callback message, `ABM_QUERYPOS`, `ABM_SETPOS`, then the window moved to the result). It negotiates again on `ABN_POSCHANGED`, `ABN_STATECHANGE`, `WM_DISPLAYCHANGE`, `WM_DPICHANGED`, `WM_SETTINGCHANGE` (`SPI_SETWORKAREA`) and after edge, monitor or height changes, and registers again on `TaskbarCreated` (Explorer restarted). The window procedure forwards `WM_ACTIVATE` and `WM_WINDOWPOSCHANGED` to the shell, and keeps other code (Aero Snap, DPI-change resizes) from moving the bar or lifting it out of its z-order. A message-only window would not do: it receives no broadcasts, and an appbar registered under another window loses the shell's z-order coupling. `ABM_REMOVE` runs on exit and in the fatal cleanups; Windows removes the appbar with the window when the process is killed.
- **Docked** without `reserveSpace`: placed at the edge without registering.
- **Floating**: kept inside the work area of the monitor holding the largest share of it; 800 px wide and centred 12 px from the edge until first moved or sized. The window follows the pointer while dragged and while an end is dragged (`bar/resize.rs`: the ends' hit-test and the rect from the pointer's screen x; `BarWindow::resize_to`); the rect is stored on release.

### Z-order, Show desktop, full screen

`zOrder` is `topmost`, `normal` or `bottom`. A foreground hook (`EVENT_SYSTEM_FOREGROUND`, and `EVENT_SYSTEM_MINIMIZEEND` for windows restored from the taskbar, which get the foreground while still minimised) tells the bar thread when the desktop (`Progman`, `WorkerW`) comes to the front. With `overDesktop` on, a bar that is not topmost is made topmost while the desktop is in front, and when another window comes in front goes back behind (`HWND_NOTOPMOST`, then `HWND_BOTTOM`) or right behind that window. Only windows that can cover the bar count: the bar's own windows, the taskbars and windows that are not shown (hidden, minimised, cloaked) are passed over; the foreground is read when the hook's message is handled, and again every 250 ms for 2 s after either event. Peek is excluded with `DWMWA_EXCLUDED_FROM_PEEK`, set when the setting changes. A topmost bar that loses that place takes it back, at most once a second, with `HWND_NOTOPMOST` first when `HWND_TOPMOST` does not take; each change is logged at debug level. The shadow band stays in the bar's layer, since Windows takes a topmost owner down with an owned window that stops being topmost.

`ABN_FULLSCREENAPP` for the bar's monitor drops topmost and hides the hint and popup while `hideOnFullscreen` is on.

### Layout

`bar/layout.rs`, pure: `fit` then `place`, from the widths each sensor measures (`Sensor::measure`) and the minimum it can be cut to (`min_width`). Each zone's array is in visual left-to-right order.

```
pad = 7, gap = settings.sensorSpacing
left:   x = pad; each sensor placed at x, then x += width + gap
center: total = Σ(width + gap); x = round((barWidth − total) / 2),
        clamped between the end of left + gap and the start of right − gap − width of center
right:  x = barWidth − pad; from the last: x −= width, placed, x −= gap
```

The centre total includes one trailing gap. When the zones do not fit side by side, sensors whose text can be cut give way, the centre zone first, then left, then right, in proportion to what each can give, in whole px; a sensor given less than it measured is told so (`fit`) and lays out its text again with that width (DirectWrite trimming with an ellipsis). What still does not fit overlaps, with the centre midway between its neighbours. Each sensor draws in the strip `bar.sensorPadding` leaves above and below (`Theme::sensor_top`, `sensor_height`). Values use tabular numerals and reserved widths, so changing numbers do not move the bar.

### Input

- **Pointer** (`bar/interact.rs`, `bar/thread/pointer.rs`): presses, clicks and the Lock rules (`clicksWhenLocked`), the reorder drag (4 px threshold, the placer line, the move index counted with the sensor already removed), moving and resizing a floating bar. A sensor's click returns a `ClickResult`: nothing, handled, a confirmation the bar asks, or a popup the bar shows.
- **Hover** (`bar/thread/hover.rs`, `bar/hint.rs`): the hint after its show delay; `HintContent` is text (lines split at `\n`, cells at `\t`) or rich (a sensor draws it, e.g. the calendar).
- **Menus** (`bar/menus.rs`, `bar/thread/menu.rs`, `bar/tray.rs`): the bar's, each sensor's (its `menu()` items added) and the tray's, as native menus.
- **Keyboard** (`bar/focus.rs`, pure; `bar/thread/keyboard.rs`): see [Accessibility](#accessibility).
- **Drops from other apps** (`bar/drop.rs`, `bar/thread/external_drop.rs`, `platform/windows/droptarget.rs`): an `IDropTarget` registered on the bar window reads `CF_HDROP` paths and URLs. Over a sensor that takes the items (`Sensor::drop_action`), it is darkened and a floating hint says what would happen; elsewhere the placer shows where new sensors would go (`sensors/drop.rs` makes them: a Folder Menu for a folder, a Disk for a drive, a Shortcut for a file or URL).

### Hints and popups

`bar/placement.rs` (pure) puts a hint or popup beside the bar, away from the nearer screen edge for a floating bar, within the monitor. A popup (`sensors/popup.rs`, `bar/popup.rs`, `platform/windows/popup.rs`) is a `Box<dyn Popup>` a click returned: it lays itself out in the room beside the bar, draws into the popup window and gets its input as `PopupInput`; folder listings and icons arrive from workers.

### Animation, shadow, material

`look/anim.rs`: fades are values 0–1 with cubic easing that a reversal resumes from where they are. The bar's frame timer (16 ms) runs only while one moves; sensors animate through their wake-ups. `animations: auto` follows the OS *Animation effects* (`SPI_GETCLIENTAREAANIMATION`, read again on `WM_SETTINGCHANGE`); without animations every transition jumps to its end.

- **Rise**: the bar thread moves the window off its place (`BarWindow::slide`) and paints its content in an opacity layer; a floating bar only fades. When a docked bar's edge changes, the bar thread slides it out, then asks for the redock (`BarHost::redock`) and rises at the new edge once it is placed.
- **Shadow band** (`bar/shadow.rs`): black rows over the bar's width, 6 logical px deep in whole physical px, each row's alpha 18 % × (1 − d / depth)². It, or a skin's own band, is handed to the platform after each frame (`BarHandler::band`); the `band` window shows it beside the bar on the side facing the screen centre, moves with the bar window and hides while the bar is hidden or off its place. The bar window itself stays as tall as the bar.
- **Material** (`platform/windows/material.rs`): Acrylic through the accent policy (`SetWindowCompositionAttribute`, `ACCENT_ENABLE_ACRYLICBLURBEHIND`, almost untinted), since Windows 11's system backdrop draws a solid fallback behind a window that is not active, and the bar never is; Mica as the system backdrop (`DWMWA_SYSTEMBACKDROP_TYPE`). The window's dark-mode attribute follows the bar's background (`Theme::dark_backdrop`), so Mica is dark under a dark bar. Without transparency effects there is no material, and the background opacity still applies.

## Sensors

`sensors/mod.rs` holds the contract.

- **`SensorType`**: one static, thread-safe value per type, registered in `sensors/registry.rs`: its id, category, name and description (catalog keys), legacy XML types and keys (`legacy_types`, `legacy_keys`, `convert_legacy`), required capabilities (`requires`), property schema (`schema`), machine-derived defaults (`machine_defaults`), `create`, `summary` (tells instances apart in lists) and `picker_note`.
- **`Sensor`**: a placed instance, on the bar thread, re-created when its props change unless it takes them in place (`reconfigure`). Layout and drawing: `measure`, `min_width`, `fit`, `draw`, `inactive`. Data and time: `topics`, `on_sample`, `wake_at`, `on_wake`, `on_clock_change`, `take_state`. Interaction: `on_hover`, `hint`, `click`, `confirmed`, `has_popup`, `clickable`, `menu`, `on_menu`, `drop_action`, `drop_items`, `take_props`. Accessibility: `accessible_value`, `focusable`.
- **Contexts**, built by the bar thread for each call: `SensorCtx` (settings, Lock and Alt state, whether to animate, the locale and its `Numbers`, the platform, the `ChangeSink`), `LayoutCtx` (the theme, the text engine, the scale, the locale, the icon source), `DrawCtx` (the theme, the sensor's rect in logical bar coordinates).

Every method but the identity ones, `create`, `measure` and `draw` has a default, so simple types stay small. Each type deserialises a typed props struct from its schema's defaults overlaid with the stored props; the schema is the only source of static defaults. Sensors act on the bar only through what they return (`Dirty`, `ClickResult`, the state and props the bar takes from them and saves); everything else goes through the platform or the `ChangeSink`.

### Property schemas

`sensors/schema.rs`: a type's `schema()` is an ordered list of `PropField`s, `{ key, type, label, description?, default, machineDefault?, type options, visibleIf?, required? }`. It drives the Settings form, validation, defaults and what is stored.

| Field type | Stored as |
|---|---|
| `text` (option `emptyIsDefault`), `file`, `folder`, `font` (`""` follows the global sensor font) | string |
| `bool` | boolean |
| `select` | an option id, or a number for numeric options |
| `slider` (`min`, `max`, `step`, `unit`), `interval` (seconds), `opacity` (0–100) | number |
| `color` | `#RRGGBB` |
| `colorMode` | `{ mode, color }`: `standard`, `custom` or `dynamic` |
| `fontStyle` | `normal`, `semibold`, `bold`, `italic`, `semiboldItalic`, `boldItalic` |
| `drive` | `X:\`, listed from the `volumes` topic |
| `netAdapter` | `{ id, name }`, listed from `Platform::net_interfaces`, or Automatic |
| `battery` | an index, listed from the `batteries` topic |
| `cpuSeries` | `total`, `user`, `system` or `core<i>` |
| `powerMode` | a mode id of `capabilities.power_modes` |
| `hidden` | any internal value (e.g. a popup's size) |

`visibleIf` (`{ key, in: [values] }`) and `required` (`{ message, unless? }`) are declarative, so Settings evaluates them. Validation normalises each value for its type: colours to upper-case `#RRGGBB`, numbers clamped to their range, intervals to whole seconds, whole-step sliders to whole numbers, drives to `X:\`. A value of the wrong type is ignored on load (the field takes its default, and it is logged) and refused by an update with the code `invalid`. A machine-derived field (`machineDefault`, e.g. `drive`, the Time's `use24h`) takes its default from `machine_defaults` and is always stored (see [Config](#config)).

### Building blocks

`sensors/common/`:

- `two_line.rs`: the label line above the value line, both in the sensor font, each baseline 1.5 px above its row's bottom, tabular digits.
- `meter.rs`: a 4 px bar beside a two-line block, the fill from the bottom.
- `graph.rs`: one sample per logical px column, newest at the right, scrolling left; layers background, grid (5 px cells) and columns, transparent layers not drawn; column edges snapped to whole physical px.
- `dual_graph.rs`: two halves as tall in whole physical px with the gap taking the rest; the top series grows up from the middle, the bottom down from it or up from the bottom edge.
- `history.rs`: a fixed-capacity ring buffer of `f32`; a graph whose width changes is created anew.
- `icon.rs`: every icon a sensor shows: the user's image if set and readable, else a shell icon or a built-in SVG; sized by `IconSize`, drawn by `BarIcon` at the physical height, tinted, faded and drawn inactive; its square stays free while it loads.
- `icon_label.rs`: an icon, a 3 px gap and a two-line block.
- `folder_browser.rs`: the Folder Menu and Start Menu popup: listing on a worker, merged folders, keys, scrolling, sliding into a subfolder, drops onto rows.
- `eject_list.rs`, `eject.rs`: the Eject USB popup and the notifications an eject reports.

Colours (`sensors/color.rs`): `standard` is the sensor text colour (a meter's fill at 59 %), `custom` the chosen colour, `dynamic` green → yellow → red by value (`v < 0.5`: R = round(v × 510), G = 255; else R = 255, G = round((1 − v) × 510)). An icon's Custom colour tints it (see [Theme and skins](#theme-and-skins)); Standard leaves shell icons and the user's images unchanged and gives the built-in white icons the sensor text colour. Inactive drawing is the inactive opacity or colour of the theme (`theme.inactive`, through `color::inactive` or `Icon`), applied by each sensor while `inactive()` holds; a sensor with an error sample is drawn inactive as a whole by the bar.

Formatting (`sensors/format.rs`): sizes and rates in base 1024 with automatic digits chosen after rounding (a value that would round to 1024 moves to the next unit), durations compact and long, percentages; units and words are catalog entries with plural forms (`i18n::t_count`). Dates (`sensors/dates.rs`, `platform/locale.rs`): `Platform::locale` returns the locale as plain data (names, the genitive month names, date patterns in the Windows picture format, the hour cycle, am/pm, the first day of the week, the decimal separator), read with `GetLocaleInfoEx`; Rust formats dates with those patterns and computes weekdays, days of the year and ISO weeks itself, so all of it is tested against fixed locales. The bar keeps the `Locale` of `settings.dateLocale` and its `Numbers` and hands them to every context; it reads them again when the setting or the OS regional format (`WM_SETTINGCHANGE` "intl") changes.

### Types

| Type | Module | Topics |
|---|---|---|
| `text`, `image`, `spacer` | `text.rs`, `image.rs`, `spacer.rs` | none (wake-ups) |
| `date`, `time` | `date.rs`, `time.rs` | none (wake-ups on the local time) |
| `uptime` | `uptime.rs` | `uptime` |
| `cpu-meter`, `cpu-graph`, `cpu-graph-dual` | `cpu.rs` | `cpu` |
| `memory-meter`, `memory-graph`, `swap-meter`, `swap-graph` | `memory.rs` (written for any "used of total" topic) | `memory`, `swap` |
| `disk-meter` | `disk.rs` | `disk/<X:\>` |
| `network-graph` | `network.rs` | `net/<id\|auto>` |
| `battery` | `battery.rs` | `battery/<i>` |
| `wifi` | `wifi.rs` | `wifi` |
| `recycle-bin` | `recycle_bin.rs` | `recycle-bin` |
| `shortcut` | `shortcut.rs` | `path/<path>` |
| `disk` | `disk_launcher.rs` | `volumes` |
| `folder-menu`, `start-menu` | `folder_menu.rs`, `start_menu.rs` | none (listed on a worker when opened) |
| `power` | `power.rs` | none |
| `eject-usb` | `eject_usb.rs` | `volumes` |

A type whose capabilities this machine lacks, and an entry of a type this version does not know, are drawn by `placeholder.rs`: a dimmed, outlined label with a hint that says why; the entry stays in the config unchanged. The picker hides types whose capabilities are missing.

## Config

### Files

| What | Where |
|---|---|
| Config | `%APPDATA%\app.infodock.desktop\config.json` (`RoamingAppData`) |
| Skin packs | `%APPDATA%\app.infodock.desktop\skins\<pack-id>\` |
| Logs | `%LOCALAPPDATA%\app.infodock.desktop\logs\infodock.YYYY-MM-DD.log`, kept 7 days |
| Icon cache | in memory only |

`platform::AppDirs` names them; the installed and the portable app use the same folders.

### Schema

```jsonc
{
  "schema": 1,
  "meta": { "createdWith": "1.0.0", "lastWrittenWith": "1.0.0", "firstRunDone": true },
  "settings": { /* config::model::Settings: general keys, bar, theme, background, classic, sensors, popups, apps, settingsWindow */ },
  "layout": {
    "left":   [ { "id": "s_8f3k2", "type": "cpu-graph", "props": {} } ],
    "center": [ { "id": "s_1a9x0", "type": "image", "props": { "popupText": "Infodock %v" } } ],
    "right":  [ { "id": "s_z0c4d", "type": "time", "props": { "use24h": true } } ]
  },
  "state": { "s_u1": { "longestSecs": 1093221 } },
  "legacy": { "unsupported": [ { "type": "DockRSSSensor", "zone": "left", "props": { "url": "…" } } ] }
}
```

`config/model.rs` is the typed form. The first run's layout: left CPU Graph, Memory Meter, Network Graph; centre Image (the logo); right Date, Time.

- **`props` store only values that differ from the static defaults**, so defaults can change. Machine-derived fields are resolved once, when the sensor is created or imported, and always stored, so a sensor keeps its meaning when drives, the locale or the hardware change. Fields whose default follows a global setting store `""` for it. Keys a type's schema does not know (written by a newer version) are kept.
- Settings whose default comes from the theme are `null` until the user sets them.
- Unknown types are kept and drawn as placeholders, never deleted.
- `state` holds what sensors store for themselves (`Sensor::take_state`), by sensor id; removing a sensor drops it.

### The store

`config/store.rs`. Writes are debounced by 500 ms, on the config writer thread, and atomic: `config.json.tmp`, flush, rename. `config.json.bak` keeps the previous good version: each write copies the current file over it, but only while that file is known good (loaded or written by this run). A config that fails to parse is renamed to `config.broken-<timestamp>.json`, the `.bak` (or the defaults) is loaded, and a toast says so; when the rename fails, the file stays until a write replaces it. Each change takes the next revision under the store's lock; listeners run after the lock (see [Data flow](#data-flow)).

Migrations (`config/migrate.rs`) run on load and on import, one pure, tested function per schema step. Settings changes are deep partial JSON patches of `Settings` (`config/patch.rs`); unknown keys are refused.

**Errors** (`config/mod.rs`): `ConfigError` has the codes `io`, `json`, `notFound`, `unknownType`, `unknownSettings` and `invalid` and a message from the catalog, which Settings and the import show; its English text, with the code, goes to the log.

### Operations

Settings runs in the same process and calls the core directly (`sensors/ops.rs`, `config/`); there is no IPC. Add, update, move and remove a sensor; preview one (the bar shows props without saving them, and ending the preview restores the saved ones); update settings (a changed hotkey is registered first, and nothing is saved if that fails); export and import. `add` and `update` fill the machine-derived props a sensor lacks first, validate, and store only what differs from the defaults; `move` takes the index counted with the sensor already removed. What the machine says for machine-derived props is asked before the store is locked (`ops::machine_props`, `import::prepare`, `Registry::machine_defaults` at start-up), since a type may list the drives and every thread's read of the config waits for the lock.

### Export and import

`config/exchange.rs`, `sensors/import.rs`, `transfer.rs`. An export is the whole config, or the layout only (`{ schema, layout, state }`, with `legacy` while it holds unsupported legacy sensors), as `.infodock.json`. An import *replaces* the layout or *adds* to it, and takes only the layout, its sensors' state and unsupported legacy sensors, never settings; ids are regenerated. The file is migrated to the current schema; a newer schema, a file that is not JSON or holds no `layout`, and a file over 16 MB are refused. Each sensor is checked against its type as on load (wrong values give way to defaults with a warning, numbers clamped, unknown keys kept, machine-derived props filled, only differences stored); unknown types are kept with a warning. The result is an `ImportReport { added, skipped, warnings }`.

`transfer.rs` runs it for the user: the picker, the question (*Replace the layout*, *Add to the layout*, with the number of sensors, and the network paths the sensors would open), the report as a toast or a dialog of at most 15 lines. A file with no sensors this version can show asks nothing; one with more than 500 sensors is refused; names and values are cut to 80 characters. JSON files may be UTF-8 or UTF-16. One import or export runs at a time.

### Legacy import

`config/legacy.rs` reads the XML; each sensor type converts its own properties (`convert_legacy`, `sensors/legacy.rs`), then the props go through the import above.

- The text is UTF-8 (with or without a byte order mark), UTF-16 (with one, or without when its first character is ASCII), or an ANSI code page: the one the XML declaration names, or the system's (`Platform::decode_code_page`, `MultiByteToWideChar`), Windows-1252 where it cannot be converted. The first `<sensors>` element is read with `quick-xml`; text without one, and broken XML, are refused. A property given twice keeps its last value.
- A sensor without a known side, or of a legacy type Infodock does not know, is skipped and reported. Unsupported types keep their properties in `legacy.unsupported`, less any whose name contains `password`.
- Each type names its XML keys (`LegacyKey`): the prop each sets, read by that field's encoding (booleans also `true`/`false` and `1`/`0`; selects ignoring case, with a table of values that differ from the field's own; a font style as `bold,italic`; a drive as `C:` or `C:/`), a graph width, a key read without a warning, or a key dropped with a warning. A value that cannot be read keeps the default with a warning. Graphs share their keys (`graph::legacy_keys!`); types with more to do (a state, a matched adapter) override `convert_legacy`.

The fixtures are `tests/fixtures/legacy/` (`example.xml` with its expected result in `transfer.rs`'s tests, and one file per sensor family).

## Settings window

`ui/settings.slint` holds `SettingsWindow` and the `SettingsState` global; `ui/app.slint` exports what Rust uses; `ui/i18n.slint` the `I18n` global. Controls bind two-way to `SettingsState`. Rust (`settings/`) fills it from the config, saves each change as a settings patch or a sensor operation, and fills it again from the store's change listener, so changes made elsewhere (the tray, the hotkey, the bar) show at once.

- **Routes** (`settings/model.rs`): a page (`general`, `bar`, `appearance`, `sensors`, `popups`, `applications`, `help`, `about`), `layout` (the Layout manager on Sensors), `sensor/<id>` (its properties), `new?zone=…&index=…` or `new?target=<id>` (the picker), `welcome`. The bar opens Settings on a route through `BarHost`.
- **The window** (`settings/window.rs`): created when Settings opens and dropped when it closes. The main thread keeps it in one place, reached only through `with_open` and `with_open_mut`, which borrow it for one closure; a call made while it is borrowed (Slint or winit calling back during a refill, a modal picker's loop) does nothing and is logged. It opens where it was closed (`settingsWindow`, `model::place_reachable`). It is opaque (`with_transparent(false)` through the winit backend's window attributes hook, `lib.rs`), and painted whole again when restored from minimized or focused (`settings::Restore`, `SettingsState.repaint`), because Windows drops a transparent or minimized window's pixels and the software renderer repaints only what changed.
- **Pages**: the plain pages in `settings/mod.rs`; `appearance.rs` (theme, colours, material, the skin gallery, installing and removing packs); `applications.rs`; `layout.rs` (the Layout manager); `picker.rs` (the sensor picker); `properties.rs` and `form.rs` (a sensor's properties: one row per schema field, one control per field type, `visibleIf` and `required` evaluated, a live preview on the bar); `welcome.rs` (the first run: Settings opens on `welcome` while `meta.firstRunDone` is false); `about.rs` (the version, the licence, `assets/third-party.tsv`, the Fluent icons' licence, `AboutSlint`). *Restore defaults* builds one patch from the page's settings (`model::page_paths`, `page_defaults`).
- **Dialogs** go through `settings::dialogs()`, modal to the window while it is open.
- **Dropdowns** (`ui/dropdown.slint`): every list of choices is a `Dropdown`, drawn as the Fluent style's ComboBox and a combobox to screen readers (expandable, with its label and value; its options are list items). Its list opens as WinUI's does, with the selected option over the control, centred on it and so under the pointer that opened it, and the options before it above. At most six rows show: a longer list scrolls so that the selected option is as near its middle as the options allow, and the list stays where it opened while the arrow keys move the selection, scrolling to keep it shown. Slint draws a popup inside the window and moves one that would cross the window's edge back in, so near the window's top or bottom the list opens off the control. Up and Down choose on the closed control, and so does the wheel once it has the focus; Enter or Space opens the list and closes it again, and Esc or a click elsewhere closes it. A choice is saved as it is made, also while the list is open.
- **The contract** (`settings/contract.rs`, a test): the page, row and line numbers both sides use, named in the window's `Pages`, `RowKinds` and `LineKinds`, and every settings path the window or the Rust glue saves exists in `Settings` and takes a value of its kind.

## Theme and skins

`look/theme.rs` resolves a `Theme` from the theme's defaults (`ThemeDefaults`, modern or classic), the user's settings and the OS appearance (light or dark, the accent colour, transparency effects; on Windows from `HKCU\…\Themes\Personalize` and `UISettings.GetColorValue(Accent)`, with `HKCU\…\DWM\AccentColor` only where that fails), and resolves it again when any of them changes (`WM_SETTINGCHANGE` with `ImmersiveColorSet`). All drawing reads from it: the bar's height, the sensors' strip, background colour and opacity, padding and gap, the sensor text colour and font, the text sizes, the popups' look, the accent, the inactive style, the hairline. Settings has its own look and uses the bar's renderer only for skin previews (`bar::skin_preview`). `Settings::light_bar` says when the modern theme takes its light look (light mode and colours not customised).

**Tint** (`platform/bitmap.rs`; skins and icons with a custom colour): per pixel `L = (0.299R + 0.587G + 0.114B) / 255`, output RGB = `L × tint`, alpha kept.

**Skins** (`look/skins/`), for the classic theme's background:

- Built-in (`builtin.rs`): `flat`, `gloss`, `brushed`, `carbon`, `glass`, drawn pixel by pixel from `background.color` into a bitmap of the bar's physical size (lines one logical px in whole physical px, textures sized in logical px); `custom`, the user's image (`classic.customSkin`) tiled like a pack's tile without a band. The bitmap is kept until the skin, its colour, the bar's size or its edge changes, and drawn with the background opacity.
- Packs (`pack.rs`, `raster.rs`): a folder with a `skin.json` (`name`, `image` or `imageTop` and `imageBottom`, `height` (default 25), `shadowBand`). The tile is decoded by the platform, cut into its bar part and band, tinted, scaled vertically to the bar's height with a linear filter on premultiplied pixels (the band keeps its size), scaled to the monitor's scale, and tiled from the bar's left end; `image` serving a top bar has its band mirrored below. Checked when listed and when drawn: PNG or JPEG by their first bytes, ≤ 1 MB per file, ≤ 512 px a side, only files named in `skin.json`, `height + shadowBand` within 1–512 px (± 1 px), each tile taller than its band. A pack that cannot be drawn, or a configured skin that is missing, gives Gloss, and the reason is logged.
- Installing (`install.rs`, `zip.rs`): from a folder or a `.zip` (`skin.json` at its top or in one folder; at most 32 MB; entries stored or deflated, checked against their size and CRC; encrypted and ZIP64 archives refused). The files are copied into a staging folder `.install-<name>` in the skins folder (names starting with `.` are not packs), checked there, and put in place of an installed pack of the same name. Removing a pack moves its folder to the Recycle Bin. After either, the bar draws its skin again.

## Localisation

`i18n.rs`. Every user-visible string goes through `i18n::t(key, …)` in Rust (bar, tray, menus, dialogs, notifications) or `I18n.tr()` in `.slint` (Settings, `install_i18n!` in every component instance). The catalogs are `i18n/<lang>.json`, embedded in the binary, with stable keys (`sensor.cpu.hint.activity`); `en.json` is the source and the only catalog, so the language selector is hidden. A key missing in another catalog falls back to English and is logged once.

- Plurals: entries hold the CLDR categories `one`, `few`, `many`, `other`; `t_count` picks one with CLDR rules for the UI language.
- Arguments fill a message's `{name}` placeholders in one pass (`i18n::fill`): a value is copied as it is, so one that holds `{…}` (a file name) is never filled in, and a `{name}` no argument gives stays.
- Dates and numbers come from the platform locale, never from the catalogs (see [Sensors](#sensors)).
- Slint draws a character its font lacks as a gap, so messages use beyond ASCII only `i18n::NON_ASCII`.
- Tests: every key used in `.slint`, every literal key in the Rust code and every key built from ids exists in `en.json`; the catalog holds no other non-ASCII characters; on Windows, Segoe UI Variable and Segoe UI have every one of them.

## Windows data sources

| Topic | Payload | Source | Read |
|---|---|---|---|
| `cpu` | `{ total, user, kernel, idle, cores, logical }`, 0–100 | PDH `\Processor Information(_Total)\% Processor Utility` and its `(*)` instances, opened once (`platform/windows/cpu.rs`); `GetSystemTimes` deltas for user, kernel and idle (kernel includes idle) | by the sampler; the first sample after start or resume is discarded |
| `memory` | `{ total, available, used, load }` | `GlobalMemoryStatusEx` | by the sampler |
| `swap` | `{ total, used, load }` over all page files | `NtQuerySystemInformation(SystemPageFileInformation)`, in pages of `GetSystemInfo`'s size; `swapfile.sys` skipped | by the sampler; `total == 0` means no page file |
| `disk/<X:\>` | `{ present, total, free }` | `GetDiskFreeSpaceExW` | on a worker |
| `volumes` | `Volume[]`: `{ root, kind, label, ejectable, media }` | `GetLogicalDriveStringsW`, `GetDriveTypeW`, `GetVolumeInformationW`; the bus (`IOCTL_STORAGE_QUERY_PROPERTY`: USB, FireWire, SD, MMC) for `ejectable`; `IOCTL_STORAGE_CHECK_VERIFY2` for `media` without a spin-up (`device.rs`) | on a worker, every 60 s and at once on `WM_DEVICECHANGE` |
| `net/<id\|auto>` | `{ present, id, alias, description, up, rxRate, txRate, rxTotal, txTotal, linkRx, linkTx }` | `GetIfEntry2`; `GetBestInterfaceEx` for Automatic; `GetIfTable2` lists the connections | by the sampler; rates from QPC-timed deltas, a negative delta gives 0 and a new baseline |
| `battery/<i>`, `batteries` | a battery's charge, state, times and names; the names in their places | `starship-battery` (the battery class driver), `GetSystemPowerStatus` (the AC line) | on a worker every 10 s; pushed on `PBT_APMPOWERSTATUSCHANGE` |
| `wifi` | the connection, its signal and the radio | `wlanapi`, loaded dynamically (`wlan.rs`); a client kept open while the topic has subscribers | by the sampler; pushed on WLAN notifications |
| `uptime` | `{ bootUnixMs, uptimeSecs }` | `GetTickCount64` | by the sampler |
| `path/<path>` | `{ kind }`: `missing`, `file` or `folder` | `GetFileAttributesW` | on a worker |
| `recycle-bin` | `{ items, sizeBytes }` | `SHQueryRecycleBinW` of each fixed drive, summed; a drive whose bin cannot be read is left out, and the shell's message is the error only when none can be read | on a worker, which hands the reading to the drives thread; pushed after the sensor's actions |

**Drives Windows takes** (`platform/drives.rs`, `platform/windows/drives.rs`). Windows asks the programs that keep a drive open before it removes, locks or dismounts the drive, and a reading that has the drive open at that moment stops the removal. So the platform layer's readings of a drive pass the drive gate, one slot per letter: `volumes`, `disk_space`, `path_kind`, `list_dir`, `shell_icon` and each drive's Recycle Bin enter it for the drive they read and leave it when they are done. Image files are read without it: the sensors' images by `platform::load_image` on the `icons` worker, the classic skin's custom image by `look::skins::read_custom` where the skin is drawn; so are the app's own files, and the actions and the pickers do not enter it either. The drives thread keeps a handle with no access on each fixed, removable and optical drive but the system drive, registered for its device events (`RegisterDeviceNotificationW`, `DBT_DEVTYP_HANDLE`), and lists the drives again on each volume broadcast. On `DBT_DEVICEQUERYREMOVE`, `GUID_IO_VOLUME_LOCK` or `GUID_IO_VOLUME_DISMOUNT` it closes the drive in the gate, so no reading starts there, waits up to 2 s for the readings already running, closes the handle and says yes, keeping the registration to hear how it ends. On `DBT_DEVICEQUERYREMOVEFAILED`, `GUID_IO_VOLUME_LOCK_FAILED`, `GUID_IO_VOLUME_UNLOCK`, `GUID_IO_VOLUME_DISMOUNT_FAILED` or `GUID_IO_VOLUME_MOUNT` it opens the gate and watches the drive again; on `DBT_DEVICEREMOVEPENDING` or `DBT_DEVICEREMOVECOMPLETE` it forgets the drive, whose gate opens with the volume broadcast that follows. While a drive is closed, `volumes` lists it as it was last listed, so the lists do not change meanwhile; `disk_space` and `path_kind` read it as absent, `list_dir` fails (the drive is not ready), `shell_icon` says busy, so the icon is asked for again later, and its Recycle Bin is not counted. The Recycle Bin is read on the drives thread (`drives::run`), so whatever the shell keeps of a bin lives on a thread that handles messages. Those jobs run between the thread's answers, except when the shell handles messages during one and a request comes meanwhile: that answer comes inside the job, and waits for the other threads' readings of the drive but not for the job's own, which cannot end before it (each reading counts its drive for the thread it runs on). A let-go while the job still reads that drive is logged as a warning. The watch's handles have no access, so they hold up no lock, and they are the only handles Infodock keeps on a drive no sensor shows.

Other Windows services: the Recycle Bin's actions (`shell.rs`, `fileops.rs`), power actions (`power.rs`), ejecting (`eject.rs`: the volume's disk found by `IOCTL_STORAGE_GET_DEVICE_NUMBER` among SetupAPI's disk and CD-ROM interfaces, then the first removable node from its parent up ejected with `CM_Request_Device_EjectW`, a veto naming what holds it; optical and other removable media locked, dismounted and ejected with `IOCTL_STORAGE_EJECT_MEDIA`; the shell's *Eject* verb last; only volumes the drive list marks `ejectable` are touched), folder listings (`list_dir`, with each entry's attributes) and `.lnk`/`.url` resolution, and the locale (`locale.rs`, NLS).

## Errors and logging

- **Readings** never crash the app. A failed reading keeps the previous sample and is retried at the next tick; after three in a row the topic's sample is `Sample::Error` with the platform's message, delivered again only when the message changes, and its retries back off from 1 s to 30 s (never sooner than the topic's interval; a change or a resume still reads it at once). The first success ends it. The bar keeps the error per sensor (`bar/sensors.rs`; sensors never see it): the sensor is drawn inactive as a whole, and its hint and accessible value say the data could not be read. The sampler logs when a topic turns to an error and when it can be read again. Absent data (a missing drive or adapter, no page file) is a sample of its own.
- **Panics** are logged with a backtrace (the panic hook in `logging.rs`). In callbacks the OS calls, they are caught there (`platform/guard.rs`) and the app goes on; the drives window's procedure is one, and each job the drives thread runs is caught on its own. A panic on the main thread runs the fatal cleanups (releasing the AppBar), flushes the log and ends the app; one while the bar thread starts ends that thread, and so the start; one on another thread (the sampler, the config writer, a worker) ends that thread.
- **The log** (`logging.rs`): `tracing` to a daily file in the logs folder, kept 7 days. It records the start-up's steps and the bar's first frame, the app's own memory at set moments (see [Budgets](#budgets)), z-order changes at debug level, each drive the drive watch lets go of and watches again, the quit, the window a close request came to and why a session ended, failures the user does not see.

## Accessibility

- **Settings** is Slint with AccessKit: every control has an accessible name that tells it apart on its page, Tab follows reading order, lists carry their state (`accessible-item-index`, `-count`). `settings/accessibility.rs` walks every page through Slint's testing API (debug builds keep the names) and follows the focus as Tab moves it; `build.rs` keeps those names in debug builds only.
- **The bar** is a UI Automation provider (`platform/windows/uia.rs`): `WM_GETOBJECT` for `UiaRootObjectId` answers with a fragment root (*Tool bar*, "Infodock", automation id `InfodockBar`) whose children are the sensors in focus order, Spacers left out: *Button*, named by the type, automation id the sensor's id, the read-only Value pattern (`accessible_value`), its screen rect, the Invoke pattern, ExpandCollapse for sensors with a popup (`has_popup`), `SetFocus`. Once a client has asked, the bar logic describes the bar after each frame (`BarHandler::accessible`: an `AccessibleBar` of `AccessibleItem`s and the focus), and the providers answer from that snapshot, never waiting for the bar logic. Patterns queue their request (`AccessibleAction`) and post the bar window, which hands it to the bar logic as `UiEvent::Accessible`. `platform/accessible.rs` (pure) compares each snapshot with the last and, while clients listen (`UiaClientsAreListening`), raises focus changes, value changes (at most once a second per sensor), expand/collapse changes and structure changes. Hint and popup windows carry titles screen readers read (`Popup::name`).
- **Popups** (`platform/windows/uia_popup.rs`): the popup window's own provider, a *List* (`InfodockPopup`) of *List item* rows (`Popup::accessible`: an `AccessiblePopup` of `AccessibleRow`s), with `PositionInSet`, `SizeOfSet`, `IsOffscreen`, and the Selection and SelectionItem patterns (`Popup::accessible_select`). The description is built only once a client has asked for the window and only while one listens, since a folder lists every item; a stale one is described again when asked (`AccessibleAction::DescribePopup`).
- **Keyboard on the bar** (`bar/focus.rs`, pure; `bar/thread/keyboard.rs`): the hotkey gives the bar window the keyboard focus (`SetForegroundWindow`), remembering the window that had it. Tab, Shift+Tab, the arrows, Home and End move the focus ring across the zones in visual order, wrapping, past Spacers (`focusable`); Enter and Space click whatever the Lock rules say for the pointer; Shift+F10 and the Menu key open the menu beside the sensor; Esc, or hiding the bar, gives the focus back. The ring is a 2 px outline in the accent colour, with a white or black line inside it where the accent has under 3:1 contrast with the bar.

## DPI and monitors

The app is per-monitor DPI aware v2 (`resources/infodock.manifest`, with common controls 6 and Windows 10/11 compatibility). Layout and drawing are in logical px; the bar, hint and popup windows render at their monitor's scale and convert geometry to physical px. Moving the bar to a monitor with another scale negotiates the AppBar again and renders again after `WM_DPICHANGED`. Icons are asked for at their physical size (size × scale, rounded). `platform/windows/monitors.rs` lists the monitors with their device and friendly names, bounds, work area, scale and the primary flag.

## Budgets

Measured on the release build with the first-run layout and 1 s graphs. CI enforces all of them except the memory with Settings open:

| Measure | Budget | Measured by |
|---|---|---|
| Idle CPU | ≤ 0.5 % over 60 s | `measure-memory.ps1` |
| Private working set of `infodock.exe` and its children, Settings closed | ≤ 30 MB | `measure-memory.ps1` |
| The same 10 s after Settings was opened and closed | ≤ 30 MB | `measure-memory.ps1` |
| Settings open | adds ≤ 20 MB; reported only | `measure-memory.ps1` |
| Process creation to the bar's first frame | cold ≤ 500 ms, warm ≤ 300 ms | the app's log, read by `measure-memory.ps1` |
| Full bar redraw, 2560 px wide | ≤ 2 ms | `budget_bar_redraw` (`bar/paint.rs`) |
| Sampler tick | ≤ 2 ms | `budget_sampler_tick` (`sampler/mod.rs`) |
| Hint shown after its delay (the Date's calendar) | ≤ 30 ms | `budget_hint` (`bar/paint.rs`) |
| Folder Menu, a folder of 500 items, to its first frame | ≤ 200 ms | `budget_folder_menu` (`bar/paint.rs`) |
| A frame of a Folder Menu of 5,000 items, described for a listening UI Automation client | ≤ 200 ms | `budget_folder_menu_described` (`platform/windows/uia_popup.rs`) |

The `budget_*` tests are Windows-only and ignored by default; they take the median of many runs and write `target/budgets/*.json`, which `measure-memory.ps1` reads. They are built unoptimised and leave out the calls that put a frame on the screen. The app logs `bar visible … ms after the process started` with the times of the start-up's steps (config, toolkit, bar thread, window, bar logic, capabilities, sensors), and its own memory (private and whole working set, commit, threads) at the bar's first frame, 10 s after Settings opened and closed, and on the welcome page; CI lists these beside its own measurements.

What keeps them: one sampler thread; PDH handles opened once; no WMI; only dirty sensors measured again, and at most one frame per 16 ms, only when something changed; software Direct2D; text layouts and bitmaps cached per sensor; hint and popup windows reused; the popups' icon LRU; the Settings window dropped on close; nothing slow on the way to the bar's first frame.

## Artwork

All artwork is Infodock's own, except the Settings navigation's icons (`ui/icons/nav-*.svg`, Fluent UI System Icons, MIT, credited on About). The app icon and the README logo set their letters in Roboto Mono as outlines, so they need no installed font; the tray icons, the logo and the installer images are drawn from them.

| Item | Files | Used by |
|---|---|---|
| App icon | `icons/app-icon.svg` (16–24 px from `app-icon-small.svg`) → `icon.ico` (16–256), the bundler's PNGs | installer, Start Menu, Settings |
| README logo | `icons/logo-light.svg`, `logo-dark.svg` | README, About and the welcome page, installer images |
| Tray icon | `icons/tray-dark.svg`, `tray-light.svg` → `.ico` 16/20/24/32, drawn on the 16 px grid | the tray, by the taskbar's theme |
| Logo | `assets/logo.svg`, white so it can be tinted | the Image sensor's default |
| Sensor icons | `assets/icons/*.svg`, monochrome white: battery layers, Wi-Fi levels 0–5, the seven power modes, Recycle Bin empty and full, USB, folder, Start Menu, globe, generic app, drive, file, back | the sensors and the Folder Menu popup |
| Chime | `assets/sounds/chime.wav` (16-bit PCM, 44.1 kHz) | Time |
| Installer images | `icons/installer/header.svg` (150×57), `sidebar.svg` (164×314) → 24-bit BMP | the NSIS installer |

`cargo run --example icons` (`examples/icons.rs`) renders the `.ico`, PNG and BMP files from their SVGs with resvg, as the app draws its SVGs, supersampled so each pixel takes its exact coverage; its tests check the committed files against the SVGs. `cargo test --example icons -- --ignored` renders the tray icons and installer images to `target/screenshots/` for a look.

## Building and releasing

`build.rs` compiles the Slint UI and, for Windows, the resources: the exe icon, the manifest, and the version information from `Cargo.toml`'s version (file version `x.y.z.0`, product version the whole version).

**Versions** follow Semantic Versioning and come from the git tags `v<major>.<minor>.<patch>` (`publish.sh version`): from `master` the next release (`1.0.0` while there is none, then the latest tag of the major stream with its patch, minor or major raised); on another branch the next preview (`1.0.1-preview.1`, then `-preview.2`). While a preview is ahead of the latest release, a bump past it starts a new version and a bump it covers continues it. The stream is the major of the latest release or preview tag; *Base Ver* (`publish.sh -M`) picks an older one. Local builds are `0.0.0`, the version in `Cargo.toml`; CI builds are `<next>-build.<run>`. `publish.sh build` writes the version into `Cargo.toml` for the build and puts the file back after; the app shows it (About, the logo's hint, the log, `meta.lastWrittenWith`).

**Packaging** (`[package.metadata.packager]` in `Cargo.toml`, cargo-packager 0.11.8's NSIS template, pinned in CI): a per-user installer to `%LOCALAPPDATA%\Infodock` without elevation (`asInvoker`), listed under `HKCU` only, with the Start Menu shortcut `Infodock` (AppUserModelID `app.infodock.desktop`) and a desktop shortcut on the finish page's option. `/AUTOSTART`, for silent deployments, writes the `Run` value for the installed exe and clears Task Manager's mark; the app's first run takes it into its setting. An entry naming an exe that is not in place when the installer ends is removed. The installer, before it copies the files, and the uninstaller close a running Infodock, asking first with the template's question unless the run is silent (or a passive install), where Cancel stops them: a section of Infodock's own before the template's (`-CloseRunning`, `-un.CloseRunning`) sends the bar window `WM_CLOSE` and waits up to 10 s for the process to end, so the app quits as on any close request and writes its config, and the template's own check then finds it gone. One that has not quit by then is terminated by the template (`nsis_tauri_utils::KillProcess`), after the question again in an interactive run, and cleans up nothing: Windows removes its AppBar with its window, the tray icon stays until the pointer passes over it, and a config change not yet written (at most 2 s old) is lost. An upgrade that removes the old version first (the reinstall page's choice) runs that version's uninstaller. The uninstaller removes the files, shortcuts, registration, the sign-in entry when it names the installed exe, the install location under `HKCU\Software\jurakovic\Infodock` and the AppUserModelID's registration; an upgrade keeps all but the last. The config, skins and logs stay. The portable build is the bare `infodock.exe` of the same release build, using the same per-user folders.

**Releases** are made by hand with the Publish workflow (`.github/workflows/publish.yml`), whose steps are `publish.sh`'s: **build** (the tests, then `infodock.exe` and the installer with the version), **package** (`release/`: `Infodock_v<version>_win-x64-setup.exe`, `Infodock_v<version>_win-x64.zip` with the portable exe, `checksums.txt` with SHA-256 checksums), **release** (the tag `v<version>` on the built commit and a GitHub release with those files and generated notes; previews are pre-releases). Its inputs choose the steps, the branch, the bump, the major stream, preview or not, or an explicit version. Its build job runs the third-party actions (`dtolnay/rust-toolchain`, `Swatinem/rust-cache`, `taiki-e/install-action`) pinned by commit, each with the branch or tag that names its commit in a comment; CI takes them by branch or tag (`stable`, `v2`). `publish.sh` also runs in Git Bash on Windows (`./publish.sh --help`).

## Testing

| Level | What | How |
|---|---|---|
| Unit | layout, the move index, hint and popup placement, focus moves, resizing; formatters, colours, tint; schemas: validation, defaults, storage rules; each sensor's hint and drawing (against the recording canvas with fixed text metrics), its legacy conversion, and creation from its defaults and from every option of its lists; dates against fixed locales; CPU, network and battery arithmetic; geometry; config migrations, patches, exchange and the legacy import; Settings' mapping and every schema's form; the i18n keys | `cargo test`, on Linux and Windows |
| Windows integration | providers on a real machine, locale data, folder listings, links, shell icons, skin packs, the drive watch and the Recycle Bin read drive by drive; the Direct2D renderer drawing the bar and each sensor type to PNG | `cargo test -- --ignored` on Windows; the bar's renders with `cargo test render_bar -- --ignored` |
| Settings | every page rendered offscreen to PNG and driven with clicks and keys that find their controls by accessible role and name; the accessible names and Tab order; where a dropdown's list opens and how the mouse and keys choose in it; the contract with the window | `cargo test`, the renders with `cargo test render_settings_pages -- --ignored` |
| Budgets | see [Budgets](#budgets) | `cargo test budget_ -- --ignored --test-threads=1` on Windows; `measure-memory.ps1` |
| CI checks | the hotkey, Show desktop, releasing a drive Windows locks and watching it again after the unlock, the installer, quitting when `taskkill` or Restart Manager asks | `.github/scripts/` (see [CI](#ci)) |

Test support:

- `test_util::FakePlatform`, one fake for every test: what a test sets (capabilities, volumes, connections, shell icons and whether they are busy, the clock, the locale) it answers with; what it is asked to do (opening, launching, file operations, ejecting, notifying) it records instead.
- `sensors/test_support.rs`: the sensor tests' `Harness` (the fake platform, default settings and theme, fixed text metrics; `ctx()`, `layout()`, `drawn()`), and `resolved` / `created` for props from a schema.
- `bar/thread/tests/`: the bar logic driven end to end with a fake window and host.
- Renders go to `target/screenshots/`; drawing code is tested on Linux against the recording canvas.

## CI

`.github/workflows/ci.yml` runs on pushes to `master` and on pull requests that change build inputs (the paths it lists; docs-only commits skip it), and by hand.

- **Linux**: `cargo fmt --check`, clippy with warnings as errors, `cargo test`.
- **Windows**: the version (`publish.sh version`, as `<next>-build.<run>`); clippy; `cargo test`; the integration tests with `--ignored` (the Settings and bar renders included); the budget tests one at a time; the release build and the installer (`publish.sh build`); then on that build:
  - `measure-memory.ps1`: memory, idle CPU and the start times, with the budget tests' records and the app's own log lines, in the job summary; fails above a budget, when a measurement is missing or when the app has to be killed. Its first run opens the welcome page and ends the first run, so the measured run starts with the bar alone. Restart Manager closes the first run, as an installer that uses it would, and `taskkill` without `/f` the measured run; each must quit within 10 s, and one that has to be killed lists its windows in z-order.
  - `check-hotkey.ps1`: a hotkey another app holds, pressed while Settings records one, with the keyboard hook's trace.
  - `check-show-desktop.ps1`: a bar kept behind the windows beside a maximised window, through Win+D, clicks on the bar, a window back from the taskbar, every window minimised and restored, and the taskbar's far corner; each step records what is at the bar, its topmost flag, the foreground window and a screenshot, and the clicks check that the z-order did not change.
  - `check-drive-release.ps1`: a 64 MB VHD attached as a fixed NTFS drive with an item in its Recycle Bin, locked and unlocked (`FSCTL_LOCK_VOLUME`, `FSCTL_UNLOCK_VOLUME`) in a tight loop for a minute while the app runs each of three layouts (a Recycle Bin read every 5 s; a Disk for `C:` and Eject USB; both and a Disk Meter for `C:`), after a baseline without the app (the drive locks, and a file held open on it keeps it from locking); a lock that fails means a reading had the drive open. Under each layout the app's log must also have a line that it watched the drive again after an unlock, and its last line of letting go of the drive or watching it again must be a watch-again, since a drive left let go of is read no more and every lock would then take. The app's log lines that name the drive are kept; a failed lock, or a layout whose log misses either, fails the build.
  - `check-installer.ps1`: the interactive installer's first pages saved as screenshots, then closed; a silent install without elevation (per user, no Program Files, no `HKLM`), the Start Menu shortcut with its AppUserModelID, the app started (`/R`), its first run taking an existing sign-in entry and quitting within 10 s of a `taskkill` without `/f` with its welcome page open; a silent install over the running app and an uninstall while it runs, each of which must close it cleanly: the log has one more clean exit, and the config the welcome page's place, moved just before; the uninstall removing the app, its shortcuts, its registration, the sign-in entry and Task Manager's mark; a second silent install with `/AUTOSTART` writing the entry, and its uninstall removing it.
- Artifacts, named like the release files: `Infodock_v<version>_win-x64-setup.exe`, `Infodock_v<version>_win-x64` (the portable exe), `_screenshots`, `_memory-check`, `_show-desktop-check`, `_drive-release-check`, `_installer-check`.

`.github/scripts/third-party.py` writes `assets/third-party.tsv` from `cargo tree --target x86_64-pc-windows-msvc --edges normal,no-proc-macro`: the crates the Windows build links, with their licences.

From Linux, `cargo check-win` runs clippy for the `x86_64-pc-windows-msvc` target over every `#[cfg(windows)]` path without linking; it needs that target and `llvm-rc`.

## Adding a sensor type

1. A module in `sensors/` with the `SensorType` (id, category, name and description keys, `requires`, `schema`, `create`, and the legacy keys if the legacy format had the sensor) and the `Sensor`, built from the blocks in `sensors/common/`.
2. Its entry in `REGISTRY` (`sensors/registry.rs`).
3. Its strings in `i18n/en.json`; its icon, if any, in `assets/icons/`.
4. A provider and topic in `sampler/topics.rs` and `providers/`, with a `Platform` reading and its Windows implementation, if it needs new system data.
5. Tests with the `Harness`: its hint, its drawing, creation from its defaults and every option, its legacy conversion. The registry's tests and the Settings form tests then cover it.

## Known limitations

- Windows only. Other OSes run Settings and the tests on the stub platform; there is no bar.
- The Settings window follows light and dark mode and the display scale, but not Windows' high-contrast themes or the separate *Text size* setting: Slint's Fluent style has only a light and a dark palette and sizes text relative to the window's default font size.
- Only English ships.
- The installer and the exe are not code-signed.
- A Wi-Fi adapter plugged in after start-up is found at the next start, since capabilities are probed once per run.
- A drive mounted in a folder rather than at a letter is not watched, so a reading there can still keep Windows from removing it.
