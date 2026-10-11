<script lang="ts">
  import AppShell from "../lib/AppShell.svelte";
  import { config } from "../lib/config.svelte";

  const edge = $derived(config.current?.settings.bar.edge ?? "bottom");
</script>

<AppShell>
  <main class="bar" data-edge={edge}></main>
</AppShell>

<style>
  :global(html, body) {
    margin: 0;
    height: 100%;
    overflow: hidden;
    background: transparent;
  }
  .bar {
    position: fixed;
    inset: 0;
    background: color-mix(in srgb, var(--bar-bg) calc(var(--bar-bg-opacity) * 100%), transparent);
    color: var(--sensor-fg);
    font-family: var(--sensor-font);
    font-weight: var(--sensor-weight);
    font-style: var(--sensor-style);
    font-variant-numeric: tabular-nums;
  }
  /* 1 px hairline on the edge facing the screen centre (SPEC §12.2). */
  .bar[data-edge="bottom"] {
    box-shadow: inset 0 1px 0 var(--bar-hairline);
  }
  .bar[data-edge="top"] {
    box-shadow: inset 0 -1px 0 var(--bar-hairline);
  }
</style>
