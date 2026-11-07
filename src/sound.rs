//! Sound effects, all synthesised in code: no audio files, no system sounds.
//!
//! Compiled in with the `sound` cargo feature (on by default) and switched
//! on with `config::SOUND_ENABLED`. Each show gets a [`Speaker`]: a helper
//! thread opens the default output when the show starts, mixes in each cue
//! as the choreography reaches it, and closes the device again once the show
//! is over, so nothing is held open while idle.

use crate::config;

/// Every effect the shows can trigger.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(feature = "sound"), allow(dead_code))]
pub enum Sfx {
    /// ZAP charge-up: a rising capacitor whine with crackles.
    ChargeUp,
    /// ZAP strike: a double crack over a falling sweep and a thump.
    Strike,
    /// ZAP "BEHOLD!": a boom under a diminished organ stab.
    Behold,
    /// Lab hum with crackles, for the monologue's arcs.
    Hum { millis: u32 },
    /// One typewriter key.
    Type,
    /// ZAP retreat: the generator spinning down.
    PowerDown,
    /// The little spark puff when the notch closes.
    Puff,
    /// DEFEAT: a sad, drooping three-note slide.
    Sad,
    /// VICTORY: a bright rising arpeggio with sparkles.
    Fanfare,
    /// A short rising whoosh as the ears open.
    Open,
    /// A short falling whoosh as the notch closes.
    Close,
}

/// One show's soundtrack. Silent when sound is off or unavailable.
pub struct Speaker {
    #[cfg(feature = "sound")]
    track: Option<imp::Track>,
    /// Show speed (`--slow`), to stretch timed beds like the hum.
    speed: f32,
}

impl Speaker {
    pub fn silent() -> Self {
        Self {
            #[cfg(feature = "sound")]
            track: None,
            speed: 1.0,
        }
    }

    /// Opens the output in the background if sound is enabled.
    pub fn open(speed: f32) -> Self {
        if !config::SOUND_ENABLED {
            return Self::silent();
        }
        #[cfg(feature = "sound")]
        {
            Self {
                track: imp::Track::open(),
                speed,
            }
        }
        #[cfg(not(feature = "sound"))]
        {
            log::warn!("SOUND_ENABLED is on but this build was made without the `sound` feature");
            Self { speed }
        }
    }

    pub fn play(&self, sfx: Sfx) {
        let enabled = match sfx {
            Sfx::Type => config::SOUND_TYPEWRITER,
            Sfx::Hum { .. } => config::SOUND_HUM,
            _ => true,
        };
        if !enabled {
            return;
        }
        #[cfg(feature = "sound")]
        if let Some(t) = &self.track {
            t.play(sfx);
        }
        #[cfg(not(feature = "sound"))]
        let _ = sfx;
    }

    /// A hum lasting `seconds` of show time.
    pub fn hum(&self, seconds: f64) {
        let millis = (seconds / f64::from(self.speed) * 1000.0).round() as u32;
        self.play(Sfx::Hum { millis });
    }

    /// The show ended normally: let the last sounds ring out.
    pub fn finish(self) {
        #[cfg(feature = "sound")]
        if let Some(t) = self.track {
            t.finish();
        }
    }
}

#[cfg(feature = "sound")]
mod imp {
    use std::num::NonZero;
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::time::{Duration, Instant};

    use super::{Sfx, synth};
    use crate::config;

    enum Msg {
        Play(Sfx, Instant),
        /// Show cancelled: stop everything now.
        Cut,
    }

    /// Hits older than this when the device finally opens are skipped, so a
    /// slow device never fires two cues on top of each other.
    const STALE: Duration = Duration::from_millis(300);

    pub struct Track {
        tx: Sender<Msg>,
        graceful: bool,
    }

    impl Track {
        pub fn open() -> Option<Self> {
            let (tx, rx) = mpsc::channel();
            match std::thread::Builder::new()
                .name("notch-sound".into())
                .spawn(move || run(rx))
            {
                Ok(_) => Some(Self {
                    tx,
                    graceful: false,
                }),
                Err(e) => {
                    log::warn!("couldn't start the sound thread: {e}");
                    None
                }
            }
        }

        pub fn play(&self, sfx: Sfx) {
            let _ = self.tx.send(Msg::Play(sfx, Instant::now()));
        }

