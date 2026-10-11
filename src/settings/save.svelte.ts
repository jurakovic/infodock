/** Saves settings patches from the Settings window and keeps the last error for display. */
import type { CmdError } from "../lib/bindings/CmdError";
import type { Settings } from "../lib/bindings/Settings";
import { ipc, toCmdError, type DeepPartial } from "../lib/ipc";

export const saveStatus = $state<{ error: CmdError | null }>({ error: null });

/** Applies `patch`; returns the error (also kept in `saveStatus`) or null. */
export async function save(patch: DeepPartial<Settings>): Promise<CmdError | null> {
  try {
    await ipc.updateSettings(patch);
    saveStatus.error = null;
    return null;
  } catch (e) {
    const error = toCmdError(e);
    saveStatus.error = error;
    return error;
  }
}
