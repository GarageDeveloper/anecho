import { describe, expect, it } from "vitest";
import {
  AUTO_FLOOR,
  amplitudeLabel,
  autoHalfRange,
  sigFigs,
  snap13,
} from "./yrange";

describe("snap13", () => {
  it("snaps up to the 1-3 sequence", () => {
    expect(snap13(0.002)).toBe(0.003);
    expect(snap13(0.004)).toBe(0.01);
    expect(snap13(0.3)).toBe(0.3);
    expect(snap13(0.31)).toBe(1);
    expect(snap13(1)).toBe(1);
  });
  it("floors invalid input", () => {
    expect(snap13(0)).toBe(AUTO_FLOOR);
    expect(snap13(NaN)).toBe(AUTO_FLOOR);
  });
});

describe("autoHalfRange", () => {
  it("fits a small sine with margin", () => {
    // ~0.002 peak (a -10 dBV sine on the 42 dBV range): 0.002 * 1.15 -> 0.003
    expect(autoHalfRange(0.002, null)).toBe(0.003);
  });
  it("keeps the current range while the peak stays in the hysteresis band", () => {
    // peak at 66 % of the range: no change
    expect(autoHalfRange(0.002, 0.003)).toBe(0.003);
    // still above 40 %: no change
    expect(autoHalfRange(0.00125, 0.003)).toBe(0.003);
  });
  it("shrinks when the peak drops below 40 %", () => {
    expect(autoHalfRange(0.0005, 0.003)).toBe(0.001);
  });
  it("grows when the peak exceeds 95 %", () => {
    expect(autoHalfRange(0.0029, 0.003)).toBe(0.01);
  });
  it("never goes below the floor nor above 1", () => {
    expect(autoHalfRange(1e-9, null)).toBe(AUTO_FLOOR);
    expect(autoHalfRange(5, null)).toBe(1);
  });
  it("keeps the current range on silence", () => {
    expect(autoHalfRange(0, 0.01)).toBe(0.01);
    expect(autoHalfRange(NaN, 0.01)).toBe(0.01);
    expect(autoHalfRange(0, null)).toBe(AUTO_FLOOR);
  });
});

describe("formatting", () => {
  it("sigFigs keeps 4 significant digits and trims", () => {
    expect(sigFigs(0.0024514)).toBe("0.002451");
    expect(sigFigs(0.35)).toBe("0.35");
    expect(sigFigs(1)).toBe("1");
    expect(sigFigs(0)).toBe("0");
    expect(sigFigs(NaN)).toBe("—");
  });
  it("amplitudeLabel uses engineering suffixes", () => {
    expect(amplitudeLabel(0.3)).toBe("0.3");
    expect(amplitudeLabel(0.002)).toBe("2m");
    expect(amplitudeLabel(-0.002)).toBe("-2m");
    expect(amplitudeLabel(0.0005)).toBe("0.5m");
    expect(amplitudeLabel(0.000005)).toBe("5µ");
    expect(amplitudeLabel(0)).toBe("0");
  });
});
