<script lang="ts">
  // uPlot wrapper: draws exactly the values it is given against the axis it is given.
  // No smoothing, no interpolation — the backend prepares plot-ready data.
  import uPlot from "uplot";
  import "uplot/dist/uPlot.min.css";
  import { onMount } from "svelte";
  import type { CursorReadout } from "../lib/stores.svelte";

  interface Props {
    axis: number[];
    series: ArrayLike<number>[];
    seq?: bigint | number;
    xLog?: boolean;
    xLabel?: string;
    yLabel?: string;
    yRange?: [number, number];
    bars?: boolean;
    seriesNames?: string[];
    onCursor?: (c: CursorReadout | null) => void;
  }

  let {
    axis,
    series,
    seq = 0,
    xLog = false,
    xLabel = "",
    yLabel = "",
    yRange = [-120, 20],
    bars = false,
    seriesNames = [],
    onCursor,
  }: Props = $props();

  const COLORS = ["#4fa3ff", "#f5b942", "#3ccf7a", "#ff5c5c", "#c58cff", "#5ee0e6"];

  let host: HTMLDivElement;
  let plot: uPlot | null = null;
  let raf = 0;
  let pending = false;

  function toData(): uPlot.AlignedData {
    return [axis, ...series.map((v) => Array.from(v))];
  }

  function build() {
    plot?.destroy();
    plot = null;
    if (!host) return;
    const s: uPlot.Series[] = [{}];
    for (let i = 0; i < series.length; i++) {
      const color = COLORS[i % COLORS.length];
      s.push({
        label: seriesNames[i] ?? `CH ${i + 1}`,
        stroke: color,
        width: 1.5,
        points: { show: false },
        paths: bars ? uPlot.paths.bars!({ size: [0.7, 40], align: 0 }) : undefined,
        fill: bars ? color + "55" : undefined,
      });
    }
    const [yMin, yMax] = yRange;
    const opts: uPlot.Options = {
      width: host.clientWidth || 600,
      height: host.clientHeight || 300,
      series: s,
      cursor: { drag: { x: false, y: false } },
      legend: { show: false },
      scales: {
        x: xLog ? { distr: 3, log: 10 } : { time: false },
        y: { range: () => [yMin, yMax] },
      },
      axes: [
        {
          label: xLabel,
          stroke: "#8b929c",
          grid: { stroke: "#2e333b", width: 1 },
          ticks: { stroke: "#2e333b" },
          values: xLog ? (_u, vals) => vals.map((v) => (v >= 1000 ? `${v / 1000}k` : `${v}`)) : undefined,
        },
        {
          label: yLabel,
          stroke: "#8b929c",
          grid: { stroke: "#2e333b", width: 1 },
          ticks: { stroke: "#2e333b" },
          size: 60,
        },
      ],
      hooks: {
        setCursor: [
          (u) => {
            if (!onCursor) return;
            const idx = u.cursor.idx;
            if (idx == null || idx < 0) {
              onCursor(null);
              return;
            }
            const x = u.data[0][idx];
            const values = u.data.slice(1).map((d) => (d[idx] == null ? null : (d[idx] as number)));
            onCursor({ x, values });
          },
        ],
      },
    };
    plot = new uPlot(opts, toData(), host);
  }

  function refresh() {
    if (!plot || pending) return;
    pending = true;
    raf = requestAnimationFrame(() => {
      pending = false;
      plot?.setData(toData(), true);
    });
  }

  onMount(() => {
    build();
    const ro = new ResizeObserver(() => plot?.setSize({ width: host.clientWidth, height: host.clientHeight }));
    ro.observe(host);
    return () => {
      ro.disconnect();
      cancelAnimationFrame(raf);
      plot?.destroy();
      plot = null;
    };
  });

  // Rebuild when the structure changes (axis, series count, mode); otherwise just push data.
  $effect(() => {
    void axis;
    void series.length;
    void xLog;
    void bars;
    void yRange;
    if (plot) build();
  });
  $effect(() => {
    void seq;
    void series;
    refresh();
  });
</script>

<div class="graph" bind:this={host}></div>

<style>
  .graph {
    width: 100%;
    height: 100%;
    min-height: 240px;
  }
  .graph :global(.u-wrap) {
    font-family: "JetBrains Mono", ui-monospace, monospace;
    font-size: 11px;
  }
</style>
