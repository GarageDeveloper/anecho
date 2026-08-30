# qa40x-driver

Unofficial Rust driver for the [QuantAsylum](https://quantasylum.com) QA402 and
QA403 USB audio analyzers. No UI, no DSP: the crate speaks the device's USB
protocol and exposes the analyzer as a calibrated, range-switched, synchronous
generate-and-capture device.

Extracted from [qa40x-rs](https://github.com/GarageDeveloper/qa40x-rs) and used
by [Anecho](https://github.com/garagedeveloper/anecho). Pure Rust USB via
[`nusb`](https://crates.io/crates/nusb) — no libusb.

## What it does

- **Discovery** — enumerate units on the USB bus (`UsbDeviceSource`), stable
  ids (`usb/<serial>`), capability records (channels, sample rates incl. the
  QA403-only 384 kHz, input/output range tables, calibration source).
- **Control** — input ranges 0…42 dBV, output ranges −12…+18 dBV, sample rate
  48/96/192/384 kHz, telemetry (USB voltage/current, temperature), LINK-LED
  keepalive, firmware version, serial, bootloader entry.
- **Factory calibration** — reads the 512-byte calibration page, verifies its
  CRC-16/BUYPASS, and converts dBFS readings to absolute dBV
  (`input_dbv_offset`, `output_dbv_offset`, `dac_trims`).
- **Streaming** — synchronous `generate_and_capture` (DAC out + ADC in in one
  USB stream), `acquire_data` (capture only), cooperative cancellation,
  automatic range-relay settling and in-capture keepalive.
- **Front-panel I2S** — width selection and the paced-writer endpoint.

## Features

| feature | default | purpose |
|---|---|---|
| `serde` | yes | serde derives on the public value types |
| `ts` | no | ts-rs derives (TypeScript bindings) |
| `sim` | no | embedded virtual QA40x ([`vqa40x-core`](https://github.com/GarageDeveloper/virtual-qa40x-rs)) behind the same endpoint queues as the hardware — hardware-free tests and demos |

## Example

```rust,no_run
use qa40x_driver::{Channel, InputGain, OutputGain, QA40xDevice, SampleRate};

#[tokio::main]
async fn main() -> qa40x_driver::Result<()> {
    let dev = QA40xDevice::new();
    dev.connect().await?; // first QA40x on the bus
    dev.set_sample_rate(SampleRate::Rate48kHz).await?;
    dev.set_output_gain(OutputGain::Gain8dBV).await?;
    dev.set_input_gain(InputGain::Gain6dBV).await?;

    let tone: Vec<f32> = (0..48_000)
        .map(|i| 0.1 * (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 48_000.0).sin())
        .collect();
    let captured = dev.generate_and_capture(&tone, &tone).await?;

    let (dbv_offset, calibrated) = dev.input_dbv_offset(Channel::Left).await;
    println!("{} samples, dBFS→dBV offset {dbv_offset:+.2} dB (calibrated: {calibrated})",
        captured.left_channel.len());
    dev.disconnect().await
}
```

Hardware examples: `cargo run -p qa40x-driver --example hw_keepalive`,
`hw_readstate`, `hw_loopback` (the last one needs a resistive loopback:
OUT L+/R+ → IN L+/R+, IN L−/R− terminated).

Hardware-free tests: `cargo test -p qa40x-driver --features sim`.

## Protocol summary

- USB VID `0x16C0`, PID `0x4E37` (QA402) / `0x4E39` (QA403), one interface.
- EP1 OUT/IN: registers. Write = `[addr][u32 big-endian]`; read = write
  `[addr | 0x80][0,0,0,0]` then read a 4-byte reply. Key registers: input/output
  range (5/6), stream control (8), sample rate (9), I2S control (0x0A), page
  select (0x0D) + calibration (0x19), telemetry (0x11–0x16), serial (0x1D).
- EP2 OUT/IN: audio streaming, 16 KiB blocks of interleaved `i32` little-endian
  stereo samples (right channel first on the wire).
- EP3 OUT: front-panel I2S sink.
- Calibration page: flash page 0, 512 bytes, 6-byte records
  (`i16` level dBV + `f32` LE dB trim), CRC-16/BUYPASS big-endian at 0x1FE.

See the module docs (`device`, `register`, `transport`, `settle`, `i2s`) for
the details and the measurements behind the timing constants.

## Linux: udev rule

Without a rule the device is root-only. Install `udev/99-qa40x.rules` to
`/etc/udev/rules.d/` and replug the analyzer:

```sh
sudo cp udev/99-qa40x.rules /etc/udev/rules.d/ && sudo udevadm control --reload
```

## Status

Unofficial, reverse-engineered, not affiliated with QuantAsylum. The QA402 has
been validated on hardware (including a resistive-loopback calibration check);
the QA403 path is exercised through the simulator and its range/rate tables.

## License

MIT OR Apache-2.0, at your option. See `LICENSE-MIT` and `LICENSE-APACHE`.
