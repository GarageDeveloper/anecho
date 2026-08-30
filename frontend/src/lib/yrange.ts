// Scope Y-scale helpers: pure display scaling (no data processing), unit-tested.
//
// The scope plots raw dBFS-scaled samples (±1.0 full scale). A quiet signal — e.g. a
// −10 dBV sine captured on the 42 dBV input range — is a few thousandths of full scale,
// invisible on a fixed ±1 axis. Auto mode fits the axis to the signal with a margin and
// hysteresis so it neither hides the waveform nor flickers on every frame.

/** Fixed half-range choices offered next to Auto. */
export const FIXED_Y_RANGES: ReadonlyArray<number> = [1, 0.3, 0.1, 0.03, 0.01, 0.003, 0.001];

/** Smallest half-range the auto scale ever uses. */
export const AUTO_FLOOR = 1e-4;

/** Auto rescales only when the peak leaves this fraction band of the current range. */
export const HYSTERESIS_LOW = 0.4;
export const HYSTERESIS_HIGH = 0.95;

/** Margin applied when picking a new range: the peak lands at ≤ 1/1.15 of the range. */
export const MARGIN = 1.15;

/** Smallest value of the 1-3 sequence (… 1e-4, 3e-4, 1e-3, 3e-3, 1e-2 …) ≥ `v`. */
export function snap13(v: number): number {
  if (!Number.isFinite(v) || v <= 0) return AUTO_FLOOR;
  const d = Math.floor(Math.log10(v));
  // Build candidates from decimal strings so 3 × 10⁻¹ is exactly 0.3, not 0.30000…04.
  for (const m of [1, 3]) {
    const c = Number(`${m}e${d}`);
    if (c >= v * (1 - 1e-9)) return c;
  }
  return Number(`1e${d + 1}`);
}

/**
 * Auto half-range for a window whose absolute peak is `peak`, given the `current`
 * half-range. Symmetric around 0. Keeps `current` while the peak stays inside
 * [HYSTERESIS_LOW, HYSTERESIS_HIGH] × current; otherwise snaps `peak × MARGIN` to the
 * 1-3 sequence, never below AUTO_FLOOR and never above 1.
 */
export function autoHalfRange(peak: number, current: number | null): number {
  const cur = current != null && Number.isFinite(current) && current > 0 ? current : null;
  if (!Number.isFinite(peak) || peak <= 0) return cur ?? AUTO_FLOOR;
  if (cur != null && peak >= HYSTERESIS_LOW * cur && peak <= HYSTERESIS_HIGH * cur) return cur;
  return Math.min(1, Math.max(AUTO_FLOOR, snap13(peak * MARGIN)));
}

/** `v` with `n` significant digits, trailing zeros trimmed: 0.002451, 0.35, 1. */
export function sigFigs(v: number, n = 4): string {
  if (!Number.isFinite(v)) return "—";
  if (v === 0) return "0";
  const s = v.toPrecision(n);
  // toPrecision may emit exponent notation for tiny values; expand it.
  const plain = Number(s).toString();
  return plain.includes("e") ? Number(s).toFixed(Math.max(0, n - 1 - Math.floor(Math.log10(Math.abs(v))))) : plain;
}

/** Engineering-style amplitude tick label: 0.3, 2m, 500µ. */
export function amplitudeLabel(v: number): string {
  if (!Number.isFinite(v)) return "";
  if (v === 0) return "0";
  const a = Math.abs(v);
  const sign = v < 0 ? "-" : "";
  const trim = (x: number) => {
    const r = Math.round(x * 1000) / 1000;
    return `${r}`.replace(/\.?0+$/, "");
  };
  if (a >= 1e-2) return sign + trim(a);
  if (a >= 1e-5) return `${sign}${trim(a * 1e3)}m`;
  return `${sign}${trim(a * 1e6)}µ`;
}
