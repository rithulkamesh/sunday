//! Always on and tiny: Silero VAD + a keyword spotter ("sunday", "stop"). Parakeet TDT (hundreds
//! of MB) is loaded only after a wake word or fn, and dropped after IDLE. Once loaded it decodes
//! the growing utterance every 0.2 s so words appear live, then once more when the VAD closes it.
use crate::models::path;
use crate::speaker::{self, Voice};
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

/// Each speech event carries a similarity to the owner's voiceprint (None: not enrolled, or too little audio to tell).
pub enum Heard {
    /// The utterance so far; will be superseded.
    Interim(String, Option<f32>),
    /// A finished utterance.
    Final(String, Option<f32>),
    /// A wake/stop word was spotted ("sunday" or "stop"), even while muted.
    Keyword(String, Option<f32>),
    /// Seconds of speech collected so far while learning the owner's voice.
    Enrolling(f32),
    /// Learning finished: the voiceprint, or why it failed.
    Enrolled(Result<Vec<f32>, String>),
}

/// A short sign-off ("bye", "okay goodbye", "that's all") ends the conversation.
pub fn is_goodbye(text: &str) -> bool {
    let t = text.to_lowercase();
    let w: Vec<&str> = t.split(|c: char| !c.is_alphanumeric() && c != '\'').filter(|w| !w.is_empty()).collect();
    w.len() <= 4 && (w.iter().any(|x| matches!(*x, "bye" | "goodbye" | "goodnight")) || t.contains("that's all") || t.contains("stop listening"))
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
    /// Speaker verification (None if its model isn't fetched) and the owner's voiceprint (None until enrolled).
    voice: Option<Voice>,
    print: Option<Vec<f32>>,
    /// The last 3 s of mic audio, for judging whose voice a wake word was.
    recent: Vec<f32>,
    /// Speech collected for a voiceprint while enrolling.
    enrolling: Option<Vec<f32>>,
    /// Similarity for the utterance in progress, and the buffer length it was computed at.
    utt_sim: Option<f32>,
    sim_at: usize,
    voice_vad: VoiceActivityDetector,
    muted_rest: Vec<f32>, // samples not yet a full VAD window
    was_muted: bool,
    muted_samples: usize, // fed to `voice_vad` since its last reset
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
        let vad_cfg = |min_silence: f32, min_speech: f32, max_speech: f32| {
            let mut vad = VadModelConfig::default();
            vad.silero_vad.model = Some(path("silero_vad.onnx"));
            vad.silero_vad.threshold = 0.5;
            vad.silero_vad.min_silence_duration = min_silence;
            vad.silero_vad.min_speech_duration = min_speech;
            vad.silero_vad.max_speech_duration = max_speech;
            vad.silero_vad.window_size = WINDOW as i32;
            vad.sample_rate = 16_000;
            vad
        };
        let vad = vad_cfg(0.4, 0.2, 20.0);

        let kws = load_kws()?;
        Some(Self {
            vad: VoiceActivityDetector::create(&vad, 60.0)?,
            voice: Voice::new(),
            print: None,
            recent: Vec::new(),
            enrolling: None,
            utt_sim: None,
            sim_at: 0,
            // Quicker to react; only used while muted, to tell voices from claps.
            voice_vad: VoiceActivityDetector::create(&vad_cfg(0.2, 0.1, 20.0), 5.0)?,
            muted_rest: Vec::new(),
            was_muted: false,
            muted_samples: 0,
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

    /// While muted: does the mic currently sound like speech (rather than a clap or a thump)?
    pub fn voiced(&self) -> bool {
        self.was_muted && self.voice_vad.detected()
    }

    /// Conversation over: drop the big model now instead of waiting out IDLE.
    pub fn cool(&mut self) {
        self.recognizer = None;
        self.buf.clear();
        self.fed = 0;
        self.speaking = false;
        eprintln!("[stt] cooled, model unloaded");
    }

    pub fn set_voiceprint(&mut self, p: Option<Vec<f32>>) {
        self.print = p;
    }

    pub fn start_enroll(&mut self) {
        self.enrolling = Some(Vec::new());
    }

    pub fn cancel_enroll(&mut self) {
        self.enrolling = None;
    }

    /// Is this the owner? Some(true/false) once a voiceprint exists and there was enough audio to judge;
    /// None when unknown. `mixed`: Sunday's own voice was playing too, so the bar is lower.
    pub fn is_owner(&self, sim: Option<f32>, mixed: bool) -> Option<bool> {
        if self.print.is_none() {
            return Some(true);
        }
        sim.map(|s| s >= if mixed { speaker::MIXED } else { speaker::SAME })
    }

    /// Similarity of the last 2 s of mic audio to the owner (for wake words and barge-in).
    pub fn recent_sim(&self) -> Option<f32> {
        let (v, p) = (self.voice.as_ref()?, self.print.as_ref()?);
        v.similarity(p, &self.recent[self.recent.len().saturating_sub(2 * 16_000)..])
    }

    fn sim_of(&self, samples: &[f32]) -> Option<f32> {
        self.voice.as_ref()?.similarity(self.print.as_ref()?, samples)
    }

    /// Load the big model now (wake word or fn) and keep it for IDLE.
    pub fn warm(&mut self) {
        self.last_active = Instant::now();
        if self.recognizer.is_none() {
            self.recognizer = load_recognizer();
        }
    }

    /// While learning a voice: keep only closed speech segments, report progress, and finish at ENROLL_SECS.
    fn enroll_step(&mut self, mut out: Vec<Heard>) -> Vec<Heard> {
        let mut closed = false;
        while !self.vad.is_empty() {
            if let (Some(seg), Some(e)) = (self.vad.front(), self.enrolling.as_mut()) {
                let s = seg.samples();
                if s.len() >= 9_600 {
                    e.extend(s.iter().copied());
                    out.push(Heard::Enrolling(e.len() as f32 / 16_000.0));
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
            let drop = self.buf.len() - 10 * WINDOW;
            self.buf.drain(..drop);
            self.fed = self.fed.saturating_sub(drop);
        }
        if self.enrolling.as_ref().is_some_and(|e| e.len() as f32 / 16_000.0 >= speaker::ENROLL_SECS) {
            let speech = self.enrolling.take().unwrap_or_default();
            let r = match &self.voice {
                Some(v) => v.enroll(&speech),
                None => Err("the speaker model is missing: run scripts/fetch-models.sh".into()),
            };
            if let Ok(p) = &r {
                self.print = Some(p.clone());
            }
            out.push(Heard::Enrolled(r));
        }
        out
    }

    /// Feed 16 kHz mono samples; returns whatever was heard since the last call.
    /// `muted` (we're speaking): only the keyword spotter listens, so "stop" can barge in.
    pub fn push(&mut self, chunk: &[f32], muted: bool) -> Vec<Heard> {
        let mut out = Vec::new();
        self.recent.extend_from_slice(chunk);
        let extra = self.recent.len().saturating_sub(3 * 16_000);
        self.recent.drain(..extra);
        self.kws_stream.accept_waveform(16_000, chunk);
        while self.kws.is_ready(&self.kws_stream) {
            self.kws.decode(&self.kws_stream);
            if let Some(r) = self.kws.get_result(&self.kws_stream).filter(|r| !r.keyword.is_empty()) {
                self.kws.reset(&self.kws_stream);
                if self.enrolling.is_some() {
                    continue; // learning a voice: nothing else should react
                }
                let sim = self.recent_sim();
                if r.keyword == "sunday" && !muted && self.is_owner(sim, false) != Some(false) {
                    self.warm();
                }
                out.push(Heard::Keyword(r.keyword, sim));
            }
        }
        if muted {
            // Keep a second VAD on the mic so barge-in can tell speech from a clap.
            self.was_muted = true;
            self.muted_rest.extend_from_slice(chunk);
            let n = self.muted_rest.len() / WINDOW * WINDOW;
            for w in self.muted_rest[..n].chunks(WINDOW) {
                self.voice_vad.accept_waveform(w);
            }
            self.muted_rest.drain(..n);
            while !self.voice_vad.is_empty() {
                self.voice_vad.pop();
            }
            // Its buffer holds 5 s and our own voice never pauses to let it drain: start over every 3 s.
            self.muted_samples += n;
            if self.muted_samples > 3 * 16_000 {
                self.voice_vad.reset();
                self.muted_samples = 0;
            }
            return out;
        }
        if self.was_muted {
            self.was_muted = false;
            self.voice_vad.reset();
            self.muted_rest.clear();
            self.muted_samples = 0;
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
            self.utt_sim = None;
            self.sim_at = 0;
        }
        if self.enrolling.is_some() {
            return self.enroll_step(out);
        }

        // Speech keeps a loaded model warm; it never loads one (only a wake word or fn does).
        if self.recognizer.is_some() && (self.speaking || !self.vad.is_empty()) {
            self.last_active = Instant::now();
        }

        // Live words while the utterance is still going.
        if let (true, Some(rec)) = (self.speaking && self.since_interim >= INTERIM_EVERY, &self.recognizer) {
            let text = decode(rec, &self.buf);
            // Whose voice is it so far? Re-judged as the utterance grows (needs ~0.75 s, uses the last 4 s).
            if self.print.is_some() && self.buf.len() >= self.sim_at + 8_000 {
                self.utt_sim = self.sim_of(&self.buf[self.buf.len().saturating_sub(4 * 16_000)..]);
                self.sim_at = self.buf.len();
            }
            if !text.is_empty() {
                out.push(Heard::Interim(text, self.utt_sim));
            }
            self.since_interim = 0;
        }

        // Closed segments: final decode, then start over.
        let mut closed = false;
        while !self.vad.is_empty() {
            if let Some(seg) = self.vad.front() {
                let text = self.recognizer.as_ref().map(|r| decode(r, seg.samples())).unwrap_or_default();
                if !text.is_empty() {
                    let sim = self.sim_of(&seg.samples());
                    out.push(Heard::Final(text, sim));
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
            Heard::Final(t, _) => Some(t.clone()),
            _ => None,
        }).collect();
        assert!(!finals.is_empty(), "no final transcript");
        println!("{finals:?}");
        assert!(heard.iter().any(|h| matches!(h, Heard::Interim(..))), "no live words");

        // Goes cold after IDLE of silence, and wakes again on speech.
        assert!(stt.recognizer.is_some());
        stt.last_active = Instant::now() - IDLE - Duration::from_secs(1);
        stt.push(&silence[..1600], false);
        assert!(stt.recognizer.is_none(), "model still loaded when idle");
    }

    #[test]
    fn goodbyes() {
        for yes in ["Bye.", "Okay, goodbye!", "Thanks, bye bye", "That's all.", "Sunday, stop listening"] {
            assert!(is_goodbye(yes), "{yes}");
        }
        for no in ["What's the weather", "Say bye to my mom in French please", "Goodbye is a long word to spell"] {
            assert!(!is_goodbye(no), "{no}");
        }
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
                Heard::Keyword(k, _) => Some(k),
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

    /// Sunday's own voice never pauses; the quick VAD used while muted must keep flushing it
    /// (sherpa prints "circular-buffer ... Overflow!" to stderr if segments pile up).
    #[test]
    fn long_muted_speech_is_flushed() {
        let tts = crate::tts::Speaker::new().expect("tts unavailable");
        let rs = sherpa_onnx::LinearResampler::create(24_000, 16_000).unwrap();
        let mut stt = Stt::new().expect("models missing");
        let said = "This is a long reply that keeps going without any real pause between the sentences so the detector sees one continuous stretch of speech. ".repeat(4);
        let audio = rs.resample(&tts.synth(&said), true);
        assert!(audio.len() > 16_000 * 12, "need >12 s of speech, got {}", audio.len() / 16_000);
        for c in audio.chunks(1600) {
            stt.push(c, true);
        }
    }

    /// Learn a voice from ~20 s of speech, then: the owner's wake word scores high and wakes the model,
    /// another voice scores lower. (Kokoro voices stand in for people; they resemble each other, so only
    /// the ordering is asserted here, and the real thresholds are tuned from the logged scores.)
    #[test]
    fn enrolls_then_verifies() {
        let tts = crate::tts::Speaker::new().expect("tts unavailable");
        let rs = sherpa_onnx::LinearResampler::create(24_000, 16_000).unwrap();
        let say = |sid: i32, t: &str| {
            let mut a = vec![0.0f32; 12_000];
            a.extend(rs.resample(&tts.synth_sid(t, sid), true));
            a.extend(vec![0.0f32; 16_000]); // a pause, so the VAD closes the segment
            a
        };
        let mut stt = Stt::new().expect("models missing");
        stt.start_enroll();
        let mut enrolled = None;
        let lines = ["The scheduler picks the next process from the ready queue.", "A binary search tree keeps its keys in order.",
            "Dynamic programming stores answers to small subproblems.", "A transaction either happens completely or not at all.",
            "Congestion control slows the sender when packets are dropped.", "Paging splits memory into fixed size frames and pages."];
        'feed: for t in lines.iter().cycle().take(12) {
            for c in say(0, t).chunks(1600) {
                for h in stt.push(c, false) {
                    if let Heard::Enrolled(r) = h {
                        enrolled = Some(r);
                        break 'feed;
                    }
                }
            }
        }
        let print = enrolled.expect("enrollment never finished").expect("enrollment failed");
        assert!(stt.print.is_some() && print.len() > 100);

        let mut hear = |sid: i32| {
            stt.recognizer = None;
            let sims: Vec<Option<f32>> = say(sid, "Sunday, what is a deadlock?").chunks(1600).flat_map(|c| stt.push(c, false)).filter_map(|h| match h {
                Heard::Keyword(k, sim) if k == "sunday" => Some(sim),
                _ => None,
            }).collect();
            (sims.first().copied().flatten(), stt.recognizer.is_some())
        };
        let (own, woke) = hear(0);
        let (other, other_woke) = hear(6);
        println!("owner: sim {own:?} woke {woke}; other voice: sim {other:?} woke {other_woke}");
        let own = own.expect("no sim for the owner");
        assert!(own >= speaker::SAME && woke, "the owner should wake it");
        assert!(other.is_none_or(|o| o < own), "another voice scored at least as high as the owner");
    }
}
