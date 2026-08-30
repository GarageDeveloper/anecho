//! `update_stream` on the virtual loopback: block size, sink and generator change in
//! place; the worker never stops.

use anecho_device::backends::virtual_loopback::{LoopbackOptions, VirtualLoopbackBackend};
use anecho_device::{DeviceBackend, DeviceConfig, OutputSource, StreamConfig, StreamUpdate};

struct Dc(f32);

impl OutputSource for Dc {
    fn fill(&mut self, buf: &mut [f32], _channels: u16, _sample_rate: u32) {
        buf.iter_mut().for_each(|s| *s = self.0);
    }
}

#[tokio::test]
async fn update_stream_swaps_block_size_sink_and_generator() {
    let backend = VirtualLoopbackBackend::new(LoopbackOptions {
        latency_frames: 0,
        ..Default::default()
    });
    let d = backend.enumerate().await.remove(0);
    let dev = backend.open(&d.id).await.unwrap();
    dev.configure(DeviceConfig::with_sample_rate(48_000))
        .await
        .unwrap();

    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let h = dev
        .start(
            StreamConfig {
                block_frames: 512,
                capture: true,
                generate: true,
            },
            tx,
            Some(Box::new(Dc(0.25))),
        )
        .await
        .unwrap();
    let b = rx.recv().await.unwrap();
    assert_eq!(b.frames, 512);
    assert!((b.samples[0] - 0.25).abs() < 1e-6);

    // Swap everything at once: new sink, new block size, new generator level.
    let (tx2, mut rx2) = tokio::sync::mpsc::channel(4);
    dev.update_stream(
        h,
        StreamUpdate {
            block_frames: Some(1024),
            output: Some(Some(Box::new(Dc(0.125)))),
            input: Some(tx2),
        },
    )
    .await
    .unwrap();

    // The old sink ends (its sender is dropped by the worker)...
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    // Drain blocks already in flight until the old sink closes.
    while (tokio::time::timeout_at(deadline, rx.recv()).await.unwrap()).is_some() {}
    // ...and the new sink gets 1024-frame blocks at the new level, counters restarted.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    let first = loop {
        let b = tokio::time::timeout_at(deadline, rx2.recv())
            .await
            .unwrap()
            .expect("new sink receives blocks");
        // The delay line and the swap boundary may leave one transition block; wait for a
        // block fully at the new level.
        if (b.samples[0] - 0.125).abs() < 1e-6
            && (b.samples[b.samples.len() - 1] - 0.125).abs() < 1e-6
        {
            break b;
        }
    };
    assert_eq!(first.frames, 1024);

    // Silence the generator in place.
    dev.update_stream(
        h,
        StreamUpdate {
            output: Some(None),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let b = tokio::time::timeout_at(deadline, rx2.recv())
            .await
            .unwrap()
            .expect("still streaming");
        if b.samples.iter().all(|&s| s == 0.0) {
            break;
        }
    }

    dev.stop(h).await.unwrap();
}
