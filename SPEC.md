# Infodock — Specification

| | |
|---|---|
| Status | Draft 3 — ready for implementation |
| Date | 2026-10-11 |
| Product | **Infodock** — a compact system-information and launcher bar for the desktop |
| Target | Windows 10/11 first; macOS/Linux later |
| Stack | **Tauri 2** (Rust core) + **TypeScript / Svelte 5** UI |
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

* G1 — A **single, standalone, always-on desktop bar app**; no host engine or runtime besides the OS WebView.
* G2 — The full v1 sensor set (§1.2) with the behaviour defined in §9.
* G3 — **Light footprint**: idle CPU ≤ 0.5 %; private memory ≤ 100 MB including WebView2 (stretch goal 70 MB).
* G4 — **Correct Windows integration**: AppBar work-area reservation, multi-monitor, per-monitor DPI, Explorer restarts, sleep/resume, full-screen apps.
* G5 — **Modern default theme**, plus a compact **Classic** theme with skins (§12).
* G6 — **Data-driven sensors**: adding a sensor type means adding one TS module (plus a Rust provider if it needs new system data).
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
| Stack | Tauri 2 (Rust + TypeScript UI) |
| v1 sensors | §1.2 |
| Look | Modern default theme + Classic theme with skins |
| Assets | All artwork and sounds are produced for Infodock (Appendix C). No third-party artwork is bundled except permissively licensed icon sets (e.g. Fluent UI System Icons, MIT) with attribution |
| Languages | i18n infrastructure from day one; ship English only |
| First-run layout | Left: CPU Graph, Memory Meter, Network Graph · Center: Image (logo) · Right: Date, Time (§11.5) |

---

## 3. Technology stack

