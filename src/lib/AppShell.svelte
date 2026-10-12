<script lang="ts">
  /** Root of the Settings window: loads the config and applies the UI language. */
  import { onMount, type Snippet } from "svelte";
  import { config } from "./config.svelte";
  import { setLanguage } from "./i18n";

  let { children }: { children: Snippet } = $props();

  onMount(() => {
    config.init().catch((e: unknown) => console.error("config unavailable", e));
  });

  $effect(() => {
    const c = config.current;
    if (c) setLanguage(c.settings.language);
  });
</script>

{#if config.current}
  {@render children()}
{/if}
