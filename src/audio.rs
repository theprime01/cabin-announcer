//! Playback on a dedicated thread. Announcements queue up and play one after
//! another; boarding music and cabin ambience loop underneath, ducking while
//! someone is speaking. Everything passes through a filter that muffles the
//! sound when you're on the flight deck.

use std::collections::VecDeque;
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rodio::buffer::SamplesBuffer;
use rodio::{OutputStream, OutputStreamHandle, Sink, Source};

use crate::config::Sound;

#[derive(Clone)]
pub struct Item {
    pub title: String,
    /// Played in order.
    pub recordings: Vec<PathBuf>,
    /// Add the app's PA chime first.
    pub chime: bool,
}

/// Which camera you're looking through.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum View {
    #[default]
    Cabin,
    Cockpit,
    Outside,
}

enum Msg {
    Play(Item),
    Replay,
    Stop,
    Pause(bool),
    /// Tracks to loop; empty stops (fading out).
    Music(Vec<PathBuf>),
    Ambience(Vec<PathBuf>),
    Settings(Sound),
    View(View),
}

#[derive(Default, Clone)]
pub struct Status {
    pub now_playing: Option<String>,
    pub queued: Vec<String>,
    pub music: Option<String>,
    pub paused: bool,
    pub error: Option<String>,
}

#[derive(Clone)]
pub struct Audio {
    tx: Sender<Msg>,
    pub status: Arc<Mutex<Status>>,
}

impl Audio {
    pub fn start(sound: Sound) -> Audio {
        let (tx, rx) = channel();
        let status = Arc::new(Mutex::new(Status::default()));
        let s = status.clone();
        std::thread::Builder::new()
            .name("audio".into())
            .spawn(move || run(rx, s, sound))
            .expect("spawn audio thread");
        Audio { tx, status }
    }

    fn send(&self, m: Msg) {
        let _ = self.tx.send(m);
    }

    pub fn play(&self, item: Item) {
        self.send(Msg::Play(item));
    }

    /// Plays the last announcement again.
    pub fn replay(&self) {
        self.send(Msg::Replay);
    }

    /// Stops the current announcement and clears the queue.
    pub fn stop(&self) {
        self.send(Msg::Stop);
    }

    pub fn pause(&self, paused: bool) {
        self.send(Msg::Pause(paused));
    }

    pub fn toggle_pause(&self) {
        let paused = self.status.lock().unwrap().paused;
        self.pause(!paused);
    }

    pub fn music(&self, tracks: Vec<PathBuf>) {
        self.send(Msg::Music(tracks));
    }

    pub fn ambience(&self, tracks: Vec<PathBuf>) {
        self.send(Msg::Ambience(tracks));
    }

    pub fn settings(&self, sound: Sound) {
        self.send(Msg::Settings(sound));
    }

    pub fn view(&self, view: View) {
        self.send(Msg::View(view));
    }
}

/// The live cockpit filter settings, shared with every playing sound.
#[derive(Default)]
struct Muffle {
    /// Low-pass cutoff in Hz (f32 bits); 0 means open.
    cutoff: AtomicU32,
    /// Gain (f32 bits).
    gain: AtomicU32,
}

impl Muffle {
    fn set(&self, cutoff: f32, gain: f32) {
        self.cutoff.store(cutoff.to_bits(), Ordering::Relaxed);
        self.gain.store(gain.to_bits(), Ordering::Relaxed);
    }
    fn get(&self) -> (f32, f32) {
        (f32::from_bits(self.cutoff.load(Ordering::Relaxed)), f32::from_bits(self.gain.load(Ordering::Relaxed)))
    }
}

const MUFFLED_HZ: f32 = 900.0;

/// Applies the shared muffle: a two-pole low-pass and a gain, both eased so
/// switching cameras doesn't click.
struct Muffled<S> {
    inner: S,
    muffle: Arc<Muffle>,
    rate: u32,
    channels: u16,
    channel: usize,
    state: Vec<[f32; 2]>,
    coef: f32,
    gain: f32,
    counter: u32,
}

