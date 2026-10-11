/** Typed wrappers for Rust commands and events (SPEC §4.5). Types come from ./bindings (ts-rs). */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AppInfo } from "./bindings/AppInfo";
import type { BarGeometry } from "./bindings/BarGeometry";
import type { Capabilities } from "./bindings/Capabilities";
import type { CmdError } from "./bindings/CmdError";
import type { Config } from "./bindings/Config";
import type { HotkeyStatus } from "./bindings/HotkeyStatus";
import type { MonitorInfo } from "./bindings/MonitorInfo";
import type { Settings } from "./bindings/Settings";

/** Every key optional at every depth; `null` resets a theme-dependent setting to its default. */
export type DeepPartial<T> = T extends readonly unknown[]
  ? T
  : T extends object
    ? { [K in keyof T]?: DeepPartial<T[K]> }
    : T;

export const EVENT_CONFIG_CHANGED = "config-changed";
export const EVENT_BAR_GEOMETRY = "bar-geometry";
export const EVENT_SETTINGS_NAVIGATE = "settings-navigate";

export function isCmdError(e: unknown): e is CmdError {
  return typeof e === "object" && e !== null && "code" in e && "message" in e;
}

/** Normalises anything thrown by `invoke` into a CmdError. */
export function toCmdError(e: unknown): CmdError {
  return isCmdError(e) ? e : { code: "unknown", message: String(e) };
}

export const ipc = {
  // config
  getConfig: () => invoke<Config>("get_config"),
  updateSettings: (patch: DeepPartial<Settings>) => invoke<Config>("update_settings", { patch }),
  // system
  capabilities: () => invoke<Capabilities>("capabilities"),
  listMonitors: () => invoke<MonitorInfo[]>("list_monitors"),
  // ui
  openSettings: (route: string) => invoke<void>("open_settings", { route }),
  // app
  appInfo: () => invoke<AppInfo>("app_info"),
  hotkeyStatus: () => invoke<HotkeyStatus>("hotkey_status"),
  quit: () => invoke<void>("quit"),
  // events
  onConfigChanged: (cb: (config: Config) => void): Promise<UnlistenFn> =>
    listen<Config>(EVENT_CONFIG_CHANGED, (e) => cb(e.payload)),
  onBarGeometry: (cb: (geometry: BarGeometry) => void): Promise<UnlistenFn> =>
    listen<BarGeometry>(EVENT_BAR_GEOMETRY, (e) => cb(e.payload)),
  onSettingsNavigate: (cb: (route: string) => void): Promise<UnlistenFn> =>
    listen<string>(EVENT_SETTINGS_NAVIGATE, (e) => cb(e.payload)),
};
