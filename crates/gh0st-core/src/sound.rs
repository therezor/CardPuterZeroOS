//! System sounds: short ticks and chimes for moving, choosing, going back,
//! locking and the rest. They are made here from a few notes each, so no
//! sound files ship and each sound costs a few kilobytes of RAM, made on
//! first use. The shell plays them; an app on the link asks it to with
//! `sound NAME`.

/// Samples a second. PipeWire's own rate on the device, so nothing resamples.
pub const RATE: u32 = 48_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Sound {
    /// The selection moved one step.
    Move,
    /// A row, a chip or a toggle took the press.
    Select,
    /// A step back, a page closed.
    Back,
    /// An app opened or came back.
    Open,
    /// An app closed.
    Close,
    Lock,
    Unlock,
    /// A press that did nothing: a wrong password, a page that can't go on.
    Error,
    /// A notice arrived.
    Notice,
    /// The battery is low.
    Warn,
    /// The first lock screen after power on.
    Boot,
    /// A photo taken: two clicks, as a shutter's blades.
    Shutter,
    /// Recording starts: one beep.
    RecStart,
    /// Recording stopped and saved: two beeps.
    RecStop,
    /// A second of a self-timer.
    Count,
}

impl Sound {
    pub const ALL: [Sound; 15] = [
        Sound::Move,
        Sound::Select,
        Sound::Back,
        Sound::Open,
        Sound::Close,
        Sound::Lock,
        Sound::Unlock,
        Sound::Error,
        Sound::Notice,
        Sound::Warn,
        Sound::Boot,
        Sound::Shutter,
        Sound::RecStart,
        Sound::RecStop,
        Sound::Count,
    ];

    /// Its word on the link.
    pub fn name(self) -> &'static str {
        match self {
            Sound::Move => "move",
            Sound::Select => "select",
            Sound::Back => "back",
            Sound::Open => "open",
            Sound::Close => "close",
            Sound::Lock => "lock",
            Sound::Unlock => "unlock",
            Sound::Error => "error",
            Sound::Notice => "notice",
            Sound::Warn => "warn",
            Sound::Boot => "boot",
            Sound::Shutter => "shutter",
            Sound::RecStart => "rec-start",
            Sound::RecStop => "rec-stop",
            Sound::Count => "count",
        }
    }

    pub fn from_name(name: &str) -> Option<Sound> {
        Sound::ALL.into_iter().find(|s| s.name() == name)
    }

    /// The sound as signed 16-bit mono samples at [`RATE`].
    pub fn render(self) -> Vec<i16> {
        // Notes of A major, so any two sounds that overlap agree.
        const A4: f32 = 440.0;
        const E5: f32 = 659.3;
        const A5: f32 = 880.0;
        const CS6: f32 = 1108.7;
        const E6: f32 = 1318.5;
        const A6: f32 = 1760.0;
        const B6: f32 = 1975.5;
        const EB4: f32 = 311.1;
        let plain = Voice::Sine;
        let (notes, peak): (&[Note], f32) = match self {
            Sound::Move => (&[Note::new(0, A5, 22, 5.0, Voice::Pop)], 0.12),
            Sound::Select => (&[Note::new(0, E6, 90, 30.0, Voice::Wood)], 0.18),
            Sound::Back => (&[Note::new(0, A5, 90, 30.0, Voice::Wood)], 0.16),
            Sound::Open => (
                &[
                    Note::new(0, CS6, 50, 14.0, plain),
                    Note::new(30, E6, 50, 14.0, plain),
                    Note::new(60, A6, 110, 30.0, plain),
                ],
                0.24,
            ),
            Sound::Close => (
                &[
                    Note::new(0, A6, 50, 14.0, plain),
                    Note::new(30, E6, 50, 14.0, plain),
                    Note::new(60, CS6, 110, 30.0, plain),
                ],
                0.22,
            ),
            Sound::Lock => (
                &[
                    Note::new(0, E6, 120, 40.0, Voice::Wood),
                    Note::new(70, CS6, 120, 40.0, Voice::Wood),
                    Note::new(140, A5, 320, 110.0, Voice::Wood),
                ],
                0.22,
            ),
            Sound::Unlock => (
                &[
                    Note::new(0, A5, 120, 40.0, Voice::Wood),
                    Note::new(70, CS6, 120, 40.0, Voice::Wood),
                    Note::new(140, E6, 340, 120.0, Voice::Wood),
                ],
                0.22,
            ),
            Sound::Error => (
                &[
                    Note::new(0, EB4, 70, 40.0, Voice::Hollow),
                    Note::new(110, EB4, 90, 45.0, Voice::Hollow),
                ],
                0.26,
            ),
            Sound::Notice => (
                &[
                    Note::new(0, E6, 260, 70.0, Voice::Bell),
                    Note::new(110, B6, 340, 90.0, Voice::Bell),
                ],
                0.24,
            ),
            Sound::Warn => (
                &[
                    Note::new(0, A5, 80, 60.0, Voice::Hollow),
                    Note::new(150, A5, 80, 60.0, Voice::Hollow),
                    Note::new(300, A5, 80, 60.0, Voice::Hollow),
                ],
                0.26,
            ),
            Sound::Boot => (
                &[
                    Note::new(0, A4, 500, 160.0, Voice::Bell),
                    Note::new(90, E5, 500, 160.0, Voice::Bell),
                    Note::new(180, A5, 500, 160.0, Voice::Bell),
                    Note::new(270, CS6, 700, 240.0, Voice::Bell),
                ],
                0.24,
            ),
            Sound::Shutter => (
                &[
                    Note::new(0, 3000.0, 20, 4.0, Voice::Noise),
                    Note::new(60, 1700.0, 32, 7.0, Voice::Noise),
                ],
                0.24,
            ),
            Sound::RecStart => (&[Note::new(0, E6, 140, 90.0, plain)], 0.22),
            Sound::RecStop => (
                &[
                    Note::new(0, E6, 70, 40.0, plain),
                    Note::new(110, E6, 70, 40.0, plain),
                ],
                0.22,
            ),
            Sound::Count => (&[Note::new(0, A5, 50, 25.0, plain)], 0.18),
        };
        mix(notes, peak)
    }
}

