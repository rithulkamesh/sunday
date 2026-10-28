//! Kokoro TTS (sherpa-onnx) played through the default output device.
use crate::voice::models::path;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use sherpa_onnx::{
    GenerationConfig, OfflineTts, OfflineTtsConfig, OfflineTtsKokoroModelConfig,
    OfflineTtsModelConfig,
};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Samples waiting for the output device. Global so `interrupt` can silence playback at once.
static QUEUE: Mutex<std::collections::VecDeque<f32>> = Mutex::new(std::collections::VecDeque::new());
/// Bumped by `interrupt`; speech stamped with an older value is dropped.
/// Smoothed RMS of what we're playing, as f32 bits; the barge-in detector's echo reference.
static PLAY_LEVEL: AtomicU32 = AtomicU32::new(0);

pub fn playback_level() -> f32 {
    f32::from_bits(PLAY_LEVEL.load(Ordering::Relaxed))
}

/// Sentences handed to the speaker thread but not yet synthesized.
static PENDING: AtomicU32 = AtomicU32::new(0);
static REPLYING: AtomicBool = AtomicBool::new(false);

/// Queue a sentence for the speaker thread.
pub fn say(tx: &std::sync::mpsc::Sender<Cmd>, text: String, epoch: u64) {
    PENDING.fetch_add(1, Ordering::SeqCst);
    if tx.send(Cmd::Speak(text, epoch)).is_err() {
        PENDING.fetch_sub(1, Ordering::SeqCst);
    }
}

/// llm.rs marks the whole reply (thinking, synthesis, playback) so idle timers wait for it.
pub fn set_replying(on: bool) {
    REPLYING.store(on, Ordering::SeqCst);
}

/// Sunday is still talking or about to: audio playing, or sentences still being synthesized.
pub fn talking() -> bool {
    PENDING.load(Ordering::SeqCst) > 0 || speaking()
}

/// Anything of Sunday's in flight, thinking included.
pub fn busy() -> bool {
    REPLYING.load(Ordering::SeqCst) || talking()
}

/// The last few hundred chars Sunday said, to recognise its own echo coming back via the mic.
static SAID: Mutex<String> = Mutex::new(String::new());
const ECHO_WINDOW: Duration = Duration::from_secs(3);

fn words(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric() && c != '\'').filter(|w| !w.is_empty()).map(str::to_lowercase).collect()
}

/// True if `text` is (mostly) what Sunday just said: a transcript of our own speaker output.
pub fn is_echo(text: &str) -> bool {
    let recent = UNTIL.lock().unwrap().is_some_and(|t| Instant::now() < t + ECHO_WINDOW);
    let w = words(text);
    if !recent || w.len() < 2 {
        return false;
    }
    let said = words(&SAID.lock().unwrap());
    w.iter().filter(|x| said.contains(x)).count() * 10 >= w.len() * 8
}

static EPOCH: AtomicU64 = AtomicU64::new(0);

pub fn epoch() -> u64 {
    EPOCH.load(Ordering::SeqCst)
}

/// Barge-in: silence playback now and cancel everything stamped before this call.
pub fn interrupt() {
    eprintln!("[tts] interrupt");
    EPOCH.fetch_add(1, Ordering::SeqCst);
    QUEUE.lock().unwrap().clear();
    *UNTIL.lock().unwrap() = None;
}

/// When queued audio finishes playing.
static UNTIL: Mutex<Option<Instant>> = Mutex::new(None);
const TAIL: Duration = Duration::from_millis(400); // room echo

/// True while Sunday is audible, so the mic can ignore itself.
pub fn speaking() -> bool {
    UNTIL.lock().unwrap().is_some_and(|t| Instant::now() < t + TAIL)
}

/// Sunday's voice: a speaker id in kokoro-en-v0_19. 0 is the default American voice; 1..=4 other American women, 5..=6
/// American men, 7..=8 British women (bf_emma, bf_isabella), 9..=10 British men.
pub const VOICE: i32 = 0;
pub const SPEED: f32 = 1.0;

