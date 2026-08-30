# QA402: DAC data appears in the ADC stream after cancelling a just-started capture

Status: reproduced on one QA402 (firmware 60, macOS host, nusb 0.2.5 and 0.2.7).
Not reproduced on a QA403 (same firmware version number) in the same campaign.
Cause on the device side unconfirmed — to be reported to QuantAsylum with the
reproduction below.

## Symptom

After the trigger, `generate_and_capture` calls return, inside otherwise normal ADC
data, stretches that are **bit-exact copies of the DAC stimulus** (both channels,
verified with distinct tones per channel): about 1 KiB of DAC data every 20 KiB of
ADC data (~128 frames every 2560), starting at a random offset; a call is either
entirely affected or clean. Affected fraction starts around 25–45 % of calls and
worsens with use. The register interface stays fully functional.

The state survives process restarts, interface re-claims and software disconnects.
**Only unplugging the USB cable clears it.** Tried and ineffective as recovery:
idle input/output range writes with pauses, sample-rate rewrite, a long silent
acquisition, software disconnect/reconnect, endpoint `clear_halt`.

## Trigger

Cancelling the USB bulk transfers of a capture **early in a stream cycle** — i.e.
`STREAM_START` (reg 0x08 = 0x05), transfers queued, then transfers cancelled and
`STREAM_STOP` written within the first tens/hundreds of ms. In application terms:
stopping/restarting acquisition rapidly (e.g. changing FFT size twice in a row), or
writing the input range register between two capture cycles of a running stream.

Campaign (fresh, power-cycled device per row; probe = 20 subsequent
8192-frame captures at the 42 dBV input range, counting calls with inserted data):

| trigger (36 events unless noted) | affected |
|---|---|
| input-range writes between capture cycles of a running stream | 7–15 / 20 |
| same + 300 ms / 1 s pause, `clear_halt`, or a silent capture after each write | 1–14 / 20 |
| stop/restart races: cancel 20–500 ms after start (any call size, incl. 8192 only) | 12–15 / 20 |
| stop after a fully received block, then write, then restart | 0 / 20 |
| same races with a **draining** stop (in-flight call always completes) | 0 / 20 |

## Reproduction

`cargo run -p anecho-device --features qa40x --example qa40x_restart_probe` with
`MODE=race` (requires outputs wired to inputs): prints a 20-call baseline, runs 36
early-cancelled restarts, prints the probe again. `qa40x_raw_dump` prints the
inserted samples of affected captures next to the expected signal.

## Guidance for driver users

Do not cancel in-flight bulk transfers of a capture unless the alternative is worse
(application shutdown): prefer letting the in-flight transaction complete and
stopping between transactions. Range registers (0x05/0x06) should be written with
no capture cycle in flight and no immediate restart racing them.