#[derive(Clone, Copy)]
enum Voice {
    /// A sine with a touch of its octave: a soft tick or blip.
    Sine,
    /// Odd harmonics, square-ish: a buzz for errors and warnings.
    Hollow,
    /// A sine with the inharmonic partials of a small bell.
    Bell,
    /// Noise through a band around the note's pitch: a click.
    Noise,
    /// A drop: a sine that falls from half again its pitch in a few ms.
    Pop,
    /// A marimba's bar: a sine and its fourth partial, which dies at once
    /// as a mallet's knock.
    Wood,
}

#[derive(Clone, Copy)]
struct Note {
    at_ms: u32,
    hz: f32,
    len_ms: u32,
    /// Milliseconds for the level to fall to 1/e.
    decay_ms: f32,
    voice: Voice,
}

impl Note {
    const fn new(at_ms: u32, hz: f32, len_ms: u32, decay_ms: f32, voice: Voice) -> Note {
        Note {
            at_ms,
            hz,
            len_ms,
            decay_ms,
            voice,
        }
    }
}

fn ms(ms: u32) -> usize {
    (ms * RATE / 1000) as usize
}

/// Adds the notes together and scales the loudest sample to `peak` of full
/// scale. The level is in the samples, not in a stream volume, so it
/// follows the device volume and nothing is saved for the stream.
fn mix(notes: &[Note], peak: f32) -> Vec<i16> {
    let len = notes
        .iter()
        .map(|n| ms(n.at_ms) + ms(n.len_ms))
        .max()
        .unwrap_or(0);
    let mut out = vec![0f32; len];
    let attack = ms(2) as f32;
    // The last 4 ms fade out, so no note ends on a click.
    let release = ms(4) as f32;
    for n in notes {
        let start = ms(n.at_ms);
        let count = ms(n.len_ms);
        let decay = n.decay_ms / 1000.0 * RATE as f32;
        let mut noise = Band::new(n.hz, 1.2);
        // Fixed, so a sound is the same every time.
        let mut seed = 0x2545_f491u32;
        for i in 0..count {
            let t = i as f32 / RATE as f32;
            let w = std::f32::consts::TAU * n.hz * t;
            let wave = match n.voice {
                Voice::Sine => w.sin() + 0.15 * (2.0 * w).sin(),
                Voice::Hollow => w.sin() + (3.0 * w).sin() / 3.0 + (5.0 * w).sin() / 5.0,
                Voice::Bell => w.sin() + 0.4 * (2.76 * w).sin() * (-t * 30.0).exp(),
                Voice::Pop => {
                    // The phase of a pitch gliding from 1.5x down to 1x.
                    let tau = 0.003;
                    let glide = 0.5 * tau * (1.0 - (-t / tau).exp());
                    (std::f32::consts::TAU * n.hz * (t + glide)).sin()
                }
                Voice::Wood => w.sin() + 0.45 * (3.93 * w).sin() * (-t / 0.004).exp(),
                Voice::Noise => {
                    seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    let white = (seed >> 8) as f32 / (1u32 << 23) as f32 - 1.0;
                    noise.run(white)
                }
            };
            let rise = (i as f32 / attack).min(1.0);
            let fall = ((count - i) as f32 / release).min(1.0);
            out[start + i] += wave * (-(i as f32) / decay).exp() * rise * fall;
        }
    }
    let loudest = out.iter().fold(0f32, |m, s| m.max(s.abs()));
    let gain = if loudest > 0.0 { peak / loudest } else { 0.0 };
    out.iter()
        .map(|s| (s * gain * i16::MAX as f32) as i16)
        .collect()
}