impl<S: Source<Item = f32>> Muffled<S> {
    fn new(inner: S, muffle: Arc<Muffle>) -> Self {
        let (rate, channels) = (inner.sample_rate(), inner.channels());
        let (cutoff, gain) = muffle.get();
        let coef = coefficient(cutoff, rate);
        Muffled { inner, muffle, rate, channels, channel: 0, state: vec![[0.0; 2]; channels.max(1) as usize], coef, gain, counter: 0 }
    }
}

/// One-pole low-pass coefficient; 1.0 passes everything.
fn coefficient(cutoff: f32, rate: u32) -> f32 {
    if cutoff <= 0.0 { 1.0 } else { 1.0 - (-2.0 * std::f32::consts::PI * cutoff / rate as f32).exp() }
}

impl<S: Source<Item = f32>> Iterator for Muffled<S> {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        let x = self.inner.next()?;
        // Re-read the settings every few milliseconds, easing towards them.
        if self.counter == 0 {
            let (cutoff, gain) = self.muffle.get();
            self.coef += (coefficient(cutoff, self.rate) - self.coef) * 0.2;
            self.gain += (gain - self.gain) * 0.2;
        }
        self.counter = (self.counter + 1) % 256;
        let st = &mut self.state[self.channel];
        st[0] += self.coef * (x - st[0]);
        st[1] += self.coef * (st[0] - st[1]);
        let y = st[1] * self.gain;
        self.channel = (self.channel + 1) % self.channels.max(1) as usize;
        Some(y)
    }
}

impl<S: Source<Item = f32>> Source for Muffled<S> {
    fn current_frame_len(&self) -> Option<usize> {
        self.inner.current_frame_len()
    }
    fn channels(&self) -> u16 {
        self.channels
    }
    fn sample_rate(&self) -> u32 {
        self.rate
    }
    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }
}

/// RBJ biquad filter.
struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
    x: [f32; 2],
    y: [f32; 2],
}

impl Biquad {
    fn new(kind: char, freq: f32, rate: u32) -> Biquad {
        let w = 2.0 * std::f32::consts::PI * freq / rate as f32;
        let (sin, cos) = w.sin_cos();
        let alpha = sin / (2.0 * std::f32::consts::FRAC_1_SQRT_2);
        let (b0, b1, b2) = if kind == 'h' {
            ((1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0)
        } else {
            ((1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0)
        };
        let a0 = 1.0 + alpha;
        Biquad { b: [b0 / a0, b1 / a0, b2 / a0], a: [-2.0 * cos / a0, (1.0 - alpha) / a0], x: [0.0; 2], y: [0.0; 2] }
    }

    fn run(&mut self, x: f32) -> f32 {
        let y = self.b[0] * x + self.b[1] * self.x[0] + self.b[2] * self.x[1] - self.a[0] * self.y[0] - self.a[1] * self.y[1];
        self.x = [x, self.x[0]];
        self.y = [y, self.y[0]];
        y
    }
}

/// Makes a clean studio recording sound like it comes through small cabin
/// speakers: band-limited, slightly driven, with a short cabin reflection.
pub fn pa_effect(samples: &[f32], rate: u32) -> Vec<f32> {
    let mut filters = [Biquad::new('h', 320.0, rate), Biquad::new('h', 320.0, rate), Biquad::new('l', 3400.0, rate), Biquad::new('l', 3400.0, rate)];
    let drive = 2.2f32;
    let norm = drive.tanh();
    let mut out: Vec<f32> = samples
        .iter()
        .map(|&s| {
            let f = filters.iter_mut().fold(s, |acc, f| f.run(acc));
            (f * drive).tanh() / norm
        })
        .collect();
    let dry = out.clone();
    for (secs, gain) in [(0.011, 0.22f32), (0.019, 0.14), (0.031, 0.08)] {
        let d = (secs * rate as f64) as usize;
        for i in d..out.len() {
            out[i] += dry[i - d] * gain;
        }
    }
    normalize(&mut out, 0.9);
    out
}

fn normalize(samples: &mut [f32], peak: f32) {
    let max = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if max > 1e-6 {
        let k = peak / max;
        samples.iter_mut().for_each(|s| *s *= k);
    }
}

/// A two-tone "hi-lo" cabin chime, for recordings that don't have their own.
pub fn chime(rate: u32) -> Vec<f32> {
    let len = (rate as f32 * 2.2) as usize;
    let mut out = vec![0.0f32; len];
    for (start, freq) in [(0.0f32, 1046.5f32), (0.42, 784.0)] {
        let first = (start * rate as f32) as usize;
        for (i, s) in out.iter_mut().enumerate().skip(first) {
            let t = (i - first) as f32 / rate as f32;
            let env = (-t * 2.6).exp() * (t * 400.0).min(1.0);
            let ph = 2.0 * std::f32::consts::PI * freq * t;
            *s += env * (ph.sin() + 0.28 * (2.0 * ph).sin() + 0.08 * (3.01 * ph).sin());
        }
    }
    normalize(&mut out, 0.6);
    out
}

fn to_mono(samples: &[f32], channels: u16) -> Vec<f32> {
    let n = channels.max(1) as usize;
    samples.chunks(n).map(|c| c.iter().sum::<f32>() / n as f32).collect()
}

type Decoded = rodio::source::SamplesConverter<rodio::Decoder<BufReader<std::fs::File>>, f32>;

fn open(path: &PathBuf) -> Result<Decoded, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let dec = rodio::Decoder::new(BufReader::new(file)).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(dec.convert_samples::<f32>())
}

fn file_title(path: &PathBuf) -> String {
    path.file_stem().map_or(String::new(), |s| s.to_string_lossy().to_string())
}

/// One looping background layer (music or ambience).
struct Layer {
    sink: Sink,
    tracks: Vec<PathBuf>,
    /// Current volume, eased towards the target.
    level: f32,
    now: Option<String>,
}

impl Layer {
    fn new(handle: &OutputStreamHandle) -> Option<Layer> {
        let sink = Sink::try_new(handle).ok()?;
        sink.set_volume(0.0);
        Some(Layer { sink, tracks: Vec::new(), level: 0.0, now: None })
    }

