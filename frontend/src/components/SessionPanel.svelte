<script lang="ts">
  import { app } from "../lib/stores.svelte";
</script>

{#if app.selectedDevice}
  {@const d = app.selectedDevice}
  <div class="grid">
    <label for="sr">Sample rate</label>
    <select id="sr" bind:value={app.sampleRate} disabled={app.running}>
      {#each d.sampleRates as r (r)}
        <option value={r}>{r / 1000} kHz</option>
      {/each}
    </select>

    {#if d.inputRanges.length > 0}
      <label for="in">Input range</label>
      <select id="in" bind:value={app.inputRange} disabled={app.running}>
        {#each d.inputRanges as r, i (i)}
          <option value={i}>{r.label}</option>
        {/each}
      </select>
    {/if}

    {#if d.outputRanges.length > 0}
      <label for="out">Output range</label>
      <select id="out" bind:value={app.outputRange} disabled={app.running}>
        {#each d.outputRanges as r, i (i)}
          <option value={i}>{r.label}</option>
        {/each}
      </select>
    {/if}

    {#if d.outputChannels > 0}
      <label for="gen">Generator</label>
      <div class="gen">
        <input id="gen" type="checkbox" bind:checked={app.generatorOn} disabled={app.running} />
        <input
          type="number"
          bind:value={app.generatorHz}
          min="1"
          step="1"
          disabled={app.running || !app.generatorOn}
          title="Frequency (Hz)"
        />
        <span class="muted">Hz</span>
        <input
          type="number"
          bind:value={app.generatorDbfs}
          max="0"
          step="1"
          disabled={app.running || !app.generatorOn}
          title="Amplitude (dBFS peak)"
        />
        <span class="muted">dBFS</span>
      </div>
    {/if}
  </div>
{/if}

<style>
  .grid {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 8px 10px;
    align-items: center;
  }
  .gen {
    display: flex;
    gap: 4px;
    align-items: center;
  }
  .gen input[type="number"] {
    width: 64px;
    padding: 4px 6px;
  }
  .muted {
    color: var(--muted);
    font-size: 12px;
  }
</style>
