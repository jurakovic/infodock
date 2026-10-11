<script lang="ts">
  import { onMount } from "svelte";
  import type { Settings } from "../../lib/bindings/Settings";
  import { t } from "../../lib/i18n";
  import { ipc } from "../../lib/ipc";
  import Row from "../Row.svelte";
  import { save } from "../save.svelte";

  let { settings }: { settings: Settings } = $props();

  // Follows the saved hotkey; typing overrides it until the next save.
  let hotkeyDraft = $derived(settings.hotkey);
  let hotkeyError = $state<string | null>(null);

  onMount(async () => {
    const status = await ipc.hotkeyStatus();
    if (status.error) hotkeyError = status.error;
  });

  async function applyHotkey() {
    const value = hotkeyDraft.trim();
    if (value === settings.hotkey && !hotkeyError) return;
    const error = await save({ hotkey: value });
    hotkeyError = error?.code === "hotkey" ? error.message : null;
  }
</script>

<Row
  id="hotkey"
  label={t("settings.general.hotkey")}
  help={t("settings.general.hotkey.help")}
  error={hotkeyError ? t("settings.general.hotkey.error", { error: hotkeyError }) : null}
>
  <input
    id="hotkey"
    type="text"
    spellcheck="false"
    autocomplete="off"
    bind:value={hotkeyDraft}
    onkeydown={(e) => e.key === "Enter" && applyHotkey()}
    onblur={applyHotkey}
    aria-describedby="hotkey-help"
  />
</Row>

<Row id="hotkey-action" label={t("settings.general.hotkeyAction")}>
  <select
    id="hotkey-action"
    value={settings.hotkeyAction}
    onchange={(e) => save({ hotkeyAction: e.currentTarget.value as Settings["hotkeyAction"] })}
  >
    <option value="bringToFront">{t("settings.general.hotkeyAction.bringToFront")}</option>
    <option value="toggleVisibility">{t("settings.general.hotkeyAction.toggleVisibility")}</option>
  </select>
</Row>

<Row id="tray" label={t("settings.general.showTrayIcon")}>
  <input id="tray" type="checkbox" checked={settings.showTrayIcon} onchange={(e) => save({ showTrayIcon: e.currentTarget.checked })} />
</Row>

<Row id="locked" label={t("settings.general.locked")} help={t("settings.general.locked.help")}>
  <input id="locked" type="checkbox" checked={settings.locked} onchange={(e) => save({ locked: e.currentTarget.checked })} />
</Row>

<Row id="popups" label={t("settings.general.popupsEnabled")}>
  <input id="popups" type="checkbox" checked={settings.popupsEnabled} onchange={(e) => save({ popupsEnabled: e.currentTarget.checked })} />
</Row>

<Row id="clicks" label={t("settings.general.clicksWhenLocked")}>
  <select
    id="clicks"
    value={settings.clicksWhenLocked}
    onchange={(e) => save({ clicksWhenLocked: e.currentTarget.value as Settings["clicksWhenLocked"] })}
  >
    <option value="always">{t("settings.general.clicksWhenLocked.always")}</option>
    <option value="altOnly">{t("settings.general.clicksWhenLocked.altOnly")}</option>
  </select>
</Row>

<Row id="spacing" label={t("settings.general.sensorSpacing")}>
  <div class="slider">
    <input
      id="spacing"
      type="range"
      min="1"
      max="20"
      value={settings.sensorSpacing}
      onchange={(e) => save({ sensorSpacing: e.currentTarget.valueAsNumber })}
    />
    <output for="spacing">{t("common.px", { value: settings.sensorSpacing })}</output>
  </div>
</Row>

<style>
  .slider {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  output {
    min-width: 3.5em;
    white-space: nowrap;
  }
</style>
