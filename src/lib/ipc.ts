/** Typed wrappers for Rust commands and events (SPEC §4.5). Types come from ./bindings (ts-rs). */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { CmdError } from "./bindings/CmdError";
import type { Config } from "./bindings/Config";
import type { Settings } from "./bindings/Settings";

/** Every key optional at every depth; `null` resets a theme-dependent setting to its default. */
export type DeepPartial<T> = T extends readonly unknown[]
  ? T
  : T extends object
    ? { [K in keyof T]?: DeepPartial<T[K]> }
    : T;

export const EVENT_CONFIG_CHANGED = "config-changed";

export function isCmdError(e: unknown): e is CmdError {
  return typeof e === "object" && e !== null && "code" in e && "message" in e;
}

export const ipc = {
  getConfig: () => invoke<Config>("get_config"),
  updateSettings: (patch: DeepPartial<Settings>) => invoke<Config>("update_settings", { patch }),
  onConfigChanged: (cb: (config: Config) => void): Promise<UnlistenFn> =>
    listen<Config>(EVENT_CONFIG_CHANGED, (e) => cb(e.payload)),
};