/// The voice and pace. SUNDAY_VOICE and SUNDAY_VOICE_SPEED try others without a rebuild.
fn voice() -> (i32, f32) {
    let get = |k: &str| std::env::var(k).ok().and_then(|v| v.parse().ok());
    (get("SUNDAY_VOICE").map_or(VOICE, |v| v as i32), get("SUNDAY_VOICE_SPEED").unwrap_or(SPEED))
}

pub struct Speaker {
    tts: OfflineTts,
    _stream: cpal::Stream,
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
        let _ = tts.generate_with_config("Hi.", &GenerationConfig { sid: voice().0, speed: voice().1, ..Default::default() }, none);

        let dev = cpal::default_host().default_output_device()?;
        let mut cfg = dev.default_output_config().ok()?.config();
        cfg.sample_rate = cpal::SampleRate(rate);
        cfg.channels = 1;
        // Tests run the whole path (queue, timing, level) but stay silent unless SUNDAY_AUDIBLE is set.
        let mute = cfg!(test) && std::env::var_os("SUNDAY_AUDIBLE").is_none();
        let stream = dev
            .build_output_stream(
                &cfg,
                move |out: &mut [f32], _| {
                    let mut q = QUEUE.lock().unwrap();
                    out.iter_mut().for_each(|s| *s = q.pop_front().unwrap_or(0.0));
                    let rms = (out.iter().map(|s| s * s).sum::<f32>() / out.len().max(1) as f32).sqrt();
                    let prev = playback_level();
                    PLAY_LEVEL.store((prev + 0.1 * (rms - prev)).to_bits(), Ordering::Relaxed);
                    if mute {
                        out.fill(0.0);
                    }
                },
                |e| eprintln!("tts output error: {e}"),
                None,
            )
            .ok()?;
        stream.play().ok()?;
        Some(Self { tts, _stream: stream })
    }

    /// Synthesizes at the model's sample rate. Blocks for the synthesis time (RTF is well under 1).
    pub fn synth(&self, text: &str) -> Vec<f32> {
        let (sid, speed) = voice();
        self.synth_with(text, sid, speed)
    }

    /// Same, in another of the model's voices (speaker id).
    #[cfg(test)]
    pub fn synth_sid(&self, text: &str, sid: i32) -> Vec<f32> {
        self.synth_with(text, sid, 1.0)
    }

    fn synth_with(&self, text: &str, sid: i32, speed: f32) -> Vec<f32> {
        let cfg = GenerationConfig { sid, speed, ..Default::default() };
        let none: Option<fn(&[f32], f32) -> bool> = None;
        self.tts.generate_with_config(text, &cfg, none).map(|a| a.samples().to_vec()).unwrap_or_default()
    }

    /// Queues the audio unless an interrupt happened since `epoch`. False if dropped.
    pub fn speak(&self, text: &str, epoch: u64) -> bool {
        let audio = self.synth(text);
        if !self.enqueue_if(&audio, epoch) {
            eprintln!("[tts] dropped a sentence: interrupted while it was being made");
            return false;
        }
        let mut said = SAID.lock().unwrap();
        said.push_str(text);
        said.push(' ');
        if said.len() > 800 {
            let cut = said.len() - 800;
            let cut = (cut..said.len()).find(|i| said.is_char_boundary(*i)).unwrap_or(0);
            said.drain(..cut);
        }
        true
    }

    /// Queues the audio only if no interrupt has happened since `epoch`. The check and the push happen under the same locks
    /// `interrupt` takes to clear the queue, so a barge-in can't slip between them and leave stale audio playing.
    fn enqueue_if(&self, samples: &[f32], epoch: u64) -> bool {
        let secs = samples.len() as f64 / self.tts.sample_rate() as f64;
        let mut until = UNTIL.lock().unwrap();
        let mut queue = QUEUE.lock().unwrap();
        if epoch != self::epoch() {
            return false;
        }
        *until = Some(until.filter(|t| *t > Instant::now()).unwrap_or_else(Instant::now) + Duration::from_secs_f64(secs));
        queue.extend(samples);
        true
    }

    fn enqueue(&self, samples: &[f32]) {
        let secs = samples.len() as f64 / self.tts.sample_rate() as f64;
        let mut until = UNTIL.lock().unwrap();
        *until = Some(until.filter(|t| *t > Instant::now()).unwrap_or_else(Instant::now) + Duration::from_secs_f64(secs));
        QUEUE.lock().unwrap().extend(samples);
    }

    /// Two soft rising notes: "heard you, working on it".
    fn chime(&self) {
        let rate = self.tts.sample_rate() as f32;
        let mut out = Vec::new();
        for (hz, ms) in [(659.25_f32, 110.0_f32), (880.0, 260.0)] {
            for i in 0..(rate * ms / 1000.0) as usize {
                let t = i as f32 / rate;
                out.push(0.18 * (-t * 12.0).exp() * (std::f32::consts::TAU * hz * t).sin());
            }
        }
        self.enqueue(&out);
    }
}

