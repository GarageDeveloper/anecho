<script lang="ts">
  import { onMount } from "svelte";
  import { app } from "./lib/stores.svelte";
  import DevicePicker from "./components/DevicePicker.svelte";
  import SessionPanel from "./components/SessionPanel.svelte";
  import LevelMeters from "./components/LevelMeters.svelte";
  import StatusBar from "./components/StatusBar.svelte";

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
      <h2>3 · Levels</h2>
      {#if app.running}
        <button onclick={() => app.stop()}>Stop</button>
      {:else}
        <button
          class="primary"
          onclick={() => app.start()}
          disabled={!app.selectedDevice || app.connection !== "connected"}
        >
          Start
        </button>
      {/if}
    </section>
  </aside>
  <main>
    <LevelMeters />
  </main>
  <StatusBar />
</div>

<style>
  .layout {
    display: grid;
    grid-template-columns: 300px 1fr;
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
  main {
    padding: 24px;
    overflow: auto;
  }
</style>