/// A band-pass filter (RBJ's biquad, 0 dB at the centre).
struct Band {
    b0: f32,
    a1: f32,
    a2: f32,
    x: [f32; 2],
    y: [f32; 2],
}

impl Band {
    fn new(hz: f32, q: f32) -> Band {
        let w = std::f32::consts::TAU * hz / RATE as f32;
        let alpha = w.sin() / (2.0 * q);
        let a0 = 1.0 + alpha;
        Band {
            b0: alpha / a0,
            a1: -2.0 * w.cos() / a0,
            a2: (1.0 - alpha) / a0,
            x: [0.0; 2],
            y: [0.0; 2],
        }
    }

    fn run(&mut self, x: f32) -> f32 {
        let y = self.b0 * (x - self.x[1]) - self.a1 * self.y[0] - self.a2 * self.y[1];
        self.x = [x, self.x[0]];
        self.y = [y, self.y[0]];
        y
    }
}

/// A WAV file of the samples, to listen to a sound off the device.
pub fn wav(samples: &[i16]) -> Vec<u8> {
    let data = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sound_has_a_name_that_reads_back() {
        for s in Sound::ALL {
            assert_eq!(Sound::from_name(s.name()), Some(s));
        }
        assert_eq!(Sound::from_name("louder"), None);
    }

    #[test]
    fn sounds_are_short_quiet_and_start_and_end_at_rest() {
        for s in Sound::ALL {
            let pcm = s.render();
            let secs = pcm.len() as f32 / RATE as f32;
            let limit = if s == Sound::Boot { 1.0 } else { 0.5 };
            assert!(secs > 0.0 && secs <= limit, "{s:?} lasts {secs} s");
            let loudest = pcm.iter().map(|v| v.unsigned_abs()).max().unwrap_or(0);
            // Never past a third of full scale: the amplifier is loud.
            assert!(
                loudest > 1000 && loudest < i16::MAX as u16 / 3,
                "{s:?} peaks at {loudest}"
            );
            let edge = |v: &[i16]| v.iter().map(|x| x.unsigned_abs()).max().unwrap_or(0);
            assert!(edge(&pcm[..16]) < 2000, "{s:?} starts with a click");
            assert!(
                edge(&pcm[pcm.len() - 16..]) < 200,
                "{s:?} ends with a click"
            );
        }
    }

    #[test]
    fn a_wav_has_its_header_and_every_sample() {
        let w = wav(&[1, -1, 300]);
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(&w[8..16], b"WAVEfmt ");
        assert_eq!(w.len(), 44 + 6);
        assert_eq!(u32::from_le_bytes([w[24], w[25], w[26], w[27]]), RATE);
    }
}
