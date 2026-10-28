//! "Is this me?": speaker embeddings (wespeaker ResNet34, via sherpa-onnx) compared by cosine
//! similarity against a voiceprint learned from about 20 s of the owner's speech.
use crate::voice::models::path;
use sherpa_onnx::{SpeakerEmbeddingExtractor, SpeakerEmbeddingExtractorConfig};

const MODEL: &str = "wespeaker_en_voxceleb_resnet34_LM.onnx";
/// Shorter audio gives unreliable embeddings: the caller treats it as "unknown".
const MIN_SAMPLES: usize = 16_000 * 3 / 4;
/// Similarity at or above this is the owner (clean speech).
pub const SAME: f32 = 0.45;
/// Lower bar for audio that has Sunday's own voice mixed in (barge-in while it talks).
pub const MIXED: f32 = 0.30;
/// Enrollment collects this much speech, split into windows that are embedded separately.
pub const ENROLL_SECS: f32 = 20.0;
const WINDOW: usize = 16_000 * 5;

pub struct Voice(SpeakerEmbeddingExtractor);

impl Voice {
    /// None if the model hasn't been fetched (scripts/fetch-models.sh).
    pub fn new() -> Option<Self> {
        if !std::path::Path::new(&path(MODEL)).exists() {
            return None;
        }
        let cfg = SpeakerEmbeddingExtractorConfig { model: Some(path(MODEL)), num_threads: 1, debug: false, provider: None };
        SpeakerEmbeddingExtractor::create(&cfg).map(Self)
    }

    /// 16 kHz mono samples -> unit-length embedding, or None if there isn't enough audio.
    pub fn embed(&self, samples: &[f32]) -> Option<Vec<f32>> {
        if samples.len() < MIN_SAMPLES {
            return None;
        }
        let s = self.0.create_stream()?;
        s.accept_waveform(16_000, samples);
        s.input_finished();
        if !self.0.is_ready(&s) {
            return None;
        }
        self.0.compute(&s).map(unit)
    }

    /// Similarity of `samples` to a voiceprint; None if the audio is too short to judge.
    pub fn similarity(&self, print: &[f32], samples: &[f32]) -> Option<f32> {
        self.embed(samples).map(|e| cosine(print, &e))
    }

    /// Builds a voiceprint from the enrollment speech. Fails if the windows disagree with each other
    /// (several speakers, or mostly noise), since a bad voiceprint would lock the owner out.
    pub fn enroll(&self, speech: &[f32]) -> Result<Vec<f32>, String> {
        let embs: Vec<Vec<f32>> = speech.chunks(WINDOW).filter_map(|w| self.embed(w)).collect();
        combine(&embs)
    }
}

fn unit(mut v: Vec<f32>) -> Vec<f32> {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
    v.iter_mut().for_each(|x| *x /= n);
    v
}

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let (dot, na, nb) = a.iter().zip(b).fold((0.0, 0.0, 0.0), |(d, na, nb), (x, y)| (d + x * y, na + x * x, nb + y * y));
    dot / (na.sqrt() * nb.sqrt()).max(1e-9)
}

/// Mean of the window embeddings, required to be consistent.
pub fn combine(embs: &[Vec<f32>]) -> Result<Vec<f32>, String> {
    if embs.len() < 3 {
        return Err("not enough speech to learn your voice; try again".into());
    }
    let mut mean = vec![0.0; embs[0].len()];
    for e in embs {
        mean.iter_mut().zip(e).for_each(|(m, x)| *m += x / embs.len() as f32);
    }
    let mean = unit(mean);
    if embs.iter().any(|e| cosine(e, &mean) < 0.6) {
        return Err("the recording was inconsistent (other voices or noise); try again somewhere quiet".into());
    }
    Ok(mean)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combines_consistent_embeddings_only() {
        let a = vec![1.0, 0.1, 0.0];
        let b = vec![0.9, 0.2, 0.1];
        let c = vec![1.0, 0.0, 0.1];
        let m = combine(&[a.clone(), b, c]).unwrap();
        assert!((m.iter().map(|x| x * x).sum::<f32>() - 1.0).abs() < 1e-4, "unit length");
        assert!(cosine(&m, &a) > 0.95);
        assert!(combine(&[a.clone(), vec![-1.0, 0.0, 0.0], vec![0.0, 1.0, 0.0]]).is_err());
        assert!(combine(&[a.clone(), a]).is_err()); // too little speech
    }

    /// Needs the models. Kokoro's other voices stand in for other people: same voice must score
    /// well above them. Prints the numbers the thresholds are chosen from.
    #[test]
    #[ignore = "needs the speech models from scripts/fetch-models.sh"]
    fn tells_voices_apart() {
        let tts = crate::voice::tts::Speaker::new().expect("tts unavailable");
        let rs = sherpa_onnx::LinearResampler::create(24_000, 16_000).unwrap();
        let voice = Voice::new().expect("speaker model missing: scripts/fetch-models.sh");
        let say = |sid: i32, t: &str| rs.resample(&tts.synth_sid(t, sid), true);
        let train = [
            "The scheduler picks the next process from the ready queue and gives it the processor for a short time slice.",
            "A binary search tree keeps its keys in order, so lookups follow one path from the root down to a leaf.",
            "Dynamic programming stores the answers to small subproblems so that each one is only computed once.",
            "In a relational database, a transaction groups several operations so that they either all happen or none do.",
        ];
        let mut speech = Vec::new();
        for t in train {
            speech.extend(say(0, t));
        }
        let print = voice.enroll(&speech).expect("enroll");
        let probe = "Congestion control in the transport layer slows the sender down when the network starts to drop packets.";
        let own = voice.similarity(&print, &say(0, probe)).unwrap();
        let others: Vec<f32> = (1..8).map(|sid| voice.similarity(&print, &say(sid, probe)).unwrap()).collect();
        println!("same voice: {own:.2}\nother voices: {:?}", others.iter().map(|s| format!("{s:.2}")).collect::<Vec<_>>());
        assert!(own > SAME, "own voice scored {own}");
        assert!(others.iter().all(|s| *s < own), "someone else scored higher than the owner: {others:?}");
    }
}
