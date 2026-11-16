//! Xbox One–style UI sounds and ambient dashboard music, synthesised (no recorded assets). Files in
//! `%APPDATA%\Win2Console\sounds\` replace them: move.wav, select.wav, back.wav, guide.wav, and
//! ambient.wav or ambient.mp3.

use std::cell::Cell;
use std::f32::consts::TAU;
use std::path::PathBuf;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy)]
pub enum Sound {
    Move,
    Select,
    Back,
    Guide,
    Startup,
}

const RATE: u32 = 44_100;
const NAMES: [&str; 5] = ["move", "select", "back", "guide", "startup"];

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
    }
    // Startup gets a bigger room.
    let (delay, feedback) = if matches!(sound, Sound::Startup) { (0.11, 0.35) } else { (0.07, 0.22) };
    echo(&mut b, delay, feedback);
    b
}

/// ~32 s pad that loops seamlessly: slow chords of soft partials with a gentle swell.
fn synth_ambient() -> Vec<f32> {
    const SR: f32 = 22_050.0;
    let chords: [[f32; 4]; 4] = [
        [130.8, 196.0, 246.9, 329.6], // Cmaj7
        [110.0, 164.8, 261.6, 329.6], // Am7
        [87.3, 174.6, 220.0, 329.6],  // Fmaj7
        [98.0, 146.8, 246.9, 293.7],  // G
    ];
    let per = 8.0;
    let n = (per * chords.len() as f32 * SR) as usize;
    let mut out = vec![0.0f32; n];
    for (c, chord) in chords.iter().enumerate() {
        // Each chord fades in/out over the neighbouring ones (wrapping), so the loop has no seam.
        for k in -1i32..=1 {
            let start = (c as f32 + k as f32 * chords.len() as f32) * per - per * 0.5;
            for i in 0..n {
                let t = i as f32 / SR - start;
                if !(0.0..per * 2.0).contains(&t) {
                    continue;
                }
                let env = (std::f32::consts::PI * t / (per * 2.0)).sin().powi(2);
                let mut v = 0.0;
                for (j, &hz) in chord.iter().enumerate() {
                    let wob = 1.0 + 0.002 * (TAU * 0.13 * t + j as f32).sin();
                    v += (TAU * hz * wob * (i as f32 / SR)).sin() * 0.06;
                    v += (TAU * hz * 2.0 * (i as f32 / SR)).sin() * 0.012;
                }
                out[i] += v * env;
            }
        }
    }
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

pub fn play(sound: Sound) {
    static CACHE: OnceLock<[Vec<u8>; 5]> = OnceLock::new();
    let all = CACHE.get_or_init(|| {
        let custom = |i: usize| sounds_dir().and_then(|d| std::fs::read(d.join(format!("{}.wav", NAMES[i]))).ok());
        [Sound::Move, Sound::Select, Sound::Back, Sound::Guide, Sound::Startup]
            .map(|s| custom(s as usize).unwrap_or_else(|| wav(&synth(s), RATE)))
    });
    cs_platform::sound::play_wav(&all[sound as usize]);
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
        std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("Win2Console").join("cache").join("ambient-v1.wav"))
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
