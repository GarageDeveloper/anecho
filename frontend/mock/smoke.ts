// Smoke test of the mock backend through the contract: version, devices, session, RTA
// stream (one frame), measure, stop. Run: `pnpm exec tsx mock/smoke.ts [port]`.

import WebSocket from "ws";
import { create, fromBinary, toBinary } from "@bufbuild/protobuf";
import { EnvelopeSchema, MeasureKind, StreamKind, type Envelope } from "../src/gen/anecho_pb";

const port = Number(process.argv[2] ?? 4800);
const ws = new WebSocket(`ws://127.0.0.1:${port}/ws`);
ws.binaryType = "arraybuffer";

let nextId = 1n;
const pending = new Map<bigint, (p: Envelope["payload"]) => void>();
let frames = 0;
let lastFrameLen = 0;

ws.on("message", (data) => {
  const bytes = new Uint8Array(data as ArrayBuffer);
  try {
    const env = fromBinary(EnvelopeSchema, bytes);
    if (env.payload.case !== undefined && env.requestId !== 0n) {
      pending.get(env.requestId)?.(env.payload);
      pending.delete(env.requestId);
      return;
    }
    if (env.payload.case === "event") {
      console.log("event", env.payload.value.kind.case);
      return;
    }
  } catch {
    /* frame */
  }
  frames++;
  lastFrameLen = bytes.byteLength;
});

function request(payload: Envelope["payload"]): Promise<Envelope["payload"]> {
  const requestId = nextId++;
  return new Promise((resolve) => {
    pending.set(requestId, resolve);
    ws.send(toBinary(EnvelopeSchema, create(EnvelopeSchema, { requestId, payload })));
  });
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

ws.on("open", async () => {
  const v = await request({ case: "getVersion", value: { $typeName: "anecho.v0.GetVersionRequest" } });
  console.log("version", v.case, v.case === "version" ? v.value.backendVersion : "");
  const d = await request({ case: "listDevices", value: { $typeName: "anecho.v0.ListDevicesRequest" } });
  if (d.case !== "devices") throw new Error("devices");
  console.log("devices", d.value.devices.map((x) => x.id));
  const s = await request({
    case: "openSession",
    value: {
      $typeName: "anecho.v0.OpenSessionRequest",
      deviceId: d.value.devices[0].id,
      config: { $typeName: "anecho.v0.DeviceConfig", sampleRate: 48000, inputChannels: [], outputChannels: [], inputRange: 1, outputRange: 1 },
    },
  });
  if (s.case !== "sessionOpened") throw new Error(`open: ${JSON.stringify(s)}`);
  const st = await request({
    case: "startStream",
    value: {
      $typeName: "anecho.v0.StartStreamRequest",
      sessionId: s.value.sessionId,
      kind: StreamKind.RTA,
      blockFrames: 0,
      levelsRateHz: 0,
      rta: { $typeName: "anecho.v0.RtaConfig", fftLength: 16384, window: 2, minHz: 20, maxHz: 20000, points: 200, octaveFraction: 0, updateRateHz: 20 },
      generator: {
        $typeName: "anecho.v0.Generator",
        signal: { case: "sine", value: { $typeName: "anecho.v0.Generator.Sine", frequencyHz: 1000, amplitudeDbfs: -20 } },
        level: { $typeName: "anecho.v0.Generator.Level", unit: { case: "dbvRms", value: -10 } },
        outputChannels: [],
      },
    },
  });
  if (st.case !== "streamStarted") throw new Error(`start: ${JSON.stringify(st)}`);
  console.log("stream", st.value.streamId, "points", st.value.valuesPerChannel, "axis", st.value.axisHz.length, "unit", st.value.scale?.unit.case);
  await sleep(300);
  console.log("frames received", frames, "last frame bytes", lastFrameLen, "expected", 24 + 2 * st.value.valuesPerChannel * 4);
  const stop = await request({ case: "stopStream", value: { $typeName: "anecho.v0.StopStreamRequest", streamId: st.value.streamId } });
  console.log("stop", stop.case);
  const m = await request({
    case: "measure",
    value: { $typeName: "anecho.v0.MeasureRequest", sessionId: s.value.sessionId, kind: MeasureKind.THD, fftLength: 0, window: 0, averages: 0, maxHarmonic: 0, bandMinHz: 0, bandMaxHz: 0 },
  });
  console.log("measure", m.case, m.case === "measurement" ? `${m.value.perChannel.length} ch, THD ${m.value.perChannel[0].thdPct} %` : "");
  const c = await request({ case: "closeSession", value: { $typeName: "anecho.v0.CloseSessionRequest", sessionId: s.value.sessionId } });
  console.log("close", c.case);
  const ok = frames > 0 && lastFrameLen === 24 + 2 * st.value.valuesPerChannel * 4 && m.case === "measurement";
  console.log(ok ? "SMOKE OK" : "SMOKE FAILED");
  ws.close();
  process.exit(ok ? 0 : 1);
});
