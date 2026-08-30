// WebSocket client for the Anecho API: protobuf envelopes for control, binary frames for
// real-time data. Mirrors backend/crates/anecho-client.

import { create, fromBinary, toBinary } from "@bufbuild/protobuf";
import {
  CloseSessionRequestSchema,
  EnvelopeSchema,
  GetVersionRequestSchema,
  ListDevicesRequestSchema,
  OpenSessionRequestSchema,
  StopStreamRequestSchema,
  type DeviceConfig,
  type DeviceInfo,
  type Envelope,
  type ErrorCode,
  type Event,
  type GetVersionResponse,
  type OpenSessionResponse,
  type StartStreamRequest,
  type StartStreamResponse,
} from "../gen/anecho_pb";
import { decodeFrame, type Frame } from "./wire";

type Payload = Envelope["payload"];
type PayloadCase = NonNullable<Payload["case"]>;
type PayloadValue<K extends PayloadCase> = Extract<Payload, { case: K }>["value"];

export class ServerError extends Error {
  constructor(
    public code: ErrorCode,
    message: string,
  ) {
    super(message);
  }
}

export class Client {
  private ws: WebSocket;
  private nextId = 1n;
  private pending = new Map<bigint, { resolve: (p: Payload) => void; reject: (e: Error) => void }>();
  private frameListeners = new Set<(f: Frame) => void>();
  private eventListeners = new Set<(e: Event) => void>();
  private closeListeners = new Set<() => void>();

  private constructor(ws: WebSocket) {
    this.ws = ws;
    ws.binaryType = "arraybuffer";
    ws.onmessage = (m) => this.onMessage(m.data as ArrayBuffer);
    ws.onclose = () => {
      for (const p of this.pending.values()) p.reject(new Error("connection closed"));
      this.pending.clear();
      for (const l of this.closeListeners) l();
    };
  }

  static connect(url: string): Promise<Client> {
    return new Promise((resolve, reject) => {
      const ws = new WebSocket(url);
      ws.onopen = () => resolve(new Client(ws));
      ws.onerror = () => reject(new Error(`cannot connect to ${url}`));
    });
  }

  close() {
    this.ws.close();
  }

  onFrame(l: (f: Frame) => void): () => void {
    this.frameListeners.add(l);
    return () => this.frameListeners.delete(l);
  }

  onEvent(l: (e: Event) => void): () => void {
    this.eventListeners.add(l);
    return () => this.eventListeners.delete(l);
  }

  onClose(l: () => void): () => void {
    this.closeListeners.add(l);
    return () => this.closeListeners.delete(l);
  }

  private onMessage(data: ArrayBuffer) {
    // Envelopes and frames share the binary channel: try the envelope first, then the frame.
    let env: Envelope | null = null;
    try {
      env = fromBinary(EnvelopeSchema, new Uint8Array(data));
    } catch {
      env = null;
    }
    if (env && env.payload.case !== undefined) {
      if (env.requestId === 0n) {
        if (env.payload.case === "event") for (const l of this.eventListeners) l(env.payload.value);
        return;
      }
      const p = this.pending.get(env.requestId);
      if (p) {
        this.pending.delete(env.requestId);
        p.resolve(env.payload);
      }
      return;
    }
    const frame = decodeFrame(data);
    if (frame) for (const l of this.frameListeners) l(frame);
  }

  request(payload: Payload): Promise<Payload> {
    const requestId = this.nextId++;
    const env = create(EnvelopeSchema, { requestId, payload });
    return new Promise((resolve, reject) => {
      this.pending.set(requestId, {
        resolve: (p) => {
          if (p.case === "error") reject(new ServerError(p.value.code, p.value.message));
          else resolve(p);
        },
        reject,
      });
      this.ws.send(toBinary(EnvelopeSchema, env));
    });
  }

  private async expect<K extends PayloadCase>(payload: Payload, kind: K): Promise<PayloadValue<K>> {
    const p = await this.request(payload);
    if (p.case !== kind) throw new Error(`unexpected response ${p.case}`);
    return p.value as PayloadValue<K>;
  }

  version(): Promise<GetVersionResponse> {
    return this.expect({ case: "getVersion", value: create(GetVersionRequestSchema) }, "version");
  }

  async listDevices(): Promise<DeviceInfo[]> {
    return (await this.expect({ case: "listDevices", value: create(ListDevicesRequestSchema) }, "devices"))
      .devices;
  }

  openSession(deviceId: string, config: DeviceConfig): Promise<OpenSessionResponse> {
    const value = create(OpenSessionRequestSchema, { deviceId, config });
    return this.expect({ case: "openSession", value }, "sessionOpened");
  }

  async closeSession(sessionId: bigint): Promise<void> {
    const value = create(CloseSessionRequestSchema, { sessionId });
    await this.expect({ case: "closeSession", value }, "sessionClosed");
  }

  startStream(req: StartStreamRequest): Promise<StartStreamResponse> {
    return this.expect({ case: "startStream", value: req }, "streamStarted");
  }

  async stopStream(streamId: number): Promise<void> {
    const value = create(StopStreamRequestSchema, { streamId });
    await this.expect({ case: "stopStream", value }, "streamStopped");
  }
}
