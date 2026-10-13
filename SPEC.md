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
| System info helpers | `sysinfo` (fallback/cross-platform), `starship-battery`, `wmi` (page-file sizes only) | |
| Threads | A dedicated **bar thread** (§4.1), a **sampler thread** (§4.4), and worker threads for slow work (directory listings, file operations, icon extraction) | |
| Serialisation | `serde`, `serde_json`; `quick-xml` (legacy import) | |
| Logging | `tracing` + `tracing-appender` (rolling files) | |
| Other OS services | Through the platform layer (§4.6): single-instance lock, known folders, launching files and URLs, file pickers (`IFileOpenDialog`), toast notifications (WinRT), autostart (the per-user `Run` key) | |
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
| `settings` | Slint window, main thread | normal decorated window, opened on a route (a page such as `general`, `bar`, `sensors` or `popups`; `layout` for the Layout manager on the Sensors page; `sensor/<id>`; `new?zone=…&index=…` or `new?target=<id>`), created on demand, single instance, **dropped on close** | Settings, sensor properties, sensor picker, layout manager, welcome, help, about |

Plus a **tray icon** (native, registered for the bar window, bar thread), menu: Show/Hide bar · Lock · Popups · Settings… · Quit. The menu is built from the current state each time it opens.

Threads:
* **main**: Slint's event loop — the Settings window and the global hotkey. Other threads reach it with `slint::invoke_from_event_loop`.
* **bar thread**: a COM single-threaded apartment (`OleInitialize`, needed for drag & drop) with its own message loop. It owns the bar, hint and popup windows, the tray icon, the sensor instances, layout, rendering, menus and native dialogs. Other threads never touch these; they send commands through the `BarController` handle, which queues the command and posts a wake-up message to the bar window.
* **sampler thread**: polls the providers and delivers samples to the bar thread's queue (§4.4).
* **worker threads**: slow work (directory listings, file operations, icon extraction); results go back to the bar thread's queue.

**Single instance**: a named mutex (`Local\app.infodock.desktop`). A second launch finds the running bar window by its class, allows it to take the foreground and posts it a registered activation message; the running instance shows the bar and opens Settings (§6.5), and the second launch exits.

### 4.2 Rust modules

```
src/
  main.rs / lib.rs          start-up and shutdown; starts the bar thread, the sampler and the main event loop
  app.rs                    App (state shared by all threads), bar visibility, quitting
  hotkey.rs                 global hotkey (main thread)
  settings/                 Settings window glue: fills and saves the Slint state; mapping helpers
  config/                   model.rs (typed config), store.rs (load/save/atomic write/debounce),
                            migrate.rs (schema versions), patch.rs, legacy.rs (legacy XML layout import)
  i18n.rs                   message catalogs (shared with Settings), plural rules, t()
  sampler/                  scheduler.rs (topic subscriptions, intervals, fan-out), topics.rs (ids, payloads)
  providers/                cpu.rs, memory.rs, swap.rs, disk.rs, volumes.rs, network.rs,
                            battery.rs, wifi.rs, uptime.rs, recycle_bin.rs
  bar/                      platform-neutral bar logic; runs on the bar thread
    mod.rs                  BarController: the cross-thread handle
    thread.rs               bar thread: event dispatch, sensor instances, timers, redraw scheduling
    layout.rs               zone layout (§5.4), pure
    interact.rs             hover, clicks and Lock rules, reorder drag, drop routing (§6)
    hint.rs, popup.rs       hover hint (§7.1) and interactive popup host (§7.2)
    placement.rs            hint/popup placement (§7.3), pure
    menus.rs, tray.rs       context menus (§6.4) and the tray menu (§6.5)
    theme.rs, skins.rs      resolved theme (§12.1), built-in skins, skin packs and tint (§12.3–12.4)
    anim.rs                 rise animation, hover highlight, fades (§5.5)
  sensors/
    mod.rs                  SensorType and Sensor traits, SensorCtx (§8.1)
    schema.rs               property schema, validation, defaults and storage rules (§8.2, §11.2)
    registry.rs             all sensor types
    format.rs, color.rs     formatting (§8.6), colours and the dynamic gradient (§8.4)
    common/                 two_line.rs, meter.rs, graph.rs, dual_graph.rs, icon_label.rs, history.rs (§8.3)
    <type>.rs               one module per sensor type (text.rs, image.rs, cpu_graph.rs …)
  platform/
    mod.rs                  traits (§4.6)
    canvas.rs               colours, rectangles, brushes, shapes and the Canvas trait (§4.6)
    text.rs                 fonts, text styles, TextLayout and the TextEngine trait
    bitmap.rs               Bitmap (premultiplied RGBA), format sniffing, SVG via resvg, tint (§12.3)
    recording.rs            recording canvas and fixed-metrics text engine (tests on every OS, stub)
    geometry.rs             pure placement math: docking strips, monitor selection
    windows/                window.rs (window classes, message loop, input → events), appbar.rs,
                            instance.rs (single-instance lock, known folders), tray.rs
                            (notification-area icon),
                            monitors.rs, render.rs (Direct2D canvas), text.rs (DirectWrite),
                            image.rs (WIC decoding), menu.rs
                            (TrackPopupMenuEx), dialog.rs (TaskDialogIndirect), droptarget.rs
                            (IDropTarget), uia.rs (UI Automation provider), shell.rs (ShellExecuteEx,
                            IFileOperation, known folders, .lnk/.url), icons.rs (shell icons → bitmaps),
                            power.rs, eject.rs, wlan.rs, pdh.rs, locale.rs (NLS), sound.rs (PlaySound)
    stub.rs                 portable fallback for other OSes (development and tests only)
```

### 4.3 Settings UI

```
ui/
  app.slint                 exports the components Rust uses
  i18n.slint                global I18n { tr(key), tr-with(key, name, value) }, backed by i18n::t
  settings.slint            SettingsWindow and the SettingsState global (pages, rows, controls)
```

Controls bind two-way to the `SettingsState` global. Rust fills it from the config, saves each change as a settings patch (§4.5) and refreshes it from the config store's change listener, so changes made elsewhere (tray, hotkey) show up at once. Generated forms (PropertyForm, §6.7) are `for` loops over the field schemas with one control per field type (§8.2).

### 4.4 Data flow

