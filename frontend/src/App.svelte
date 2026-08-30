<script lang="ts">
  import { onMount } from "svelte";
  import { app, type Tab } from "./lib/stores.svelte";
  import DevicePicker from "./components/DevicePicker.svelte";
  import SessionPanel from "./components/SessionPanel.svelte";
  import GeneratorPanel from "./components/GeneratorPanel.svelte";
  import LevelMeters from "./components/LevelMeters.svelte";
  import RtaTab from "./components/RtaTab.svelte";
  import ScopeTab from "./components/ScopeTab.svelte";
  import Inspector from "./components/Inspector.svelte";
  import StatusBar from "./components/StatusBar.svelte";

  const tabs: { id: Tab; label: string }[] = [
    { id: "levels", label: "Levels" },
    { id: "rta", label: "RTA" },
    { id: "scope", label: "Scope" },
  ];

  // Any RTA/scope control or generator edit while streaming restarts the stream
  // (debounced in the store); the tab switch has its own path in selectTab.
  let lastSig = app.streamSignature;
  $effect(() => {
    const sig = app.streamSignature;
    if (sig !== lastSig) {
      lastSig = sig;
      app.scheduleRestart();
    }
  });

  onMount(() => {
    // The desktop shell starts the backend before the webview loads; retry briefly anyway.
    let attempts = 0;
    const connected = () => app.connection === "connected";
    const tick = async () => {
      if (connected()) return;
      await app.connect();
      if (!connected() && attempts++ < 20) setTimeout(tick, 500);
    };
    tick();
  });
</script>

<div class="layout">
  <aside class="rail">
    <h1>Anecho</h1>
    <section>
      <h2>1 · Device</h2>
      <DevicePicker />
    </section>
    <section>
      <h2>2 · Session</h2>
      <SessionPanel />
    </section>
    <section>
      <h2>3 · Generator</h2>
      <GeneratorPanel />
    </section>
    <section>
      <h2>4 · Capture</h2>
      {#if app.measuringPaused}
        <button disabled><span class="spinner"></span> Stop</button>
        <div class="state ok">measuring…</div>
      {:else if app.streamPhase === "starting"}
        <button class="primary" disabled><span class="spinner"></span> Starting…</button>
        <div class="state">starting {app.tab}…</div>
      {:else if app.streamPhase === "stopping"}
        <button disabled><span class="spinner"></span> Stopping…</button>
        <div class="state">stopping…</div>
      {:else if app.running}
        <button onclick={() => app.stop()}>Stop</button>
        <div class="state ok">streaming {app.tab}</div>
      {:else}
        <button
          class="primary"
          onclick={() => app.start()}
          disabled={!app.selectedDevice || app.connection !== "connected" || app.measure.busy}
        >
          Start {app.tab}
        </button>
        <div class="state">{app.sessionId !== null ? "session open" : "idle"}</div>
      {/if}
    </section>
  </aside>
  <main>
    <nav class="tabs">
      {#each tabs as t (t.id)}
        <button class:active={app.tab === t.id} onclick={() => app.selectTab(t.id)}>{t.label}</button>
      {/each}
    </nav>
    <div class="content">
      {#if app.waitingForData}
        <div class="waiting"><span class="spinner"></span> waiting for data…</div>
      {/if}
      {#if app.tab === "levels"}
        <LevelMeters />
      {:else if app.tab === "rta"}
        <RtaTab />
      {:else}
        <ScopeTab />
      {/if}
    </div>
  </main>
  <Inspector />
  <StatusBar />
</div>

<style>
  .layout {
    display: grid;
    grid-template-columns: 300px 1fr 280px;
    grid-template-rows: 1fr auto;
    height: 100vh;
  }
  .rail {
    background: var(--panel);
    border-right: 1px solid var(--border);
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 20px;
    overflow-y: auto;
  }
  h1 {
    margin: 0;
    font-size: 18px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--accent);
  }
  h2 {
    margin: 0 0 8px;
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--muted);
  }
  .state {
    margin-top: 6px;
    font-size: 12px;
    color: var(--muted);
  }
  .state.ok {
    color: var(--ok);
  }
  main {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }
  .tabs {
    display: flex;
    gap: 2px;
    padding: 8px 16px 0;
    border-bottom: 1px solid var(--border);
  }
  .tabs button {
    background: none;
    border: none;
    border-bottom: 2px solid transparent;
    border-radius: 0;
    padding: 8px 14px;
    color: var(--muted);
  }
  .tabs button.active {
    color: var(--text);
    border-bottom-color: var(--accent);
  }
  .content {
    position: relative;
    flex: 1;
    min-height: 0;
    padding: 16px 24px 24px;
    overflow: auto;
  }
  .waiting {
    position: absolute;
    top: 56px;
    left: 50%;
    transform: translateX(-50%);
    z-index: 3;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 14px;
    font-size: 12px;
    color: var(--muted);
    background: rgba(20, 23, 28, 0.85);
    border: 1px solid var(--border);
    border-radius: 6px;
  }
  .spinner {
    display: inline-block;
    width: 11px;
    height: 11px;
    border: 2px solid currentColor;
    border-top-color: transparent;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
    vertical-align: -1px;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
