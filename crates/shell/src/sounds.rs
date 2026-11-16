//! Xbox One–style UI sounds and ambient dashboard music, synthesised (no recorded assets). Files in
//! `%APPDATA%\Win2Console\sounds\` replace them: move.wav, select.wav, back.wav, guide.wav, and
//! ambient.wav or ambient.mp3.

use std::cell::Cell;
use std::f32::consts::TAU;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy)]
pub enum Sound {
    Move,
    Select,
    Back,
    Guide,
    Startup,
    SignIn,
    Launch,
    Achievement,
    Notify,
}

const RATE: u32 = 44_100;
const NAMES: [&str; 9] = ["move", "select", "back", "guide", "startup", "signin", "launch", "achievement", "notification"];
const ALL: [Sound; 9] = [
    Sound::Move,
    Sound::Select,
    Sound::Back,
    Sound::Guide,
    Sound::Startup,
    Sound::SignIn,
    Sound::Launch,
    Sound::Achievement,
    Sound::Notify,
];

fn sounds_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("Win2Console").join("sounds"))
}

/// A soft mallet note: sine + quiet octave, quick attack, exponential decay.
fn note(out: &mut [f32], start: f32, hz: f32, len: f32, gain: f32, decay: f32) {
    let s0 = (start * RATE as f32) as usize;
    let n = (len * RATE as f32) as usize;
    for i in 0..n.min(out.len().saturating_sub(s0)) {
        let t = i as f32 / RATE as f32;
        let env = (t * 300.0).min(1.0) * (-t * decay).exp();
        let p = TAU * hz * t;
        out[s0 + i] += (p.sin() + 0.18 * (2.0 * p).sin() + 0.05 * (3.0 * p).sin()) * env * gain;
    }
}

/// Short airy echo, like the console's roomy UI sounds.
fn echo(buf: &mut [f32], delay: f32, feedback: f32) {
    let d = (delay * RATE as f32) as usize;
    for i in d..buf.len() {
        buf[i] += buf[i - d] * feedback;
    }
}

fn synth(sound: Sound) -> Vec<f32> {
    let secs = match sound {
        Sound::Move => 0.12,
        Sound::Guide => 0.6,
        Sound::Startup => 3.2,
        Sound::SignIn => 1.1,
        Sound::Launch => 1.5,
        Sound::Achievement => 1.8,
        Sound::Notify => 0.8,
        _ => 0.35,
    };
    let mut b = vec![0.0; (secs * RATE as f32) as usize];
    match sound {
        // Low, muted "tock".
        Sound::Move => note(&mut b, 0.0, 740.0, 0.12, 0.16, 55.0),
        // Rising fifth.
        Sound::Select => {
            note(&mut b, 0.0, 659.3, 0.3, 0.2, 18.0);
            note(&mut b, 0.055, 987.8, 0.3, 0.22, 14.0);
        }
        // Falling fourth.
        Sound::Back => {
            note(&mut b, 0.0, 880.0, 0.3, 0.2, 18.0);
            note(&mut b, 0.055, 659.3, 0.3, 0.2, 14.0);
        }
        // Rolled chord.
        Sound::Guide => {
            for (i, hz) in [523.3, 659.3, 784.0, 1046.5].into_iter().enumerate() {
                note(&mut b, i as f32 * 0.035, hz, 0.5, 0.13, 7.0);
            }
        }
        // Boot: a soft swell into a bright, rolled major chord with a long shimmer.
        Sound::Startup => {
            for i in 0..(1.1 * RATE as f32) as usize {
                let t = i as f32 / RATE as f32;
                let env = (t / 1.1).powi(2) * 0.12;
                let p = TAU * (110.0 + 110.0 * t / 1.1) * t;
                b[i] += (p.sin() + 0.5 * (2.0 * p).sin()) * env;
            }
            for (i, hz) in [523.3, 659.3, 784.0, 987.8, 1318.5, 1568.0].into_iter().enumerate() {
                note(&mut b, 1.0 + i as f32 * 0.06, hz, 2.1, 0.11, 2.2);
            }
            note(&mut b, 1.0, 130.8, 2.1, 0.2, 1.6);
        }
        // Sign-in: a warm, quick rising arpeggio.
        Sound::SignIn => {
            for (i, hz) in [523.3, 659.3, 784.0, 1046.5].into_iter().enumerate() {
                note(&mut b, i as f32 * 0.07, hz, 0.9, 0.15, 5.0);
            }
        }
        // Launch: a soft upward sweep into an open fifth.
        Sound::Launch => {
            for i in 0..(0.35 * RATE as f32) as usize {
                let t = i as f32 / RATE as f32;
                let env = (t / 0.35).powi(2) * 0.08;
                b[i] += (TAU * (300.0 * t + 600.0 * t * t / 0.7)).sin() * env;
            }
            for hz in [392.0, 587.3, 784.0] {
                note(&mut b, 0.33, hz, 1.1, 0.15, 3.5);
            }
        }
        // Achievement: a bright chord hit, then a high chime.
        Sound::Achievement => {
            for hz in [987.8, 1318.5, 1661.2] {
                note(&mut b, 0.0, hz, 0.8, 0.12, 6.0);
            }
            note(&mut b, 0.16, 1975.5, 1.4, 0.14, 3.0);
            note(&mut b, 0.16, 493.9, 1.2, 0.12, 3.0);
        }
        // Notification: a gentle two-note ding.
        Sound::Notify => {
            note(&mut b, 0.0, 880.0, 0.6, 0.15, 8.0);
            note(&mut b, 0.09, 1318.5, 0.7, 0.15, 6.0);
        }
    }
    // Startup gets a bigger room.
    let (delay, feedback) = if matches!(sound, Sound::Startup) { (0.11, 0.35) } else { (0.07, 0.22) };
    echo(&mut b, delay, feedback);
    b
}

