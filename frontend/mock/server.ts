// Mock Anecho backend for frontend development without hardware.
//
// Speaks the contract (protobuf envelopes + binary frames) on ws://127.0.0.1:4800/ws with
// one fake, factory-calibrated stereo device. Streams are synthetic: a sine peak over a noise
// floor on the RTA axis, a sine on the scope, breathing level meters. Nothing here is
// meant to be numerically right — it exercises the UI's plumbing only.
//
// Run: `pnpm mock` (tsx), then `pnpm dev` in a browser.

import { WebSocketServer, type WebSocket } from "ws";
import { create, fromBinary, toBinary } from "@bufbuild/protobuf";
import {
  BackendKind,
  DistortionResultSchema,
  EnvelopeSchema,
  EventSchema,
  MeasureKind,
  MeasureResponseSchema,
  StreamKind,
  type Envelope,
  type ErrorCode,
  type Generator,
  type RtaConfig,
  type ScopeConfig,
} from "../src/gen/anecho_pb";

const PORT = Number(process.env.MOCK_PORT ?? 4800);
const SAMPLE_RATE = 48000;
const INPUT_RANGES = [0, 6, 12, 18, 24, 30, 36, 42];
const OUTPUT_RANGES = [-12, -2, 8, 18];

type Payload = Envelope["payload"];

interface Session {
  id: bigint;
  inputRange: number;
  outputRange: number;
  autoRange: boolean;
  stream: Stream | null;
}

interface Stream {
  id: number;
  kind: StreamKind;
  timer: NodeJS.Timeout;
}

let nextSession = 1n;
let nextStream = 1;
const sessions = new Map<bigint, Session>();

function dbvOffset(rangeDbv: number): number {
  // Pretend factory calibration: dBV = dBFS_rms + range + 3.75 (like a real QA40x).
  return rangeDbv + 3.75;
}

function generatorPeakDbfs(g: Generator | undefined, outputRange: number): { hz: number; dbfs: number } | null {
  if (!g || g.signal.case === undefined) return null;
  const hz =
    g.signal.case === "sine"
      ? g.signal.value.frequencyHz
      : g.signal.case === "square"
        ? g.signal.value.frequencyHz
        : g.signal.case === "dualTone"
          ? g.signal.value.f2Hz
          : g.signal.case === "multitone"
            ? (g.signal.value.frequenciesHz[0] ?? 1000)
            : 1000;
  let dbfs = g.signal.case === "sine" ? g.signal.value.amplitudeDbfs : -20;
  if (g.level?.unit.case === "peakDbfs") dbfs = g.level.unit.value;
  if (g.level?.unit.case === "dbvRms") dbfs = g.level.unit.value - dbvOffset(OUTPUT_RANGES[outputRange]) + 3.01;
  return { hz, dbfs };
}

function frame(streamId: number, seq: number, firstFrame: number, channels: number, perChannel: Float32Array[]): Buffer {
  const n = perChannel[0]?.length ?? 0;
  const buf = Buffer.alloc(24 + channels * n * 4);
  buf.writeUInt32LE(streamId, 0);
  buf.writeBigUInt64LE(BigInt(seq), 4);
  buf.writeBigUInt64LE(BigInt(firstFrame), 12);
  buf.writeUInt16LE(channels, 20);
  buf.writeUInt16LE(n, 22);
  let o = 24;
  for (let ch = 0; ch < channels; ch++) {
    for (let i = 0; i < n; i++) {
      buf.writeFloatLE(perChannel[ch][i], o);
      o += 4;
    }
  }
  return buf;
}

function logAxis(minHz: number, maxHz: number, points: number): number[] {
  const out: number[] = [];
  const r = Math.log(maxHz / minHz);
  for (let i = 0; i < points; i++) out.push(minHz * Math.exp((r * i) / (points - 1)));
  return out;
}

function octaveAxis(fraction: number, minHz: number, maxHz: number): number[] {
  const out: number[] = [];
  const step = Math.pow(2, 1 / fraction);
  let f = 1000;
  while (f / step >= minHz) f /= step;
  for (; f <= maxHz; f *= step) out.push(f);
  return out;
}

