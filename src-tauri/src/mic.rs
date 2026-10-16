use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::mpsc;

const RATE: f32 = 16_000.0;

/// Returns (stream, 16 kHz mono chunks). Nearest-sample resampling is plenty for speech.
pub fn open() -> Result<(cpal::Stream, mpsc::Receiver<Vec<f32>>), Box<dyn std::error::Error>> {
    let dev = cpal::default_host().default_input_device().ok_or("no input device")?;
    let cfg = dev.default_input_config()?;
    let (ch, rate) = (cfg.channels() as usize, cfg.sample_rate().0 as f32);
    let (tx, rx) = mpsc::channel();
    let step = rate / RATE;
    let stream = dev.build_input_stream(
        &cfg.into(),
        move |data: &[f32], _| {
            let n = (data.len() / ch) as f32 / step;
            let out = (0..n as usize).map(|i| data[(i as f32 * step) as usize * ch]).collect();
            let _ = tx.send(out);
        },
        |e| eprintln!("mic error: {e}"),
        None,
    )?;
    stream.play()?;
    Ok((stream, rx))
}