    /// Keeps a track queued while wanted, and fades towards `target`.
    fn tick(&mut self, target: f32, muffle: &Arc<Muffle>, errors: &mut Option<String>) {
        let wanted = !self.tracks.is_empty();
        if wanted && self.sink.empty() {
            let path = crate::packs::pick(&self.tracks);
            match open(&path) {
                Ok(src) => {
                    self.sink.append(Muffled::new(src, muffle.clone()));
                    self.now = Some(file_title(&path));
                }
                Err(e) => *errors = Some(e),
            }
        }
        let target = if wanted { target } else { 0.0 };
        // Fades in over about two seconds; fades out within a third of a second,
        // so an announcement never starts over the music.
        let step = if target < self.level { 0.06f32 } else { 0.025f32.max((target - self.level).abs() * 0.08) };
        self.level = if self.level < target { (self.level + step).min(target) } else { (self.level - step).max(target) };
        self.sink.set_volume(self.level);
        if !wanted && self.level <= 0.0 && !self.sink.empty() {
            self.sink.clear();
            self.sink.play();
            self.now = None;
        } else if wanted && self.level <= 0.0 && target <= 0.0 {
            // Held for an announcement: pause so the track carries on where it left off.
            self.sink.pause();
        } else if target > 0.0 && self.sink.is_paused() {
            self.sink.play();
        }
    }