function startRta(ws: WebSocket, s: Session, cfg: RtaConfig | undefined, gen: Generator | undefined): { axis: number[]; stream: Stream } {
  const minHz = cfg?.minHz || 20;
  const maxHz = cfg?.maxHz || 20000;
  const axis = cfg?.octaveFraction ? octaveAxis(cfg.octaveFraction, minHz, maxHz) : logAxis(minHz, maxHz, cfg?.points || 1000);
  const id = nextStream++;
  const rate = cfg?.updateRateHz || 20;
  const tone = generatorPeakDbfs(gen, s.outputRange);
  const offset = dbvOffset(INPUT_RANGES[s.inputRange]);
  const loopbackGainDb = dbvOffset(OUTPUT_RANGES[s.outputRange]) - offset; // output dBV -> input dBFS
  let seq = 0;
  const timer = setInterval(() => {
    const series: Float32Array[] = [];
    for (let ch = 0; ch < 2; ch++) {
      const v = new Float32Array(axis.length);
      for (let i = 0; i < axis.length; i++) {
        const f = axis[i];
        let db = -110 - 10 * Math.log10(f / 1000) + (Math.random() - 0.5) * 6; // pinkish floor
        if (tone) {
          const d = Math.abs(Math.log2(f / tone.hz));
          const peak = tone.dbfs - 3.01 + loopbackGainDb;
          db = Math.max(db, peak - Math.min(80, 600 * d * d));
          for (let h = 2; h <= 5; h++) {
            const dh = Math.abs(Math.log2(f / (tone.hz * h)));
            db = Math.max(db, peak - 20 * h - Math.min(80, 600 * dh * dh));
          }
        }
        v[i] = db + offset; // dBV
      }
      series.push(v);
    }
    ws.send(frame(id, seq, seq * Math.round(SAMPLE_RATE / rate), 2, series));
    seq++;
  }, 1000 / rate);
  return { axis, stream: { id, kind: StreamKind.RTA, timer } };
}

function startScope(ws: WebSocket, s: Session, cfg: ScopeConfig | undefined, gen: Generator | undefined): { axis: number[]; stream: Stream } {
  const windowFrames = cfg?.windowFrames || 2048;
  const points = cfg?.points || windowFrames;
  const axis = Array.from({ length: points }, (_, i) => (i * (windowFrames / points)) / SAMPLE_RATE);
  const id = nextStream++;
  const tone = generatorPeakDbfs(gen, s.outputRange) ?? { hz: 1000, dbfs: -20 };
  const amp = Math.pow(10, tone.dbfs / 20);
  let seq = 0;
  const timer = setInterval(() => {
    const series: Float32Array[] = [];
    for (let ch = 0; ch < 2; ch++) {
      const v = new Float32Array(points);
      for (let i = 0; i < points; i++) v[i] = amp * Math.sin(2 * Math.PI * tone.hz * axis[i]) + (Math.random() - 0.5) * 0.002;
      series.push(v);
    }
    ws.send(frame(id, seq, seq * windowFrames, 2, series));
    seq++;
  }, 50);
  return { axis, stream: { id, kind: StreamKind.SCOPE, timer } };
}

function startLevels(ws: WebSocket, s: Session, rateHz: number, gen: Generator | undefined): Stream {
  const id = nextStream++;
  const rate = rateHz || 20;
  const tone = generatorPeakDbfs(gen, s.outputRange);
  let seq = 0;
  const timer = setInterval(() => {
    const series: Float32Array[] = [];
    for (let ch = 0; ch < 2; ch++) {
      const offset = dbvOffset(INPUT_RANGES[s.inputRange]);
      const peak = tone ? tone.dbfs + dbvOffset(OUTPUT_RANGES[s.outputRange]) - offset : -90 + Math.sin(seq / 7 + ch) * 10;
      const v = new Float32Array([peak - 3.01 + offset, peak + offset]);
      series.push(v);
    }
    ws.send(frame(id, seq, seq * Math.round(SAMPLE_RATE / rate), 2, series));
    seq++;
    // Auto range: step the input range down towards the signal and announce it.
    if (s.autoRange && tone && seq % 10 === 0 && s.inputRange > 1) {
      s.inputRange -= 1;
      sendEvent(ws, { case: "rangeChanged", value: { sessionId: s.id, inputRange: s.inputRange } });
    }
  }, 1000 / rate);
  return { id, kind: StreamKind.LEVELS, timer };
}