1. On start, Rust loads the config and starts the sampler thread and the bar thread. The bar thread creates the bar window and the tray icon and instantiates one sensor per layout entry through the registry.
2. For each sensor, the bar thread asks its type for **topic requests** (`topics(props)`), e.g. `{ topic: "cpu", interval_ms: 1000 }`, and subscribes once per *(sensor, topic)*. The **scheduler** keeps the subscriber set per topic and samples at the **minimum** requested interval. Every sample goes to the bar thread's queue; slower subscribers simply use the newest sample.
3. Providers are **shared**: ten CPU sensors cost one CPU sample per tick.
4. Event-driven topics (`volumes`, `battery`, `wifi`, `recycle-bin` after actions) push immediately when the OS notifies.
5. Sensors that need no system data (Text, Image, Spacer, Date, Time) ask for a **wake-up time** instead (e.g. the next minute boundary). The bar thread keeps one timer for the earliest wake-up of all sensors.
6. When a sample or wake-up changes what a sensor shows, it marks itself dirty. The bar thread re-measures dirty sensors, re-runs the layout only if a width changed, and redraws, coalescing to at most one frame per 16 ms. An open hint updates from the same sensor.
7. Graph **history lives in the sensor instance**: a ring buffer of raw values (not pixels). It is not persisted.
8. Config changes always go through the config store, whether they come from Settings commands, the tray, the hotkey or bar actions (drag, drop, remove). The store persists them and notifies its listeners: the bar thread re-creates changed sensors (matched by id) and re-lays out; Settings receives `config-changed`.

Implementation (`sampler/`, `providers/`): topics and samples are typed (`Topic`, `Sample`). The bar thread asks a sensor for its topics once, when it creates it, and passes on only subscriptions that changed (keyed by sensor id; an empty list unsubscribes). The scheduler samples a topic at the shortest interval its subscribers ask for (at least 100 ms), keeps the cadence unless a reading comes more than an interval late, and keeps the newest sample: every subscribe returns it, addressed to that sensor alone, so a sensor receives it at once when it joins a topic. A sensor re-created under the same id (its props changed, or a preview) keeps its topics, so the bar subscribes it again for that catch-up. A failed reading keeps the previous sample and is retried at the next tick. Providers reach the OS through the `Platform` trait (§4.6). On resume (`PBT_APMRESUMEAUTOMATIC`) the bar tells the sampler, which lets providers take new baselines and reads every topic at once; resume and time changes (`WM_TIMECHANGE`) also reach every sensor (`Sensor::on_clock_change`), so clocks re-read the time and move their wake-ups. Clocks align their wake-ups to second and minute boundaries of the local time (`Platform::local_time`), 20 ms past the boundary.

### 4.5 Operations used by Settings

Settings runs in the same process, on the main thread, and calls the core directly; there is no IPC. Errors carry a stable code and a message (e.g. `hotkey`, `invalid`, `io`).

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

Thread-safe services (`Send + Sync`), shared by all threads:

```rust
pub trait Platform: Send + Sync {
    fn capabilities(&self) -> Capabilities;      // which sensors/features exist here
    fn monitors(&self) -> Vec<MonitorInfo>;      // id, name, bounds, work area, scale, primary
    fn appearance(&self) -> OsAppearance;        // light/dark, accent colour, transparency effects, animations
    fn locale(&self) -> &dyn Locale;             // weekday/month names, date patterns, hour cycle,
                                                 // first day of week, decimal separator, collation
    fn shell(&self) -> &dyn Shell;               // open, reveal, list_dir, file ops, links, recycle bin
    fn icons(&self) -> &dyn Icons;               // file/drive icon → RGBA bitmap
    fn power(&self) -> &dyn Power;               // lock/sign out/switch/sleep/hibernate/shut down/restart
    fn devices(&self) -> &dyn Devices;           // volumes, eject
    fn keys(&self) -> Modifiers;
}
pub struct Capabilities {
    pub appbar: bool, pub recycle_bin: bool, pub start_menu: bool, pub wifi: bool,
    pub battery: bool, pub power_modes: Vec<PowerMode>, pub eject: bool, pub per_core_cpu: bool,
}
```

UI services, used only on the bar thread (not `Send`):

