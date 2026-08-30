// Binary frame layout, exactly as documented by `anecho.v0.BinaryFrame` in
// contract/anecho.proto (little-endian). Decoding only: values arrive ready to plot.

export interface Frame {
  streamId: number;
  seq: bigint;
  firstFrame: bigint;
  channels: number;
  valuesPerChannel: number;
  /** Channel-major: all values of channel 0, then channel 1, ... */
  values: Float32Array;
}

export const FRAME_HEADER_LEN = 4 + 8 + 8 + 2 + 2;

export function decodeFrame(buf: ArrayBuffer): Frame | null {
  if (buf.byteLength < FRAME_HEADER_LEN) return null;
  const view = new DataView(buf);
  const streamId = view.getUint32(0, true);
  const seq = view.getBigUint64(4, true);
  const firstFrame = view.getBigUint64(12, true);
  const channels = view.getUint16(20, true);
  const valuesPerChannel = view.getUint16(22, true);
  const count = channels * valuesPerChannel;
  if (buf.byteLength !== FRAME_HEADER_LEN + count * 4) return null;
  const values = new Float32Array(count);
  for (let i = 0; i < count; i++) {
    values[i] = view.getFloat32(FRAME_HEADER_LEN + i * 4, true);
  }
  return { streamId, seq, firstFrame, channels, valuesPerChannel, values };
}

export function channelValues(frame: Frame, ch: number): Float32Array {
  const n = frame.valuesPerChannel;
  return frame.values.subarray(ch * n, (ch + 1) * n);
}
