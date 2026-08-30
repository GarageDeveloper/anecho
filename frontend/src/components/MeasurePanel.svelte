<script lang="ts">
  // One-shot distortion measurements (THD, IMD). They reuse the RTA analysis settings
  // unless "Override" is on; a running stream is paused and resumed around the measurement.
  import { app, FFT_LENGTHS, WINDOWS, windowLabel } from "../lib/stores.svelte";
  import { generator } from "../lib/generator.svelte";
  import { MeasureKind } from "../gen/anecho_pb";

  const kinds: { kind: MeasureKind; label: string }[] = [
    { kind: MeasureKind.THD, label: "THD" },
    { kind: MeasureKind.IMD_SMPTE, label: "IMD SMPTE" },
    { kind: MeasureKind.IMD_CCIF, label: "IMD CCIF" },
  ];

  const canMeasure = $derived(
    app.connection === "connected" && !!app.selectedDevice && !app.measure.busy && !app.restarting,
  );
  const isImd = $derived(
    app.measure.result?.kind === MeasureKind.IMD_SMPTE || app.measure.result?.kind === MeasureKind.IMD_CCIF,
  );
  const eff = $derived(app.measureEffective);

  function pct(v: number): string {
    return v < 0.01 ? v.toExponential(2) : v.toFixed(4);
  }
</script>

<div class="measure">
  <p class="muted small settings">
    {#if app.measure.override}
      Own analysis settings:
    {:else}
      Uses the RTA settings:
    {/if}
    <span class="mono">{eff.fftLength / 1024}k · {windowLabel(eff.window)} · ×{eff.averages}</span>
  </p>
  <label class="toggle small">
    <input type="checkbox" bind:checked={app.measure.override} disabled={app.measure.busy} />
    Override
  </label>
  <div class="grid">
    {#if app.measure.override}
      <label for="mfft">FFT</label>
      <select id="mfft" bind:value={app.measure.fftLength} disabled={app.measure.busy}>
        {#each FFT_LENGTHS as n (n)}<option value={n}>{n / 1024}k</option>{/each}
      </select>
      <label for="mwin">Window</label>
      <select id="mwin" bind:value={app.measure.window} disabled={app.measure.busy}>
        {#each WINDOWS as w (w.value)}<option value={w.value}>{w.label}</option>{/each}
      </select>
      <label for="mavg">Averages</label>
      <input id="mavg" type="number" min="1" max="64" bind:value={app.measure.averages} disabled={app.measure.busy} />
    {/if}
    <label for="mh">Harmonics</label>
    <input id="mh" type="number" min="2" max="20" bind:value={app.measure.maxHarmonic} disabled={app.measure.busy} />
  </div>
  <div class="buttons">
    {#each kinds as k (k.kind)}
      <button
        onclick={() => app.runMeasure(k.kind)}
        disabled={!canMeasure}
        class:busy={app.measure.busy && app.measure.kind === k.kind}
      >
        {k.label}
      </button>
    {/each}
  </div>
  <p class="muted small">
    {#if !generator.enabled}
      Generator is off: the measurement uses the external signal.
    {:else}
      Uses the rail generator: {generator.summary}.
    {/if}
    {#if app.running || app.measuringPaused}<br />The stream pauses during the measurement and resumes after it.{/if}
  </p>

  {#if app.measure.result}
    {@const r = app.measure.result}
    {#each r.perChannel as d, ch (ch)}
      <div class="result mono">
        <div class="head">CH {ch + 1}</div>
        <dl>
          <dt>f0</dt>
          <dd>{d.fundamentalHz.toFixed(2)} Hz</dd>
          <dt>level</dt>
          <dd>{d.fundamentalLevel.toFixed(2)} {app.unit}</dd>
          {#if isImd}
            <dt>IMD</dt>
            <dd>{pct(d.imdPct)} % · {d.imdDb.toFixed(1)} dB</dd>
          {:else}
            <dt>THD</dt>
            <dd>{pct(d.thdPct)} % · {d.thdDb.toFixed(1)} dB</dd>
            <dt>THD+N</dt>
            <dd>{pct(d.thdNPct)} % · {d.thdNDb.toFixed(1)} dB</dd>
            <dt>noise floor</dt>
            <dd>{d.noiseFloorDb.toFixed(1)} dBc</dd>
          {/if}
        </dl>
        {#if d.harmonics.length > 0}
          <table>
            <thead><tr><th>H</th><th>Hz</th><th>dBc</th></tr></thead>
            <tbody>
              {#each d.harmonics as h (h.order)}
                <tr><td>{h.order}</td><td>{h.frequencyHz.toFixed(0)}</td><td>{h.levelDbRel.toFixed(1)}</td></tr>
              {/each}
            </tbody>
          </table>
        {/if}
      </div>
    {/each}
  {/if}
</div>

<style>
  .measure {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .settings {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 6px;
  }
  .toggle {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--muted);
  }
  .grid {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 6px 10px;
    align-items: center;
  }
  .grid input {
    width: 72px;
    padding: 4px 6px;
  }
  .buttons {
    display: flex;
    gap: 6px;
  }
  .buttons button.busy {
    border-color: var(--warn);
  }
  .result {
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 8px;
    font-size: 12px;
  }
  .head {
    color: var(--muted);
    margin-bottom: 4px;
  }
  dl {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 2px 10px;
    margin: 0 0 6px;
  }
  dt {
    color: var(--muted);
  }
  dd {
    margin: 0;
    text-align: right;
  }
  table {
    width: 100%;
    border-collapse: collapse;
  }
  th,
  td {
    text-align: right;
    padding: 1px 4px;
  }
  th {
    color: var(--muted);
    font-weight: 500;
  }
  .muted {
    color: var(--muted);
  }
  .small {
    font-size: 11px;
    margin: 0;
  }
</style>
