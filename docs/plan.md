# Implementation plan

Milestones come from SPEC §17. Each milestone is split into slices; a slice is one or a few commits that pass the full gate (see `CLAUDE.md`). A milestone ends with green Windows CI, an installer artifact, and a manual check against `docs/manual-test.md`.

Legend: `[x]` done · `[ ]` open · `[~]` partly done (see note)

## M0 — Skeleton

Exit: the bar reserves space correctly on 1–2 monitors at 100/150 % DPI, survives an Explorer restart, and a clean exit restores the work area.

- [x] **S0 Scaffold**: Tauri 2 + Svelte 5 + Vite + Bun, lint/test scripts, CI (Linux checks; Windows clippy, tests, NSIS artifact), `CLAUDE.md`, this plan, SessionStart hook
- [x] **S1 Config**: typed config (§10, §11.2), theme-dependent defaults, atomic debounced store with `.bak` and broken-file recovery, migration framework, first-run layout (§11.5), `get_config` / `update_settings` + `config-changed`, ts-rs bindings
- [x] **S2 Logging, i18n, theme**: rolling logs (§11.1), panic hook, `t()` with plurals (TS) + Rust lookup, theme tokens (§12.1, modern), reactive config store in the UI
- [x] **S3 Platform + AppBar**: platform traits (§4.6), Windows monitors (device name, friendly name, scale), AppBar negotiation (§5.2), z-order and full-screen (§5.3), Explorer restart, `ABM_REMOVE` on exit and panic; stub platform elsewhere
- [x] **S4 App shell**: bar window placement and material, tray (§4.1, §6.5), single instance, global hotkey, minimal Settings window (General + Bar pages) to exercise edge/monitor/height changes, `docs/manual-test.md` M0 section
- [x] **S5 Close-out**: clean-context review of the Windows code, fixes, CI green, report. Remaining: manual Windows check (`docs/manual-test.md` M0)

## M1 — Framework

Exit: Text, Image and Spacer can be added, configured, moved and removed; hints work; the layout persists; the memory budget (§14.1: ≤ 30 MB with Settings closed) holds in CI. Sensor types that arrive in M2 (the first-run CPU Graph, Memory Meter, Network Graph, Date, Time) render as placeholders until then (§4.6).

Since D13 the bar is native (Win32 + Direct2D, bar thread); since D16 Settings uses Slint and Tauri is gone; since D17 the tray is native too. The M0 carry-overs are folded in: the geometry command is no longer needed (no bar webview), the bar thread's work already runs from posted messages, clicks not taking focus is `WS_EX_NOACTIVATE`, and the OS appearance change lands in S1/S2.