| Layer | Choice | Notes |
|---|---|---|
| App shell | **Tauri 2.x** (latest stable 2.x at implementation start; pin exact versions in the lockfiles) | System WebView2 (Evergreen) on Windows |
| Core language | **Rust** (stable, edition 2021+) | |
| Windows API | `windows` crate (Win32: Shell, UI.Shell, NetworkManagement.IpHelper, NetworkManagement.WiFi, System.Performance (PDH), System.Power, System.Shutdown, System.RemoteDesktop, Devices.DeviceAndDriverInstallation, Storage.FileSystem, Graphics.Gdi, UI.WindowsAndMessaging) | |
| System info helpers | `sysinfo` (fallback/cross-platform), `starship-battery`, `wmi` (page-file sizes only) | |
| Async/runtime | `tokio` (Tauri's runtime) + a dedicated **sampler thread** for polling | |
| Serialisation | `serde`, `serde_json`; `quick-xml` (legacy import) | |
| Logging | `tracing` + `tracing-appender` (rolling files) | |
| Tauri plugins | `global-shortcut`, `notification`, `autostart`, `single-instance`, `dialog`, `opener` (URLs only; files go through the platform layer) | |
| Window effects | `window-vibrancy` (Acrylic/Mica) | Optional per theme |
| UI language | **TypeScript** (strict) | |
| UI framework | **Svelte 5** (runes) + Vite | Small runtime; fine-grained reactivity suits many small live sensors |
| UI tests | Vitest (+ @testing-library/svelte) | |
| JS toolchain | **Bun** as package manager and script runner (`bun install`, `bun run tauri dev`) | Bun is not a runtime here; the WebView runs the UI |
| Packaging | Tauri bundler → **NSIS per-user installer** (no admin) + optional MSI | |

**Why not Electron or Node/Bun alone:** Node and Bun cannot create desktop windows on their own. Electron would idle at about 150 MB+ for a 25 px bar and would still need native addons for the AppBar and system counters. **Why not a pure Rust UI:** popups, calendars, animations, theming and skins are much cheaper to build in HTML/CSS. The native parts stay in Rust either way.

---

## 4. Architecture

### 4.1 Process and windows

One process (single instance enforced). Four Tauri windows (webviews), all loading the same Vite app with different routes:

| Window label | Route | Properties | Purpose |
|---|---|---|---|
| `bar` | `/bar` | undecorated, transparent, skip taskbar, not resizable, no shadow (unless the theme enables one), z-order per settings | The bar |
| `hint` | `/hint` | undecorated, transparent, topmost, **non-activating**, **click-through** (`set_ignore_cursor_events(true)`), skip taskbar, created hidden at startup | Hover popups (text hints, calendar) |
| `popup` | `/popup` | undecorated, transparent, topmost, focusable, skip taskbar, hides on blur, created hidden at startup | Interactive popups (Folder Menu, Eject list) |
| `settings` | `/settings/*` | normal decorated window, created on demand, single instance | Settings, sensor properties, sensor picker, layout manager, help, about |

Plus a **tray icon** (Rust side), menu: Show/Hide bar · Lock · Popups · Settings… · Quit.

> Non-activating hint window: use the Tauri builder's `focusable(false)` if available in the pinned version. Otherwise the platform layer sets `WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW` on the HWND and shows it with `SW_SHOWNOACTIVATE`.

### 4.2 Rust core modules

```
src-tauri/src/
  main.rs / lib.rs          app builder, plugin registration, window creation, tray
  config/                   model.rs (typed config), store.rs (load/save/atomic write/debounce),
                            migrate.rs (schema versions), legacy.rs (legacy XML layout import)
  sampler/                  scheduler.rs (topic subscriptions, intervals, fan-out to Channels),
                            topics.rs (topic ids & payload types)
  providers/                cpu.rs, memory.rs, swap.rs, disk.rs, volumes.rs, network.rs,
                            battery.rs, wifi.rs, uptime.rs, recycle_bin.rs
  platform/
    mod.rs                  traits: Platform, AppBar, Shell, Power, Devices, Icons, Monitors
    geometry.rs             pure placement math: docking strips, monitor selection
    windows/                appbar.rs, monitors.rs, shell.rs (ShellExecuteEx, IFileOperation,
                            known folders, .lnk/.url), icons.rs (shell icon → PNG),
                            power.rs, eject.rs, wlan.rs, pdh.rs, msgwnd.rs (hidden top-level
                            window for WM_DEVICECHANGE, WM_POWERBROADCAST, WM_TIMECHANGE; not
                            message-only, because message-only windows receive no broadcasts),
                            keys.rs (GetAsyncKeyState), droptarget.rs (M3b)
    stub.rs                 portable fallback for other OSes (development only)
  windowing/                bar.rs (placement, appbar glue, rise animation), hint.rs,
                            popup.rs (placement), settings.rs  (not `windows/`, which would
                            clash with the `windows` crate)
  commands/                 Tauri command handlers: config, data, shell, system, ui, app
  protocol/                 icon.rs (shell icons), skin.rs (user skin packs)
```

### 4.3 Frontend modules

```
src/
  main.ts                   route by window label → BarApp / HintApp / PopupApp / SettingsApp
  lib/ipc.ts                typed wrappers for all commands/events; subscribe() helper over Channels
  lib/config.ts             reactive config store mirrored from Rust ("config-changed" events)
  lib/i18n.ts               message catalogs, plural rules, t()
  lib/format.ts             bytes, rates, durations, percent, dates
  lib/color.ts              hex parsing, luminance, dynamic gradient, tint
  lib/theme.ts              applies theme tokens and skin to :root
  bar/                      BarApp.svelte, Zone.svelte, SensorSlot.svelte (hover/click/drag glue),
                            layout.ts (zone alignment), dnd.ts (reorder + external drop),
                            menus.ts (native context menus)
  hint/                     HintApp.svelte (text hint, rich hint host)
  popup/                    PopupApp.svelte (hosts sensor popup components)
  settings/                 SettingsApp.svelte, pages/*, PropertyForm.svelte (schema → form),
                            SensorPicker.svelte, LayoutManager.svelte
  sensors/
    registry.ts             all SensorType definitions
    types.ts                SensorType, PropSchema, HintContent, SensorCtx …
    common/                 Meter.svelte, Graph.svelte (canvas), DualGraph.svelte,
                            TwoLine.svelte, IconLabel.svelte, history.ts (ring buffer)
    <type>/index.ts         one folder per sensor type (+ View.svelte, optional Popup.svelte)
  themes/                   modern.css, classic.css, skins/*.css (built-in classic skins)
  assets/                   icons/*.svg, sounds/chime.ogg, logo.svg
  i18n/                     en.json
```

### 4.4 Data flow

1. On start, Rust loads the config and creates the windows. The bar UI calls `get_config`.
2. For each placed sensor, the UI asks its sensor type for **topic requests** (`topics(props)`), e.g. `{ topic: "cpu", intervalMs: 1000 }`.
3. The UI calls `subscribe(topic, intervalMs, channel)` once per *(sensor, topic)*. The Rust **scheduler** keeps the subscriber set per topic and samples at the **minimum** requested interval. Each sample goes to every subscriber's `Channel`; slower subscribers simply use the newest sample.
4. Providers are **shared**: ten CPU sensors cost one CPU sample per tick.
5. Event-driven topics (`volumes`, `battery`, `wifi`, `recycle-bin` after actions) push immediately when the OS notifies.
6. Pure-UI sensors (Text, Image, Spacer, Date, Time) need no topics and run their own timers in the UI.
7. Graph **history lives in the UI**: a ring buffer per sensor instance holding raw values (not pixels). It is not persisted.
8. Config changes always go through Rust commands. Rust persists them and broadcasts `config-changed` to all windows.

### 4.5 IPC surface

Commands (all `async`; errors as `{ code, message }`):

| Group | Command | Signature (TS view) |
|---|---|---|
| config | `get_config` | `() → Config` |
| | `update_settings` | `(patch: DeepPartial<Settings>) → Config` |
| | `add_sensor` | `(zone, index, type, props?) → SensorInstance` |
| | `update_sensor` | `(id, props: Partial<Props>) → SensorInstance` |
| | `set_sensor_state` | `(id, state: object) → void` (runtime state, e.g. uptime record, popup size) |
| | `move_sensor` | `(id, zone, index) → void` |
| | `remove_sensor` | `(id) → void` |
| | `export_config` / `import_config` | `(path, mode: "replace"\|"merge") → ImportReport` |
| | `import_legacy` | `(path, mode) → ImportReport` (§11.4) |
| data | `subscribe` | `(topic, intervalMs, channel: Channel<Sample>) → subId` |
| | `unsubscribe` | `(subId) → void` |
| system | `capabilities` | `() → Capabilities` (§4.6) |
| | `list_volumes` / `list_net_adapters` / `list_batteries` / `list_monitors` / `list_fonts` / `list_skins` | `() → …[]` |
| | `install_skin` | `(path) → SkinInfo` (folder or `.zip`; validated, then copied into the skins folder, §12.4) |
| | `remove_skin` | `(id) → void` |
| shell | `shell_open` | `(target, { args?, cwd?, verb? }) → void` (ShellExecuteEx) |
| | `shell_reveal` | `(path) → void` (Explorer with the item selected) |
| | `resolve_link` | `(path) → { target, args, cwd, iconPath? }` (.lnk/.url) |
| | `path_info` | `(paths[]) → { path, exists, kind: "file"\|"folder"\|"drive", volume? }[]` (`volume` = volume root; used to classify drops §6.3, check Shortcut targets §9.17, and choose move vs copy §9.19) |
| | `list_dir` | `(path, { showHidden }) → DirEntry[]` |
| | `known_folder` | `("programs"\|"commonPrograms"\|"desktop"\|…) → path` |
| | `file_op` | `(op: "move"\|"copy"\|"link", paths[], destDir) → void` (IFileOperation with native progress/conflict UI) |
| | `create_url_shortcut` | `(url, destDir, title?) → path` |
| | `recycle_bin` | `(action: "open"\|"empty"\|"info") → RecycleBinInfo?` |
| | `move_to_recycle_bin` | `(paths[]) → void` |
| | `eject_volume` | `(path) → void` |
| | `open_safely_remove` | `() → void` |
| | `power_action` | `(mode, { force }) → void` |
| | `get_modifier_keys` | `() → { ctrl, shift, alt, win }` |
| ui | `show_hint` / `update_hint` / `hide_hint` | `(anchor: Rect, content: HintPayload)` |
| | `show_popup` / `hide_popup` / `popup_ready` | `(kind, anchor, payload)`; `popup_ready(w, h)` reports the measured size |
| | `open_settings` | `(route: string)` |
| | `bar_focus` | `()` |
| app | `app_info` | `() → { version, locale, dataDir, logDir, skinsDir }` |
| | `set_autostart` / `quit` | |

Events (Rust → windows): `config-changed`, `bar-geometry` (bar rect, edge, monitor, scale), `popup-payload`, `hint-payload`, `system-resume`, `theme-changed` (OS light/dark, accent, transparency changes), `drop` (M3b).

Custom protocols:
* `icon://localhost/<urlencoded path>?size=<px>` (WebView2 sees `http://icon.localhost/…`). Returns a PNG of the shell icon at the requested **physical** size, LRU-cached (default 256 entries).
* `skin://localhost/<pack>/<file>` serves files of installed skin packs (§12.4), restricted to files listed in each pack's `skin.json`.

### 4.6 Platform abstraction

```rust
pub trait Platform: Send + Sync {
    fn capabilities(&self) -> Capabilities;           // which sensors/features exist here
    fn appbar(&self) -> &dyn AppBar;                   // reserve edge, notifications
    fn monitors(&self) -> Vec<MonitorInfo>;            // id, name, bounds, work area, scale, primary
    fn shell(&self) -> &dyn Shell;                     // open, reveal, list_dir, file ops, links, recycle bin
    fn icons(&self) -> &dyn Icons;                     // file/drive icon → RGBA
    fn power(&self) -> &dyn Power;                     // lock/sign out/switch/sleep/hibernate/shut down/restart
    fn devices(&self) -> &dyn Devices;                 // volumes, eject
    fn keys(&self) -> Modifiers;
}
pub struct Capabilities {
    pub appbar: bool, pub recycle_bin: bool, pub start_menu: bool, pub wifi: bool,
    pub battery: bool, pub power_modes: Vec<PowerMode>, pub eject: bool, pub per_core_cpu: bool,
}
```

`per_core_cpu` means per-processor counters can be read. It does not depend on the number of logical processors: a single-processor system still has it.

The AppBar needs the bar's window handle and its UI thread, so it is attached to the bar window when that window is created (`platform::attach_appbar(window, sink) → AppBar`) instead of being reached through `Platform::appbar()`.

Sensor types declare `requires: (keyof Capabilities)[]`. The picker hides types whose capabilities are missing. Configured sensors that are unsupported on the current platform render as a dimmed placeholder with the hint "Not available on this system".

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

### 5.2 Work-area reservation (AppBar)

When `reserveSpace = true` (default) and docked:

1. Register with `SHAppBarMessage(ABM_NEW)` using a private callback message.
2. `ABM_QUERYPOS` → adjust → `ABM_SETPOS` with `ABE_TOP`/`ABE_BOTTOM` and the monitor rect; then move the window to the returned rect.
3. Callback handling: `ABN_POSCHANGED` → repeat step 2; `ABN_FULLSCREENAPP` → §5.3; `ABN_STATECHANGE` → repeat step 2.
4. Repeat step 2 on `WM_DISPLAYCHANGE`, `WM_DPICHANGED`, `WM_SETTINGCHANGE (SPI_SETWORKAREA)`, and after edge, monitor or height changes.
5. **Explorer restart**: on the registered `TaskbarCreated` message → `ABM_REMOVE` + `ABM_NEW` + reposition.
6. On exit (and in a panic hook): `ABM_REMOVE`. If the process is killed, Windows removes the appbar together with the window, so no recovery logic is needed.

When `reserveSpace = false`: place the window at the edge without registering; maximised windows may overlap it.

Implementation note: the AppBar is registered for the bar window itself, and the bar window is subclassed with `SetWindowSubclass` to receive the callback message, `WM_ACTIVATE`/`WM_WINDOWPOSCHANGED` (forwarded as `ABM_ACTIVATE`/`ABM_WINDOWPOSCHANGED`) and the broadcasts above. A hidden message-only window does not work: message-only windows receive no broadcasts (`TaskbarCreated`, `WM_DISPLAYCHANGE`, `WM_SETTINGCHANGE`), and an appbar registered under a different window than the visible bar loses the shell's z-order coupling. The subclass also keeps other code (Aero Snap, DPI-change resizes, activation) from moving the bar or lifting it out of its z-order.

### 5.3 Z-order, full-screen, focus, power

* `zOrder`: `topmost` (default, usual for appbars) | `normal` | `bottom` (always behind normal windows). With `bottom`, the hotkey brings the bar to the front until it loses focus.
* **Full-screen apps** (`hideOnFullscreen`, default true): on `ABN_FULLSCREENAPP(true)` for our monitor → drop topmost and hide the hint/popup windows; restore on `false`.
* **Sleep/resume**: `WM_POWERBROADCAST / PBT_APMRESUMEAUTOMATIC` → emit `system-resume`. Providers reset delta baselines (CPU, network); the UI re-renders Date/Time/Uptime immediately.
* **Time or time-zone changes** (`WM_TIMECHANGE`) → Date/Time refresh.
* Clicking a sensor MUST NOT steal focus from the user's window, except for actions that open UI.

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
* Implementation: absolute positioning computed by `layout.ts` (a pure, tested function). Widths are measured with `ResizeObserver`. Values use `font-variant-numeric: tabular-nums` and per-sensor reserved minimum widths (§9) so changing numbers do not shift the bar.

### 5.5 Appearance

* **Background**: theme/skin (§12) + `background.color` (tint), `background.opacity`, optional material (Acrylic/Mica), optional `shadow`.
* **Highlight on hover** (`highlightOnHover`, default false): on pointer enter, background opacity → min(100 %, opacity × 1.7) with a 300 ms ease-in; back with a 300 ms ease-out on leave.
* **Rise animation**: on startup and after an edge change, the bar slides in from off-screen on its edge and fades 0→1 over **300 ms, ease-out**. After an edge change, the bar first slides out toward the old edge (300 ms). If the OS "animation effects" setting is off (`SPI_GETCLIENTAREAANIMATION`), there is no animation.
* **Sensor text colour**: `settings.sensors.textColor` (default `#FFFFFF`) applies to all labels and to icons in "standard" colour mode.

---

## 6. Interaction

### 6.1 Lock mode and clicks

* `locked` (default false; context-menu item "Lock" ✓).
* **Unlocked**: pointer-down on a sensor dims it (opacity −40 %). Moving more than 4 px starts a **reorder drag** (§6.2). Releasing without moving is a **click**.
* **Locked**: reordering is disabled. Click behaviour follows `clicksWhenLocked`:
  * `always` (default): clicks work normally.
  * `altOnly`: clicks only work while Alt is held. This makes the bar a passive display.
* Hint texts end with a "Click to …" line only when a click would actually do something in the current mode.
* Double-click and middle-click have no default action (reserved).

### 6.2 Reordering by drag

* Only while unlocked. During the drag, hints are hidden and suppressed.
* Over a sensor, a **placer** (2 px vertical line in the accent colour) shows the drop position: **left** of the target if the pointer is over its left half, otherwise right of it.
* Over an empty area: the zone is chosen by the pointer's x in **thirds** of the bar width. The placer appears at that zone's insertion point: left zone → after its last sensor (or at `pad`); center → after its last sensor (or at the center); right zone → before its first sensor (or at `barWidth − pad`).
* Drop → `move_sensor(id, zone, index)`. The index accounts for removing the source first: within the same zone, if the source is before the target, the target index shifts down by one.
* Esc cancels the drag.

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
* **Implementation (v1)**: Tauri's webview drag-drop events (`onDragDropEvent`: `enter/over/drop/leave` with `paths` and physical `position`) for file paths. Each path is classified (file, folder or drive) with `path_info`. Modifier keys for file operations come from `get_modifier_keys` at drop time.
* **URL drops (milestone M3b)**: Tauri's handler only delivers file paths (`CF_HDROP`); browsers offer links as `UniformResourceLocatorW` / `CFSTR_INETURLW`. M3b replaces the drop target with a custom `IDropTarget` registered on the WebView2 child HWNDs (revoke and re-register). It handles `CF_HDROP`, `CFSTR_INETURLW` and `CFSTR_SHELLIDLIST`, reads `grfKeyState`, and forwards a `drop` event to the UI.

### 6.4 Context menus

Native menus (Tauri `Menu` API, `popup()` at the cursor).

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

Sensor-specific items come from the sensor type's `menu()` (§8.1). Opening a menu hides any hint/popup.

### 6.5 Global hotkey and tray

* `hotkey` (default `Ctrl+Alt+Shift+I`) with `hotkeyAction`: `bringToFront` (default) | `toggleVisibility`.
* If registration fails (key already taken), Settings shows the error next to the field; the app MUST still start.
* Tray icon: left-click toggles bar visibility; right-click opens the tray menu (§4.1). It can be hidden (`showTrayIcon`, default true). If both the tray icon and the bar are hidden, the hotkey or a second launch (single instance → show the bar and open Settings) brings it back.

### 6.6 Sensor picker and creation flow

* Modal page in the Settings window (`/settings/new?zone=&index=&target=`).
* Sensor types grouped by **category**, sorted alphabetically by localised name; each item shows name and description. A search box filters by name or description.
* With a target sensor: an **Insert: Before / After** selector (default After) and the text "… the '{target name}' sensor".
* **Add** (or double-click) → `add_sensor` with defaults → immediately opens **Properties** for the new instance. **Cancel** in Properties for a *newly created* sensor removes it again. Sensors created by drop skip this step.
* Insertion point when not targeting a sensor: the zone under the right-click position (thirds). Left and center zones append at the end; the right zone inserts at its **start** (next to the center).

### 6.7 Properties dialog

* Route `/settings/sensor/<id>`. The form is generated from the sensor's `props` schema (§8.2); `hidden` fields are not shown.
* Changes **preview live** on the bar (debounced 150 ms). **Save** persists; **Cancel** restores the snapshot taken when the dialog opened.
* Validation per field type; invalid fields show inline errors and disable Save.
* Each field shows its description as helper text.

---

## 7. Popups

### 7.1 Hover hint

* Trigger: the pointer enters a sensor, `popupsEnabled` is true, no drag is in progress, and the sensor returns a non-empty hint.
* **Show delay** 600 ms (`popups.showDelayMs`); **hide delay** 100 ms after the pointer leaves (`popups.hideDelayMs`). Moving to another sensor within the hide delay swaps the content immediately, without a new show delay.
* While visible, the hint **updates live** when the sensor's data changes (CPU %, battery time…).
* Content (`HintContent`): `{ kind: "text", text }` (multi-line plain text; `\n` line breaks; tab-aligned "label: value" columns) or `{ kind: "component", component, props }` for rich hints (Date calendar).
* Visuals: rounded rectangle (modern radius 8 px, classic 6 px); background `popups.background` at `popups.opacity`; text `popups.textColor`; font `popups.font` / `popups.fontStyle` / `popups.fontSize` (9–15, default 12); optional shadow (`popups.shadow`); padding 6 px; minimum size 30×20. A **pointer triangle** 13×7 points at the sensor (downward when the bar is at the bottom, upward at the top).
* The hint is click-through. It closes when the pointer leaves the sensor, on any click, when a menu opens and when a drag starts.

### 7.2 Interactive popups

* Opened by a sensor click (Folder Menu, Start Menu, Eject USB). Popup kinds are registered by sensor types.
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

Flow: the bar calls `show_hint` / `show_popup` with the anchor and payload. Rust forwards the payload to the hint/popup window, which renders invisibly (opacity 0), measures itself and calls `popup_ready(w, h)`. Rust then sizes and positions the window (physical px) and shows it, without activation for hints and with focus for popups, using a 120 ms fade.

### 7.4 Notifications

* Windows toast notifications via `tauri-plugin-notification`; app name "Infodock", toast title = sensor title.
* v1 producers: Battery charge alert (§9.14), Eject results (§9.22), layout import summary.
* Sound and placement follow the Windows notification settings.

---

## 8. Sensor framework

### 8.1 Sensor type contract (TypeScript)

```ts
export type Zone = "left" | "center" | "right";
export type Category = "general" | "info" | "system" | "network" | "launchers";

export interface SensorType<P extends object, S extends object = {}> {
  type: string;                         // stable id, e.g. "cpu-graph"
  legacyTypes?: string[];               // legacy XML type ids handled by this type (§11.4)
  category: Category;
  name: MsgKey;                         // localised display name
  description: MsgKey;
  requires?: (keyof Capabilities)[];
  props: PropSchema<P>;                 // ordered fields: drives the form and validation
  defaults(ctx: DefaultsCtx): P;        // ctx: locale, volumes, CPU count; storage rules in §11.2
  topics?(p: P): TopicRequest[];        // system data needed (none for pure-UI sensors)
  view: Component<ViewProps<P, S>>;     // rendered in the bar slot (height = bar content height)
  hint?(c: SensorCtx<P, S>): HintContent | null;
  popup?: { kind: string; component: Component<PopupProps<P, S>> };  // click popup
  onClick?(c: SensorCtx<P, S>): void | Promise<void>;
  menu?(c: SensorCtx<P, S>): MenuItemSpec[];
  drop?: { accepts(d: DropPayload, c): boolean; onDrop(d: DropPayload, c): Promise<void> };
  initialState?(): S;                   // persisted runtime state (e.g. uptime record)
  inactive?(c: SensorCtx<P, S>): boolean; // drives inactive rendering (§8.5)
  fromLegacy?(raw: Record<string, string>, legacyType: string): Partial<P>;  // §11.4
}

export interface SensorCtx<P, S> {
  id: string; props: P; state: S; data: Record<string, Sample | undefined>;
  settings: Settings; locked: boolean; altKey: boolean;
  setState(patch: Partial<S>): void;  updateProps(patch: Partial<P>): void;
  ipc: Ipc;  t: (k: MsgKey, ...args: unknown[]) => string;  notify(title: string, body: string): void;
}
```

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
| `fontStyle` | Normal / Semibold / Bold / Italic / Semibold Italic / Bold Italic | string: `normal`, `semibold`, `bold`, `italic`, `semiboldItalic`, `boldItalic` | Semibold = CSS weight 600; fonts without a semibold face render bold |
| `drive` | dropdown from `list_volumes` (live) | `X:\` | option `hideOptical` |
| `netAdapter` | dropdown from `list_net_adapters` (live) + "Automatic" | `{ id, name }` | |
| `battery` | dropdown from `list_batteries` | index | |
| `hidden` | — | any | internal values (e.g. popup size) |

Field definition: `{ key, type, label, description?, default, machineDefault?, ...typeOptions, visibleIf?(p) }`.

`default` is normally a static value. A **machine-derived** field (`machineDefault: true`) instead gets its default from `defaults(ctx)`, e.g. the first fixed drive or the locale's clock format. §11.2 defines how this affects what is stored.

### 8.3 Reusable building blocks

**Two-line block** (`TwoLine.svelte`) — the core visual idiom: a small **label** line above a **value** line, both in the sensor font (modern: 10 px semibold; classic: Arial 9 px bold). In a 25 px bar the classic text baselines are y = 11 and y = 21. Modern: a two-row CSS grid centred vertically; line height = font size + 1.

**Meter** (`Meter.svelte`) — CPU, Memory, Swap and Disk meters:
```
[▮] LABEL
    value
```
* Bar: 4 px wide × (bar height − 6) px tall (19 px at 25 px), at x = 0, vertically centred.
* Bar background: sensor text colour at 31 % opacity.
* Fill: height = value × bar height, anchored at the bottom. Colour by `colorMode`: Standard → sensor text colour at **59 %**; Custom → the colour at 100 %; Dynamic → gradient(value) at 100 % (§8.4).
* Label at x = 8. Value right-aligned to x = 32 (classic geometry), i.e. a minimum column that fits "100%".

**Graph** (`Graph.svelte`, canvas) — CPU, Memory and Swap graphs:
* Plot area: `width` logical px (50–150, step 5, default 50) × (bar height − 4), at y = 2.
* **One sample per logical px column**, newest at the **right edge**, scrolling left. Each column is a bar from the bottom with height = value × plot height. History length = width.
* Layers: background (`bkColor` standard = black; `bkOpacity` default **0 %**) → grid (pattern of 5 px cells drawn on the canvas; `gridColor` standard = `#A0FF9B`; `gridOpacity` default **0 %**) → columns (`graphColor` default **Custom `#00FF00`**; `graphOpacity` default **55 %**, min 20 %). With Dynamic colour, each column takes the colour of its own value.
* Two-line block to the right of the plot (gap 5 px): label (e.g. "CPU") and value (e.g. "37%") right-aligned in a reserved column (fits "100%" or the sensor-specific reserve).
* DPR-aware canvas (backing store = logical size × devicePixelRatio), fully redrawn per sample (cheap at ≤ 150 columns).

**Dual graph** (`DualGraph.svelte`) — CPU dual, Network:
* The plot is split into two halves of (plotHeight − 1)/2 with a 1 px gap. The **top** series grows upward from the middle line. The **bottom** series grows **downward** from the middle (`flipBottom = true`, default) or upward from the bottom edge (`false`).
* Colour and opacity per series (`topColor`/`topOpacity`, `bottomColor`/`bottomOpacity`).
* Values are normalised to 0..1 per series by the owning sensor.

**Icon + two-line** (`IconLabel.svelte`) — Battery, Wi-Fi, Recycle Bin: an icon (≤ bar height − 2) followed by a two-line block after a 2–3 px gap.

**History** (`history.ts`): fixed-capacity ring buffer (`Float32Array`); resizing keeps the newest samples.

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

### 8.6 Formatting helpers (`format.ts`)

* **Bytes** (`formatBytes(n, digits = auto)`), base **1024**, units `TB, GB, MB, KB, bytes` (singular `1 byte`). Auto digits: `0` → "0"; `< 10` → 2 decimals; `< 100` → 1 decimal; otherwise 0. Plain bytes always use 0 decimals. Examples: `0 bytes`, `512 bytes`, `1.50 KB`, `12.3 MB`, `431 GB`.
* **Rates** (`formatRate`): same scaling, units `TB/s … KB/s, B/s`.
* **Durations**: compact `3d 04h 12m` (uptime); long `3 days, 4 hours, 12 minutes` with plural rules; battery `h:mm`.
* **Percent**: integer, `37%` (locale-specific spacing allowed).
* **Dates**: `Intl.DateTimeFormat`; ISO 8601 week numbers.

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

View: the image scaled to height = bar height − 1, keeping aspect ratio. Hint = popupText (none if empty). The first-run instance uses popupText "Infodock %v".

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
* **Year = none**: a single large line combining weekday and month, e.g. `Saturday, October 11`, `Sat, Oct 11`, `Saturday, 10/11`, `11`. Use `Intl.DateTimeFormat` parts so that ordering follows the locale.
* Re-render when the local date changes: a timer to the next local midnight, plus `WM_TIMECHANGE`, resume, and a 60 s safety tick.

**Hint (rich)**: when calendar ≠ none, a **month calendar**: a header row of short weekday names starting at the chosen week start, then 4–6 week rows with right-aligned day numbers in columns ≥ 29 px wide. Weekend columns (Sat, Sun) use `popups.strongTextColor`. **Today** is a filled rounded rect in the text colour (strong colour on weekends) with the number in the popup background colour. Below the calendar: `Week {isoWeek}, Day {dayOfYear}`. With calendar = none, only that line.

**Click**: open `settings.apps.calendar` if set (file path or URI, e.g. `outlookcal:`); otherwise `ms-settings:dateandtime`.

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
* **Chime**: when the minute changes and `minute % chime == 0`, play the bundled chime sound (`assets/sounds/chime.ogg`) via `<audio>`. No chime within 2 s after app start or resume.

**Hint**: the long date, e.g. `Saturday, October 11, 2026` (Intl `dateStyle: "full"`).
**Click**: `ms-settings:dateandtime`.

### 9.6 `uptime` — Uptime  (Info; `DockUpTime`)

| Key | XML key | Type | Default |
|---|---|---|---|
| text | `text` | text | "Uptime:" — prefix; an empty value means no prefix (no `emptyIsDefault`) |
| font / fontStyle / fontSize / shadow | as Text | | style bold, size 12 |

State (persisted via `set_sensor_state`): `longestSecs` (XML key `max-uptime`, format `d:h:m:s`; empty = none).

Topic `uptime`. Rust takes the boot time from `GetTickCount64()`; the UI counts locally between samples (resample every 60 s and on resume).

View: `{prefix} 3d 04h 12m`, updated at minute boundaries.
Hint: `3 days, 4 hours, 12 minutes`; when a longer record exists, a second line `Longest uptime: 12 days, 1 hour, 3 minutes`. The record updates whenever the current uptime exceeds it.

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

**Scaling** per series: `scale = max(configured max (if any), max of the visible history, 1 KB/s)`, recomputed every tick. The canvas redraws from raw history values, so rescaling is lossless.

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
**Drop**: files/folders → `move_to_recycle_bin` (IFileOperation with `FOFX_RECYCLEONDELETE`; confirmation per OS settings). While dragging over it, show the floating drop hint "Delete {n} items" (§9.19).

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

* **Icon**: the custom image; otherwise the shell icon of the target via `icon://` (`.lnk` shows its target's icon); for URLs, the built-in globe icon. Size: bar height − 4, centred.
* **Automatic title**: URL → the URL, middle-truncated to 150 characters (first 75 + `…` + last 75). File → file name without extension. Folder → folder name.
* **Hint**: the title if set; otherwise `Open '{title}'` (only `{title}` when clicks need Alt, §6.1).
* **Click**: URL → open with the default handler. Folder → open in Explorer. File → `ShellExecuteExW(verb = runAs ? "runas" : "open", file = target, params = args, dir = workDir)`.
* **Inactive** while the target does not exist (checked with `path_info` when properties change and every 60 s).
* Created by dropping a file or URL onto the bar (§6.3).

### 9.18 `disk` — Disk  (Launchers; `DockDisk`)

| Key | XML key | Type | Default |
|---|---|---|---|
| drive | `disk` | drive (optical/removable included) | machine-derived: first drive |
| title | `title` | text | "" → shell display name, e.g. `Local Disk (C:)` |
| image | `image` | file | "" → shell drive icon |
| color / opacity | `colorize` / `opacity` | | standard / 100 % |

Topic `volumes` (event-driven list). View: the drive icon only.
Hint: `Open '{display name}'` (or the title). Click: open the drive. Menu: **Eject** for removable and optical drives (`eject_volume`; on success the toast "{name} can be safely removed", on failure a toast with the reason, e.g. "in use by {app}"). Inactive while the drive is absent; an optical drive without media stays active, with the hint `{name} — no media`.

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
* **List**: 20 px rows with a 16 px shell icon and the name (truncated at the end). **Folders first**, then files; each group sorted case-insensitively with natural number order (`Intl.Collator(locale, { numeric: true, sensitivity: "base" })`). `.lnk` and `.url` extensions are hidden in names. `desktop.ini` is always skipped; hidden/system items are skipped unless `showHidden`. Hovered rows are highlighted (dark popup background: `#A9DFFF` at 40 %; light: `#00649E` at 55 %). An empty folder shows a centred "no items". Slow folders show a loading indicator; listing runs asynchronously in Rust.
* **Item click**: folder → navigate in place (the list slides left, 250 ms ease-out; Back slides right). File → open it (ShellExecute) and close the popup. **Alt+click** → reveal in Explorer.
* **Keyboard**: ↑/↓ select; Enter opens; → or Enter on a folder navigates into it; ← or Backspace goes back; Esc closes; typing jumps to the first name with that prefix.
* **Mouse wheel**: native scrolling.
* **Resize**: a grip on the popup's free vertical edge (width) and on the edge away from the bar (height). The new size is saved to popupWidth/popupHeight.
* **Placement**: as §7.3, but the popup's left edge aligns with the sensor's left edge. If that would overflow the monitor, its right edge aligns with the sensor's right edge instead.
* **Menu**: `Open folder`.
* **Drop onto the sensor**:
  * Files/folders follow the **Explorer convention**: same volume → move, other volume → copy (volumes from `path_info`). Ctrl forces copy, Shift forces move, Alt creates shortcuts (`.lnk`).
  * URLs → `create_url_shortcut`: a `.url` file named after the page title, or after the URL with the characters `:/\&+?*=.` replaced by `-`.
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
Hint: the mode name. Click: if `confirm`, a native dialog ("Do you really want to {lock|sign out of|…} this computer?"), then `power_action`:

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
Click → interactive popup, kind `eject-list`: one row per ejectable volume (removable USB drives, USB-attached disks, optical media) with icon, display name, letter, size and an **Eject** button. Footer button "Safely Remove Hardware…" opens the system dialog (`rundll32.exe shell32.dll,Control_RunDLL hotplug.dll`). Eject → `eject_volume` → toast with the result; the list refreshes on `volumes` events. With nothing to eject: "Nothing to eject".

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
| hotkey | accelerator | `Ctrl+Alt+Shift+I` | |
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
| floating | `{ x, y, width }` | null | |
| highlightOnHover | bool | false | |
| shadow | bool | false | |
| animations | auto / on / off | auto | auto follows the OS |

### 10.3 Appearance

| Key | Type | Default | Notes |
|---|---|---|---|
| theme | modern / classic | modern | |
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
* **Appearance**: theme, skin gallery with previews, colours, material.
* **Sensors**: global defaults, plus the **Layout manager** — three zone lists with drag reordering, edit, remove and add; an accessible alternative to dragging on the bar.
* **Help**: getting started, drag & drop rules (§6.3), Folder Menu tips (§9.19).
* **About**: version, MIT licence, third-party licences (crates, npm packages, icon sets), "Open logs folder".

---

## 11. Persistence

### 11.1 Locations

* Config: `%APPDATA%\app.infodock.desktop\config.json` (Tauri `app_config_dir`).
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
* Unknown `type`s are preserved and rendered as placeholders (§4.6), never deleted silently.
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

Each sensor type converts its own properties in `fromLegacy(raw, legacyType)`. An example file with its expected result is in Appendix B; it MUST be part of the importer's test suite.

### 11.5 First run

1. Create the default config with the layout of §11.2: **left** — CPU Graph, Memory Meter, Network Graph; **center** — Image (logo, popup "Infodock %v"); **right** — Date, Time.
2. Show a one-time welcome page in Settings: theme choice (Modern/Classic with previews), edge (top/bottom), autostart opt-in, an "Import layout…" button (JSON or legacy XML), and short getting-started tips.

---

## 12. Theming

### 12.1 Token model

All styling uses CSS custom properties on `:root`, set by `theme.ts` from theme defaults and user settings:

```
--bar-height, --bar-bg, --bar-bg-opacity, --bar-radius, --bar-pad (7px), --gap,
--sensor-fg, --sensor-font, --sensor-weight, --sensor-small (10px|9px), --sensor-medium (11px),
--sensor-large (15px), --popup-bg, --popup-opacity, --popup-fg, --popup-strong, --popup-font,
--popup-size, --popup-radius (8px|6px), --accent (OS accent), --inactive-opacity, --inactive-color
```

### 12.2 Modern theme (default)

* Bar: `#1C1C1F` at 80 % over **Acrylic** (DWM via `window-vibrancy`; Mica on Windows 11 when chosen); 1 px hairline `rgba(255,255,255,.08)` on the edge facing the screen center; height 28.
* Typography: Segoe UI Variable; small 10 px, medium 11 px, large 15 px; semibold labels; tabular numerals.
* Icons: the Infodock SVG set (Appendix C), monochrome, tinted with `--sensor-fg`.
* Popups: radius 8 px, `#202024` at 92 %, soft shadow, 12 px text.
* Follows the OS **light/dark** mode: in light mode (if the user has not customised colours) the bar is `#F3F3F3` at 80 % with sensor text `#1A1A1A`.
* Placer: 2 px line in the accent colour.

### 12.3 Classic theme

A compact, denser look:
* Height 25; Arial; label/value 9 px bold; day/hour 15 px bold; month/minutes 11 px.
* Popups: radius 6 px, 13×7 pointer triangle, all drawn in CSS.
* No material (opaque or plain-alpha background), so skins render predictably.
* The background is a **skin** (§12.4), tinted with `background.color` when `background.colorMode = custom`.
* **Tint algorithm** (skins, and icons with a custom colour): per pixel `L = (0.299R + 0.587G + 0.114B) / 255`; output RGB = `L × tintRGB`; alpha preserved. Implemented on an `OffscreenCanvas` and cached as an object URL keyed by `(src, tint)`. CSS-drawn skins apply the tint by using `background.color` as their base colour.
* Background opacity applies to the skin layer only, not to sensors.

### 12.4 Skins

**Built-in skins** (pure CSS in `themes/skins/*.css`, base colour = `background.color`):

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
* When the configured bar height (§5.1) differs from `height`, the bar part of the tile is scaled vertically to the bar height; the shadow band keeps its size. This is pre-rendered on the same canvas as the tint (§12.3). Sensors always use the full configured bar height.
* Validation on load: PNG/JPEG magic bytes, ≤ 1 MB per file, ≤ 512 px per side, only files named in `skin.json`. Invalid packs are listed in Settings with the reason.
* Settings ▸ Appearance: "Install skin pack…" (pick a folder or `.zip`; `install_skin`), "Open skins folder" (`shell_open` on `skinsDir`), "Remove" (`remove_skin`).
* A configured skin that is missing on this machine falls back to Gloss, with a one-line notice in Settings ▸ Appearance.

---

## 13. Localisation

* Infrastructure from day one: all user-visible strings go through `t(key, …args)`. Catalogs live in `src/i18n/<lang>.json`, with stable keys (e.g. `sensor.cpu.hint.activity`); `en.json` is the source.
* **v1 ships English only.** The language selector is hidden while only one catalog exists.
* A missing key in a future catalog falls back to English and is logged once.
* Plurals via `Intl.PluralRules` (catalog entries support `one/few/many/other`).
* Weekday and month names, date order and number formatting come from `Intl` using `settings.dateLocale` (default `system` = the OS regional format), never from catalogs.
* String externalisation is verified by a lint rule (no bare string literals in Svelte markup).

---

## 14. Non-functional requirements

### 14.1 Performance budgets (release build, first-run layout, 1 s graphs)

| Metric | Budget |
|---|---|
| Idle CPU (4-core laptop) | ≤ 0.5 % average over 60 s |
| Private memory, all processes incl. WebView2 | ≤ 100 MB (stretch 70 MB) |
| Cold start → bar visible | ≤ 1.5 s; warm ≤ 0.8 s |
| Sampler cost per tick | ≤ 2 ms total |
| Hint shown after its delay | ≤ 50 ms |
| Folder Menu popup, folder with 500 items | ≤ 200 ms to first paint |

Techniques: one sampler thread; PDH query handles opened once and reused; no WMI on hot paths (page-file sizes cached for 60 s); Channels instead of global events; only changed sensors re-render; canvas graphs; no CSS `backdrop-filter` (blur is done by DWM); hint/popup windows pre-created and reused; icon LRU cache.

### 14.2 Reliability

* The AppBar is removed on normal exit and on panic; Explorer restarts are handled (§5.2).
* Provider errors never crash the app. A failing provider sends `{ error }` samples; sensors show `n/a`, become inactive and show the error in their hint. The provider retries with backoff (1 s → 30 s).
* Resume, display, DPI and time changes are handled (§5.3).
* If a webview crashes, its route reloads automatically.

### 14.3 Security

* Tauri **capabilities** per window (`src-tauri/capabilities/<window>.json`), least privilege. Every window also gets the core permissions needed to listen for events.

  | Window | Commands (§4.5) and plugins |
  |---|---|
  | `bar` | config, data, system, shell, ui; `app_info`, `quit`; plugins: dialog (file pickers, confirmations), notification |
  | `hint` | `get_config`, `popup_ready` |
  | `popup` | `get_config`, `update_sensor` (popup size), `subscribe` / `unsubscribe`, `popup_ready`, `hide_popup`, `list_dir`, `known_folder`, `shell_open`, `shell_reveal`, `list_volumes`, `eject_volume`, `open_safely_remove`; plugin: notification (eject results) |
  | `settings` | config, system, app; `shell_open` / `shell_reveal` (logs and skins folders); plugin: dialog |
* Strict CSP: no remote scripts or content; `img-src 'self' icon: skin: data: blob:`.
* No `eval`. No shell-string execution: launching uses `ShellExecuteExW` with target and parameters as separate fields.
* `shell_open` validates URIs (`http`, `https`, `ftp`, `mailto`, `ms-settings`, and other registered schemes the user configured).
* No secrets in v1. Future mail sensors MUST store credentials in Windows Credential Manager (`keyring` crate), never in the config.

### 14.4 Accessibility and UX

* The Settings window is fully keyboard-accessible and respects OS text scaling and high-contrast mode (system colours, no transparency).
* The Layout manager provides reordering without dragging.
* Sensors expose `aria-label` = name + current value. The hotkey focuses the bar; Tab moves between sensors, Enter = click, Shift+F10 = context menu.

### 14.5 DPI and multi-monitor

* Per-monitor DPI v2 (Tauri default). Layout is in logical px; Rust converts with the target monitor's scale. Moving the bar to a monitor with another scale repeats the AppBar negotiation after `WM_DPICHANGED`.
* Icons are requested at physical size (`size × scale`, rounded to 16/20/24/32/48).

---

## 15. Project layout, build and packaging

### 15.1 Repository layout

```
infodock/
  SPEC.md  LICENSE (MIT)  README.md
  package.json  bun.lock  vite.config.ts  tsconfig.json  svelte.config.js
  src/                        UI (§4.3)
  src-tauri/
    Cargo.toml  tauri.conf.json  build.rs
    capabilities/{bar.json,hint.json,popup.json,settings.json}
    icons/                    app icons (Appendix C)
    src/                      Rust (§4.2)
  tests/fixtures/legacy/      legacy XML samples (Appendix B and more)
  docs/manual-test.md         manual test checklist (§16)
```

### 15.2 Commands

`bun install` · `bun run tauri dev` · `bun run tauri build` · `bun run test` (Vitest) · `cargo test` (in `src-tauri`) · `bun run lint` (eslint + svelte-check + `cargo clippy -D warnings` + `cargo fmt --check`).

### 15.3 Packaging

* NSIS installer, **per-user** (`installMode: "currentUser"`), WebView2 bootstrapper (`webviewInstallMode: downloadBootstrapper`), Start Menu shortcut, optional autostart.
* MSI MAY be built for managed environments.

### 15.4 Later

* **Code signing** (to avoid SmartScreen warnings) — after v1.
* **Auto-update** with `tauri-plugin-updater` and signed manifests on GitHub Releases — after v1.

---

## 16. Testing

| Level | What | How |
|---|---|---|
| Rust unit | CPU % from `GetSystemTimes` deltas; network rates (counter reset, first sample, resume); battery estimators (time-to-full smoothing, moving average); AppBar rect negotiation (pure geometry); popup placement (§7.3); config migrations; legacy XML import (Appendix B → expected JSON) | `cargo test` |
| Rust integration (Windows) | Providers return sane values on a real machine; `list_dir`, `resolve_link`, icon protocol, skin-pack validation | `cargo test -- --ignored` on a Windows runner |
| TS unit | Zone layout (§5.4); move index math; formatters (bytes, rates, durations, plurals); dynamic gradient; property schema validation and defaults; each sensor's `hint()` against sample fixtures (snapshot); each sensor's `fromLegacy`; Date/Time formats (all weekday/month/year combinations × several locales) | Vitest |
| Component | Meter/Graph/DualGraph rendering; PropertyForm generation for every sensor schema | Vitest + @testing-library/svelte |
| E2E smoke | App starts, bar visible, add/remove/move a sensor, settings survive a restart | `tauri-driver` + WebdriverIO on Windows (optional in CI) |
| Manual | Top/bottom; reserve space on/off; two monitors with different DPI; Explorer restart (`taskkill /f /im explorer.exe`); sleep/resume; full-screen video; Wi-Fi with location access off/on; laptop on and off AC; USB stick eject; drag & drop from Explorer and (M3b) browsers; legacy import of real files | `docs/manual-test.md`, on Windows 10 and 11 test hardware |

---

## 17. Milestones

| # | Scope | Exit criteria |
|---|---|---|
| **M0** Skeleton | Tauri app, single instance, tray, `bar` window with AppBar (top/bottom, monitor, reserve space, Explorer restart), config load/save with atomic writes, modern theme tokens, logging, i18n plumbing | Bar reserves space correctly on 1–2 monitors at 100/150 % DPI; survives an Explorer restart; clean exit restores the work area |
| **M1** Framework | Sensor registry, zones and layout, `hint` window and placement, context menus, Lock/Popups, drag reordering, Settings window shell, PropertyForm, SensorPicker, Layout manager, sampler and `subscribe` | Text/Image/Spacer can be added, configured, moved and removed; hints work; the layout persists |
| **M2** System sensors | Date (calendar hint), Time (chime), Uptime, CPU meter/graph/dual, Memory meter/graph, Swap meter/graph, Disk Meter, Network Graph | Values match Task Manager within ±3 % (CPU, memory) and ±1 % (disk); network rates match Resource Monitor; idle CPU budget met |
| **M3** Launchers & hardware | Battery, Wi-Fi, Recycle Bin, Shortcut, Disk, Folder Menu, Start Menu, Power, Eject USB; `icon://` protocol; `popup` window; file drops onto the bar and onto sensors | All §9.14–9.22 behaviours pass the manual checklist |
| **M3b** URL drag & drop | Custom `IDropTarget` for URLs with key state | Dragging a link from Edge/Chrome creates a web Shortcut, or a `.url` file in a Folder Menu |
| **M4** Classic, import, packaging | Classic theme, built-in skins, skin packs, tint; JSON export/import; legacy XML import; first-run welcome; autostart; installer | Appendix B imports exactly as specified; installer works per user without admin rights |
| **M5** Polish | Performance pass against §14.1, accessibility pass, Help/About, docs | All budgets met; no clippy warnings; all test suites green |

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
| R1 | Browser URL drag & drop is not delivered by Tauri's handler | Milestone M3b (custom `IDropTarget`). Fallback if it proves brittle: file drops only, plus "Paste URL" in Shortcut properties |
| R2 | WebView2 memory may exceed the 70 MB stretch goal | 100 MB is the hard budget; all windows share one browser process (Tauri default); keep DOMs small |
| R3 | Wi-Fi SSID requires Location permission on Windows 11 24H2+ | Graceful degradation (§9.15) |
| R4 | AppBar interaction with Tauri's window procedure | Message-only window for callbacks; integration tests on Windows 10 and 11 |
| R5 | CPU % may differ from Task Manager | Use PDH `\Processor Information(_Total)\% Processor Utility` (clamped to 100) for the total, as Task Manager does; per-core via the same counter set; user/kernel split from `GetSystemTimes` |
| R6 | Topmost transparent windows over full-screen games | `hideOnFullscreen` on by default |
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
| Chime | `chime.ogg`, ≤ 2 s, gentle, normalised to −16 LUFS | Time |
| Installer graphics | NSIS header/sidebar images | Installer |
