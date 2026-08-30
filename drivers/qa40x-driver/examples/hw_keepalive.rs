//! Smoke-test the LINK-LED keepalive: connect, then run the keepalive cycle
//! (write link reg 0x00 + read telemetry) a few times at ~1 s, like the app does
//! while connected and idle. Watch the device's LINK LED stay lit during the run.
//!
//! Run with: cargo run --example hw_keepalive

use qa40x_driver::QA40xDevice;
use std::time::Duration;

#[tokio::main]
async fn main() {
    env_logger::init();
    let device = QA40xDevice::new();
    println!("== Connecting ==");
    device.connect().await.expect("connect failed");

    println!("== 6 keepalive cycles @ ~1 s (LINK LED should stay lit) ==");
    for i in 1..=6 {
        match device.keepalive().await {
            Ok(t) => println!(
                "  {i}: link ping OK · USB {:.3} V · {:.1} °C",
                t.usb_voltage_v, t.temperature_c
            ),
            Err(e) => println!("  {i}: keepalive ERROR: {e}"),
        }
        tokio::time::sleep(Duration::from_millis(1000)).await;
    }
    println!("== Done ==");
}
