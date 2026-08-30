//! Test signals driving a device's outputs during a stream. Phase 0: sine only.

use anecho_device::OutputSource;

#[derive(Debug, Clone, PartialEq)]
pub enum Signal {
    Sine {
        frequency_hz: f32,
        amplitude_dbfs: f32,
    },
}

#[derive(Debug)]
pub struct Generator {
    signal: Signal,
    phase: f64,
}

impl Generator {
    pub fn new(signal: Signal) -> Self {
        Self { signal, phase: 0.0 }
    }
}

impl OutputSource for Generator {
    fn fill(&mut self, buf: &mut [f32], channels: u16, sample_rate: u32) {
        match self.signal {
            Signal::Sine {
                frequency_hz,
                amplitude_dbfs,
            } => {
                let amp = 10f64.powf(amplitude_dbfs as f64 / 20.0);
                let step = std::f64::consts::TAU * frequency_hz as f64 / sample_rate as f64;
                for frame in buf.chunks_exact_mut(channels as usize) {
                    let v = (amp * self.phase.sin()) as f32;
                    frame.iter_mut().for_each(|s| *s = v);
                    self.phase += step;
                    if self.phase >= std::f64::consts::TAU {
                        self.phase -= std::f64::consts::TAU;
                    }
                }
            }
        }
    }
}
