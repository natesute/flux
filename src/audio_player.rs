//! Live audio output for the preview window. Streams the loaded WAV
//! through the default output device, looping at end-of-file. Synced
//! loosely against the visual wall clock (cpal's stream starts the same
//! moment we record the visual `Instant::now()`); good enough that ear
//! and eye line up.
//!
//! v1 deliberately resamples in the callback rather than pulling in a
//! dedicated resampler crate. It's a single-pole linear interpolator —
//! audibly clean enough for monitor playback, not what you'd ship as
//! the final mix. The mono source is duplicated to all output channels.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream};

use crate::audio::AudioTrack;

/// Owns the cpal output stream. Drop to stop playback.
pub struct AudioPlayer {
    /// Stream is `!Send + !Sync`, but we only ever touch it from one
    /// thread so a single owning field is fine.
    _stream: Stream,
    /// Set to `true` to silence the stream without dropping it. Currently
    /// unused — exposed so a future "mute" toggle can flip it.
    pub muted: Arc<AtomicBool>,
}

impl AudioPlayer {
    /// Build a player that loops `track` through the system's default
    /// output device. Returns `Ok(None)` if the host has no default
    /// output (e.g. headless CI); in that case the preview keeps working
    /// silently.
    pub fn try_new(track: &AudioTrack) -> Result<Option<Self>> {
        let host = cpal::default_host();
        let Some(device) = host.default_output_device() else {
            return Ok(None);
        };
        let cfg = device
            .default_output_config()
            .context("getting default output config")?;
        let sample_format = cfg.sample_format();
        let channels = cfg.channels() as usize;
        let out_sample_rate = cfg.sample_rate().0 as f64;
        let stream_config: cpal::StreamConfig = cfg.into();

        let muted = Arc::new(AtomicBool::new(false));

        let samples: Arc<[f32]> = Arc::from(track.samples().to_vec().into_boxed_slice());
        let src_rate = track.sample_rate() as f64;
        let rate_ratio = src_rate / out_sample_rate;

        // Floating cursor into the source samples; advances by rate_ratio
        // per output frame. Wrapping keeps the loop seamless.
        let mut cursor: f64 = 0.0;
        let len = samples.len();

        let muted_cb = muted.clone();
        let err_fn = |e| tracing::warn!("audio stream error: {e}");

        let stream = match sample_format {
            SampleFormat::F32 => {
                let samples = samples.clone();
                device.build_output_stream(
                    &stream_config,
                    move |out: &mut [f32], _| {
                        if muted_cb.load(Ordering::Relaxed) {
                            for s in out.iter_mut() {
                                *s = 0.0;
                            }
                            return;
                        }
                        for frame in out.chunks_mut(channels) {
                            let sample = sample_at(&samples, len, cursor);
                            for c in frame.iter_mut() {
                                *c = sample;
                            }
                            cursor += rate_ratio;
                            if cursor >= len as f64 {
                                cursor -= len as f64;
                            }
                        }
                    },
                    err_fn,
                    None,
                )?
            }
            SampleFormat::I16 => {
                let samples = samples.clone();
                device.build_output_stream(
                    &stream_config,
                    move |out: &mut [i16], _| {
                        if muted_cb.load(Ordering::Relaxed) {
                            for s in out.iter_mut() {
                                *s = 0;
                            }
                            return;
                        }
                        for frame in out.chunks_mut(channels) {
                            let sample = sample_at(&samples, len, cursor);
                            let i = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                            for c in frame.iter_mut() {
                                *c = i;
                            }
                            cursor += rate_ratio;
                            if cursor >= len as f64 {
                                cursor -= len as f64;
                            }
                        }
                    },
                    err_fn,
                    None,
                )?
            }
            SampleFormat::U16 => {
                let samples = samples.clone();
                device.build_output_stream(
                    &stream_config,
                    move |out: &mut [u16], _| {
                        if muted_cb.load(Ordering::Relaxed) {
                            for s in out.iter_mut() {
                                *s = u16::MAX / 2;
                            }
                            return;
                        }
                        for frame in out.chunks_mut(channels) {
                            let sample = sample_at(&samples, len, cursor);
                            let scaled = (sample.clamp(-1.0, 1.0) * 0.5 + 0.5) * u16::MAX as f32;
                            let v = scaled as u16;
                            for c in frame.iter_mut() {
                                *c = v;
                            }
                            cursor += rate_ratio;
                            if cursor >= len as f64 {
                                cursor -= len as f64;
                            }
                        }
                    },
                    err_fn,
                    None,
                )?
            }
            other => return Err(anyhow!("unsupported audio format {other:?}")),
        };
        stream.play().context("starting audio stream")?;

        Ok(Some(Self {
            _stream: stream,
            muted,
        }))
    }
}

/// Linear-interpolated sample lookup. `cursor` is a fractional index into
/// `samples`; values past the end wrap.
#[inline]
fn sample_at(samples: &[f32], len: usize, cursor: f64) -> f32 {
    if len == 0 {
        return 0.0;
    }
    let i = cursor as usize % len;
    let next = (i + 1) % len;
    let frac = (cursor - cursor.floor()) as f32;
    samples[i] * (1.0 - frac) + samples[next] * frac
}
