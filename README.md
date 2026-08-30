# Anecho

Open source audio analyzer for acoustic measurement (rooms, loudspeakers) and
electrical measurement (amplifiers, electronics). Inspired by REW, which it does not try
to replace: Anecho is a much smaller tool that focuses on a scriptable engine and a
modern UI.

- **Headless Rust backend** — the whole engine (acquisition, DSP, analysis) is driven
  through an API (WebSocket + protobuf, binary Float32 frames for real-time streams).
- **Tauri + Svelte 5 frontend** — pure display client, replaceable, usable remotely
  (e.g. from a tablet). The frontend never computes anything: no FFT, smoothing or
  interpolation on the client side.
- **Devices** — QuantAsylum QA402/QA403 natively with calibration (`qa40x-driver`),
  plus any sound card via cpal (WASAPI/ASIO, Core Audio, ALSA).

## Design principles

1. **API-first.** Every feature is scriptable without the UI; the frontend is just one
   client among others.
2. **`contract/` is the source of truth.** Rust and TypeScript types are generated from
   the `.proto` files. Contract changes are additive only and shipped as separate commits.
3. **Golden headless tests.** Backend behaviour is locked by snapshots
   (WAV fixtures → expected results).
4. **Numerical correctness.** An A/B bench compares results against REW on the same
   WAV files; deviations beyond documented tolerances are blocking bugs.

## Status

Early development — not usable yet. If you need measurements today, use REW.

## License

GPLv3 — see [LICENSE](LICENSE).
