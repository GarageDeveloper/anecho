<script lang="ts">
  import { app, AVERAGING_MODES, windowLabel } from "../lib/stores.svelte";
  import { generator } from "../lib/generator.svelte";
  import { RtaConfig_Averaging_Mode, StreamKind } from "../gen/anecho_pb";
  import { sigFigs } from "../lib/yrange";
  import MeasurePanel from "./MeasurePanel.svelte";

  const kindName: Record<number, string> = {
    [StreamKind.LEVELS]: "Levels",
    [StreamKind.RTA]: "RTA",
    [StreamKind.SCOPE]: "Scope",
    [StreamKind.RAW_INPUT]: "Raw",
  };

  function fmt(v: number | null, digits = 2): string {
    if (v == null || !Number.isFinite(v)) return "—";
    // Scope values are raw samples, often thousandths of full scale: show significant
    // digits instead of two decimals (which would read 0.00).
    return app.tab === "scope" ? sigFigs(v, 4) : v.toFixed(digits);
  }

  function fmtX(x: number): string {
    if (app.tab === "scope") return `${(x * 1000).toFixed(3)} ms`;
    return x >= 1000 ? `${(x / 1000).toFixed(3)} kHz` : `${x.toFixed(1)} Hz`;
  }

  const averagingLabel = $derived(
    AVERAGING_MODES.find((m) => m.value === app.rta.averagingMode)?.label ?? "None",
  );
  const counted = $derived(
    app.rta.averagingMode === RtaConfig_Averaging_Mode.EXPONENTIAL ||
      app.rta.averagingMode === RtaConfig_Averaging_Mode.LINEAR,
  );
  // Rows shown even before the cursor moves: the stream's channels, else the device's.
  const cursorChannels = $derived(
    app.cursor?.values.length ?? app.stream?.channels ?? app.selectedDevice?.inputChannels ?? 2,
  );
</script>

<aside class="inspector">
  <section>
    <h2>Stream</h2>
    {#if app.stream}
      <dl class="mono">
        <dt>kind</dt>
        <dd>{kindName[app.stream.kind] ?? app.stream.kind}</dd>
        <dt>sample rate</dt>
        <dd>{app.stream.sampleRate / 1000} kHz</dd>
        <dt>channels</dt>
        <dd>{app.stream.channels}</dd>
        <dt>unit</dt>
        <dd>{app.unit}</dd>
        {#if app.stream.kind === StreamKind.RTA}
          <dt>FFT</dt>
          <dd>{app.rta.fftLength}</dd>
          <dt>window</dt>
          <dd>{windowLabel(app.rta.window)}</dd>
          <dt>averaging</dt>
          <dd>{averagingLabel}{counted ? ` ×${app.rta.averagingCount}` : ""}</dd>
          <dt>points</dt>
          <dd>{app.stream.valuesPerChannel}</dd>
        {:else if app.stream.kind === StreamKind.SCOPE}
          <dt>window</dt>
          <dd>{app.scope.windowFrames} frames</dd>
          <dt>points</dt>
          <dd>{app.stream.valuesPerChannel}</dd>
        {/if}
        <dt>generator</dt>
        <dd>{generator.summary}</dd>
        {#if app.overruns > 0}
          <dt>overruns</dt>
          <dd class="warn">{app.overruns}</dd>
        {/if}
        {#if app.rangeChanges > 0}
          <dt>range changes</dt>
          <dd>{app.rangeChanges}</dd>
        {/if}
      </dl>
    {:else}
      <p class="muted">No stream running.</p>
    {/if}
    {#if app.error}
      <p class="error">{app.error}</p>
    {/if}
  </section>

  <section>
    <h2>Distortion (one-shot)</h2>
    <MeasurePanel />
  </section>

  {#if app.tab !== "levels"}
    <section>
      <h2>
        Cursor
        {#if app.cursor}
          <button class="clear" title="clear" onclick={() => app.clearCursor()}>×</button>
        {/if}
      </h2>
      <dl class="mono">
        <dt>x</dt>
        <dd>{app.cursor ? fmtX(app.cursor.x) : "—"}</dd>
        {#each Array.from({ length: cursorChannels }, (_, i) => i) as i (i)}
          <dt>CH {i + 1}</dt>
          <dd>{app.cursor ? fmt(app.cursor.values[i] ?? null) : "—"} {app.tab === "scope" ? "" : app.unit}</dd>
        {/each}
      </dl>
    </section>
  {/if}
</aside>

<style>
  .inspector {
    background: var(--panel);
    border-left: 1px solid var(--border);
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 20px;
    overflow-y: auto;
    font-size: 13px;
  }
  h2 {
    margin: 0 0 8px;
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--muted);
  }
  dl {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 4px 10px;
    margin: 0;
  }
  dt {
    color: var(--muted);
  }
  dd {
    margin: 0;
    text-align: right;
  }
  .warn {
    color: var(--warn);
  }
  .muted {
    color: var(--muted);
  }
  .error {
    color: var(--err);
    font-size: 12px;
    margin: 8px 0 0;
    word-break: break-word;
  }
  h2 {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .clear {
    background: none;
    border: 1px solid var(--border);
    border-radius: 3px;
    padding: 0 5px;
    font-size: 11px;
    line-height: 16px;
    color: var(--muted);
  }
</style>