        pub fn finish(mut self) {
            self.graceful = true;
        }
    }

    impl Drop for Track {
        fn drop(&mut self) {
            if !self.graceful {
                let _ = self.tx.send(Msg::Cut);
            }
        }
    }

    fn run(rx: Receiver<Msg>) {
        let mut sink = match rodio::DeviceSinkBuilder::open_default_sink() {
            Ok(s) => s,
            Err(e) => {
                log::warn!("no audio output: {e}");
                return;
            }
        };
        sink.log_on_drop(false);
        let (Some(channels), Some(rate)) = (NonZero::new(1), NonZero::new(synth::RATE)) else {
            return;
        };
        let mut rng = rand::rng();
        let mut busy_until = Instant::now();
        // Ends when the show drops its Speaker.
        while let Ok(msg) = rx.recv() {
            match msg {
                Msg::Cut => return,
                Msg::Play(sfx, sent) => {
                    let late = sent.elapsed();
                    let mut samples = synth::render(sfx, &mut rng);
                    match sfx {
                        // Beds stay in sync with the picture: skip what we missed.
                        Sfx::ChargeUp | Sfx::Hum { .. } => {
                            let skip = (late.as_secs_f64() * f64::from(synth::RATE)) as usize;
                            samples.drain(..skip.min(samples.len()));
                        }
                        // Hits play late rather than lose their attack, unless
                        // they're so late they'd land on the next cue.
                        _ if late > STALE => {
                            log::debug!("skipping stale {sfx:?}");
                            continue;
                        }
                        _ => {}
                    }
                    samples.iter_mut().for_each(|s| *s *= config::SOUND_VOLUME);
                    let secs = samples.len() as f64 / f64::from(synth::RATE);
                    busy_until = busy_until.max(Instant::now() + Duration::from_secs_f64(secs));
                    sink.mixer()
                        .add(rodio::buffer::SamplesBuffer::new(channels, rate, samples));
                }
            }
        }
        // Let the tail ring out, then close the device.
        std::thread::sleep(
            busy_until.saturating_duration_since(Instant::now()) + Duration::from_millis(60),
        );
    }
}

/// The synthesiser. Every effect is a pure function of an RNG, returning
/// mono samples at [`synth::RATE`] that peak at or below 1.0.
#[cfg(feature = "sound")]
mod synth {
    use std::f32::consts::TAU;

    use rand::{Rng, RngExt};

    use super::Sfx;

    pub const RATE: u32 = 44_100;
    const DT: f32 = 1.0 / RATE as f32;

    pub fn render(sfx: Sfx, rng: &mut impl Rng) -> Vec<f32> {
        let mut out = match sfx {
            Sfx::ChargeUp => charge_up(rng),
            Sfx::Strike => strike(rng),
            Sfx::Behold => behold(rng),
            Sfx::Hum { millis } => hum(rng, millis as f32 / 1000.0),
            Sfx::Type => key(rng),
            Sfx::PowerDown => power_down(rng),
            Sfx::Puff => puff(rng),
            Sfx::Sad => sad(),
            Sfx::Fanfare => fanfare(rng),
            Sfx::Open => whoosh(rng, 160.0, 720.0, 0.22),
            Sfx::Close => whoosh(rng, 650.0, 140.0, 0.22),
        };
        declick(&mut out);
        normalize(&mut out);
        out
    }

    // ----- Building blocks ------------------------------------------------

    fn len(seconds: f32) -> usize {
        (seconds * RATE as f32) as usize
    }

    fn noise(rng: &mut impl Rng) -> f32 {
        rng.random_range(-1.0f32..=1.0)
    }

    /// Exponential glide from `a` to `b` as `p` goes 0 -> 1.
    fn glide(a: f32, b: f32, p: f32) -> f32 {
        a * (b / a).powf(p.clamp(0.0, 1.0))
    }

    /// A phase-accumulating oscillator, so frequency sweeps stay smooth.
    #[derive(Default)]
    struct Osc {
        phase: f32,
    }