/// ~32 s floaty pad that loops seamlessly: bright, airy chords (detuned for a soft shimmer), sparse
/// glassy chimes from a pentatonic scale, and a wrap-around echo so the loop has no seam.
fn synth_ambient() -> Vec<f32> {
    const SR: f32 = 22_050.0;
    let chords: [&[f32]; 4] = [
        &[174.6, 220.0, 329.6, 392.0, 523.3], // Fmaj9
        &[261.6, 392.0, 587.3, 659.3],        // Cadd9
        &[220.0, 329.6, 493.9, 523.3],        // Am(add9)
        &[196.0, 293.7, 440.0, 493.9],        // Gsus2(add3)
    ];
    let per = 8.0;
    let n = (per * chords.len() as f32 * SR) as usize;
    let mut out = vec![0.0f32; n];
    for (c, chord) in chords.iter().enumerate() {
        // Each chord fades in/out over the neighbouring ones (wrapping), so the loop has no seam.
        for k in -1i32..=1 {
            let start = (c as f32 + k as f32 * chords.len() as f32) * per - per * 0.5;
            for (i, sample) in out.iter_mut().enumerate() {
                let t = i as f32 / SR - start;
                if !(0.0..per * 2.0).contains(&t) {
                    continue;
                }
                let env = (std::f32::consts::PI * t / (per * 2.0)).sin().powi(2);
                let x = i as f32 / SR;
                let mut v = 0.0;
                for (j, &hz) in chord.iter().enumerate() {
                    // Two slightly detuned voices per note, gently breathing in and out of phase.
                    let breathe = 0.75 + 0.25 * (TAU * 0.07 * x + j as f32 * 1.7).sin();
                    v += ((TAU * hz * 1.0015 * x).sin() + (TAU * hz * 0.9985 * x).sin()) * 0.022 * breathe;
                    v += (TAU * hz * 2.0 * x).sin() * 0.004;
                }
                *sample += v * env;
            }
        }
    }
    // Chimes: one every 1.6 s or so, picked from C major pentatonic up high, with long soft tails.
    let chimes = [1046.5, 1174.7, 1318.5, 1568.0, 1760.0, 784.0, 880.0];
    let mut seed: u32 = 0x9E37_79B9;
    let mut at = 0.4;
    while at < per * chords.len() as f32 {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        let hz = chimes[(seed % chimes.len() as u32) as usize];
        let s0 = (at * SR) as usize;
        for i in 0..(5.0 * SR) as usize {
            let t = i as f32 / SR;
            let env = (t * 60.0).min(1.0) * (-t * 1.1).exp() * 0.03;
            let p = TAU * hz * t;
            out[(s0 + i) % n] += (p.sin() + 0.25 * (2.0 * p).sin()) * env;
        }
        at += 1.2 + (seed % 1000) as f32 / 1000.0 * 0.9;
    }
    // Airy wrap-around echo (a few taps), then normalise.
    let dry = out.clone();
    for (tap, gain) in [(0.43, 0.42), (0.86, 0.22), (1.29, 0.11)] {
        let d = (tap * SR) as usize;
        for i in 0..n {
            out[i] += dry[(i + n - d) % n] * gain;
        }
    }
    let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(1e-6);
    out.iter_mut().for_each(|v| *v *= 0.45 / peak);
    out
}

