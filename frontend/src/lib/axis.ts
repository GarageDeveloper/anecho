// Log-frequency axis planning: pure functions, no uPlot, unit-tested.
//
// REW-style density: gridlines at every integer mantissa (2, 3, ... 9) of every decade,
// labels on the classic 1-2-3-4-5-7 sequence, automatically thinned to 1-2-5 and then to
// decades only when the labels would collide.

/** Compact frequency label: 20, 315, 1k, 1.25k, 12.5k. */
export function hzLabel(v: number): string {
  if (v >= 1000) {
    const k = v / 1000;
    const txt = Number.isInteger(k) ? `${k}` : k.toFixed(2).replace(/\.?0+$/, "");
    return `${txt}k`;
  }
  return Number.isInteger(v) ? `${v}` : v.toFixed(1).replace(/\.0$/, "");
}

/** Mantissa of `v` in [1, 10) — 2000 → 2, 50 → 5 — with a tolerance for float noise. */
export function mantissaOf(v: number): number {
  const m = v / Math.pow(10, Math.floor(Math.log10(v) + 1e-9));
  const r = Math.round(m);
  return Math.abs(m - r) < 1e-6 ? r : m;
}

/**
 * Every gridline position of the log axis: integer mantissas 1..9 of each decade
 * intersecting [minHz, maxHz] (bounds included with a small tolerance).
 */
export function logSplits(minHz: number, maxHz: number): number[] {
  if (!(minHz > 0) || !(maxHz > minHz)) return [];
  const out: number[] = [];
  const d0 = Math.floor(Math.log10(minHz) + 1e-9);
  const d1 = Math.ceil(Math.log10(maxHz) - 1e-9);
  for (let d = d0; d <= d1; d++) {
    const decade = Math.pow(10, d);
    for (let m = 1; m <= 9; m++) {
      const v = m * decade;
      if (v >= minHz * (1 - 1e-9) && v <= maxHz * (1 + 1e-9)) out.push(v);
    }
  }
  return out;
}

/** Labelled mantissas per density level: 0 = 1-2-3-4-5-7, 1 = 1-2-5, 2 = decades only. */
export const LABEL_LEVELS: ReadonlyArray<ReadonlyArray<number>> = [
  [1, 2, 3, 4, 5, 7],
  [1, 2, 5],
  [1],
];

export interface LogAxisPlan {
  /** Every gridline position (minor and labelled). */
  splits: number[];
  /** One label per split; `""` for unlabelled gridlines. */
  labels: string[];
  /** Decade positions (10^n) — drawn with a stronger gridline. */
  decades: number[];
}

/**
 * Plan the log axis for a plot of `widthPx` CSS pixels: densest label level whose labels
 * fit (each label budgeted `labelPx` pixels).
 */
export function logAxisPlan(minHz: number, maxHz: number, widthPx: number, labelPx = 34): LogAxisPlan {
  const splits = logSplits(minHz, maxHz);
  let chosen: ReadonlyArray<number> = LABEL_LEVELS[LABEL_LEVELS.length - 1];
  for (const level of LABEL_LEVELS) {
    const count = splits.filter((v) => level.includes(mantissaOf(v))).length;
    if (count * labelPx <= widthPx) {
      chosen = level;
      break;
    }
  }
  const labels = splits.map((v) => (chosen.includes(mantissaOf(v)) ? hzLabel(v) : ""));
  const decades = splits.filter((v) => mantissaOf(v) === 1);
  return { splits, labels, decades };
}

// ---------------------------------------------------------------------------------------
// X-axis model: which treatment a graph uses. A pure decision, unit-tested, so the
// log-points and octave-band modes cannot silently swap again (the t12 regression applied
// the log plan to the band mode).
// ---------------------------------------------------------------------------------------

export type XAxisModel =
  | { kind: "linear" }
  | { kind: "log" }
  | { kind: "bands"; indices: number[] };

/**
 * Octave bands always win: band centres are positioned by index on a plain linear scale
 * (one even slot per band) — never through a log mapping, and never through uPlot's
 * ordinal `distr: 2`, whose custom splits and scale bounds are indices into `data[0]`,
 * not values.
 */
export function xAxisModel(xLog: boolean, bars: boolean, pointCount: number): XAxisModel {
  if (bars) return { kind: "bands", indices: Array.from({ length: pointCount }, (_, i) => i) };
  return xLog ? { kind: "log" } : { kind: "linear" };
}

/** Label-thinning step for band labels given the pixel width of one band slot. */
export function bandLabelStep(slotPx: number): number {
  return slotPx >= 44 ? 1 : slotPx >= 24 ? 2 : slotPx >= 14 ? 3 : 6;
}

/** One label per band centre, thinned to every `bandLabelStep`-th so they never collide. */
export function bandLabels(centres: ArrayLike<number>, widthPx: number): string[] {
  const n = centres.length;
  const step = bandLabelStep(widthPx / Math.max(1, n));
  const out: string[] = [];
  for (let i = 0; i < n; i++) out.push(i % step === 0 ? hzLabel(centres[i]) : "");
  return out;
}
