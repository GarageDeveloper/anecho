// Application state (Svelte 5 runes). Nothing here computes audio data: level values are
// stored exactly as the backend sends them.

import { create } from "@bufbuild/protobuf";
import {
  DeviceConfigSchema,
  StartStreamRequestSchema,
  StreamKind,
  type DeviceInfo,
  type StartStreamResponse,
} from "../gen/anecho_pb";
import { Client, ServerError } from "./client";
import { channelValues, type Frame } from "./wire";

export const API_URL = "ws://127.0.0.1:4800/ws";

export interface ChannelLevel {
  rms: number;
  peak: number;
}

export type ConnectionState = "disconnected" | "connecting" | "connected";

class AppState {
  connection = $state<ConnectionState>("disconnected");
  backendVersion = $state("");
  error = $state("");
  devices = $state<DeviceInfo[]>([]);
  selectedDeviceId = $state("");
  sampleRate = $state(48000);
  inputRange = $state<number | undefined>(undefined);
  outputRange = $state<number | undefined>(undefined);
  sessionId = $state<bigint | null>(null);
  stream = $state<StartStreamResponse | null>(null);
  levels = $state<ChannelLevel[]>([]);
  unit = $state("dBFS");
  overruns = $state(0);
  generatorOn = $state(false);
  generatorHz = $state(1000);
  generatorDbfs = $state(-20);

  private client: Client | null = null;

  get selectedDevice(): DeviceInfo | undefined {
    return this.devices.find((d) => d.id === this.selectedDeviceId);
  }

  get running(): boolean {
    return this.stream !== null;
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
        this.stream = null;
        this.levels = [];
      });
      c.onFrame((f) => this.onFrame(f));
      c.onEvent((e) => {
        if (e.kind.case === "streamOverrun") this.overruns += e.kind.value.droppedBlocks;
        if (e.kind.case === "deviceLost") this.error = `device lost: ${e.kind.value.deviceId}`;
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
  }

  async start() {
    const c = this.client;
    const d = this.selectedDevice;
    if (!c || !d || this.running) return;
    this.error = "";
    this.overruns = 0;
    try {
      const config = create(DeviceConfigSchema, {
        sampleRate: this.sampleRate,
        inputRange: this.inputRange,
        outputRange: this.outputRange,
      });
      const session = await c.openSession(d.id, config);
      this.sessionId = session.sessionId;
      const req = create(StartStreamRequestSchema, {
        sessionId: session.sessionId,
        kind: StreamKind.LEVELS,
        blockFrames: 0,
        levelsRateHz: 20,
        generator: this.generatorOn
          ? {
              signal: {
                case: "sine",
                value: { frequencyHz: this.generatorHz, amplitudeDbfs: this.generatorDbfs },
              },
            }
          : undefined,
      });
      const stream = await c.startStream(req);
      this.stream = stream;
      this.unit = stream.scale?.unit.case === "dbvOffset" ? "dBV" : "dBFS";
      this.levels = Array.from({ length: stream.channels }, () => ({ rms: -200, peak: -200 }));
    } catch (e) {
      this.error = describe(e);
      await this.stop();
    }
  }

  async stop() {
    const c = this.client;
    if (!c) return;
    try {
      if (this.stream) await c.stopStream(this.stream.streamId);
      if (this.sessionId !== null) await c.closeSession(this.sessionId);
    } catch (e) {
      this.error = describe(e);
    } finally {
      this.stream = null;
      this.sessionId = null;
      this.levels = [];
    }
  }

  private onFrame(f: Frame) {
    if (!this.stream || f.streamId !== this.stream.streamId) return;
    const next: ChannelLevel[] = [];
    for (let ch = 0; ch < f.channels; ch++) {
      const v = channelValues(f, ch);
      next.push({ rms: v[0], peak: v[1] });
    }
    this.levels = next;
  }
}

function describe(e: unknown): string {
  if (e instanceof ServerError) return e.message;
  if (e instanceof Error) return e.message;
  return String(e);
}

export const app = new AppState();
