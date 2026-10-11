<script lang="ts">
  /** One setting: label, control, optional help text and error. */
  import type { Snippet } from "svelte";

  let {
    id,
    label,
    help,
    error,
    children,
  }: { id: string; label: string; help?: string; error?: string | null; children: Snippet } = $props();
</script>

<div class="row">
  <label for={id}>{label}</label>
  <div class="control">
    {@render children()}
    {#if help}<p class="help" id={`${id}-help`}>{help}</p>{/if}
    {#if error}<p class="error" role="alert">{error}</p>{/if}
  </div>
</div>

<style>
  .row {
    display: grid;
    grid-template-columns: 200px 1fr;
    gap: 4px 16px;
    align-items: start;
    padding: 10px 0;
    border-bottom: 1px solid color-mix(in srgb, CanvasText 10%, transparent);
  }
  label {
    padding-top: 5px;
    font-weight: 600;
  }
  .help,
  .error {
    margin: 4px 0 0;
    font-size: 12px;
  }
  .help {
    opacity: 0.7;
  }
  .error {
    color: #d13438;
  }
</style>
