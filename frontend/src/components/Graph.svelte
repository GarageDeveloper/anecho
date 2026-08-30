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

  /** Compact frequency label: 20, 315, 1k, 1.25k, 12.5k. */
  function hzLabel(v: number): string {
    if (v >= 1000) {
      const k = v / 1000;
      const txt = Number.isInteger(k) ? `${k}` : k.toFixed(2).replace(/\.?0+$/, "");
      return `${txt}k`;
    }
    return Number.isInteger(v) ? `${v}` : v.toFixed(1).replace(/\.0$/, "");
  }

  /** Decades and their 2·/5· multiples: 20, 50, 100, 200, 500, 1k, 2k, 5k, 10k, 20k... */
  function isLogMajor(v: number): boolean {
    if (v <= 0) return false;
    const m = v / Math.pow(10, Math.floor(Math.log10(v) + 1e-9));
    return [1, 2, 5].some((k) => Math.abs(m - k) < 1e-6);
  }

  /** Labels for the log axis: named majors only, "" elsewhere (uPlot would print null). */
  function logAxisValues(_u: uPlot, vals: number[]): string[] {
    return vals.map((v) => (isLogMajor(v) ? hzLabel(v) : ""));
  }

  /** Labels for octave bands: one per band centre, thinned when they would overlap. */
  function bandAxisValues(u: uPlot, vals: number[]): string[] {
    const centres = u.data[0] as number[];
    const px = u.bbox.width / Math.max(1, centres.length) / (window.devicePixelRatio || 1);
    const every = px >= 44 ? 1 : px >= 24 ? 2 : px >= 14 ? 3 : 6;
    return vals.map((v) => {
      const i = centres.findIndex((c) => Math.abs(c - v) <= Math.abs(c) * 1e-6);
      if (i < 0 || i % every !== 0) return "";
      return hzLabel(v);
    });
  }

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
        // Bars rise from the bottom of the Y scale, not from 0 (levels are negative dB).
        paths: bars
          ? uPlot.paths.bars!({
              size: [0.7, 40],
              align: 0,
              disp: {
                y0: { unit: 1, values: (u, sidx) => (u.data[sidx] as number[]).map(() => yRange[0]) },
              },
            })
          : undefined,
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
        // Octave bands: ordinal X (one slot per band); log points: true log scale.
        x: xLog ? (bars ? { distr: 2 } : { distr: 3, log: 10 }) : { time: false },
        y: { range: () => [yMin, yMax] },
      },
      axes: [
        {
          label: xLabel,
          stroke: "#8b929c",
          grid: { stroke: "#2e333b", width: 1 },
          ticks: { stroke: "#2e333b" },
          values: bars ? bandAxisValues : xLog ? logAxisValues : undefined,
          splits: bars ? (u) => u.data[0] as number[] : undefined,
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
            // Leaving the graph keeps the last readout (the inspector shows it until cleared).
            if (idx == null || idx < 0) return;
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