    impl Osc {
        fn step(&mut self, freq: f32) -> f32 {
            self.phase = (self.phase + TAU * freq * DT) % TAU;
            self.phase
        }
        fn sine(&mut self, freq: f32) -> f32 {
            self.step(freq).sin()
        }
        /// Band-limited-ish sawtooth from `n` harmonics with a brightness
        /// tilt (0 = dull, 1 = full saw).
        fn saw(&mut self, freq: f32, n: u32, bright: f32) -> f32 {
            let ph = self.step(freq);
            (1..=n)
                .filter(|k| freq * *k as f32 <= 9_000.0)
                .map(|k| (k as f32 * ph).sin() / k as f32 * bright.powi(k as i32 - 1))
                .sum()
        }
    }

    /// One-pole high-pass, for crisp noise.
    #[derive(Default)]
    struct HighPass {
        prev_in: f32,
        prev_out: f32,
    }

    impl HighPass {
        fn step(&mut self, x: f32, cutoff: f32) -> f32 {
            let rc = 1.0 / (TAU * cutoff);
            let a = rc / (rc + DT);
            self.prev_out = a * (self.prev_out + x - self.prev_in);
            self.prev_in = x;
            self.prev_out
        }
    }

    /// Random short noise pops ("crackle"), `rate` per second on average.
    struct Crackle {
        left: usize,
        amp: f32,
        hp: HighPass,
    }

    impl Crackle {
        fn new() -> Self {
            Self {
                left: 0,
                amp: 0.0,
                hp: HighPass::default(),
            }
        }
        fn step(&mut self, rng: &mut impl Rng, rate: f32) -> f32 {
            if self.left == 0 && rng.random_range(0.0f32..1.0) < rate * DT {
                self.left = rng.random_range(len(0.0015)..len(0.006));
                self.amp = rng.random_range(0.3f32..1.0);
            }
            if self.left == 0 {
                return 0.0;
            }
            self.left -= 1;
            self.hp.step(noise(rng) * self.amp, 2_500.0)
        }
    }

    fn adsr(t: f32, attack: f32, decay_tau: f32) -> f32 {
        (t / attack).min(1.0) * (-(t - attack).max(0.0) / decay_tau).exp()
    }

    /// 4ms fades at both ends so nothing clicks.
    fn declick(s: &mut [f32]) {
        let n = len(0.004).min(s.len() / 2);
        for i in 0..n {
            let g = i as f32 / n as f32;
            s[i] *= g;
            let j = s.len() - 1 - i;
            s[j] *= g;
        }
    }

    /// Soft-limits and scales so the loudest sample is at most 0.95.
    fn normalize(s: &mut [f32]) {
        let peak = s.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        if peak > 0.95 {
            let g = 0.95 / peak;
            s.iter_mut().for_each(|v| *v *= g);
        }
    }

    // ----- Effects -----------------------------------------------------------

    fn charge_up(rng: &mut impl Rng) -> Vec<f32> {
        let n = len(0.38);
        let (mut osc, mut wobble, mut crackle) = (Osc::default(), Osc::default(), Crackle::new());
        (0..n)
            .map(|i| {
                let p = i as f32 / n as f32;
                let f = glide(240.0, 3_200.0, p);
                let ph = osc.step(f);
                let whine = ph.sin() + 0.3 * (2.0 * ph).sin();
                // The wobble speeds up as the charge builds.
                let trem = 1.0 - 0.4 * (0.5 + 0.5 * wobble.sine(14.0 + 50.0 * p));
                let body = whine * trem * p.powf(1.4) * 0.5;
                body + crackle.step(rng, 30.0 + 260.0 * p) * 0.5 * p
            })
            .collect()
    }

    fn strike(rng: &mut impl Rng) -> Vec<f32> {
        let n = len(0.42);
        let (mut sweep, mut thump, mut hp) = (Osc::default(), Osc::default(), HighPass::default());
        (0..n)
            .map(|i| {
                let t = i as f32 * DT;
                let p = (t / 0.32).min(1.0);
                // Two cracks: the snap, and a smaller echo 45ms later.
                let crack_env = (-t / 0.016).exp()
                    + 0.5 * (-(t - 0.045).max(0.0) / 0.012).exp() * f32::from(t >= 0.045);
                let crack = hp.step(noise(rng), 1_800.0) * crack_env;
                let zap = sweep.sine(glide(2_300.0, 90.0, p)) * (1.0 - p).powf(1.5);
                let boom = thump.sine(glide(150.0, 55.0, t / 0.2)) * (-t / 0.11).exp();
                (1.1 * crack + 0.65 * zap + 0.8 * boom).tanh()
            })
            .collect()
    }

