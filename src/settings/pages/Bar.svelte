<script lang="ts">
  import { onMount } from "svelte";
  import type { MonitorInfo } from "../../lib/bindings/MonitorInfo";
  import type { Settings } from "../../lib/bindings/Settings";
  import { t } from "../../lib/i18n";
  import { ipc } from "../../lib/ipc";
  import { THEME_DEFAULTS } from "../../lib/theme";
  import Row from "../Row.svelte";
  import { save } from "../save.svelte";

  let { settings }: { settings: Settings } = $props();

  let monitors = $state<MonitorInfo[]>([]);
  onMount(async () => {
    monitors = await ipc.listMonitors();
  });

  const themeHeight = $derived(THEME_DEFAULTS[settings.theme].barHeight);
  const height = $derived(settings.bar.height ?? themeHeight);
  const savedMonitorMissing = $derived(
    settings.bar.monitor !== "primary" && monitors.length > 0 && !monitors.some((m) => m.key === settings.bar.monitor),
  );

  function monitorLabel(m: MonitorInfo): string {
    const params = { name: m.name, width: m.bounds.right - m.bounds.left, height: m.bounds.bottom - m.bounds.top };
    return t(m.primary ? "settings.bar.monitor.itemPrimary" : "settings.bar.monitor.item", params);
  }
</script>

<Row id="edge" label={t("settings.bar.edge")}>
  <select id="edge" value={settings.bar.edge} onchange={(e) => save({ bar: { edge: e.currentTarget.value as Settings["bar"]["edge"] } })}>
    <option value="top">{t("settings.bar.edge.top")}</option>
    <option value="bottom">{t("settings.bar.edge.bottom")}</option>
  </select>
</Row>

<Row id="monitor" label={t("settings.bar.monitor")}>
  <select id="monitor" value={settings.bar.monitor} onchange={(e) => save({ bar: { monitor: e.currentTarget.value } })}>
    <option value="primary">{t("settings.bar.monitor.primary")}</option>
    {#each monitors as m (m.key)}
      <option value={m.key}>{monitorLabel(m)}</option>
    {/each}
    {#if savedMonitorMissing}
      <option value={settings.bar.monitor}>
        {t("settings.bar.monitor.missing", { name: settings.bar.monitor.split("|")[1] ?? settings.bar.monitor })}
      </option>
    {/if}
  </select>
</Row>

<Row id="reserve" label={t("settings.bar.reserveSpace")} help={t("settings.bar.reserveSpace.help")}>
  <input
    id="reserve"
    type="checkbox"
    checked={settings.bar.reserveSpace}
    onchange={(e) => save({ bar: { reserveSpace: e.currentTarget.checked } })}
    aria-describedby="reserve-help"
  />
</Row>

<Row id="zorder" label={t("settings.bar.zOrder")} help={t("settings.bar.zOrder.help")}>
  <select
    id="zorder"
    value={settings.bar.zOrder}
    onchange={(e) => save({ bar: { zOrder: e.currentTarget.value as Settings["bar"]["zOrder"] } })}
    aria-describedby="zorder-help"
  >
    <option value="topmost">{t("settings.bar.zOrder.topmost")}</option>
    <option value="normal">{t("settings.bar.zOrder.normal")}</option>
    <option value="bottom">{t("settings.bar.zOrder.bottom")}</option>
  </select>
</Row>

<Row id="fullscreen" label={t("settings.bar.hideOnFullscreen")}>
  <input
    id="fullscreen"
    type="checkbox"
    checked={settings.bar.hideOnFullscreen}
    onchange={(e) => save({ bar: { hideOnFullscreen: e.currentTarget.checked } })}
  />
</Row>

<Row id="height" label={t("settings.bar.height")} help={t("settings.bar.height.help", { height: themeHeight })}>
  <div class="slider">
    <input
      id="height"
      type="range"
      min="22"
      max="40"
      value={height}
      onchange={(e) => save({ bar: { height: e.currentTarget.valueAsNumber } })}
      aria-describedby="height-help"
    />
    <output for="height">{t("common.px", { value: height })}</output>
    <button type="button" disabled={settings.bar.height === null} onclick={() => save({ bar: { height: null } })}>
      {t("settings.bar.height.reset")}
    </button>
  </div>
</Row>

<style>
  .slider {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  output,
  button {
    white-space: nowrap;
  }
  output {
    min-width: 3.5em;
  }
</style>
