//! Always on and tiny: Silero VAD + a keyword spotter ("sunday", "stop"). Parakeet TDT (hundreds
//! of MB) is loaded only after a wake word or fn, and dropped after IDLE. Once loaded it decodes
//! the growing utterance every 0.2 s so words appear live, then once more when the VAD closes it.
use crate::models::path;
use std::time::{Duration, Instant};
use sherpa_onnx::{
    KeywordSpotter, KeywordSpotterConfig, OfflineRecognizer, OfflineRecognizerConfig, OnlineStream,
    VadModelConfig, VoiceActivityDetector,
};

const WINDOW: usize = 512; // Silero's fixed window at 16 kHz
const INTERIM_EVERY: usize = 3_200; // samples: 0.2 s of audio
/// Parakeet is dropped this long after the last speech; the tiny VAD keeps listening.
const IDLE: Duration = Duration::from_secs(10);
const KWS: &str = "sherpa-onnx-kws-zipformer-gigaspeech-3.3M-2024-01-01";
// BPE pieces from the model's bpe.model; @name is what the spotter reports.
const KEYWORDS: &str = "▁SU N DAY @sunday\n▁ST O P @stop\n";
const PARAKEET: &str = "sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8";

pub enum Heard {
    /// The utterance so far; will be superseded.
    Interim(String),
    /// A finished utterance.
    Final(String),
    /// A wake/stop word was spotted ("sunday" or "stop"), even while muted.
    Keyword(String),
}

fn load_kws() -> Option<KeywordSpotter> {
    let mut c = KeywordSpotterConfig::default();
    let t = &mut c.model_config;
    let chunk = "epoch-12-avg-2-chunk-16-left-64.int8.onnx";
    t.transducer.encoder = Some(path(&format!("{KWS}/encoder-{chunk}")));
    t.transducer.decoder = Some(path(&format!("{KWS}/decoder-{chunk}")));
    t.transducer.joiner = Some(path(&format!("{KWS}/joiner-{chunk}")));
    t.tokens = Some(path(&format!("{KWS}/tokens.txt")));
    t.num_threads = 1;
    c.keywords_buf = Some(KEYWORDS.into());
    KeywordSpotter::create(&c)
}

fn load_recognizer() -> Option<OfflineRecognizer> {
    let t0 = Instant::now();
    let mut asr = OfflineRecognizerConfig::default();
    let t = &mut asr.model_config;
    t.transducer.encoder = Some(path(&format!("{PARAKEET}/encoder.int8.onnx")));
    t.transducer.decoder = Some(path(&format!("{PARAKEET}/decoder.int8.onnx")));
    t.transducer.joiner = Some(path(&format!("{PARAKEET}/joiner.int8.onnx")));
    t.tokens = Some(path(&format!("{PARAKEET}/tokens.txt")));
    t.model_type = Some("nemo_transducer".into());
    t.num_threads = 4;

    let r = OfflineRecognizer::create(&asr);
    eprintln!("[stt] model loaded in {:?}", t0.elapsed());
    r
}

fn decode(recognizer: &OfflineRecognizer, samples: &[f32]) -> String {
    let stream = recognizer.create_stream();
    stream.accept_waveform(16_000, samples);
    recognizer.decode(&stream);
    stream.get_result().map(|r| r.text.trim().to_string()).unwrap_or_default()
}

pub struct Stt {
    vad: VoiceActivityDetector,
    kws: KeywordSpotter,
    kws_stream: OnlineStream,
    /// Loaded on demand (hundreds of MB), dropped after IDLE.
    recognizer: Option<OfflineRecognizer>,
    last_active: Instant,
    buf: Vec<f32>,
    fed: usize,
    speaking: bool,
    /// Samples received since the last interim decode (audio time, not wall time).
    since_interim: usize,
}

impl Stt {
    /// None (with a hint) if the models haven't been fetched.
    pub fn new() -> Option<Self> {
        if !std::path::Path::new(&path("silero_vad.onnx")).exists() {
            eprintln!("models missing: run scripts/fetch-models.sh");
            return None;
        }
        let mut vad = VadModelConfig::default();
        vad.silero_vad.model = Some(path("silero_vad.onnx"));
        vad.silero_vad.threshold = 0.5;
        vad.silero_vad.min_silence_duration = 0.4;
        vad.silero_vad.min_speech_duration = 0.2;
        vad.silero_vad.max_speech_duration = 20.0;
        vad.silero_vad.window_size = WINDOW as i32;
        vad.sample_rate = 16_000;

        let kws = load_kws()?;
        Some(Self {
            vad: VoiceActivityDetector::create(&vad, 60.0)?,
            kws_stream: kws.create_stream(),
            kws,
            recognizer: None,
            last_active: Instant::now(),
            buf: Vec::new(),
            fed: 0,
            speaking: false,
            since_interim: 0,
        })
    }

    /// Load the big model now (wake word or fn) and keep it for IDLE.
    pub fn warm(&mut self) {
        self.last_active = Instant::now();
        if self.recognizer.is_none() {
            self.recognizer = load_recognizer();
        }
    }

