# Implementation plan

Milestones come from SPEC §17. Each milestone is split into slices; a slice is one or a few commits that pass the full gate (see `CLAUDE.md`). A milestone ends with green Windows CI, an installer artifact, and a manual check against `docs/manual-test.md`.

Legend: `[x]` done · `[ ]` open · `[~]` partly done (see note)

## M0 — Skeleton

Exit: the bar reserves space correctly on 1–2 monitors at 100/150 % DPI, survives an Explorer restart, and a clean exit restores the work area.

- [x] **S0 Scaffold**: Tauri 2 + Svelte 5 + Vite + Bun, lint/test scripts, CI (Linux checks; Windows clippy, tests, NSIS artifact), `CLAUDE.md`, this plan, SessionStart hook
- [x] **S1 Config**: typed config (§10, §11.2), theme-dependent defaults, atomic debounced store with `.bak` and broken-file recovery, migration framework, first-run layout (§11.5), `get_config` / `update_settings` + `config-changed`, ts-rs bindings
- [x] **S2 Logging, i18n, theme**: rolling logs (§11.1), panic hook, `t()` with plurals (TS) + Rust lookup, theme tokens (§12.1, modern), reactive config store in the UI
- [ ] **S3 Platform + AppBar**: platform traits (§4.6), Windows monitors (device name, friendly name, scale), AppBar negotiation (§5.2), z-order and full-screen (§5.3), Explorer restart, `ABM_REMOVE` on exit and panic; stub platform elsewhere
- [ ] **S4 App shell**: bar window placement and material, tray (§4.1, §6.5), single instance, global hotkey, minimal Settings window (General + Bar pages) to exercise edge/monitor/height changes, `docs/manual-test.md` M0 section
- [ ] **S5 Close-out**: clean-context review of the Windows code, fixes, CI green, report

## M1 — Framework

Sensor registry and types (§8.1–8.2), zones and layout (§5.4), hint window and placement (§7.1, §7.3), context menus (§6.4), Lock/Popups, drag reordering (§6.2), floating mode (§5.1), rise animation and hover highlight (§5.5), Settings shell, PropertyForm, SensorPicker, Layout manager, sampler and `subscribe` (§4.4), sensor CRUD commands, Text/Image/Spacer sensors, browser mock backend for screenshots. On load, fill machine-derived props that are missing (e.g. Time `use24h` in the first-run layout, §11.2) and store them.

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