fn wav(samples: &[f32], rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut w = Vec::with_capacity(44 + data_len as usize);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data_len).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    for v in [16u32] {
        w.extend_from_slice(&v.to_le_bytes());
    }
    for v in [1u16, 1] {
        w.extend_from_slice(&v.to_le_bytes()); // PCM, mono
    }
    w.extend_from_slice(&rate.to_le_bytes());
    w.extend_from_slice(&(rate * 2).to_le_bytes());
    for v in [2u16, 16] {
        w.extend_from_slice(&v.to_le_bytes());
    }
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        w.extend_from_slice(&((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).to_le_bytes());
    }
    w
}

/// Writes `sounds\README.txt`, listing every file that replaces a built-in sound.
pub fn write_index() {
    let Some(dir) = sounds_dir() else { return };
    let text = "\
Win2Console sounds
==================

Put a file with one of these names in this folder to replace the built-in sound.
Delete it to go back to the built-in one. Changes apply the next time Win2Console starts.

UI sounds (.wav only):
  move.wav      moving the selection
  select.wav    pressing A / opening something
  back.wav      pressing B / going back
  guide.wav     opening the Guide
  startup.wav   the built-in startup animation
  signin.wav    signing in to a profile
  launch.wav    starting a game
  achievement.wav     unlocking an achievement
  notification.wav    messages, friend requests and other notifications

Background music (.mp3 or .wav, loops):
  ambient.mp3   or   ambient.wav

The startup video isn't in here: choose it in Settings > System > Startup animation.
";
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join("README.txt"), text.replace('\n', "\r\n"));
}

thread_local! {
    /// A mixer, so sounds overlap instead of cutting each other off (which clicks).
    static OUTPUT: Option<(rodio::OutputStream, rodio::OutputStreamHandle)> = rodio::OutputStream::try_default()
        .inspect_err(|e| tracing::warn!("sound output: {e}"))
        .ok();
    static LAST_MOVE: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// Fast scrolling plays at most one "move" sound this often (more just piles up into noise).
const MOVE_GAP: Duration = Duration::from_millis(45);

pub fn play(sound: Sound) {
    static CACHE: OnceLock<[Vec<u8>; 9]> = OnceLock::new();
    let all = CACHE.get_or_init(|| {
        let custom = |i: usize| sounds_dir().and_then(|d| std::fs::read(d.join(format!("{}.wav", NAMES[i]))).ok());
        ALL.map(|s| custom(s as usize).unwrap_or_else(|| wav(&synth(s), RATE)))
    });
    if matches!(sound, Sound::Move) {
        let now = Instant::now();
        if LAST_MOVE.get().is_some_and(|t| now - t < MOVE_GAP) {
            return;
        }
        LAST_MOVE.set(Some(now));
    }
    let bytes: &'static [u8] = &all[sound as usize];
    OUTPUT.with(|out| match out {
        Some((_, handle)) => {
            use rodio::Source;
            match rodio::Decoder::new(std::io::Cursor::new(bytes)) {
                Ok(source) => {
                    let _ = handle.play_raw(source.convert_samples());
                }
                Err(e) => tracing::warn!("sound {}: {e}", NAMES[sound as usize]),
            }
        }
        None => cs_platform::sound::play_wav(bytes),
    });
}

thread_local! {
    static MUSIC: Cell<Option<bool>> = const { Cell::new(None) };
}

/// Starts or pauses the ambient music (UI thread). The generated track is written once to the cache.
pub fn set_music(on: bool) {
    if MUSIC.get() == Some(on) {
        return;
    }
    MUSIC.set(Some(on));
    if !on {
        cs_platform::sound::music_pause();
        return;
    }
    let user =
        sounds_dir().and_then(|d| ["ambient.mp3", "ambient.wav"].into_iter().map(|f| d.join(f)).find(|p| p.is_file()));
    if let Some(path) = user {
        cs_platform::sound::music_play(&path, 300);
        return;
    }
    let Some(path) =
        std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("Win2Console").join("cache").join("ambient-v2.wav"))
    else {
        return;
    };
    if path.is_file() {
        cs_platform::sound::music_play(&path, 300);
        return;
    }
    std::thread::spawn(move || {
        let _ = std::fs::create_dir_all(path.parent().unwrap_or(&path));
        if std::fs::write(&path, wav(&synth_ambient(), 22_050)).is_ok() {
            crate::app::post(move |_| {
                if MUSIC.get() == Some(true) {
                    cs_platform::sound::music_play(&path, 300);
                }
            });
        }
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn wav_header_is_consistent() {
        let w = super::wav(&super::synth(super::Sound::Select), super::RATE);
        assert_eq!(&w[..4], b"RIFF");
        let data_len = u32::from_le_bytes(w[40..44].try_into().unwrap()) as usize;
        assert_eq!(w.len(), 44 + data_len);
    }
}