    /// Feed 16 kHz mono samples; returns whatever was heard since the last call.
    /// `muted` (we're speaking): only the keyword spotter listens, so "stop" can barge in.
    pub fn push(&mut self, chunk: &[f32], muted: bool) -> Vec<Heard> {
        let mut out = Vec::new();
        self.kws_stream.accept_waveform(16_000, chunk);
        while self.kws.is_ready(&self.kws_stream) {
            self.kws.decode(&self.kws_stream);
            if let Some(r) = self.kws.get_result(&self.kws_stream).filter(|r| !r.keyword.is_empty()) {
                self.kws.reset(&self.kws_stream);
                if r.keyword == "sunday" && !muted {
                    self.warm();
                }
                out.push(Heard::Keyword(r.keyword));
            }
        }
        if muted {
            return out;
        }
        self.buf.extend_from_slice(chunk);
        self.since_interim += chunk.len();

        while self.fed + WINDOW <= self.buf.len() {
            self.vad.accept_waveform(&self.buf[self.fed..self.fed + WINDOW]);
            self.fed += WINDOW;
        }
        if self.vad.detected() && !self.speaking {
            self.speaking = true;
            self.since_interim = 0;
        }

        // Speech keeps a loaded model warm; it never loads one (only a wake word or fn does).
        if self.recognizer.is_some() && (self.speaking || !self.vad.is_empty()) {
            self.last_active = Instant::now();
        }

        // Live words while the utterance is still going.
        if let (true, Some(rec)) = (self.speaking && self.since_interim >= INTERIM_EVERY, &self.recognizer) {
            let text = decode(rec, &self.buf);
            if !text.is_empty() {
                out.push(Heard::Interim(text));
            }
            self.since_interim = 0;
        }

        // Closed segments: final decode, then start over.
        let mut closed = false;
        while !self.vad.is_empty() {
            if let Some(seg) = self.vad.front() {
                let text = self.recognizer.as_ref().map(|r| decode(r, seg.samples())).unwrap_or_default();
                if !text.is_empty() {
                    out.push(Heard::Final(text));
                }
            }
            self.vad.pop();
            closed = true;
        }
        if closed {
            self.buf.clear();
            self.fed = 0;
            self.speaking = false;
        } else if !self.speaking && self.buf.len() > 10 * WINDOW {
            // Idle: keep only a short lead-in so the buffer doesn't grow forever.
            let drop = self.buf.len() - 10 * WINDOW;
            self.buf.drain(..drop);
            self.fed = self.fed.saturating_sub(drop);
        }
        if !self.speaking && self.recognizer.is_some() && self.last_active.elapsed() > IDLE {
            self.recognizer = None;
            eprintln!("[stt] idle, model unloaded");
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Needs the models from scripts/fetch-models.sh.
    #[test]
    fn transcribes_a_wav() {
        let wav = sherpa_onnx::Wave::read(&path(&format!("{PARAKEET}/test_wavs/0.wav"))).unwrap();
        let mut stt = Stt::new().expect("models missing");
        stt.warm(); // cold until a wake word; the wav has none
        let mut heard = Vec::new();
        let silence = vec![0.0f32; 16_000]; // lets the VAD close the segment
        for chunk in wav.samples().chunks(1600).chain(silence.chunks(1600)).chain(silence.chunks(1600)) {
            heard.extend(stt.push(chunk, false));
        }
        let finals: Vec<_> = heard.iter().filter_map(|h| match h {
            Heard::Final(t) => Some(t.clone()),
            _ => None,
        }).collect();
        assert!(!finals.is_empty(), "no final transcript");
        println!("{finals:?}");
        assert!(heard.iter().any(|h| matches!(h, Heard::Interim(_))), "no live words");

        // Goes cold after IDLE of silence, and wakes again on speech.
        assert!(stt.recognizer.is_some());
        stt.last_active = Instant::now() - IDLE - Duration::from_secs(1);
        stt.push(&silence[..1600], false);
        assert!(stt.recognizer.is_none(), "model still loaded when idle");
    }

    /// Kokoro says it, the spotter must hear it: wake on "Sunday", not on "Monday".
    #[test]
    fn spots_keywords() {
        let tts = crate::tts::Speaker::new().expect("tts unavailable");
        let rs = sherpa_onnx::LinearResampler::create(24_000, 16_000).unwrap();
        let run = |text: &str, muted: bool| {
            let mut stt = Stt::new().expect("models missing");
            let mut audio = vec![0.0f32; 8_000];
            audio.extend(rs.resample(&tts.synth(text), true));
            audio.extend(vec![0.0f32; 16_000]);
            let heard: Vec<String> = audio.chunks(1600).flat_map(|c| stt.push(c, muted)).filter_map(|h| match h {
                Heard::Keyword(k) => Some(k),
                _ => None,
            }).collect();
            println!("{text:?} muted={muted} -> {heard:?} warm={}", stt.recognizer.is_some());
            (heard, stt.recognizer.is_some())
        };
        assert_eq!(run("Sunday, what time is it?", false), (vec!["sunday".to_string()], true));
        assert!(run("Monday, what time is it?", false).0.is_empty());
        assert!(run("Please stop talking right now.", true).0.contains(&"stop".to_string()));
        assert_eq!(run("Sunday, wait", true), (vec!["sunday".to_string()], false)); // muted: no model load
    }
}
