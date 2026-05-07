//! Offline audio analysis. Loads a WAV file once, then exposes per-frame
//! features (FFT bands, RMS) by sampling a window centered on the frame's
//! timestamp.

use std::cell::RefCell;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use rustfft::{num_complex::Complex, FftPlanner};

/// Default FFT window size. Larger = better frequency resolution, worse
/// time resolution. 2048 at 48kHz gives ~43ms resolution which is fine.
const FFT_SIZE: usize = 2048;

/// Audio features for a single rendered frame. Cheap to copy; nodes
/// pass it around freely.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameAudioFeatures {
    /// Root-mean-square amplitude of the window. ~0..1 typically.
    pub rms: f32,
    /// Bass band magnitude (~20-250 Hz), normalized 0..1.
    pub bass: f32,
    /// Low-mid band magnitude (~250-1000 Hz).
    pub low_mid: f32,
    /// High-mid band magnitude (~1000-4000 Hz).
    pub high_mid: f32,
    /// Treble band (~4000-16000 Hz).
    pub treble: f32,
}

pub struct AudioTrack {
    /// Decoded mono PCM. Held as `Arc<[f32]>` so the cpal output stream
    /// in `audio_player` can take its own clone without copying the
    /// whole buffer (typical 90s 48kHz stereo clip is ~17 MB).
    samples: Arc<[f32]>,
    sample_rate: u32,
    fft: Arc<dyn rustfft::Fft<f32>>,
    window: Vec<f32>,
    /// Reusable FFT scratch buffer. Wrapped in RefCell so `features_at`
    /// can stay `&self` (called from the preview's render loop alongside
    /// other shared borrows). Costs a runtime borrow check; saves 16 KB
    /// of allocation per call.
    fft_buf: RefCell<Vec<Complex<f32>>>,
}

impl AudioTrack {
    pub fn load(path: &Path) -> Result<Self> {
        let mut reader = hound::WavReader::open(path)
            .with_context(|| format!("opening WAV {}", path.display()))?;
        let spec = reader.spec();
        let sample_rate = spec.sample_rate;

        // Convert to mono f32 in [-1, 1].
        let samples: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Float => {
                let raw: Vec<f32> = reader.samples::<f32>().filter_map(|s| s.ok()).collect();
                downmix(&raw, spec.channels as usize)
            }
            hound::SampleFormat::Int => {
                let max = (1i64 << (spec.bits_per_sample - 1)) as f32;
                let raw: Vec<f32> = reader
                    .samples::<i32>()
                    .filter_map(|s| s.ok())
                    .map(|s| s as f32 / max)
                    .collect();
                downmix(&raw, spec.channels as usize)
            }
        };

        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);

        // Hann window.
        let window: Vec<f32> = (0..FFT_SIZE)
            .map(|i| {
                let t = i as f32 / (FFT_SIZE - 1) as f32;
                0.5 - 0.5 * (std::f32::consts::TAU * t).cos()
            })
            .collect();

        Ok(Self {
            samples: Arc::from(samples.into_boxed_slice()),
            sample_rate,
            fft,
            window,
            fft_buf: RefCell::new(vec![Complex::new(0.0, 0.0); FFT_SIZE]),
        })
    }

    pub fn duration_seconds(&self) -> f32 {
        self.samples.len() as f32 / self.sample_rate as f32
    }

    /// Mono PCM samples in [-1, 1]. Exposed so the preview audio player
    /// can stream them to the output device.
    pub fn samples(&self) -> &[f32] {
        &self.samples
    }

    /// Cheap clone of the shared sample buffer. The cpal output stream
    /// in `audio_player` uses this — `Arc::clone` is bumping a refcount,
    /// not copying the audio data.
    pub fn samples_arc(&self) -> Arc<[f32]> {
        self.samples.clone()
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Compute features for the frame at the given time (seconds). The
    /// analysis window is centered on `time` and sized by `FFT_SIZE`, so
    /// the rendering framerate doesn't enter the calculation.
    pub fn features_at(&self, time: f32) -> FrameAudioFeatures {
        let center = (time * self.sample_rate as f32) as isize;
        let half = FFT_SIZE as isize / 2;
        let start = center - half;

        let mut buf = self.fft_buf.borrow_mut();

        // Refill the windowed buffer in place. Same math as before, but
        // no fresh Vec per call.
        for (i, slot) in buf.iter_mut().enumerate() {
            let idx = start + i as isize;
            let s = if idx < 0 || idx as usize >= self.samples.len() {
                0.0
            } else {
                self.samples[idx as usize]
            };
            *slot = Complex::new(s * self.window[i], 0.0);
        }

        // RMS from the time-domain windowed signal.
        let rms = (buf.iter().map(|c| c.re * c.re).sum::<f32>() / FFT_SIZE as f32).sqrt();

        // FFT.
        self.fft.process(&mut buf);

        let bin_hz = self.sample_rate as f32 / FFT_SIZE as f32;
        let band_mag = |lo: f32, hi: f32| -> f32 {
            let lo_bin = (lo / bin_hz) as usize;
            let hi_bin = ((hi / bin_hz) as usize).min(FFT_SIZE / 2);
            if hi_bin <= lo_bin {
                return 0.0;
            }
            let sum: f32 = buf[lo_bin..hi_bin].iter().map(|c| c.norm()).sum();
            // Normalize: empirical scaling so a loud band ends up around 1.0.
            let avg = sum / (hi_bin - lo_bin) as f32;
            (avg * 0.05).min(1.0)
        };

        FrameAudioFeatures {
            rms: (rms * 4.0).min(1.0),
            bass: band_mag(20.0, 250.0),
            low_mid: band_mag(250.0, 1000.0),
            high_mid: band_mag(1000.0, 4000.0),
            treble: band_mag(4000.0, 16000.0),
        }
    }
}

fn downmix(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    interleaved
        .chunks(channels)
        .map(|c| c.iter().sum::<f32>() / channels as f32)
        .collect()
}
