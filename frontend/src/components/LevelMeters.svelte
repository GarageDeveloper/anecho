<script lang="ts">
  import { app } from "../lib/stores.svelte";

  // Meter display range in dB. Bars are a linear map of the dB value the backend sent.
  const MIN_DB = -100;
  const MAX_DB = 20;
  const TICKS = [-100, -80, -60, -40, -20, 0, 20];

  function pct(db: number): number {
    const p = ((db - MIN_DB) / (MAX_DB - MIN_DB)) * 100;
    return Math.max(0, Math.min(100, p));
  }

  function fmt(db: number): string {
    return db <= -199 ? "—" : db.toFixed(1);
  }

  function tone(db: number): string {
    if (db >= 0) return "clip";
    if (db >= -6) return "hot";
    return "";
  }
</script>

{#if app.running && app.levels.length > 0}
  <div class="meters">
    {#each app.levels as l, ch (ch)}
      <div class="channel">
        <div class="head">
          <span class="name">CH {ch + 1}</span>
          <span class="mono readout">
            rms <b>{fmt(l.rms)}</b> · peak <b>{fmt(l.peak)}</b>
            {app.unit}
          </span>
        </div>
        <div class="bar">
          <div class="fill {tone(l.rms)}" style="width: {pct(l.rms)}%"></div>
          <div class="peak {tone(l.peak)}" style="left: {pct(l.peak)}%"></div>
        </div>
      </div>
    {/each}
    <div class="scale mono">
      {#each TICKS as t (t)}
        <span style="left: {pct(t)}%">{t}</span>
      {/each}
    </div>
    {#if app.overruns > 0}
      <div class="warn">{app.overruns} dropped block(s)</div>
    {/if}
  </div>
{:else}
  <div class="empty">
    <p>Select a device, then <b>Start</b> to see live input levels.</p>
    <p class="muted">Levels are computed by the backend and arrive ready to display.</p>
  </div>
{/if}

<style>
  .meters {
    display: flex;
    flex-direction: column;
    gap: 18px;
    max-width: 900px;
  }
  .channel .head {
    display: flex;
    justify-content: space-between;
    margin-bottom: 6px;
  }
  .name {
    color: var(--muted);
    font-size: 12px;
    letter-spacing: 0.06em;
  }
  .readout {
    font-size: 13px;
    color: var(--muted);
  }
  .readout b {
    color: var(--text);
    font-weight: 600;
  }
  .bar {
    position: relative;
    height: 26px;
    background: var(--panel-2);
    border: 1px solid var(--border);
    border-radius: 4px;
    overflow: hidden;
  }
  .fill {
    height: 100%;
    background: var(--meter);
    transition: width 40ms linear;
  }
  .fill.hot {
    background: var(--meter-hot);
  }
  .fill.clip {
    background: var(--meter-clip);
  }
  .peak {
    position: absolute;
    top: 0;
    width: 2px;
    height: 100%;
    background: var(--text);
    transition: left 40ms linear;
  }
  .peak.hot {
    background: var(--meter-hot);
  }
  .peak.clip {
    background: var(--meter-clip);
  }
  .scale {
    position: relative;
    height: 16px;
    color: var(--muted);
    font-size: 11px;
  }
  .scale span {
    position: absolute;
    transform: translateX(-50%);
  }
  .warn {
    color: var(--warn);
    font-size: 12px;
  }
  .empty {
    max-width: 480px;
  }
  .muted {
    color: var(--muted);
  }
</style>