    fn behold(rng: &mut impl Rng) -> Vec<f32> {
        let n = len(1.15);
        // D2 bass, then a diminished D-F-Ab: properly villainous.
        let notes = [73.42f32, 146.83, 174.61, 207.65, 293.66];
        let mut voices: Vec<(Osc, Osc, f32)> = notes
            .iter()
            .map(|f| (Osc::default(), Osc::default(), *f))
            .collect();
        let (mut boom, mut hp) = (Osc::default(), HighPass::default());
        (0..n)
            .map(|i| {
                let t = i as f32 * DT;
                let env = adsr(t, 0.012, 0.42);
                // Brightness closes as the stab decays, like a filter sweep.
                let bright = 0.55 + 0.35 * (-t / 0.25).exp();
                let organ: f32 = voices
                    .iter_mut()
                    .map(|(a, b, f)| a.saw(*f * 0.997, 10, bright) + b.saw(*f * 1.004, 10, bright))
                    .sum::<f32>()
                    * 0.16;
                let thud = boom.sine(glide(120.0, 42.0, t / 0.5)) * (-t / 0.3).exp();
                let air = hp.step(noise(rng), 3_000.0) * (-t / 0.05).exp() * 0.3;
                (1.4 * (organ * env + 0.9 * thud + air)).tanh()
            })
            .collect()
    }

    fn hum(rng: &mut impl Rng, seconds: f32) -> Vec<f32> {
        let n = len(seconds.clamp(0.1, 20.0));
        let (mut mains, mut flutter, mut crackle) =
            (Osc::default(), Osc::default(), Crackle::new());
        let fade_in = len(0.15) as f32;
        let fade_out = len(0.3) as f32;
        (0..n)
            .map(|i| {
                let ph = mains.step(100.0);
                let buzz = ph.sin()
                    + 0.55 * (2.0 * ph).sin()
                    + 0.4 * (3.0 * ph).sin()
                    + 0.2 * (5.0 * ph).sin();
                let wob = 0.8 + 0.2 * flutter.sine(5.5);
                let edge = (i as f32 / fade_in).min(1.0).min((n - i) as f32 / fade_out);
                (0.16 * buzz * wob + 0.55 * crackle.step(rng, 13.0)) * edge
            })
            .collect()
    }

    fn key(rng: &mut impl Rng) -> Vec<f32> {
        let n = len(0.035);
        let pitch = rng.random_range(1_700.0f32..2_600.0);
        let (mut ping, mut hp) = (Osc::default(), HighPass::default());
        (0..n)
            .map(|i| {
                let t = i as f32 * DT;
                let click = hp.step(noise(rng), 2_000.0) * (-t / 0.0025).exp();
                let tone = ping.sine(pitch) * (-t / 0.007).exp();
                0.35 * (click + 0.6 * tone)
            })
            .collect()
    }

    fn power_down(rng: &mut impl Rng) -> Vec<f32> {
        let n = len(0.62);
        let (mut osc, mut wobble) = (Osc::default(), Osc::default());
        (0..n)
            .map(|i| {
                let p = i as f32 / n as f32;
                let ph = osc.step(glide(760.0, 50.0, p));
                let tone = ph.sin() + 0.35 * (2.0 * ph).sin();
                // The tremolo slows down along with the "generator".
                let trem = 0.65 + 0.35 * wobble.sine(glide(32.0, 3.0, p));
                tone * trem * (1.0 - p).powf(1.2) * 0.6 + noise(rng) * 0.02 * (1.0 - p)
            })
            .collect()
    }

    fn puff(rng: &mut impl Rng) -> Vec<f32> {
        let n = len(0.2);
        let mut crackle = Crackle::new();
        let mut zip = Osc::default();
        (0..n)
            .map(|i| {
                let p = i as f32 / n as f32;
                crackle.step(rng, 70.0) * (1.0 - p)
                    + zip.sine(glide(4_200.0, 1_500.0, p)) * 0.12 * (1.0 - p).powi(3)
            })
            .collect()
    }

