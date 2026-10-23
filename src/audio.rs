//! Speech and playback. Announcements queue up and play one after another on
//! a dedicated thread, so a long safety demo never cuts off a later call.

use std::collections::VecDeque;
use std::io::{BufReader, Cursor};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rodio::buffer::SamplesBuffer;
use rodio::{OutputStream, Sink, Source};

use crate::announcements::Role;
use crate::config::Voice;

pub struct Item {
    pub title: String,
    pub role: Role,
    pub text: String,
    pub chime: bool,
    /// Recordings played in order instead of speech; empty means speak.
    pub recordings: Vec<PathBuf>,
    pub voice: Voice,
}

enum Msg {
    Play(Item),
    Stop,
    Pause(bool),
}

#[derive(Default, Clone)]
pub struct Status {
    pub now_playing: Option<String>,
    pub queued: Vec<String>,
    pub voices: Vec<String>,
    pub error: Option<String>,
}

#[derive(Clone)]
pub struct Audio {
    tx: Sender<Msg>,
    pub status: Arc<Mutex<Status>>,
}

impl Audio {
    pub fn start() -> Audio {
        let (tx, rx) = channel();
        let status = Arc::new(Mutex::new(Status::default()));
        let s = status.clone();
        std::thread::Builder::new()
            .name("audio".into())
            .spawn(move || run(rx, s))
            .expect("spawn audio thread");
        Audio { tx, status }
    }

    pub fn play(&self, item: Item) {
        let _ = self.tx.send(Msg::Play(item));
    }

    /// Stops the current announcement and clears the queue.
    pub fn stop(&self) {
        let _ = self.tx.send(Msg::Stop);
    }

    pub fn pause(&self, paused: bool) {
        let _ = self.tx.send(Msg::Pause(paused));
    }
}

struct Speaker {
    synth: windows::Media::SpeechSynthesis::SpeechSynthesizer,
    voices: Vec<windows::Media::SpeechSynthesis::VoiceInformation>,
}

impl Speaker {
    fn new() -> windows::core::Result<Speaker> {
        use windows::Media::SpeechSynthesis::SpeechSynthesizer;
        unsafe {
            // Already initialised is fine; anything else surfaces below.
            let _ = windows::Win32::System::WinRT::RoInitialize(windows::Win32::System::WinRT::RO_INIT_MULTITHREADED);
        }
        let synth = SpeechSynthesizer::new()?;
        let voices = SpeechSynthesizer::AllVoices()?.into_iter().collect();
        Ok(Speaker { synth, voices })
    }

    fn names(&self) -> Vec<String> {
        self.voices.iter().filter_map(|v| v.DisplayName().ok()).map(|n| n.to_string()).collect()
    }

    /// The named voice, else the first one whose voice gender suits the role.
    fn pick(&self, name: &str, role: Role) -> Option<&windows::Media::SpeechSynthesis::VoiceInformation> {
        use windows::Media::SpeechSynthesis::VoiceGender;
        let by_name = self.voices.iter().find(|v| v.DisplayName().is_ok_and(|n| n.to_string() == name));
        let wanted = if role == Role::Captain { VoiceGender::Male } else { VoiceGender::Female };
        by_name
            .or_else(|| self.voices.iter().find(|v| v.Gender().is_ok_and(|g| g == wanted)))
            .or(self.voices.first())
    }

    /// Returns mono samples and their sample rate.
    fn speak(&self, text: &str, role: Role, voice: &Voice) -> Result<(Vec<f32>, u32), String> {
        let wav = (|| -> windows::core::Result<Vec<u8>> {
            use windows::Storage::Streams::DataReader;
            let name = if role == Role::Captain { &voice.captain_voice } else { &voice.purser_voice };
            if let Some(v) = self.pick(name, role) {
                self.synth.SetVoice(v)?;
            }
            self.synth.Options()?.SetSpeakingRate(voice.rate.clamp(0.5, 2.0))?;
            let stream = self.synth.SynthesizeTextToStreamAsync(&windows::core::HSTRING::from(text))?.get()?;
            let size = stream.Size()? as u32;
            let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0)?)?;
            reader.LoadAsync(size)?.get()?;
            let mut buf = vec![0u8; size as usize];
            reader.ReadBytes(&mut buf)?;
            Ok(buf)
        })()
        .map_err(|e| format!("speech: {e}"))?;
        decode_wav(&wav)
    }
}

