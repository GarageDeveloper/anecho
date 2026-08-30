import { describe, expect, it } from "vitest";
import { bandLabels, hzLabel, logAxisPlan, logSplits, mantissaOf, xAxisModel } from "./axis";

describe("log axis plan", () => {
  it("formats compact frequency labels", () => {
    expect(hzLabel(20)).toBe("20");
    expect(hzLabel(700)).toBe("700");
    expect(hzLabel(2000)).toBe("2k");
    expect(hzLabel(12500)).toBe("12.5k");
  });

  it("splits at every integer mantissa of each decade inside the range", () => {
    const s = logSplits(20, 20000);
    expect(s[0]).toBe(20);
    expect(s[s.length - 1]).toBe(20000);
    // 20..90 (8), 100..900 (9), 1k..9k (9), 10k + 20k (2)
    expect(s).toHaveLength(28);
    expect(s).toContain(70);
    expect(s).toContain(6000);
    expect(s).not.toContain(10);
  });

  it("labels 1-2-3-4-5-7 when there is room, with minors left unlabelled", () => {
    const plan = logAxisPlan(20, 20000, 1200);
    const labelled = plan.splits.filter((_, i) => plan.labels[i] !== "");
    expect(labelled).toEqual([
      20, 30, 40, 50, 70, 100, 200, 300, 400, 500, 700, 1000, 2000, 3000, 4000, 5000, 7000, 10000, 20000,
    ]);
    // 60/80/90 & co stay as unlabelled gridlines.
    expect(plan.labels[plan.splits.indexOf(60)]).toBe("");
    expect(plan.labels[plan.splits.indexOf(9000)]).toBe("");
  });

  it("thins to 1-2-5 and then to decades when width shrinks", () => {
    const mid = logAxisPlan(20, 20000, 420);
    const midLabelled = mid.splits.filter((_, i) => mid.labels[i] !== "");
    expect(midLabelled).toEqual([20, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 20000]);
    const tiny = logAxisPlan(20, 20000, 140);
    const tinyLabelled = tiny.splits.filter((_, i) => tiny.labels[i] !== "");
    expect(tinyLabelled).toEqual([100, 1000, 10000]);
  });

  it("exposes decades for the stronger gridlines", () => {
    expect(logAxisPlan(20, 20000, 800).decades).toEqual([100, 1000, 10000]);
    expect(mantissaOf(20000)).toBe(2);
  });
});

describe("x-axis model", () => {
  it("octave bands always use the index scale, never the log plan", () => {
    const bands = xAxisModel(true, true, 29);
    expect(bands.kind).toBe("bands");
    if (bands.kind === "bands") {
      expect(bands.indices).toHaveLength(29);
      expect(bands.indices[0]).toBe(0);
      expect(bands.indices[28]).toBe(28);
    }
    expect(xAxisModel(true, false, 1000).kind).toBe("log");
    expect(xAxisModel(false, false, 512).kind).toBe("linear");
  });

  it("labels every band centre, thinned by slot width", () => {
    const centres = Array.from({ length: 29 }, (_, i) => 25 * Math.pow(2, i / 3));
    const wide = bandLabels(centres, 29 * 50);
    expect(wide.filter((l) => l !== "")).toHaveLength(29);
    expect(wide[0]).toBe("25");
    const mid = bandLabels(centres, 29 * 30);
    expect(mid.filter((l) => l !== "")).toHaveLength(15);
    const narrow = bandLabels(centres, 29 * 6);
    expect(narrow.filter((l) => l !== "")).toHaveLength(5);
  });
});
