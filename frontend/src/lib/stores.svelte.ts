// Application state (Svelte 5 runes). Nothing here computes audio data: every value is
// stored exactly as the backend sends it; axes come from StartStreamResponse.

import { create } from "@bufbuild/protobuf";
import {
  DeviceConfigSchema,
  MeasureKind,
  MeasureRequestSchema,
  RtaConfig_Averaging_Mode,
  RtaConfig_Window,
  RtaConfigSchema,
  ScopeConfig_Trigger_Mode,
  ScopeConfigSchema,
  StartStreamRequestSchema,
  StreamKind,
  type DeviceInfo,
  type MeasureResponse,
  type StartStreamResponse,
} from "../gen/anecho_pb";
import { Client, ServerError } from "./client";
import { generator } from "./generator.svelte";
import { channelValues, type Frame } from "./wire";

export const API_URL = "ws://127.0.0.1:4800/ws";

export type Tab = "levels" | "rta" | "scope";
export type ConnectionState = "disconnected" | "connecting" | "connected";

export interface ChannelLevel {
  rms: number;
  peak: number;
}

/** Plot-ready data of a stream: the axis sent once, then the latest frame per channel. */
export interface PlotData {
  axis: number[];
  series: Float32Array[];
  seq: bigint;
}

export interface CursorReadout {
  x: number;
  values: (number | null)[];
}

export const FFT_LENGTHS = [4096, 8192, 16384, 32768, 65536, 131072, 262144];
export const WINDOWS: { value: RtaConfig_Window; label: string }[] = [
  { value: RtaConfig_Window.HANN, label: "Hann" },
  { value: RtaConfig_Window.RECTANGULAR, label: "Rectangular" },
  { value: RtaConfig_Window.BLACKMAN_HARRIS_4, label: "Blackman-Harris 4" },
  { value: RtaConfig_Window.BLACKMAN_HARRIS_7, label: "Blackman-Harris 7" },
  { value: RtaConfig_Window.FLAT_TOP, label: "Flat-top" },
];
export const AVERAGING_MODES: { value: RtaConfig_Averaging_Mode; label: string }[] = [
  { value: RtaConfig_Averaging_Mode.UNSPECIFIED, label: "None" },
  { value: RtaConfig_Averaging_Mode.EXPONENTIAL, label: "Exponential" },
  { value: RtaConfig_Averaging_Mode.LINEAR, label: "Linear" },
  { value: RtaConfig_Averaging_Mode.PEAK_HOLD, label: "Peak hold" },
];
export const OCTAVE_FRACTIONS = [1, 3, 6, 12, 24];

export function windowLabel(w: RtaConfig_Window): string {
  return WINDOWS.find((x) => x.value === w)?.label ?? "default";
}

class RtaSettings {
  fftLength = $state(16384);
  window = $state<RtaConfig_Window>(RtaConfig_Window.HANN);
  averagingMode = $state<RtaConfig_Averaging_Mode>(RtaConfig_Averaging_Mode.EXPONENTIAL);
  averagingCount = $state(8);
  display = $state<"log" | "octave">("log");
  points = $state(1000);
  octaveFraction = $state(3);
  minHz = $state(20);
  maxHz = $state(20000);
  updateRateHz = $state(20);
}

class ScopeSettings {
  windowFrames = $state(2048);
  points = $state(1024);
  triggerMode = $state<ScopeConfig_Trigger_Mode>(ScopeConfig_Trigger_Mode.RISING);
  triggerLevel = $state(0);
  triggerChannel = $state(0);
}

class MeasureSettings {
  fftLength = $state(65536);
  window = $state<RtaConfig_Window>(RtaConfig_Window.BLACKMAN_HARRIS_7);
  averages = $state(4);
  maxHarmonic = $state(9);
  busy = $state(false);
  result = $state<MeasureResponse | null>(null);
  kind = $state<MeasureKind>(MeasureKind.THD);
}

class AppState {
  connection = $state<ConnectionState>("disconnected");
  backendVersion = $state("");
  error = $state("");
  devices = $state<DeviceInfo[]>([]);
  selectedDeviceId = $state("");
  sampleRate = $state(48000);
  inputRange = $state<number | undefined>(undefined);
  outputRange = $state<number | undefined>(undefined);
  autoRangeInput = $state(false);
  sessionId = $state<bigint | null>(null);
  /** Firmware version of the opened device (known once the session is open). */
  firmwareVersion = $state("");
  stream = $state<StartStreamResponse | null>(null);
  tab = $state<Tab>("levels");
  levels = $state<ChannelLevel[]>([]);
  rtaData = $state<PlotData | null>(null);
  scopeData = $state<PlotData | null>(null);
  unit = $state("dBFS");
  overruns = $state(0);
  rangeChanges = $state(0);
  cursor = $state<CursorReadout | null>(null);
  /** True while a running stream is stopped and started again after a control change. */
  restarting = $state(false);
  rta = new RtaSettings();
  scope = new ScopeSettings();
  measure = new MeasureSettings();

