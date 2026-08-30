//! Regroups arbitrarily sized interleaved callbacks into fixed-size [`InputBlock`]s.
//!
//! Shared by every backend whose native buffer size does not match the requested
//! `block_frames`. Designed for audio callbacks: no allocation after construction except
//! the `Arc<[f32]>` of each emitted block, and never blocks (`try_send`).

use crate::InputBlock;
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Debug)]
pub struct Blocker {
    channels: u16,
    block_frames: usize,
    buf: Vec<f32>,
    seq: u64,
    first_frame: u64,
    dropped: u32,
    tx: mpsc::Sender<InputBlock>,
    blocking: bool,
}

impl Blocker {
    pub fn new(channels: u16, block_frames: u32, tx: mpsc::Sender<InputBlock>) -> Self {
        let block_frames = block_frames.max(1) as usize;
        Self {
            channels,
            block_frames,
            buf: Vec::with_capacity(block_frames * channels as usize),
            seq: 0,
            first_frame: 0,
            dropped: 0,
            tx,
            blocking: false,
        }
    }

    /// Wait for channel room instead of dropping (only for threads that are allowed to
    /// block — never for a real audio callback).
    pub fn blocking(mut self) -> Self {
        self.blocking = true;
        self
    }

    /// Push interleaved samples. Emits zero or more blocks.
    pub fn push(&mut self, mut data: &[f32]) {
        let block_len = self.block_frames * self.channels as usize;
        while !data.is_empty() {
            let room = block_len - self.buf.len();
            let take = room.min(data.len());
            self.buf.extend_from_slice(&data[..take]);
            data = &data[take..];
            if self.buf.len() == block_len {
                self.emit();
            }
        }
    }

    fn emit(&mut self) {
        let samples: Arc<[f32]> = Arc::from(self.buf.as_slice());
        self.buf.clear();
        let block = InputBlock {
            seq: self.seq,
            first_frame: self.first_frame,
            channels: self.channels,
            frames: self.block_frames as u32,
            samples,
            dropped_before: self.dropped,
        };
        self.seq += 1;
        self.first_frame += self.block_frames as u64;
        if self.blocking {
            if self.tx.blocking_send(block).is_ok() {
                self.dropped = 0;
            }
            return;
        }
        match self.tx.try_send(block) {
            Ok(()) => self.dropped = 0,
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.dropped = self.dropped.saturating_add(1);
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {}
        }
    }

    pub fn into_sender(self) -> mpsc::Sender<InputBlock> {
        self.tx
    }

    pub fn is_closed(&self) -> bool {
        self.tx.is_closed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn regroups_into_fixed_blocks() {
        let (tx, mut rx) = mpsc::channel(8);
        let mut b = Blocker::new(2, 4, tx);
        let data: Vec<f32> = (0..20).map(|i| i as f32).collect(); // 10 frames stereo
        b.push(&data[..6]);
        b.push(&data[6..]);
        let b0 = rx.recv().await.unwrap();
        let b1 = rx.recv().await.unwrap();
        assert_eq!(b0.seq, 0);
        assert_eq!(b0.first_frame, 0);
        assert_eq!(&b0.samples[..], &data[..8]);
        assert_eq!(b1.first_frame, 4);
        assert_eq!(&b1.samples[..], &data[8..16]);
        assert!(
            rx.try_recv().is_err(),
            "2 leftover frames must stay buffered"
        );
    }

    #[tokio::test]
    async fn counts_drops_when_channel_full() {
        let (tx, mut rx) = mpsc::channel(1);
        let mut b = Blocker::new(1, 2, tx);
        b.push(&[0.0; 6]); // 3 blocks, capacity 1 -> 2 dropped
        let first = rx.recv().await.unwrap();
        assert_eq!(first.dropped_before, 0);
        b.push(&[0.0; 2]);
        let next = rx.recv().await.unwrap();
        assert_eq!(next.seq, 3);
        assert_eq!(next.dropped_before, 2);
    }
}
