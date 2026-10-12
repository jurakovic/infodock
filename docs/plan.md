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

Exit: Text, Image and Spacer can be added, configured, moved and removed; hints work; the layout persists. Sensor types that arrive in M2 (the first-run CPU Graph, Memory Meter, Network Graph, Date, Time) render as placeholders until then (§4.6).

- [ ] **S1 M0 carry-overs**: a command that returns the current bar geometry (the first `bar-geometry` event fires before the bar page listens); the AppBar sink's work runs via a posted message instead of inside the window procedure, before hint/popup hiding hangs off it; `theme-changed` event so toggling OS transparency effects applies without a restart; clicks on the bar do not take focus (§5.3)
- [ ] **S2 Sensor framework**: `sensors/types.ts`, `registry.ts`, property schema with validation and defaults (§8.1–8.2), storage rules (only non-defaults; machine-derived props filled on load and stored, e.g. Time `use24h` in the first-run layout, §11.2), placeholders for unknown/unsupported types (§4.6)
- [ ] **S3 Layout and rendering**: `layout.ts` (§5.4, pure and tested), zone/slot components measured with `ResizeObserver`, Text/Image/Spacer views (§9.1–9.3), browser mock backend for screenshots
- [ ] **S4 Sensor commands**: `add_sensor`, `update_sensor`, `move_sensor`, `remove_sensor`, `set_sensor_state` (§4.5), move-index math (§6.2) with tests
- [ ] **S5 Menus, Lock, Popups**: native bar and sensor context menus (§6.4), remove confirmation, Lock and `clicksWhenLocked` (§6.1)
- [ ] **S6 Hint window**: pre-created non-activating, click-through `hint` window; `show_hint`/`update_hint`/`hide_hint`/`popup_ready`; placement (§7.3, pure and tested in Rust); show/hide delays, live updates and pointer triangle (§7.1)
- [ ] **S7 Drag reordering**: 4 px threshold, placer, zone by thirds, Esc cancels, hints suppressed (§6.2)
- [ ] **S8 Settings**: Settings shell (§10.7), SensorPicker (§6.6), PropertyForm with live preview and Cancel (§6.7), Layout manager, Sensors and Popups pages
- [ ] **S9 Sampler**: scheduler thread, topics, `subscribe`/`unsubscribe` over Channels at the minimum requested interval (§4.4); `uptime` provider as the first topic
- [ ] **S10 Floating mode and animation**: floating mode (§5.1, D3), rise animation and hover highlight (§5.5)
- [ ] **S11 Close-out**: clean-context review, fixes, `docs/manual-test.md` M1 section, CI green

## M2 — System sensors

Date (calendar hint), Time (chime), Uptime, CPU meter/graph/dual, Memory meter/graph, Swap meter/graph, Disk Meter, Network Graph; providers and Appendix A sources; `format.ts`, `color.ts`, Meter/Graph/DualGraph; resume and time-change events.

## M3 — Launchers & hardware

Battery, Wi-Fi, Recycle Bin, Shortcut, Disk, Folder Menu, Start Menu, Power, Eject USB; `icon://` protocol; `popup` window; file drops onto the bar and onto sensors; notifications.

## M3b — URL drag & drop

Custom `IDropTarget` on the WebView2 child windows (§6.3).

## M4 — Classic, import, packaging

Classic theme, built-in skins, skin packs, tint (§12.3–12.4); JSON export/import (§11.3); legacy XML import (§11.4, Appendix B); first-run welcome (§11.5); autostart; installer polish.

## M5 — Polish

Performance pass (§14.1), accessibility (§14.4), Help/About, final artwork (Appendix C), docs.

## Decision log

| # | Date | Decision |
|---|---|---|
| D1 | 2026-10-11 | Toolchain: Tauri 2.12, Svelte 5.57, Vite 8, Vitest 5, TypeScript 6.0 (typescript-eslint supports < 6.1, so not TS 7), Bun 1.4, Rust edition 2024, `windows` 0.62 (same as Tauri). |
| D2 | 2026-10-11 | A minimal Settings window (General + Bar) moves into M0 so edge, monitor and height changes can be tested; M1 builds the full Settings shell on top of it. |
| D3 | 2026-10-11 | Floating mode moves to M1: it needs background dragging, which is M1 interaction work. |
| D4 | open (M4) | Legacy import split between Rust (`config/legacy.rs`) and per-sensor `fromLegacy` in TS (§8.1 vs §4.2/§16). Proposal: Rust parses XML and the shared encodings into raw maps; TS sensor types map their own properties. |
| D5 | 2026-10-11 | Config: §10.2 bar settings live under `settings.bar`; theme-dependent settings are `null` (= theme default, table in `ThemeDefaults`, exported to `src/lib/bindings/themeDefaults.json`). `update_settings` rejects unknown keys. Spec §10.2 clarified. |
| D6 | 2026-10-11 | ts-rs without its `format` feature: that feature pulls in `psm`, whose assembly cannot be built when cross-checking the Windows target from Linux. |
| D7 | 2026-10-11 | AppBar registered for the bar window itself, which is subclassed (`SetWindowSubclass`) for the callback and broadcasts; the spec's message-only window cannot receive broadcasts. Spec §4.2/§4.6/§5.2 updated. Until M1, floating mode behaves like a docked bar without reservation. |
| D8 | 2026-10-11 | App-window module is `windowing/` (spec said `windows/`, which clashes with the `windows` crate). `settings.bar.monitor` stores a monitor key `<device>\|<name>\|<w>x<h>` (§10.2). |
| D9 | 2026-10-11 | A changed hotkey is registered before the setting is saved; failure returns error code `hotkey` and saves nothing. A hotkey taken at start-up is logged and shown in Settings ▸ General; the app still starts (§6.5). |
| D10 | 2026-10-11 | Hiding the bar (tray, hotkey) also releases its reserved space; it is runtime state, not a setting. |
| D11 | 2026-10-12 | M0 review fixes: Alt+F4/system Close blocked on the bar, any other close request quits cleanly; the AppBar subclass enforces visibility and `WS_EX_TOOLWINDOW` against tao re-applying its state; geometry pinned during our own moves (DPI change while crossing monitors); `ABM_QUERYPOS` proposes the whole monitor; z-order `normal` uses `HWND_TOP` when not topmost; `fullscreen` reset on `TaskbarCreated`; AppBar callback allowed through UIPI; display-config query retried; config listeners run without the store lock; second launch shows the bar without stealing focus; `WM_NCACTIVATE` forwarded as active so DWM keeps the backdrop on the unfocused bar (to verify on hardware). |
| D12 | 2026-10-12 | CI also publishes a portable build: the release `infodock.exe` as the `infodock-windows-portable` artifact. It uses the same %APPDATA% / %LOCALAPPDATA% folders as the installed app; a self-contained "data next to the exe" mode is not planned. Spec §15.3 updated. |