    fn sad() -> Vec<f32> {
        // G3 -> F#3 -> F3, the last one long, wobbling and sagging (like the notch).
        let notes: [(f32, f32); 3] = [(196.0, 0.30), (185.0, 0.30), (174.6, 0.85)];
        let mut out = Vec::new();
        let mut osc = Osc::default();
        let mut vib = Osc::default();
        for (idx, (f, dur)) in notes.iter().enumerate() {
            let last = idx == notes.len() - 1;
            let n = len(*dur);
            for i in 0..n {
                let t = i as f32 * DT;
                let p = i as f32 / n as f32;
                // Each note "wah"s: brightness opens then closes.
                let wah = (p * std::f32::consts::PI).sin();
                let bend = if last {
                    1.0 - 0.06 * p * p
                } else {
                    1.0 - 0.015 * p
                };
                let vibrato = if last {
                    1.0 + 0.018 * vib.sine(5.5) * p.min(0.6) / 0.6
                } else {
                    1.0
                };
                let tone = osc.saw(f * bend * vibrato, 8, 0.25 + 0.5 * wah);
                // Each note fades out over its last 40ms, so the joins don't click.
                let release = ((n - i) as f32 / len(0.04) as f32).min(1.0);
                let env = (t / 0.03).min(1.0)
                    * release
                    * if last {
                        (1.0 - p).powf(0.8)
                    } else {
                        1.0 - 0.3 * p
                    };
                out.push(0.45 * tone * env);
            }
        }
        out
    }

    fn fanfare(rng: &mut impl Rng) -> Vec<f32> {
        // C5 E5 G5 C6, 80ms apart, ringing like little bells.
        let notes = [523.25f32, 659.25, 783.99, 1_046.5];
        let step = 0.08;
        let n = len(1.1);
        let mut voices: Vec<Osc> = notes.iter().map(|_| Osc::default()).collect();
        let mut sweep = Osc::default();
        let mut sparkle = Osc::default();
        let mut sparkle_f = 3_000.0f32;
        let mut sparkle_t = 1.0f32;
        (0..n)
            .map(|i| {
                let t = i as f32 * DT;
                let mut s = 0.0;
                for (k, (osc, f)) in voices.iter_mut().zip(notes).enumerate() {
                    let start = k as f32 * step;
                    let ph = osc.step(f);
                    if t >= start {
                        let lt = t - start;
                        let ring = if k == notes.len() - 1 { 0.5 } else { 0.22 };
                        let bell = ph.sin() + 0.35 * (2.0 * ph).sin() + 0.12 * (3.01 * ph).sin();
                        s += bell * adsr(lt, 0.004, ring) * 0.33;
                    }
                }
                // A soft rising power-up underneath.
                s += sweep.sine(glide(200.0, 900.0, t / 0.35)) * 0.12 * (-t / 0.25).exp();
                // Sparkles: random high pings after the last note.
                if t > 0.25 && rng.random_range(0.0f32..1.0) < 22.0 * DT {
                    sparkle_f = rng.random_range(3_000.0..6_000.0);
                    sparkle_t = 0.0;
                }
                sparkle_t += DT;
                s += sparkle.sine(sparkle_f) * (-sparkle_t / 0.02).exp() * 0.12 * (1.0 - t / 1.1);
                s
            })
            .collect()
    }

