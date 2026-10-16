//! Kokoro TTS (sherpa-onnx) played through the default output device.
use crate::models::path;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use sherpa_onnx::{
    GenerationConfig, OfflineTts, OfflineTtsConfig, OfflineTtsKokoroModelConfig,
    OfflineTtsModelConfig,
};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// When queued audio finishes playing.
static UNTIL: Mutex<Option<Instant>> = Mutex::new(None);
const TAIL: Duration = Duration::from_millis(400); // room echo

/// True while Sunday is audible, so the mic can ignore itself.
pub fn speaking() -> bool {
    UNTIL.lock().unwrap().is_some_and(|t| Instant::now() < t + TAIL)
}

pub struct Speaker {
    tts: OfflineTts,
    _stream: cpal::Stream,
    queue: Arc<Mutex<std::collections::VecDeque<f32>>>,
}

impl Speaker {
    pub fn new() -> Option<Self> {
        let k = "kokoro-en-v0_19";
        let config = OfflineTtsConfig {
            model: OfflineTtsModelConfig {
                kokoro: OfflineTtsKokoroModelConfig {
                    model: Some(path(&format!("{k}/model.onnx"))),
                    voices: Some(path(&format!("{k}/voices.bin"))),
                    tokens: Some(path(&format!("{k}/tokens.txt"))),
                    data_dir: Some(path(&format!("{k}/espeak-ng-data"))),
                    ..Default::default()
                },
                num_threads: 2,
                ..Default::default()
            },
            ..Default::default()
        };
        let tts = OfflineTts::create(&config)?;
        let rate = tts.sample_rate() as u32;
        // The first synthesis is slow (lazy init); pay for it now instead of on the first reply.
        let none: Option<fn(&[f32], f32) -> bool> = None;
        let _ = tts.generate_with_config("Hi.", &GenerationConfig { sid: 0, speed: 1.0, ..Default::default() }, none);

        let dev = cpal::default_host().default_output_device()?;
        let mut cfg = dev.default_output_config().ok()?.config();
        cfg.sample_rate = cpal::SampleRate(rate);
        cfg.channels = 1;
        let queue: Arc<Mutex<std::collections::VecDeque<f32>>> = Arc::default();
        let q = queue.clone();
        let stream = dev
            .build_output_stream(
                &cfg,
                move |out: &mut [f32], _| {
                    let mut q = q.lock().unwrap();
                    out.iter_mut().for_each(|s| *s = q.pop_front().unwrap_or(0.0));
                },
                |e| eprintln!("tts output error: {e}"),
                None,
            )
            .ok()?;
        stream.play().ok()?;
        Some(Self { tts, _stream: stream, queue })
    }

    /// Generates then queues the audio. Blocks for the synthesis time (RTF is well under 1).
    pub fn speak(&self, text: &str) {
        let cfg = GenerationConfig { sid: 0, speed: 1.0, ..Default::default() };
        let none: Option<fn(&[f32], f32) -> bool> = None;
        if let Some(audio) = self.tts.generate_with_config(text, &cfg, none) {
            let secs = audio.samples().len() as f64 / self.tts.sample_rate() as f64;
            let mut until = UNTIL.lock().unwrap();
            *until = Some(until.filter(|t| *t > Instant::now()).unwrap_or_else(Instant::now) + Duration::from_secs_f64(secs));
            self.queue.lock().unwrap().extend(audio.samples());
        }
    }

    /// Cut playback immediately (user interruption).
    pub fn stop(&self) {
        self.queue.lock().unwrap().clear();
        *UNTIL.lock().unwrap() = None;
    }
}

pub enum Cmd {
    Speak(String),
    Stop,
}

/// cpal streams aren't Send, so the speaker lives on its own thread; talk to it by channel.
pub fn spawn() -> std::sync::mpsc::Sender<Cmd> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let Some(speaker) = Speaker::new() else { return eprintln!("tts unavailable") };
        for cmd in rx {
            match cmd {
                Cmd::Speak(t) => speaker.speak(&t),
                Cmd::Stop => speaker.stop(),
            }
        }
    });
    tx
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs the models from scripts/fetch-models.sh. Prints synthesis time vs audio length.
    #[test]
    fn synthesis_speed() {
        let s = Speaker::new().expect("tts unavailable");
        for text in ["Hello there.", "I can answer questions, write different kinds of creative text, and help you with information.", "Just let me know what you need."] {
            let t = Instant::now();
            s.speak(text);
            let wait = UNTIL.lock().unwrap().unwrap().saturating_duration_since(Instant::now());
            println!("synth {:?} for {:?} of audio: {text}", t.elapsed(), wait);
        }
    }
}
