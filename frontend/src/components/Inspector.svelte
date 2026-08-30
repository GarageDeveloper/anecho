<script lang="ts">
  import { app, AVERAGING_MODES, windowLabel } from "../lib/stores.svelte";
  import { generator } from "../lib/generator.svelte";
  import { RtaConfig_Averaging_Mode, StreamKind } from "../gen/anecho_pb";
  import MeasurePanel from "./MeasurePanel.svelte";

  const kindName: Record<number, string> = {
    [StreamKind.LEVELS]: "Levels",
    [StreamKind.RTA]: "RTA",
    [StreamKind.SCOPE]: "Scope",
    [StreamKind.RAW_INPUT]: "Raw",
  };

  function fmt(v: number | null, digits = 2): string {
    return v == null || !Number.isFinite(v) ? "—" : v.toFixed(digits);
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
  </section>

  {#if app.cursor && app.stream && app.stream.kind !== StreamKind.LEVELS}
    <section>
      <h2>Cursor</h2>
      <dl class="mono">
        <dt>x</dt>
        <dd>{fmtX(app.cursor.x)}</dd>
        {#each app.cursor.values as v, i (i)}
          <dt>CH {i + 1}</dt>
          <dd>{fmt(v)} {app.tab === "scope" ? "" : app.unit}</dd>
        {/each}
      </dl>
    </section>
  {/if}

  <section>
    <h2>Measure</h2>
    <MeasurePanel />
  </section>
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
</style>