  private client: Client | null = null;
  private restartTimer: ReturnType<typeof setTimeout> | null = null;

  get selectedDevice(): DeviceInfo | undefined {
    return this.devices.find((d) => d.id === this.selectedDeviceId);
  }

  get running(): boolean {
    return this.stream !== null;
  }

  get calibrated(): boolean {
    return this.selectedDevice?.factoryCalibrated ?? false;
  }

  async connect() {
    if (this.connection !== "disconnected") return;
    this.connection = "connecting";
    this.error = "";
    try {
      const c = await Client.connect(API_URL);
      this.client = c;
      c.onClose(() => {
        this.connection = "disconnected";
        this.client = null;
        this.sessionId = null;
        this.firmwareVersion = "";
        this.clearStream();
      });
      c.onFrame((f) => this.onFrame(f));
      c.onEvent((e) => {
        switch (e.kind.case) {
          case "streamOverrun":
            this.overruns += e.kind.value.droppedBlocks;
            break;
          case "deviceLost":
            this.error = `device lost: ${e.kind.value.deviceId}`;
            break;
          case "rangeChanged": {
            const r = e.kind.value;
            if (r.sessionId !== this.sessionId) break;
            if (r.inputRange !== undefined) this.inputRange = r.inputRange;
            if (r.outputRange !== undefined) this.outputRange = r.outputRange;
            this.rangeChanges += 1;
            break;
          }
          case "deviceListChanged":
            this.refreshDevices();
            break;
        }
      });
      this.backendVersion = (await c.version()).backendVersion;
      this.connection = "connected";
      await this.refreshDevices();
    } catch (e) {
      this.connection = "disconnected";
      this.error = describe(e);
    }
  }

  async refreshDevices() {
    if (!this.client) return;
    try {
      this.devices = await this.client.listDevices();
      if (!this.selectedDevice && this.devices.length > 0) this.selectDevice(this.devices[0].id);
    } catch (e) {
      this.error = describe(e);
    }
  }

  selectDevice(id: string) {
    this.selectedDeviceId = id;
    const d = this.selectedDevice;
    if (!d) return;
    if (!d.sampleRates.includes(this.sampleRate)) this.sampleRate = d.sampleRates[0] ?? 48000;
    // Safe defaults: widest input range, lowest output range.
    this.inputRange = d.inputRanges.length > 0 ? d.inputRanges.length - 1 : undefined;
    this.outputRange = d.outputRanges.length > 0 ? 0 : undefined;
    if (!d.factoryCalibrated) generator.levelUnit = "dbfs";
  }

  /** Switch tab; when a stream is running, restart it with the new kind (one stream per session). */
  async selectTab(tab: Tab) {
    if (tab === this.tab) return;
    const wasRunning = this.running;
    if (wasRunning) await this.stopStream();
    this.tab = tab;
    if (wasRunning) await this.startStream();
  }

  /**
   * A stream parameter (RTA/scope controls, generator) changed while streaming: restart
   * the stream with the new request, debounced so typing a number does not spam the
   * backend. Session settings (sample rate, ranges) still need an explicit stop.
   */
  scheduleRestart() {
    if (!this.running && !this.restarting) return;
    if (this.restartTimer) clearTimeout(this.restartTimer);
    this.restarting = true;
    this.restartTimer = setTimeout(() => {
      this.restartTimer = null;
      void this.restartNow();
    }, 250);
  }

  private async restartNow() {
    try {
      await this.stopStream();
      await this.startStream();
    } finally {
      this.restarting = false;
    }
  }

  /** Every request parameter as one string; the tabs watch it to trigger restarts. */
  get streamSignature(): string {
    const r = this.rta;
    const sc = this.scope;
    return JSON.stringify([
      r.fftLength,
      r.window,
      r.averagingMode,
      r.averagingCount,
      r.display,
      r.points,
      r.octaveFraction,
      r.minHz,
      r.maxHz,
      r.updateRateHz,
      sc.windowFrames,
      sc.points,
      sc.triggerMode,
      sc.triggerLevel,
      sc.triggerChannel,
      generator.signature,
    ]);
  }

  private async ensureSession(): Promise<bigint | null> {
    const c = this.client;
    const d = this.selectedDevice;
    if (!c || !d) return null;
    if (this.sessionId !== null) return this.sessionId;
    const config = create(DeviceConfigSchema, {
      sampleRate: this.sampleRate,
      inputRange: this.inputRange,
      outputRange: this.outputRange,
      autoRangeInput: this.autoRangeInput ? true : undefined,
    });
    const session = await c.openSession(d.id, config);
    this.sessionId = session.sessionId;
    this.firmwareVersion = session.device?.firmwareVersion ?? "";
    return session.sessionId;
  }

  /** Start the stream of the current tab (opening the session if needed). */
  async start() {
    await this.startStream();
  }

