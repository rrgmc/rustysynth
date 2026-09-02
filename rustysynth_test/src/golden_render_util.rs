#![allow(dead_code)]

use rustysynth::SoundFont;
use rustysynth::Synthesizer;
use rustysynth::SynthesizerSettings;
use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;

/// One scheduled MIDI event: `(step, channel, command, data1, data2)`.
/// `step` counts 64-sample render blocks from the start of the run.
pub type Event = (usize, i32, i32, i32, i32);

pub const SAMPLE_RATE: i32 = 44100;
pub const BLOCK: usize = 64;

/// Only every Nth frame is kept in the reference waveform. A gain, filter or
/// pan error shifts every sample, so decimating costs almost no sensitivity
/// and keeps the checked-in fixture under 100 KB.
pub const DECIMATION: usize = 16;

/// FNV-1a over the raw bit patterns of the rendered samples.
///
/// Hashing the bits rather than the values makes the comparison exact: a
/// change of one ULP anywhere in the run moves the hash.
pub struct Fnv1a(u64);

impl Fnv1a {
    pub fn new() -> Self {
        Fnv1a(0xcbf2_9ce4_8422_2325)
    }

    pub fn write_f32(&mut self, value: f32) {
        for byte in value.to_bits().to_le_bytes() {
            self.0 ^= byte as u64;
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }

    pub fn finish(&self) -> u64 {
        self.0
    }
}

impl Default for Fnv1a {
    fn default() -> Self {
        Fnv1a::new()
    }
}

/// Summary of a scripted render: the exact hash plus stats that make a
/// mismatch diagnosable rather than merely red.
#[derive(Debug)]
pub struct RenderResult {
    pub hash: u64,
    pub peak: f32,
    pub rms: f32,
    pub non_finite: usize,
    pub nonzero_blocks: usize,
    /// Decimated stereo waveform, left and right interleaved.
    pub samples: Vec<f32>,
}

pub fn open_timgm6mb() -> Arc<SoundFont> {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop();
    path.push("TimGM6mb.sf2");

    let mut file = File::open(&path).unwrap();
    Arc::new(SoundFont::new(&mut file).unwrap())
}

/// Runs `events` against a fresh synthesizer and renders `steps` blocks.
pub fn render_script(sound_font: &Arc<SoundFont>, events: &[Event], steps: usize) -> RenderResult {
    let settings = SynthesizerSettings::new(SAMPLE_RATE);
    let mut synthesizer = Synthesizer::new(sound_font, &settings).unwrap();

    let mut left = vec![0_f32; BLOCK];
    let mut right = vec![0_f32; BLOCK];

    let mut hash = Fnv1a::new();
    let mut peak = 0_f32;
    let mut sum_squares = 0_f64;
    let mut non_finite = 0_usize;
    let mut nonzero_blocks = 0_usize;

    let mut samples: Vec<f32> = Vec::with_capacity(2 * steps * BLOCK / DECIMATION + 2);
    let mut frame = 0_usize;

    let mut next = 0_usize;

    for step in 0..steps {
        while next < events.len() && events[next].0 == step {
            let (_, channel, command, data1, data2) = events[next];
            synthesizer.process_midi_message(channel, command, data1, data2);
            next += 1;
        }

        synthesizer.render(&mut left[..], &mut right[..]);

        let mut block_energy = 0_f32;
        for i in 0..BLOCK {
            if frame.is_multiple_of(DECIMATION) {
                samples.push(left[i]);
                samples.push(right[i]);
            }
            frame += 1;

            for value in [left[i], right[i]] {
                hash.write_f32(value);

                if value.is_finite() {
                    peak = peak.max(value.abs());
                    sum_squares += (value as f64) * (value as f64);
                    block_energy += value.abs();
                } else {
                    non_finite += 1;
                }
            }
        }

        if block_energy > 0_f32 {
            nonzero_blocks += 1;
        }
    }

    RenderResult {
        hash: hash.finish(),
        peak,
        rms: (sum_squares / (2 * BLOCK * steps) as f64).sqrt() as f32,
        non_finite,
        nonzero_blocks,
        samples,
    }
}

/// The legacy control script.
///
/// It deliberately exercises **only** what RustySynth already responds to
/// today - velocity, CC1, CC7, CC10, CC11, CC64, CC91, CC93 and pitch bend -
/// so that implementing the SF2 default modulators has to reproduce it bit for
/// bit. Channel pressure and polyphonic pressure are excluded on purpose:
/// default modulator #3 gives channel pressure an audible effect it does not
/// have today, so including it would make an expected change look like a
/// regression.
pub fn legacy_script() -> Vec<Event> {
    // Three timbres with quite different envelopes and sample sets.
    let mut events: Vec<Event> = vec![
        (0, 0, 0xC0, 0, 0),  // acoustic grand
        (0, 1, 0xC0, 48, 0), // string ensemble
        (0, 2, 0xC0, 30, 0), // overdriven guitar
        (0, 9, 0xC0, 0, 0),  // percussion (channel 9 is forced to bank 128)
    ];

    // Velocity response across the full range, on a sustaining patch.
    for (i, velocity) in [1, 16, 32, 48, 64, 80, 96, 112, 127].iter().enumerate() {
        let start = 8 + 40 * i;
        events.push((start, 1, 0x90, 48 + i as i32, *velocity));
        events.push((start + 32, 1, 0x80, 48 + i as i32, 0));
    }

    // CC7 volume and CC11 expression sweeps over held notes - the two
    // controllers that become concave attenuation modulators.
    events.push((400, 0, 0x90, 60, 100));
    events.push((400, 0, 0x90, 64, 100));
    events.push((400, 0, 0x90, 67, 100));
    for step in 0..64_usize {
        let value = 127 - (step * 2).min(127);
        events.push((410 + step, 0, 0xB0, 0x07, value as i32));
    }
    for step in 0..64_usize {
        let value = (step * 2).min(127);
        events.push((480 + step, 0, 0xB0, 0x0B, value as i32));
    }
    events.push((560, 0, 0x80, 60, 0));
    events.push((560, 0, 0x80, 64, 0));
    events.push((560, 0, 0x80, 67, 0));

    // Fine-resolution CC7: RustySynth tracks volume at 14 bits, and the
    // modulator path has to keep doing so.
    events.push((580, 0, 0xB0, 0x07, 100));
    events.push((580, 0, 0xB0, 0x27, 64));
    events.push((580, 0, 0x90, 55, 90));
    events.push((660, 0, 0x80, 55, 0));

    // CC10 pan sweep, including both saturating extremes.
    events.push((680, 2, 0x90, 52, 105));
    for step in 0..48_usize {
        let value = (step * 127 / 47).min(127);
        events.push((690 + step, 2, 0xB0, 0x0A, value as i32));
    }
    events.push((760, 2, 0x80, 52, 0));

    // CC1 modulation depth - the vibrato LFO path.
    events.push((780, 1, 0x90, 62, 100));
    for step in 0..32_usize {
        let value = (step * 4).min(127);
        events.push((790 + step, 1, 0xB0, 0x01, value as i32));
    }
    events.push((850, 1, 0x80, 62, 0));

    // CC91 reverb and CC93 chorus sends, which drive the IIR effect state.
    events.push((870, 0, 0xB0, 0x5B, 127));
    events.push((870, 0, 0xB0, 0x5D, 127));
    events.push((870, 0, 0x90, 57, 110));
    events.push((930, 0, 0x80, 57, 0));
    events.push((950, 0, 0xB0, 0x5B, 0));
    events.push((950, 0, 0xB0, 0x5D, 0));
    events.push((950, 0, 0x90, 59, 110));
    events.push((1010, 0, 0x80, 59, 0));

    // Pitch bend, with an explicit RPN 0 sensitivity change first. The
    // decision to keep pitch bend native rather than routing it through a
    // fineTune modulator is what this covers.
    events.push((1030, 1, 0xB0, 0x65, 0));
    events.push((1030, 1, 0xB0, 0x64, 0));
    events.push((1030, 1, 0xB0, 0x06, 12));
    events.push((1030, 1, 0x90, 60, 100));
    for step in 0..48_usize {
        let bend = (step * 16383 / 47).min(16383);
        events.push((
            1040 + step,
            1,
            0xE0,
            (bend & 0x7F) as i32,
            (bend >> 7) as i32,
        ));
    }
    events.push((1100, 1, 0x80, 60, 0));
    events.push((1110, 1, 0xE0, 0, 64)); // centre

    // Hold pedal: notes have to sustain past their note-off.
    events.push((1130, 0, 0xB0, 0x40, 127));
    events.push((1130, 0, 0x90, 48, 100));
    events.push((1140, 0, 0x80, 48, 0));
    events.push((1200, 0, 0xB0, 0x40, 0));

    // Percussion, which exercises the exclusive-class choke path.
    for i in 0..8_usize {
        events.push((1220 + 12 * i, 9, 0x90, 42, 100));
        events.push((1226 + 12 * i, 9, 0x80, 42, 0));
    }

    // A dense chord to push polyphony and voice stealing.
    for (i, key) in [36, 40, 43, 47, 50, 55, 60, 64, 67, 72].iter().enumerate() {
        events.push((1330 + i, 0, 0x90, *key, 100));
        events.push((1330 + i, 1, 0x90, *key - 12, 90));
        events.push((1330 + i, 2, 0x90, *key + 5, 80));
    }
    events.push((1450, 0, 0xB0, 0x7B, 0)); // all notes off
    events.push((1450, 1, 0xB0, 0x7B, 0));
    events.push((1450, 2, 0xB0, 0x7B, 0));

    events.sort_by_key(|event| event.0);
    events
}