fn decode_wav(bytes: &[u8]) -> Result<(Vec<f32>, u32), String> {
    let mut r = hound::WavReader::new(Cursor::new(bytes)).map_err(|e| format!("speech audio: {e}"))?;
    let spec = r.spec();
    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().filter_map(Result::ok).collect(),
        hound::SampleFormat::Int => {
            let scale = (1i64 << (spec.bits_per_sample - 1)) as f32;
            r.samples::<i32>().filter_map(Result::ok).map(|s| s as f32 / scale).collect()
        }
    };
    Ok((to_mono(&interleaved, spec.channels), spec.sample_rate))
}

fn to_mono(samples: &[f32], channels: u16) -> Vec<f32> {
    let n = channels.max(1) as usize;
    samples.chunks(n).map(|c| c.iter().sum::<f32>() / n as f32).collect()
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
        Biquad {
            b: [b0 / a0, b1 / a0, b2 / a0],
            a: [-2.0 * cos / a0, (1.0 - alpha) / a0],
            x: [0.0; 2],
            y: [0.0; 2],
        }
    }

    fn run(&mut self, x: f32) -> f32 {
        let y = self.b[0] * x + self.b[1] * self.x[0] + self.b[2] * self.x[1] - self.a[0] * self.y[0] - self.a[1] * self.y[1];
        self.x = [x, self.x[0]];
        self.y = [y, self.y[0]];
        y
    }
}

