// Generator settings (runes) and their mapping to the contract's `Generator` message.
// The frontend only describes the signal; every sample is rendered by the backend.

import { create } from "@bufbuild/protobuf";
import { Generator_NoiseKind, GeneratorSchema, type Generator } from "../gen/anecho_pb";

export type SignalKind = "sine" | "dualTone" | "multitone" | "noise" | "square";
export type LevelUnit = "dbfs" | "dbv";

export const DUAL_TONE_PRESETS = {
  smpte: { label: "SMPTE 60 Hz + 7 kHz (4:1)", f1: 60, f2: 7000, ratioDb: 12.04 },
  ccif: { label: "CCIF 19 kHz + 20 kHz (1:1)", f1: 19000, f2: 20000, ratioDb: 0 },
} as const;

export class GeneratorState {
  enabled = $state(false);
  kind = $state<SignalKind>("sine");
  sineHz = $state(1000);
  dual = $state({ f1: 60, f2: 7000, ratioDb: 12.04 });
  multitone = $state("100, 1000, 10000");
  schroeder = $state(true);
  noiseKind = $state<"white" | "pink">("pink");
  periodic = $state(false);
  periodFrames = $state(65536);
  squareHz = $state(1000);
  levelUnit = $state<LevelUnit>("dbfs");
  levelDbfs = $state(-20);
  levelDbv = $state(-10);
  /** Output channels to drive; empty = all. */
  outputChannels = $state<number[]>([]);

  applyDualPreset(name: keyof typeof DUAL_TONE_PRESETS) {
    const p = DUAL_TONE_PRESETS[name];
    this.dual = { f1: p.f1, f2: p.f2, ratioDb: p.ratioDb };
  }

  /** Frequencies of the multitone list, in Hz (invalid entries are dropped). */
  get multitoneHz(): number[] {
    return this.multitone
      .split(/[,;\s]+/)
      .map((t) => Number(t))
      .filter((f) => Number.isFinite(f) && f > 0);
  }

  /** Everything that changes the contract message, as one string (restart trigger). */
  get signature(): string {
    return JSON.stringify([
      this.enabled,
      this.kind,
      this.sineHz,
      this.dual,
      this.multitone,
      this.schroeder,
      this.noiseKind,
      this.periodic,
      this.periodFrames,
      this.squareHz,
      this.levelUnit,
      this.levelDbfs,
      this.levelDbv,
      this.outputChannels,
    ]);
  }

  /** A short description for the inspector / status bar. */
  get summary(): string {
    if (!this.enabled) return "off";
    const level = this.levelUnit === "dbv" ? `${this.levelDbv} dBV` : `${this.levelDbfs} dBFS`;
    switch (this.kind) {
      case "sine":
        return `sine ${this.sineHz} Hz @ ${level}`;
      case "dualTone":
        return `${this.dual.f1} + ${this.dual.f2} Hz @ ${level}`;
      case "multitone":
        return `multitone ×${this.multitoneHz.length} @ ${level}`;
      case "noise":
        return `${this.noiseKind} noise${this.periodic ? " (periodic)" : ""} @ ${level}`;
      case "square":
        return `square ${this.squareHz} Hz @ ${level}`;
    }
  }

  /** The contract message, or undefined when the generator is off. */
  message(calibrated: boolean): Generator | undefined {
    if (!this.enabled) return undefined;
    const useDbv = this.levelUnit === "dbv" && calibrated;
    const level = useDbv
      ? { unit: { case: "dbvRms" as const, value: this.levelDbv } }
      : { unit: { case: "peakDbfs" as const, value: this.levelDbfs } };
    let signal: Generator["signal"];
    switch (this.kind) {
      case "sine":
        signal = {
          case: "sine",
          value: { frequencyHz: this.sineHz, amplitudeDbfs: this.levelDbfs },
        } as Generator["signal"];
        break;
      case "dualTone":
        signal = {
          case: "dualTone",
          value: { f1Hz: this.dual.f1, f2Hz: this.dual.f2, ratioDb: this.dual.ratioDb },
        } as Generator["signal"];
        break;
      case "multitone":
        signal = {
          case: "multitone",
          value: { frequenciesHz: this.multitoneHz, schroederPhases: this.schroeder },
        } as Generator["signal"];
        break;
      case "noise":
        signal = {
          case: "noise",
          value: {
            kind: this.noiseKind === "pink" ? Generator_NoiseKind.PINK : Generator_NoiseKind.WHITE,
            periodFrames: this.periodic ? this.periodFrames : 0,
            seed: 0,
          },
        } as Generator["signal"];
        break;
      case "square":
        signal = { case: "square", value: { frequencyHz: this.squareHz } } as Generator["signal"];
        break;
    }
    return create(GeneratorSchema, {
      signal,
      level,
      outputChannels: this.outputChannels,
    });
  }
}

export const generator = new GeneratorState();
