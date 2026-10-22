# Infodock — Specification

| | |
|---|---|
| Status | Draft 5 — native bar, Slint Settings; ready for implementation |
| Date | 2026-10-12 |
| Product | **Infodock** — a compact system-information and launcher bar for the desktop |
| Target | Windows 10/11 first; macOS/Linux later |
| Stack | **Rust**: a **native bar** (Win32 + Direct2D/DirectWrite) and **Slint** for the Settings window |
| Licence | **MIT** |

---

## 0. How to read this document

* §1–2: what Infodock is, its goals and fixed decisions.
* §3–8: technology, architecture, the bar, interaction, popups and the sensor framework.
* §9: the **sensor catalog** — every v1 sensor with its properties, layout, data source and behaviour.
* §10–13: settings, persistence (including the legacy XML import), theming and localisation.
* §14–19: non-functional requirements, project layout, testing, milestones, backlog, risks.
* Appendices: Windows data sources, an example legacy file with its expected import result, and the artwork to be produced.
* "**XML key**" columns in §9 give the property name used by the legacy XML layout format (§11.4). "—" means the property has no legacy equivalent.
* MUST / SHOULD / MAY carry their RFC 2119 meanings.
* This document is self-contained. No other source material is needed or should be consulted.

---

## 1. Product overview

Infodock is a **thin horizontal bar** (25–28 px high, full monitor width) docked to the top or bottom edge of the screen. It holds small **sensors** in three zones: **left, center, right**.

Each sensor:
* shows a compact live value (a CPU graph, the clock, a disk-usage meter, a folder icon…);
* has a **hover popup** ("hint") with details;
* has a **click action** (e.g. open Task Manager, open a folder menu);
* has a **context menu** and a **properties dialog**.

Users add sensors through a picker, or by dragging files, folders, drives and URLs onto the bar. They reorder sensors by dragging.

### 1.1 Feature summary

* Bar at the top or bottom edge of a chosen monitor, full width; docked or free-floating.
* Reserves desktop space like the taskbar does (Windows AppBar), so maximised windows do not cover it.
* Bar background: theme, tint colour, opacity, optional material (Acrylic/Mica), optional shadow, optional **highlight on hover**.
* Slide-in/fade-in animation on start and after the edge changes.
* Three zones with a configurable gap between sensors (1–20 px, default 10) and 7 px edge padding.
* **Lock** toggle (prevents rearranging) and **Popups** toggle (disables hover popups).
* **Drag to reorder**, with an indicator showing the drop position.
* **External drag & drop**: file → Shortcut, URL → web Shortcut, drive → Disk, folder → Folder Menu.
* **Global hotkey** to bring the bar to the front.
* **Tray icon** with show/hide, lock, popups, settings and quit.
* **Sensor picker** grouped by category; insert before/after an existing sensor.
* **Properties dialog** with live preview.
* **Layout import/export** (JSON) and **legacy XML layout import**.
* **Inactive rendering** of sensors whose state is "nothing to show / offline".
* Global defaults for sensor text colour, font and style, and for popup colours, font, size and shadow.
* Modern theme (default) and a compact **Classic** theme with skins.
* Multi-monitor support, per-monitor DPI, resume-from-sleep handling.
* Localisation infrastructure; **English only** at launch.

### 1.2 v1 sensor set

| Category | Sensors |
|---|---|
| General | Text, Image, Spacer |
| Info | Date, Time, Uptime |
| System | CPU Meter, CPU Graph, CPU Graph (dual), Memory Meter, Memory Graph, Swap Meter, Swap Graph, Disk Meter, Battery Life, Recycle Bin |
| Network | Network Graph, Wi-Fi Signal |
| Launchers | Shortcut, Disk, Folder Menu, Start Menu, Power, Eject USB |

Later (backlog §18): RSS/Atom feeds, mail, hardware temperatures/fans, internet radio.

---

## 2. Goals, non-goals and fixed decisions

### 2.1 Goals

* G1 — A **single, standalone, always-on desktop bar app**: one exe, no host engine, runtime or browser engine.
* G2 — The full v1 sensor set (§1.2) with the behaviour defined in §9.
* G3 — **Light footprint**: idle CPU ≤ 0.5 %; private memory ≤ 30 MB while Settings is closed (stretch goal 20 MB). No browser engine runs while only the bar is visible (§14.1).
* G4 — **Correct Windows integration**: AppBar work-area reservation, multi-monitor, per-monitor DPI, Explorer restarts, sleep/resume, full-screen apps.
* G5 — **Modern default theme**, plus a compact **Classic** theme with skins (§12).
* G6 — **Data-driven sensors**: adding a sensor type means adding one Rust module in `sensors/` (plus a provider if it needs new system data). Settings needs no changes: its forms are generated from the property schema each type exports (§8.2).
* G7 — **Portability seam**: every OS-specific call goes through a platform trait, so macOS/Linux can be added without touching sensors.

### 2.2 Non-goals (v1)

* Feed, mail, hardware-temperature and radio sensors (backlog §18).
* A third-party plugin API or scripting.
* macOS/Linux implementations (only the abstraction is required).
* Vertical (left/right edge) bars; multiple bars.
* Auto-update and code signing (later, §15.4).
* Translations other than English (the infrastructure is required, §13).

### 2.3 Fixed decisions

| Topic | Decision |
|---|---|
| Name | **Infodock**. Identifier `app.infodock.desktop`, executable `infodock.exe`, Cargo crate `infodock`. A trademark search (WIPO Global Brand Database) found only two marks, both expired about 10 years ago |
| Licence | MIT |
| Platforms | Windows first; platform code behind interfaces for later macOS/Linux |
| Stack | Rust. The bar, hints and popups are native windows drawn with Direct2D/DirectWrite. The Settings window uses Slint and is created on demand (§3, §4.1) |
| v1 sensors | §1.2 |
| Look | Modern default theme + Classic theme with skins |
| Assets | All artwork and sounds are produced for Infodock (Appendix C). No third-party artwork is bundled except permissively licensed icon sets (e.g. Fluent UI System Icons, MIT) with attribution |
| Languages | i18n infrastructure from day one; ship English only |
| First-run layout | Left: CPU Graph, Memory Meter, Network Graph · Center: Image (logo) · Right: Date, Time (§11.5) |

---

## 3. Technology stack

| Layer | Choice | Notes |
|---|---|---|
| Core language | **Rust** (stable, edition 2024) | |
| Bar, hints, popups | **Native Win32 windows** owned by the core, drawn with **Direct2D** + **DirectWrite** (software render target, §4.6), images decoded with **WIC** | No browser engine. Everything is drawn by Infodock's own renderer (§8.3) |
| SVG assets | `resvg` (rasterises the Infodock icon set and user SVG images to bitmaps, cached per size) | Infodock does not use its text feature (Slint, which links the same version, enables it); images referenced by path inside an SVG are not loaded |
| Windows API | `windows` crate (Win32: Shell, UI.Shell, UI.WindowsAndMessaging, UI.Input.KeyboardAndMouse, UI.Accessibility, Graphics.Gdi, Graphics.Dwm, Graphics.Direct2D, Graphics.DirectWrite, Graphics.Imaging, System.Com, System.Ole, Media.Audio, Globalization, NetworkManagement.IpHelper, NetworkManagement.WiFi, System.Performance (PDH), System.Power, System.Shutdown, System.RemoteDesktop, Devices.DeviceAndDriverInstallation, Storage.FileSystem) | |
| Settings window | **Slint 1.x** (winit backend, software renderer, Fluent style, AccessKit), UI in `.slint` files compiled by `build.rs`; pin exact versions in the lockfile | Event loop on the main thread. Settings is created when opened and dropped when closed. Licence: Slint Royalty-free 2.0, which requires the Slint attribution badge where the app is downloaded (README) or `AboutSlint` in the About page |
| Global hotkey | `global-hotkey` | |
| Tray icon and menus | Native: `Shell_NotifyIconW` registered for the bar window, `TrackPopupMenuEx` menus, both on the bar thread | Slint 1.18's tray ignores menu check marks on Windows (plan D17) |
| System info helpers | `sysinfo` (fallback/cross-platform), `starship-battery` | No WMI: page files come from `NtQuerySystemInformation` (plan D24) |
| Threads | A dedicated **bar thread** (§4.1), a **sampler thread** (§4.4), and worker threads for slow work (directory listings, file operations, icon extraction) | |
| Serialisation | `serde`, `serde_json`; `quick-xml` (legacy import) | |
| Logging | `tracing` + `tracing-appender` (rolling files) | |
| Other OS services | Through the platform layer (§4.6): single-instance lock, known folders, launching files and URLs, file pickers (`IFileOpenDialog`, `IFileSaveDialog`), toast notifications (WinRT), autostart (the per-user `Run` key) | |
| Window effects | `window-vibrancy` (Acrylic/Mica on the bar window) | Optional per theme |
| Windows resources | `embed-resource`: exe icon, application manifest (per-monitor DPI v2, common controls 6), version information | |
| UI tests | Rust unit tests for the bar and the Settings mapping; offscreen rendering of the Settings pages to PNG (§16) | |
| Packaging | `cargo-packager` → **NSIS per-user installer** (no admin); the portable exe needs nothing else | |

**Why a native bar.** The bar is visible all the time; Settings is opened now and then. A WebView2 bar costs a browser process tree for as long as the app runs: the M0 build (an empty webview bar) used about 360 MB across `infodock.exe` and six `msedgewebview2.exe` processes, and Settings added a seventh (about 65 MB). Comparable native bars use about 30 MB. The bar's content is small and regular (text, meters, graphs, icons, a calendar, a list popup), so drawing it directly is affordable, and it removes the IPC between bar and core. Settings uses Slint: declarative markup, Fluent widgets that follow light/dark mode, accessibility through AccessKit, and no browser engine, so Settings costs a few MB while it is open instead of WebView2's ~65 MB, and the toolchain is Rust only (plan D16).

**Why not Electron, WPF or WinUI:** they idle at 50–150 MB or more and would still need native code for the AppBar and system counters. **Why not a cross-platform Rust GUI toolkit for the bar:** the bar needs DWM materials, AppBar integration, layered popups, OLE drag & drop and UI Automation, which are Win32-level anyway; a toolkit would sit between Infodock and those APIs without saving much code.

---

## 4. Architecture

### 4.1 Process, threads and windows

One process (single instance enforced). No browser engine runs at any time.

| Window | Kind | Properties | Purpose |
|---|---|---|---|
| `bar` | native, bar thread | `WS_POPUP`; `WS_EX_TOOLWINDOW` (no taskbar button, no Alt+Tab); `WS_EX_NOACTIVATE` (clicks do not take focus, §5.3); per-pixel alpha composited by DWM, with an optional material (§5.5); z-order per settings | The bar |
| `hint` | native, bar thread | layered (`UpdateLayeredWindow`, per-pixel alpha), topmost, non-activating, click-through (`WS_EX_TRANSPARENT`), tool window; created on first use and reused | Hover popups (text hints, calendar) |
| `popup` | native, bar thread | layered, topmost, tool window, focusable; hides when it loses activation; created on first use and reused | Interactive popups (Folder Menu, Start Menu, Eject list) |
| `band` | native, bar thread | layered, click-through (`WS_EX_TRANSPARENT`), non-activating, tool window, owned by the bar window and kept right above it in the z-order; created when a skin's shadow band or the bar's shadow is first shown | A skin pack's shadow band, or the bar's shadow (§5.5), beside the bar (§5.1) |
| `settings` | Slint window, main thread | normal decorated window, opened on a route (a page such as `general`, `bar`, `sensors`, `popups`, `applications`, `help` or `about`; `layout` for the Layout manager on the Sensors page; `sensor/<id>`; `new?zone=…&index=…` or `new?target=<id>`; `welcome`), created on demand, single instance, **dropped on close** | Settings, sensor properties, sensor picker, layout manager, welcome, help, about |

Plus a **tray icon** (native, registered for the bar window, bar thread), menu: Show/Hide bar · Lock · Popups · Settings… · Quit. The menu is built from the current state each time it opens.

