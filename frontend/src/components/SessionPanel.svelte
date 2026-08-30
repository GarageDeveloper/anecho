<script lang="ts">
  import { app } from "../lib/stores.svelte";

  const locked = $derived(app.sessionId !== null);
</script>

{#if app.selectedDevice}
  {@const d = app.selectedDevice}
  <div class="grid">
    <label for="sr">Sample rate</label>
    <select id="sr" bind:value={app.sampleRate} disabled={locked}>
      {#each d.sampleRates as r (r)}
        <option value={r}>{r / 1000} kHz</option>
      {/each}
    </select>

    {#if d.inputRanges.length > 0}
      <label for="in">Input range</label>
      <select id="in" bind:value={app.inputRange} disabled={locked || app.autoRangeInput}>
        {#each d.inputRanges as r, i (i)}
          <option value={i}>{r.label}</option>
        {/each}
      </select>
      <label for="ar">Auto range</label>
      <input id="ar" type="checkbox" bind:checked={app.autoRangeInput} disabled={locked} />
    {/if}

    {#if d.outputRanges.length > 0}
      <label for="out">Output range</label>
      <select id="out" bind:value={app.outputRange} disabled={locked}>
        {#each d.outputRanges as r, i (i)}
          <option value={i}>{r.label}</option>
        {/each}
      </select>
    {/if}
  </div>
  {#if locked}
    <div class="muted">Session open — stop to change the configuration.</div>
  {/if}
{/if}

<style>
  .grid {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 8px 10px;
    align-items: center;
  }
  .muted {
    margin-top: 6px;
    color: var(--muted);
    font-size: 12px;
  }
</style>