- [x] **S1 Native bar window**: bar thread (own message loop; OLE is initialised when drag & drop arrives in M3) and `BarController`; native bar window (`WS_POPUP`, tool window, `WS_EX_NOACTIVATE`) replacing the webview bar; AppBar moved from the subclass into our own window procedure; per-pixel alpha + material (window-vibrancy) as in M0; Direct2D software canvas drawing background and hairline; material re-applied when OS transparency changes; webview `bar` route, `src/bar` and the bar capability removed; CI memory check (report first, enforce from S12). Closing Settings no longer quits the app (it is the only Tauri window). To verify on hardware: `docs/manual-test.md` M1
- [x] **S1b Slint Settings and tray (D16)**: Tauri, WebView2 and the web toolchain removed; Slint event loop on the main thread with the Settings window (General and Bar pages, same behaviour as before); `global-hotkey` directly; own single-instance lock and data folders in the platform layer; Windows resources (icon, manifest, version) via `embed-resource`; NSIS installer via `cargo-packager`; offscreen rendering of the Settings pages to PNG (test, CI artifact). Tray: native icon and menu on the bar thread (D17). Hotkey recorded by pressing the keys (D18)
- [x] **S2 Canvas, text, theme**: canvas API (§4.6) with the Direct2D/DirectWrite/WIC implementation and a recording canvas (shared by tests on every OS and the stub); bitmaps (WIC, `resvg`, tint); the rest of the resolved `Theme` (§12.1: accent colour, inactive style); a `--ignored` test that renders the bar and a canvas specimen to PNG, uploaded by CI with the Settings screenshots (`infodock-screenshots`). D19
- [x] **S3 Sensor framework**: `SensorType`/`Sensor` traits (§8.1), schema, validation, defaults and storage rules (§8.2, §11.2; machine-derived props filled on load and stored), registry, placeholders for unknown/unsupported types (§4.6), schemas readable by Settings (§4.5: `Registry::infos`). D20
- [ ] **S4 Layout and first sensors**: `bar/layout.rs` (§5.4), measurement and redraw scheduling, wake-up timers, Text/Image/Spacer (§9.1–9.3)
- [ ] **S5 Sensor operations**: add, update, preview, move and remove sensors (§4.5), move-index math (§6.2)
- [ ] **S6 Menus, Lock, Popups**: native context menus (§6.4), remove confirmation (TaskDialog), Lock and `clicksWhenLocked` (§6.1), opening Settings routes from the bar. The menu code (`platform/windows/menu.rs`, `MenuEntry`) exists since D17
- [ ] **S7 Hint window**: layered hint window, placement (§7.3), delays, live updates, pointer triangle and shadow (§7.1)
- [ ] **S8 Drag reordering**: pointer capture, 4 px threshold, placer, zone by thirds, Esc via `GetAsyncKeyState`, hints suppressed (§6.2)
- [ ] **S9 Settings**: Settings shell (§10.7), SensorPicker (§6.6), PropertyForm generated from the sensor schemas with live preview and Cancel (§6.7), Layout manager, Sensors and Popups pages
- [ ] **S10 Sampler**: scheduler thread, topics, subscriptions at the minimum requested interval, delivery to the bar thread (§4.4); `uptime` provider as the first topic
- [ ] **S11 Floating mode and animation**: floating mode (§5.1, D3), rise animation and hover highlight (§5.5)
- [ ] **S12 Close-out**: clean-context review, fixes, `docs/manual-test.md` M1 section, memory budget enforced in CI, CI green

## M2 — System sensors

Date (calendar hint), Time (chime), Uptime, CPU meter/graph/dual, Memory meter/graph, Swap meter/graph, Disk Meter, Network Graph; providers and Appendix A sources; `sensors/format.rs`, `sensors/color.rs`, meter/graph/dual-graph building blocks (§8.3); platform `Locale` (§8.6); resume and time-change events.

## M3 — Launchers & hardware

Battery, Wi-Fi, Recycle Bin, Shortcut, Disk, Folder Menu, Start Menu, Power, Eject USB; shell icons; `popup` window; drag & drop of files, folders, drives and URLs through the bar's own `IDropTarget` (§6.3; the former M3b, merged by D13); notifications.

## M4 — Classic, import, packaging

Classic theme, built-in skins, skin packs, tint (§12.3–12.4); `colorScheme` system/light/dark for Settings and the modern bar (§10.3); JSON export/import (§11.3); legacy XML import (§11.4, Appendix B); first-run welcome (§11.5); autostart; installer polish.

## M5 — Polish

Performance pass (§14.1), accessibility (§14.4), Help/About, final artwork (Appendix C), docs.

## Decision log