Threads:
* **main**: Slint's event loop — the Settings window and the global hotkey. Other threads reach it with `slint::invoke_from_event_loop`.
* **bar thread**: a COM single-threaded apartment (`OleInitialize`, needed for drag & drop) with its own message loop. It owns the bar, hint, popup and band windows, the tray icon, the sensor instances, layout, rendering, menus and native dialogs. Other threads never touch these; they send commands through the `BarController` handle, which queues the command and posts a wake-up message to the bar window.
* **sampler thread**: polls the providers and delivers samples to the bar thread's queue (§4.4).
* **worker threads**: slow work (directory listings, file operations, icon extraction, the sensors' image files); results go back to the bar thread's queue. The bar thread never reads a file a user chose: shell icons and image files (`bar/icons.rs`, `IconSource::image`) are read and decoded on one worker, kept by path and size, so a file on a network share that does not answer holds nothing up; the sensors measure again when one arrives. (The classic skin's custom image is still read where the skin is drawn, §12.4.)

**Single instance**: a named mutex (`Local\app.infodock.desktop`). A second launch finds the running bar window by its class, allows it to take the foreground and posts it a registered activation message; the running instance shows the bar and opens Settings on General (§6.5), or brings an open Settings window to the front on the page it shows (leaving the welcome page would end the first run, §11.5), and the second launch exits.

### 4.2 Rust modules

```
src/
  main.rs / lib.rs          start-up and shutdown: the capability probe, the bar thread, the sampler,
                            the main event loop (Settings window, hotkey)
  app.rs                    App (state shared by all threads; AppHost, the bar's BarHost), bar
                            visibility, quitting
  hotkey.rs                 global hotkey (main thread)
  autostart.rs              starting at sign-in: the `Run` entry follows `settings.autostart` (§10.1)
  transfer.rs               import and export as the user runs them: pickers, questions, report (main thread)
  logging.rs                the log file (§11.1) and the panic hook (§14.2)
  i18n.rs                   message catalogs (shared with Settings), plural rules, t()
  settings/                 Settings window glue (§10.7): mod.rs (opening, filling and saving the Slint
                            state, the plain pages), window.rs (the open window, the one way to it,
                            the dialogs modal to it), model.rs (pure mapping helpers: pages,
                            routes, choices, patches), appearance.rs, applications.rs, layout.rs
                            (Layout manager), picker.rs (§6.6), properties.rs and form.rs (§6.7),
                            welcome.rs (§11.5), about.rs; accessibility.rs (the accessible-name
                            test, §14.4), screenshots.rs (the pages rendered and driven, §16)
  config/                   mod.rs (ConfigError), model.rs (typed config), store.rs (load/save/atomic
                            write/debounce), migrate.rs (schema versions), patch.rs (settings patches),
                            exchange.rs (export/import files, §11.3), legacy.rs (legacy XML reading, §11.4)
  sampler/                  mod.rs (the sampler thread, ChangeSink), scheduler.rs (topic subscriptions,
                            intervals, backoff), topics.rs (topics, changes and samples)
  providers/                mod.rs (one provider per topic kind; volumes, Wi-Fi and the Recycle Bin read
                            the platform directly), cpu.rs, memory.rs, swap.rs, disk.rs, net.rs,
                            battery.rs, uptime.rs, worker.rs (readings that can stall, one worker per key)
  bar/                      platform-neutral bar logic; runs on the bar thread
    mod.rs                  BarController (the cross-thread handle), BarHost (what the bar asks of the app)
    thread/                 BarLogic (§4.1, §4.4): mod.rs (state, event dispatch, config, tray) and a
                            module per job: pointer, hover, menu, popups, sensors, external_drop,
                            keyboard, frame; tests/ split the same way
    sensors.rs              the sensor instances: creation, samples, measuring, placing, drawing
    layout.rs               zone layout (§5.4), pure
    interact.rs             presses, clicks and Lock rules, reorder drag (§6.1–6.2), pure
    focus.rs                the keyboard on the bar (§14.4), pure
    resize.rs               resizing a floating bar by its ends (§5.1), pure
    drop.rs                 drops from other apps: what each item is (§6.3)
    hint.rs, popup.rs       hover hint (§7.1) and interactive popup host (§7.2)
    placement.rs            hint/popup placement (§7.3), pure
    menus.rs, tray.rs       context menus (§6.4) and the tray menu (§6.5)
    icons.rs                shell icons and image files, read on a worker (§4.1)
    paint.rs                the bar's background; the screenshot and budget harness (§16, §14.1)
    shadow.rs               the bar's shadow band (§5.5)
  look/                     how Infodock looks, shared by the bar, the sensors and Settings
    theme.rs                resolved theme (§12.1)
    skins/                  built-in skins, skin packs, `.zip` reading, installing, tint (§12.3–12.4)
    anim.rs                 fades and easing for the rise, the hover highlight and popups (§5.5)
  sensors/
    mod.rs                  SensorType and Sensor traits, SensorCtx, LayoutCtx, DrawCtx (§8.1)
    schema.rs               property schema, validation, defaults and storage rules (§8.2, §11.2)
    registry.rs             all sensor types
    ops.rs, import.rs       operations used by Settings (§4.5); imported sensors checked by their types (§11.3)
    legacy.rs               legacy properties to props: XML keys, encodings by field type (§11.4)
    format.rs, color.rs     formatting (§8.6), colours and the dynamic gradient (§8.4–8.5)
    dates.rs                dates written with the locale's patterns (§8.6)
    popup.rs, drop.rs       the Popup trait (§7.2); the sensor a drop makes, drops onto sensors (§6.3)
    placeholder.rs          unknown and unsupported types (§4.6)
    common/                 building blocks (§8.3): two_line.rs, meter.rs, graph.rs, dual_graph.rs,
                            history.rs, icon.rs, icon_label.rs; the popups folder_browser.rs (§9.19)
                            and eject_list.rs (§9.22); eject.rs (eject notifications)
    <type>.rs               one module per sensor type or family: text.rs, image.rs, spacer.rs,
                            date.rs, time.rs, uptime.rs, cpu.rs (CPU meter, graph, dual graph),
                            memory.rs (memory and swap meters and graphs), disk.rs (Disk Meter),
                            network.rs, battery.rs, wifi.rs, recycle_bin.rs, shortcut.rs,
                            disk_launcher.rs (Disk), folder_menu.rs, start_menu.rs, power.rs, eject_usb.rs
  platform/
    mod.rs                  the Platform trait and its reading types, the bar thread's BarWindow,
                            BarHandler, UiEvent and AppBar, dialogs, app folders and the single
                            instance (§4.6)
    canvas.rs               colours, rectangles, brushes, shapes and the Canvas trait (§4.6)
    text.rs                 fonts, text styles, TextLayout and the TextEngine trait
    bitmap.rs               Bitmap (premultiplied RGBA), format sniffing, SVG via resvg, tint (§12.3)
    recording.rs            recording canvas and fixed-metrics text engine (tests on every OS, stub)
    geometry.rs             pure placement math: docking strips, monitor selection
    locale.rs, time.rs      the locale, calendar dates and the local time as plain data (§8.6)
    accessible.rs           the bar as screen readers see it, pure (§14.4)
    drag.rs, notify.rs      drag & drop items and effects (§6.3); a notification's content (§7.4)
    windows/                mod.rs (WindowsPlatform: capabilities, appearance, local time, sound),
                            window.rs (bar thread, window class, message loop, input → events),
                            appbar.rs, band.rs (shadow band window), hint.rs, popup.rs (layered
                            windows), tray.rs (notification-area icon), menu.rs (TrackPopupMenuEx),
                            dialog.rs (TaskDialogIndirect), droptarget.rs (IDropTarget), uia.rs (UI
                            Automation provider), render.rs (Direct2D canvas), text.rs (DirectWrite),
                            image.rs (WIC decoding), monitors.rs, instance.rs (single-instance lock,
                            data folders), com.rs, shell.rs (ShellExecuteEx, Recycle Bin, pickers),
                            fileops.rs (IFileOperation, .lnk/.url), icons.rs (shell icons → bitmaps),
                            autostart.rs, toast.rs, power.rs, eject.rs, device.rs (volume devices),
                            cpu.rs (PDH), memory.rs, disk.rs, net.rs, battery.rs, wlan.rs, locale.rs (NLS)
    stub.rs                 portable fallback for other OSes (development and tests only)
```

Dependencies run one way (tests aside): `app`, `settings` and the other main-thread modules (`hotkey.rs`, `transfer.rs`) use everything below them (Settings uses the bar only for skin previews, `bar::skin_preview`); `bar` uses `sensors`, `look`, `sampler`, `platform` and `config`; `sensors` use `look`, `sampler`, `platform` and `config`; `providers` and `sampler` use `platform`; `look` uses `platform` and `config`; `platform` uses only `config`'s model types, `i18n` and `logging`. Nothing below them names `settings` or `app`, and the modules are private to the crate, so the compiler reports unused items (on Windows builds; elsewhere most of the bar is unreachable).

### 4.3 Settings UI

```
ui/
  app.slint                 exports the components Rust uses
  i18n.slint                global I18n { tr(key), tr-with(key, name, value) }, backed by i18n::t
  settings.slint            SettingsWindow and the SettingsState global (pages, rows, controls)
```

The window's layout and colours follow Windows 11 Settings (§10.7). Controls bind two-way to the `SettingsState` global. Rust fills it from the config, saves each change as a settings patch (§4.5) and refreshes it from the config store's change listener, so changes made elsewhere (tray, hotkey) show up at once. Generated forms (PropertyForm, §6.7) are `for` loops over the field schemas with one control per field type (§8.2).

### 4.4 Data flow

1. On start, Rust loads the config and starts the sampler thread and the bar thread. The bar thread creates the bar window and the tray icon and instantiates one sensor per layout entry through the registry.
2. For each sensor, the bar thread asks its type for **topic requests** (`topics(props)`), e.g. `{ topic: "cpu", interval_ms: 1000 }`, and subscribes once per *(sensor, topic)*. The **scheduler** keeps the subscriber set per topic and samples at the **minimum** requested interval. Every sample goes to the bar thread's queue; slower subscribers simply use the newest sample: the bar hands a sample to a sensor only once the sensor's own interval has passed since the last one it took (a quarter of the interval early, at most 500 ms, still counts), so a 5 s graph keeps its time scale next to a 1 s meter on the same topic; a catch-up always reaches its sensor.
3. Providers are **shared**: ten CPU sensors cost one CPU sample per tick.
4. Event-driven topics (`volumes`, `battery`, `wifi`, `recycle-bin` after actions) push immediately when the OS notifies. Implementation: the OS notification reaches the bar window, the bar tells the sampler, and the sampler makes the topics it concerns due at once (`Scheduler::refresh`); a drive arriving or going refreshes `volumes`, every `disk/…` and every `path/…`. What changed is named (`Change`: devices, Recycle Bin, power, …); a sensor reports what its own action changed through `SensorCtx::changes`, a handle it can pass to the thread that finishes the action (emptying the Recycle Bin, §9.16); providers report OS notifications that arrive on other threads the same way (WLAN, §9.15). A reading made because of a change is marked *pushed* and reaches every subscriber at once, however recently it took a sample.
5. Sensors that need no system data (Text, Image, Spacer, Date, Time) ask for a **wake-up time** instead (e.g. the next minute boundary). The bar thread keeps one timer for the earliest wake-up of all sensors.
6. When a sample or wake-up changes what a sensor shows, it marks itself dirty. The bar thread re-measures dirty sensors, re-runs the layout only if a width changed, and redraws, coalescing to at most one frame per 16 ms. An open hint updates from the same sensor (rebuilt only when that sensor changed or moved, so an open calendar is not laid out again for every CPU sample).
7. Graph **history lives in the sensor instance**: a ring buffer of raw values (not pixels). It is not persisted.
8. Config changes always go through the config store, whether they come from Settings commands, the tray, the hotkey or bar actions (drag, drop, remove). The store persists them and notifies its listeners: the bar thread re-creates changed sensors (matched by id) and re-lays out; Settings receives `config-changed`.

Implementation (`sampler/`, `providers/`): topics and samples are typed (`Topic`, `Sample`). The bar thread asks a sensor for its topics once, when it creates it, and passes on only subscriptions that changed (keyed by sensor id; an empty list unsubscribes). Subscribers are sensors by their id or the Settings window, which takes the drive and battery lists while a form needs them (`sampler::Subscriber`), so a sensor whose id is `settings` stays a sensor. The scheduler samples a topic at the shortest interval its subscribers ask for (at least 100 ms), on one grid counted from its first sample, so topics with whole-second intervals are read (and drawn) together, and keeps the newest sample: every subscribe returns it, addressed to that sensor alone, so a sensor receives it at once when it joins a topic. A sensor re-created under the same id (its props changed, or a preview) keeps its topics, so the bar subscribes it again for that catch-up. A failed reading keeps the previous sample and is retried at the next tick; the third failure in a row turns into an error sample (§14.2). A topic nobody subscribes to any more is reported to the providers, which drop its baseline (a CPU topic subscribed again discards its first reading again). Disks, volumes, paths (`path/<path>`, §9.17), the Recycle Bin and batteries are read on worker threads (`providers/worker.rs`), one per key (a drive, a path), so a network drive whose server is away holds up nothing else: the sampler waits 200 ms for an answer it asks for, and does not wait again for a key still being read from an earlier tick. An answer that comes after the wait reports `Change::Answered`, so its topics are read again at once and take it (a drive plugged in shows without waiting for the next minute). Providers reach the OS through the `Platform` trait (§4.6). On resume (`PBT_APMRESUMEAUTOMATIC`) the bar tells the sampler, which lets providers take new baselines and reads every topic at once; resume and time changes (`WM_TIMECHANGE`) also reach every sensor (`Sensor::on_clock_change`), so clocks re-read the time and move their wake-ups. Clocks align their wake-ups to second and minute boundaries of the local time (`Platform::local_time`), 20 ms past the boundary.

### 4.5 Operations used by Settings

Settings runs in the same process, on the main thread, and calls the core directly; there is no IPC. Errors carry a stable code and a message (e.g. `hotkey`, `invalid`, `io`). Implementation (`config/mod.rs`): `ConfigError` has the codes `io`, `json`, `notFound`, `unknownType`, `unknownSettings` and `invalid` and a message from the catalog (§13), which Settings and the import show; its English text, with the code, goes to the log. What the machine says for machine-derived props (§8.2) is asked before the config store is locked (`ops::machine_props`, `import::prepare`, `Registry::machine_defaults` at start-up), since a type may list the drives and every thread's read of the config waits for the lock; the operations then take the answer.

| Group | Operation | Notes |
|---|---|---|
| config | get config | |
| | update settings | A deep partial of `Settings` (a JSON patch); unknown keys are rejected. A changed hotkey is registered first; if that fails nothing is saved (§6.5) |
| | add / update / move / remove sensor | `add(type, zone, index, props)` fills the machine-derived props first (so their static defaults never stand in for them), validates the props (only values that differ from the defaults, e.g. from a drop) and returns the new id. `update(id, changes)` validates the changed keys against the schema and stores the result by §11.2; it and the preview fill machine-derived props the sensor lacks the same way. `move(id, zone, index)` takes the index counted with the sensor already removed (§6.2). `remove` also drops the sensor's `state`. Implemented in `sensors/ops.rs` |
| | preview sensor | The bar shows these props without saving them; ending the preview restores the saved props (§6.7) |
| | export / import config | `replace` or `merge` → `ImportReport` (§11.3) |
| | import legacy layout | §11.4 |
| sensors | sensor types | Id, category, localised name and description, `requires`, field schema (§8.2) and this machine's derived defaults |
| system | capabilities; lists of volumes, network adapters, batteries, monitors, fonts, skins | |
| | install / remove skin pack; skin preview | Previews are rendered by the bar renderer (§12.4) |
| shell | open the logs or skins folder | |
| app | version, hotkey status, autostart, quit | |

The bar opens Settings on a route (e.g. `sensor/<id>`, `new?zone=left&index=2`) by calling the core directly.

### 4.6 Platform abstraction

Thread-safe services (`Send + Sync`), shared by all threads: one `Platform` trait (`platform/mod.rs`), passed in as `Arc<dyn Platform>` (`platform::current()` is called once, at start-up), so tests replace it with fakes. Its methods fall into groups, each used by a clear set of callers:

```rust
pub trait Platform: Send + Sync {
    // Machine and session (all callers)
    fn capabilities(&self) -> Capabilities;               // which sensors/features exist here
    fn probed_capabilities(&self) -> Option<Capabilities>; // without waiting for the probe (§14.1)
    fn monitors(&self) -> Vec<MonitorInfo>;               // id, name, bounds, work area, scale, primary
    fn appearance(&self) -> OsAppearance;                 // light/dark, accent colour, transparency, animations
    fn locale(&self, which: DateLocale) -> Locale;        // plain data: names, date patterns, hour cycle,
                                                          // first day of week, decimal separator (§8.6)
    fn local_time(&self) -> LocalTime;
    fn font_families(&self) -> Vec<String>;
    // Readings (providers, Settings' lists): may block, never called on the bar thread
    fn uptime(&self) -> Duration;
    fn logical_processors(&self) -> u32;
    fn cpu_reader(&self) -> Option<Box<dyn CpuReader>>;
    fn memory(&self) -> Result<MemoryStatus, String>;
    fn page_files(&self) -> Result<Vec<PageFile>, String>;
    fn volumes(&self) -> Vec<Volume>;
    fn disk_space(&self, root: &str) -> Option<DiskSpace>;
    fn net_interfaces(&self) -> Vec<NetInterface>;
    fn net_interface(&self, id: Option<&str>) -> Option<NetInterface>;
    fn power_status(&self) -> Result<PowerStatus, String>;
    fn wifi_reader(&self, changed: Box<dyn Fn() + Send + Sync>) -> Option<Box<dyn WifiReader>>;
    fn recycle_bin(&self) -> Result<RecycleBin, String>;
    fn path_kind(&self, path: &str) -> PathKind;
    fn list_dir(&self, path: &str) -> Result<Vec<DirItem>, String>;
    fn shell_icon(&self, path: &str, px: u32) -> Option<Bitmap>;
    // Shell (sensors): actions return at once and run in the background
    fn open(&self, target: &str);
    fn launch(&self, launch: &Launch);
    fn reveal(&self, path: &str);
    fn known_folder(&self, folder: KnownFolder) -> Option<String>;
    fn compare_names(&self, a: &str, b: &str) -> Ordering;
    fn volume_of(&self, path: &str) -> Option<String>;
    fn file_operation(&self, op: FileOp, paths: Vec<String>, folder: String, done: Box<dyn FnOnce() + Send>);
    fn recycle(&self, paths: Vec<String>, done: Box<dyn FnOnce() + Send>);
    fn create_url_file(&self, folder: &str, stem: &str, url: &str);
    fn empty_recycle_bin(&self, done: Box<dyn FnOnce() + Send>);
    // Power, devices, notifications, sound (sensors)
    fn power(&self, action: PowerAction, force: bool);
    fn eject(&self, root: &str, report: EjectReport);
    fn notify(&self, title: &str, body: &str);
    fn play_sound(&self, wav: &'static [u8]);
    // System (Settings, the app)
    fn autostart(&self, name: &str) -> Option<String>;
    fn set_autostart(&self, name: &str, change: &AutostartChange) -> Result<(), String>;
    fn decode_code_page(&self, bytes: &[u8], code_page: Option<u32>) -> Option<String>;
}
pub struct Capabilities {
    pub appbar: bool, pub recycle_bin: bool, pub start_menu: bool, pub wifi: bool,
    pub battery: bool, pub power_modes: Vec<String>, pub eject: bool, pub per_core_cpu: bool,
}
```

Only `capabilities`, `monitors`, `appearance` and `uptime` are required; every other method has a default that is what other OSes get (the stub, `platform/stub.rs`, implements just the four). Windows (`platform/windows/`) overrides all of them except three that are portable (`open`, which calls `launch`; `volume_of`, by the path's form; `create_url_file`, plain file I/O).

Conventions, stated on the trait:
* **Blocking.** A reading may wait for slow hardware or a network drive whose server is away, so none runs on the bar thread: the sampler reads the CPU, memory, page files, network and Wi-Fi itself, and volumes, disk space, paths, batteries and the Recycle Bin on its workers (§4.4); folder listings and shell icons are read on the bar's workers. Settings reads volumes (machine defaults), network adapters and fonts on the main thread. The one accepted exception: a drop onto the bar reads `path_kind` of each dropped item on the bar thread, once per drop and never per drag-over, since the sensors it creates depend on it (§6.3). Actions return at once and do their work in the background (a short-lived thread with COM, or the shell's own).
* **Absence and failure.** A reading that can fail returns `Result<_, String>` with the OS's message (memory, page files, folder listings, power status, the Recycle Bin, the CPU and Wi-Fi readers), which the sampler turns into an error sample after three failures in a row (§14.2). Disk space, volumes, network interfaces and `path_kind` fold a failure into absence (`None`, an empty list, `Missing`), which sensors show as missing or "n/a". Typed errors (`EjectError`, `ImageError`) are for callers that act on the reason.

UI services, used only on the bar thread (not `Send`), are not part of `Platform`; `start_bar_thread(make_handler)` starts the bar thread and returns its `AppBar`, tray icon and waker:

* **Bar window** (`BarWindow`, the bar logic's view of the native window, §4.1): invalidate, material, tray icon, native menus at the pointer or beside the focused sensor, one-shot timers by `TimerId` (the sensors' earliest wake-up, §4.4; hint delays; animation frames; …), the hint and popup windows (shown at a physical rect with what a closure paints, faded, updated in place), pointer capture and cursors, the keyboard focus, confirmation dialogs, moving and resizing a floating bar, and the slide of the rise animation. Input arrives at the bar's `BarHandler` as platform-neutral `UiEvent`s: pointer move/leave/down/up (button, modifiers), wheel, key, timer, popup input, focus, UI Automation requests, and the OS broadcasts the bar cares about (resume, time change, regional format, device and power changes, appearance change); drags from other apps arrive as `DragEvent`s (§6.3). The handler also paints the bar into a canvas and describes the shadow band and the accessible tree after each frame.
* **Canvas**: drawing in logical px at the window's scale: filled and stroked rectangles, rounded rectangles, lines and polygons, linear gradients, text layouts, bitmaps (with opacity), clipping and layer opacity. **Text**: layout and measurement with a font family, size, weight and style, optional maximum width with an end ellipsis, tabular numerals (`text_engine()`). **Bitmaps**: decode files (PNG, JPEG, GIF, BMP, ICO; SVG through `resvg`) and wrap RGBA buffers.
* **Menus** and **dialogs**: native popup menus (checkable items, separators, submenus) through `BarWindow::show_menu`, and confirmations through `BarWindow::confirm`. Flows that ask in steps (an import, §11.3) go through the `Dialogs` trait: open and save pickers, a question with command links, a message with OK; `NativeDialogs` shows them modal to a given window (`OwnerWindow`, the native handle the platform takes from a window's `raw-window-handle`) on the calling thread. Settings has one way to them, `settings::dialogs()`, modal to its window while it is open; every picker of Settings and the import and export go through it.
* **Tray icon**: shown, updated or removed through the bar window; clicks and menu requests arrive as events; removable from any thread on exit (`TrayIcon`).
* **AppBar**: attached to the bar window when the bar thread starts, because it needs that window and its thread; `configure(DockRequest)`, `refresh`, `raise`, `release` from any thread.
* **Accessibility**: a UI Automation provider for the bar (§14.4), fed the `AccessibleBar` the handler describes.

On Windows the canvas is Direct2D with a **software** render target drawing into a premultiplied 32-bit bitmap, with DirectWrite text (grayscale antialiasing, since the background is transparent). A full 28 px bar is cheap to rasterise on the CPU, and no GPU driver is loaded into the process. Presentation: the bar copies the bitmap to its DWM-composited window; hint and popup use `UpdateLayeredWindow`; `render_offscreen` returns it as a `Bitmap` (skin previews, CI screenshots). A recording canvas with a fixed-metrics text engine records draw calls, so layout and rendering are unit-testable on any OS; the stub platform uses it as its renderer.

Implementation notes:
* **Text** is laid out by a `TextEngine` (DirectWrite on Windows), separate from the canvas, because sensors measure (§8.1 `measure`) outside drawing. A `TextLayout` carries its size and first baseline and is drawn with `draw_text`. A font family that is not installed falls back to Segoe UI (Segoe UI Variable exists from Windows 11 on).
* **Bitmaps** are premultiplied RGBA in shared memory, so worker threads can decode them. Only the formats above are decoded, recognised by their first bytes, so third-party WIC codecs never see Infodock's files; decoding to a requested height scales with high-quality filtering (an `.ico` picks its closest frame). The renderer uploads each bitmap once and keeps it while the bitmap lives. WIC decodes with its built-in decoder for the recognised container (`GUID_VendorMicrosoftBuiltIn`). Limits, so that a small file cannot ask for gigabytes: files up to 32 MB; results up to 8192 px a side; raster sources up to 64 megapixels (decoders read them whole), while an SVG may declare any size as long as the result fits; an `.svgz` is inflated to at most 32 MB; images embedded in an SVG (data URLs) are skipped above 16 megapixels, embedded `.svgz` inflated with the same cap, and images referenced by path are not loaded. Bitmaps are drawn from whole physical px, so they stay sharp at 125 % and 150 %.
* **Clips and opacity layers** are pushed and popped; layers still open at the end of a frame are closed.

Sensor clicks open programs, files and URIs with `Platform::open`, or `Platform::launch` with parameters, a working folder and the `runas` verb (`ShellExecuteExW` on a short-lived thread with COM, so the bar thread never waits on the shell; a declined elevation prompt is only logged).

**Autostart** (`Platform::autostart(name)` and `set_autostart(name, change)`, `platform/windows/autostart.rs`): reads and changes the value `name` of `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` (`RegGetValueW`, `RegSetKeyValueW`, `RegDeleteKeyValueW`) and Task Manager's mark for it under `…\Explorer\StartupApproved\Run`; a change enables (writes and clears the mark), updates (writes, keeping the mark) or removes (both). Other OSes start nothing at sign-in (§10.1).

**Shell icons** (`Platform::shell_icon(path, px)`, `platform/windows/icons.rs`): `IShellItemImageFactory::GetImage(SIIGBF_ICONONLY)` at the physical size, read with `GetDIBits`; the shell hands out premultiplied alpha, but a pixel whose colour exceeds its alpha marks a straight-alpha icon, and an icon without any alpha is opaque. Shell calls run in a single-threaded COM apartment entered once per thread (`platform/windows/com.rs`) with critical-error dialogs off. The bar reads icons on its `icons` worker thread (`bar/icons.rs`), keyed by path and size; sensors ask while they measure (`LayoutCtx::icons`), get *loading* until the icon arrives, and are measured again when it does (the worker wakes the bar thread through the waker its handler is built with). At most 256 icons are kept; beyond that the ready ones are dropped and read again when asked. `Platform::path_kind` (`GetFileAttributesW`, no dialogs) tells a missing path from a file or folder; it is read on the `path/<path>` topic, off the bar thread, and once per drop onto the bar (the accepted exception above).

Capabilities are read once per run on Windows: `recycle_bin` when the shell knows the Recycle Bin folder (`SHGetKnownFolderIDList`, no drive is touched), `start_menu` when the per-user Programs folder exists, `wifi` when the WLAN service answers and has a wireless interface (`wlanapi.dll` is loaded only for that, since editions without wireless LAN lack it), `battery` unless `GetSystemPowerStatus` says there is no system battery (or does not know), `eject` always (Eject USB lists what there is). A Wi-Fi adapter plugged in later is found at the next start. They are probed on a thread of their own as soon as the process starts, so the probing overlaps the rest of the start-up (§14.1); the slow probes (the per-processor counters, the WLAN service, the shell's folders) run side by side, and whoever asks first waits for the answer. The bar does not wait: until the probe has answered (`Platform::probed_capabilities`), it shows only the sensors whose type needs no capability, so its first frame comes without them and none shows as unavailable for a moment; it checks every 25 ms, and the others join once the answer is there (after 2 s it waits for a probe that never started).

`per_core_cpu` means per-processor counters can be read. It does not depend on the number of logical processors: a single-processor system still has it. Windows checks once whether the per-processor `% Processor Utility` counter can be added (it cannot while the performance counters are broken).

Sensor types declare `requires: &[Capability]`. The picker hides types whose capabilities are missing. Configured sensors that are unsupported on the current platform render as a dimmed placeholder with the hint "Not available on this system". Entries of an unknown type (from a newer version, or a v1 type that arrives in a later milestone) render the same way, labelled with the type id and the hint "This sensor ({type}) is not available in this version of Infodock". The placeholder is an outlined label in the sensor font at reduced opacity; the entry stays in the config unchanged.

---

## 5. The bar

### 5.1 Geometry and placement

* **Edge**: `top` or `bottom` (default `bottom`).
* **Monitor**: `primary` (default) or a specific monitor, identified by device name (e.g. `\\.\DISPLAY2`) with fallback to friendly name + resolution. If the chosen monitor is absent, use primary and keep the setting for when it returns.
* **Width**: the full width of the monitor's work area as negotiated by the AppBar protocol (other appbars are respected). Configurable in floating mode.
* **Height**: theme default (modern **28** logical px, classic **25**), user-adjustable 22–40. All coordinates are logical (DIP); Rust converts to physical px using the monitor scale.
* **Shadow band**: a skin MAY define a shadow band (§12.4); the bar's `shadow` (§5.5) is shown as one when the skin has none. It sits beside the bar on the side facing the screen center (above a bar at the bottom, below one at the top; for a floating bar, away from the nearer edge), outside the reserved work area, is transparent and MUST be click-through. Implementation: the bar logic hands the band, drawn with the skin, to the platform after each frame (`BarHandler::band`); the `band` window (§4.1) shows it, moves with the bar window (`WM_WINDOWPOSCHANGED`), and hides while the bar is hidden or off its place during the rise and slide (§5.5). The bar window itself stays as tall as the bar, so the AppBar, hints, popups, floating and the animations work from the bar alone.
* **Modes**
  * **Docked** (default): attached to the chosen edge; optionally **reserves space** (§5.2).
  * **Floating**: free position, draggable by the bar background while unlocked (drag threshold 4 px), and resizable by its ends: within 6 logical px of its left or right end the pointer shows the resize cursor, and dragging that end changes the width (200–3840 logical px, within the work area of its monitor) while the other end stays. Position and width are persisted. No space reservation.
  * Implementation (`platform/geometry.rs` `floating_rect`, `appbar.rs`, `bar/thread/pointer.rs`): a floating bar is kept inside the work area of the monitor holding the largest share of it, at that monitor's scale (or the one nearest to its top-left corner); before it was ever moved or sized it starts 800 px wide, centred, 12 px from the work area's `edge` on the selected monitor. While it is dragged the window follows the pointer; the position is stored when the button is released (or the capture is lost). An end follows the pointer at once, without the drag threshold (`bar/resize.rs`, pure and tested: the ends' hit-test and the rect while one is dragged, from the screen x of the pointer, since the window moves with its left end; `BarWindow::resize_to` puts the window there); the width and position are stored on release as the position is, and the *Width* setting (§10.2) follows. Its hints open away from the nearer screen edge.

### 5.2 Work-area reservation (AppBar)

When `reserveSpace = true` (default) and docked:

1. Register with `SHAppBarMessage(ABM_NEW)` using a private callback message.
2. `ABM_QUERYPOS` → adjust → `ABM_SETPOS` with `ABE_TOP`/`ABE_BOTTOM` and the monitor rect; then move the window to the returned rect.
3. Callback handling: `ABN_POSCHANGED` → repeat step 2; `ABN_FULLSCREENAPP` → §5.3; `ABN_STATECHANGE` → repeat step 2.
4. Repeat step 2 on `WM_DISPLAYCHANGE`, `WM_DPICHANGED`, `WM_SETTINGCHANGE (SPI_SETWORKAREA)`, and after edge, monitor or height changes.
5. **Explorer restart**: on the registered `TaskbarCreated` message → `ABM_REMOVE` + `ABM_NEW` + reposition.
6. On exit (and in a panic hook): `ABM_REMOVE`. If the process is killed, Windows removes the appbar together with the window, so no recovery logic is needed.

When `reserveSpace = false`: place the window at the edge without registering; maximised windows may overlap it.

Implementation note: the AppBar is registered for the bar window itself. Infodock creates that window and owns its window procedure, which receives the callback message, `WM_ACTIVATE`/`WM_WINDOWPOSCHANGED` (forwarded as `ABM_ACTIVATE`/`ABM_WINDOWPOSCHANGED`) and the broadcasts above. A hidden message-only window does not work: message-only windows receive no broadcasts (`TaskbarCreated`, `WM_DISPLAYCHANGE`, `WM_SETTINGCHANGE`), and an appbar registered under a different window than the visible bar loses the shell's z-order coupling. The window procedure also keeps other code (Aero Snap, DPI-change resizes) from moving the bar or lifting it out of its z-order.

### 5.3 Z-order, full-screen, focus, power

* `zOrder`: `topmost` (default, usual for appbars) | `normal` | `bottom` (always behind normal windows). With `bottom`, the hotkey brings the bar to the front until it loses focus.
* **Full-screen apps** (`hideOnFullscreen`, default true): on `ABN_FULLSCREENAPP(true)` for our monitor → drop topmost and hide the hint/popup windows; restore on `false`.
* **Sleep/resume**: `WM_POWERBROADCAST / PBT_APMRESUMEAUTOMATIC` → emit `system-resume`. Providers reset delta baselines (CPU, network); the UI re-renders Date/Time/Uptime immediately.
* **Time or time-zone changes** (`WM_TIMECHANGE`) → Date/Time refresh.
* Clicking a sensor MUST NOT steal focus from the user's window, except for actions that open UI. The bar window has `WS_EX_NOACTIVATE`; the hotkey still focuses it explicitly (`SetForegroundWindow`) for keyboard use (§14.4).

### 5.4 Zones and layout algorithm

Three ordered zones: `left`, `center`, `right`. **Each zone's array is stored in visual left→right order.**

```
pad = 7            // logical px edge padding
gap = settings.sensorSpacing (1..20, default 10)
left:   x = pad; for s in left:   place(s, x);  x += width(s) + gap
center: total = Σ(width(s) + gap) over center; x = round((barWidth − total)/2)
        for s in center: place(s, x); x += width(s) + gap
right:  x = barWidth − pad; for s in reverse(right): x −= width(s); place(s, x); x −= gap
```

(The center total deliberately includes one trailing gap.)

* Sensor width is **intrinsic** (content), except for sensors with a width property (Spacer, graphs).
* When the zones do not fit the bar with a gap between them and the padding at its ends, sensors whose text can be cut give way with an ellipsis: the center zone first, then left, then right, each zone only as far as its own overlap needs (the left and right zones as far as they meet the center, or each other when the center is empty), its sensors in proportion to what they can give, in whole px, down to a minimum each. What still does not fit overlaps; the center zone is drawn on top. The Text sensor gives way, down to two ems of its font size (plus its shadow); its hint shows the whole text while it is cut (§9.1). Other sensors keep their width: values have reserved widths, and the launchers show only icons (§9.17–9.19).
* Implementation: widths and positions computed by `bar/layout.rs` (pure, tested functions: `fit`, then `place`) from the widths each sensor measures (`measure()`, §8.1) and the minimums it can be cut to (`min_width()`); a sensor given less than it measured is told so (`fit()`), and lays out its text again with that maximum width (DirectWrite trimming with an ellipsis sign). Values use tabular numerals (DirectWrite `tnum`) and per-sensor reserved minimum widths (§9) so changing numbers do not shift the bar.

### 5.5 Appearance

* **Background**: theme/skin (§12) + `background.color` (tint), `background.opacity`, optional material (Acrylic/Mica), optional `shadow`.
* **Shadow** (`shadow`, default false): a soft shadow 6 px deep beside the bar, where a shadow band goes (§5.1: on the side facing the screen center, outside the reserved work area, click-through), in either theme: black at about 18 % next to the bar, fading to nothing, like the popups' shadow (§7.1). A skin with a shadow band of its own (§12.4) shows its band instead, whatever the setting. Implementation (`bar/shadow.rs`): black rows over the bar's whole width, 6 logical px in whole physical px, each row's alpha 18 % × (1 − d / depth)² for its distance d from the bar; kept until the bar's width, scale or side changes, and handed to the platform after each frame like a skin's band (`BarHandler::band`).
* **Highlight on hover** (`highlightOnHover`, default false): on pointer enter, background opacity → min(100 %, opacity × 1.7) with a 300 ms ease-in; back with a 300 ms ease-out on leave.
* Implementation (`bar/anim.rs`): fades are values 0–1 with cubic easing that a reversal resumes from where they are (taking the remaining share of the time). The bar's frame timer (16 ms) runs only while one moves; sensors animate through their wake-ups (§4.4). `animations` (§10.2): `auto` follows the OS "Animation effects" (`SPI_GETCLIENTAREAANIMATION`, re-read when `WM_SETTINGCHANGE` reports it); without animations every transition jumps to its end.
* Rise implementation: the bar thread moves the window off its place (`BarWindow::slide`, physical px) and paints its content in an opacity layer; a floating bar only fades. When a docked bar's edge changes, the controller does not redock it; the bar thread slides it out, then asks for the redock (`BarHost::redock`) and rises at the new edge once it is placed; if it is already placed at that edge (the edge changed back meanwhile), no placement follows and it rises at once. Without animations, or while the bar is hidden, it redocks at once. The window is moved only when its offset changes, and a placement that changes nothing keeps the offset. The hairline (§12.2) is drawn on the edge the bar is placed on: the nearer one for a floating bar, the old one while it slides out.
* **Rise animation**: on startup and after an edge change, the bar slides in from off-screen on its edge while its content fades 0→1 over **300 ms, ease-out**. After an edge change, the bar first slides out toward the old edge (300 ms). If the OS "animation effects" setting is off (`SPI_GETCLIENTAREAANIMATION`), there is no animation. The material (if any) moves with the window; only the drawn content fades.
* **Sensor text colour**: `settings.sensors.textColor` (default `#FFFFFF`) applies to all labels and to icons in "standard" colour mode.

---

## 6. Interaction

### 6.1 Lock mode and clicks

* `locked` (default false; context-menu item "Lock" ✓).
* **Unlocked**: pointer-down on a sensor dims it (opacity −40 %). Moving more than 4 px starts a **reorder drag** (§6.2). Releasing without moving is a **click**. The bar captures the pointer while the button is down, so the release is seen anywhere; releasing outside the pressed sensor is no click.
* **Locked**: reordering is disabled. Click behaviour follows `clicksWhenLocked`:
  * `always` (default): clicks work normally.
  * `altOnly`: clicks only work while Alt is held. This makes the bar a passive display.
* While locked, a press dims the sensor only when a click would reach it (`always`, or Alt held with `altOnly`); otherwise the press does nothing.
* Hint texts end with a "Click to …" line only when a click would actually do something in the current mode.
* Double-click and middle-click have no default action (reserved).

### 6.2 Reordering by drag

* Only while unlocked. During the drag, hints are hidden and suppressed.
* Over a sensor, a **placer** (2 px vertical line in the accent colour) shows the drop position: **left** of the target if the pointer is over its left half, otherwise right of it.
* Over an empty area: the zone is chosen by the pointer's x in **thirds** of the bar width. The placer appears at that zone's insertion point: left zone → after its last sensor (or at `pad`); center → after its last sensor (or at the center); right zone → before its first sensor (or at `barWidth − pad`).
* Drop → `move_sensor(id, zone, index)`. The index accounts for removing the source first: within the same zone, if the source is before the target, the target index shifts down by one.
* Esc cancels the drag. The bar does not take focus, so Esc is read with `GetAsyncKeyState` while the drag runs (pointer captured), every 50 ms. Losing the pointer capture to another window cancels it too.
* The dragged sensor stays dimmed in place during the drag. The placer is centred in the gap at the drop position: half a gap left of the target or right of it; in an empty zone at `pad`, the bar's centre or `barWidth − pad`. Dropping where the sensor already is (next to itself) moves nothing. A press that moves past 4 px while locked is neither a drag nor a click.

### 6.3 External drag & drop onto the bar

Sources: Explorer items (paths) and browser links (URLs).

| Dropped | On empty bar area | On a sensor |
|---|---|---|
| Drive root (`X:\`) | New **Disk** sensor | Insert a new Disk before/after the target (by pointer half) |
| Folder | New **Folder Menu** | Target is Folder Menu → file operation into its folder (§9.19); otherwise insert a new Folder Menu |
| File | New **Shortcut** | Folder Menu → file operation; Recycle Bin → move to Recycle Bin; otherwise insert a new Shortcut |
| URL (`http`, `https`, `ftp`) | New **Shortcut** with that URL (title from the link text, if provided) | Folder Menu → create a `.url` file; otherwise insert a new Shortcut |
| Several items | One sensor per item, in order | Same rule per item |
| Anything else | Ignored (no-drop cursor) | Ignored |

* Sensors created by a drop are added **without** opening the Properties dialog.
* Zone and index come from the drop position, using the rules of §6.2.
* **Implementation**: an `IDropTarget` registered on the bar window (`RegisterDragDrop`, bar thread). It reads `CF_HDROP` (paths), `CFSTR_INETURLW` / `UniformResourceLocatorW` (URLs, with the link text from `FileGroupDescriptorW` or the HTML fragment when offered) and `CFSTR_SHELLIDLIST` (drives and virtual items that have a file-system path), and takes modifier keys from `grfKeyState`. Each path is classified (file, folder or drive, and its volume) through the platform shell. The drop effect shown by the cursor follows the target (§9.16, §9.19); the Explorer drag image is kept with `IDropTargetHelper`.
* The target hands the bar logic enter, over, leave and drop events (`BarHandler::on_drag`, `bar/drop.rs`), which answer with the effect at once; while a menu is open nothing can be dropped. Paths come from `CF_HDROP`, else from the `Shell IDList Array`; a drag with neither is read as a URL (`UniformResourceLocatorW`, else `UniformResourceLocator`), its title from the `.url` file the browser names in `FileGroupDescriptorW`, else from the first link's text in `HTML Format` (a title that is the address itself is none). Items are classified by their form while dragging, so a drag touches no drive (OLE's callbacks are synchronous, and the source waits too): a drive root is `X:\`, anything else a path, and a URL must not contain control characters (they would add lines to a `.url` file); whether a path is a file or a folder is asked (`Platform::path_kind`) only when a drop makes sensors, and missing ones are left out then. A `Shell IDList Array` item reaches the shell only if its ID list ends inside the data. While the drag is over the bar the placer of §6.2 shows where the new sensors go, and the pointer shows a link (a copy where the source allows no link); drops are taken whether the bar is locked or not. The new sensors are added in one config change (`BarHost::add_sensors`); machine-derived fields the drop sets (a Disk's drive) are not asked of the machine. The sensor each item makes is the sensors' (`sensors/drop.rs` `new_sensor`, naming the types through their modules), and a test adds each through `ops::add` with the real registry, so a renamed type or prop fails there rather than at a drop. The drag image helper is created at the first drag, so it costs nothing until then.
* Onto a sensor: the bar asks the sensor under the pointer (`Sensor::drop_action`, again only when the sensor or the keys change) whether it takes the items; Folder Menu and Recycle Bin take them when every item is something they handle (all files and folders, or for a Folder Menu all links), and otherwise the bar inserts new sensors as above. A sensor that takes the drop is darkened, the floating drop hint (the hint window, without delay) says what would happen, and no placer shows; `Sensor::drop_items` does it. A sensor moves items itself (an optimized move, as the shell calls it), so after a move the target answers `DROPEFFECT_NONE` and sets `Performed DropEffect` to none (and `Logical Performed DropEffect` to move) on the data object: the source deletes nothing. A sensor may also take a drop it cannot do (moving a folder into itself, an effect the source does not allow): no-drop pointer, no hint.

### 6.4 Context menus

Native menus (`TrackPopupMenuEx` on the bar thread, at the cursor). Confirmations use native dialogs (`TaskDialogIndirect`).

**Bar background menu**

```
New sensor…                 (or a submenu by category if settings.newSensorMenu = "submenu")
────────
Tools ▸  Layout manager…
         Import layout…
         Export layout…
         Import legacy XML layout…
         ────────
         Help
Settings…
────────
✓ Lock
✓ Popups
────────
Quit Infodock
```

**Sensor menu**

```
New sensor…                 (inserts relative to this sensor; the picker shows "Insert: Before | After")
Properties…                 (disabled if the type has no visible properties)
Remove sensor               (confirmation: "Remove sensor '{name}'?")
────────
<sensor-specific items>     (e.g. Eject; Open/Empty Recycle Bin; Open folder)
────────
Tools ▸ …                   (same as the bar menu)
Settings…
✓ Lock
✓ Popups
```

Sensor-specific items come from the sensor type's `menu()` (§8.1). Opening a menu hides any hint/popup. The menu opens on the right button's release, or from the keyboard beside the focused sensor (§14.4). *Tools ▸ Help* opens Settings ▸ Help (route `help`). *Import layout…* and *Export layout…* run on the main thread (§11.3), so the bar goes on while their pickers are open. "New sensor…" opens Settings on `new?zone=<zone>&index=<index>` (bar menu: the zone by thirds and its insertion point, §6.6) or `new?target=<id>` (sensor menu); in submenu mode, choosing a type adds it after the target sensor (or at the zone's insertion point) and opens its Properties (`sensor/<id>`). Implementation (`bar/menus.rs`): the sensor's items (`MenuItem`: its own id, label, enabled) are asked for each time the menu opens; the chosen one goes back to the same sensor (`on_menu`), which redraws if what it shows changed. Without own items the menu has no second separator.

### 6.5 Global hotkey and tray

* `hotkey` (default `Ctrl+Alt+Shift+I`) with `hotkeyAction`: `bringToFront` (default) | `toggleVisibility`. Either way a bar it leaves shown takes the keyboard focus (§14.4); one it hides gives it back.
* Settings records the hotkey: the user selects the hotkey button and presses the new combination; Esc, leaving the window or leaving the General page cancels. Keys are read as physical keys, so the keyboard layout does not matter (Ctrl+Alt is AltGr on many layouts). A hotkey needs Ctrl, Alt or Win; only F1–F24 may stand alone, so a hotkey never takes a key away from typing. A refused combination is explained and recording goes on.
* A recorded hotkey is registered before it is saved. If registration fails (typically another app already uses the combination), Settings shows the error next to the button and the previous hotkey stays; since that one still works, the error is not shown again after leaving the page. A hotkey that cannot be registered at start-up is shown the same way; the app MUST still start. Recording the same combination again retries it.
* Tray icon: left-click toggles bar visibility; right-click opens the tray menu (§4.1). It can be hidden (`showTrayIcon`, default true). If both the tray icon and the bar are hidden, the hotkey or a second launch (single instance → show the bar and open Settings) brings it back.

### 6.6 Sensor picker and creation flow

* Modal page in the Settings window (`/settings/new?zone=&index=&target=`); the bar's "New sensor…" opens Settings on that route.
* Sensor types grouped by **category**, sorted alphabetically by localised name; each item shows name and description. A search box filters by name or description.
* With a target sensor: an **Insert: Before / After** selector (default After) and the text "… the '{target name}' sensor".
* **Add** (or double-click) → `add_sensor` with defaults → immediately opens **Properties** for the new instance. **Cancel** in Properties for a *newly created* sensor removes it again. Sensors created by drop skip this step.
* Insertion point when not targeting a sensor: the zone under the right-click position (thirds). Left and center zones append at the end; the right zone inserts at its **start** (next to the center).
* Implementation (`settings/picker.rs`): a click selects, Enter in the search box adds the selection. A search keeps the selection while it is still listed and otherwise selects the first match; Up and Down in the search box move the selection, so the picker works from the keyboard. Only the list scrolls (the search box, the insert choice and the buttons stay), and it follows the selection. A route without a zone uses the left zone; without an index, the insertion point above. A target that has been removed meanwhile inserts at the end of the left zone. Leaving the page by the navigation drops it.

### 6.7 Properties dialog

* Route `/settings/sensor/<id>`. The form is generated from the sensor's `props` schema (§8.2); `hidden` fields are not shown.
* Changes **preview live** on the bar (debounced 150 ms, `preview_sensor`). **Save** persists (`update_sensor`) and ends the preview; **Cancel**, or closing Settings, ends the preview, which restores the saved props.
* Implementation (`settings/form.rs`, `settings/properties.rs`): one row per field in schema order, with one control per field type: text box (`text`), switch (`bool`), list (`select`, `font` with a first *Default* entry, `fontStyle`), slider with its unit (`slider`, `interval` in s, `opacity` in %), colour box (`color`), mode list plus a colour box for Custom (`colorMode`), path box with *Browse…* (`file` with its extensions, `folder`; `IFileOpenDialog`). Rows hidden by `visibleIf` disappear at once. Text and path boxes preview as they are typed, colour boxes once the text is a colour (as on the settings pages, §10.7); sliders when released. A colour box reports every edit, so text that is not a colour shows its row's error and disables Save at once (Save is a click that does not take the focus). Leaving the page by the navigation counts as Cancel. Drive and battery lists can be slow to read (a network drive, battery firmware), so the main thread never reads them: while a form with such a field is open, Settings subscribes to the `volumes` and `batteries` topics (`App::watch_lists`), whose newest sample arrives at once when the bar reads them already, and the lists follow drives and batteries as they come and go; until a list arrives, the field offers only its stored choice, unmarked. The network connections are read when the form opens (`GetIfTable2`, which does not wait).
* Validation per field type; invalid fields show inline errors and disable Save, as do required fields while they are empty (§8.2), also when the form opens (a new Folder Menu cannot be saved until its folder is chosen).
* Each field shows its description as helper text.

---

## 7. Popups

### 7.1 Hover hint

* Trigger: the pointer enters a sensor, `popupsEnabled` is true, no drag is in progress, and the sensor returns a non-empty hint.
* **Show delay** 600 ms (`popups.showDelayMs`); **hide delay** 100 ms after the pointer leaves (`popups.hideDelayMs`). Moving to another sensor within the hide delay swaps the content immediately, without a new show delay.
* While visible, the hint **updates live** when the sensor's data changes (CPU %, battery time…).
* Content (`HintContent`): `Text(String)` (multi-line plain text; `\n` line breaks; tab-aligned "label: value" columns) or `Rich(Box<dyn RichHint>)` for hints that measure and draw themselves (Date calendar). A rich hint lays itself out (`layout` → its size) and draws itself at an origin in the popup font and colours; the hint window frames it like text.
* Visuals: rounded rectangle (modern radius 8 px, classic 6 px); background `popups.background` at `popups.opacity`; text `popups.textColor`; font `popups.font` / `popups.fontStyle` / `popups.fontSize` (9–15, default 12); optional shadow (`popups.shadow`: a soft shadow drawn into the layered window's margin); padding 6 px; minimum size 30×20. A **pointer triangle** 13×7 points at the sensor (downward when the bar is at the bottom, upward at the top).
* The hint is click-through. It closes when the pointer leaves the sensor, on any click, when a menu opens and when a drag starts.
* After a click, a menu or a drag, the hint stays closed while the pointer remains over that sensor; it shows again over another sensor (or after leaving and coming back). Full-screen apps (§5.3) and turning `popupsEnabled` off close it too.
* Implementation: `Text` hints split lines at `\n` and cells at `\t`; each column is as wide as its widest cell (a row's last cell may run on), with a 12 px gap. The shadow is a few rings of black at low alpha around the bubble, in a 6 px window margin (none without `popups.shadow`). The hint window is owned by the bar window; the bar's timers drive the show and hide delays and the fade-in.

### 7.2 Interactive popups

* Opened by a sensor click (Folder Menu, Start Menu, Eject USB). Popup kinds are provided by sensor types: a popup measures, draws and handles its own input inside the shared `popup` window.
* Focusable. Closes on **blur**, **Esc**, a second click on the same sensor, or after an action that opens something.
* A sensor whose popup has just closed ignores clicks for 500 ms, so the click that caused the blur does not reopen it.
* While open, the sensor shows an **active** state (subtle highlight behind it).
* Implementation (`sensors/popup.rs`, `bar/popup.rs`, `platform/windows/popup.rs`): a click returns `ClickResult::Popup(Box<dyn Popup>)`. A popup lays itself out within the room beside the bar (the monitor's width, the height on its side of the bar, less margins, pointer, shadow and padding), draws itself, takes input in its own coordinates (pointer, the wheel in the system's lines, keys, typed characters) and answers *nothing*, *redraw*, *relayout* or *close*; it can ask for wake-ups (animations, deadlines) and is polled whenever the bar thread is woken (results of its workers); props it changes (a dragged size) are saved through the config. The bar frames it like a hint (bubble, pointer, shadow) with 4 px padding, placed by §7.3 or its left-edge variant (§9.19). The window is layered, topmost, owned by the bar and focusable: it takes the foreground when it appears (the click on the bar was this process's input), hides itself when it loses the activation or on Alt+F4, and fades in like the hint. The bar never takes the activation, so a press on the bar closes the popup itself; a press that closed its own sensor's popup never opens it again (however long it is held), and a sensor whose popup closed otherwise ignores clicks for 500 ms. Esc, menus, full-screen apps, hiding the bar and removing the sensor close it too; no hints show while it is open, and the pointer moving meanwhile does not end the dismissal of its sensor's hint (§7.1). While a grip is dragged the popup keeps the side it is lined up on, and is no wider than that side has room for. Popups' shell icons are transient: only the newest 48 requests wait to be read, and beyond 256 the least recently used go; the sensors' icons are always kept. The active state is the text colour at 16 % in a rounded rectangle reaching 3 px beyond the sensor.

### 7.3 Placement (hint and popup)

Inputs: the sensor rect in screen DIPs (`anchor`), the measured content size `w×h`, the bar edge, the bar rect and the monitor rect `M`.

```
margin = 7
x = anchor.centerX − w/2
x = clamp(x, M.left + margin, M.right − margin − w)
if edge == bottom: y = barRect.top − h        (popup above the bar, pointer at its bottom)
else:              y = barRect.bottom         (popup below the bar, pointer at its top)
pointerX = clamp(anchor.centerX − x − pointerW/2, radius + 1, w − radius − 1 − pointerW)
```

Flow (all on the bar thread): measure the content, compute the placement (`bar/placement.rs`, pure), render into the window's bitmap, then size, position (physical px) and show the window, without activation for hints and with focus for popups, fading in over 120 ms (layered-window alpha). A live hint update re-measures and re-renders in place; the window moves only if its size changed.

### 7.4 Notifications

* Windows toast notifications (WinRT `ToastNotificationManager` through the platform layer); app name "Infodock", toast title = sensor title. The installer's Start Menu shortcut carries the AppUserModelID `app.infodock.desktop`.
* v1 producers: Battery charge alert (§9.14), Eject results (§9.22), layout import summary.
* Sound and placement follow the Windows notification settings.
* Implementation (`platform/notify.rs`, `platform/windows/toast.rs`): `Platform::notify(title, body)` shows a `ToastGeneric` toast with two text lines (XML-escaped) through `CreateToastNotifierWithId`, on a short-lived thread, since the first toast loads WinRT. At start-up the process takes the AppUserModelID (`SetCurrentProcessExplicitAppUserModelID`, which also groups the Settings window with the Start Menu shortcut) and registers it with the display name "Infodock" under `HKCU\Software\Classes\AppUserModelId\app.infodock.desktop`, so the portable build, which has no shortcut, shows toasts too. A config restored after a read error (§11.2) is announced this way.

---

## 8. Sensor framework

### 8.1 Sensor type contract (Rust)

`sensors/mod.rs` holds the contract; this is a summary.

```rust
pub enum Zone { Left, Center, Right }                     // config::model
pub enum Category { General, Info, System, Network, Launchers }

/// One per sensor type; registered in `sensors/registry.rs`. Static and thread-safe.
pub trait SensorType: Send + Sync {
    fn id(&self) -> &'static str;                         // stable id, e.g. "cpu-graph"
    fn category(&self) -> Category;
    fn name(&self) -> MsgKey;                             // localised display name
    fn description(&self) -> MsgKey;
    fn legacy_types(&self) -> &'static [&'static str];    // legacy XML type ids handled (§11.4)
    fn legacy_keys(&self) -> &'static [LegacyKey];        // its XML keys and the props they set (§11.4)
    fn convert_legacy(&self, raw: &LegacyProps, cx: &mut LegacyCtx) -> Props;
                                                          // cx: the legacy type, machine defaults,
                                                          // warnings, state (e.g. the uptime record);
                                                          // by default reads legacy_keys by their
                                                          // fields' encodings
    fn requires(&self) -> &'static [Capability];
    fn schema(&self) -> &'static [PropField];             // ordered fields: drives the Settings form,
                                                          // validation and static defaults (§8.2)
    fn machine_defaults(&self, ctx: &DefaultsCtx) -> Props;  // ctx: the platform (volumes, locale, …; §11.2)
    fn create(&self, props: &Props, state: &State) -> serde_json::Result<Box<dyn Sensor>>;
                                                          // props resolved; fails only if the typed
                                                          // props do not match the schema
    fn summary(&self, props: &Props) -> Option<String>;   // tells instances apart in lists (Layout manager)
    fn picker_note(&self, ctx: &DefaultsCtx) -> Option<MsgKey>;  // a note on this machine (§6.6)
}

/// A placed instance. Lives on the bar thread; re-created when its props change, unless it
/// takes them in place (`reconfigure`).
pub trait Sensor {
    // Layout and drawing
    fn measure(&mut self, cx: &LayoutCtx) -> f32;         // width in logical px; height = the bar's;
                                                          // builds and keeps text layouts and bitmaps
    fn min_width(&self) -> Option<f32>;                   // how narrow its text can be cut (§5.4)
    fn fit(&mut self, width: f32, cx: &LayoutCtx);        // the width the layout gives it (§5.4)
    fn draw(&self, canvas: &mut dyn Canvas, cx: &DrawCtx);
    fn inactive(&self) -> bool;                           // its icon/graphic is drawn inactive (§8.5)
    // Data and time
    fn topics(&self) -> Vec<TopicRequest>;                // system data needed; asked once per instance
    fn on_sample(&mut self, topic: &Topic, sample: &Sample, cx: &SensorCtx) -> Dirty;
    fn wake_at(&self, now: Instant) -> Option<Instant>;   // timer-driven sensors (clock, uptime, animations)
    fn on_wake(&mut self, now: Instant, cx: &SensorCtx) -> Dirty;  // Dirty = bool: what it shows changed
    fn on_clock_change(&mut self, now: Instant, cx: &SensorCtx) -> Dirty;  // time, time zone or resume
    fn take_state(&mut self) -> Option<State>;            // state to store (e.g. the uptime record), taken once
    // Interaction
    fn on_hover(&mut self, hovered: bool, now: Instant, cx: &SensorCtx) -> Dirty;
    fn hint(&self, cx: &SensorCtx) -> Option<HintContent>;   // Text or Rich (draws itself, e.g. a calendar)
    fn click(&mut self, cx: &SensorCtx) -> ClickResult;   // None, Handled, Confirm(Question) or Popup(Box<dyn Popup>)
    fn confirmed(&mut self, cx: &SensorCtx) -> Dirty;     // the user agreed to its Confirm question
    fn has_popup(&self) -> bool;                          // true exactly when click can return Popup
    fn menu(&self, cx: &SensorCtx) -> Vec<MenuItem>;      // sensor-specific items (§6.4)
    fn on_menu(&mut self, item: MenuId, cx: &SensorCtx) -> Dirty;
    fn drop_action(&self, items: &[Dropped], drag: DragInfo, cx: &SensorCtx) -> Option<DropAction>;
                                                          // effect and floating hint, or None: the bar
                                                          // inserts new sensors instead (§6.3)
    fn drop_items(&mut self, items: &[Dropped], drag: DragInfo, cx: &SensorCtx);
    fn take_props(&mut self) -> Option<Props>;            // props it changed itself, to save (§9.14), taken once
    fn reconfigure(&mut self, props: &Props) -> bool;     // takes changed props in place?
    // Accessibility (§14.4)
    fn accessible_value(&self, cx: &SensorCtx) -> String;
    fn focusable(&self) -> bool;                          // false only for the Spacer
}

/// What a sensor may use when it handles a sample, wake-up, click, menu item or drop, or is
/// asked for its hint, menu or accessible value; built by the bar thread for each call.
pub struct SensorCtx<'a> {
    pub settings: &'a Settings, pub locked: bool, pub alt: bool, pub animate: bool,
    pub locale: &'a Locale, pub numbers: Numbers,        // kept by the bar (§8.6)
    pub platform: &'a dyn Platform,
    pub changes: &'a ChangeSink,                          // what its actions changed (§4.4)
}

/// Measuring may lay out text and decode images; drawing happens in the sensor's place on the bar.
pub struct LayoutCtx<'a> {
    pub theme: &'a Theme, pub text: &'a dyn TextEngine, pub scale: f32,
    pub locale: &'a Locale, pub numbers: Numbers,        // the locale and how it writes numbers (§8.6)
    pub icons: &'a dyn IconSource,                        // shell icons and image files, read on a worker
}
pub struct DrawCtx<'a> { pub theme: &'a Theme, pub rect: RectF }   // logical px, bar coordinates
```

All trait methods except the identity ones, `create`, `measure` and `draw` have default implementations (no topics, no hint, no click action, …), so simple types stay small. Each type deserialises a typed props struct from its schema's defaults overlaid with the stored props; the schema is the single source of static defaults. Settings reads the schemas through the sensor types operation (§4.5).

Sensors act on the bar only through what they return, which the bar applies: `Dirty` (re-measure and redraw), `ClickResult` (a confirmation the bar asks, a popup it shows), and state and props it takes from them after samples, wake-ups and clicks (`take_state`, `take_props`, saved through the config store). Everything else goes through the platform (opening, launching, notifying, ejecting) or `changes` (what an action changed, so its topics are read again).

### 8.2 Property field types

| Field type | UI control | Stored as | Notes |
|---|---|---|---|
| `text` | text input | string | Option `emptyIsDefault` |
| `bool` | checkbox / switch | boolean | |
| `select` | dropdown | string option id (e.g. `"none"`, `"number"`); numeric options (intervals, percentages) as numbers | |
| `slider` | slider + number box | number | `min`, `max`, `step`, `unit` |
| `interval` | slider (seconds) | number (s) | per-sensor `min` / `max` / `default` |
| `color` | colour picker | `#RRGGBB` | |
| `colorMode` | segmented control (Standard / Custom / Dynamic) + colour | `{ mode, color }` | Some sensors allow only Standard/Custom |
| `opacity` | slider in % | integer 0–100 | option `min` |
| `file` | path + Browse… (extension filter) | string | Existence is checked when used, not on save |
| `folder` | path + Browse… | string | |
| `font` | dropdown of system font families | string | In sensor properties, a first **Default** entry (stored as `""`) follows the global sensor font (§10.4) |
| `fontStyle` | Normal / Semibold / Bold / Italic / Semibold Italic / Bold Italic | string: `normal`, `semibold`, `bold`, `italic`, `semiboldItalic`, `boldItalic` | Semibold = weight 600; fonts without a semibold face render bold |
| `drive` | dropdown from the `volumes` topic (live) | `X:\` | option `hideOptical` |
| `netAdapter` | dropdown from `list_net_adapters` (live) + "Automatic" | `{ id, name }` | |
| `battery` | dropdown from the `batteries` topic (live) | index | |
| `hidden` | — | any | internal values (e.g. popup size) |

Field definition: `{ key, type, label, description?, default, machineDefault?, ...typeOptions, visibleIf?, required? }`. `visibleIf` is declarative so Settings can evaluate it: `{ key, in: [values] }` (shown while another field has one of these values).

`required: { message, unless? }` marks a field that means nothing while it is empty (absent, `null` or blank text), unless the field `unless` names holds a value: the Properties form shows `message` on its row and disables Save while it is visible and empty (§6.7). It is a rule of the form only: a stored or imported sensor may hold it empty, and the sensor says so (*No folder set*, *No target set*). Required: the Text's `text` (an empty one has no width on the bar), the Shortcut's `target` unless `url`, the Folder Menu's `folder`.

Validation (`sensors/schema.rs`): each value is normalised for its field type: colours to upper-case `#RRGGBB`, numbers clamped to their range (`slider`, `interval`, `opacity`), intervals rounded to whole seconds and sliders with whole steps to whole numbers (a hand-edited `2.5` would not load into an integer), drives to `X:\`. A value of the wrong type or shape is invalid: on load it is ignored (the field takes its default) and logged; `update_sensor` rejects it with error code `invalid`. An empty `text` with `emptyIsDefault` means the default.

`default` is normally a static value. A **machine-derived** field (`machineDefault: true`) instead gets its default from `defaults(ctx)`, e.g. the first fixed drive or the locale's clock format. §11.2 defines how this affects what is stored.

### 8.3 Reusable building blocks

**Two-line block** (`common/two_line.rs`) — the core visual idiom: a small **label** line above a **value** line, both in the sensor font (modern: 10 px semibold; classic: Arial 9 px bold). In a 25 px bar the classic text baselines are y = 11 and y = 21. Modern: two rows centred vertically as a block; line height = font size + 1. Implementation: both themes place each baseline 1.5 px above its row's bottom (which gives the classic y = 11 and 21), with tabular digits; each line is left-aligned at an x or right-aligned to one.

**Meter** (`common/meter.rs`) — CPU, Memory, Swap and Disk meters:
```
[▮] LABEL
    value
```
* Bar: 4 px wide × (bar height − 6) px tall (19 px at 25 px), at x = 0, vertically centred.
* Bar background: sensor text colour at 31 % opacity.
* Fill: height = value × bar height, anchored at the bottom. Colour by `colorMode`: Standard → sensor text colour at **59 %**; Custom → the colour at 100 %; Dynamic → gradient(value) at 100 % (§8.4).
* Label at x = 8. Value right-aligned to x = 32 (classic geometry), i.e. a minimum column that fits "100%".

**Graph** (`common/graph.rs`) — CPU, Memory and Swap graphs:
* Plot area: `width` logical px (50–150, step 5, default 50) × (bar height − 4), at y = 2.
* **One sample per logical px column**, newest at the **right edge**, scrolling left. Each column is a bar from the bottom with height = value × plot height. History length = width.
* Layers: background (`bkColor` standard = black; `bkOpacity` default **0 %**) → grid (pattern of 5 px cells; `gridColor` standard = `#A0FF9B`; `gridOpacity` default **0 %**) → columns (`graphColor` default **Custom `#00FF00`**; `graphOpacity` default **55 %**, min 20 %). With Dynamic colour, each column takes the colour of its own value.
* Two-line block to the right of the plot (gap 5 px): label (e.g. "CPU") and value (e.g. "37%") right-aligned in a reserved column (fits "100%" or the sensor-specific reserve).
* Drawn at the window's scale (one logical column = `scale` physical px, snapped to whole pixels), fully redrawn per sample (cheap at ≤ 150 columns).

**Dual graph** (`common/dual_graph.rs`) — CPU dual, Network:
* The plot is split into two halves of (plotHeight − 1)/2 with a 1 px gap; implementation: both halves are as tall in whole physical px, and the gap takes what is left (1 or 2 physical px; at 28 px, halves of 11 px and a 2 px gap; classic 25 px, 10 and 1). The **top** series grows upward from the middle line. The **bottom** series grows **downward** from the middle (`flipBottom = true`, default) or upward from the bottom edge (`false`).
* Colour and opacity per series (`topColor`/`topOpacity`, `bottomColor`/`bottomOpacity`).
* Values are normalised to 0..1 per series by the owning sensor.

**Icon** (`common/icon.rs`) — every icon a sensor shows on the bar: the user's own image (`image`: ICO, PNG, GIF, JPEG or SVG) if set and readable, otherwise the sensor's own (a shell icon, or a built-in SVG where the shell has none); tinted by `color` (§8.4), drawn at `opacity` (20–100 %) and as inactive (§8.5), at the sensor's left, centred vertically on whole physical px. Heights: bar height − 4 (Shortcut, Disk, Recycle Bin), 21 px (Folder Menu, Start Menu, Power, Eject USB), bar height − 1 (Image), bar height − 8 but at least 12 px (Wi-Fi), 23 px or bar height − 2 if less (Battery). Implementation: the fields and their legacy keys (`image`, `colorize`, `opacity`), the heights (`IconSize`) and the measuring and drawing (`BarIcon`: decoded at the physical height; while an image or shell icon is read its square stays free) are defined there once; Image draws its picture from the bar's top and Battery its layers itself.

**Icon + two-line** (`common/icon_label.rs`) — Battery, Wi-Fi, Recycle Bin: an icon (≤ bar height − 2) followed by a two-line block after a 2–3 px gap. Implementation: a 3 px gap; the icon's place is kept when it cannot be drawn.

**History** (`common/history.rs`): fixed-capacity ring buffer of `f32`. A sensor whose plot width changes is created anew, with an empty history.

Implementation notes: the meter's value column fits `100%` as formatted (§8.6) or the value if wider; its bar and fill edges sit on whole physical px. Graph grid lines are one physical px, the vertical ones every 5 px from the plot's left, the horizontal ones every 5 px from its bottom; transparent layers are not drawn. Column edges are snapped, so at 150 % columns are alternately 2 and 1 physical px wide. A graph's text column fits the sensor's reserve (e.g. `100%`) or its text, and both lines end at the sensor's right edge.

### 8.4 Colour modes and the dynamic gradient

* `standard`: use the sensor text colour (meters: 59 % opacity); follows global settings.
* `custom`: the chosen colour.
* `dynamic` (green → yellow → red by value `v ∈ [0,1]`):
  ```
  v < 0.5:  R = round(v × 510), G = 255, B = 0
  v ≥ 0.5:  R = 255, G = round((1 − v) × 510), B = 0
  ```
* Icon sensors offer only Standard/Custom: Custom **tints** the icon (§12.3 tint algorithm); Standard leaves it unchanged. Implementation (`sensors/common/icon.rs`): unchanged means shell icons and the user's own images; Infodock's built-in icons (logo, globe, generic app, drive, Power modes) are monochrome white, so with Standard they take the sensor text colour and read on a light bar too.

### 8.5 Inactive rendering

When `inactive()` returns true, the sensor's icon or graphic is rendered as:
* `inactiveMode = "opacity"` (default): opacity `inactiveOpacity` (default 78 %);
* `inactiveMode = "colorize"`: tinted with `inactiveColor` (default `#555555`).

Labels stay readable; only the icon/graphic dims. v1 uses it for: Disk and Disk Meter (drive absent), Wi-Fi (disconnected), Battery (no battery), Shortcut (target missing), Swap (no page file). Each sensor applies it itself while it measures and draws (`theme.inactive`, through `color::inactive` or the launchers' `Icon`); `inactive()` says when, and the bar does not ask. A sensor whose data cannot be read (an error sample, §14.2) is drawn inactive as a whole, labels too: that the bar applies, with the inactive opacity (78 % in colourise mode).

### 8.6 Formatting helpers (`sensors/format.rs`)

* **Bytes** (`formatBytes(n, digits = auto)`), base **1024**, units `TB, GB, MB, KB, bytes` (singular `1 byte`). Auto digits: `0` → "0"; `< 10` → 2 decimals; `< 100` → 1 decimal; otherwise 0. Plain bytes always use 0 decimals. Examples: `0 bytes`, `512 bytes`, `1.50 KB`, `12.3 MB`, `431 GB`.
* **Rates** (`formatRate`): same scaling, units `TB/s … KB/s, B/s`.
* **Durations**: compact `3d 04h 12m` (uptime; the leading unit unpadded and units above it left out: `4h 05m`, `12m`); long `3 days, 4 hours, 12 minutes` with plural rules, leaving out parts that are zero (`1 day`; under a minute `0 minutes`); battery `h:mm`.
* Implementation (`platform/locale.rs`, `sensors/dates.rs`): `Platform::locale(dateLocale)` returns the locale as plain data (weekday and month names, the month names as declined next to a day number, the long, short and month-day date patterns in the Windows picture format, the hour cycle, am/pm, the first day of the week, the decimal separator), read with `GetLocaleInfoEx` (`ui` reads `en-US`, the only UI language in v1) and English (United States) elsewhere. Rust formats dates with those patterns and computes weekdays, days of the year and ISO weeks itself, so all of it is tested against fixed locales. Month names decline as `GetDateFormatEx` declines them: in languages with genitive names (`LOCALE_SMONTHNAME*` and `LOCALE_SABBREVMONTHNAME*` with `LOCALE_RETURN_GENITIVE_NAMES`; Croatian `21. listopada`, Polish `21 października`, Russian, Czech), a pattern's `MMMM` or `MMM` takes the genitive when the nearest day before it is a day number (`d`, `dd`), or else the nearest day after it is; a weekday name (`ddd`, `dddd`) is not a day number. Elsewhere (`MMMM yyyy`, a month alone, the Date sensor's month line) the name stays as it is named. A Windows test compares the patterns of several such locales with `GetDateFormatEx`.
* Implementation: the bar keeps the `Locale` of `settings.dateLocale` and its `Numbers`, and hands both to sensors on every context (`LayoutCtx`, `SensorCtx`; popups get the numbers on `PopupCtx`), so text on the bar, hints and accessible values use the same locale and no sensor reads it from the platform (Windows reads it with about 70 `GetLocaleInfoEx` calls). It is read again when the setting or the OS regional format (`WM_SETTINGCHANGE` "intl") changes, and every sensor then measures again; a clock change (`on_clock_change`) only means the wall clock jumped.
* Implementation (`sensors/format.rs`): a value that would round to 1024 moves to the next unit (`1.00 MB`, not `1024 KB`), and the automatic digits are chosen after rounding (`10.0 KB`, not `10.00 KB`); units and words are catalog entries (`format.*`, plural forms through `i18n::t_count`).
* **Percent**: integer, `37%` (locale-specific spacing allowed). Decimal separator from the locale.
* **Dates**: weekday and month names, date patterns and the hour cycle from the platform `Locale` (Windows NLS: `GetLocaleInfoEx`, `GetDateFormatEx`, `GetTimeFormatEx`) for `settings.dateLocale`; ISO 8601 week numbers computed in Rust.

---

## 9. Sensor catalog (v1)

Conventions:
* Properties table: key · XML key (§11.4) · type · default · range/options.
* Intervals are in seconds.
* "Hint" is the hover popup text; `{braces}` mark data. "Click to …" lines are added per §6.1 where a hint shows one: the examples are complete, so Date, Time and the Disk Meter have none. Hints are built without Alt held, so with Lock and *Alt only* no hint has the line (`SensorCtx::clicks`).
* Sizes refer to the modern 28 px bar; classic geometry is given where it matters.

### 9.1 `text` — Text  (General; XML `DockText`)

| Key | XML key | Type | Default | Notes |
|---|---|---|---|---|
| text | `text` | text | "Text" | required (§8.2) |
| font | `font` | font | `""` (Default = global sensor font) | |
| fontStyle | `fontstyle` | fontStyle | normal | XML `""` → normal, `bold` → bold, `italic` → italic, `bold, italic` → boldItalic |
| fontSize | `fontsize` | slider 9–25 | 13 | |
| shadow | `shadow` | bool | false | 1 px offset (1,1). Shadow colour = black if the text colour is light (grey = 0.2R + 0.5G + 0.3B ≥ 128), else white |

View: one line, vertically centred, in the sensor text colour. No click action. No hint, except while the zones do not fit the bar and the text is cut with an ellipsis (§5.4): then the whole text.

### 9.2 `image` — Image  (General; `DockImage`)

| Key | XML key | Type | Default |
|---|---|---|---|
| image | `image` | file (.png .jpg .gif .bmp .ico .svg) | "" → Infodock logo |
| color | `colorize` | colorMode (standard/custom) | standard |
| opacity | `opacity` | opacity, min 20 % | 100 % |
| popupText | `text` | text | "" — `%v` is replaced by the app version |

View: the image scaled to height = bar height − 1, keeping aspect ratio, decoded at that physical height off the bar thread (§4.1); until it arrives the sensor takes no room (it keeps the image it had when only the size changed). A file that cannot be read shows the logo (and is logged). Hint = popupText (none if empty). The first-run instance uses popupText "Infodock %v".

### 9.3 `spacer` — Spacer  (General; `DockSpacer`)

| Key | XML key | Type | Default |
|---|---|---|---|
| showDivider | `show` | bool | false |
| width | `width` | slider 3–145 px | 9 |

View: a transparent hit area `width` × bar height. A 1 px divider is centred (x = round((width − 1)/2)), (bar height − 6) px tall, in the sensor text colour at 40 % opacity when `showDivider` is on. When it is off, the divider fades to 40 % on hover (200 ms in, 300 ms out) so users can find the spacer.

### 9.4 `date` — Date  (Info; `DockDate`)

| Key | XML key | Type | Default | Options (XML value) |
|---|---|---|---|---|
| weekday | `dayname` | select | long | none (0) / short (1) / long (2) |
| month | `month-format` | select | long | none (0) / short (1) / long (2) / number (3) |
| year | `year-format` | select | top | none (0) / top (4) / bottom (5) |
| shadow | `shadow` | bool | false | as Text |
| calendar | `show-calendar` | select | monday | none (0) / monday (1) / sunday (2) — week start of the calendar |

**View**
* **Year shown**: on the left, the **day line** in large text (15 px bold), e.g. `Saturday, 11` / `Sat, 11` / `11`. To its right (3 px gap), two small stacked lines (11 px): **month** in lower case (`october`, `oct` or `10`) and **year** (`2026`). The year is on the top line for `top`, on the bottom line for `bottom`. With month = none, the month line is hidden.
* **Year = none**: a single large line combining weekday and month, e.g. `Saturday, October 11`, `Sat, Oct 11`, `Saturday, 10/11`, `11`. Build it from the locale's date patterns (§8.6) so that ordering follows the locale.
* Re-render when the local date changes: a timer to the next local midnight, plus `WM_TIMECHANGE`, resume, and a 60 s safety tick.

**Hint (rich)**: when calendar ≠ none, a **month calendar**: a header row of short weekday names starting at the chosen week start, then 4–6 week rows with right-aligned day numbers in columns ≥ 29 px wide. Weekend columns (Sat, Sun) use `popups.strongTextColor`. **Today** is a filled rounded rect in the text colour (strong colour on weekends) with the number in the popup background colour. Below the calendar: `Week {isoWeek}, Day {dayOfYear}`. With calendar = none, only that line.

**Click**: open `settings.apps.calendar` if set (file path or URI, e.g. `outlookcal:`); otherwise `ms-settings:dateandtime`.

Implementation (`sensors/date.rs`): the month and year lines are 11 px on the two-line block's rows (§8.3) for that size; the day line is the weekday name, a comma and the day. Without a year, the line is the weekday (`dddd, ` or `ddd, `) followed by the locale's month-day pattern (`MMMM` → `MMM` for short months) or, for numeric months, its short-date pattern without the year (`M/d/yyyy` → `M/d`); next to the day the month name declines where the language does (§8.6: `srijeda, 21. listopada`), while the month line beside the day line keeps the name as it is named (`listopad`). The sensor reads the clock at its first wake-up (due at once) and then each minute, which also notices midnight; a clock change reads it again. The calendar is a rich hint (`RichHint`, §7.1): columns at least 29 px, numbers right-aligned 4 px from the column's edge, today on a rounded highlight, the week line half a row below (`Week {week}, Day {day}`).

### 9.5 `time` — Time  (Info; `DockTime`)

| Key | XML key | Type | Default | Options (XML value) |
|---|---|---|---|---|
| use24h | `24hours` | bool | machine-derived: from the locale's hour cycle (`h11`/`h12`, e.g. en-US → false; otherwise true) | |
| showSeconds | `seconds` | bool | false | |
| shadow | `shadow` | bool | false | |
| chime | `chime` | select | never | never (0) / every hour (60) / every half hour (30) / every 15 minutes (15) |

**View**
* Without seconds: the **hour** in large text (15 px bold, no leading zero: `9`, `21`). To its right (2 px gap), the **minutes** in small text (11 px), raised like a superscript. In 12 h mode, a small `am`/`pm` (9 px) sits under the minutes. The minutes column has a fixed width = max(width("00"), width("am"), width("pm")) so the bar does not shift. Updates when the minute changes (timer aligned to the minute boundary, plus `WM_TIMECHANGE`).
* With seconds: `H:MM` in large text, and the **seconds** small and raised (am/pm under them in 12 h mode). Updates at second boundaries (aligned timer, no polling).
* **Chime**: when the minute changes and `minute % chime == 0`, play the bundled chime sound (`assets/sounds/chime.wav`, embedded in the binary) with `PlaySound(SND_MEMORY | SND_ASYNC)`. No chime within 2 s after app start or resume.
* Implementation (`sensors/time.rs`): `chime` is stored as minutes (0 = never, 60, 30, 15). The small column uses the two-line block's rows (§8.3): minutes or seconds on the upper one, am/pm (the locale's, in lower case) on the lower one; the hour is vertically centred. The sensor reads `Platform::local_time` at its first wake-up (due at once, so it is empty until then), then just after each minute or second; a clock change (§4.4) reads it again and is quiet for 2 s like a start. The bundled sound is a generated placeholder until the artwork (Appendix C).

**Hint**: the long date, e.g. `Saturday, October 11, 2026` (the locale's long date format).
**Click**: `ms-settings:dateandtime`.

### 9.6 `uptime` — Uptime  (Info; `DockUpTime`)

| Key | XML key | Type | Default |
|---|---|---|---|
| text | `text` | text | "Uptime:" — prefix; an empty value means no prefix (no `emptyIsDefault`) |
| font / fontStyle / fontSize / shadow | as Text | | style bold, size 12 |

State (persisted in the config's `state`, §11.2): `longestSecs` (XML key `max-uptime`, format `d:h:m:s`; empty or `0:0:0:0` = none; a value that cannot be read, or one over a century, keeps none with a warning).

Topic `uptime`. Rust takes the boot time from `GetTickCount64()`; the UI counts locally between samples (resample every 60 s and on resume).

View: `{prefix} 3d 04h 12m`, updated at minute boundaries.
Hint: `3 days, 4 hours, 12 minutes`; when a longer record exists, a second line `Longest uptime: 12 days, 1 hour, 3 minutes`. The record updates whenever the current uptime exceeds it.

Implementation (`sensors/uptime.rs`): the sensor counts from its last sample (which carries when it was read, since a catch-up can arrive up to a minute later) and wakes 20 ms after each whole minute of uptime, counted from the minute it shows, so a late timer still finds the wake-up due. The record updates in memory at once but is stored only when it has grown by 10 minutes, so the config is not rewritten every minute: a sensor hands state to store through `Sensor::take_state`, which the bar passes on after samples and wake-ups (`BarHost::set_sensor_state` → the config's `state`, §11.2).

### 9.7 `cpu-meter` — CPU Meter  (System; `DockCPUMeter`)

| Key | XML key | Type | Default | Range |
|---|---|---|---|---|
| interval | `interval` | interval | 1 | 1–10 |
| color | `colorize` | colorMode (all) | standard | |

Topic `cpu`. View: Meter, label "CPU", value `{total}%`.

**Hint (shared by all CPU sensors)**:
```
{n} CPUs use              ("1 CPU uses" when n = 1; n = logical processors)
   activity: 37.0%
   by user tasks: 25.1%
   by system: 11.9%
   idle: 63.0%
Click to open Task Manager
```
One decimal. User and system are clamped so that their sum does not exceed 100.
**Click**: launch `taskmgr.exe`.

### 9.8 `cpu-graph` — CPU Graph  (System; `DockCPUGraph`)

| Key | XML key | Type | Default | Range |
|---|---|---|---|---|
| interval | `interval` | interval | 1 | 1–10 |
| width | `width` | slider, step 5 | 50 | 50–150 (other values round down to a multiple of 5) |
| bkColor | `bk-color` | colorMode (standard/custom) | standard (= black) | |
| bkOpacity | `bk-opacity` | opacity | 0 % | 0–100 |
| gridColor | `grid-color` | colorMode (standard/custom) | standard (= `#A0FF9B`) | |
| gridOpacity | `grid-opacity` | opacity | 0 % | 0–100 |
| graphColor | `graph-color` | colorMode (all) | custom `#00FF00` | |
| graphOpacity | `graph-opacity` | opacity | 55 % | 20–100 |

View: Graph with label "CPU" and value `{total}%`. Hint and click as §9.7.

Implementation (`sensors/cpu.rs`, `providers/cpu.rs`, `platform/windows/cpu.rs`): the platform's `CpuReader` (owned by the sampler thread) reads `GetSystemTimes` and a PDH query opened once with `\Processor Information(_Total)\% Processor Utility` and the same per processor (`(*)`, instances `group,number`); the provider turns two readings into the sample (user, kernel without idle, idle from the time deltas; the busy share from the utility counter, else from the times; all clamped to 0–100), discarding the first reading after a start or resume. Graphs share their properties and look (`common/graph.rs` `GraphStyle`): the width rounds down to a multiple of 5, Standard background is black, Standard grid `#A0FF9B`, Standard columns the sensor text colour. Before the first sample the value line is empty.

### 9.9 `cpu-graph-dual` — CPU Graph (dual)  (System; `DockCPUGraphDual`; requires `per_core_cpu`)

All properties of §9.8 except graphColor/graphOpacity, plus:

| Key | XML key | Type | Default |
|---|---|---|---|
| topSeries | — | select | `core0` — options: `total`, `user`, `system`, `core0` … `core<N−1>` (N = logical processors) |
| bottomSeries | — | select | `core1` |
| topColor / topOpacity | `graph-top-color` / `graph-top-opacity` | colorMode / opacity | custom `#20B7FF` / 86 % |
| bottomColor / bottomOpacity | `graph-bottom-color` / `graph-bottom-opacity` | | custom `#FFAD5B` / 86 % |
| flipBottom | `graph-bottom-flip` | bool | true |

View: Dual graph, label "CPUs", value `{total}%`. A `core<i>` series whose processor does not exist on this machine (e.g. `core1` on a single-processor system, or a layout imported from a larger machine) shows total instead, and the Properties dialog marks that selection as unavailable. On single-processor systems the picker adds the note "This computer has one processor; both halves show the same data." Hint and click as §9.7.

Implementation: `topSeries`/`bottomSeries` are `cpuSeries` fields (`total`, `user`, `system`, `core<i>`); Settings lists Total, User tasks, System and *Processor 1…N* for this computer's logical processors (`Platform::logical_processors`), and a stored processor it lacks as *Processor n (not on this computer: shows the total)*. A processor whose counter reports no valid data keeps its place in the sample (as NaN) and shows the total too. The picker adds the single-processor note to the description (`SensorType::picker_note`, from `Platform::logical_processors`).

### 9.10 `memory-meter` / `memory-graph` — Memory  (System; `DockMemMeter`, `DockMemGraph`)

Meter: interval (1–60, default 2), color. Graph: as §9.8 (interval 1–10, default 1), plus:

| Key | XML key | Type | Default | Options (XML value) |
|---|---|---|---|---|
| display | `display-mode` | select | bytes | bytes — used bytes, e.g. `5.21 GB` (0) / percent (1) |

Topic `memory`. Label "RAM". Meter value `{load}%`; graph value per `display` (the reserved column fits "9999 MB" in bytes mode).

**Hint**:
```
Total Physical: 31.7 GB
Available Physical: 18.2 GB
Memory Load: 43%
Click to open Task Manager
```
Click: Task Manager.

Implementation (`sensors/memory.rs`, `providers/memory.rs`, `platform/windows/memory.rs`): the meter and the graph are written for any "used of total" topic, so swap (§9.11) shares them. The graph always plots the load; `display` changes only the value line (bytes: the used size). Before the first sample the value line is empty and there is no hint; a sample that changes nothing does not redraw the meter.

### 9.11 `swap-meter` / `swap-graph` — Swap  (System; `DockSwapMeter`, `DockSwapGraph`)

Same properties as memory (graph `display`: `bytes` / `percent`). Topic `swap`. Label "Swap".

**Hint**:
```
Total Virtual: 8.00 GB          (sum of allocated page-file sizes)
Available Virtual: 7.41 GB
Swap Load: 7%
Click to open Task Manager
```
Without a page file: value `n/a`, inactive, hint "No page file configured".

Implementation: the meter and graph of §9.10 (`sensors/memory.rs`) on the `swap` topic. `Platform::page_files` reads every page file's allocated size and use with `NtQuerySystemInformation(SystemPageFileInformation)`, one cheap call per tick, so neither WMI nor a size cache is needed (plan D24); `providers/swap.rs` adds them up. Inactive, the meter's bar and the graph's background, grid and columns dim (§8.5).

### 9.12 `disk-meter` — Disk Meter  (System; `DockDiskMeter`)

| Key | XML key | Type | Default | Notes |
|---|---|---|---|---|
| title | `title` | text | "" → `HD: C:\` | |
| drive | `disk` | drive (optical hidden) | machine-derived: first fixed drive | |
| mode | `mode` | select | used % | used % (0) / free % (1) / used bytes (2) / free bytes (3) |
| interval | `interval` | interval | 30 | 10–300 |
| color | `colorize` | colorMode (all) | standard | |

Topic `disk/<drive>`. View: Meter. The bar always shows the **used fraction**; the value text follows the mode (`63%`, `37%`, `412 GB`, `241 GB`). In byte modes the value is left-aligned at x = 8 under the label.

**Hint**:
```
Disk Meter for C:\
Total Space: 953 GB
Free Space: 241 GB
Disk Space Used: 75%
```
Click: open the drive in Explorer. Drive absent: value `n/a`, inactive.

Implementation (`sensors/disk.rs`, `providers/disk.rs`, `platform/windows/disk.rs`): `Platform::disk_space` (`GetDiskFreeSpaceExW`; free = all free bytes, used = total − free) and `Platform::volumes` (`GetLogicalDriveStringsW`, `GetDriveTypeW`, `GetVolumeInformationW` for the label) run with critical-error dialogs off for the thread, so an empty card reader or optical drive never asks for a disk. A missing drive (or one without a medium) is a sample with `present: false`; its hint says the drive is not available and a click does nothing. Mode ids: `usedPercent`, `freePercent`, `usedBytes`, `freeBytes`. The `drive` control lists the volumes when the form opens, named by label or kind (`Local Disk (C:)`, `Data (D:)`), and keeps the drive stored when the form opened as `F:\ (not connected)` while it is not there, even after another is chosen (a stored optical drive that is there is listed by its kind); while a form with a drive field is open, Settings subscribes to the `volumes` topic, so the list follows drives that arrive or go and keeps the choice. The Layout manager shows the drive beside the name. Labels are read only from fixed, removable and RAM drives (network and optical drives are named by kind), so listing the volumes never waits for a server or spins up a disc; reading a volume's space runs on the disk worker (§4.4).

### 9.13 `network-graph` — Network Graph  (Network; `DockNetwork`)

| Key | XML key | Type | Default | Notes |
|---|---|---|---|---|
| adapter | `net-adapter` | netAdapter | Automatic | Automatic = the interface carrying the default route (best metric), re-evaluated on change. The XML value is an adapter description, matched against Description/Alias |
| bandwidth | `bandwidth` | text | "" | Max bytes/s as `"in,out"`, or one value for both; empty = automatic scaling |
| interval, width, bk/grid properties | | | as §9.8 | |
| top/bottom colour, opacity, flip | as §9.9 | | top `#20B7FF` (receive), bottom `#FFAD5B` (send), 86 %, flip on | |

Topic `net/<adapter>` → `{ rxRate, txRate, rxTotal, txTotal, up, linkRx, linkTx, name, alias }`.

**Scaling** per series: `scale = max(configured max (if any), max of the visible history, 1 KB/s)`, recomputed every tick. The graph redraws from raw history values, so rescaling is lossless.

**View**: Dual graph. The text column shows **two right-aligned lines**: receive rate on top (e.g. `1.24 MB/s`) and send rate below (`86.0 KB/s`). The column is wide enough for `1023 MB/s`.

**Hint**:
```
Adapter: Wi-Fi (Intel(R) Wi-Fi 6E AX211)
   Status: active
   Receive: 1.24 MB/s (max. 11.8 MB/s)
   Send: 86.0 KB/s (max. 2.31 MB/s)
Click to open Network Connections
```
"max" is the scale currently in use. Status is `inactive` when the interface is down or missing.
**Click**: `control.exe ncpa.cpl`.

Implementation (`sensors/network.rs`, `providers/net.rs`, `platform/windows/net.rs`): the topic is `Topic::Net(Option<id>)`, `None` for Automatic. `Platform::net_interface` reads one interface per tick with `GetIfEntry2` (by the GUID, through its LUID; Automatic re-evaluates `GetBestInterfaceEx` for a public IPv4, else IPv6, address each tick, a route lookup that sends nothing); `Platform::net_interfaces` (`GetIfTable2`) lists the connections for Settings: no loopback or filter-driver views, hardware adapters and any other connected interface (VPNs), named by alias. The stored adapter is `{ id: <interface GUID>, name: <alias> }`. The provider keeps one baseline per topic: rates are byte deltas over the monotonic clock; the first reading, a counter that went back, Automatic moving to another interface and the first reading after resume give 0 and a new baseline. A missing interface is a sample with `present: false`. `bandwidth` takes bytes per second (`"in,out"` or one number); anything else scales automatically. The click opens `ncpa.cpl` (the same Control Panel item). The adapter control lists *Automatic* and the connections when the form opens, matched by id (a connection renamed since is the same one), keeping the stored one that is not there as `… (not connected)`; the Layout manager shows the connection's name. The legacy `net-adapter` is matched against `Platform::net_interfaces`: by the adapter's description, else by the connection's name, ignoring case; empty is Automatic, and an adapter this computer lacks is Automatic with a warning ("this computer has no network adapter …").

### 9.14 `battery` — Battery Life  (System; `DockBatteryLife`; requires `battery`)

| Key | XML key | Type | Default | Options (XML value) |
|---|---|---|---|---|
| battery | `battery` | battery | 0 | list "1. {name}", … |
| onBatteryShow | `when-in-use-display-percent` | select | percent | remaining time (`no`) / percent (`yes`) |
| smoothTimeToEmpty | `smooth-time-to-empty` | bool | false | moving average, α = 0.1 |
| chargingShow | `when-charging-display-percent` | select | percent | time until full (`no`) / percent (`yes`) |
| timeToFullSource | `calculate-time-to-full` | select | os | OS-reported (`no`) / calculated (`yes`) |
| assumeChargedAt | `assume-charged` | select | os | OS-reported (−1) / 95 / 96 / 97 / 98 / 99 / 100 % |
| alertAbove | `alert-level` | select | never | never (−1) / 80 / 85 / 90 / 91 / 92 / 93 / 94 / 95 % |

Topic `battery/<index>` → `{ present, name, percent, acOnline, state: charging|discharging|full|notCharging|unknown, timeToEmptyMin?, timeToFullMin? }`.

**State**
* `noBattery`: no battery present.
* `charging`: on AC and either the OS reports charging (`assumeChargedAt = os`) or `percent < assumeChargedAt`.
* `full`: on AC and not charging.
* `draining`: on battery; "low" when `percent < 15`.

**Icon** (19×23; SVG layers): back → level fill → level cap → front → overlay (charging bolt, charged plug, no-battery cross).
* Fill height maps percent piecewise for visibility: 0–15 % → 0–30 %, 15–40 % → 30–70 %, 40–100 % → 70–100 % of the fill area.
* Fill colour: `p < 50` → `#FF{g}00` with g = round(p × 5.1); otherwise `#{r}FF00` with r = round((100 − p) × 5.1).
* The level cap is hidden above 88 %; the fill is hidden at 0 % and with no battery.

**Two-line text** right of the icon. Label: `In Use` | `Low Battery` | `Charging` | `Full` | `No Battery`. Value:
* draining: `73%`, or remaining time `~ 2:41` (`*` while unknown);
* charging: `73%`, or time until full `0:52` (`*` while unknown);
* full: `100%` (empty when `assumeChargedAt ≠ os`).

**Calculated time to full**: when charging starts, remember (t₀, p₀). Once `p > p₀`: `est = (elapsed_s / (p − p₀)) × (target − p) / 60` minutes. On the first estimate, `timeToFull = max(previous, est)`; afterwards smooth with `timeToFull += (est − timeToFull) / 30`.

**Hint**:
```
{battery name}
    Capacity is 73%
    2:41 Remaining          (or: Calculating Time Remaining | 0:52 Until Full |
                                 Calculating Time Until Full | Battery Is Charged)
    1:12 Already In Use     (time since unplugged; on battery only)
```
**Click**: toggles percent ↔ time for the current state (`onBatteryShow` on battery, otherwise `chargingShow`) and saves it.
**Alert**: while charging, when `percent > alertAbove`, show the toast "Battery at {p}% — you can unplug the charger", once per charging session (from plugging the charger in to unplugging it, so a charge that pauses near full does not repeat it).

Implementation (`sensors/battery.rs`, `providers/battery.rs`, `platform/windows/battery.rs`): batteries come from `starship-battery` (the battery class driver), the power line from `GetSystemPowerStatus`. `battery/<index>` is the battery's place in the driver's list: one that cannot be read keeps its place with the driver's message (a failed reading, §14.2), so the batteries after it keep theirs and their sensors show the right battery; when none can be read (or the crate lists none, e.g. one reporting its capacity only relatively), the first place takes `GetSystemPowerStatus` alone, if Windows sees a battery. The topic is read on a worker thread (§4.4) every 10 s and at once on `PBT_APMPOWERSTATUSCHANGE` (`Change::Power`). Option ids: `time` / `percent` for `onBatteryShow` and `chargingShow`, `os` / `calculated` for `timeToFullSource`, `"os"` or a number for `assumeChargedAt`, `"never"` or a number for `alertAbove`. Percentages are compared whole. *Full* shows the battery's percentage when the OS says it is not charging (a charge limit shows `80%`) and nothing when it is assumed charged. The calculated time to full is recomputed at every sample once the charge has grown; the first estimate takes the longer of it and the battery's own. *Already In Use* appears only once the sensor saw the computer go on battery. Before the first sample both lines are empty. The icon is 19×23 (smaller only on bars lower than 25 px): back, the level (a rectangle in the level area), a 1 px lighter level cap, front, overlay; the layers are built-in white SVGs tinted with the text colour (placeholders, Appendix C), the overlays with a dark edge so they read on the level; *No Battery* is inactive by the §8.5 rules. The Settings list names batteries `1. {vendor model}` (`1. Battery` without a name) and keeps a stored battery that is missing, marked *(not connected)*. A click saves the toggled prop through the config (`Sensor::take_props`), and the sensor takes the saved props in place (`Sensor::reconfigure`), keeping its charge history; choosing another battery creates it anew. The alert's title is the sensor type's name.

### 9.15 `wifi` — Wi-Fi Signal  (Network; `DockWiFi`; requires `wifi`)

| Key | XML key | Type | Default | Options (XML value) |
|---|---|---|---|---|
| title | `title` | text | "" → network name (SSID) | |
| interval | `interval` | interval | 2 | 1–30 |
| display | `display-mode` | select | percent | network name (0) / signal percent (1) / signal dBm (2) |

Topic `wifi` → `{ available, radioOn, connected, ssid?, quality (0–100), rssiDbm?, band?, channel?, linkMbps?, phy?, locationDenied }`.

* Source: `WlanOpenHandle` → `WlanEnumInterfaces` → `WlanQueryInterface(wlan_intf_opcode_current_connection)` for SSID, `wlanSignalQuality`, PHY and rates. RSSI and channel come from `WlanGetNetworkBssList` for the connected BSSID. `wlan_intf_opcode_radio_state` gives the radio state. `WlanRegisterNotification` pushes connect/disconnect.
* **Windows 11 24H2 and later**: reading SSID/BSSID requires **Location** permission. On `ERROR_ACCESS_DENIED`, set `locationDenied`, keep showing quality, use "Wi-Fi" instead of the SSID, and add the hint line "Allow location access to show the network name" (click → `ms-settings:privacy-location`).

**Icon**: 6 levels (SVG): `0` when offline or the radio is off; otherwise quality ≥ 80 → 5, ≥ 60 → 4, ≥ 40 → 3, ≥ 20 → 2, else 1. Tinted with the sensor text colour at 75 % opacity.
**Two-line text**: label = title or SSID; value per `display` (`78%`, SSID, `-58 dBm`). Offline: value `offline`, inactive. If the title is empty and display is "network name", display falls back to percent.
**Hint**:
```
Wi-Fi Signal
   Network: HomeNet
   Info: 802.11ax, 5 GHz, channel 36, 1201 Mbps
   Signal level: -58 dBm
   Signal percentage: 78%
```
Offline: `Wi-Fi Signal` / `Offline`.
**Click**: `ms-settings:network-wifi`.

Implementation (`sensors/wifi.rs`, `platform/windows/wlan.rs`): the sampler keeps a WLAN client open (`Platform::wifi_reader`) while the topic has subscribers; `wlanapi.dll` is loaded dynamically (only the `windows` crate's types are used). It reads the first connected interface (or the first). The radio is on when a PHY is on in software and hardware; the link rate is the higher of receive and transmit; band and channel follow from the BSS's centre frequency (2.4, 5 and 6 GHz). Connection complete, disconnected, roaming end, radio and interface changes report `Change::Wifi` (§4.4), so the topic is read at once; the signal is polled at the interval. Without location access the RSSI opcode still gives the signal, and the quality is computed as Windows does (−100 dBm → 0, −50 dBm → 100). Display ids `name`, `percent`, `dbm`; without a known name or dBm the percentage shows. The icon is bar height − 8 tall (20 px at 28 px), 26:22, at 75 % of the text colour; offline or with the radio off it is level 0 and inactive, and the hint says *Offline* or *Wi-Fi is turned off*. The hint ends with *Click to open the Wi-Fi settings*, or without location access *Click to open the location settings*, which the click then opens. The six level icons are placeholders (Appendix C).

### 9.16 `recycle-bin` — Recycle Bin  (System; `DockTrashMeter`; requires `recycle_bin`)

| Key | XML key | Type | Default |
|---|---|---|---|
| title | `title` | text | "" → "Recycle Bin" |
| imageEmpty | `image-empty` | file (.ico .png .gif .jpg .svg) | "" → built-in empty icon |
| imageFull | `image-full` | file (.ico .png .gif .jpg .svg) | "" → built-in full icon |
| opacity | `opacity` | opacity, min 20 % | 100 % |
| color | `colorize` | colorMode (standard/custom) | standard |
| interval | — | interval | 10 (5–120) |

Topic `recycle-bin` → `{ items, sizeBytes }` via `SHQueryRecycleBinW` (empty root = all drives). Also pushed immediately after empty/delete/drop actions.
View: icon (full or empty) + two-line text: title / `empty` | `1 item` | `{n} items`.
**Hint**:
```
Recycle Bin
   Items: 12
   Space used: 431 MB
```
**Click**: open the Recycle Bin (`shell:RecycleBinFolder`).
**Menu**: `Open Recycle Bin`; `Empty Recycle Bin` (`SHEmptyRecycleBinW` with the OS default confirmation and sound).
**Drop**: files/folders → moved to the Recycle Bin (IFileOperation with `FOFX_RECYCLEONDELETE`; confirmation per OS settings). While dragging over it, show the floating drop hint "Delete {n} items" (§9.19).

Implementation (`sensors/recycle_bin.rs`, `platform/windows/shell.rs`, `platform/windows/fileops.rs`): a drop of files and folders shows the move pointer (nothing if the source does not allow moving) and the hint "Delete 'name'" or "Delete {n} items"; `Platform::recycle` deletes them with `IFileOperation` (`FOF_ALLOWUNDO | FOFX_RECYCLEONDELETE`) on a short-lived thread and the sensor then reports `Change::RecycleBin`. Links and drives dropped on it make new sensors (§6.3). The topic is read on a worker thread like volumes (§4.4), since counting a full Recycle Bin can take a while. The icon is bar height − 4, as the launchers', and the custom images are drawn at that height; *opacity* applies to the icon. Before the first reading the second line is empty and the hint is the title alone. *Empty Recycle Bin* is greyed out while the bin is known to be empty; `SHEmptyRecycleBinW` runs on a short-lived thread (the shell's confirmation may wait for the user), and when it returns, emptied or declined, the sensor reports `Change::RecycleBin`, so the topic is read at once. The built-in empty and full icons are placeholders (Appendix C).

### 9.17 `shortcut` — Shortcut  (Launchers; `DockShortcut`)

| Key | XML key | Type | Default | Notes |
|---|---|---|---|---|
| target | `link` | file (any file or folder; filter .exe .cmd .bat .lnk .url .ps1 + All) | "" | Application, document or folder; required unless `url` is set (§8.2) |
| args | `param` | text | "" | Parameters, passed as `lpParameters` to ShellExecuteEx; never interpreted by a shell |
| url | `url` | text | "" | Takes precedence over target. `http(s)://`, `ftp://` or a registered URI scheme (`ms-settings:`, `mailto:` …) |
| workDir | — | folder | "" → the target's folder | |
| runAs | — | select | normal | normal / administrator (`runas` verb) |
| title | `title` | text | "" → automatic | |
| image | `image` | file (.ico .png .gif .jpg .svg) | "" → file icon | |
| color | `colorize` | colorMode (standard/custom) | standard | |
| opacity | `opacity` | opacity, min 20 % | 100 % | |

* **Icon**: the custom image; otherwise the shell icon of the target (platform `Icons`, requested at physical size; `.lnk` shows its target's icon); for URLs, the built-in globe icon. Size: bar height − 4, centred.
* **Automatic title**: URL → the URL, middle-truncated to 150 characters (first 75 + `…` + last 75). File → file name without extension. Folder → folder name.
* **Hint**: the title if set; otherwise `Open '{title}'` (only `{title}` when clicks need Alt, §6.1).
* **Click**: URL → open with the default handler. Folder → open in Explorer. File → `ShellExecuteExW(verb = runAs ? "runas" : "open", file = target, params = args, dir = workDir)`.
* **Inactive** while the target does not exist (checked when properties change and every 60 s).
* Created by dropping a file or URL onto the bar (§6.3).

Implementation (`sensors/shortcut.rs`): `runAs` ids `normal` / `administrator`. The target is checked on the topic `path/<path>` every 60 s; a bare program name (`notepad.exe`, no folder or drive) is found by the shell itself (App Paths, `PATH`), so it is neither checked nor inactive. Until the shell icon or the custom image arrives its square stays free (both are read off the bar thread, §4.1); a target without a shell icon (missing, or a bare name) shows the built-in generic app icon, and when the target appears or changes, its icon is read again. Folder targets open without parameters. A Shortcut without a target or URL shows the generic icon, its hint says *No target set*, and a click does nothing. Paths are split at `\` and `/`, so names work the same on every OS; `C:\app.exe` starts in `C:\`. The Layout manager shows the title, the URL or the target's file name.

### 9.18 `disk` — Disk  (Launchers; `DockDisk`)

| Key | XML key | Type | Default |
|---|---|---|---|
| drive | `disk` | drive (optical/removable included) | machine-derived: first drive |
| title | `title` | text | "" → shell display name, e.g. `Local Disk (C:)` |
| image | `image` | file | "" → shell drive icon |
| color / opacity | `colorize` / `opacity` | | standard / 100 % |

Topic `volumes` (event-driven list). View: the drive icon only.
Hint: `Open '{display name}'` (or the title). Click: open the drive. Menu: **Eject** for removable and optical drives (§9.22; on success the toast "{name} can be safely removed", on failure a toast with the reason, e.g. "in use by {app}"). Inactive while the drive is absent; an optical drive without media stays active, with the hint `{name} — no media`.

Implementation (`sensors/disk_launcher.rs`): the *Eject* item appears while the drive is there and the drive list marks it `ejectable` (removable and optical drives, disks on USB, FireWire, SD or MMC). The display name is the one the drive lists use (§9.12): the label or kind with the letter, `Local Disk (C:)`, so naming never waits for a network server or a disc; before the first `volumes` sample it is the drive itself. The icon is the drive's shell icon at bar height − 4, centred (the built-in drive icon when the shell has none, e.g. for an absent drive), read again when the drive arrives, goes, or a medium is inserted or removed. Hints: `Open '{name}'` (only the name when clicks need Alt), `{name} — no media` (also for an empty card reader), `{drive} is not connected` while absent; a click on an absent drive does nothing.

### 9.19 `folder-menu` — Folder Menu  (Launchers; `TurboFolder`)

| Key | XML key | Type | Default |
|---|---|---|---|
| folder | `folder` | folder | "" (required) |
| title | `text` | text | "" → folder display name |
| image | `image` | file | "" → built-in folder icon |
| color / opacity | `colorize` / `opacity` | | standard / 100 % |
| showHidden | — | bool | false |
| popupWidth | `popup-width` | hidden | 240 (130–600) |
| popupHeight | `popup-height` | hidden | 320 (130 to 70 % of the monitor height) |

**View**: folder icon (21 px). Hint: the title, only while the popup is closed.

**Click** → interactive popup (§7.2), kind `folder-browser`:
* **Header**: the current folder's title (truncated at the end). Click → open the current folder in Explorer; Alt+click → reveal it. Below it, a 2 px line in `popups.strongTextColor`. A **Back** button on the right appears after navigating into a subfolder.
* **List**: 20 px rows with a 16 px shell icon and the name (truncated at the end). **Folders first**, then files; each group sorted case-insensitively with natural number order (`CompareStringEx` with `LINGUISTIC_IGNORECASE | SORT_DIGITSASNUMBERS`, as Explorer sorts). `.lnk` and `.url` extensions are hidden in names. `desktop.ini` is always skipped; hidden/system items are skipped unless `showHidden`. Hovered rows are highlighted (dark popup background: `#A9DFFF` at 40 %; light: `#00649E` at 55 %). An empty folder shows a centred "no items". Slow folders show a loading indicator; listing runs asynchronously in Rust.
* **Item click**: folder → navigate in place (the list slides left, 250 ms ease-out; Back slides right). File → open it (ShellExecute) and close the popup. **Alt+click** → reveal in Explorer.
* **Keyboard**: ↑/↓ select; Enter opens; → or Enter on a folder navigates into it; ← or Backspace goes back; Esc closes; typing jumps to the first name with that prefix.
* **Mouse wheel**: smooth scrolling (the system's wheel scroll lines, animated over 100 ms).
* **Resize**: a grip on the popup's free vertical edge (width) and on the edge away from the bar (height). The new size is saved to popupWidth/popupHeight.
* **Placement**: as §7.3, but the popup's left edge aligns with the sensor's left edge. If that would overflow the monitor, its right edge aligns with the sensor's right edge instead.
* **Menu**: `Open folder`.
* **Drop onto the sensor**:
  * Files/folders follow the **Explorer convention**: same volume → move, other volume → copy (volumes from the platform shell). Ctrl forces copy, Shift forces move, Alt creates shortcuts (`.lnk`).
  * URLs → a `.url` file named after the page title, or after the URL with the characters `:/\&+?*=.` replaced by `-`.
  * Executed with `IFileOperation` (native progress and conflict dialogs).
  * While dragging over the sensor: its background is highlighted (black at 78 %) and a **floating drop hint** appears above it, e.g. `Move 'report.pdf' to 'Downloads'`, `Copy 3 items to 'Downloads'`, `Create shortcut to 'https://…' in 'Downloads'` (source text truncated to 100 characters).
* Created by dropping a folder onto the bar.

Implementation (`sensors/folder_menu.rs`; the popup in `sensors/common/folder_browser.rs`, shared with Start Menu): the folder is listed on a worker thread (`Platform::list_dir`: each entry's attributes come with the listing, with critical-error dialogs off; junctions count as folders) and sorted with `Platform::compare_names` (`CompareStringEx` in the user's locale). The first layout waits up to 150 ms for the listing, so most folders appear complete; a slower one shows *Loading…* after 200 ms, and a folder that cannot be read says so. The popup is `popupWidth` × `popupHeight` (130–600 wide and 130 to 70 % of the monitor tall, both at most the room beside the bar), whatever the folder holds, so its grips always change what shows. The header is 24 px with the title in the popup text colour (it highlights like a row and opens the current folder, Alt reveals it); the Back button is its right 24 px. Only visible rows are laid out; their 16 px icons come from the bar's icon worker, with built-in folder and file icons where the shell has none. The pointer and the keys mark one row (the same highlight); Home, End, Page Up and Page Down move too. A click or Enter opens a file and closes the popup; on a folder it goes in (→ too), and coming back (Back, ← or Backspace) shows that folder as it was left, scrolled and with the subfolder marked. Alt reveals the row instead (`SHOpenFolderAndSelectItems`). Drops (`sensors/drop.rs`, `platform/windows/fileops.rs`): the operation follows Explorer: Alt (or Ctrl+Shift) makes shortcuts, Ctrl copies, Shift moves, and otherwise items on the folder's volume move and others are copied; volumes are compared by their roots (`Platform::volume_of`: the drive letter or `\\server\share`), so no drive is touched while dragging, and a volume mounted in a folder counts as the one holding the folder; an operation the source does not allow falls back to the other of move and copy, and a forced one that is not allowed refuses. Moving items to the folder they are in, or copying or moving a folder into itself, is refused. `Platform::file_operation` copies or moves with one `IFileOperation` (`FOF_ALLOWUNDO`, the shell's progress and conflict dialogs) on a short-lived thread, and makes shortcuts as `{name}.lnk` with `IShellLinkW` (numbered `name (2).lnk` when taken). Links become `.url` files (`Platform::create_url_file`, numbered when taken) named after the title, with `\/:*?"<>|` replaced by `-`, or after the address as above; the hint is "Create shortcut to '{url}' in '{title}'". A drop that mixes files with drives or links makes new sensors instead (§6.3). Typed characters within a second make one prefix; one that matches nothing with the earlier ones starts a new prefix. The slide and the wheel's glide follow the bar's animation setting (§5.5) and otherwise jump. The grips reach 4 px either side of the free edges; dragging captures the pointer (the popup keeps its side meanwhile, §7.2), and the size is saved when the drag ends. The built-in folder, file and Back icons are placeholders (Appendix C).

### 9.20 `start-menu` — Start Menu  (Launchers; `TurboFolderStartMenu`; requires `start_menu`)

Properties: title (default "Start Menu"), image (default: built-in Start Menu icon), color, opacity, popup size. No folder property. XML keys as the Folder Menu's (`text`, `image`, `colorize`, `opacity`, `popup-width`, `popup-height`); a `folder` is read without a warning and not imported.
* The root listing **merges** `FOLDERID_Programs` (per user) and `FOLDERID_CommonPrograms` (all users). Folders with the same name merge into one virtual folder containing the union of their children. Shortcuts with the same name appear once (per-user wins).
* Otherwise identical to Folder Menu (popup, navigation, keyboard).
* Menu: `Open folder` (per-user Programs), `Open (all users) folder`.
* No drop handling (the all-users Start Menu folder needs admin rights).

Implementation (`sensors/start_menu.rs`): the folders come from `Platform::known_folder` (`FOLDERID_Programs`, `FOLDERID_CommonPrograms`). The `folder-browser` popup takes a list of folders to show as one: each is listed and prepared on the same worker, and the lists are merged by name ignoring case (folders hold every merged path, so going into one lists them all and merges again; of files with the same name the user's wins). A folder that cannot be read is left out unless none can. The header opens the user's folder of the level shown. An empty title shows "Start Menu"; the built-in icon is a placeholder (Appendix C).

### 9.21 `power` — Power  (Launchers; `DockPower`)

| Key | XML key | Type | Default | Options (XML value) |
|---|---|---|---|---|
| mode | `mode` | select | sleep | lock (0) / sign out (1) / switch user (2) / sleep (3) / hibernate (4) / shut down (5) / restart (6). Only modes in `capabilities.power_modes` are offered |
| confirm | `safety-dialog` | bool | true | |
| force | — | bool | false | Sign out / shut down / restart: close apps without asking |
| opacity | `opacity` | opacity, min 20 % | 100 % | |

View: the mode's icon (SVG), 21 px.
Hint: the mode name. Click: if `confirm`, a native dialog ("Do you really want to {lock|sign out of|…} this computer?"), then the platform power action:

| Mode | Win32 |
|---|---|
| lock | `LockWorkStation()` |
| sign out | `ExitWindowsEx(EWX_LOGOFF [\| EWX_FORCE], SHTDN_REASON_FLAG_PLANNED)` |
| switch user | `WTSDisconnectSession(WTS_CURRENT_SERVER_HANDLE, WTS_CURRENT_SESSION, FALSE)` (sign-in screen, session kept) |
| sleep | `SetSuspendState(FALSE, FALSE, FALSE)` |
| hibernate | `SetSuspendState(TRUE, FALSE, FALSE)`; offered only if `GetPwrCapabilities().HiberFilePresent` |
| shut down / restart | enable `SeShutdownPrivilege` (`AdjustTokenPrivileges`), then `InitiateShutdownW(…, SHUTDOWN_POWEROFF \| SHUTDOWN_RESTART [\| SHUTDOWN_FORCE_OTHERS], reason)` |

Implementation (`sensors/power.rs`, `platform/windows/power.rs`): mode ids `lock`, `signOut`, `switchUser`, `sleep`, `hibernate`, `shutDown`, `restart` (the `powerMode` field type lists the modes in `capabilities.power_modes`, plus the stored one if this computer lacks it). `capabilities.power_modes` holds every mode except sleep, offered when `GetPwrCapabilities` reports S1–S3 or Modern Standby (`AoAc`), and hibernate, offered with `SystemS4` and a hibernation file. On Modern Standby without S1–S3 there is no sleep call, so sleep turns the displays off (`WM_SYSCOMMAND` / `SC_MONITORPOWER` to all windows), after which the computer goes to standby. *Force* is shown, and applies, only for sign out, shut down and restart. A mode this computer does not offer (a config from another machine) says "{mode} is not available on this computer" and its click does nothing. The confirmation is the bar's native dialog: the click returns `ClickResult::Confirm` with the question and the mode name as the button, and the sensor acts in `confirmed` when the user agrees. Sleep and hibernate enable `SeShutdownPrivilege` too. Actions run on a short-lived thread, since sleep returns only once the computer wakes and the bar thread must keep answering the broadcasts before it sleeps; failures are logged. The reason for sign out, shut down and restart is "other, planned".

### 9.22 `eject-usb` — Eject USB  (Launchers; `DockUSB`; requires `eject`)

| Key | XML key | Type | Default |
|---|---|---|---|
| opacity | `opacity` | opacity, min 20 % | 100 % |

View: USB icon (30×21, SVG). Hint: `Eject USB`, plus `{n} removable drives` when n > 0.
Click → interactive popup, kind `eject-list`: one row per ejectable volume (removable USB drives, USB-attached disks, optical media) with icon, display name, letter, size and an **Eject** button. Footer button "Safely Remove Hardware…" opens the system dialog (`rundll32.exe shell32.dll,Control_RunDLL hotplug.dll`). Eject → toast with the result; the list refreshes on `volumes` events. With nothing to eject: "Nothing to eject".

**Eject implementation** (`platform/windows/eject.rs`):
1. Volume → device: `GetVolumeNameForVolumeMountPoint` → `IOCTL_STORAGE_GET_DEVICE_NUMBER` → match the disk interface (`GUID_DEVINTERFACE_DISK` via SetupAPI) → `CM_Get_Parent` up to the removable USB device node.
2. `CM_Request_Device_EjectW(devInst, &vetoType, vetoName, …)`; map a veto to a message ("in use by {vetoName}").
3. Optical media: `FSCTL_LOCK_VOLUME` + `FSCTL_DISMOUNT_VOLUME`, then `IOCTL_STORAGE_EJECT_MEDIA`.
4. Fallback: invoke the shell "Eject" verb via `IContextMenu` on the drive item.

Eject USB (`sensors/eject_usb.rs`, the popup in `sensors/common/eject_list.rs`): the sensor subscribes to `volumes` and keeps the drives marked `ejectable` that hold a medium (an empty card reader or DVD drive has nothing to eject), shared with an open list, which takes them when the bar thread is woken; so the list follows drives arriving and going while it is open. The list is 300 px wide: 28 px rows with the 16 px shell icon (the built-in drive icon where there is none), the drive's name as the drive lists write it (`STICK (E:)`), its size (read on a worker) and the *Eject* button, or *Nothing to eject*; a line, then *Safely Remove Hardware…*, which opens the system dialog and closes the list. A button shows *Ejecting…* until its result comes back; the result is the toast, and the sampler reads `volumes` again at once. Hovering or ↑/↓ (Home, End) marks a button or the footer; Enter or a click acts. The list stays open after an eject, so several drives can go one after another. A marked button names its drive, so it stays on it (or goes with it) when other drives come or go; sizes are kept by drive and label, and one that could not be read is read again when the drives change. The USB icon follows the sensor text colour and is a placeholder (Appendix C).

Implementation (`platform/windows/eject.rs`, `sensors/common/eject.rs`): `Platform::eject(root, report)` runs on a short-lived thread and shows the notification `report` words from the result, with the drive's name as the title: "{name} can be safely removed.", "{name} is in use by {app}. Close it and try again." (or without the app when the veto names a device rather than a program), "{name} cannot be ejected.", "{name} could not be ejected." Only volumes the drive list marks `ejectable` are touched, so an internal disk is never asked to go. The disk's device node is found by matching `IOCTL_STORAGE_GET_DEVICE_NUMBER` of the volume against the disk (or CD-ROM) interfaces; from its parent up (at most 8 levels, and not past the first USB device, so a dock's hub never goes with the card reader in it), the first node with `CM_DEVCAP_REMOVABLE` is ejected, with the veto name buffer given so Windows shows no dialog of its own; a veto is judged by its type: an app names its program (the file name of the path Windows gives), a service its name, open files and other devices nobody ("in use"), and is retried twice, 500 ms apart; other vetoes are failures, after which the shell's verb is tried. The Windows drive (`SystemDrive`) is never ejectable, and a disk on an SD or MMC bus only with removable media (a laptop's eMMC system disk sits on the same bus). Before a medium is ejected, a lock another program set on it is lifted (`IOCTL_STORAGE_MEDIA_REMOVAL`). Media in a reader without a removable node above it (an internal SD slot) are ejected like optical media. A veto is reported at once; other failures fall back to the shell verb.

---

## 10. Global settings

Stored under `settings` in the config (§11.2).

### 10.1 General

| Key | Type | Default | Notes |
|---|---|---|---|
| language | auto / available catalogs | auto | Selector hidden while only English exists |
| dateLocale | system / ui | system | Locale for date/number formats and weekday/month names |
| autostart | bool | false | Asked on first run. Implementation (`autostart.rs`): the per-user `Run` key's value `Infodock` holds the exe's path in quotes while it is on (the name is fixed: the uninstaller removes it, §15.3). At start-up the entry follows the setting: written when missing, pointed at this exe when the exe it names is gone, e.g. moved (Task Manager's *Disabled* mark for it, under `Explorer\StartupApproved\Run`, is kept; an entry for another copy that is still there stays, so a copy started once does not take it), removed while the setting is off; until the first run is done (`meta.firstRunDone`, §11.5) an existing entry is taken into the setting instead, so an installer that writes it needs nothing more from the app. Turning the setting on writes the entry and clears that mark; turning it off removes both. A failure is logged and shown under the switch |
| showTrayIcon | bool | true | |
| hotkey | accelerator | `Ctrl+Alt+Shift+I` | Modifiers in the order `Win+Ctrl+Alt+Shift`, then the key: a letter, a digit or a `KeyboardEvent.code` name (`F5`, `ArrowUp`, `Backquote`) |
| hotkeyAction | bringToFront / toggleVisibility | bringToFront | |
| locked | bool | false | |
| popupsEnabled | bool | true | |
| clicksWhenLocked | always / altOnly | always | |
| sensorSpacing | 1–20 px | 10 | |
| newSensorMenu | dialog / submenu | dialog | |

### 10.2 Bar

Stored under `settings.bar` (e.g. `settings.bar.edge`); the other sections use the prefixes shown in their keys, and §10.1 keys sit directly under `settings`. Settings whose default depends on the theme are `null` until the user changes them.

| Key | Type | Default | Notes |
|---|---|---|---|
| edge | top / bottom | bottom | |
| mode | docked / floating | docked | |
| monitor | "primary" \| monitor key | primary | Key from `list_monitors`: `<device>\|<friendly name>\|<width>x<height>`. Matching (§5.1): device and name → name and resolution → device → primary |
| reserveSpace | bool | true | |
| zOrder | topmost / normal / bottom | topmost | |
| hideOnFullscreen | bool | true | |
| height | 22–40 | theme default (28 / 25) | |
| floating | `{ x, y, width }` | null | `x`, `y`: the top-left corner in physical screen px (they name the monitor); `width`: logical px, 200–3840 in Settings and when dragging an end (§5.1) |
| highlightOnHover | bool | false | |
| shadow | bool | false | A soft shadow beside the bar (§5.5) |
| animations | auto / on / off | auto | auto follows the OS |

### 10.3 Appearance

| Key | Type | Default | Notes |
|---|---|---|---|
| theme | modern / classic | modern | |
| colorScheme | system / light / dark | system | Light or dark for the Settings window and the modern bar (§12.2); `system` follows the OS app mode, live. Settings sets the Fluent palette's colour scheme (`unknown` for `system`, which follows the OS through winit) and its title bar's theme |
| background.colorMode | standard / custom | custom | standard = theme colour |
| background.color | color | modern `#1C1C1F`; classic `#2A2A2E` | Tint for skins |
| background.opacity | 0–100 % | modern 80 %; classic 90 % | |
| background.material | none / acrylic / mica | modern: acrylic; classic: none | Disabled automatically when OS transparency effects are off |
| classic.skin | skin id | "gloss" | §12.4 |
| classic.customSkin | file (.png .jpg) | "" | Used when skin = custom |

### 10.4 Sensors (global defaults)

| Key | Type | Default |
|---|---|---|
| sensors.textColor | color | `#FFFFFF` |
| sensors.font | font | modern "Segoe UI Variable Text"; classic "Arial" |
| sensors.fontStyle | fontStyle | modern semibold; classic bold |
| sensors.inactiveMode | opacity / colorize | opacity |
| sensors.inactiveColor | color | `#555555` |
| sensors.inactiveOpacity | % | 78 % |

### 10.5 Popups

| Key | Type | Default |
|---|---|---|
| popups.background | color | modern `#202024`; classic `#000000` |
| popups.opacity | % | modern 92 %; classic 78 % |
| popups.strongTextColor | color | `#93ECFF` |
| popups.textColor | color | `#FFFFFF` |
| popups.font | font | modern "Segoe UI Variable Text"; classic "Arial" |
| popups.fontStyle | fontStyle | modern normal; classic bold |
| popups.fontSize | 9–15 | 12 |
| popups.shadow | bool | true |
| popups.showDelayMs | 0–2000 | 600 |
| popups.hideDelayMs | 0–1000 | 100 |

### 10.6 Applications

| Key | Type | Default |
|---|---|---|
| apps.calendar | file or URI | "" (→ `ms-settings:dateandtime`) |

Implementation (`settings/applications.rs`): the page's *Calendar* row holds the text, saved as it is typed (as the custom skin's path, §10.7), and *Browse…*, whose picker offers programs (`.exe`) first.

### 10.7 Settings window

The window is laid out as Windows 11 Settings is (`ui/settings.slint`, the `Look` global): a navigation on the left with an icon per page (Fluent UI System Icons, MIT), a subtle fill on the page hovered and the one shown, fading in 83 ms as WinUI's, and the accent pill on the latter, sliding to the next page shown (one Tab stop; Up, Down, Home and End move to another page; the properties, the picker and the welcome page select none); the page's title on Windows 11's type ramp (28 px semibold) and the text in Windows 11's Segoe UI Variable at WinUI's sizes (Body 14 px, help 12 px; its Display cut for the title; Segoe UI where it is missing); a page fades in when shown (167 ms); each setting a card (rounded, the card fill and stroke, 4 px apart, at least 68 px tall as WinUI's SettingsCard) with its label and help on the left and its control on the right, a switch with its state (*On*, *Off*) on its left, and group headings (14 px semibold) above their cards. The Layout manager's rows, the picker's types and the gallery's and welcome page's tiles are cards too; the picker's chosen type has the accent pill. Controls share one width (240 px; 280 px for font lists and the form's text, 220 px for a path beside *Browse…*), and a slider shows its value on its left. The pages are at most 1000 px wide, on the left beside the scrollbar. The colours are WinUI's solid layers (the base behind everything, the cards above it) in light and dark: Mica cannot show through, since the software renderer draws every pixel opaque, which would also give text on a transparent background dark edges. Slint scrolls the mouse wheel smoothly itself.

Pages:
* **General**, **Bar**, **Popups**, **Applications**.
* Colours are typed as `#RRGGBB` next to a swatch and saved as soon as the text is a colour, so clicking a button or the navigation (which do not take the focus) finds the colour saved; on Enter or when the field loses focus, anything else is refused with a message. Leaving a page shows the saved values again, which drops a colour left incomplete. Settings whose default comes from the theme (e.g. `popups.background`) show the theme value and a *Use theme default* button; they stay `null` until a different value is set, so leaving a colour box or clicking a slider without changing it does not pin the theme's value (§11.2). A slider shows its value rounded to its step and snaps to it when released, so it shows what is saved. Font lists start with *Theme default (…)* (stored as `null`), followed by the installed families (DirectWrite); a configured family that is not installed stays in the list.
* **Appearance**: theme, skin gallery with previews, colours, material. Implementation (`settings/appearance.rs`): the navigation lists General, Bar, Appearance, Sensors, Popups and Applications. *Colour scheme* (Like Windows, Light, Dark) follows *Theme*. *Background colour* chooses the theme's colour (`standard`) or a colour of the user's (`custom`, with the colour box and *Use theme default*); *Background opacity* has its theme default too. *Material* lists *Theme default (…)* (stored as `null`), then None, Acrylic and Mica; it is disabled, with the reason, while the OS transparency effects are off. With the classic theme a **skin gallery** follows: a tile per built-in skin, the custom image and each usable pack (by its name), with a preview of the bar's background as that skin draws it (`skin_preview`: the bar's renderer, 160 logical px wide, at the bar's height and edge and the window's scale; a plain swatch of the background colour where there is no renderer). A click, or the arrow keys once the gallery has the focus, saves the skin; the configured one is outlined. Previews are drawn again only when what they show changes (the skin, colours, opacity, height, edge, scale or the pack as listed); the skins folder is listed when the page is shown. With the custom skin chosen, a row below holds its path, saved as it is typed, *Browse…*, and why the image cannot be used. A configured skin that is not installed, or a pack that cannot be used, gives the one-line notice of §12.4 under the gallery's heading, with the pack's reason, and no tile is outlined.
* **Sensors**: global defaults, plus the **Layout manager** — three zone lists with drag reordering, edit, remove and add; an accessible alternative to dragging on the bar. Implementation (`settings/layout.rs`): at the top of the page (so *Tools ▸ Layout manager…* lands on it), above the defaults; one list with a heading per zone (with *Add…*, which opens the picker for the end of that zone) and one row per sensor showing its type name and a summary (`SensorType::summary`, e.g. a Text sensor's text) or why it is a placeholder. Each row has *Move up* / *Move down* (crossing into the neighbouring zone at a zone's ends), *Properties* (disabled for placeholders) and *Remove*, which asks for confirmation inside the row. Dragging a row shows an accent line where it would go; releasing away from the list sideways cancels. The list follows the config, so changes made on the bar show at once; it is rebuilt only when a line changed, and a sensor moved with its arrow buttons keeps the focus on that button, so it can be moved on from the keyboard.
* **Help**: getting started, drag & drop rules (§6.3), Folder Menu tips (§9.19). Implementation (`ui/settings.slint` `HelpPage`): in the navigation before About (route `help`, *Tools ▸ Help* on the bar); text only, from the catalog: getting started (the welcome page's tips and Lock with *Clicks while locked*), the keyboard on the bar (§14.4) with a button to General for the hotkey, dropping onto the bar (§6.3), the Folder Menu (§9.19: going in and out, the keys, Alt, drops and their keys, the size), and the Layout manager with a button to it.
* **About**: version, MIT licence, third-party licences (crates, the navigation's icons), the `AboutSlint` widget (Slint licence attribution), "Open logs folder". Implementation (`settings/about.rs`): last in the navigation (route `about`). The version and a line on what the logs are for, *Open logs folder*; the licence (`LICENSE`, its paragraphs reflowed to the page); the crates the Windows build links, as "name version: licence" in a list (`assets/third-party.tsv`, embedded; written by `.github/scripts/third-party.py` from `cargo tree --target x86_64-pc-windows-msvc --edges normal,no-proc-macro`, which resolves features as that build does: build, dev and proc-macro dependencies are not linked); the navigation's icons, Fluent UI System Icons by Microsoft, with their MIT licence (`ui/icons/LICENSE-fluentui-system-icons.txt`, embedded); `AboutSlint`, 128 px wide, the smallest it draws cleanly: its logo 48 px tall (the widget's minimum) and its caption whole. The file lists every package of `Cargo.lock` (those not linked with `-`), so a test that compares the two fails until the script is run after a dependency changes.

Implementation (`settings/window.rs`): the window is created when Settings opens and dropped when it closes; the main thread keeps it in one place, reached only through `with_open` and `with_open_mut`, which borrow it for one closure. A call made while it is borrowed (Slint or winit calling back during a refill, or a modal picker's loop) does nothing and is logged, so no path panics on a second borrow; callbacks take what they need and let go before they save, preview or show a dialog.

---

## 11. Persistence

### 11.1 Locations

* Config: `%APPDATA%\app.infodock.desktop\config.json` (known folder `RoamingAppData`).
* Skin packs: `%APPDATA%\app.infodock.desktop\skins\` (§12.4).
* Logs: `%LOCALAPPDATA%\app.infodock.desktop\logs\infodock.YYYY-MM-DD.log` (kept 7 days).
* Icon cache: in memory only.

### 11.2 Config schema (v1)

```jsonc
{
  "schema": 1,
  "meta": { "createdWith": "1.0.0", "lastWrittenWith": "1.0.0", "firstRunDone": true },
  "settings": { /* §10, nested as named there */ },
  "layout": {
    "left":   [ { "id": "s_8f3k2", "type": "cpu-graph", "props": {} },
                { "id": "s_m41qa", "type": "memory-meter", "props": {} },
                { "id": "s_n7d0w", "type": "network-graph", "props": {} } ],
    "center": [ { "id": "s_1a9x0", "type": "image", "props": { "popupText": "Infodock %v" } } ],
    "right":  [ { "id": "s_q7m2p", "type": "date", "props": {} },
                { "id": "s_z0c4d", "type": "time", "props": { "use24h": true } } ]  // machine-derived, see below
  },
  "state": { "s_u1": { "longestSecs": 1093221 } },
  "legacy": { "unsupported": [ { "type": "DockRSSSensor", "zone": "left", "props": { "url": "…" } } ] }
}
```

* `props` stores **only values that differ from the static defaults**, so defaults can evolve. Two refinements:
  * **Machine-derived** fields (`machineDefault: true`, §8.2) are resolved once by `defaults(ctx)` when the sensor is created or imported, and are **always stored**. A sensor therefore keeps its meaning when drives, the locale or the hardware change. v1: `drive` (Disk Meter, Disk) and `use24h` (Time).
  * Fields whose default is "follow the global setting" (`font` → global sensor font) store `""` for that default. An explicit value is stored even if it currently equals the global setting.
* Unknown `type`s are preserved and rendered as placeholders (§4.6), never deleted silently. Likewise, `props` keys that a type's schema does not know (e.g. written by a newer version) are kept when the sensor is saved.
* Writes are **debounced by 500 ms** and **atomic** (write `config.json.tmp`, flush, rename). `config.json.bak` keeps the previous good version. If the config fails to parse on load: rename it to `config.broken-<timestamp>.json`, load the `.bak` (or defaults), and notify the user with a toast.
* Schema migrations run in Rust (`migrate.rs`) on load; each step is a pure, tested function.

### 11.3 Export / import (JSON)

* **Export**: the whole config, or layout only (`{ schema, layout, state }`, with `legacy` while it holds unsupported legacy sensors), as `.infodock.json`.
* **Import**: *replace* the layout, or *merge* (append imported sensors to their zones). Either kind of file gives only its layout, the state of its sensors and its unsupported legacy sensors: the settings of a whole-config file are not applied. *Replace* also replaces the sensor state and `legacy.unsupported`. Sensor ids are regenerated. The result is an `ImportReport { added, skipped: [{ type, reason }], warnings }`, shown in a dialog.
* Implementation (`config/exchange.rs`, `sensors/import.rs`): the file is migrated to this schema (`migrate.rs`). A newer schema, a file that is not JSON or holds no `layout`, and a file over 16 MB are refused with a message. Layout entries that are not objects with a `type` (and, if any, object `props`) are skipped as "not a sensor entry". Each sensor is checked against its type by the §11.2 rules, as on load: a value of the wrong type gives way to its default with a warning, numbers are clamped, keys the schema does not know are kept, machine-derived props that are missing or invalid are filled from this machine, and only values that differ from the defaults are stored. A type this version does not know is kept as it is (a placeholder, §4.6) with a warning. The legacy XML import (§11.4) goes through the same steps.
* As the user runs it (`transfer.rs`, from the Tools menu, §6.4): *Import layout…* opens the file picker (`*.infodock.json`, or all files), reads the file (an error is shown in a message), asks *Replace the layout* or *Add to the layout* (command links, with the number of sensors the file holds; Cancel changes nothing), imports, and reports: a clean import with a toast ("Layout imported", §7.4), one that skipped entries or has warnings in a dialog listing them (at most 15 lines). A file without sensors this version can show says so, with the lines of what it skipped, and asks nothing, so *Replace* can never empty the bar; a file with more than 500 sensors (its unsupported legacy ones included) is refused. When its sensors' props hold network paths (`\\server\share…`), the question names them (at most three), since the bar opens them, and so connects to their servers, as soon as the sensors are on it. Names and values from the file are cut to 80 characters in the report. JSON files may be UTF-8 or UTF-16, as legacy files. *Export layout…* opens the save picker, whose file type chooses the content: *Infodock layout* (the default) or *Infodock settings and layout* (the whole config); `.infodock.json` is added to a name without `.json`. Dialogs are modal to Settings while it is open. One import or export runs at a time.

### 11.4 Legacy XML layout import

Infodock imports layouts saved in an older XML format (file extension `.xml`):

```xml
<?xml version="1.0" ?>
<sensors>
  <sensor side="left|center|right" type="<LegacyTypeId>">
    <property name="<xml key>">value</property>
    …
  </sensor>
  …
</sensors>
```

**Structure rules**
* `side` gives the zone. Within each side, the document order is the visual left→right order (also for `right`).
* Values are XML-escaped text. Missing properties take Infodock defaults. Unknown property names produce warnings.
* Files that contain a `<sensors>` element embedded in other text (e.g. a settings dump) are accepted: extract the first `<sensors>…</sensors>`.

**Value encodings**

| Kind | Legacy encoding | Infodock |
|---|---|---|
| Boolean | `yes` / `no` | true / false |
| Colour mode | `off:#RRGGBB` (standard), `col:#RRGGBB` (custom), `dyn:#RRGGBB` (dynamic), or a bare `#RRGGBB` (custom) | `{ mode, color }`. For standard, the stored colour is ignored and the sensor default colour is kept |
| Opacity | integer 0–255 | round(v / 2.55) %, then clamped to the field's minimum |
| Interval | seconds (may be fractional) | clamped to the field's range |
| Select | numeric/string values as listed in §9 "Options (XML value)" | the matching option; unknown → default + warning |
| Text | plain text | Empty means "use the default" for fields with `emptyIsDefault`. For `title` fields, empty means "automatic". For Uptime `text`, empty means "no prefix" |
| File / folder paths | may use `/` | normalised to `\`; not checked for existence at import (missing files show up as inactive sensors) |
| Width (graphs) | pixels | rounded down to a multiple of 5, clamped to 50–150 |

**Type mapping**

| Legacy type id | Infodock type | Notes |
|---|---|---|
| `DockText` | `text` | |
| `DockImage` | `image` | |
| `DockSpacer` | `spacer` | |
| `DockDate` | `date` | |
| `DockTime` | `time` | |
| `DockUpTime` | `uptime` | `max-uptime` → state `longestSecs` |
| `DockCPUMeter` | `cpu-meter` | |
| `DockCPUGraph` | `cpu-graph` | |
| `DockCPUGraphDual` | `cpu-graph-dual` | topSeries/bottomSeries keep their defaults (`core0`/`core1`) |
| `DockMemMeter` / `DockMemGraph` | `memory-meter` / `memory-graph` | |
| `DockSwapMeter` / `DockSwapGraph` | `swap-meter` / `swap-graph` | |
| `DockDiskMeter` | `disk-meter` | |
| `DockDisk` | `disk` | |
| `DockNetwork` | `network-graph` | |
| `DockWiFi` | `wifi` | |
| `DockBatteryLife` | `battery` | |
| `DockTrashMeter` | `recycle-bin` | |
| `DockShortcut` | `shortcut` | |
| `TurboFolder` | `folder-menu` | |
| `TurboFolderStartMenu` | `start-menu` | |
| `DockPower` | `power` | property `path` is dropped (a warning when it has a value) |
| `DockUSB` | `eject-usb` | |
| `DockRSSSensor`, `DockStandardMail`, `DockGMail`, `DockGMailForDomains`, `DockYMail`, `DockSpeedFan`, `DockRadio` | — | Stored in `legacy.unsupported` with their properties (any `password` property is **dropped**). Listed in the report as "not supported yet". Kept for future feed/mail/hardware/radio sensors |
| anything else | — | Skipped; the report lists it as an unknown type |

`config/legacy.rs` parses the XML and the shared value encodings; each sensor type converts its own properties in `convert_legacy(raw, cx)` (§8.1). An example file with its expected result is in Appendix B; it MUST be part of the importer's test suite.

Implementation (`config/legacy.rs`, `sensors/legacy.rs`):
* The file's text is UTF-8 (with or without a byte order mark), UTF-16 (with a byte order mark, or without one when its first character is ASCII), or else in an ANSI code page: the one its XML declaration's `encoding` names (`windows-125x`, `cp125x`, `iso-8859-x` and a few East Asian ones), or the system's, which the program that wrote it used (`Platform::decode_code_page`, `MultiByteToWideChar`); Windows-1252 where the platform cannot convert it. The first `<sensors>…</sensors>` (or `<sensors/>`) is read with `quick-xml`: entities and character references are resolved, CDATA is taken as it is, values keep their spaces. Text without a `<sensors>` element and broken XML are refused with a message. A property given twice keeps its last value.
* A sensor without a side, or with another side, is skipped ("no side"); a legacy type Infodock does not know is skipped ("not a sensor type Infodock knows"); the report lists both. Unsupported types drop every property whose name contains `password`.
* Each type names its XML keys (`LegacyKey`): the prop each sets, read by the encoding of that prop's field; a select gets a table of the XML values that differ from its own (`-1` → `os` for the battery's *Assume charged at*; other values are its own ids or numbers); a graph width; a key Infodock has no use for, read without a warning; or a key it drops, with a warning when it has a value (the Power's `path`). Selects match ignoring case; a power mode is read by its id (ignoring case), a hidden field (the popup size) by its default's kind. The default `convert_legacy` reads them: a value that cannot be read (e.g. `maybe` for a boolean, a select value not listed, a dynamic colour where the field has no dynamic mode) keeps the default with a warning, as does a property the type does not name. Booleans also accept `true`/`false` and `1`/`0`; a font style is `bold` and `italic` separated by a comma; a drive may be written `C:` or `C:/`. Types with more to do (a state, a matched adapter) override `convert_legacy`. Graphs share the XML keys of their interval, width, background and grid (`graph::legacy_keys!`, §9.8); the dual graphs' series keys are `graph-top-*`, `graph-bottom-*` and `graph-bottom-flip`.
* The props then go through the import of §11.3: values are clamped, only what differs from the defaults is stored, machine-derived props are filled.

### 11.5 First run

1. Create the default config with the layout of §11.2: **left** — CPU Graph, Memory Meter, Network Graph; **center** — Image (logo, popup "Infodock %v"); **right** — Date, Time.
2. Show a one-time welcome page in Settings: theme choice (Modern/Classic with previews), edge (top/bottom), autostart opt-in, an "Import layout…" button (JSON or legacy XML), and short getting-started tips.

Implementation (`settings/welcome.rs`): Settings opens on the route `welcome` at start-up while `meta.firstRunDone` is false, so a run that ends with the page open shows it again. The page is not in the navigation: an introduction; a tile per theme with a preview of the bar's background in it (`skin_preview`, 240 logical px, the bar's height in that theme, its edge and colours, the colour scheme as the OS's mode resolves it), chosen by a click or the arrow keys and saved at once; *Screen edge*; *Start with Windows* (§10.1); *Import layout…*, whose picker offers both kinds (a `.json` file is read as an exchange file, any other as a legacy layout) and runs the import of §11.3; four tips (the menu, dragging and dropping, hovering and clicking, the hotkey); *Get started*, which closes Settings. `meta.firstRunDone` is set when the page is left: *Get started*, another page, or closing Settings.

---

## 12. Theming

### 12.1 Theme model

The bar thread resolves a `Theme` (`bar/theme.rs`) from the theme defaults (`ThemeDefaults`), the user's settings and the OS appearance (light/dark, accent colour, transparency effects; on Windows from `HKCU\…\Themes\Personalize` and `HKCU\…\DWM\AccentColor`), and re-resolves it when any of them changes. All drawing reads from it:

```
bar height, background colour and opacity, edge padding (7), gap,
sensor text colour, sensor font family / weight / style, small (10|9), medium (11) and large (15) sizes,
popup background / opacity / text colour / strong colour / font / size / radius (8|6) / shadow,
accent colour, inactive style (opacity or colour, §8.5), hairline colour
```

Settings has its own look (§4.3) and does not use the bar theme, except for skin previews rendered by the bar renderer (`skin_preview`).

### 12.2 Modern theme (default)

* Bar: `#1C1C1F` at 80 % over **Acrylic** (DWM via `window-vibrancy`; Mica on Windows 11 when chosen); 1 px hairline `rgba(255,255,255,.08)` on the edge facing the screen center; height 28.
* Typography: Segoe UI Variable; small 10 px, medium 11 px, large 15 px; semibold labels; tabular numerals.
* Icons: the Infodock SVG set (Appendix C), monochrome, rasterised at the physical size and tinted with the sensor text colour.
* Popups: radius 8 px, `#202024` at 92 %, soft shadow, 12 px text.
* Follows `colorScheme` (§10.3; by default the OS **light/dark** mode): in light mode (if the user has not customised colours) the bar is `#F3F3F3` at 80 % with sensor text `#1A1A1A`. "Not customised" means no background colour set in custom colour mode and the default sensor text colour (`Settings::light_bar`); the light look's background is then the theme's colour, which Settings ▸ Appearance shows as the default, so leaving its colour box keeps it.
* Placer: 2 px line in the accent colour.

### 12.3 Classic theme

A compact, denser look:
* Height 25; Arial; label/value 9 px bold; day/hour 15 px bold; month/minutes 11 px.
* Popups: radius 6 px, 13×7 pointer triangle.
* No material (opaque or plain-alpha background), so skins render predictably.
* The background is a **skin** (§12.4), tinted with `background.color` when `background.colorMode = custom` and the user has set a colour; with the theme's colour (`null`), skin packs and the custom image keep their own colours.
* **Tint algorithm** (skins, and icons with a custom colour): per pixel `L = (0.299R + 0.587G + 0.114B) / 255`; output RGB = `L × tintRGB`; alpha preserved. Applied on the CPU to the decoded RGBA and cached keyed by `(source, tint, size)`. Built-in skins apply the tint by using `background.color` as their base colour.
* Background opacity applies to the skin layer only, not to sensors.

### 12.4 Skins

**Built-in skins** (procedural, drawn by `bar/skins/`; base colour = `background.color`):

| Id | Name | Look |
|---|---|---|
| `flat` | Flat | Solid base colour |
| `gloss` | Gloss (default) | Base colour; upper half overlaid with a white gradient 30 % → 5 %; 1 px darker line at the bottom |
| `brushed` | Brushed metal | Vertical gradient (lighter top, darker bottom) + a fine repeating horizontal line texture at 4 % |
| `carbon` | Carbon | Base colour + a 4 px diagonal checker pattern at 6 % |
| `glass` | Glass | Base colour at reduced alpha; 1 px white highlight at 25 % on the inner edge; soft inner glow |
| `custom` | Custom image | `classic.customSkin`, tiled horizontally like a pack's tile without a band (its own height is the height it was drawn for; the same checks as a pack's files); missing or unusable file → Flat |

Implementation (`bar/skins/`): a skin is drawn pixel by pixel into a bitmap of the bar's physical size (lines are one logical px in whole physical px; textures are sized in logical px), kept until the skin, its colour, the bar's size or its edge changes, and drawn with the background opacity (and the hover highlight, §5.5). Gloss's line is black at 30 %. Brushed metal runs from 16 % white at the top to 20 % black at the bottom; its texture is a lighter or darker line per physical row with streaks along it, at most 4 %. Carbon's checker is turned by 45°: diamonds 4 px across, the light ones 6 % white. Glass is the base colour at 55 % alpha, with a glow (10 % white) fading over a third of the height from both long edges and its highlight on the edge facing the screen centre (the top of a bottom bar).

**Skin packs** (user-installable raster skins), in `%APPDATA%\app.infodock.desktop\skins\<pack-id>\`:

```jsonc
// skin.json
{
  "name": "My Skin",
  "image": "tile.png",           // tiled horizontally across the bar
  "imageTop": "tile-top.png",    // optional: used when the bar is at the top
  "imageBottom": "tile-bottom.png", // optional: used when the bar is at the bottom
  "height": 25,                  // optional: bar height the tiles were drawn for (default 25)
  "shadowBand": 9                // optional: px of transparent shadow on the side facing the screen center
}
```

* `name` defaults to the folder's name. Every edge needs a tile: `image`, or both `imageTop` and `imageBottom`. A folder named like a built-in skin (or `custom`) is not a pack.
* Tiles are never stretched horizontally. A tile is `height + shadowBand` px tall (± 1 px tolerance). The band is on the side facing the screen center for the edge the tile is drawn for: at the top of `imageBottom` and `image`, at the bottom of `imageTop`. When `image` serves a top bar, its bar part is used as it is and its band is mirrored below the bar, so the band's darkest rows still touch the bar.
* When the configured bar height (§5.1) differs from `height`, the bar part of the tile is scaled vertically to the bar height; the shadow band keeps its size. This is pre-rendered together with the tint (§12.3) and cached. Sensors always use the full configured bar height. Tiles are drawn for 100 %: at other scales their width, the bar part's height and the band follow the monitor's scale.
* Validation on load: PNG/JPEG magic bytes, ≤ 1 MB per file, ≤ 512 px per side, only files named in `skin.json` (plain file names in the pack's folder), `height` + `shadowBand` 1–512 px, each tile taller than the band. Invalid packs are listed in Settings with the reason. A pack is checked when listed and again when drawn; one that cannot be drawn gives Gloss, and the reason is logged.
* Implementation (`bar/skins/pack.rs`, `raster.rs`): the tile is decoded by the platform (WIC), cut into its bar part and band, tinted, scaled with a linear filter on premultiplied pixels (widened when shrinking), and tiled from the bar's left end; the band is drawn the same way for the side facing the screen center.
* Settings ▸ Appearance: a gallery of the built-in skins and installed packs with previews (`skin_preview`, §10.7), and the skins folder's packs below it: *Install from folder…* and *Install from .zip…*, *Open skins folder*, and per pack *Remove*, which moves its folder to the Recycle Bin; a pack that cannot be used is listed with its reason.
* Installing (`bar/skins/install.rs`): a pack's name is its folder's name; in a `.zip`, `skin.json` lies at the top (the file's name names the pack) or in one folder (which names it). `skin.json` and the files it names are copied into a staging folder in the skins folder (`.install-<name>`; folders whose names start with `.` are not packs), checked there as the bar checks a pack, and put in place of an installed pack of the same name (an update). A `.zip` is at most 32 MB, with entries stored or deflated, each checked against its size and CRC (`bar/skins/zip.rs`); encrypted and ZIP64 archives are refused. After an install or a removal the bar draws its skin again and the gallery lists the folder again; removing the configured pack sets the skin to Gloss.
* A configured skin that is missing on this machine falls back to Gloss, with a one-line notice in Settings ▸ Appearance.

---

## 13. Localisation

* Infrastructure from day one: all user-visible strings go through `i18n::t(key, …args)` — directly in Rust (bar, tray, menus, dialogs, notifications) and through the `I18n` global in `.slint` files (Settings). The catalogs are `i18n/<lang>.json`, embedded in the binary, with stable keys (e.g. `sensor.cpu.hint.activity`); `en.json` is the source.
* **v1 ships English only.** The language selector is hidden while only one catalog exists.
* A missing key in a future catalog falls back to English and is logged once.
* Plurals: catalog entries support the CLDR categories `one/few/many/other`; Rust selects the category with CLDR plural rules for the UI language.
* Weekday and month names, date order and number formatting come from the platform locale (§8.6) using `settings.dateLocale` (default `system` = the OS regional format), never from catalogs.
* String externalisation: a test checks that every key used in `.slint` files exists in `en.json`; `.slint` text comes only from `I18n`, and Rust passes no string literals to drawing, menu or dialog calls (review).

---

## 14. Non-functional requirements

### 14.1 Performance budgets (release build, first-run layout, 1 s graphs)

| Metric | Budget |
|---|---|
| Idle CPU (4-core laptop) | ≤ 0.5 % average over 60 s |
| Memory, Settings closed: private working set (Task Manager's *Memory* column) of `infodock.exe` plus any child processes | ≤ 30 MB (stretch 20 MB) |
| Memory with Settings open | ≤ 20 MB (measured 9.7 MB, plan D16) |
| Memory after Settings was opened and closed | back within the budget above 10 s after closing |
| Cold start → bar visible | ≤ 0.5 s; warm ≤ 0.3 s |
| Full bar redraw (2560 px wide, first-run layout) | ≤ 2 ms |
| Sampler cost per tick | ≤ 2 ms total |
| Hint shown after its delay | ≤ 30 ms |
| Folder Menu popup, folder with 500 items | ≤ 200 ms to first paint |

Techniques: one sampler thread; PDH query handles opened once and reused; no WMI at all (page files from one `NtQuerySystemInformation` call); only dirty sensors are re-measured, and a frame is drawn only when something changed (at most one per 16 ms); software Direct2D (no GPU device in the process); text layouts and decoded bitmaps cached per sensor and invalidated on change; hint and popup windows created on first use and reused; icon LRU cache (256 entries); the Settings window dropped on close. CI measures the budgets on every Windows build (§16) and fails above them: memory and idle CPU on the release build; the start from the time the app logs from its process's creation to the bar's first frame on its window (`bar visible … ms after the process started`, followed by when the start-up's steps were done: config, toolkit, bar thread, window, bar logic, capabilities, sensors), on the first run (cold) and a later one (warm); Windows tests measure the rest and take the median of many runs: the redraw draws the first-run layout 2560 px wide with the Direct2D renderer, as the bar draws a frame; the sampler tick reads the first-run layout's topics on the real providers; the hint is the Date sensor's calendar asked for, laid out, framed and drawn; the Folder Menu opens a folder of 500 items until its first frame shows them (listed, sorted, laid out and drawn, the icons still loading). The window calls that put a frame on the screen (`BitBlt`, `UpdateLayeredWindow`) are not in the tests' times. The tests are built unoptimised; the system's Direct2D and DirectWrite do most of the work, so their numbers are an upper bound for the release build.

### 14.2 Reliability

* The AppBar is removed on normal exit and on panic; Explorer restarts are handled (§5.2).
* Provider errors never crash the app. A failed reading keeps the previous sample and is retried at the next tick (§4.4); after three failed readings in a row the topic's sample is `{ error }` (`Sample::Error`, the platform's message: platform readers that can fail return it, e.g. `Platform::memory`), delivered again only when the message changes, and its retries back off 1 s → 2 s → 4 s … → 30 s (never sooner than the topic's interval; a change or a resume still reads it at once, §4.4). The first success ends it: the sample is delivered and the topic is back on its interval and grid. The bar keeps the error for each sensor on that topic (`bar/sensors.rs`; sensors never see it and keep what they showed): the sensor is drawn inactive, all of it since what it shows is out of date (at the inactive opacity, or 78 % with the colourise style, whose colour would hide its text; §8.5), its hint and accessible value are "Could not read the data: {error}", and the error and the first sample after it reach it even within its own interval. The sampler logs when a topic turns to an error and when it can be read again. Data that is absent rather than unreadable (a missing drive or adapter, no page file) is a sample of its own, which sensors show as `n/a` or inactive.
* Resume, display, DPI and time changes are handled (§5.3).
* A panic on the bar thread is logged, releases the AppBar (panic hook) and restarts the bar thread with the current config.

### 14.3 Security

* No IPC surface: Settings is part of the process. The only cross-process input is the single-instance activation message, which carries no data and only shows the bar and Settings.
* No shell-string execution: launching uses `ShellExecuteExW` with target and parameters as separate fields.
* Launching from the bar (Shortcut, Date/Time click, Wi-Fi hint) validates URIs (`http`, `https`, `ftp`, `mailto`, `ms-settings`, and other registered schemes the user configured). Settings only opens Infodock's own folders.
* No secrets in v1. Future mail sensors MUST store credentials in Windows Credential Manager (`keyring` crate), never in the config.

### 14.4 Accessibility and UX

* The Settings window is fully keyboard-accessible and exposes its controls to screen readers (Slint with AccessKit, which implements UI Automation). Every control on every page has an accessible name that tells it apart from the others on the page (the Layout manager's row buttons name their sensor: "Move CPU Graph up", "Remove CPU Graph"), and Tab reaches each enabled control in reading order: the navigation, then the page from top to bottom and left to right; items are reached with the arrow keys inside their list. Lists of choices announce their state: the skin gallery and the theme tiles carry the chosen tile's name as their value and each tile its selection and place (`accessible-item-index`, `-count`), the Layout manager's rows their place in their zone. A test (`settings/accessibility.rs`) walks every page's elements through Slint's testing API (debug builds keep the elements' names for it) and follows the focus as Tab moves it. The window follows light/dark mode and the display scale. It does not follow Windows' high-contrast themes or the separate *Text size* setting (§19 R8): Slint 1.18 reads neither — its Fluent style has only a light and a dark palette (with the accent colour), and its widgets size text in `rem`, relative to the window's `default-font-size`, which nothing ties to *Text size*. Following them would mean palettes of our own drawn from the system colours while high contrast is on, and reading the text scale (`UISettings.TextScaleFactor`) into `default-font-size` with the window's own fixed text sizes in `rem` (deferred decision 1 of M5).
* The Layout manager provides reordering without dragging.
* The bar is a **UI Automation provider** (`platform/windows/uia.rs`): `WM_GETOBJECT` for `UiaRootObjectId` answers with a root provider for the bar window, a fragment root (control type *Tool bar*, named "Infodock", automation id `InfodockBar`) whose children are the sensors in focus order, Spacers left out: control type *Button*, name = the sensor type's name, automation id = the sensor's id, value = `accessible_value()` through the Value pattern (read-only), bounding rectangle = its place on the screen, and the keyboard focus of the ring below (`HasKeyboardFocus`, the root's `GetFocus`). Once a client has asked, the bar logic describes the bar after each frame (`BarHandler::accessible`: an `AccessibleBar` of `AccessibleItem`s — id, name, value, rect in logical px — and the focus), and the providers answer from that snapshot, never waiting for the bar logic; they use COM threading, so UI Automation calls them on the bar thread. A sensor's runtime id is the window's with a number given to its id, kept while it is moved or changes.
* **Patterns and events**: every sensor has the Invoke pattern (a click) and a sensor whose click opens a popup (`Sensor::has_popup`: Folder Menu with a folder, Start Menu, Eject USB) the ExpandCollapse pattern (expanded while its popup is open; *Expand* clicks it unless open, *Collapse* closes it); `SetFocus` moves the focus ring to it, focusing the bar first. These are deliberate, like keys, so the Lock rules for the pointer do not apply. Patterns must return at once, so the provider queues the request (`AccessibleAction`) and posts the bar window a message, which hands it to the bar logic (`UiEvent::Accessible`). Comparing each snapshot with the last (`platform/accessible.rs`, pure) raises, while clients listen (`UiaClientsAreListening`): a focus-changed event when the ring moves to a sensor while the bar has the keyboard; a value change at most once a second per sensor (a value that changes again within the second waits for a later frame and is announced against the one last announced), so graphs do not flood screen readers; an expand/collapse change when a popup opens or closes; a structure change (children invalidated) when sensors are added, removed or moved. The hint and popup windows carry titles that screen readers read: a text hint's lines joined by commas, a rich hint's sensor value (the calendar's date), a popup's name (`Popup::name`: the Folder Menu's current folder, "Eject USB"), else the app's name.
* **Keyboard on the bar** (`bar/focus.rs`, pure; `bar/thread/keyboard.rs`): the hotkey (with either action, once the bar is shown) gives the bar window the keyboard focus (`SetForegroundWindow`; the hotkey was this process's input), remembering the window of another app that had it, and a **focus ring** goes around the bar itself: a 2 px rounded outline in the accent colour, 1.5 px inside the bar's edges. Tab and Right move the ring to the next sensor, Shift+Tab and Left to the previous one, Home and End to the first and the last, across the zones in visual order (left, centre, right, each in its order) and wrapping at the ends; Spacers, which are dividers, are passed over (`Sensor::focusable`). Around a sensor the ring reaches into the gap, up to 3 px. The focused sensor's hint shows after the show delay, as on hover. Enter and Space click the focused sensor whatever the Lock rules say for the pointer (§6.1), since a key is deliberate; Shift+F10 and the Menu key open its menu, or the bar menu with no sensor focused, beside it (below, or above where there is no room) rather than at the pointer; that bar menu's new sensors go to the end of the left zone. A popup opened from the keyboard takes the focus as on a click; when it closes by Esc or by itself (an item chosen), the bar takes the focus back with the ring where it was. Esc gives the focus back to the remembered window, and so does hiding the bar; a click elsewhere ends the keyboard on the bar without giving anything back. A confirmation or popup of the bar's own (a window of the bar thread) takes the focus for a while, and the ring comes back with it; a press on the bar while the focus is elsewhere ends what was left. A focused sensor that is removed hands the ring to the sensor that took its place, else the one before it, else the bar itself. Alt+F4 on the focused bar does nothing: the bar quits only from its menu.

### 14.5 DPI and multi-monitor

* Per-monitor DPI v2 (application manifest). Layout and drawing are in logical px; the bar and popup windows render at their monitor's scale and convert geometry to physical px. Moving the bar to a monitor with another scale repeats the AppBar negotiation and re-renders after `WM_DPICHANGED`.
* Icons are requested at physical size (`size × scale`, rounded to 16/20/24/32/48).

---

## 15. Project layout, build and packaging

### 15.1 Repository layout

```
infodock/                     the Rust crate
  SPEC.md  LICENSE (MIT)  README.md
  Cargo.toml  build.rs        build.rs compiles the Slint UI and the Windows resources
  src/                        Rust (§4.2)
  ui/                         Slint markup (§4.3)
  i18n/                       message catalogs (§13)
  icons/                      app icons (Appendix C)
  assets/                     icons/*.svg, logo.svg, sounds/chime.wav (embedded)
  resources/                  Windows application manifest
  tests/fixtures/legacy/      legacy XML samples (Appendix B and more)
  docs/manual-test.md         manual test checklist (§16)
```

### 15.2 Commands

`cargo run` · `cargo test` · `cargo fmt --check` · `cargo lint` (clippy, warnings are errors) · `cargo check-win` (clippy for the Windows target, from Linux) · `cargo packager --release` (installer). `lint` and `check-win` are aliases in `.cargo/config.toml`.

### 15.3 Packaging

* NSIS installer built with `cargo-packager`, **per-user** (`installMode: "currentUser"`, no admin rights), Start Menu shortcut. Autostart is offered only to silent installs, as the switch `/AUTOSTART` (for deployments); an interactive install leaves it to the welcome page (§11.5). Configured in `[package.metadata.packager]`.
* Implementation (`[package.metadata.packager.nsis]`, cargo-packager 0.11.8's template; CI pins that version): installs to `%LOCALAPPDATA%\Infodock` without asking for elevation (`asInvoker`), lists the app in *Apps & features* under `HKCU` only, and creates the Start Menu shortcut `Infodock` (in a folder of that name) with the AppUserModelID `app.infodock.desktop` (§7.4), and a desktop shortcut on the finish page's option (always when silent, unless `/NS`). The finish page offers to start Infodock (`/R` when silent). `/AUTOSTART` runs an installer section placed before the template's own (`preinstallSection`): it writes the `Run` value `Infodock` for the installed exe and clears Task Manager's mark for it; the app's first run takes the entry into the setting (§10.1), and where Infodock already ran, its setting wins at the next start. When the installer ends without the exe in place (a failed install, or one closed after its reinstall page removed the old version), an entry naming that exe is removed (`.onInstFailed`, `.onGUIEnd`). The uninstaller closes a running Infodock and removes the files, shortcuts and registration; once the uninstall has succeeded (`un.onUninstSuccess`, so one cancelled at "Infodock is running" changes nothing) it also removes the sign-in entry and Task Manager's mark for it when the entry names the installed exe, the install location the template keeps under `HKCU\Software\jurakovic\Infodock`, and the AppUserModelID's registration; an upgrade (the old uninstaller run with `/P`) keeps all but the last. The config, skins and logs stay. Header and sidebar images are placeholders (`icons/installer/`, Appendix C). CI installs silently, checks these locations, the shortcut's AppUserModelID, that the started app takes an existing sign-in entry on its first run, then sets Task Manager's mark, uninstalls and checks that everything is gone (the mark and the `HKCU` install location too); a second silent install with `/AUTOSTART` must write the entry for the installed exe and clear Task Manager's mark, and its uninstall must remove the entry (`.github/scripts/check-installer.ps1`).
* MSI MAY be built for managed environments.
* **Portable build**: the bare `infodock.exe` from the same release build; it needs nothing else. It stores config, skins and logs in the same per-user folders as the installed app (§11.1); no Start Menu shortcut.

### 15.4 Later

* **Code signing** (to avoid SmartScreen warnings) — after v1.
* **Auto-update** with signed manifests on GitHub Releases (`cargo-packager`'s updater) — after v1.

---

## 16. Testing

| Level | What | How |
|---|---|---|
| Rust unit | Zone layout (§5.4); move index math (§6.2); hint/popup placement (§7.3); formatters (bytes, rates, durations, plurals); dynamic gradient and tint; property schema validation, defaults and storage rules; each sensor's `hint()` against sample fixtures (snapshot); each sensor's drawing against a recording canvas with fixed text metrics (snapshot of draw calls); each sensor's `convert_legacy`; Date/Time formats (all weekday/month/year combinations) against fixed locale data; CPU % from `GetSystemTimes` deltas; network rates (counter reset, first sample, resume); battery estimators (time-to-full smoothing, moving average); AppBar rect negotiation (pure geometry); config migrations; legacy XML import (Appendix B → expected JSON); every type creates a sensor from its defaults and from every option of its lists (selects, power modes, CPU series) | `cargo test` (Linux and Windows) |
| Rust integration (Windows) | Providers return sane values on a real machine; locale data; `list_dir`, link resolution, shell icons, skin-pack validation; the Direct2D renderer draws the first-run layout and each sensor type to PNG (uploaded as a CI artifact for review) | `cargo test -- --ignored` on a Windows runner |
| Budgets | Start the release build, wait 10 s, sum the private working set of `infodock.exe` and its children; same after opening and closing Settings; idle CPU over 60 s; the cold and warm start from the log. The budget tests (`budget_*`) measure the times the app cannot log and record their medians in `target/budgets`. The job summary lists everything; the step fails above a §14.1 budget (memory with Settings open aside) or when a measurement is missing | CI, Windows runner (`.github/scripts/measure-memory.ps1`; `cargo test budget_ -- --ignored --test-threads=1`) |
| Settings | Settings mapping (pages, choices, patches, monitor list) and every sensor schema's form; `visibleIf`; validation messages; every key used in `.slint`, every literal key of the Rust code and every key built from ids exists in `en.json`; the contract with the window (`settings/contract.rs`: the page, row and line numbers both sides use, named in the window's `Pages`, `RowKinds` and `LineKinds`; every settings path the window or the Rust glue saves exists in `Settings` and takes a value of its kind); every Settings page rendered offscreen to PNG (uploaded by CI for review) and driven with clicks and keys, which find their control by its accessible role and name, so they follow the layout | `cargo test` (the rendering with `--ignored`) |
| E2E smoke | App starts, bar visible, add/remove/move a sensor, settings survive a restart | Optional, Windows |
| Manual | Top/bottom; reserve space on/off; two monitors with different DPI; Explorer restart (`taskkill /f /im explorer.exe`); sleep/resume; full-screen video; Wi-Fi with location access off/on; laptop on and off AC; USB stick eject; drag & drop from Explorer and browsers; legacy import of real files; Narrator on the bar | `docs/manual-test.md`, on Windows 10 and 11 test hardware |

Test support: one fake platform for every test (`test_util::FakePlatform`: what a test sets, such as capabilities, volumes, connections, shell icons, the clock and the locale, it answers with; what it is asked to do, such as opening, launching, file operations, ejecting, notifying, it records instead of doing, so the tests run the same on Windows). The sensor types' tests share a `Harness` (`sensors/test_support.rs`: the fake platform, default settings and theme, fixed text metrics; `ctx()`, `layout()`, `drawn()`) and `resolved`/`created` for props from a schema. The bar logic's tests drive it end to end with a fake window and host (`bar/thread/tests/`).

---

## 17. Milestones

| # | Scope | Exit criteria |
|---|---|---|
| **M0** Skeleton | App shell, single instance, tray, bar window with AppBar (top/bottom, monitor, reserve space, Explorer restart), config load/save with atomic writes, modern theme defaults, logging, i18n plumbing. (M0 used Tauri with webviews for the bar and Settings; M1 replaced both, plan D13 and D16.) | Bar reserves space correctly on 1–2 monitors at 100/150 % DPI; survives an Explorer restart; clean exit restores the work area |
| **M1** Framework | Native bar window and renderer, theme, sensor framework and registry, zones and layout, `hint` window and placement, context menus, Lock/Popups, drag reordering, floating mode, animations, Settings window shell, PropertyForm, SensorPicker, Layout manager, sampler | Text/Image/Spacer can be added, configured, moved and removed; hints work; the layout persists; the memory budget (§14.1) holds in CI |
| **M2** System sensors | Date (calendar hint), Time (chime), Uptime, CPU meter/graph/dual, Memory meter/graph, Swap meter/graph, Disk Meter, Network Graph | Values match Task Manager within ±3 % (CPU, memory) and ±1 % (disk); network rates match Resource Monitor; idle CPU budget met |
| **M3** Launchers & hardware | Battery, Wi-Fi, Recycle Bin, Shortcut, Disk, Folder Menu, Start Menu, Power, Eject USB; shell icons; `popup` window; drag & drop of files, folders, drives and URLs onto the bar and onto sensors | All §9.14–9.22 behaviours pass the manual checklist; dragging a link from Edge/Chrome creates a web Shortcut, or a `.url` file in a Folder Menu |
| **M4** Classic, import, packaging | Classic theme, built-in skins, skin packs, tint; JSON export/import; legacy XML import; first-run welcome; autostart; installer | Appendix B imports exactly as specified; installer works per user without admin rights |
| **M5** Polish | Performance pass against §14.1, accessibility pass (UI Automation, keyboard), Help/About (with `AboutSlint`), docs | All budgets met; no clippy warnings; all test suites green |

---

## 18. Backlog (after v1)

Ordered by expected value; each item reuses the framework unchanged.

1. **Feed sensor (RSS/Atom)**: `feed-rs` parsing in Rust. Poll interval options 1, 5, 10, 15 (default), 20, 30, 45 min, 1, 2, 6, 12 h. New items are detected by `(id/guid/link, date)` against the last *seen* snapshot, cached on disk. "New" badge plus `{new}/{total}` status; an error badge on failure. Headline popup with new items in the strong colour and a dates column. Row limit (no limit, 5…50; default 15), title width limit (no limit, 20…100 characters; default 35), reverse order, sort by date. Toast "{n} new headlines". Click opens the site or item. Dropping a feed URL onto the bar creates the sensor. Imports legacy `DockRSSSensor` entries.
2. **Mail sensor**: IMAP (`async-imap` + TLS) with app passwords, plus OAuth2 for Gmail and Microsoft 365 through the system browser and a loopback redirect. Unread count via `STATUS INBOX (MESSAGES UNSEEN)`; a header popup (from, subject, date) with row and width limits; new mail detected by UID. Credentials in Windows Credential Manager. Click opens the mail client or webmail.
3. **Hardware sensor**: reads LibreHardwareMonitor (WMI namespace `root\LibreHardwareMonitor` or its HTTP JSON server). Pick a sensor (temperature, fan, voltage, load); meter with min/max limits and units (°C, °F, rpm, V); min/max tracking in the hint; critical-value toast with a repeat interval (never, 30 s, 1–5 min).
4. **Internet radio**: stream playback (HTML5 audio in a hidden window, or `rodio`/`symphonia`), one station at a time, autoplay option, ICY metadata for the current song in the hint, inactive look while stopped.
5. In-app notification cards as an alternative to toasts.
6. macOS and Linux implementations of the platform layer (macOS: top-edge window + `NSStatusItem`, IOKit/`sysctl` providers; Linux: X11 `_NET_WM_STRUT_PARTIAL`, Wayland layer-shell).
7. Multiple bars (e.g. one per monitor), vertical bars, auto-hide.
8. Favicons for URL shortcuts; the native shell context menu in Folder Menu; dragging items out of Folder Menu.
9. Third-party sensor plugins (sandboxed JS modules with a declared topic allow-list).
10. Additional languages (catalog files only).

---

## 19. Risks and open items

| # | Item | Mitigation / status |
|---|---|---|
| R1 | Browsers offer links in different clipboard formats | The bar's own `IDropTarget` reads `CFSTR_INETURLW`, `UniformResourceLocatorW` and the HTML fragment (§6.3). Fallback: "Paste URL" in Shortcut properties |
| R2 | Per-pixel alpha with a DWM material on a natively drawn window | Validated first in M1 (same window setup as the M0 webview bar: DWM blur-behind for alpha, `window-vibrancy` for the material). Fallback: a layered window (`UpdateLayeredWindow`) with the accent-policy blur |
| R3 | Wi-Fi SSID requires Location permission on Windows 11 24H2+ | Graceful degradation (§9.15) |
| R4 | Custom drawing costs more code than HTML/CSS (text layout, the Folder Menu popup, the calendar, accessibility) | Shared building blocks (§8.3), a small canvas API (§4.6), snapshot tests against a recording canvas, rendered PNGs from CI for review. UI Automation is scoped to name/value/invoke per sensor |
| R5 | CPU % may differ from Task Manager | Use PDH `\Processor Information(_Total)\% Processor Utility` (clamped to 100) for the total, as Task Manager does; per-core via the same counter set; user/kernel split from `GetSystemTimes` |
| R6 | Topmost transparent windows over full-screen games | `hideOnFullscreen` on by default |
| R7 | WebView2 processes may outlive the Settings window | Resolved, then moot: Settings no longer uses WebView2 (plan D15, D16) |
| R8 | Slint does not follow Windows high-contrast themes or the separate *Text size* setting (checked for 1.18 in M5 S9, §14.4), and its 1.x API still changes between minor versions | Follow Slint releases (the lockfile pins the version); whether to follow them anyway is M5's deferred decision 1. Licence attribution: badge in the README, `AboutSlint` in About |
| O1 | Artwork (Appendix C) | To be produced; placeholders acceptable until M5 |
| O2 | Code signing | Deferred until after v1 (§15.4) |
| O3 | Domain name | Optional; not needed for v1 |

---

## Appendix A — Windows data sources

| Topic | Payload | Source | Notes |
|---|---|---|---|
| `cpu` | `{ total, user, kernel, idle, cores: number[], logical }` (0–100) | PDH `\Processor Information(_Total)\% Processor Utility` and `(*)` instances; `GetSystemTimes` deltas for user/kernel/idle (kernel time includes idle: busy = (kernel − idle) + user) | The first sample after start/resume is discarded |
| `memory` | `{ total, available, used, load }` | `GlobalMemoryStatusEx` | `load` = `dwMemoryLoad` |
| `swap` | `{ total, used, load }` (all page files together) | `NtQuerySystemInformation(SystemPageFileInformation)`: each page file's allocated size and use, in pages (`GetSystemInfo` page size); an entry named `swapfile.sys` is skipped | `total == 0` → no page file |
| `disk/<X:\>` | `{ present, total, free }` (kind and label come with `volumes`) | `GetDiskFreeSpaceExW`, `GetDriveTypeW`, `GetVolumeInformationW`, `SHGetFileInfoW(SHGFI_DISPLAYNAME)` | |
| `volumes` | `Volume[]`: `{ root, kind, label, ejectable, media }` | `GetLogicalDriveStringsW` + the above + bus type (`IOCTL_STORAGE_QUERY_PROPERTY`: USB, FireWire, SD, MMC) for "ejectable" (removable and optical drives always are); `IOCTL_STORAGE_CHECK_VERIFY2` for "media" on removable and optical drives (no spin-up) | Read on a worker thread, every 60 s and at once on `WM_DEVICECHANGE` (`DBT_DEVICEARRIVAL`/`DBT_DEVICEREMOVECOMPLETE`, volume type, broadcast to the bar window), which also re-reads the `disk/…` and `path/…` topics |
| `net/<id\|auto>` | `{ present, id, alias, description, up, rxRate, txRate, rxTotal, txTotal, linkRx, linkTx }` | `GetIfTable2` (`InOctets`, `OutOctets`, `OperStatus`, `ReceiveLinkSpeed`, `TransmitLinkSpeed`, `Alias`, `Description`); `GetBestInterfaceEx` for Automatic | Rates from QPC-timed deltas; a negative delta → 0 and new baseline |
| `battery/<i>` | §9.14 | `starship-battery` (state, charge, time to full/empty, vendor/model) + `GetSystemPowerStatus` (AC line) | Read on a worker thread every 10 s; also pushed on `PBT_APMPOWERSTATUSCHANGE` |
| `batteries` | the batteries' names in their places (empty for one without a name or that cannot be read) | the same reading as `battery/<i>` | For Settings' battery lists (§8.2); pushed like `battery/<i>` |
| `wifi` | §9.15 | `wlanapi` | Also pushed on WLAN connect/disconnect notifications |
| `uptime` | `{ bootUnixMs, uptimeSecs }` | `GetTickCount64` | |
| `path/<path>` | `{ kind }`: `missing`, `file` or `folder` | `GetFileAttributesW` | Read on a worker thread (§4.4); a Shortcut checks every 60 s |
| `recycle-bin` | `{ items, sizeBytes }` | `SHQueryRecycleBinW` | Polled at the sensor interval, pushed after actions |

---

## Appendix B — Legacy import example (test fixture)

Save as `tests/fixtures/legacy/example.xml`:

```xml
<?xml version="1.0" ?>
<sensors>
  <sensor side="left" type="DockBatteryLife">
    <property name="battery">0</property>
    <property name="when-in-use-display-percent">yes</property>
    <property name="smooth-time-to-empty">no</property>
    <property name="when-charging-display-percent">yes</property>
    <property name="calculate-time-to-full">no</property>
    <property name="assume-charged">-1</property>
    <property name="alert-level">-1</property>
  </sensor>
  <sensor side="left" type="DockSpacer">
    <property name="show">yes</property>
    <property name="width">9</property>
  </sensor>
  <sensor side="left" type="DockDiskMeter">
    <property name="title">System</property>
    <property name="disk">C:\</property>
    <property name="mode">0</property>
    <property name="interval">10</property>
    <property name="colorize">off:#000000</property>
  </sensor>
  <sensor side="left" type="DockTrashMeter">
    <property name="title"></property>
    <property name="image-empty">D:/Icons/Recycle-Empty.ico</property>
    <property name="image-full">D:/Icons/Recycle.ico</property>
    <property name="opacity">196</property>
    <property name="colorize">off:#000000</property>
  </sensor>
  <sensor side="left" type="DockShortcut">
    <property name="link">D:/Documents</property>
    <property name="param"></property>
    <property name="url"></property>
    <property name="title">Documents</property>
    <property name="image">D:/Icons/Documents.ico</property>
    <property name="colorize">off:#000000</property>
    <property name="opacity">255</property>
  </sensor>
  <sensor side="right" type="DockMemGraph">
    <property name="display-mode">0</property>
    <property name="interval">1</property>
    <property name="width">70</property>
    <property name="bk-color">off:#000000</property>
    <property name="bk-opacity">0</property>
    <property name="grid-color">off:#000000</property>
    <property name="grid-opacity">18</property>
    <property name="graph-color">col:#C0C0C0</property>
    <property name="graph-opacity">137</property>
  </sensor>
  <sensor side="right" type="DockCPUGraphDual">
    <property name="interval">1</property>
    <property name="width">70</property>
    <property name="bk-color">off:#000000</property>
    <property name="bk-opacity">0</property>
    <property name="grid-color">off:#000000</property>
    <property name="grid-opacity">18</property>
    <property name="graph-top-color">col:#666666</property>
    <property name="graph-top-opacity">225</property>
    <property name="graph-bottom-color">col:#C0C0C0</property>
    <property name="graph-bottom-opacity">225</property>
    <property name="graph-bottom-flip">yes</property>
  </sensor>
  <sensor side="right" type="DockUpTime">
    <property name="text"></property>
    <property name="font">Arial</property>
    <property name="fontstyle">bold</property>
    <property name="fontsize">11</property>
    <property name="shadow">no</property>
    <property name="max-uptime"></property>
  </sensor>
  <sensor side="right" type="DockDate">
    <property name="dayname">0</property>
    <property name="month-format">3</property>
    <property name="year-format">4</property>
    <property name="shadow">no</property>
    <property name="show-calendar">1</property>
  </sensor>
  <sensor side="center" type="DockRSSSensor">
    <property name="title">News</property>
    <property name="url">https://example.com/feed.xml</property>
  </sensor>
</sensors>
```

Expected `layout` after `import_legacy(path, "replace")` (ids omitted; only non-default props shown):

```jsonc
{
  "left": [
    { "type": "battery",     "props": {} },                       // all values equal defaults
    { "type": "spacer",      "props": { "showDivider": true } },
    { "type": "disk-meter",  "props": { "title": "System", "drive": "C:\\", "interval": 10 } },
    { "type": "recycle-bin", "props": { "imageEmpty": "D:\\Icons\\Recycle-Empty.ico",
                                         "imageFull": "D:\\Icons\\Recycle.ico", "opacity": 77 } },
    { "type": "shortcut",    "props": { "target": "D:\\Documents", "title": "Documents",
                                         "image": "D:\\Icons\\Documents.ico" } }
  ],
  "center": [],
  "right": [
    { "type": "memory-graph", "props": { "width": 70, "gridOpacity": 7,
                                          "graphColor": { "mode": "custom", "color": "#C0C0C0" },
                                          "graphOpacity": 54 } },
    { "type": "cpu-graph-dual", "props": { "width": 70, "gridOpacity": 7,
                                            "topColor": { "mode": "custom", "color": "#666666" },
                                            "topOpacity": 88,
                                            "bottomColor": { "mode": "custom", "color": "#C0C0C0" },
                                            "bottomOpacity": 88 } },
    { "type": "uptime", "props": { "text": "", "font": "Arial", "fontSize": 11 } },
    { "type": "date",   "props": { "weekday": "none", "month": "number" } }
  ]
}
```

Also expected:
* `legacy.unsupported` contains the `DockRSSSensor` entry (zone `center`, props `title`, `url`).
* `state` has no uptime record (empty `max-uptime`).
* The report: 9 added, 1 skipped (`DockRSSSensor`: "not supported yet"), 0 warnings.
* Rounding: opacity 196 → 77 %, 18 → 7 %, 137 → 54 %, 225 → 88 %.
* The memory graph's `display-mode` 0 equals the default (`bytes`), so it is not stored. The Date sensor's `year-format` 4 (top) and `show-calendar` 1 (monday) equal the defaults. The Uptime `fontstyle` bold equals the Uptime default.
* The Disk Meter's `drive` is stored because it is machine-derived. The Uptime `font: "Arial"` is stored because the default is `""` (global sensor font), whatever the current global font is (§11.2).

---

## Appendix C — Artwork and sounds to produce

All artwork is created for Infodock (or from MIT-licensed icon sets, credited on the About page). Placeholders are acceptable until M5.

| Item | Format / sizes | Used by |
|---|---|---|
| App icon | `.ico` (16, 20, 24, 32, 40, 48, 64, 256) + PNGs for the bundler | Installer, Start Menu, Settings window |
| Tray icon | `.ico` 16/20/24/32, light and dark variants | Tray |
| Logo | SVG (monochrome, tintable) + PNG fallback | Image sensor default, About, welcome page |
| Battery icon | SVG layers: outline/back, level fill mask, level cap, front, overlays (charging, charged, none) | Battery |
| Wi-Fi icon | SVG, levels 0–5 | Wi-Fi |
| Power icons | SVG × 7: lock, sign out, switch user, sleep, hibernate, shut down, restart | Power |
| Recycle Bin | SVG empty + full | Recycle Bin |
| USB | SVG | Eject USB |
| Folder, Start Menu | SVG | Folder Menu, Start Menu |
| Settings navigation | SVG × 8 from Fluent UI System Icons (20 Regular, MIT, credited on About): settings, panel bottom, paint brush, gauge, comment, apps, question circle, info | Settings navigation (`ui/icons/nav-*.svg`) |
| Globe, generic app | SVG | Shortcut fallbacks |
| Popup UI | SVG back arrow; row icons (folder, file, web) | Folder Menu popup |
| Chime | `chime.wav` (16-bit PCM, 44.1 kHz), ≤ 2 s, gentle, normalised to −16 LUFS | Time |
| Installer graphics | NSIS header/sidebar images | Installer |
