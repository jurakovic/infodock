<script lang="ts">
  /** Shared root for every window: loads the config, applies theme tokens and language. */
  import { onMount, type Snippet } from "svelte";
  import { config } from "./config.svelte";
  import { setLanguage } from "./i18n";
  import { applyTheme, DEFAULT_ACCENT, themeTokens } from "./theme";

  let { children }: { children: Snippet } = $props();

  const darkQuery = window.matchMedia("(prefers-color-scheme: dark)");
  let dark = $state(darkQuery.matches);

  onMount(() => {
    const onChange = (e: MediaQueryListEvent) => (dark = e.matches);
    darkQuery.addEventListener("change", onChange);
    config.init().catch((e: unknown) => console.error("config unavailable", e));
    return () => darkQuery.removeEventListener("change", onChange);
  });

  $effect(() => {
    const c = config.current;
    if (!c) return;
    setLanguage(c.settings.language);
    applyTheme(themeTokens(c.settings, { dark, accent: DEFAULT_ACCENT }), c.settings.theme);
  });
</script>

{#if config.current}
  {@render children()}
{/if}