pub enum Cmd {
    /// Text, stamped with the `epoch()` it was requested under.
    Speak(String, u64),
    Chime,
}

/// cpal streams aren't Send, so the speaker lives on its own thread; talk to it by channel.
pub fn spawn(app: tauri::AppHandle) -> std::sync::mpsc::Sender<Cmd> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let Some(speaker) = Speaker::new() else { return eprintln!("tts unavailable") };
        for cmd in rx {
            match cmd {
                Cmd::Speak(t, epoch) => {
                    // Words go on screen as their audio starts, not before.
                    if speaker.speak(&t, epoch) {
                        let _ = tauri::Emitter::emit(&app, "say", t.trim());
                    }
                    PENDING.fetch_sub(1, Ordering::SeqCst);
                }
                Cmd::Chime => speaker.chime(),
            }
        }
    });
    tx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_own_echo() {
        *SAID.lock().unwrap() = "I can answer questions and help you with information. ".into();
        assert!(!is_echo("answer questions")); // nothing has just finished playing
        *UNTIL.lock().unwrap() = Some(Instant::now());
        assert!(is_echo("I can answer questions and help"));
        assert!(!is_echo("what is the weather in Paris"));
        assert!(!is_echo("answer")); // one word is too ambiguous to drop
        *UNTIL.lock().unwrap() = Some(Instant::now() - Duration::from_secs(10));
        assert!(!is_echo("I can answer questions and help")); // long ago: a real person
    }

    /// Plays the same sentence in a few voices and paces, so one can be picked by ear:
    /// `SUNDAY_AUDIBLE=1 cargo test speaks_aloud -- --ignored --nocapture`, then set SUNDAY_VOICE and SUNDAY_VOICE_SPEED (or the constants).
    #[test]
    #[ignore = "needs the speech models from scripts/fetch-models.sh and a speaker"]
    fn speaks_aloud() {
        let s = Speaker::new().expect("tts unavailable");
        for (i, (sid, speed)) in [(0, 1.0), (0, 1.1), (7, 1.0), (8, 1.0)].into_iter().enumerate() {
            println!("option {}: voice {sid} speed {speed}", i + 1);
            s.enqueue(&s.synth_with(&format!("Option {}. Good evening. I've opened Notes. Is there anything else you need?", i + 1), sid, speed));
            std::thread::sleep(UNTIL.lock().unwrap().unwrap().saturating_duration_since(Instant::now()) + Duration::from_millis(700));
        }
    }

    /// Needs the models from scripts/fetch-models.sh. Prints synthesis time vs audio length.
    #[test]
    #[ignore = "needs the speech models from scripts/fetch-models.sh; a benchmark"]
    fn synthesis_speed() {
        let s = Speaker::new().expect("tts unavailable");
        for text in ["Hello there.", "I can answer questions, write different kinds of creative text, and help you with information.", "Just let me know what you need."] {
            let t = Instant::now();
            s.speak(text, epoch());
            let wait = UNTIL.lock().unwrap().unwrap().saturating_duration_since(Instant::now());
            println!("synth {:?} for {:?} of audio: {text}", t.elapsed(), wait);
        }
    }
}
