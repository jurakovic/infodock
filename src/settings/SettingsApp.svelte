<script lang="ts">
  import { onMount } from "svelte";
  import AppShell from "../lib/AppShell.svelte";
  import { config } from "../lib/config.svelte";
  import { t, type MsgKey } from "../lib/i18n";
  import { ipc } from "../lib/ipc";
  import { settingsPage } from "../lib/route";
  import Bar from "./pages/Bar.svelte";
  import General from "./pages/General.svelte";
  import { saveStatus } from "./save.svelte";

  const PAGES = [
    { id: "general", label: "settings.nav.general", component: General },
    { id: "bar", label: "settings.nav.bar", component: Bar },
  ] as const satisfies readonly { id: string; label: MsgKey; component: unknown }[];

  type PageId = (typeof PAGES)[number]["id"];
  const isPage = (id: string): id is PageId => PAGES.some((p) => p.id === id);

  let page = $state<PageId>("general");

  function go(id: string) {
    if (!isPage(id)) return;
    page = id;
    history.replaceState(null, "", `/settings/${id}`);
  }

  onMount(() => {
    go(settingsPage(location.pathname));
    const unlisten = ipc.onSettingsNavigate((route) => go(route.split("/")[0] ?? ""));
    return () => void unlisten.then((f) => f());
  });

  const Current = $derived(PAGES.find((p) => p.id === page)?.component ?? General);
</script>

<AppShell>
  <div class="settings">
    <nav aria-label={t("settings.nav.label")}>
      {#each PAGES as p (p.id)}
        <button type="button" class:active={p.id === page} aria-current={p.id === page ? "page" : undefined} onclick={() => go(p.id)}>
          {t(p.label)}
        </button>
      {/each}
    </nav>
    <main>
      <h1>{t(PAGES.find((p) => p.id === page)?.label ?? "settings.nav.general")}</h1>
      {#if saveStatus.error && saveStatus.error.code !== "hotkey"}
        <p class="error" role="alert">{t("settings.saveError", { error: saveStatus.error.message })}</p>
      {/if}
      {#if config.current}
        <Current settings={config.current.settings} />
      {/if}
    </main>
  </div>
</AppShell>

<style>
  :global(html) {
    color-scheme: light dark;
  }
  :global(body) {
    margin: 0;
    background: Canvas;
    color: CanvasText;
    font: 14px/1.4 "Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif;
  }
  .settings {
    display: grid;
    grid-template-columns: 180px 1fr;
    min-height: 100vh;
  }
  nav {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 16px 8px;
    border-right: 1px solid color-mix(in srgb, CanvasText 12%, transparent);
  }
  nav button {
    text-align: left;
    padding: 8px 12px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: inherit;
    font: inherit;
    cursor: pointer;
  }
  nav button:hover {
    background: color-mix(in srgb, CanvasText 6%, transparent);
  }
  nav button.active {
    background: color-mix(in srgb, CanvasText 10%, transparent);
    font-weight: 600;
  }
  main {
    padding: 16px 24px 32px;
    overflow-y: auto;
  }
  h1 {
    margin: 0 0 8px;
    font-size: 20px;
    font-weight: 600;
  }
  .error {
    color: #d13438;
  }
</style>
