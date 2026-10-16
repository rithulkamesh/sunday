//! Silero VAD -> Parakeet TDT (sherpa-onnx). Decodes the growing utterance every 0.2 s so
//! words appear live, then once more when the VAD closes the segment.
use crate::models::path;
use sherpa_onnx::{
    OfflineRecognizer, OfflineRecognizerConfig, VadModelConfig, VoiceActivityDetector,
};

const WINDOW: usize = 512; // Silero's fixed window at 16 kHz
const INTERIM_EVERY: usize = 3_200; // samples: 0.2 s of audio
const PARAKEET: &str = "sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8";

pub enum Heard {
    /// The utterance so far; will be superseded.
    Interim(String),
    /// A finished utterance.
    Final(String),
}

pub struct Stt {
    vad: VoiceActivityDetector,
    recognizer: OfflineRecognizer,
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

        let mut asr = OfflineRecognizerConfig::default();
        let t = &mut asr.model_config;
        t.transducer.encoder = Some(path(&format!("{PARAKEET}/encoder.int8.onnx")));
        t.transducer.decoder = Some(path(&format!("{PARAKEET}/decoder.int8.onnx")));
        t.transducer.joiner = Some(path(&format!("{PARAKEET}/joiner.int8.onnx")));
        t.tokens = Some(path(&format!("{PARAKEET}/tokens.txt")));
        t.model_type = Some("nemo_transducer".into());
        t.num_threads = 4;

        Some(Self {
            vad: VoiceActivityDetector::create(&vad, 60.0)?,
            recognizer: OfflineRecognizer::create(&asr)?,
            buf: Vec::new(),
            fed: 0,
            speaking: false,
            since_interim: 0,
        })
    }

    fn decode(&self, samples: &[f32]) -> String {
        let stream = self.recognizer.create_stream();
        stream.accept_waveform(16_000, samples);
        self.recognizer.decode(&stream);
        stream.get_result().map(|r| r.text.trim().to_string()).unwrap_or_default()
    }

    /// Feed 16 kHz mono samples; returns whatever was heard since the last call.
    pub fn push(&mut self, chunk: &[f32]) -> Vec<Heard> {
        let mut out = Vec::new();
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

        // Live words while the utterance is still going.
        if self.speaking && self.since_interim >= INTERIM_EVERY {
            let text = self.decode(&self.buf);
            if !text.is_empty() {
                out.push(Heard::Interim(text));
            }
            self.since_interim = 0;
        }

        // Closed segments: final decode, then start over.
        let mut closed = false;
        while !self.vad.is_empty() {
            if let Some(seg) = self.vad.front() {
                let text = self.decode(seg.samples());
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
        let mut heard = Vec::new();
        let silence = vec![0.0f32; 16_000]; // lets the VAD close the segment
        for chunk in wav.samples().chunks(1600).chain(silence.chunks(1600)).chain(silence.chunks(1600)) {
            heard.extend(stt.push(chunk));
        }
        let finals: Vec<_> = heard.iter().filter_map(|h| match h {
            Heard::Final(t) => Some(t.clone()),
            _ => None,
        }).collect();
        assert!(!finals.is_empty(), "no final transcript");
        println!("{finals:?}");
        assert!(heard.iter().any(|h| matches!(h, Heard::Interim(_))), "no live words");
    }
}