function stopStream(s: Session) {
  if (s.stream) clearInterval(s.stream.timer);
  s.stream = null;
}

function sendEvent(ws: WebSocket, kind: NonNullable<Parameters<typeof create<typeof EventSchema>>[1]>["kind"]) {
  const env = create(EnvelopeSchema, { requestId: 0n, payload: { case: "event", value: create(EventSchema, { kind }) } });
  ws.send(toBinary(EnvelopeSchema, env));
}

function error(code: ErrorCode, message: string): Payload {
  return { case: "error", value: { code, message, $typeName: "anecho.v0.Error" } };
}

function handle(ws: WebSocket, payload: Payload, owned: Set<bigint>): Payload {
  switch (payload.case) {
    case "getVersion":
      return { case: "version", value: { $typeName: "anecho.v0.GetVersionResponse", backendVersion: "mock", contractVersion: "v0" } };
    case "listDevices":
      return {
        case: "devices",
        value: {
          $typeName: "anecho.v0.ListDevicesResponse",
          devices: [
            {
              $typeName: "anecho.v0.DeviceInfo",
              id: "mock/qa403",
              displayName: "Mock QA403 (calibrated)",
              factoryCalibrated: true,
              sampleRates: [48000, 96000, 192000],
              inputChannels: 2,
              outputChannels: 2,
              backend: BackendKind.QA40X,
              transport: "mock",
              inputRanges: INPUT_RANGES.map((r) => ({ $typeName: "anecho.v0.Range", fullScaleDbv: r, label: `${r >= 0 ? "+" : ""}${r} dBV` })),
              outputRanges: OUTPUT_RANGES.map((r) => ({ $typeName: "anecho.v0.Range", fullScaleDbv: r, label: `${r >= 0 ? "+" : ""}${r} dBV` })),
              synchronousIo: true,
              nominalLatencyFrames: 46,
              // Real units report it once opened; the mock knows it upfront.
              firmwareVersion: "60",
            },
            {
              $typeName: "anecho.v0.DeviceInfo",
              id: "mock/soundcard",
              displayName: "Mock sound card",
              factoryCalibrated: false,
              sampleRates: [44100, 48000, 96000],
              inputChannels: 2,
              outputChannels: 2,
              backend: BackendKind.CPAL,
              transport: "mock",
              inputRanges: [],
              outputRanges: [],
              synchronousIo: false,
              firmwareVersion: "",
            },
          ],
        },
      };
    case "openSession": {
      const cfg = payload.value.config;
      const s: Session = {
        id: nextSession++,
        inputRange: cfg?.inputRange ?? INPUT_RANGES.length - 1,
        outputRange: cfg?.outputRange ?? 0,
        autoRange: cfg?.autoRangeInput ?? false,
        stream: null,
      };
      sessions.set(s.id, s);
      owned.add(s.id);
      return {
        case: "sessionOpened",
        value: {
          $typeName: "anecho.v0.OpenSessionResponse",
          sessionId: s.id,
          applied: { ...cfg!, inputChannels: [0, 1], outputChannels: [0, 1] },
        },
      };
    }
    case "closeSession": {
      const s = sessions.get(payload.value.sessionId);
      if (!s) return error(2, `no such session ${payload.value.sessionId}`);
      stopStream(s);
      sessions.delete(s.id);
      owned.delete(s.id);
      return { case: "sessionClosed", value: { $typeName: "anecho.v0.CloseSessionResponse" } };
    }
    case "startStream": {
      const req = payload.value;
      const s = sessions.get(req.sessionId);
      if (!s) return error(2, `no such session ${req.sessionId}`);
      if (s.stream) return error(3, "session already has a running stream");
      if (req.generator?.level?.unit.case === "dbvRms") {
        const want = req.generator.level.unit.value;
        const idx = OUTPUT_RANGES.findIndex((r) => r >= want + 0.5);
        if (idx < 0) return error(4, `no output range can produce ${want} dBV`);
        if (idx !== s.outputRange) {
          s.outputRange = idx;
          sendEvent(ws, { case: "rangeChanged", value: { sessionId: s.id, outputRange: idx } });
        }
      }
      const scale = { $typeName: "anecho.v0.Scale" as const, unit: { case: "dbvOffset" as const, value: dbvOffset(INPUT_RANGES[s.inputRange]) } };
      const base = { $typeName: "anecho.v0.StartStreamResponse" as const, channels: 2, sampleRate: SAMPLE_RATE, scale, axisHz: [] as number[], axisSeconds: [] as number[] };
      if (req.kind === StreamKind.RTA) {
        const { axis, stream } = startRta(ws, s, req.rta, req.generator);
        s.stream = stream;
        return { case: "streamStarted", value: { ...base, streamId: stream.id, kind: StreamKind.RTA, valuesPerChannel: axis.length, axisHz: axis } };
      }
      if (req.kind === StreamKind.SCOPE) {
        const { axis, stream } = startScope(ws, s, req.scope, req.generator);
        s.stream = stream;
        return { case: "streamStarted", value: { ...base, streamId: stream.id, kind: StreamKind.SCOPE, valuesPerChannel: axis.length, axisSeconds: axis } };
      }
      const stream = startLevels(ws, s, req.levelsRateHz, req.generator);
      s.stream = stream;
      return { case: "streamStarted", value: { ...base, streamId: stream.id, kind: StreamKind.LEVELS, valuesPerChannel: 2 } };
    }
    case "stopStream": {
      const s = [...sessions.values()].find((x) => x.stream?.id === payload.value.streamId);
      if (!s) return error(2, `no such stream ${payload.value.streamId}`);
      stopStream(s);
      return { case: "streamStopped", value: { $typeName: "anecho.v0.StopStreamResponse" } };
    }
    case "measure": {
      const req = payload.value;
      const s = sessions.get(req.sessionId);
      if (!s) return error(2, `no such session ${req.sessionId}`);
      if (s.stream) return error(3, "stop the stream before measuring");
      const tone = generatorPeakDbfs(req.generator, s.outputRange) ?? { hz: 1000, dbfs: -20 };
      const imd = req.kind === MeasureKind.IMD_SMPTE || req.kind === MeasureKind.IMD_CCIF;
      const per = [0, 1].map((ch) =>
        create(DistortionResultSchema, {
          fundamentalHz: tone.hz,
          fundamentalLevel: tone.dbfs - 3.01 + dbvOffset(OUTPUT_RANGES[s.outputRange]) - 0.02 * ch,
          thdPct: 0.00089,
          thdDb: -101,
          thdNPct: 0.0021,
          thdNDb: -93.5,
          harmonics: Array.from({ length: (req.maxHarmonic || 9) - 1 }, (_, i) => ({
            $typeName: "anecho.v0.DistortionResult.Harmonic" as const,
            order: i + 2,
            frequencyHz: tone.hz * (i + 2),
            levelDbRel: -101 - 6 * i,
          })),
          noiseFloorDb: -130,
          imdPct: imd ? 0.0031 : 0,
          imdDb: imd ? -90.2 : 0,
        }),
      );
      const scale = { $typeName: "anecho.v0.Scale" as const, unit: { case: "dbvOffset" as const, value: dbvOffset(INPUT_RANGES[s.inputRange]) } };
      return { case: "measurement", value: create(MeasureResponseSchema, { kind: req.kind, channel: 0, perChannel: per, sampleRate: SAMPLE_RATE, scale }) };
    }
    default:
      return error(1, `not a request: ${payload.case}`);
  }
}

const wss = new WebSocketServer({ host: "127.0.0.1", port: PORT, path: "/ws" });
wss.on("connection", (ws) => {
  const owned = new Set<bigint>();
  ws.on("message", (data) => {
    let env: Envelope;
    try {
      env = fromBinary(EnvelopeSchema, new Uint8Array(data as Buffer));
    } catch (e) {
      console.warn("undecodable envelope", e);
      return;
    }
    const reply = handle(ws, env.payload, owned);
    ws.send(toBinary(EnvelopeSchema, create(EnvelopeSchema, { requestId: env.requestId, payload: reply })));
  });
  ws.on("close", () => {
    for (const id of owned) {
      const s = sessions.get(id);
      if (s) stopStream(s);
      sessions.delete(id);
    }
  });
});
console.log(`mock anecho backend listening on ws://127.0.0.1:${PORT}/ws`);
