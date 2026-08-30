# testbench/ — A/B bench, Anecho vs REW

Numerical reference harness. REW runs in API mode (`http://localhost:4735`, it serves its
own Swagger spec at `/swagger-spec.js`); Anecho exposes its WebSocket API. The bench feeds
both engines the same WAV files / signals and compares FR, IR, THD, RT60 within documented
tolerances.

- **Phase 0 (this crate):** plumbing only — `rew` client (version, devices, measurements),
  `anecho` client (reuses `anecho-client`), `compare` command printing both.
- **Phase 2+:** numerical comparisons in CI, per-quantity tolerances; text-export
  comparison as a fallback for quantities REW's API does not expose.

```
make testbench            # REW and `anecho serve` must both be running
cargo run -p anecho-testbench -- compare --rew http://localhost:4735 --anecho ws://127.0.0.1:4800/ws
```
