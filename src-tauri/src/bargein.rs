//! Voice barge-in without echo cancellation: while Sunday talks, the mic hears Sunday at some
//! fixed fraction of the playback level (the "echo ratio"). Learn that ratio, then fire when the
//! mic stays well above it, i.e. someone is talking over us.

const WARMUP_MS: f32 = 600.0; // learn the ratio before judging (also covers output latency)
const FACTOR: f32 = 2.2; // mic/playback this far above the learned ratio counts as "hot"
const HOLD_MS: f32 = 250.0; // ...for this long in a row
const FLOOR: f32 = 0.01; // playback quieter than this is a gap between words: no info
const MIN_MIC: f32 = 0.02; // and the mic must be audibly loud in absolute terms

#[derive(Default)]
pub struct BargeIn {
    ratio: f32,
    age_ms: f32,
    hot_ms: f32,
}

impl BargeIn {
    /// Call when Sunday stops talking so the next reply re-learns the echo ratio.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// `mic` and `play` are smoothed RMS levels over a chunk of `ms`; `voiced` is the VAD's
    /// verdict on the mic (a clap is loud but not voiced).
    pub fn over(&mut self, mic: f32, play: f32, ms: f32, voiced: bool) -> bool {
        self.age_ms += ms;
        if play < FLOOR {
            self.hot_ms = 0.0;
            return false;
        }
        let r = mic / play;
        if self.ratio == 0.0 {
            self.ratio = r;
        }
        if self.age_ms < WARMUP_MS {
            self.ratio += 0.2 * (r - self.ratio);
            return false;
        }
        if r > self.ratio * FACTOR && mic > MIN_MIC && voiced {
            self.hot_ms += ms;
        } else {
            self.hot_ms = 0.0;
            if !voiced || r <= self.ratio * FACTOR {
                self.ratio += 0.05 * (r - self.ratio); // only learn from echo, never from the user
            }
        }
        self.hot_ms >= HOLD_MS
    }

    pub fn ratio(&self) -> f32 {
        self.ratio
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fires_on_talk_over_not_on_echo() {
        let mut b = BargeIn::default();
        let mut fired_at = None;
        // 100 ms chunks; playback wobbles 0.1..0.3, echo is 25% of it; the user joins at 2 s.
        for i in 0..40 {
            let play = 0.2 + 0.1 * ((i as f32) * 1.3).sin();
            let voice = if i >= 20 { 0.15 } else { 0.0 };
            if b.over(0.25 * play + voice, play, 100.0, true) && fired_at.is_none() {
                fired_at = Some(i);
            }
        }
        let i = fired_at.expect("never fired");
        assert!((20..=24).contains(&i), "fired at chunk {i}"); // within ~0.5 s of the user starting
    }

    #[test]
    fn ignores_claps() {
        let mut b = BargeIn::default();
        for i in 0..40 {
            let play = 0.2;
            // Loud for 300 ms from 2 s on, but the VAD says it isn't voice.
            let clap = if (20..23).contains(&i) { 0.5 } else { 0.0 };
            assert!(!b.over(0.05 + clap, play, 100.0, false), "fired at {i}");
        }
    }
}
