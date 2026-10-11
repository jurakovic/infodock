/** Reactive mirror of the Rust config (SPEC §4.3 lib/config.ts), kept fresh by config-changed. */
import type { Config } from "./bindings/Config";
import { ipc } from "./ipc";

class ConfigMirror {
  current = $state<Config | null>(null);
  #started = false;

  /** Subscribes first, then fetches, so no change between the two is lost. */
  async init(): Promise<void> {
    if (this.#started) return;
    this.#started = true;
    let changedMeanwhile = false;
    await ipc.onConfigChanged((c) => {
      changedMeanwhile = true;
      this.current = c;
    });
    const initial = await ipc.getConfig();
    if (!changedMeanwhile) this.current = initial;
  }
}

export const config = new ConfigMirror();
