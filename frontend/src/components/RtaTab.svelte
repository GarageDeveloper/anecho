<script lang="ts">
  import Graph from "./Graph.svelte";
  import { app, AVERAGING_MODES, FFT_LENGTHS, OCTAVE_FRACTIONS, WINDOWS } from "../lib/stores.svelte";
  import { RtaConfig_Averaging_Mode } from "../gen/anecho_pb";

  const yRange = $derived<[number, number]>(app.unit === "dBV" ? [-140, 30] : [-140, 10]);
  const octave = $derived(app.rta.display === "octave");
  const counted = $derived(
    app.rta.averagingMode === RtaConfig_Averaging_Mode.EXPONENTIAL ||
      app.rta.averagingMode === RtaConfig_Averaging_Mode.LINEAR,
  );
</script>

<div class="tab">
  <div class="controls">
    <label>
      FFT
      <select bind:value={app.rta.fftLength} disabled={app.running}>
        {#each FFT_LENGTHS as n (n)}<option value={n}>{n / 1024}k</option>{/each}
      </select>
    </label>
    <label>
      Window
      <select bind:value={app.rta.window} disabled={app.running}>
        {#each WINDOWS as w (w.value)}<option value={w.value}>{w.label}</option>{/each}
      </select>
    </label>
    <label>
      Averaging
      <select bind:value={app.rta.averagingMode} disabled={app.running}>
        {#each AVERAGING_MODES as m (m.value)}<option value={m.value}>{m.label}</option>{/each}
      </select>
      {#if counted}
        <input type="number" min="2" max="256" bind:value={app.rta.averagingCount} disabled={app.running} />
      {/if}
    </label>
    <label>
      Display
      <select bind:value={app.rta.display} disabled={app.running}>
        <option value="log">Log points</option>
        <option value="octave">Octave bands</option>
      </select>
      {#if octave}
        <select bind:value={app.rta.octaveFraction} disabled={app.running}>
          {#each OCTAVE_FRACTIONS as f (f)}<option value={f}>1/{f}</option>{/each}
        </select>
      {:else}
        <input
          type="number"
          min="50"
          max="8192"
          step="50"
          bind:value={app.rta.points}
          disabled={app.running}
          title="points"
        />
      {/if}
    </label>
    <label>
      Range
      <input type="number" min="1" bind:value={app.rta.minHz} disabled={app.running} title="min Hz" />
      –
      <input type="number" min="10" bind:value={app.rta.maxHz} disabled={app.running} title="max Hz" />
      Hz
    </label>
    <label>
      Rate
      <input type="number" min="1" max="60" bind:value={app.rta.updateRateHz} disabled={app.running} />
      Hz
    </label>
  </div>
  <div class="plot">
    {#if app.rtaData}
      <Graph
        axis={app.rtaData.axis}
        series={app.rtaData.series}
        seq={app.rtaData.seq}
        xLog={true}
        bars={octave}
        xLabel="Hz"
        yLabel={app.unit}
        {yRange}
        onCursor={(c) => (app.cursor = c)}
      />
    {:else}
      <div class="empty">
        <p>Press <b>Start</b> to stream the spectrum.</p>
        <p class="muted">
          FFT, window, averaging and axis decimation run in the backend; the graph only draws the points it
          receives.
        </p>
      </div>
    {/if}
  </div>
</div>

<style>
  .tab {
    display: flex;
    flex-direction: column;
    height: 100%;
    gap: 8px;
  }
  .controls {
    display: flex;
    flex-wrap: wrap;
    gap: 12px;
    font-size: 12px;
    color: var(--muted);
  }
  .controls label {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .controls input[type="number"] {
    width: 72px;
    padding: 4px 6px;
  }
  .plot {
    flex: 1;
    min-height: 0;
  }
  .empty {
    max-width: 480px;
    padding: 24px 0;
  }
  .muted {
    color: var(--muted);
  }
</style>
