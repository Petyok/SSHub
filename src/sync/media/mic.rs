//! Microphone capture (feature `audio`, `cpal`): moves samples from the default input device into
//! [`FskDecoder`]. The stream lives on this thread only (some backends make `Stream` `!Send`),
//! is dropped when the thread ends, and nothing is written anywhere.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample, Stream, StreamConfig};

use super::fsk::FskDecoder;
use super::{Emitter, MediaKind};

/// A default input device exists.
pub(super) fn present() -> bool {
    cpal::default_host().default_input_device().is_some()
}

/// Names of the input devices; the first line marks the default.
pub(super) fn devices() -> Vec<String> {
    let host = cpal::default_host();
    let default = host
        .default_input_device()
        .and_then(|d| d.description().ok())
        .map(|d| d.name().to_string());
    let Ok(devs) = host.input_devices() else {
        return Vec::new();
    };
    devs.filter_map(|d| d.description().ok())
        .map(|d| {
            let n = d.name().to_string();
            if default.as_deref() == Some(&n) {
                format!("{n} (default)")
            } else {
                n
            }
        })
        .collect()
}

enum Msg {
    Pcm(Vec<f32>),
    Failed(String),
}

fn build<T>(device: &cpal::Device, config: StreamConfig, tx: Sender<Msg>) -> Result<Stream, String>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = usize::from(config.channels).max(1);
    let err_tx = tx.clone();
    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                let mono = data
                    .chunks(channels)
                    .map(|f| f.iter().map(|s| f32::from_sample(*s)).sum::<f32>() / channels as f32)
                    .collect();
                let _ = tx.send(Msg::Pcm(mono));
            },
            move |e| {
                let _ = err_tx.send(Msg::Failed(e.to_string()));
            },
            None,
        )
        .map_err(|e| e.to_string())
}

pub(super) fn run(stop: &AtomicBool, em: &Emitter) {
    debug_assert_eq!(em.kind, MediaKind::Audio);
    let Some(device) = cpal::default_host().default_input_device() else {
        em.status("no microphone found");
        return;
    };
    let supported = match device.default_input_config() {
        Ok(c) => c,
        Err(e) => {
            em.status(format!("microphone unavailable: {e}"));
            return;
        }
    };
    let format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let rate = config.sample_rate;
    let (tx, rx) = mpsc::channel();
    let stream = match format {
        SampleFormat::F32 => build::<f32>(&device, config, tx),
        SampleFormat::I16 => build::<i16>(&device, config, tx),
        SampleFormat::U16 => build::<u16>(&device, config, tx),
        SampleFormat::I32 => build::<i32>(&device, config, tx),
        other => Err(format!("unsupported sample format {other}")),
    };
    let stream = match stream {
        Ok(s) => s,
        Err(e) => {
            em.status(format!("microphone unavailable: {e}"));
            return;
        }
    };
    if let Err(e) = stream.play() {
        em.status(format!("microphone unavailable: {e}"));
        return;
    }
    if !em.status(format!("listening on the microphone ({rate} Hz)")) {
        return;
    }
    let mut dec = FskDecoder::new(rate);
    let mut receiving = false;
    let started = Instant::now();
    let mut peak = 0.0f32;
    let mut warned = false;
    while !stop.load(Ordering::Relaxed) {
        let pcm = match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Msg::Pcm(p)) => p,
            Ok(Msg::Failed(e)) => {
                em.status(format!("microphone error: {e}"));
                return;
            }
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        peak = pcm.iter().fold(peak, |m, v| m.max(v.abs()));
        if !warned && started.elapsed() > Duration::from_secs(3) && peak == 0.0 {
            warned = true;
            em.status("the microphone delivers silence (muted, or access denied)");
        }
        for payload in dec.push(&pcm) {
            if em.reply(payload) {
                return;
            }
        }
        if dec.receiving() != receiving {
            receiving = dec.receiving();
            em.status(if receiving {
                "hearing a transmission"
            } else {
                "listening on the microphone"
            });
        }
    }
}
