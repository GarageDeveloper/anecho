import { describe, expect, it } from "vitest";
import { barSpan, channelBars, nearestSlot, slotWidth } from "./bars";

describe("grouped octave bars", () => {
  const centres = [10, 30, 50, 70]; // 4 slots of 20 px
  const bottom = 100;
  const valueToY = (v: number) => bottom - v; // 1 px per unit, 0 at the bottom

  it("derives the slot width from adjacent centres", () => {
    expect(slotWidth(centres, 400)).toBe(20);
    expect(slotWidth([33], 400)).toBe(400);
  });

  it("places two channels side by side inside a slot without overlap", () => {
    const a = barSpan(30, 20, 0, 2);
    const b = barSpan(30, 20, 1, 2);
    expect(a.left).toBeLessThan(b.left);
    expect(a.left + a.width).toBeLessThanOrEqual(b.left);
    // Group centred on the slot centre.
    expect((a.left + b.left + b.width) / 2).toBeCloseTo(30, 6);
    // Group covers 80 % of the slot.
    expect(b.left + b.width - a.left).toBeCloseTo(16, 6);
  });

  it("a single channel fills the group width", () => {
    const s = barSpan(30, 20, 0, 1);
    expect(s.width).toBeCloseTo(16, 6);
    expect(s.left).toBeCloseTo(22, 6);
  });

  it("bars rise from the bottom of the plot to their value", () => {
    const rects = channelBars(centres, [40, null, 0, 120], 0, 1, 20, bottom, valueToY);
    // null and the value at/below the bottom (0 → y = bottom) draw nothing.
    expect(rects.map((r) => r.h)).toEqual([40, 120]);
    expect(rects[0]).toMatchObject({ y: 60, h: 40 });
    expect(rects[0].y + rects[0].h).toBe(bottom);
  });

  it("snaps the cursor to the nearest slot centre", () => {
    expect(nearestSlot(centres, 0)).toBe(0);
    expect(nearestSlot(centres, 41)).toBe(2);
    expect(nearestSlot(centres, 39)).toBe(1);
    expect(nearestSlot([], 5)).toBe(-1);
  });
});