/// Makes clean speech sound like it comes through small cabin speakers:
/// band-limited, slightly driven, with a short reflection off the cabin walls.
pub fn pa_effect(samples: &[f32], rate: u32) -> Vec<f32> {
    let mut filters = [
        Biquad::new('h', 320.0, rate),
        Biquad::new('h', 320.0, rate),
        Biquad::new('l', 3400.0, rate),
        Biquad::new('l', 3400.0, rate),
    ];
    let drive = 2.2f32;
    let norm = drive.tanh();
    let mut out: Vec<f32> = samples
        .iter()
        .map(|&s| {
            let f = filters.iter_mut().fold(s, |acc, f| f.run(acc));
            (f * drive).tanh() / norm
        })
        .collect();
    let taps = [(0.011, 0.22f32), (0.019, 0.14), (0.031, 0.08)];
    let dry = out.clone();
    for (secs, gain) in taps {
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

/// A two-tone "hi-lo" cabin chime, synthesised (no sampled audio).
pub fn chime(rate: u32) -> Vec<f32> {
    let len = (rate as f32 * 2.2) as usize;
    let mut out = vec![0.0f32; len];
    let notes = [(0.0f32, 1046.5f32), (0.42, 784.0)];
    for (start, freq) in notes {
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

fn silence(rate: u32, secs: f32) -> Vec<f32> {
    vec![0.0; (rate as f32 * secs) as usize]
}

fn load_recording(path: &PathBuf, voice: &Voice) -> Result<SamplesBuffer<f32>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let dec = rodio::Decoder::new(BufReader::new(file)).map_err(|e| format!("{}: {e}", path.display()))?;
    let (channels, rate) = (dec.channels(), dec.sample_rate());
    let samples: Vec<f32> = dec.convert_samples::<f32>().collect();
    Ok(if voice.pa_effect_on_recordings {
        SamplesBuffer::new(1, rate, pa_effect(&to_mono(&samples, channels), rate))
    } else {
        SamplesBuffer::new(channels, rate, samples)
    })
}

/// Builds everything the sink plays for one announcement.
fn render(item: &Item, speaker: Option<&Speaker>) -> Result<Vec<SamplesBuffer<f32>>, String> {
    const CHIME_RATE: u32 = 44_100;
    let mut parts = Vec::new();
    if item.chime && item.voice.chime {
        let c = chime(CHIME_RATE);
        let c = if item.voice.pa_effect { pa_effect(&c, CHIME_RATE).iter().map(|s| s * 0.65).collect() } else { c };
        parts.push(SamplesBuffer::new(1, CHIME_RATE, c));
        parts.push(SamplesBuffer::new(1, CHIME_RATE, silence(CHIME_RATE, 0.4)));
    }
    if !item.recordings.is_empty() {
        for (i, path) in item.recordings.iter().enumerate() {
            if i > 0 {
                parts.push(SamplesBuffer::new(1, CHIME_RATE, silence(CHIME_RATE, 0.6)));
            }
            parts.push(load_recording(path, &item.voice)?);
        }
    } else {
        let speaker = speaker.ok_or("Windows speech isn't available")?;
        let (speech, rate) = speaker.speak(&item.text, item.role, &item.voice)?;
        let speech = if item.voice.pa_effect { pa_effect(&speech, rate) } else { speech };
        parts.push(SamplesBuffer::new(1, rate, speech));
    }
    Ok(parts)
}

fn run(rx: Receiver<Msg>, status: Arc<Mutex<Status>>) {
    let set = |f: &dyn Fn(&mut Status)| f(&mut status.lock().unwrap());
    let speaker = match Speaker::new() {
        Ok(s) => {
            let names = s.names();
            set(&|st| st.voices = names.clone());
            Some(s)
        }
        Err(e) => {
            let msg = format!("Windows speech isn't available: {e}");
            set(&|st| st.error = Some(msg.clone()));
            None
        }
    };
    let output = OutputStream::try_default();
    let (_stream, handle) = match output {
        Ok(o) => o,
        Err(e) => {
            let msg = format!("No audio output: {e}");
            set(&|st| st.error = Some(msg.clone()));
            return;
        }
    };

    let mut queue: VecDeque<Item> = VecDeque::new();
    let mut paused = false;
    loop {
        let titles: Vec<String> = queue.iter().map(|i| i.title.clone()).collect();
        set(&|st| st.queued = titles.clone());
        let item = match queue.pop_front() {
            Some(i) => i,
            None => match rx.recv() {
                Ok(Msg::Play(i)) => i,
                Ok(Msg::Pause(p)) => {
                    paused = p;
                    continue;
                }
                Ok(Msg::Stop) => continue,
                Err(_) => return,
            },
        };
        let Ok(sink) = Sink::try_new(&handle) else { continue };
        sink.set_volume(item.voice.volume.clamp(0.0, 1.0));
        match render(&item, speaker.as_ref()) {
            Ok(parts) => parts.into_iter().for_each(|p| sink.append(p)),
            Err(e) => {
                set(&|st| st.error = Some(e.clone()));
                continue;
            }
        }
        if paused {
            sink.pause();
        }
        let title = item.title.clone();
        set(&|st| {
            st.now_playing = Some(title.clone());
            st.error = None;
        });
        while !sink.empty() {
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(Msg::Play(i)) => {
                    queue.push_back(i);
                    let titles: Vec<String> = queue.iter().map(|i| i.title.clone()).collect();
                    set(&|st| st.queued = titles.clone());
                }
                Ok(Msg::Stop) => {
                    queue.clear();
                    sink.stop();
                    break;
                }
                Ok(Msg::Pause(p)) => {
                    paused = p;
                    if p { sink.pause() } else { sink.play() }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
        set(&|st| st.now_playing = None);
        // A short gap between back-to-back announcements.
        std::thread::sleep(Duration::from_millis(800));
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

    /// Needs Windows speech; writes target\speech-test.wav to listen to.
    #[test]
    #[ignore]
    fn speaks_through_windows() {
        let speaker = Speaker::new().unwrap();
        assert!(!speaker.names().is_empty());
        println!("voices: {:?}", speaker.names());
        let voice = Voice::default();
        let (speech, rate) = speaker.speak("Cabin crew, please take your seats for takeoff.", Role::Captain, &voice).unwrap();
        assert!(speech.len() > rate as usize, "under a second of speech");
        let mut all = pa_effect(&chime(rate), rate);
        all.extend(silence(rate, 0.4));
        all.extend(pa_effect(&speech, rate));
        let spec = hound::WavSpec { channels: 1, sample_rate: rate, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let mut w = hound::WavWriter::create(concat!(env!("CARGO_MANIFEST_DIR"), "/target/speech-test.wav"), spec).unwrap();
        for s in all {
            w.write_sample((s * i16::MAX as f32) as i16).unwrap();
        }
        w.finalize().unwrap();
    }

    #[test]
    fn chime_is_two_seconds_and_quiet_at_the_end() {
        let c = chime(44_100);
        assert!(c.len() > 88_000);
        assert!(c[c.len() - 1].abs() < 0.05);
    }
}