    fn silent(&self) -> bool {
        self.level <= 0.0 || self.tracks.is_empty()
    }
}

/// Builds everything the sink plays for one announcement.
fn build(item: &Item, sound: &Sound, muffle: &Arc<Muffle>, sink: &Sink) -> Result<(), String> {
    const CHIME_RATE: u32 = 44_100;
    if item.chime {
        let mut c = chime(CHIME_RATE);
        c.extend(vec![0.0; (CHIME_RATE as f32 * 0.4) as usize]);
        sink.append(Muffled::new(SamplesBuffer::new(1, CHIME_RATE, c), muffle.clone()));
    }
    for (i, path) in item.recordings.iter().enumerate() {
        if i > 0 {
            sink.append(Muffled::new(SamplesBuffer::new(1, CHIME_RATE, vec![0.0; (CHIME_RATE as f32 * 0.6) as usize]), muffle.clone()));
        }
        let src = open(path)?;
        if sound.pa_effect {
            let (channels, rate) = (src.channels(), src.sample_rate());
            let samples: Vec<f32> = src.collect();
            let wet = pa_effect(&to_mono(&samples, channels), rate);
            sink.append(Muffled::new(SamplesBuffer::new(1, rate, wet), muffle.clone()));
        } else {
            sink.append(Muffled::new(src, muffle.clone()));
        }
    }
    Ok(())
}

fn apply_view(muffle: &Muffle, view: View, sound: &Sound) {
    match view {
        View::Cabin => muffle.set(0.0, 1.0),
        View::Cockpit => muffle.set(if sound.cockpit_muffle { MUFFLED_HZ } else { 0.0 }, sound.cockpit_volume),
        View::Outside => muffle.set(MUFFLED_HZ, sound.outside_volume),
    }
}

fn run(rx: Receiver<Msg>, status: Arc<Mutex<Status>>, mut sound: Sound) {
    let set = |f: &mut dyn FnMut(&mut Status)| f(&mut status.lock().unwrap());
    let (_stream, handle) = match OutputStream::try_default() {
        Ok(o) => o,
        Err(e) => {
            let msg = format!("No audio output: {e}");
            set(&mut |st| st.error = Some(msg.clone()));
            return;
        }
    };
    let muffle = Arc::new(Muffle::default());
    let mut view = View::Cabin;
    apply_view(&muffle, view, &sound);
    let (Ok(speech), Some(mut music), Some(mut ambience)) = (Sink::try_new(&handle), Layer::new(&handle), Layer::new(&handle)) else {
        set(&mut |st| st.error = Some("Couldn't open the audio device".into()));
        return;
    };

    let mut queue: VecDeque<Item> = VecDeque::new();
    let mut last: Option<Item> = None;
    let mut playing: Option<String> = None;
    let mut gap_until = Instant::now();
    let mut paused = false;
    loop {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(Msg::Play(i)) => queue.push_back(i),
            Ok(Msg::Replay) => {
                if let Some(i) = last.clone() {
                    queue.push_front(i);
                }
            }
            Ok(Msg::Stop) => {
                queue.clear();
                speech.clear();
                speech.play();
                if paused {
                    speech.pause();
                }
            }
            Ok(Msg::Pause(p)) => {
                paused = p;
                for s in [&speech, &music.sink, &ambience.sink] {
                    if p { s.pause() } else { s.play() }
                }
                set(&mut |st| st.paused = p);
            }
            Ok(Msg::Music(t)) => music.tracks = t,
            Ok(Msg::Ambience(t)) => ambience.tracks = t,
            Ok(Msg::Settings(s)) => {
                sound = s;
                apply_view(&muffle, view, &sound);
            }
            Ok(Msg::View(v)) => {
                view = v;
                apply_view(&muffle, view, &sound);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }

        let mut error = None;
        if speech.empty() && playing.take().is_some() {
            // A short gap between back-to-back announcements.
            gap_until = Instant::now() + Duration::from_millis(800);
        }
        // Music stops for announcements and comes back a moment after the last one.
        let speaking = !speech.empty() || !queue.is_empty() || Instant::now() < gap_until + Duration::from_millis(1500);
        if speech.empty() && !paused && Instant::now() >= gap_until && music.silent() {
            if let Some(item) = queue.pop_front() {
                speech.set_volume(sound.volume.clamp(0.0, 1.0));
                match build(&item, &sound, &muffle, &speech) {
                    Ok(()) => {
                        playing = Some(item.title.clone());
                        last = Some(item);
                    }
                    Err(e) => {
                        speech.clear();
                        speech.play();
                        error = Some(e);
                    }
                }
            }
        }
        let music_target = if sound.music && !speaking { sound.music_volume } else { 0.0 };
        // Ambience is the cabin itself, so it only dips.
        let ambience_target = if sound.ambience { sound.ambience_volume * if speaking { 0.6 } else { 1.0 } } else { 0.0 };
        if !paused {
            music.tick(music_target, &muffle, &mut error);
            ambience.tick(ambience_target, &muffle, &mut error);
        }

        let titles: Vec<String> = queue.iter().map(|i| i.title.clone()).collect();
        let music_now = music.now.clone().filter(|_| music.level > 0.0);
        set(&mut |st| {
            st.now_playing = playing.clone();
            st.queued = titles.clone();
            st.music = music_now.clone();
            if error.is_some() {
                st.error = error.clone();
            } else if playing.is_some() {
                st.error = None;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pa_effect_keeps_length_and_level() {
        let rate = 22_050;
        let tone: Vec<f32> = (0..rate).map(|i| (i as f32 * 0.1).sin() * 0.3).collect();
        let out = pa_effect(&tone, rate);
        assert_eq!(out.len(), tone.len());
        let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!((peak - 0.9).abs() < 1e-3);
    }

    #[test]
    fn chime_is_two_seconds_and_quiet_at_the_end() {
        let c = chime(44_100);
        assert!(c.len() > 88_000);
        assert!(c[c.len() - 1].abs() < 0.05);
    }

    /// A silent 16-bit mono WAV file, long enough to keep playing during the test.
    fn silent_wav(path: &std::path::Path, secs: u32) {
        let rate = 8000u32;
        let data = rate * 2 * secs;
        let mut b = Vec::new();
        b.extend(b"RIFF");
        b.extend((36 + data).to_le_bytes());
        b.extend(b"WAVEfmt ");
        b.extend(16u32.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(rate.to_le_bytes());
        b.extend((rate * 2).to_le_bytes());
        b.extend(2u16.to_le_bytes());
        b.extend(16u16.to_le_bytes());
        b.extend(b"data");
        b.extend(data.to_le_bytes());
        b.resize(b.len() + data as usize, 0);
        std::fs::write(path, b).unwrap();
    }

    #[test]
    fn music_fades_out_fast_and_pauses_for_announcements() {
        let path = std::env::temp_dir().join(format!("ca-music-{}.wav", std::process::id()));
        silent_wav(&path, 30);
        let (sink, mut out) = Sink::new_idle();
        // Stands in for the sound card: reads samples at about real time.
        // (Sink::clear waits for the output to catch up, so something must read.)
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let reader = {
            let stop = stop.clone();
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    for _ in 0..80 {
                        out.next();
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
            })
        };
        let mut music = Layer { sink, tracks: vec![path.clone()], level: 0.0, now: None };
        let muffle = Arc::new(Muffle::default());
        let mut err = None;
        for _ in 0..60 {
            music.tick(0.35, &muffle, &mut err);
        }
        assert!(err.is_none(), "{err:?}");
        assert!((music.level - 0.35).abs() < 1e-6);
        // An announcement comes in: silent within a third of a second (7 ticks).
        let mut ticks = 0;
        while !music.silent() {
            music.tick(0.0, &muffle, &mut err);
            ticks += 1;
        }
        assert!(ticks <= 7, "took {ticks} ticks");
        music.tick(0.0, &muffle, &mut err);
        assert!(music.sink.is_paused(), "held, not stopped");
        assert_eq!(music.sink.len(), 1, "same track kept");
        music.tick(0.35, &muffle, &mut err);
        assert!(!music.sink.is_paused());
        // GSX boarding ends: the track is dropped once faded out.
        music.tracks.clear();
        for _ in 0..10 {
            music.tick(0.35, &muffle, &mut err);
        }
        assert!(music.sink.empty() && music.now.is_none());
        stop.store(true, Ordering::Relaxed);
        reader.join().unwrap();
        let _ = std::fs::remove_file(&path);
    }

    fn rms(v: &[f32]) -> f32 {
        (v.iter().map(|s| s * s).sum::<f32>() / v.len() as f32).sqrt()
    }

    #[test]
    fn muffle_cuts_highs_and_keeps_lows() {
        let rate = 44_100;
        let tone = |hz: f32| -> Vec<f32> { (0..rate).map(|i| (2.0 * std::f32::consts::PI * hz * i as f32 / rate as f32).sin()).collect() };
        let through = |hz: f32, cutoff: f32| -> f32 {
            let m = Arc::new(Muffle::default());
            m.set(cutoff, 1.0);
            let out: Vec<f32> = Muffled::new(SamplesBuffer::new(1, rate as u32, tone(hz)), m).collect();
            rms(&out[rate as usize / 2..])
        };
        let open = through(4000.0, 0.0);
        assert!((open - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.02, "open filter passes everything: {open}");
        assert!(through(4000.0, MUFFLED_HZ) < 0.1, "highs cut");
        assert!(through(200.0, MUFFLED_HZ) > 0.6, "lows kept");
    }
}
