// Geometry of grouped bars (octave-band RTA): pure functions, no uPlot, unit-tested.
//
// Each band owns one slot on the ordinal X axis; the channels of a band are drawn side by
// side inside the slot, every bar rising from the bottom of the plot to its value.

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** Fraction of a slot covered by the group of bars (the rest is spacing between bands). */
export const GROUP_FILL = 0.8;
/** Gap between the bars of one group, as a fraction of the slot. */
export const BAR_GAP = 0.02;

/**
 * Width of one slot in pixels from the positions of the band centres. With a single band
 * the slot is the whole plot width.
 */
export function slotWidth(centresPx: ArrayLike<number>, plotWidthPx: number): number {
  const n = centresPx.length;
  if (n < 2) return plotWidthPx;
  let min = Infinity;
  for (let i = 1; i < n; i++) min = Math.min(min, Math.abs(centresPx[i] - centresPx[i - 1]));
  return Number.isFinite(min) && min > 0 ? min : plotWidthPx / n;
}

/** Horizontal extent (left, width) of bar `channel` of `channels` inside a slot centred on `cx`. */
export function barSpan(cx: number, slot: number, channel: number, channels: number): { left: number; width: number } {
  const n = Math.max(1, channels);
  const group = slot * GROUP_FILL;
  const gap = n > 1 ? slot * BAR_GAP : 0;
  const width = Math.max(1, (group - gap * (n - 1)) / n);
  const left = cx - group / 2 + channel * (width + gap);
  return { left, width };
}

/**
 * Rectangles of one channel's bars. `valueToY` maps a value to a pixel Y (growing
 * downwards); `bottomY` is the pixel Y of the plot bottom (Y-scale minimum). Values that
 * are null/NaN or below the bottom produce no rectangle.
 */
export function channelBars(
  centresPx: ArrayLike<number>,
  values: ArrayLike<number | null | undefined>,
  channel: number,
  channels: number,
  slot: number,
  bottomY: number,
  valueToY: (v: number) => number,
): Rect[] {
  const out: Rect[] = [];
  const n = Math.min(centresPx.length, values.length);
  for (let i = 0; i < n; i++) {
    const v = values[i];
    if (v == null || !Number.isFinite(v)) continue;
    const top = valueToY(v);
    if (!(top < bottomY)) continue;
    const { left, width } = barSpan(centresPx[i], slot, channel, channels);
    out.push({ x: left, y: top, w: width, h: bottomY - top });
  }
  return out;
}

/** Index of the slot whose centre is nearest to `xPx` (−1 when there are no slots). */
export function nearestSlot(centresPx: ArrayLike<number>, xPx: number): number {
  let best = -1;
  let dist = Infinity;
  for (let i = 0; i < centresPx.length; i++) {
    const d = Math.abs(centresPx[i] - xPx);
    if (d < dist) {
      dist = d;
      best = i;
    }
  }
  return best;
}