* **Windows**: create a native window of a kind (§4.1) with an event handler; set geometry (physical px), show/hide, global alpha (fades), capture the pointer, invalidate, read its scale, set a one-shot timer (the sensors' earliest wake-up, §4.4). Input arrives as platform-neutral events: pointer move/leave/down/up (button, modifiers), wheel, key, DPI change, timer, drag-enter/over/leave/drop (§6.3), and the OS broadcasts the bar cares about (resume, time change, device change, appearance change).
* **Canvas**: drawing in logical px at the window's scale: filled and stroked rectangles, rounded rectangles, lines and polygons, linear gradients, text layouts, bitmaps (with opacity), clipping and layer opacity. **Text**: layout and measurement with a font family, size, weight and style, optional maximum width with an end ellipsis, tabular numerals. **Bitmaps**: decode files (PNG, JPEG, GIF, BMP, ICO; SVG through `resvg`) and wrap RGBA buffers.
* **Menus** and **dialogs**: native popup menus at the cursor (checkable items, separators) and confirmation dialogs.
* **Tray icon**: shown, updated or removed on the bar window; clicks and menu requests arrive as events; removable from any thread on exit.
* **AppBar**: attached to the bar window (`attach_appbar(window, sink) → AppBar`), because it needs that window and its thread.
* **Accessibility**: a UI Automation provider for the bar (§14.4).

On Windows the canvas is Direct2D with a **software** render target drawing into a premultiplied 32-bit bitmap, with DirectWrite text (grayscale antialiasing, since the background is transparent). A full 28 px bar is cheap to rasterise on the CPU, and no GPU driver is loaded into the process. Presentation: the bar copies the bitmap to its DWM-composited window; hint and popup use `UpdateLayeredWindow`; `render_offscreen` returns it as a `Bitmap` (skin previews, CI screenshots). A recording canvas with a fixed-metrics text engine records draw calls, so layout and rendering are unit-testable on any OS; the stub platform uses it as its renderer.

Implementation notes:
* **Text** is laid out by a `TextEngine` (DirectWrite on Windows), separate from the canvas, because sensors measure (§8.1 `measure`) outside drawing. A `TextLayout` carries its size and first baseline and is drawn with `draw_text`. A font family that is not installed falls back to Segoe UI (Segoe UI Variable exists from Windows 11 on).
* **Bitmaps** are premultiplied RGBA in shared memory, so worker threads can decode them. Only the formats above are decoded, recognised by their first bytes, so third-party WIC codecs never see Infodock's files; decoding to a requested height scales with high-quality filtering (an `.ico` picks its closest frame). The renderer uploads each bitmap once and keeps it while the bitmap lives. WIC decodes with its built-in decoder for the recognised container (`GUID_VendorMicrosoftBuiltIn`). Limits, so that a small file cannot ask for gigabytes: files up to 32 MB; results up to 8192 px a side; raster sources up to 64 megapixels (decoders read them whole), while an SVG may declare any size as long as the result fits; an `.svgz` is inflated to at most 32 MB; images embedded in an SVG (data URLs) are skipped above 16 megapixels, embedded `.svgz` inflated with the same cap, and images referenced by path are not loaded. Bitmaps are drawn from whole physical px, so they stay sharp at 125 % and 150 %.
* **Clips and opacity layers** are pushed and popped; layers still open at the end of a frame are closed.

Sensor clicks open programs, files and URIs with `Platform::open` (`ShellExecuteExW` on a short-lived thread with COM, so the bar thread never waits on the shell).

`per_core_cpu` means per-processor counters can be read. It does not depend on the number of logical processors: a single-processor system still has it.

Sensor types declare `requires: &[Capability]`. The picker hides types whose capabilities are missing. Configured sensors that are unsupported on the current platform render as a dimmed placeholder with the hint "Not available on this system". Entries of an unknown type (from a newer version, or a v1 type that arrives in a later milestone) render the same way, labelled with the type id and the hint "This sensor ({type}) is not available in this version of Infodock". The placeholder is an outlined label in the sensor font at reduced opacity; the entry stays in the config unchanged.

---

## 5. The bar

### 5.1 Geometry and placement

* **Edge**: `top` or `bottom` (default `bottom`).
* **Monitor**: `primary` (default) or a specific monitor, identified by device name (e.g. `\\.\DISPLAY2`) with fallback to friendly name + resolution. If the chosen monitor is absent, use primary and keep the setting for when it returns.
* **Width**: the full width of the monitor's work area as negotiated by the AppBar protocol (other appbars are respected). Configurable in floating mode.
* **Height**: theme default (modern **28** logical px, classic **25**), user-adjustable 22–40. All coordinates are logical (DIP); Rust converts to physical px using the monitor scale.
* **Shadow band**: a skin MAY define a shadow band (§12.4). It makes the **window** taller by the band height. The band sits on the side facing the screen center, is transparent and MUST be click-through (window region excludes it, or the shadow is painted inside the bar height — implementer's choice).
* **Modes**
  * **Docked** (default): attached to the chosen edge; optionally **reserves space** (§5.2).
  * **Floating**: free position, draggable by the bar background while unlocked (drag threshold 4 px). Position and width are persisted. No space reservation.
  * Implementation (`platform/geometry.rs` `floating_rect`, `appbar.rs`, `bar/thread.rs`): a floating bar is kept inside the work area of the monitor holding the largest share of it, at that monitor's scale (or the one nearest to its top-left corner); before it was ever moved or sized it starts 800 px wide, centred, 12 px from the work area's `edge` on the selected monitor. While it is dragged the window follows the pointer; the position is stored when the button is released (or the capture is lost). Its hints open away from the nearer screen edge.

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
* Zones MAY overlap when content exceeds the bar width; the center zone is drawn on top. The layout SHOULD then shrink text sensors with an ellipsis, center zone first, then left, then right.
* Implementation: positions computed by `bar/layout.rs` (a pure, tested function) from the widths each sensor measures (`measure()`, §8.1). Values use tabular numerals (DirectWrite `tnum`) and per-sensor reserved minimum widths (§9) so changing numbers do not shift the bar.

### 5.5 Appearance

* **Background**: theme/skin (§12) + `background.color` (tint), `background.opacity`, optional material (Acrylic/Mica), optional `shadow`.
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

Sensor-specific items come from the sensor type's `menu()` (§8.1). Opening a menu hides any hint/popup. The menu opens on the right button's release. Tools items whose features arrive later (import and export in M4, Help in M5) are shown greyed out until then. "New sensor…" opens Settings on `new?zone=<zone>&index=<index>` (bar menu: the zone by thirds and its insertion point, §6.6) or `new?target=<id>` (sensor menu); in submenu mode, choosing a type adds it after the target sensor (or at the zone's insertion point) and opens its Properties (`sensor/<id>`).

### 6.5 Global hotkey and tray

* `hotkey` (default `Ctrl+Alt+Shift+I`) with `hotkeyAction`: `bringToFront` (default) | `toggleVisibility`.
* Settings records the hotkey: the user selects the hotkey button and presses the new combination; Esc, leaving the window or leaving the General page cancels. Keys are read as physical keys, so the keyboard layout does not matter (Ctrl+Alt is AltGr on many layouts). A hotkey needs Ctrl, Alt or Win; only F1–F24 may stand alone, so a hotkey never takes a key away from typing. A refused combination is explained and recording goes on.
* A recorded hotkey is registered before it is saved. If registration fails (typically another app already uses the combination), Settings shows the error next to the button and the previous hotkey stays; since that one still works, the error is not shown again after leaving the page. A hotkey that cannot be registered at start-up is shown the same way; the app MUST still start. Recording the same combination again retries it.
* Tray icon: left-click toggles bar visibility; right-click opens the tray menu (§4.1). It can be hidden (`showTrayIcon`, default true). If both the tray icon and the bar are hidden, the hotkey or a second launch (single instance → show the bar and open Settings) brings it back.

### 6.6 Sensor picker and creation flow

* Modal page in the Settings window (`/settings/new?zone=&index=&target=`); the bar's "New sensor…" opens Settings on that route.
* Sensor types grouped by **category**, sorted alphabetically by localised name; each item shows name and description. A search box filters by name or description.
* With a target sensor: an **Insert: Before / After** selector (default After) and the text "… the '{target name}' sensor".
* **Add** (or double-click) → `add_sensor` with defaults → immediately opens **Properties** for the new instance. **Cancel** in Properties for a *newly created* sensor removes it again. Sensors created by drop skip this step.
* Insertion point when not targeting a sensor: the zone under the right-click position (thirds). Left and center zones append at the end; the right zone inserts at its **start** (next to the center).
* Implementation (`settings/picker.rs`): a click selects, Enter in the search box adds the selection. A search keeps the selection while it is still listed and otherwise selects the first match; Up and Down in the search box move the selection, so the picker works from the keyboard. A route without a zone uses the left zone; without an index, the insertion point above. A target that has been removed meanwhile inserts at the end of the left zone. Leaving the page by the navigation drops it.

### 6.7 Properties dialog

* Route `/settings/sensor/<id>`. The form is generated from the sensor's `props` schema (§8.2); `hidden` fields are not shown.
* Changes **preview live** on the bar (debounced 150 ms, `preview_sensor`). **Save** persists (`update_sensor`) and ends the preview; **Cancel**, or closing Settings, ends the preview, which restores the saved props.
* Implementation (`settings/form.rs`, `settings/properties.rs`): one row per field in schema order, with one control per field type: text box (`text`), switch (`bool`), list (`select`, `font` with a first *Default* entry, `fontStyle`), slider with its unit (`slider`, `interval` in s, `opacity` in %), colour box (`color`), mode list plus a colour box for Custom (`colorMode`), path box with *Browse…* (`file` with its extensions, `folder`; `IFileOpenDialog`). Rows hidden by `visibleIf` disappear at once. Text and path boxes preview as they are typed, colour boxes once the text is a colour (as on the settings pages, §10.7); sliders when released. A colour box reports every edit, so text that is not a colour shows its row's error and disables Save at once (Save is a click that does not take the focus). Leaving the page by the navigation counts as Cancel. `drive`, `netAdapter` and `battery` controls arrive with the sensors that use them (M2, M3).
* Validation per field type; invalid fields show inline errors and disable Save.
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

---

## 8. Sensor framework

### 8.1 Sensor type contract (Rust)

```rust
pub enum Zone { Left, Center, Right }
pub enum Category { General, Info, System, Network, Launchers }

/// One per sensor module; registered in `sensors/registry.rs`. Static and thread-safe.
pub trait SensorType: Send + Sync {
    fn id(&self) -> &'static str;                         // stable id, e.g. "cpu-graph"
    fn legacy_types(&self) -> &'static [&'static str];    // legacy XML type ids handled (§11.4)
    fn category(&self) -> Category;
    fn name(&self) -> MsgKey;                             // localised display name
    fn description(&self) -> MsgKey;
    fn requires(&self) -> &'static [Capability];
    fn schema(&self) -> &'static [PropField];             // ordered fields: drives the Settings form,
                                                          // validation and static defaults (§8.2)
    fn machine_defaults(&self, ctx: &DefaultsCtx) -> Props;  // ctx: locale, volumes, CPU count (§11.2)
    fn from_legacy(&self, raw: &LegacyProps, legacy_type: &str, report: &mut ImportReport) -> Props;
    fn create(&self, props: &Props, state: &State) -> serde_json::Result<Box<dyn Sensor>>;
                                                          // fails only if the typed props do not match
                                                          // the schema; the bar then shows a placeholder
}

/// A placed instance. Lives on the bar thread; re-created when its props change.
pub trait Sensor {
    fn topics(&self) -> Vec<TopicRequest>;                // system data needed (none for UI-only sensors)
    fn on_sample(&mut self, topic: &Topic, sample: &Sample, cx: &mut SensorCtx) -> Dirty;
    fn on_hover(&mut self, hovered: bool, now: Instant, cx: &SensorCtx) -> Dirty;  // pointer entered/left
    fn wake_at(&self, now: Instant) -> Option<Instant>;   // timer-driven sensors (clock, uptime)
    fn on_wake(&mut self, now: Instant, cx: &mut SensorCtx) -> Dirty;  // Dirty = bool: what it shows changed
    fn measure(&mut self, cx: &LayoutCtx) -> f32;         // width in logical px; height = bar content height;
                                                          // builds and keeps text layouts and bitmaps
    fn draw(&self, canvas: &mut dyn Canvas, cx: &DrawCtx);
    fn hint(&self, cx: &SensorCtx) -> Option<HintContent>;
    fn click(&mut self, cx: &mut SensorCtx) -> ClickResult;  // None, Handled, or OpenPopup(Box<dyn Popup>)
    fn menu(&self, cx: &SensorCtx) -> Vec<MenuItem>;      // sensor-specific items (§6.4)
    fn on_menu(&mut self, item: MenuId, cx: &mut SensorCtx);
    fn drop_effect(&self, d: &DropPayload, cx: &SensorCtx) -> Option<DropEffect>;  // accepts? (§6.3)
    fn on_drop(&mut self, d: DropPayload, cx: &mut SensorCtx);
    fn inactive(&self) -> bool;                           // drives inactive rendering (§8.5)
    fn accessible_value(&self, cx: &SensorCtx) -> String; // §14.4
}

/// What a sensor may use; built by the bar thread for each call.
pub struct SensorCtx<'a> {
    pub id: &'a str, pub settings: &'a Settings, pub theme: &'a Theme,
    pub locked: bool, pub alt: bool, pub platform: &'a dyn Platform,
    /* set_state(patch), update_props(patch), notify(title, body), open_settings(route), t(key, args) */
}

/// Measuring may lay out text and decode images; drawing happens in the sensor's place on the bar.
pub struct LayoutCtx<'a> { pub theme: &'a Theme, pub text: &'a dyn TextEngine, pub scale: f32 }
pub struct DrawCtx<'a> { pub theme: &'a Theme, pub rect: RectF }   // logical px, bar coordinates
```

All trait methods except the identity ones, `create`, `measure` and `draw` have default implementations (no topics, no hint, no click action, …), so simple types stay small. Each type deserialises a typed props struct from its schema's defaults overlaid with the stored props; the schema is the single source of static defaults. Settings reads the schemas through the sensor types operation (§4.5).

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
| `drive` | dropdown from `list_volumes` (live) | `X:\` | option `hideOptical` |
| `netAdapter` | dropdown from `list_net_adapters` (live) + "Automatic" | `{ id, name }` | |
| `battery` | dropdown from `list_batteries` | index | |
| `hidden` | — | any | internal values (e.g. popup size) |

Field definition: `{ key, type, label, description?, default, machineDefault?, ...typeOptions, visibleIf? }`. `visibleIf` is declarative so Settings can evaluate it: `{ key, in: [values] }` (shown while another field has one of these values).

Validation (`sensors/schema.rs`): each value is normalised for its field type: colours to upper-case `#RRGGBB`, numbers clamped to their range (`slider`, `interval`, `opacity`), drives to `X:\`. A value of the wrong type or shape is invalid: on load it is ignored (the field takes its default) and logged; `update_sensor` rejects it with error code `invalid`. An empty `text` with `emptyIsDefault` means the default.

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
* The plot is split into two halves of (plotHeight − 1)/2 with a 1 px gap. The **top** series grows upward from the middle line. The **bottom** series grows **downward** from the middle (`flipBottom = true`, default) or upward from the bottom edge (`false`).
* Colour and opacity per series (`topColor`/`topOpacity`, `bottomColor`/`bottomOpacity`).
* Values are normalised to 0..1 per series by the owning sensor.

**Icon + two-line** (`common/icon_label.rs`) — Battery, Wi-Fi, Recycle Bin: an icon (≤ bar height − 2) followed by a two-line block after a 2–3 px gap.

**History** (`common/history.rs`): fixed-capacity ring buffer of `f32`; resizing keeps the newest samples.

Implementation notes: the meter's value column fits `100%` as formatted (§8.6) or the value if wider; its bar and fill edges sit on whole physical px. Graph grid lines are one physical px, the vertical ones every 5 px from the plot's left, the horizontal ones every 5 px from its bottom; transparent layers are not drawn. Column edges are snapped, so at 150 % columns are alternately 2 and 1 physical px wide. A graph's text column fits the sensor's reserve (e.g. `100%`) or its text, and both lines end at the sensor's right edge.

### 8.4 Colour modes and the dynamic gradient

* `standard`: use the sensor text colour (meters: 59 % opacity); follows global settings.
* `custom`: the chosen colour.
* `dynamic` (green → yellow → red by value `v ∈ [0,1]`):
  ```
  v < 0.5:  R = round(v × 510), G = 255, B = 0
  v ≥ 0.5:  R = 255, G = round((1 − v) × 510), B = 0
  ```
* Icon sensors offer only Standard/Custom: Custom **tints** the icon (§12.3 tint algorithm); Standard leaves it unchanged.

### 8.5 Inactive rendering

When `inactive()` returns true, the sensor's icon or graphic is rendered as:
* `inactiveMode = "opacity"` (default): opacity `inactiveOpacity` (default 78 %);
* `inactiveMode = "colorize"`: tinted with `inactiveColor` (default `#555555`).

Labels stay readable; only the icon/graphic dims. v1 uses it for: Disk (drive absent), Wi-Fi (disconnected), Battery (no battery), Shortcut (target missing), Swap (no page file).

### 8.6 Formatting helpers (`sensors/format.rs`)

* **Bytes** (`formatBytes(n, digits = auto)`), base **1024**, units `TB, GB, MB, KB, bytes` (singular `1 byte`). Auto digits: `0` → "0"; `< 10` → 2 decimals; `< 100` → 1 decimal; otherwise 0. Plain bytes always use 0 decimals. Examples: `0 bytes`, `512 bytes`, `1.50 KB`, `12.3 MB`, `431 GB`.
* **Rates** (`formatRate`): same scaling, units `TB/s … KB/s, B/s`.
* **Durations**: compact `3d 04h 12m` (uptime; the leading unit unpadded and units above it left out: `4h 05m`, `12m`); long `3 days, 4 hours, 12 minutes` with plural rules, leaving out parts that are zero (`1 day`; under a minute `0 minutes`); battery `h:mm`.
* Implementation (`platform/locale.rs`, `sensors/dates.rs`): `Platform::locale(dateLocale)` returns the locale as plain data (weekday and month names, the long, short and month-day date patterns in the Windows picture format, the hour cycle, am/pm, the first day of the week, the decimal separator), read with `GetLocaleInfoEx` (`ui` reads `en-US`, the only UI language in v1) and English (United States) elsewhere. Rust formats dates with those patterns and computes weekdays, days of the year and ISO weeks itself, so all of it is tested against fixed locales. Genitive month names (e.g. Russian `d MMMM`) are not used yet.
* Implementation (`sensors/format.rs`): a value that would round to 1024 moves to the next unit (`1.00 MB`, not `1024 KB`), and the automatic digits are chosen after rounding (`10.0 KB`, not `10.00 KB`); units and words are catalog entries (`format.*`, plural forms through `i18n::t_count`).
* **Percent**: integer, `37%` (locale-specific spacing allowed). Decimal separator from the locale.
* **Dates**: weekday and month names, date patterns and the hour cycle from the platform `Locale` (Windows NLS: `GetLocaleInfoEx`, `GetDateFormatEx`, `GetTimeFormatEx`) for `settings.dateLocale`; ISO 8601 week numbers computed in Rust.

---

## 9. Sensor catalog (v1)

Conventions:
* Properties table: key · XML key (§11.4) · type · default · range/options.
* Intervals are in seconds.
* "Hint" is the hover popup text; `{braces}` mark data. "Click to …" lines are added per §6.1.
* Sizes refer to the modern 28 px bar; classic geometry is given where it matters.

### 9.1 `text` — Text  (General; XML `DockText`)

| Key | XML key | Type | Default | Notes |
|---|---|---|---|---|
| text | `text` | text | "Text" | |
| font | `font` | font | `""` (Default = global sensor font) | |
| fontStyle | `fontstyle` | fontStyle | normal | XML `""` → normal, `bold` → bold, `italic` → italic, `bold, italic` → boldItalic |
| fontSize | `fontsize` | slider 9–25 | 13 | |
| shadow | `shadow` | bool | false | 1 px offset (1,1). Shadow colour = black if the text colour is light (grey = 0.2R + 0.5G + 0.3B ≥ 128), else white |

View: one line, vertically centred, in the sensor text colour. No hint, no click action.

### 9.2 `image` — Image  (General; `DockImage`)

| Key | XML key | Type | Default |
|---|---|---|---|
| image | `image` | file (.png .jpg .gif .bmp .ico .svg) | "" → Infodock logo |
| color | `colorize` | colorMode (standard/custom) | standard |
| opacity | `opacity` | opacity, min 20 % | 100 % |
| popupText | `text` | text | "" — `%v` is replaced by the app version |

View: the image scaled to height = bar height − 1, keeping aspect ratio, decoded at that physical height. A file that cannot be read shows the logo (and is logged). Hint = popupText (none if empty). The first-run instance uses popupText "Infodock %v".

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

Implementation (`sensors/date.rs`): the month and year lines are 11 px on the two-line block's rows (§8.3) for that size; the day line is the weekday name, a comma and the day. Without a year, the line is the weekday (`dddd, ` or `ddd, `) followed by the locale's month-day pattern (`MMMM` → `MMM` for short months) or, for numeric months, its short-date pattern without the year (`M/d/yyyy` → `M/d`). The sensor reads the clock at its first wake-up (due at once) and then each minute, which also notices midnight; a clock change reads it again. The calendar is a rich hint (`RichHint`, §7.1): columns at least 29 px, numbers right-aligned 4 px from the column's edge, today on a rounded highlight, the week line half a row below (`Week {week}, Day {day}`).

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

State (persisted in the config's `state`, §11.2): `longestSecs` (XML key `max-uptime`, format `d:h:m:s`; empty = none).

Topic `uptime`. Rust takes the boot time from `GetTickCount64()`; the UI counts locally between samples (resample every 60 s and on resume).

View: `{prefix} 3d 04h 12m`, updated at minute boundaries.
Hint: `3 days, 4 hours, 12 minutes`; when a longer record exists, a second line `Longest uptime: 12 days, 1 hour, 3 minutes`. The record updates whenever the current uptime exceeds it.

Implementation (`sensors/uptime.rs`): the sensor counts from its last sample and wakes at each whole minute of uptime. The record updates in memory at once but is stored only when it has grown by 10 minutes, so the config is not rewritten every minute: a sensor hands state to store through `Sensor::take_state`, which the bar passes on after samples and wake-ups (`BarHost::set_sensor_state` → the config's `state`, §11.2).

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
**Alert**: while charging, when `percent > alertAbove`, show the toast "Battery at {p}% — you can unplug the charger", once per charging session.

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

### 9.17 `shortcut` — Shortcut  (Launchers; `DockShortcut`)

| Key | XML key | Type | Default | Notes |
|---|---|---|---|---|
| target | `link` | file (any file or folder; filter .exe .cmd .bat .lnk .url .ps1 + All) | "" | Application, document or folder |
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

### 9.18 `disk` — Disk  (Launchers; `DockDisk`)

| Key | XML key | Type | Default |
|---|---|---|---|
| drive | `disk` | drive (optical/removable included) | machine-derived: first drive |
| title | `title` | text | "" → shell display name, e.g. `Local Disk (C:)` |
| image | `image` | file | "" → shell drive icon |
| color / opacity | `colorize` / `opacity` | | standard / 100 % |

Topic `volumes` (event-driven list). View: the drive icon only.
Hint: `Open '{display name}'` (or the title). Click: open the drive. Menu: **Eject** for removable and optical drives (§9.22; on success the toast "{name} can be safely removed", on failure a toast with the reason, e.g. "in use by {app}"). Inactive while the drive is absent; an optical drive without media stays active, with the hint `{name} — no media`.

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

### 9.20 `start-menu` — Start Menu  (Launchers; `TurboFolderStartMenu`; requires `start_menu`)

Properties: title (default "Start Menu"), image (default: built-in Start Menu icon), color, opacity, popup size. No folder property.
* The root listing **merges** `FOLDERID_Programs` (per user) and `FOLDERID_CommonPrograms` (all users). Folders with the same name merge into one virtual folder containing the union of their children. Shortcuts with the same name appear once (per-user wins).
* Otherwise identical to Folder Menu (popup, navigation, keyboard).
* Menu: `Open folder` (per-user Programs), `Open (all users) folder`.
* No drop handling (the all-users Start Menu folder needs admin rights).

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

---

## 10. Global settings

Stored under `settings` in the config (§11.2).

### 10.1 General

| Key | Type | Default | Notes |
|---|---|---|---|
| language | auto / available catalogs | auto | Selector hidden while only English exists |
| dateLocale | system / ui | system | Locale for date/number formats and weekday/month names |
| autostart | bool | false | Asked on first run |
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
| floating | `{ x, y, width }` | null | `x`, `y`: the top-left corner in physical screen px (they name the monitor); `width`: logical px, 200–3840 in Settings |
| highlightOnHover | bool | false | |
| shadow | bool | false | |
| animations | auto / on / off | auto | auto follows the OS |

### 10.3 Appearance

| Key | Type | Default | Notes |
|---|---|---|---|
| theme | modern / classic | modern | |
| colorScheme | system / light / dark | system | Light or dark for the Settings window and the modern bar (§12.2); `system` follows the OS app mode |
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

### 10.7 Settings window

Pages:
* **General**, **Bar**, **Popups**, **Applications**.
* Colours are typed as `#RRGGBB` next to a swatch and saved as soon as the text is a colour, so clicking a button or the navigation (which do not take the focus) finds the colour saved; on Enter or when the field loses focus, anything else is refused with a message. Leaving a page shows the saved values again, which drops a colour left incomplete. Settings whose default comes from the theme (e.g. `popups.background`) show the theme value and a *Use theme default* button; they stay `null` until a different value is set, so leaving a colour box or clicking a slider without changing it does not pin the theme's value (§11.2). A slider shows its value rounded to its step and snaps to it when released, so it shows what is saved. Font lists start with *Theme default (…)* (stored as `null`), followed by the installed families (DirectWrite); a configured family that is not installed stays in the list.
* **Appearance**: theme, skin gallery with previews, colours, material.
* **Sensors**: global defaults, plus the **Layout manager** — three zone lists with drag reordering, edit, remove and add; an accessible alternative to dragging on the bar. Implementation (`settings/layout.rs`): at the top of the page (so *Tools ▸ Layout manager…* lands on it), above the defaults; one list with a heading per zone (with *Add…*, which opens the picker for the end of that zone) and one row per sensor showing its type name and a summary (`SensorType::summary`, e.g. a Text sensor's text) or why it is a placeholder. Each row has *Move up* / *Move down* (crossing into the neighbouring zone at a zone's ends), *Properties* (disabled for placeholders) and *Remove*, which asks for confirmation inside the row. Dragging a row shows an accent line where it would go; releasing away from the list sideways cancels. The list follows the config, so changes made on the bar show at once; it is rebuilt only when a line changed, and a sensor moved with its arrow buttons keeps the focus on that button, so it can be moved on from the keyboard.
* **Help**: getting started, drag & drop rules (§6.3), Folder Menu tips (§9.19).
* **About**: version, MIT licence, third-party licences (crates, icon sets), the `AboutSlint` widget (Slint licence attribution), "Open logs folder".

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

* **Export**: the whole config, or layout only (`{ schema, layout, state }`), as `.infodock.json`.
* **Import**: *replace* the layout, or *merge* (append imported sensors to their zones). Sensor ids are regenerated. The result is an `ImportReport { added, skipped: [{ type, reason }], warnings }`, shown in a dialog.

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
| File / folder paths | may use `/` | normalised to `\` on Windows; not checked for existence at import (missing files show up as inactive sensors) |
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
| `DockPower` | `power` | property `path` is dropped (warning) |
| `DockUSB` | `eject-usb` | |
| `DockRSSSensor`, `DockStandardMail`, `DockGMail`, `DockGMailForDomains`, `DockYMail`, `DockSpeedFan`, `DockRadio` | — | Stored in `legacy.unsupported` with their properties (any `password` property is **dropped**). Listed in the report as "not supported yet". Kept for future feed/mail/hardware/radio sensors |
| anything else | — | Skipped, with a warning |

`config/legacy.rs` parses the XML and the shared value encodings; each sensor type converts its own properties in `from_legacy(raw, legacy_type, report)` (§8.1). An example file with its expected result is in Appendix B; it MUST be part of the importer's test suite.

### 11.5 First run

1. Create the default config with the layout of §11.2: **left** — CPU Graph, Memory Meter, Network Graph; **center** — Image (logo, popup "Infodock %v"); **right** — Date, Time.
2. Show a one-time welcome page in Settings: theme choice (Modern/Classic with previews), edge (top/bottom), autostart opt-in, an "Import layout…" button (JSON or legacy XML), and short getting-started tips.

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
* Follows `colorScheme` (§10.3; by default the OS **light/dark** mode): in light mode (if the user has not customised colours) the bar is `#F3F3F3` at 80 % with sensor text `#1A1A1A`.
* Placer: 2 px line in the accent colour.

### 12.3 Classic theme

A compact, denser look:
* Height 25; Arial; label/value 9 px bold; day/hour 15 px bold; month/minutes 11 px.
* Popups: radius 6 px, 13×7 pointer triangle.
* No material (opaque or plain-alpha background), so skins render predictably.
* The background is a **skin** (§12.4), tinted with `background.color` when `background.colorMode = custom`.
* **Tint algorithm** (skins, and icons with a custom colour): per pixel `L = (0.299R + 0.587G + 0.114B) / 255`; output RGB = `L × tintRGB`; alpha preserved. Applied on the CPU to the decoded RGBA and cached keyed by `(source, tint, size)`. Built-in skins apply the tint by using `background.color` as their base colour.
* Background opacity applies to the skin layer only, not to sensors.

### 12.4 Skins

**Built-in skins** (procedural, drawn by `bar/skins.rs`; base colour = `background.color`):

| Id | Name | Look |
|---|---|---|
| `flat` | Flat | Solid base colour |
| `gloss` | Gloss (default) | Base colour; upper half overlaid with a white gradient 30 % → 5 %; 1 px darker line at the bottom |
| `brushed` | Brushed metal | Vertical gradient (lighter top, darker bottom) + a fine repeating horizontal line texture at 4 % |
| `carbon` | Carbon | Base colour + a 4 px diagonal checker pattern at 6 % |
| `glass` | Glass | Base colour at reduced alpha; 1 px white highlight at 25 % on the inner edge; soft inner glow |
| `custom` | Custom image | `classic.customSkin`, tiled horizontally; missing file → Flat |

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

* Tiles are never stretched horizontally. A tile is `height + shadowBand` px tall (± 1 px tolerance).
* When the configured bar height (§5.1) differs from `height`, the bar part of the tile is scaled vertically to the bar height; the shadow band keeps its size. This is pre-rendered together with the tint (§12.3) and cached. Sensors always use the full configured bar height.
* Validation on load: PNG/JPEG magic bytes, ≤ 1 MB per file, ≤ 512 px per side, only files named in `skin.json`. Invalid packs are listed in Settings with the reason.
* Settings ▸ Appearance: a gallery of the built-in skins and installed packs with previews (`skin_preview`), "Install skin pack…" (pick a folder or `.zip`; `install_skin`), "Open skins folder" (`shell_open` on `skinsDir`), "Remove" (`remove_skin`).
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

Techniques: one sampler thread; PDH query handles opened once and reused; no WMI on hot paths (page-file sizes cached for 60 s); only dirty sensors are re-measured, and a frame is drawn only when something changed (at most one per 16 ms); software Direct2D (no GPU device in the process); text layouts and decoded bitmaps cached per sensor and invalidated on change; hint and popup windows created on first use and reused; icon LRU cache (256 entries); the Settings window dropped on close. CI measures memory on every Windows build (§16).

### 14.2 Reliability

* The AppBar is removed on normal exit and on panic; Explorer restarts are handled (§5.2).
* Provider errors never crash the app. A failing provider sends `{ error }` samples; sensors show `n/a`, become inactive and show the error in their hint. The provider retries with backoff (1 s → 30 s).
* Resume, display, DPI and time changes are handled (§5.3).
* A panic on the bar thread is logged, releases the AppBar (panic hook) and restarts the bar thread with the current config.

### 14.3 Security

* No IPC surface: Settings is part of the process. The only cross-process input is the single-instance activation message, which carries no data and only shows the bar and Settings.
* No shell-string execution: launching uses `ShellExecuteExW` with target and parameters as separate fields.
* Launching from the bar (Shortcut, Date/Time click, Wi-Fi hint) validates URIs (`http`, `https`, `ftp`, `mailto`, `ms-settings`, and other registered schemes the user configured). Settings only opens Infodock's own folders.
* No secrets in v1. Future mail sensors MUST store credentials in Windows Credential Manager (`keyring` crate), never in the config.

### 14.4 Accessibility and UX

* The Settings window is fully keyboard-accessible and exposes its controls to screen readers (Slint with AccessKit, which implements UI Automation). It follows light/dark mode and the display scale. High-contrast themes and the separate *Text size* setting are not followed yet (§19 R8).
* The Layout manager provides reordering without dragging.
* The bar is a UI Automation provider (`platform/windows/uia.rs`): one element per sensor with name = sensor name and value = `accessible_value()`. The hotkey focuses the bar; Tab moves between sensors (focus ring in the accent colour), Enter = click, Shift+F10 or the Menu key = context menu, Esc returns focus to the previous window.

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

* NSIS installer built with `cargo-packager`, **per-user** (`installMode: "currentUser"`, no admin rights), Start Menu shortcut, optional autostart. Configured in `[package.metadata.packager]`.
* MSI MAY be built for managed environments.
* **Portable build**: the bare `infodock.exe` from the same release build; it needs nothing else. It stores config, skins and logs in the same per-user folders as the installed app (§11.1); no Start Menu shortcut.

### 15.4 Later

* **Code signing** (to avoid SmartScreen warnings) — after v1.
* **Auto-update** with signed manifests on GitHub Releases (`cargo-packager`'s updater) — after v1.

---

## 16. Testing

| Level | What | How |
|---|---|---|
| Rust unit | Zone layout (§5.4); move index math (§6.2); hint/popup placement (§7.3); formatters (bytes, rates, durations, plurals); dynamic gradient and tint; property schema validation, defaults and storage rules; each sensor's `hint()` against sample fixtures (snapshot); each sensor's drawing against a recording canvas with fixed text metrics (snapshot of draw calls); each sensor's `from_legacy`; Date/Time formats (all weekday/month/year combinations) against fixed locale data; CPU % from `GetSystemTimes` deltas; network rates (counter reset, first sample, resume); battery estimators (time-to-full smoothing, moving average); AppBar rect negotiation (pure geometry); config migrations; legacy XML import (Appendix B → expected JSON) | `cargo test` (Linux and Windows) |
| Rust integration (Windows) | Providers return sane values on a real machine; locale data; `list_dir`, link resolution, shell icons, skin-pack validation; the Direct2D renderer draws the first-run layout and each sensor type to PNG (uploaded as a CI artifact for review) | `cargo test -- --ignored` on a Windows runner |
| Memory | Start the release build, wait 10 s, sum the private working set of `infodock.exe` and its children; same after opening and closing Settings. Fails above the §14.1 budget | CI, Windows runner |
| Settings | Settings mapping (pages, choices, patches, monitor list) and every sensor schema's form; `visibleIf`; validation messages; every key used in `.slint` exists in `en.json`; every Settings page rendered offscreen to PNG (uploaded by CI for review) | `cargo test` (the rendering with `--ignored`) |
| E2E smoke | App starts, bar visible, add/remove/move a sensor, settings survive a restart | Optional, Windows |
| Manual | Top/bottom; reserve space on/off; two monitors with different DPI; Explorer restart (`taskkill /f /im explorer.exe`); sleep/resume; full-screen video; Wi-Fi with location access off/on; laptop on and off AC; USB stick eject; drag & drop from Explorer and browsers; legacy import of real files; Narrator on the bar | `docs/manual-test.md`, on Windows 10 and 11 test hardware |

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
| R8 | Slint does not follow Windows high-contrast themes or the separate *Text size* setting, and its 1.x API still changes between minor versions | Follow Slint releases (the lockfile pins the version); revisit for the M5 accessibility pass. Licence attribution: badge in the README, `AboutSlint` in About |
| O1 | Artwork (Appendix C) | To be produced; placeholders acceptable until M5 |
| O2 | Code signing | Deferred until after v1 (§15.4) |
| O3 | Domain name | Optional; not needed for v1 |

---

## Appendix A — Windows data sources

| Topic | Payload | Source | Notes |
|---|---|---|---|
| `cpu` | `{ total, user, kernel, idle, cores: number[], logical }` (0–100) | PDH `\Processor Information(_Total)\% Processor Utility` and `(*)` instances; `GetSystemTimes` deltas for user/kernel/idle (kernel time includes idle: busy = (kernel − idle) + user) | The first sample after start/resume is discarded |
| `memory` | `{ total, available, used, load }` | `GlobalMemoryStatusEx` | `load` = `dwMemoryLoad` |
| `swap` | `{ total, used, load, files: [{ name, allocated, used, peak }] }` | PDH `\Paging File(*)\% Usage` + WMI `Win32_PageFileUsage` (sizes, cached 60 s) | `total == 0` → no page file |
| `disk/<X:\>` | `{ present, total, free, availUser, kind, label, displayName }` | `GetDiskFreeSpaceExW`, `GetDriveTypeW`, `GetVolumeInformationW`, `SHGetFileInfoW(SHGFI_DISPLAYNAME)` | |
| `volumes` | `Volume[]` | `GetLogicalDriveStringsW` + the above + bus type (`IOCTL_STORAGE_QUERY_PROPERTY`) for "ejectable" | Pushed on `WM_DEVICECHANGE` (`DBT_DEVICEARRIVAL`/`DBT_DEVICEREMOVECOMPLETE`, volume type) |
| `net/<id\|auto>` | `{ id, name, alias, description, up, rxRate, txRate, rxTotal, txTotal, linkRx, linkTx }` | `GetIfTable2` (`InOctets`, `OutOctets`, `OperStatus`, `ReceiveLinkSpeed`, `TransmitLinkSpeed`, `Alias`, `Description`); `GetBestInterfaceEx` for Automatic | Rates from QPC-timed deltas; a negative delta → 0 and new baseline |
| `battery/<i>` | §9.14 | `starship-battery` (state, charge, time to full/empty, vendor/model) + `GetSystemPowerStatus` (AC line) | Also pushed on `PBT_APMPOWERSTATUSCHANGE` |
| `wifi` | §9.15 | `wlanapi` | Also pushed on WLAN connect/disconnect notifications |
| `uptime` | `{ bootUnixMs, uptimeSecs }` | `GetTickCount64` | |
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
| Globe, generic app | SVG | Shortcut fallbacks |
| Popup UI | SVG back arrow; row icons (folder, file, web) | Folder Menu popup |
| Chime | `chime.wav` (16-bit PCM, 44.1 kHz), ≤ 2 s, gentle, normalised to −16 LUFS | Time |
| Installer graphics | NSIS header/sidebar images | Installer |
