<script lang="ts">
  import Graph from "./Graph.svelte";
  import { app } from "../lib/stores.svelte";
  import { ScopeConfig_Trigger_Mode } from "../gen/anecho_pb";
  import { FIXED_Y_RANGES, amplitudeLabel, autoHalfRange } from "../lib/yrange";

  const channels = $derived(app.selectedDevice?.inputChannels ?? 2);

  // Auto Y scale: follow the visible channels' peak with hysteresis (pure display
  // scaling in lib/yrange.ts — the data are never touched).
  $effect(() => {
    const d = app.scopeData;
    if (!d || app.scope.yMode !== "auto") return;
    let peak = 0;
    for (let ch = 0; ch < d.series.length; ch++) {
      if (app.scope.hidden[ch]) continue;
      const s = d.series[ch];
      for (let i = 0; i < s.length; i++) {
        const a = Math.abs(s[i]);
        if (a > peak) peak = a;
      }
    }
    const next = autoHalfRange(peak, app.scope.autoHalf);
    if (next !== app.scope.autoHalf) app.scope.autoHalf = next;
  });

  const half = $derived(app.scope.yMode === "auto" ? app.scope.autoHalf : app.scope.yMode);
  const yRange = $derived<[number, number]>([-half, half]);
</script>

<div class="tab">
  <div class="controls">
    <label>
      Window
      <input
        type="number"
        min="64"
        max="1048576"
        step="64"
        bind:value={app.scope.windowFrames}

      />
      frames
    </label>
    <label>
      Points
      <input type="number" min="16" max="8192" step="16" bind:value={app.scope.points} />
    </label>
    <label>
      Trigger
      <select bind:value={app.scope.triggerMode}>
        <option value={ScopeConfig_Trigger_Mode.UNSPECIFIED}>Free run</option>
        <option value={ScopeConfig_Trigger_Mode.RISING}>Rising</option>
        <option value={ScopeConfig_Trigger_Mode.FALLING}>Falling</option>
      </select>
      <input
        type="number"
        min="-1"
        max="1"
        step="0.05"
        bind:value={app.scope.triggerLevel}

        title="level (±1)"
      />
      <select bind:value={app.scope.triggerChannel}>
        {#each Array.from({ length: channels }, (_, i) => i) as ch (ch)}<option value={ch}>CH {ch + 1}</option>{/each}
      </select>
    </label>
    <label>
      Y
      <select bind:value={app.scope.yMode}>
        <option value="auto">Auto</option>
        {#each FIXED_Y_RANGES as r (r)}
          <option value={r}>±{r}</option>
        {/each}
      </select>
    </label>
  </div>
  <div class="plot">
    {#if app.scopeData}
      <Graph
        axis={app.scopeData.axis}
        series={app.scopeData.series}
        seq={app.scopeData.seq}
        xLabel="s"
        yLabel="full scale"
        {yRange}
        yValues={amplitudeLabel}
        hidden={app.scope.hidden}
        onToggleChannel={(i) => (app.scope.hidden[i] = !app.scope.hidden[i])}
        onCursor={(c) => (app.cursor = c)}
      />
    {:else}
      <div class="empty">
        <p>Press <b>Start</b> to stream the waveform.</p>
        <p class="muted">Triggering and decimation happen in the backend.</p>
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
    width: 80px;
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