    fn whoosh(rng: &mut impl Rng, from: f32, to: f32, seconds: f32) -> Vec<f32> {
        let n = len(seconds);
        let (mut osc, mut hp) = (Osc::default(), HighPass::default());
        (0..n)
            .map(|i| {
                let p = i as f32 / n as f32;
                let env = (p * std::f32::consts::PI).sin().powf(1.5);
                let air = hp.step(noise(rng), glide(from * 2.0, to * 2.0, p)) * 0.4;
                (osc.sine(glide(from, to, p)) * 0.35 + air) * env
            })
            .collect()
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use rand::SeedableRng;
        use rand::rngs::StdRng;

        const ALL: [Sfx; 11] = [
            Sfx::ChargeUp,
            Sfx::Strike,
            Sfx::Behold,
            Sfx::Hum { millis: 2_000 },
            Sfx::Type,
            Sfx::PowerDown,
            Sfx::Puff,
            Sfx::Sad,
            Sfx::Fanfare,
            Sfx::Open,
            Sfx::Close,
        ];

        #[test]
        fn every_effect_is_bounded_audible_and_click_free() {
            let mut rng = StdRng::seed_from_u64(11);
            for sfx in ALL {
                let s = render(sfx, &mut rng);
                assert!(!s.is_empty(), "{sfx:?} is empty");
                assert!(
                    s.iter().all(|v| v.is_finite() && v.abs() <= 0.95 + 1e-6),
                    "{sfx:?} clips"
                );
                let peak = s.iter().fold(0.0f32, |m, v| m.max(v.abs()));
                assert!(peak > 0.05, "{sfx:?} is inaudible ({peak})");
                assert!(
                    s[0].abs() < 1e-6 && s[s.len() - 1].abs() < 1e-6,
                    "{sfx:?} clicks"
                );
            }
        }

        #[test]
        fn strike_is_about_a_quarter_second_of_zap() {
            let mut rng = StdRng::seed_from_u64(1);
            let s = render(Sfx::Strike, &mut rng);
            let secs = s.len() as f32 / RATE as f32;
            assert!((0.25..=0.5).contains(&secs));
        }

        /// `cargo test --release -- --ignored export_wavs` writes every effect,
        /// plus a mixdown of the whole ZAP soundtrack, to `target/sfx/`.
        #[test]
        #[ignore = "writes WAV files for listening"]
        fn export_wavs() {
            use crate::config::zap;
            let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/sfx");
            std::fs::create_dir_all(&dir).unwrap();
            let mut rng = StdRng::seed_from_u64(4);
            for sfx in ALL {
                let name = format!("{sfx:?}")
                    .split_whitespace()
                    .next()
                    .unwrap()
                    .to_lowercase();
                write_wav(&dir.join(format!("{name}.wav")), &render(sfx, &mut rng));
            }
            // The ZAP show, cue for cue.
            let mut cues = vec![
                (0.0, Sfx::ChargeUp),
                (zap::STRIKE, Sfx::Strike),
                (zap::BEHOLD, Sfx::Behold),
                (
                    zap::ARCS_START,
                    Sfx::Hum {
                        millis: ((zap::RETREAT + zap::CONTENT_FADE - zap::ARCS_START) * 1000.0)
                            as u32,
                    },
                ),
                (zap::RETREAT + zap::CONTENT_FADE, Sfx::PowerDown),
                (zap::PUFF, Sfx::Puff),
            ];
            let typing = zap::MONOLOGUE + 0.05;
            for i in 1..=29 {
                cues.push((
                    typing + f64::from(i) / crate::config::TYPE_CHARS_PER_SEC,
                    Sfx::Type,
                ));
            }
            let mut mix = vec![0.0f32; len(zap::END as f32 + 0.3)];
            for (at, sfx) in cues {
                let start = len(at as f32);
                for (i, v) in render(sfx, &mut rng).into_iter().enumerate() {
                    if let Some(m) = mix.get_mut(start + i) {
                        *m += v * 0.6;
                    }
                }
            }
            mix.iter_mut().for_each(|v| *v = v.tanh());
            write_wav(&dir.join("zap_show.wav"), &mix);
            println!("wrote WAVs to {}", dir.display());
        }

        fn write_wav(path: &std::path::Path, samples: &[f32]) {
            let data_len = (samples.len() * 2) as u32;
            let mut b = Vec::with_capacity(44 + data_len as usize);
            b.extend_from_slice(b"RIFF");
            b.extend_from_slice(&(36 + data_len).to_le_bytes());
            b.extend_from_slice(b"WAVEfmt ");
            b.extend_from_slice(&16u32.to_le_bytes());
            b.extend_from_slice(&1u16.to_le_bytes()); // PCM
            b.extend_from_slice(&1u16.to_le_bytes()); // mono
            b.extend_from_slice(&RATE.to_le_bytes());
            b.extend_from_slice(&(RATE * 2).to_le_bytes());
            b.extend_from_slice(&2u16.to_le_bytes());
            b.extend_from_slice(&16u16.to_le_bytes());
            b.extend_from_slice(b"data");
            b.extend_from_slice(&data_len.to_le_bytes());
            for s in samples {
                b.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32_767.0) as i16).to_le_bytes());
            }
            std::fs::write(path, b).unwrap();
        }

        #[test]
        fn hum_length_follows_the_request() {
            let mut rng = StdRng::seed_from_u64(2);
            let s = render(Sfx::Hum { millis: 1_500 }, &mut rng);
            assert_eq!(s.len(), len(1.5));
        }
    }
}
