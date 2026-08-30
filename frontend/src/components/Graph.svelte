<script lang="ts">
  // uPlot wrapper: draws exactly the values it is given against the axis it is given.
  // No smoothing, no interpolation — the backend prepares plot-ready data.
  import uPlot from "uplot";
  import "uplot/dist/uPlot.min.css";
  import { onMount } from "svelte";
  import type { CursorReadout } from "../lib/stores.svelte";
  import { channelBars, nearestSlot, slotWidth } from "../lib/bars";
  import { hzLabel, logAxisPlan, type LogAxisPlan } from "../lib/axis";

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
    /** Hidden state per channel, owned by the tab so it survives tab switches. */
    hidden?: boolean[];
    onToggleChannel?: (i: number) => void;
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
    hidden = [],
    onToggleChannel,
    onCursor,
  }: Props = $props();

  const COLORS = ["#4fa3ff", "#f5b942", "#3ccf7a", "#ff5c5c", "#c58cff", "#5ee0e6"];

  // Cached plan of the log axis (splits, labels, decades) for the current range/width.
  let plan: LogAxisPlan = { splits: [], labels: [], decades: [] };
  let planKey = "";
  function currentPlan(u: uPlot): LogAxisPlan {
    const min = u.scales.x.min ?? 1;
    const max = u.scales.x.max ?? 10;
    const width = u.bbox.width / (window.devicePixelRatio || 1);
    const key = `${min}|${max}|${Math.round(width)}`;
    if (key !== planKey) {
      plan = logAxisPlan(min, max, width);
      planKey = key;
    }
    return plan;
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

  /** Pixel X (canvas space) of every X value of the data. */
  function centresPx(u: uPlot): number[] {
    return (u.data[0] as number[]).map((x) => u.valToPos(x, "x", true));
  }

  /**
   * Grouped bars: for series `sidx`, one rectangle per band from the bottom of the plot up
   * to the value, the channels of a band side by side. Explicit geometry (lib/bars.ts)
   * instead of uPlot's bar builder, whose baseline is the Y zero.
   */
  function groupedBars(u: uPlot, sidx: number): uPlot.Series.Paths {
    const centres = centresPx(u);
    const slot = slotWidth(centres, u.bbox.width);
    const bottom = u.bbox.top + u.bbox.height;
    const rects = channelBars(
      centres,
      u.data[sidx] as (number | null)[],
      sidx - 1,
      u.series.length - 1,
      slot,
      bottom,
      (v) => u.valToPos(v, "y", true),
    );
    const fill = new Path2D();
    for (const r of rects) fill.rect(r.x, r.y, r.w, r.h);
    // Same outline as fill; uPlot strokes and fills these with the series colours.
    return { stroke: fill, fill, clip: null };
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
        width: 1,
        show: !hidden[i],
        points: { show: false },
        paths: bars ? (u, sidx) => groupedBars(u, sidx) : undefined,
        fill: bars ? color + "88" : undefined,
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
          values: bars ? bandAxisValues : xLog ? (u: uPlot) => currentPlan(u).labels : undefined,
          splits: bars ? (u) => u.data[0] as number[] : xLog ? (u: uPlot) => currentPlan(u).splits : undefined,
        },
        {
          label: yLabel,
          stroke: "#8b929c",
          grid: { stroke: "#2e333b", width: 1 },
          ticks: { stroke: "#2e333b" },
          size: 60,
        },
        // Log mode only: a second X axis carrying no labels, just stronger decade lines.
        ...(xLog && !bars
          ? ([
              {
                scale: "x",
                size: 0,
                labelSize: 0,
                ticks: { show: false },
                grid: { stroke: "#3d444f", width: 1 },
                splits: (u: uPlot) => currentPlan(u).decades,
                values: (u: uPlot) => currentPlan(u).decades.map(() => ""),
              },
            ] as uPlot.Axis[])
          : []),
      ],
      hooks: {
        setCursor: [
          (u) => {
            if (!onCursor) return;
            // Leaving the graph keeps the last readout (the inspector shows it until cleared).
            const left = u.cursor.left;
            if (left == null || left < 0) return;
            // Snap to the data point (band or log point) nearest to the pointer, and read
            // the values from the data itself — never from the pixel position.
            const leftPx = left * (window.devicePixelRatio || 1);
            const idx = nearestSlot(centresPx(u), leftPx);
            if (idx < 0) return;
            const x = u.data[0][idx] as number;
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

  // Visibility toggles never rebuild the chart.
  $effect(() => {
    if (!plot) return;
    for (let i = 0; i < series.length; i++) {
      const show = !hidden[i];
      if (plot.series[i + 1] && plot.series[i + 1].show !== show) plot.setSeries(i + 1, { show });
    }
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

<div class="wrap">
  {#if series.length > 1}
    <div class="legend">
      {#each series as _s, i (i)}
        <button
          type="button"
          class="entry"
          class:off={hidden[i]}
          onclick={() => onToggleChannel?.(i)}
          title={hidden[i] ? "show" : "hide"}
        >
          <span class="swatch" style="background: {COLORS[i % COLORS.length]}"></span>
          {seriesNames[i] ?? `CH ${i + 1}`}
        </button>
      {/each}
    </div>
  {/if}
  <div class="graph" bind:this={host}></div>
</div>

<style>
  .wrap {
    position: relative;
    width: 100%;
    height: 100%;
  }
  .legend {
    position: absolute;
    top: 2px;
    right: 8px;
    z-index: 2;
    display: flex;
    gap: 8px;
  }
  .entry {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 2px 7px;
    font-size: 11px;
    font-family: inherit;
    color: var(--fg, #d6dae1);
    background: rgba(20, 23, 28, 0.75);
    border: 1px solid #2e333b;
    border-radius: 4px;
    cursor: pointer;
  }
  .entry.off {
    opacity: 0.45;
  }
  .entry.off .swatch {
    background: transparent !important;
    box-shadow: inset 0 0 0 1px #555;
  }
  .swatch {
    width: 10px;
    height: 10px;
    border-radius: 2px;
  }
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