| # | Date | Decision |
|---|---|---|
| D1 | 2026-10-11 | Toolchain: Tauri 2.12, Svelte 5.57, Vite 8, Vitest 5, TypeScript 6.0 (typescript-eslint supports < 6.1, so not TS 7), Bun 1.4, Rust edition 2024, `windows` 0.62 (same as Tauri). |
| D2 | 2026-10-11 | A minimal Settings window (General + Bar) moves into M0 so edge, monitor and height changes can be tested; M1 builds the full Settings shell on top of it. |
| D3 | 2026-10-11 | Floating mode moves to M1: it needs background dragging, which is M1 interaction work. |
| D4 | 2026-10-12 | Legacy import: resolved by D13. `config/legacy.rs` parses the XML and shared encodings; each Rust sensor type maps its own properties in `from_legacy` (§11.4). |
| D5 | 2026-10-11 | Config: §10.2 bar settings live under `settings.bar`; theme-dependent settings are `null` (= theme default, table in `ThemeDefaults`, exported to `src/lib/bindings/themeDefaults.json`). `update_settings` rejects unknown keys. Spec §10.2 clarified. |
| D6 | 2026-10-11 | ts-rs without its `format` feature: that feature pulls in `psm`, whose assembly cannot be built when cross-checking the Windows target from Linux. |
| D7 | 2026-10-11 | AppBar registered for the bar window itself, which is subclassed (`SetWindowSubclass`) for the callback and broadcasts; the spec's message-only window cannot receive broadcasts. Spec §4.2/§4.6/§5.2 updated. Until M1, floating mode behaves like a docked bar without reservation. Superseded in part by D13: Infodock creates the bar window, so no subclass is needed. |
| D8 | 2026-10-11 | App-window module is `windowing/` (spec said `windows/`, which clashes with the `windows` crate). `settings.bar.monitor` stores a monitor key `<device>\|<name>\|<w>x<h>` (§10.2). |
| D9 | 2026-10-11 | A changed hotkey is registered before the setting is saved; failure returns error code `hotkey` and saves nothing. A hotkey taken at start-up is logged and shown in Settings ▸ General; the app still starts (§6.5). |
| D10 | 2026-10-11 | Hiding the bar (tray, hotkey) also releases its reserved space; it is runtime state, not a setting. |
| D11 | 2026-10-12 | M0 review fixes: Alt+F4/system Close blocked on the bar, any other close request quits cleanly; the AppBar subclass enforces visibility and `WS_EX_TOOLWINDOW` against tao re-applying its state; geometry pinned during our own moves (DPI change while crossing monitors); `ABM_QUERYPOS` proposes the whole monitor; z-order `normal` uses `HWND_TOP` when not topmost; `fullscreen` reset on `TaskbarCreated`; AppBar callback allowed through UIPI; display-config query retried; config listeners run without the store lock; second launch shows the bar without stealing focus; `WM_NCACTIVATE` forwarded as active so DWM keeps the backdrop on the unfocused bar (to verify on hardware). |
| D12 | 2026-10-12 | CI also publishes a portable build: the release `infodock.exe` as the `infodock-windows-portable` artifact. It uses the same %APPDATA% / %LOCALAPPDATA% folders as the installed app; a self-contained "data next to the exe" mode is not planned. Spec §15.3 updated. |
| D13 | 2026-10-12 | **Native bar.** The M0 build with an empty webview bar used about 360 MB (infodock.exe + 6 msedgewebview2.exe); Settings added about 65 MB; comparable native bars use about 30 MB. The bar, hint and popup become native windows on a dedicated bar thread, drawn with Direct2D (software) + DirectWrite; sensors are Rust modules (`sensors/`); WebView2 hosts only Settings, created on demand and destroyed on close. Budget ≤ 30 MB with Settings closed (stretch 20 MB), measured in CI. M1 resliced; M3b merged into M3 (the bar's own `IDropTarget` handles URLs). Spec draft 4: §2–§9 implementation notes, §12–§17, §19. |
| D14 | 2026-10-12 | CI runs only when build inputs change (`paths` in `ci.yml`); docs-only commits skip it. |
| D15 | 2026-10-12 | First measurement of the native bar (M1 S1, CI Windows runner, 1024×768, empty bar): idle 6.9 MB in 1 process; Settings open 72 MB in 7 processes; 10 s after closing Settings 8.7 MB in 1 process. WebView2 exits with Settings, so §19 R7 is resolved. |
| D16 | 2026-10-12 | **Slint for Settings and the tray; Tauri removed.** With the bar native, Tauri only hosted the Settings webview (~65 MB while open, plus the Bun/Vite/Svelte toolchain). Settings is now a Slint window (Fluent style, software renderer, AccessKit) created when opened and dropped when closed; the tray is Slint's `SystemTrayIcon`. Replacements for Tauri: `global-hotkey` (the crate its plugin wrapped), a named-mutex single-instance lock with an activation message to the bar window, known-folder paths (same folders as before), `embed-resource` for icon/manifest/version, `cargo-packager` for the NSIS installer (same templates as Tauri's bundler). Strings reach Slint through an `I18n` global backed by `i18n::t`. Licence: Slint Royalty-free 2.0, attribution via the badge in the README until the About page (M5) shows `AboutSlint`. ts-rs dropped (no TypeScript left). The crate moved from `src-tauri/` to the repository root. Spec draft 5. Measured in CI (same runner as D15): idle 6.7 MB, Settings open 9.7 MB, after closing 8.1 MB, always one process (WebView2 Settings: 72 MB in 7 processes). |
| D17 | 2026-10-12 | **Native tray.** Slint 1.18's `SystemTrayIcon` draws no check marks on Windows (its menu code ignores `checked`), so *Lock* and *Popups* showed no state. The tray icon is now `Shell_NotifyIconW` registered for the bar window (callback in the bar window procedure, re-added on `TaskbarCreated`) and its menu a `TrackPopupMenuEx` menu built from the current state each time it opens, both on the bar thread; the same `MenuEntry` menus serve the bar's context menus in S6. The main thread keeps Settings and the hotkey. A light/dark/system switch (`colorScheme`, §10.3) is recorded for M4 Appearance. Spec §3, §4.1–4.6, §10.3, §12.2, §13. |
| D18 | 2026-10-12 | **Hotkey recorder.** The hotkey was a text field applied with Enter, which was easy to miss and could not express keys by pressing them. Settings now records it: select the button, press the combination; Esc cancels. Keys come from winit as physical keys (Slint key events carry layout-dependent text, and Ctrl+Alt is AltGr on many layouts), filtered with `on_winit_window_event` while recording. Ctrl, Alt or Win is required except for F1–F24. Errors are sentences from the catalog; *taken* names the combination (Windows cannot say which app holds it). Recording the saved combination again retries a hotkey that failed at start-up. Spec §6.5, §10.1. |
| D19 | 2026-10-12 | **Canvas API (M1 S2).** Shapes (`Rect`, `RoundedRect`, `Polygon`) are filled or stroked with a `Brush` (solid or linear gradient); clips and opacity layers are pushed and popped. Text is laid out by a `TextEngine` separate from the canvas, because sensors measure outside drawing; a `TextLayout` holds its size, first baseline and the platform's layout object. Bitmaps are premultiplied RGBA in an `Arc`, so worker threads can decode them; formats are recognised by their first bytes before WIC sees them; the Direct2D renderer caches each bitmap's upload while it lives. `resvg` is a direct dependency at the version Slint already links (no size cost). The recording canvas and fixed-metrics engine live in `platform/recording.rs` for tests on every OS. `render_offscreen` renders to a `Bitmap` (CI screenshots now, skin previews in M4). The accent colour comes from `HKCU\…\DWM\AccentColor`. The bar can only be rendered on Windows, so its screenshots come from CI. |
| D20 | 2026-10-12 | **Sensor framework (M1 S3).** The `Sensor` trait grows with the slices that use it: S3 has `measure` (now `&mut self`, so sensors build and keep their text layouts and bitmaps there), `draw`, `hint`, `inactive` and `accessible_value`; topics/samples arrive with the sampler (S10), wake-ups with the first timed sensors, clicks/menus/drops with S5–S8 and M3, `from_legacy` with the legacy import (M4). Schemas are static `PropField` lists with `Lit` defaults (const-constructible). One normalisation (`schema::check`) serves load and update: wrong types are invalid (ignored and logged on load, rejected on update), out-of-range numbers are clamped. Props keys unknown to a schema are kept on save, like unknown types. Unknown and unsupported types become `Placeholder` sensors: an outlined, dimmed label with an explanatory hint. Machine-derived props are filled once at start-up, before the bar starts. The registry is empty until S4 adds Text, Image and Spacer. |