  private async startStream() {
    const c = this.client;
    if (!c || this.running) return;
    this.error = "";
    this.overruns = 0;
    try {
      const sessionId = await this.ensureSession();
      if (sessionId === null) return;
      const gen = generator.message(this.calibrated);
      const kind =
        this.tab === "rta" ? StreamKind.RTA : this.tab === "scope" ? StreamKind.SCOPE : StreamKind.LEVELS;
      const req = create(StartStreamRequestSchema, {
        sessionId,
        kind,
        blockFrames: 0,
        levelsRateHz: 20,
        generator: gen,
        rta:
          kind === StreamKind.RTA
            ? create(RtaConfigSchema, {
                fftLength: this.rta.fftLength,
                window: this.rta.window,
                averaging: { mode: this.rta.averagingMode, count: this.rta.averagingCount },
                minHz: this.rta.minHz,
                maxHz: this.rta.maxHz,
                points: this.rta.display === "log" ? this.rta.points : 0,
                octaveFraction: this.rta.display === "octave" ? this.rta.octaveFraction : 0,
                updateRateHz: this.rta.updateRateHz,
              })
            : undefined,
        scope:
          kind === StreamKind.SCOPE
            ? create(ScopeConfigSchema, {
                windowFrames: this.scope.windowFrames,
                points: this.scope.points,
                trigger: {
                  mode: this.scope.triggerMode,
                  level: this.scope.triggerLevel,
                  channel: this.scope.triggerChannel,
                },
              })
            : undefined,
      });
      const stream = await c.startStream(req);
      this.error = "";
      this.stream = stream;
      this.unit = stream.scale?.unit.case === "dbvOffset" ? "dBV" : "dBFS";
      this.levels = Array.from({ length: stream.channels }, () => ({ rms: -200, peak: -200 }));
      const empty = () =>
        Array.from({ length: stream.channels }, () => new Float32Array(stream.valuesPerChannel));
      if (kind === StreamKind.RTA) this.rtaData = { axis: stream.axisHz, series: empty(), seq: -1n };
      if (kind === StreamKind.SCOPE) this.scopeData = { axis: stream.axisSeconds, series: empty(), seq: -1n };
    } catch (e) {
      this.error = describe(e);
      await this.stop();
    }
  }

  private async stopStream() {
    const c = this.client;
    if (!c) return;
    try {
      if (this.stream) await c.stopStream(this.stream.streamId);
    } catch (e) {
      this.error = describe(e);
    } finally {
      this.clearStream();
    }
  }

  /** Stop the stream and close the session. */
  async stop() {
    const c = this.client;
    if (!c) return;
    await this.stopStream();
    try {
      if (this.sessionId !== null) await c.closeSession(this.sessionId);
    } catch (e) {
      this.error = describe(e);
    } finally {
      this.sessionId = null;
    }
  }

  /** One-shot distortion measurement on the current device (no stream may be running). */
  async runMeasure(kind: MeasureKind) {
    const c = this.client;
    if (!c || this.measure.busy) return;
    if (this.running) {
      this.error = "stop the stream before measuring";
      return;
    }
    this.measure.busy = true;
    this.measure.kind = kind;
    this.error = "";
    try {
      const sessionId = await this.ensureSession();
      if (sessionId === null) return;
      const req = create(MeasureRequestSchema, {
        sessionId,
        kind,
        generator: generator.message(this.calibrated),
        fftLength: this.measure.fftLength,
        window: this.measure.window,
        averages: this.measure.averages,
        maxHarmonic: this.measure.maxHarmonic,
      });
      this.measure.result = await c.measure(req);
      this.error = "";
      this.unit = this.measure.result.scale?.unit.case === "dbvOffset" ? "dBV" : "dBFS";
    } catch (e) {
      this.error = describe(e);
    } finally {
      this.measure.busy = false;
    }
  }

  private clearStream() {
    this.stream = null;
    this.levels = [];
    this.rtaData = null;
    this.scopeData = null;
  }

  clearCursor() {
    this.cursor = null;
  }

  private onFrame(f: Frame) {
    const s = this.stream;
    if (!s || f.streamId !== s.streamId) return;
    switch (s.kind) {
      case StreamKind.LEVELS: {
        const next: ChannelLevel[] = [];
        for (let ch = 0; ch < f.channels; ch++) {
          const v = channelValues(f, ch);
          next.push({ rms: v[0], peak: v[1] });
        }
        this.levels = next;
        break;
      }
      case StreamKind.RTA:
        if (this.rtaData) this.rtaData = { ...this.rtaData, series: split(f), seq: f.seq };
        break;
      case StreamKind.SCOPE:
        if (this.scopeData) this.scopeData = { ...this.scopeData, series: split(f), seq: f.seq };
        break;
    }
  }
}

function split(f: Frame): Float32Array[] {
  const out: Float32Array[] = [];
  for (let ch = 0; ch < f.channels; ch++) out.push(channelValues(f, ch));
  return out;
}

function describe(e: unknown): string {
  if (e instanceof ServerError) return e.message;
  if (e instanceof Error) return e.message;
  return String(e);
}

export const app = new AppState();
export { generator };
