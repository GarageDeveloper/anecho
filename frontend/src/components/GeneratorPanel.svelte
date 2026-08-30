<script lang="ts">
  import { app } from "../lib/stores.svelte";
  import { DUAL_TONE_PRESETS, generator as g } from "../lib/generator.svelte";

  const outputs = $derived(app.selectedDevice?.outputChannels ?? 0);
  // Editable while streaming: the store restarts the stream with the new generator.
  const locked = $derived(!g.enabled);

  function toggleChannel(ch: number) {
    const all = Array.from({ length: outputs }, (_, i) => i);
    const current = g.outputChannels.length === 0 ? all : g.outputChannels;
    const set = new Set(current);
    if (set.has(ch)) set.delete(ch);
    else set.add(ch);
    const next = [...set].sort((a, b) => a - b);
    g.outputChannels = next.length === all.length ? [] : next;
  }

  function driven(ch: number): boolean {
    return g.outputChannels.length === 0 || g.outputChannels.includes(ch);
  }
</script>

{#if outputs > 0}
  <div class="gen">
    <label class="row">
      <input type="checkbox" bind:checked={g.enabled} />
      <span>Enable generator</span>
    </label>

    <div class="grid" class:off={!g.enabled}>
      <label for="gk">Signal</label>
      <select id="gk" bind:value={g.kind} disabled={locked}>
        <option value="sine">Sine</option>
        <option value="dualTone">Dual tone</option>
        <option value="multitone">Multitone</option>
        <option value="noise">Noise</option>
        <option value="square">Square</option>
      </select>

      {#if g.kind === "sine"}
        <label for="ghz">Frequency</label>
        <div class="inline">
          <input id="ghz" type="number" min="1" bind:value={g.sineHz} disabled={locked} /><span>Hz</span>
        </div>
      {:else if g.kind === "dualTone"}
        <span class="lbl">Preset</span>
        <div class="inline">
          <button onclick={() => g.applyDualPreset("smpte")} disabled={locked} title={DUAL_TONE_PRESETS.smpte.label}>
            SMPTE
          </button>
          <button onclick={() => g.applyDualPreset("ccif")} disabled={locked} title={DUAL_TONE_PRESETS.ccif.label}>
            CCIF
          </button>
        </div>
        <span class="lbl">f1 / f2</span>
        <div class="inline">
          <input type="number" min="1" bind:value={g.dual.f1} disabled={locked} title="f1 Hz" />
          <input type="number" min="1" bind:value={g.dual.f2} disabled={locked} title="f2 Hz" />
          <span>Hz</span>
        </div>
        <span class="lbl">Ratio</span>
        <div class="inline">
          <input type="number" step="0.01" bind:value={g.dual.ratioDb} disabled={locked} /><span>dB (f1 − f2)</span>
        </div>
      {:else if g.kind === "multitone"}
        <label for="gmt">Tones</label>
        <input id="gmt" type="text" bind:value={g.multitone} disabled={locked} placeholder="100, 1000, 10000" />
        <label for="gsch">Schroeder</label>
        <input id="gsch" type="checkbox" bind:checked={g.schroeder} disabled={locked} />
      {:else if g.kind === "noise"}
        <label for="gnk">Colour</label>
        <select id="gnk" bind:value={g.noiseKind} disabled={locked}>
          <option value="pink">Pink</option>
          <option value="white">White</option>
        </select>
        <label for="gper">Periodic</label>
        <div class="inline">
          <input id="gper" type="checkbox" bind:checked={g.periodic} disabled={locked} />
          {#if g.periodic}
            <input type="number" min="1024" step="1024" bind:value={g.periodFrames} disabled={locked} /><span>frames</span>
          {/if}
        </div>
      {:else}
        <label for="gsq">Frequency</label>
        <div class="inline">
          <input id="gsq" type="number" min="1" bind:value={g.squareHz} disabled={locked} /><span>Hz</span>
        </div>
      {/if}

      <label for="glv">Level</label>
      <div class="inline">
        {#if g.levelUnit === "dbv"}
          <input id="glv" type="number" step="1" bind:value={g.levelDbv} disabled={locked} />
        {:else}
          <input id="glv" type="number" step="1" max="0" bind:value={g.levelDbfs} disabled={locked} />
        {/if}
        <select bind:value={g.levelUnit} disabled={locked}>
          <option value="dbfs">dBFS peak</option>
          <option value="dbv" disabled={!app.calibrated}>dBV RMS</option>
        </select>
      </div>

      {#if outputs > 1}
        <span class="lbl">Outputs</span>
        <div class="inline">
          {#each Array.from({ length: outputs }, (_, i) => i) as ch (ch)}
            <label class="chip" class:on={driven(ch)}>
              <input type="checkbox" checked={driven(ch)} onchange={() => toggleChannel(ch)} disabled={locked} />
              {ch + 1}
            </label>
          {/each}
        </div>
      {/if}
    </div>
  </div>
{:else}
  <div class="muted">This device has no output.</div>
{/if}

<style>
  .gen {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text);
    font-size: 13px;
  }
  .grid {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 6px 10px;
    align-items: center;
  }
  .grid.off {
    opacity: 0.6;
  }
  .lbl {
    color: var(--muted);
    font-size: 12px;
  }
  .inline {
    display: flex;
    gap: 4px;
    align-items: center;
    flex-wrap: wrap;
  }
  .inline input[type="number"] {
    width: 72px;
    padding: 4px 6px;
  }
  .inline span,
  .muted {
    color: var(--muted);
    font-size: 12px;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    padding: 2px 6px;
    border: 1px solid var(--border);
    border-radius: 4px;
    font-size: 12px;
  }
  .chip.on {
    border-color: var(--accent);
    color: var(--text);
  }
</style>
