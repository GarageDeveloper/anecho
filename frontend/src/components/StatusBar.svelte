<script lang="ts">
  import { app, API_URL } from "../lib/stores.svelte";
</script>

<footer>
  <span class="dot {app.connection}"></span>
  <span>
    {#if app.connection === "connected"}
      backend {app.backendVersion} · {API_URL}
    {:else if app.connection === "connecting"}
      connecting to {API_URL}…
    {:else}
      disconnected — <button class="link" onclick={() => app.connect()}>reconnect</button>
    {/if}
  </span>
  {#if app.error}
    <span class="error">{app.error}</span>
  {/if}
</footer>

<style>
  footer {
    grid-column: 1 / -1;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 12px;
    background: var(--panel);
    border-top: 1px solid var(--border);
    color: var(--muted);
    font-size: 12px;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--err);
  }
  .dot.connecting {
    background: var(--warn);
  }
  .dot.connected {
    background: var(--ok);
  }
  .error {
    color: var(--err);
    margin-left: auto;
  }
  .link {
    background: none;
    border: none;
    padding: 0;
    color: var(--accent);
    text-decoration: underline;
  }
</style>
