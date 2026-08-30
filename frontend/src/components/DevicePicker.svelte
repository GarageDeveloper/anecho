<script lang="ts">
  import { BackendKind } from "../gen/anecho_pb";
  import { app } from "../lib/stores.svelte";

  const kindLabel: Record<number, string> = {
    [BackendKind.QA40X]: "QA40x",
    [BackendKind.CPAL]: "Sound card",
    [BackendKind.VIRTUAL]: "Virtual",
  };
</script>

<div class="picker">
  <select
    value={app.selectedDeviceId}
    onchange={(e) => app.selectDevice(e.currentTarget.value)}
    disabled={app.running || app.devices.length === 0}
  >
    {#each app.devices as d (d.id)}
      <option value={d.id}>{d.displayName}</option>
    {/each}
  </select>
  <button onclick={() => app.refreshDevices()} disabled={app.connection !== "connected" || app.running} title="Refresh">
    ↻
  </button>
</div>

{#if app.selectedDevice}
  {@const d = app.selectedDevice}
  <div class="details">
    <span class="tag">{kindLabel[d.backend] ?? "?"}</span>
    {#if d.factoryCalibrated}
      <span class="tag ok">calibrated · dBV</span>
    {:else}
      <span class="tag warn">not calibrated · dBFS</span>
    {/if}
    {#if d.synchronousIo}<span class="tag">sync I/O</span>{/if}
    <div class="muted">
      in {d.inputChannels} · out {d.outputChannels} · {d.sampleRates.map((r) => r / 1000).join("/")} kHz
    </div>
    <div class="muted small">{d.id}</div>
  </div>
{:else if app.connection === "connected"}
  <div class="muted">No device found.</div>
{/if}

<style>
  .picker {
    display: flex;
    gap: 6px;
  }
  select {
    flex: 1;
    min-width: 0;
  }
  .details {
    margin-top: 8px;
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
  }
  .tag {
    font-size: 11px;
    padding: 2px 6px;
    border-radius: 4px;
    background: var(--panel-2);
    border: 1px solid var(--border);
  }
  .tag.ok {
    color: var(--ok);
    border-color: var(--ok);
  }
  .tag.warn {
    color: var(--warn);
    border-color: var(--warn);
  }
  .muted {
    color: var(--muted);
    width: 100%;
    font-size: 12px;
  }
  .small {
    font-size: 10px;
    word-break: break-all;
  }
</style>
